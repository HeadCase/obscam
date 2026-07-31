use std::{
    io::{self, BufRead, BufReader, Read, Write},
    net::{SocketAddr, UdpSocket},
    path::Path,
    process::{Child, ChildStderr, ChildStdin, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::Duration,
};

use crate::{PublishedFrame, correlation::CorrelationState};

const RTP_INPUT_ADDRESS: &str = "127.0.0.1:5002";
const RTCP_INPUT_ADDRESS: &str = "127.0.0.1:5003";
const RTP_RELAY_ADDRESS: &str = "127.0.0.1:5004";
const RTCP_RELAY_ADDRESS: &str = "127.0.0.1:5005";
const RTP_URL: &str = "rtp://127.0.0.1:5002?rtcpport=5003&pkt_size=1200";
const RTP_CLOCK_STEP: u64 = 4_500;
const RTP_PAYLOAD_TYPE: u8 = 96;
const RTP_SSRC: u32 = 1_868_722_033;

/// One long-lived `FFmpeg` hardware-H.264 publication child.
#[derive(Debug)]
pub struct FfmpegEncoder {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stderr: Option<thread::JoinHandle<()>>,
    observer: Option<RtpObserver>,
    correlation: Option<(CorrelationState, u64)>,
}

impl FfmpegEncoder {
    /// Starts the qualified native-resolution hardware encode profile.
    ///
    /// This constructor retains the process boundary for focused tests. The
    /// production pipeline uses the correlated constructor and local RTP relay.
    ///
    /// # Errors
    ///
    /// Returns the process spawn or pipe error. No alternate encoder is attempted.
    pub fn start(program: &Path) -> io::Result<Self> {
        Self::spawn(program, None)
    }

    pub(crate) fn start_correlated(
        program: &Path,
        correlation: CorrelationState,
    ) -> io::Result<Self> {
        let stream_epoch = correlation.begin_stream();
        Self::spawn(program, Some((correlation, stream_epoch)))
    }

    fn spawn(program: &Path, correlation: Option<(CorrelationState, u64)>) -> io::Result<Self> {
        let (pts_sender, pts_receiver) = mpsc::sync_channel(128);
        let evidence_valid = Arc::new(AtomicBool::new(true));
        let observer = correlation
            .as_ref()
            .map(|(state, stream_epoch)| {
                RtpObserver::start(state.clone(), *stream_epoch, pts_receiver, &evidence_valid)
            })
            .transpose()?;
        let mut command = Command::new(program);
        command
            .args(Self::arguments())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(if correlation.is_some() {
                Stdio::piped()
            } else {
                Stdio::inherit()
            });
        let mut child = command.spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("FFmpeg stdin pipe was not created"))?;
        let stderr = if correlation.is_some() {
            let stderr = child
                .stderr
                .take()
                .ok_or_else(|| io::Error::other("FFmpeg stderr pipe was not created"))?;
            Some(spawn_timestamp_reader(stderr, pts_sender, evidence_valid)?)
        } else {
            None
        };
        Ok(Self {
            child: Some(child),
            stdin: Some(stdin),
            stderr,
            observer,
            correlation,
        })
    }

    fn arguments() -> [&'static str; 37] {
        [
            "-hide_banner",
            "-loglevel",
            "verbose",
            "-debug_ts",
            "-nostdin",
            "-f",
            "rawvideo",
            "-pixel_format",
            "yuv420p",
            "-video_size",
            "1920x1080",
            "-framerate",
            "20",
            "-i",
            "pipe:0",
            "-an",
            "-c:v",
            "h264_v4l2m2m",
            "-b:v",
            "1500k",
            "-maxrate",
            "1500k",
            "-bufsize",
            "1500k",
            "-g",
            "20",
            "-bf",
            "0",
            "-payload_type",
            "96",
            "-ssrc",
            "1868722033",
            "-flush_packets",
            "1",
            "-f",
            "rtp",
            RTP_URL,
        ]
    }

    /// Writes one complete native I420 generation to the shared encoder.
    ///
    /// # Errors
    ///
    /// Returns the pipe error when the `FFmpeg` child no longer accepts input.
    pub fn publish(&mut self, frame: &PublishedFrame) -> io::Result<()> {
        self.publish_step(frame, false)
    }

    pub(crate) fn repeat(&mut self, frame: &PublishedFrame) -> io::Result<()> {
        self.publish_step(frame, true)
    }

    fn publish_step(&mut self, frame: &PublishedFrame, repeat: bool) -> io::Result<()> {
        if let (Some((correlation, stream_epoch)), Some(metadata)) =
            (&self.correlation, frame.metadata())
        {
            let _ = correlation.submit(
                *stream_epoch,
                metadata,
                frame.generation(),
                crate::pipeline::unix_time_us(),
                repeat,
            );
        }
        self.stdin
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "FFmpeg input is closed"))?
            .write_all(frame.data())
    }

    /// Closes input and waits for the child, primarily at bounded verification seams.
    ///
    /// # Errors
    ///
    /// Returns a wait error or an error when `FFmpeg` exits unsuccessfully.
    pub fn finish(mut self) -> io::Result<()> {
        self.stdin.take();
        let status = self
            .child
            .take()
            .ok_or_else(|| io::Error::other("FFmpeg child is unavailable"))?
            .wait()?;
        self.finish_helpers();
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "FFmpeg exited with status {status}"
            )))
        }
    }

    fn finish_helpers(&mut self) {
        self.observer.take();
        if let Some(stderr) = self.stderr.take() {
            let _ = stderr.join();
        }
    }
}

impl Drop for FfmpegEncoder {
    fn drop(&mut self) {
        self.stdin.take();
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.finish_helpers();
    }
}

#[derive(Debug)]
struct RtpObserver {
    stop: Arc<AtomicBool>,
    threads: Vec<thread::JoinHandle<()>>,
}

impl RtpObserver {
    fn start(
        correlation: CorrelationState,
        stream_epoch: u64,
        pts_receiver: Receiver<u64>,
        evidence_valid: &Arc<AtomicBool>,
    ) -> io::Result<Self> {
        let media_socket = bound_socket(RTP_INPUT_ADDRESS)?;
        let control_socket = bound_socket(RTCP_INPUT_ADDRESS)?;
        let media_relay = parse_address(RTP_RELAY_ADDRESS)?;
        let control_relay = parse_address(RTCP_RELAY_ADDRESS)?;
        let stop = Arc::new(AtomicBool::new(false));
        let observed_packets = Arc::new(AtomicU32::new(0));
        let (rtp_sender, rtp_receiver) = mpsc::sync_channel(128);
        let (report_sender, report_receiver) = mpsc::sync_channel(4);
        let media_stop = Arc::clone(&stop);
        let media_evidence = Arc::clone(evidence_valid);
        let media_packet_count = Arc::clone(&observed_packets);
        let media_thread = thread::Builder::new()
            .name("obscam-rtp-observer".into())
            .spawn(move || {
                relay_rtp(
                    &media_socket,
                    media_relay,
                    &rtp_sender,
                    &media_stop,
                    &media_evidence,
                    &media_packet_count,
                );
            })?;
        let control_stop = Arc::clone(&stop);
        let control_evidence = Arc::clone(evidence_valid);
        let control_packet_count = Arc::clone(&observed_packets);
        let control_thread = thread::Builder::new()
            .name("obscam-rtcp-relay".into())
            .spawn(move || {
                relay_rtcp(
                    &control_socket,
                    control_relay,
                    &report_sender,
                    &control_stop,
                    &control_evidence,
                    &control_packet_count,
                );
            })?;
        let correlate_stop = Arc::clone(&stop);
        let correlate_evidence = Arc::clone(evidence_valid);
        let correlate_thread = thread::Builder::new()
            .name("obscam-rtp-correlation".into())
            .spawn(move || {
                pair_timestamps(
                    &correlation,
                    stream_epoch,
                    &pts_receiver,
                    &rtp_receiver,
                    &report_receiver,
                    &correlate_stop,
                    &correlate_evidence,
                );
            })?;
        Ok(Self {
            stop,
            threads: vec![media_thread, control_thread, correlate_thread],
        })
    }
}

impl Drop for RtpObserver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

fn spawn_timestamp_reader(
    stderr: ChildStderr,
    sender: SyncSender<u64>,
    evidence_valid: Arc<AtomicBool>,
) -> io::Result<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("obscam-ffmpeg-timestamps".into())
        .spawn(move || read_timestamps(stderr, &sender, &evidence_valid))
}

fn read_timestamps(stderr: impl Read, sender: &SyncSender<u64>, evidence_valid: &AtomicBool) {
    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
        if let Some(pts) = parse_muxer_pts(&line) {
            if sender.try_send(pts).is_err() {
                evidence_valid.store(false, Ordering::Release);
                return;
            }
        } else if line.starts_with("muxer <- type:video ") {
            evidence_valid.store(false, Ordering::Release);
            return;
        } else if line.to_ascii_lowercase().contains("error") {
            tracing::warn!(message = line, "FFmpeg diagnostic");
        }
    }
}

fn parse_muxer_pts(line: &str) -> Option<u64> {
    let fields = line.strip_prefix("muxer <- type:video ")?;
    let value = fields.strip_prefix("pkt_pts:")?.split_whitespace().next()?;
    value.parse().ok()
}

fn pair_timestamps(
    correlation: &CorrelationState,
    stream_epoch: u64,
    pts_receiver: &Receiver<u64>,
    rtp_receiver: &Receiver<u32>,
    report_receiver: &Receiver<(u32, u32)>,
    stop: &AtomicBool,
    evidence_valid: &AtomicBool,
) {
    let mut timeline_proven = false;
    while !stop.load(Ordering::Acquire) {
        if !evidence_valid.load(Ordering::Acquire) {
            tracing::warn!("incomplete RTP evidence; correlation stopped for stream epoch");
            return;
        }
        if timeline_proven {
            while report_receiver.try_recv().is_ok() {}
        } else {
            match report_receiver.recv_timeout(Duration::from_millis(100)) {
                Ok((sender_packets, observed_packets))
                    if sender_packets > 0 && sender_packets == observed_packets =>
                {
                    timeline_proven = true;
                }
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
        let pts = match pts_receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(pts) => pts,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        };
        let Ok(rtp_timestamp) = rtp_receiver.recv_timeout(Duration::from_secs(1)) else {
            tracing::warn!(
                pts,
                "RTP marker evidence missing; correlation stopped for stream epoch"
            );
            return;
        };
        if pts % RTP_CLOCK_STEP != 0 {
            tracing::warn!(pts, "fractional FFmpeg output timestamp is not correlated");
            continue;
        }
        let input_index = pts / RTP_CLOCK_STEP;
        if let Err(error) = correlation.observe(stream_epoch, input_index, rtp_timestamp) {
            tracing::warn!(%error, input_index, rtp_timestamp, "RTP frame correlation is unknown");
        }
    }
}

fn relay_rtp(
    socket: &UdpSocket,
    relay: SocketAddr,
    timestamps: &SyncSender<u32>,
    stop: &AtomicBool,
    evidence_valid: &AtomicBool,
    observed_packets: &AtomicU32,
) {
    let mut packet = [0_u8; 2_048];
    let mut previous_sequence = None;
    while !stop.load(Ordering::Acquire) {
        let Ok(length) = socket.recv(&mut packet) else {
            continue;
        };
        let Some((sequence, timestamp)) = rtp_packet(&packet[..length]) else {
            evidence_valid.store(false, Ordering::Release);
            continue;
        };
        if previous_sequence.is_some_and(|previous: u16| sequence != previous.wrapping_add(1)) {
            evidence_valid.store(false, Ordering::Release);
        }
        previous_sequence = Some(sequence);
        observed_packets.fetch_add(1, Ordering::AcqRel);
        let _ = socket.send_to(&packet[..length], relay);
        if let Some(timestamp) = timestamp
            && timestamps.try_send(timestamp).is_err()
        {
            evidence_valid.store(false, Ordering::Release);
            return;
        }
    }
}

fn relay_rtcp(
    socket: &UdpSocket,
    relay: SocketAddr,
    reports: &SyncSender<(u32, u32)>,
    stop: &AtomicBool,
    evidence_valid: &AtomicBool,
    observed_packets: &AtomicU32,
) {
    let mut packet = [0_u8; 2_048];
    while !stop.load(Ordering::Acquire) {
        let Ok(length) = socket.recv(&mut packet) else {
            continue;
        };
        let Some(sender_packets) = sender_report_packet_count(&packet[..length]) else {
            evidence_valid.store(false, Ordering::Release);
            continue;
        };
        let _ = socket.send_to(&packet[..length], relay);
        let observed = observed_packets.load(Ordering::Acquire);
        if reports.try_send((sender_packets, observed)).is_err() {
            evidence_valid.store(false, Ordering::Release);
            return;
        }
    }
}

fn rtp_packet(packet: &[u8]) -> Option<(u16, Option<u32>)> {
    let header_length = 12 + usize::from(packet.first().copied()? & 0x0f) * 4;
    if packet.len() < 12
        || packet.len() < header_length
        || packet[0] >> 6 != 2
        || packet[1] & 0x7f != RTP_PAYLOAD_TYPE
        || u32::from_be_bytes(packet[8..12].try_into().ok()?) != RTP_SSRC
    {
        return None;
    }
    let sequence = u16::from_be_bytes(packet[2..4].try_into().ok()?);
    let timestamp = (packet[1] & 0x80 != 0)
        .then(|| u32::from_be_bytes(packet[4..8].try_into().expect("validated RTP header")));
    Some((sequence, timestamp))
}

fn sender_report_packet_count(packet: &[u8]) -> Option<u32> {
    let packet_length =
        (usize::from(u16::from_be_bytes(packet.get(2..4)?.try_into().ok()?)) + 1) * 4;
    if packet.len() < 28
        || packet.len() < packet_length
        || packet[0] >> 6 != 2
        || packet[1] != 200
        || u16::from_be_bytes(packet[2..4].try_into().ok()?) < 6
        || u32::from_be_bytes(packet[4..8].try_into().ok()?) != RTP_SSRC
    {
        return None;
    }
    Some(u32::from_be_bytes(packet[20..24].try_into().ok()?))
}

fn bound_socket(address: &str) -> io::Result<UdpSocket> {
    let socket = UdpSocket::bind(address)?;
    socket.set_read_timeout(Some(Duration::from_millis(100)))?;
    Ok(socket)
}

fn parse_address(address: &str) -> io::Result<SocketAddr> {
    address
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_direct_muxer_video_pts_evidence() {
        assert_eq!(
            parse_muxer_pts("muxer <- type:video pkt_pts:9000 pkt_pts_time:0.1"),
            Some(9_000)
        );
        assert_eq!(
            parse_muxer_pts("encoder -> type:video pkt_pts:2 pkt_pts_time:0.1"),
            None
        );
    }

    #[test]
    fn accepts_only_rtp_marker_packets() {
        let mut packet = [0_u8; 12];
        packet[0] = 0x80;
        packet[1] = 0x80 | 0x60;
        packet[2..4].copy_from_slice(&7_u16.to_be_bytes());
        packet[4..8].copy_from_slice(&55_000_u32.to_be_bytes());
        packet[8..12].copy_from_slice(&RTP_SSRC.to_be_bytes());
        assert_eq!(rtp_packet(&packet), Some((7, Some(55_000))));
        packet[1] &= 0x7f;
        assert_eq!(rtp_packet(&packet), Some((7, None)));
        packet[1] = 97;
        assert_eq!(rtp_packet(&packet), None);
        packet[0] = 0x81;
        assert_eq!(rtp_packet(&packet), None);
    }

    #[test]
    fn accepts_only_the_configured_rtcp_sender_report() {
        let mut packet = [0_u8; 28];
        packet[0] = 0x80;
        packet[1] = 200;
        packet[2..4].copy_from_slice(&6_u16.to_be_bytes());
        packet[4..8].copy_from_slice(&RTP_SSRC.to_be_bytes());
        packet[20..24].copy_from_slice(&123_u32.to_be_bytes());
        assert_eq!(sender_report_packet_count(&packet), Some(123));
        packet[4..8].copy_from_slice(&1_u32.to_be_bytes());
        assert_eq!(sender_report_packet_count(&packet), None);
    }
}
