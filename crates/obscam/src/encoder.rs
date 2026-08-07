use std::{
    fmt,
    io::{self, BufRead, BufReader, Read, Write},
    net::{SocketAddr, UdpSocket},
    path::Path,
    process::{Child, ChildStderr, ChildStdin, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender},
    },
    thread,
    time::Duration,
};

use crate::{PublishedFrame, correlation::CorrelationState};

const RTP_INPUT_ADDRESS: &str = "127.0.0.1:5002";
const RTCP_INPUT_ADDRESS: &str = "127.0.0.1:5003";
const RTP_RELAY_ADDRESS: &str = "169.254.218.2:5004";
const RTCP_RELAY_ADDRESS: &str = "169.254.218.2:5005";
const RELAY_BIND_ADDRESS: &str = "0.0.0.0:0";
const RTP_URL: &str = "rtp://127.0.0.1:5002?rtcpport=5003&pkt_size=1200";
const RTP_CLOCK_STEP: u64 = 4_500;
const RTP_PAYLOAD_TYPE: u8 = 96;
const RTP_SSRC: u32 = 1_868_722_033;
const RTP_INITIAL_SEQUENCE: u16 = 1_000;
const RTP_RECEIVE_BUFFER_BYTES: usize = 4 * 1_048_576;

/// One long-lived `FFmpeg` hardware-H.264 publication child.
#[derive(Debug)]
pub struct FfmpegEncoder {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    writer: Option<FrameWriter>,
    stderr: Option<thread::JoinHandle<()>>,
    observer: Option<RtpObserver>,
    correlation: Option<(CorrelationState, u64)>,
    evidence_valid: Arc<AtomicBool>,
    evidence_degradation_reported: bool,
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
        Self::spawn(program, None, false)
    }

    #[cfg(test)]
    fn start_owned(program: &Path) -> io::Result<Self> {
        Self::spawn(program, None, true)
    }

    pub(crate) fn start_correlated(
        program: &Path,
        correlation: CorrelationState,
    ) -> io::Result<Self> {
        let stream_epoch = correlation.begin_stream();
        Self::spawn(program, Some((correlation, stream_epoch)), true)
    }

    fn spawn(
        program: &Path,
        correlation: Option<(CorrelationState, u64)>,
        owned_writer: bool,
    ) -> io::Result<Self> {
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
        let (stdin, writer) = if owned_writer {
            (None, Some(FrameWriter::start(stdin)?))
        } else {
            (Some(stdin), None)
        };
        let stderr = if correlation.is_some() {
            let stderr = child
                .stderr
                .take()
                .ok_or_else(|| io::Error::other("FFmpeg stderr pipe was not created"))?;
            Some(spawn_timestamp_reader(
                stderr,
                pts_sender,
                Arc::clone(&evidence_valid),
            )?)
        } else {
            None
        };
        Ok(Self {
            child: Some(child),
            stdin,
            writer,
            stderr,
            observer,
            correlation,
            evidence_valid,
            evidence_degradation_reported: false,
        })
    }

    fn arguments() -> [&'static str; 39] {
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
            "-seq",
            "1000",
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

    pub(crate) fn publish_owned(
        &mut self,
        frame: PublishedFrame,
        repeat: bool,
        timeout: Duration,
    ) -> Result<PublishedFrame, OwnedPublicationError> {
        if let Err(error) = self.verify_running() {
            return Err(OwnedPublicationError::new(frame, error));
        }
        if let Some((correlation, stream_epoch)) = &self.correlation {
            record_submission(correlation, *stream_epoch, &frame, repeat);
        }
        let request = WriterRequest { frame };
        if let Err(error) = self
            .writer
            .as_ref()
            .expect("correlated encoder has a frame writer")
            .requests
            .as_ref()
            .expect("FFmpeg writer accepts requests")
            .send(request)
        {
            return Err(OwnedPublicationError::new(
                error.0.frame,
                io::Error::new(io::ErrorKind::BrokenPipe, "FFmpeg writer stopped"),
            ));
        }
        let response = self
            .writer
            .as_ref()
            .expect("correlated encoder has a frame writer")
            .responses
            .recv_timeout(timeout);
        match response {
            Ok(response) => match response.result {
                Ok(()) => Ok(response.frame),
                Err(error) => Err(OwnedPublicationError::new(response.frame, error)),
            },
            Err(RecvTimeoutError::Disconnected) => {
                panic!("FFmpeg writer disconnected while owning a frame")
            }
            Err(RecvTimeoutError::Timeout) => {
                self.kill_child();
                let frame = self.recover_writer_frame_after_stop();
                Err(OwnedPublicationError::new(
                    frame,
                    io::Error::new(io::ErrorKind::TimedOut, "FFmpeg publication timed out"),
                ))
            }
        }
    }

    pub(crate) fn verify_running(&mut self) -> io::Result<()> {
        if !self.evidence_valid.load(Ordering::Acquire) && !self.evidence_degradation_reported {
            tracing::warn!("FFmpeg correlation evidence became incomplete; media continues");
            self.evidence_degradation_reported = true;
        }
        match self
            .child
            .as_mut()
            .expect("FFmpeg child is present")
            .try_wait()
        {
            Ok(Some(status)) => Err(io::Error::other(format!(
                "FFmpeg exited with status {status}"
            ))),
            Ok(None) => Ok(()),
            Err(error) => Err(error),
        }
    }

    fn recover_writer_frame_after_stop(&self) -> PublishedFrame {
        self.writer
            .as_ref()
            .expect("correlated encoder has a frame writer")
            .responses
            .recv()
            .expect("FFmpeg writer returns its owned frame after child termination")
            .frame
    }

    fn kill_child(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn publish_step(&mut self, frame: &PublishedFrame, repeat: bool) -> io::Result<()> {
        if let Some((correlation, stream_epoch)) = &self.correlation {
            record_submission(correlation, *stream_epoch, frame, repeat);
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
        self.writer.take();
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

fn record_submission(
    correlation: &CorrelationState,
    stream_epoch: u64,
    frame: &PublishedFrame,
    repeat: bool,
) {
    if let Some(metadata) = frame.metadata() {
        let _ = correlation.submit(
            stream_epoch,
            metadata,
            frame.generation(),
            crate::pipeline::unix_time_us(),
            repeat,
        );
    } else {
        let _ = correlation.skip_submission(stream_epoch);
    }
}

impl Drop for FfmpegEncoder {
    fn drop(&mut self) {
        self.stdin.take();
        self.kill_child();
        self.child.take();
        self.writer.take();
        self.finish_helpers();
    }
}

pub(crate) struct OwnedPublicationError {
    frame: PublishedFrame,
    error: io::Error,
}

impl OwnedPublicationError {
    const fn new(frame: PublishedFrame, error: io::Error) -> Self {
        Self { frame, error }
    }

    pub(crate) fn into_parts(self) -> (PublishedFrame, io::Error) {
        (self.frame, self.error)
    }
}

struct WriterRequest {
    frame: PublishedFrame,
}

struct WriterResponse {
    frame: PublishedFrame,
    result: io::Result<()>,
}

struct FrameWriter {
    requests: Option<SyncSender<WriterRequest>>,
    responses: Receiver<WriterResponse>,
    thread: Option<thread::JoinHandle<()>>,
}

impl fmt::Debug for FrameWriter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FrameWriter")
            .finish_non_exhaustive()
    }
}

impl FrameWriter {
    fn start(mut stdin: ChildStdin) -> io::Result<Self> {
        let (request_sender, request_receiver) = mpsc::sync_channel::<WriterRequest>(0);
        let (response_sender, response_receiver) = mpsc::sync_channel(0);
        let thread = thread::Builder::new()
            .name("obscam-ffmpeg-input".into())
            .spawn(move || {
                while let Ok(request) = request_receiver.recv() {
                    let result = stdin.write_all(request.frame.data());
                    if response_sender
                        .send(WriterResponse {
                            frame: request.frame,
                            result,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })?;
        Ok(Self {
            requests: Some(request_sender),
            responses: response_receiver,
            thread: Some(thread),
        })
    }
}

impl Drop for FrameWriter {
    fn drop(&mut self) {
        self.requests.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
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
        let media_socket = bound_rtp_socket(RTP_INPUT_ADDRESS)?;
        let control_socket = bound_socket(RTCP_INPUT_ADDRESS)?;
        let media_relay_socket = relay_socket()?;
        let control_relay_socket = relay_socket()?;
        let media_relay = parse_address(RTP_RELAY_ADDRESS)?;
        let control_relay = parse_address(RTCP_RELAY_ADDRESS)?;
        let stop = Arc::new(AtomicBool::new(false));
        let (rtp_sender, rtp_receiver) = mpsc::sync_channel(128);
        let media_stop = Arc::clone(&stop);
        let media_evidence = Arc::clone(evidence_valid);
        let media_thread = thread::Builder::new()
            .name("obscam-rtp-observer".into())
            .spawn(move || {
                relay_rtp(
                    &media_socket,
                    &media_relay_socket,
                    media_relay,
                    &rtp_sender,
                    &media_stop,
                    &media_evidence,
                );
            })?;
        let control_stop = Arc::clone(&stop);
        let control_evidence = Arc::clone(evidence_valid);
        let control_thread = thread::Builder::new()
            .name("obscam-rtcp-relay".into())
            .spawn(move || {
                relay_rtcp(
                    &control_socket,
                    &control_relay_socket,
                    control_relay,
                    &control_stop,
                    &control_evidence,
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
    let mut last_pts = None;
    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
        if let Some(pts) = parse_muxer_pts(&line) {
            if last_pts == Some(pts) {
                continue;
            }
            last_pts = Some(pts);
            if sender.try_send(pts).is_err() {
                invalidate_evidence(evidence_valid, "FFmpeg PTS queue unavailable");
                return;
            }
        } else if is_video_muxer_line(&line) {
            invalidate_evidence(evidence_valid, "unparseable FFmpeg video muxer timestamp");
            return;
        } else if line.to_ascii_lowercase().contains("error") {
            tracing::warn!(message = line, "FFmpeg diagnostic");
        }
    }
}

fn parse_muxer_pts(line: &str) -> Option<u64> {
    let value = if let Some(fields) = line.strip_prefix("muxer <- type:video ") {
        fields.strip_prefix("pkt_pts:")?.split_whitespace().next()?
    } else {
        let _ = line.strip_prefix("[vost#")?;
        let (_, fields) = line.split_once("] muxer <- ")?;
        fields.strip_prefix("pts:")?.split_whitespace().next()?
    };
    value.parse().ok()
}

fn is_video_muxer_line(line: &str) -> bool {
    line.starts_with("muxer <- type:video ")
        || (line.starts_with("[vost#") && line.contains("] muxer <- "))
}

fn pair_timestamps(
    correlation: &CorrelationState,
    stream_epoch: u64,
    pts_receiver: &Receiver<u64>,
    rtp_receiver: &Receiver<u32>,
    stop: &AtomicBool,
    evidence_valid: &AtomicBool,
) {
    while !stop.load(Ordering::Acquire) {
        if !evidence_valid.load(Ordering::Acquire) {
            tracing::warn!("incomplete RTP evidence; correlation stopped for stream epoch");
            return;
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
            if error == crate::CorrelationError::Evicted {
                tracing::debug!(
                    input_index,
                    rtp_timestamp,
                    "RTP frame has no retained exact input metadata"
                );
            } else {
                tracing::warn!(%error, input_index, rtp_timestamp, "RTP frame correlation is unknown");
                invalidate_evidence(evidence_valid, "correlation timeline rejected RTP marker");
                return;
            }
        }
    }
}

fn relay_rtp(
    socket: &UdpSocket,
    relay_socket: &UdpSocket,
    relay: SocketAddr,
    timestamps: &SyncSender<u32>,
    stop: &AtomicBool,
    evidence_valid: &AtomicBool,
) {
    let mut packet = [0_u8; 2_048];
    let mut previous_sequence = Some(RTP_INITIAL_SEQUENCE.wrapping_sub(1));
    let mut relay_available = true;
    while !stop.load(Ordering::Acquire) {
        let Ok(length) = socket.recv(&mut packet) else {
            continue;
        };
        let Some((sequence, timestamp)) = rtp_packet(&packet[..length]) else {
            invalidate_evidence(evidence_valid, "invalid RTP packet");
            continue;
        };
        let expected_sequence = previous_sequence
            .expect("RTP sequence is initialized")
            .wrapping_add(1);
        if sequence != expected_sequence
            && invalidate_evidence(evidence_valid, "RTP sequence discontinuity")
        {
            tracing::warn!(expected_sequence, sequence, "RTP packet sequence skipped");
        }
        previous_sequence = Some(sequence);
        if !forward_packet(
            relay_socket,
            &packet[..length],
            relay,
            "RTP",
            &mut relay_available,
            evidence_valid,
        ) {
            continue;
        }
        if let Some(timestamp) = timestamp {
            submit_evidence(timestamps, timestamp, evidence_valid);
        }
    }
}

fn relay_rtcp(
    socket: &UdpSocket,
    relay_socket: &UdpSocket,
    relay: SocketAddr,
    stop: &AtomicBool,
    evidence_valid: &AtomicBool,
) {
    let mut packet = [0_u8; 2_048];
    let mut relay_available = true;
    while !stop.load(Ordering::Acquire) {
        let Ok(length) = socket.recv(&mut packet) else {
            continue;
        };
        let Some(_sender_packets) = sender_report_packet_count(&packet[..length]) else {
            invalidate_evidence(evidence_valid, "invalid RTCP sender report");
            continue;
        };
        let _ = forward_packet(
            relay_socket,
            &packet[..length],
            relay,
            "RTCP",
            &mut relay_available,
            evidence_valid,
        );
    }
}

fn submit_evidence<T>(sender: &SyncSender<T>, value: T, evidence_valid: &AtomicBool) {
    if evidence_valid.load(Ordering::Acquire) && sender.try_send(value).is_err() {
        invalidate_evidence(evidence_valid, "RTP marker queue unavailable");
    }
}

fn invalidate_evidence(evidence_valid: &AtomicBool, reason: &'static str) -> bool {
    let was_valid = evidence_valid.swap(false, Ordering::AcqRel);
    if was_valid {
        tracing::warn!(reason, "exact correlation evidence invalidated");
    }
    was_valid
}

fn forward_packet(
    socket: &UdpSocket,
    packet: &[u8],
    relay: SocketAddr,
    protocol: &'static str,
    available: &mut bool,
    evidence_valid: &AtomicBool,
) -> bool {
    match socket.send_to(packet, relay) {
        Ok(_) => {
            if relay_transition(available, true) == RelayTransition::Recovered {
                tracing::info!(protocol, %relay, "media relay recovered");
            }
            true
        }
        Err(error) => {
            invalidate_evidence(evidence_valid, "media relay send failed");
            if relay_transition(available, false) == RelayTransition::Lost {
                tracing::warn!(protocol, %error, %relay, "media relay unavailable; retrying");
            }
            false
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RelayTransition {
    Stable,
    Lost,
    Recovered,
}

fn relay_transition(available: &mut bool, succeeded: bool) -> RelayTransition {
    let transition = match (*available, succeeded) {
        (true, false) => RelayTransition::Lost,
        (false, true) => RelayTransition::Recovered,
        _ => RelayTransition::Stable,
    };
    *available = succeeded;
    transition
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

fn bound_rtp_socket(address: &str) -> io::Result<UdpSocket> {
    let socket = bound_socket(address)?;
    socket2::SockRef::from(&socket).set_recv_buffer_size(RTP_RECEIVE_BUFFER_BYTES)?;
    Ok(socket)
}

fn relay_socket() -> io::Result<UdpSocket> {
    UdpSocket::bind(RELAY_BIND_ADDRESS)
}

fn parse_address(address: &str) -> io::Result<SocketAddr> {
    address
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn hung_ffmpeg_input_is_terminated_and_returns_the_owned_frame() {
        use std::{fs, os::unix::fs::PermissionsExt};

        use crate::{LatestFrameMailbox, MonochromeProcessor};
        use uuid::Uuid;
        use zwo_asi::{CameraSource, DeterministicCamera, DeterministicScenario, Settings};

        let directory = std::env::temp_dir().join(format!("obscam-ffmpeg-hang-{}", Uuid::new_v4()));
        fs::create_dir(&directory).expect("create test directory");
        let program = directory.join("fake-ffmpeg");
        fs::write(&program, "#!/bin/sh\nexec sleep 10\n").expect("write fake FFmpeg");
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).expect("make executable");
        let mut camera =
            DeterministicCamera::connect(DeterministicScenario::new([])).expect("camera present");
        camera
            .configure(Settings::new(50_000, 0).expect("settings"))
            .expect("configure");
        camera.start().expect("start");
        let source = camera.capture_next(100).expect("source generation");
        let mut processor = MonochromeProcessor::new();
        let mailbox = LatestFrameMailbox::new();
        mailbox.publish(&processor.process(&source));
        let frame = mailbox.take().expect("processed generation");
        let generation = frame.generation();
        let mut encoder = FfmpegEncoder::start_owned(&program).expect("start fake FFmpeg");

        let Err(failure) = encoder.publish_owned(frame, false, Duration::from_millis(50)) else {
            panic!("blocked input must time out");
        };
        let (frame, error) = failure.into_parts();

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(frame.generation(), generation);
        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[cfg(unix)]
    #[test]
    fn exited_ffmpeg_is_detected_without_waiting_for_a_source_frame() {
        use std::{fs, os::unix::fs::PermissionsExt};

        use uuid::Uuid;

        let directory = std::env::temp_dir().join(format!("obscam-ffmpeg-exit-{}", Uuid::new_v4()));
        fs::create_dir(&directory).expect("create test directory");
        let program = directory.join("fake-ffmpeg");
        fs::write(&program, "#!/bin/sh\nexit 17\n").expect("write fake FFmpeg");
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).expect("make executable");
        let mut encoder = FfmpegEncoder::start_owned(&program).expect("start fake FFmpeg");
        thread::sleep(Duration::from_millis(20));

        let error = encoder
            .verify_running()
            .expect_err("exited child must be observed without a frame");

        assert!(error.to_string().contains("17"));
        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[cfg(unix)]
    #[test]
    fn incomplete_correlation_evidence_does_not_stop_healthy_media() {
        use std::{fs, os::unix::fs::PermissionsExt};

        use uuid::Uuid;

        let directory =
            std::env::temp_dir().join(format!("obscam-ffmpeg-evidence-{}", Uuid::new_v4()));
        fs::create_dir(&directory).expect("create test directory");
        let program = directory.join("fake-ffmpeg");
        fs::write(&program, "#!/bin/sh\nexec sleep 10\n").expect("write fake FFmpeg");
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).expect("make executable");
        let mut encoder = FfmpegEncoder::start_owned(&program).expect("start fake FFmpeg");
        encoder.evidence_valid.store(false, Ordering::Release);

        encoder
            .verify_running()
            .expect("correlation may degrade without stopping a healthy encoder");
        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[test]
    fn parses_only_direct_muxer_video_pts_evidence() {
        assert_eq!(
            parse_muxer_pts("muxer <- type:video pkt_pts:9000 pkt_pts_time:0.1"),
            Some(9_000)
        );
        assert_eq!(
            parse_muxer_pts(
                "[vost#0:0/h264_v4l2m2m @ 0x1234] muxer <- pts:4500 pts_time:0.05 dts:4500"
            ),
            Some(4_500)
        );
        assert_eq!(
            parse_muxer_pts("encoder -> type:video pkt_pts:2 pkt_pts_time:0.1"),
            None
        );
        assert_eq!(
            parse_muxer_pts("[aost#0:0/aac @ 0x1234] muxer <- pts:4500 pts_time:0.05"),
            None
        );
    }

    #[test]
    fn timestamp_reader_deduplicates_multiple_muxer_packets_for_one_frame() {
        let diagnostics = concat!(
            "[vost#0:0/h264_v4l2m2m @ 0x1] muxer <- pts:4500 pts_time:0.05\n",
            "[vost#0:0/h264_v4l2m2m @ 0x1] muxer <- pts:4500 pts_time:0.05\n",
            "[vost#0:0/h264_v4l2m2m @ 0x1] muxer <- pts:9000 pts_time:0.1\n"
        );
        let (sender, receiver) = mpsc::sync_channel(4);
        let evidence_valid = AtomicBool::new(true);

        read_timestamps(diagnostics.as_bytes(), &sender, &evidence_valid);
        drop(sender);

        assert_eq!(receiver.iter().collect::<Vec<_>>(), [4_500, 9_000]);
        assert!(evidence_valid.load(Ordering::Acquire));
    }

    #[test]
    fn fixed_initial_sequence_and_direct_markers_establish_correlation() {
        use crate::{Treatment, correlation::CapturedFrameMetadata};
        use uuid::Uuid;

        let correlation = CorrelationState::new(Uuid::from_u128(1));
        let stream_epoch = correlation.begin_stream();
        correlation
            .submit(
                stream_epoch,
                CapturedFrameMetadata {
                    settings_generation: 3,
                    treatment: Treatment::Monochrome,
                    exposure_completed_at_unix_us: 1,
                },
                7,
                2,
                false,
            )
            .expect("current stream accepts input metadata");
        let mut updates = correlation.subscribe();
        let (pts_sender, pts_receiver) = mpsc::sync_channel(1);
        let (rtp_sender, rtp_receiver) = mpsc::sync_channel(1);
        pts_sender.send(0).expect("PTS evidence");
        rtp_sender.send(90_000).expect("RTP marker evidence");
        drop(pts_sender);
        drop(rtp_sender);

        pair_timestamps(
            &correlation,
            stream_epoch,
            &pts_receiver,
            &rtp_receiver,
            &AtomicBool::new(false),
            &AtomicBool::new(true),
        );

        let mapping = updates
            .try_recv()
            .expect("fixed-sequence evidence publishes a mapping");
        assert_eq!(mapping.source_generation(), 7);
        assert_eq!(mapping.rtp_timestamp(), 90_000);
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

    #[test]
    fn relay_socket_leaves_source_selection_to_the_private_veth_route() {
        let address = parse_address(RELAY_BIND_ADDRESS).expect("relay bind address");

        assert!(address.ip().is_unspecified());
        assert_eq!(address.port(), 0);
    }

    #[test]
    fn rtp_input_socket_requests_capacity_for_bursty_full_resolution_frames() {
        let default_socket = UdpSocket::bind("127.0.0.1:0").expect("bind default UDP socket");
        let default_capacity = socket2::SockRef::from(&default_socket)
            .recv_buffer_size()
            .expect("read default UDP receive buffer");
        let socket = bound_rtp_socket("127.0.0.1:0").expect("bind RTP input socket");
        let configured_capacity = socket2::SockRef::from(&socket)
            .recv_buffer_size()
            .expect("read RTP receive buffer");

        assert!(
            configured_capacity >= RTP_RECEIVE_BUFFER_BYTES
                || configured_capacity > default_capacity,
            "kernel neither granted the {RTP_RECEIVE_BUFFER_BYTES}-byte RTP target nor increased \
             the default receive buffer ({default_capacity} -> {configured_capacity})"
        );
    }

    #[test]
    fn relay_loss_is_retried_and_recovery_is_reported_once() {
        let mut available = true;

        assert_eq!(
            relay_transition(&mut available, false),
            RelayTransition::Lost
        );
        assert_eq!(
            relay_transition(&mut available, false),
            RelayTransition::Stable
        );
        assert_eq!(
            relay_transition(&mut available, true),
            RelayTransition::Recovered
        );
        assert_eq!(
            relay_transition(&mut available, true),
            RelayTransition::Stable
        );
    }

    #[test]
    fn disconnected_correlation_channel_cannot_stop_media_relay() {
        let (sender, receiver) = mpsc::sync_channel(1);
        drop(receiver);
        let evidence_valid = AtomicBool::new(true);

        submit_evidence(&sender, 1_u32, &evidence_valid);
        assert!(!evidence_valid.load(Ordering::Acquire));
        submit_evidence(&sender, 2_u32, &evidence_valid);
    }
}
