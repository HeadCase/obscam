use std::net::SocketAddr;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use tokio::io::AsyncWriteExt;
use tokio::net::UdpSocket;
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, watch};
use tracing::{debug, info, warn};

use crate::contract::{SubmittedFrame, Treatment};
use crate::state::{MAX_PENDING_SUBMISSIONS, ProbeState};

const WIDTH: usize = 1920;
const HEIGHT: usize = 1080;
const ENCODER_HEARTBEAT_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Clone, Debug)]
pub struct RawFrame {
    pub pixels: Arc<[u8]>,
    pub source_generation: u64,
    pub settings_generation: u64,
    pub treatment: Treatment,
    pub exposure_completed_unix_ns: u128,
}

/// Complete capture and presentation treatment applied as one generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureProfile {
    exposure_us: i64,
    gain: i64,
    treatment: Treatment,
}

impl CaptureProfile {
    /// Validates a complete capture profile before camera capture is interrupted.
    pub fn new(exposure_us: i64, gain: i64, treatment: Treatment) -> Result<Self> {
        if !(10_000..=30_000_000).contains(&exposure_us) {
            bail!("exposure_us must be between 10000 and 30000000");
        }
        if !(0..=600).contains(&gain) {
            bail!("gain must be between 0 and 600");
        }
        Ok(Self {
            exposure_us,
            gain,
            treatment,
        })
    }
}

/// Commands serialized through the sole camera owner.
#[derive(Debug)]
pub enum SourceCommand {
    ApplyProfile {
        profile: CaptureProfile,
        reply: oneshot::Sender<Result<AppliedProfile, String>>,
    },
}

/// Acknowledges the generation fence established by an applied profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppliedProfile {
    pub settings_generation: u64,
    pub treatment: Treatment,
}

#[derive(Debug)]
pub enum EncoderCommand {
    Restart {
        reply: oneshot::Sender<Result<u64, String>>,
    },
}

pub async fn run_synthetic_source(
    sender: watch::Sender<Option<RawFrame>>,
    state: ProbeState,
    fps: u32,
) {
    let mut interval = tokio::time::interval(Duration::from_secs_f64(1.0 / f64::from(fps)));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut generation = 0_u64;
    loop {
        interval.tick().await;
        state.record_capture_started(
            (generation + 1) / 200 + 1,
            generation + 1,
            i64::from(1_000_000 / fps),
            unix_time_ns(),
        );
        generation = generation.wrapping_add(1);
        let pixels = synthetic_yuv420(generation);
        let settings_generation = generation / 200 + 1;
        let treatment = if settings_generation.is_multiple_of(2) {
            Treatment::Colour
        } else {
            Treatment::Mono
        };
        let frame = RawFrame {
            pixels: pixels.into(),
            source_generation: generation,
            settings_generation,
            treatment,
            exposure_completed_unix_ns: unix_time_ns(),
        };
        sender.send_replace(Some(frame));
    }
}

pub async fn run_asi_source(
    sender: watch::Sender<Option<RawFrame>>,
    state: ProbeState,
    mut commands: mpsc::Receiver<SourceCommand>,
    exposure_us: i64,
    gain: i64,
) -> Result<()> {
    tokio::task::spawn_blocking(move || -> Result<()> {
        let mut profile = CaptureProfile::new(exposure_us, gain, Treatment::Mono)?;
        let mut camera = zwo_asi::Camera::open_exact("ZWO ASI662MC", "1d274e0920010900")
            .context("opening exact ObsCam camera")?;
        let (width, height) = camera.dimensions();
        if (width, height) != (WIDTH, HEIGHT) {
            bail!("ASI662MC reported unexpected dimensions {width}x{height}");
        }
        camera
            .configure(profile.exposure_us, profile.gain)
            .context("configuring ASI662MC")?;
        camera.start().context("starting ASI662MC video capture")?;
        state.record_capture_started(1, 1, profile.exposure_us, unix_time_ns());
        info!(width, height, exposure_us, gain, "ASI662MC capture started");
        let mut raw = vec![0_u8; width * height];
        let mut generation = 0_u64;
        let mut settings_generation = 1_u64;
        loop {
            while let Ok(command) = commands.try_recv() {
                match command {
                    SourceCommand::ApplyProfile {
                        profile: requested,
                        reply,
                    } => {
                        let result = (|| -> Result<AppliedProfile> {
                            camera.stop().context("interrupting ASI662MC capture")?;
                            camera
                                .configure(requested.exposure_us, requested.gain)
                                .context("applying ASI662MC qualification settings")?;
                            camera.start().context("restarting ASI662MC capture")?;
                            profile = requested;
                            settings_generation = settings_generation.wrapping_add(1);
                            state.record_capture_started(
                                settings_generation,
                                generation + 1,
                                profile.exposure_us,
                                unix_time_ns(),
                            );
                            info!(
                                exposure_us = profile.exposure_us,
                                gain = profile.gain,
                                treatment = ?profile.treatment,
                                settings_generation,
                                "qualification capture profile applied"
                            );
                            Ok(AppliedProfile {
                                settings_generation,
                                treatment: profile.treatment,
                            })
                        })()
                        .map_err(|error| error.to_string());
                        let _ = reply.send(result);
                    }
                }
            }
            match camera.capture(&mut raw, 100) {
                Ok(()) => {}
                Err(zwo_asi::CameraError::Timeout) => continue,
                Err(error) => {
                    return Err(error).context("capturing ASI662MC RAW8 frame");
                }
            }
            generation = generation.wrapping_add(1);
            let frame = RawFrame {
                pixels: match profile.treatment {
                    Treatment::Mono => raw8_to_mono_yuv420(&raw),
                    Treatment::Colour => raw8_rggb_to_colour_yuv420(&raw, width, height),
                }
                .into(),
                source_generation: generation,
                settings_generation,
                treatment: profile.treatment,
                exposure_completed_unix_ns: unix_time_ns(),
            };
            if sender.send(Some(frame)).is_err() {
                return Ok(());
            }
            state.record_capture_started(
                settings_generation,
                generation + 1,
                profile.exposure_us,
                unix_time_ns(),
            );
        }
    })
    .await
    .context("joining ASI662MC capture thread")?
}

pub async fn run_encoder(
    mut receiver: watch::Receiver<Option<RawFrame>>,
    state: ProbeState,
    fps: u32,
    rtp_destination: SocketAddr,
    mut commands: mpsc::Receiver<EncoderCommand>,
) -> Result<()> {
    loop {
        match run_encoder_session(&mut receiver, &state, fps, rtp_destination, &mut commands).await
        {
            EncoderSessionEnd::Restart(reply) => {
                let stream_epoch = state.begin_stream_epoch();
                let _ = reply.send(Ok(stream_epoch));
                info!(stream_epoch, "qualification encoder restart requested");
            }
            EncoderSessionEnd::Failed(error) => {
                let stream_epoch = state.begin_stream_epoch();
                warn!(%error, stream_epoch, "encoder failed; replacing component");
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
    }
}

enum EncoderSessionEnd {
    Restart(oneshot::Sender<Result<u64, String>>),
    Failed(anyhow::Error),
}

async fn run_encoder_session(
    receiver: &mut watch::Receiver<Option<RawFrame>>,
    state: &ProbeState,
    fps: u32,
    rtp_destination: SocketAddr,
    commands: &mut mpsc::Receiver<EncoderCommand>,
) -> EncoderSessionEnd {
    let mut child = match spawn_ffmpeg(fps, rtp_destination) {
        Ok(child) => child,
        Err(error) => return EncoderSessionEnd::Failed(error),
    };
    let Some(mut stdin) = child.stdin.take() else {
        return EncoderSessionEnd::Failed(anyhow::anyhow!("FFmpeg stdin was not piped"));
    };
    let writer = async {
        let mut last_frame = None;
        let heartbeat = tokio::time::sleep(ENCODER_HEARTBEAT_INTERVAL);
        tokio::pin!(heartbeat);
        loop {
            tokio::select! {
                changed = receiver.changed() => {
                    changed.context("frame source stopped")?;
                    let Some(frame) = receiver.borrow_and_update().clone() else {
                        continue;
                    };
                    submit_frame(&mut stdin, state, &frame).await?;
                    last_frame = Some(frame);
                    heartbeat.as_mut().reset(tokio::time::Instant::now() + ENCODER_HEARTBEAT_INTERVAL);
                }
                () = &mut heartbeat, if last_frame.is_some() => {
                    let frame = last_frame.as_ref().expect("heartbeat requires a completed frame");
                    submit_frame(&mut stdin, state, frame).await?;
                    heartbeat.as_mut().reset(tokio::time::Instant::now() + ENCODER_HEARTBEAT_INTERVAL);
                }
            }
        }
        #[allow(unreachable_code)]
        Ok::<(), anyhow::Error>(())
    };
    tokio::select! {
        result = writer => EncoderSessionEnd::Failed(
            result.expect_err("encoder writer never completes successfully")
        ),
        status = child.wait() => {
            let error = match status {
                Ok(status) => anyhow::anyhow!("FFmpeg exited unexpectedly with {status}"),
                Err(error) => anyhow::Error::new(error).context("waiting for FFmpeg"),
            };
            EncoderSessionEnd::Failed(error)
        }
        command = commands.recv() => {
            match command {
                Some(EncoderCommand::Restart { reply }) => EncoderSessionEnd::Restart(reply),
                None => EncoderSessionEnd::Failed(anyhow::anyhow!("encoder command channel closed")),
            }
        }
    }
}

async fn submit_frame(
    stdin: &mut tokio::process::ChildStdin,
    state: &ProbeState,
    frame: &RawFrame,
) -> Result<()> {
    if !state.record_submission(submission_for(frame, unix_time_ns())) {
        bail!("encoder correlation queue exceeded its fail-closed bound");
    }
    if let Err(error) = stdin.write_all(&frame.pixels).await {
        state.abandon_submissions();
        return Err(error).context("writing RAW8-derived YUV420 frame to FFmpeg");
    }
    Ok(())
}

fn submission_for(frame: &RawFrame, submitted_unix_ns: u128) -> SubmittedFrame {
    SubmittedFrame {
        source_generation: frame.source_generation,
        settings_generation: frame.settings_generation,
        treatment: frame.treatment,
        exposure_completed_unix_ns: frame.exposure_completed_unix_ns,
        submitted_unix_ns,
    }
}

pub async fn run_rtp_observer(
    listen: SocketAddr,
    forward: SocketAddr,
    state: ProbeState,
    fps: u32,
) -> Result<()> {
    let socket = UdpSocket::bind(listen)
        .await
        .with_context(|| format!("binding RTP observer at {listen}"))?;
    let forwarder = UdpSocket::bind("0.0.0.0:0")
        .await
        .context("binding RTP forwarding socket")?;
    info!(%listen, %forward, "RTP observer ready");
    let mut packet = vec![0_u8; 2048];
    let mut last_timestamp = None;
    let mut current_stream_epoch = state.stream_epoch();
    if fps == 0 || !90_000_u32.is_multiple_of(fps) {
        bail!("FPS must be a non-zero exact divisor of the 90 kHz RTP clock");
    }
    let expected_step = 90_000_u32 / fps;
    let mut observed_frames = 0_u64;
    let mut skipped_encoder_inputs = 0_u64;
    loop {
        let (length, _) = socket
            .recv_from(&mut packet)
            .await
            .context("receiving RTP packet")?;
        let bytes = &packet[..length];
        let observed_stream_epoch = state.stream_epoch();
        if observed_stream_epoch != current_stream_epoch {
            current_stream_epoch = observed_stream_epoch;
            last_timestamp = None;
            observed_frames = 0;
        }
        if let Some(timestamp) = rtp_timestamp(bytes)
            && last_timestamp != Some(timestamp)
        {
            let skipped = match last_timestamp {
                Some(previous) => match skipped_rtp_inputs(previous, timestamp, expected_step) {
                    Ok(skipped) => skipped,
                    Err(error) => {
                        state.abandon_submissions();
                        return Err(error).context("validating RTP input timeline");
                    }
                },
                None => 0,
            };
            if skipped > 0 {
                skipped_encoder_inputs = skipped_encoder_inputs.wrapping_add(skipped as u64);
                warn!(
                    skipped,
                    skipped_encoder_inputs,
                    previous_rtp_timestamp = last_timestamp,
                    rtp_timestamp = timestamp,
                    "hardware encoder skipped submitted inputs"
                );
            }
            let mapping = state.record_rtp_timestamp(timestamp, skipped);
            if skipped > 0 && mapping.is_none() {
                state.abandon_submissions();
                bail!("RTP gap exceeded available submitted-input timeline");
            }
            if let Some(mapping) = mapping {
                observed_frames += 1;
                if observed_frames == 1 || observed_frames.is_multiple_of(200) {
                    info!(
                        rtp_timestamp = timestamp,
                        generation = mapping.source_generation,
                        observed_frames,
                        "observed encoded frame"
                    );
                } else {
                    debug!(
                        rtp_timestamp = timestamp,
                        generation = mapping.source_generation
                    );
                }
            } else {
                warn!(
                    rtp_timestamp = timestamp,
                    "RTP frame has no pending source mapping"
                );
            }
            last_timestamp = Some(timestamp);
        }
        forwarder
            .send_to(bytes, forward)
            .await
            .with_context(|| format!("forwarding RTP packet to {forward}"))?;
    }
}

fn skipped_rtp_inputs(previous: u32, current: u32, expected_step: u32) -> Result<usize> {
    let delta = current.wrapping_sub(previous);
    if delta == 0 || !delta.is_multiple_of(expected_step) {
        bail!("RTP timestamp delta {delta} is not a whole {expected_step}-tick input step");
    }
    let input_steps = delta / expected_step;
    if input_steps as usize > MAX_PENDING_SUBMISSIONS {
        bail!(
            "RTP timestamp gap spans {input_steps} inputs, exceeding the bounded correlation queue"
        );
    }
    Ok(input_steps as usize - 1)
}

fn spawn_ffmpeg(fps: u32, destination: SocketAddr) -> Result<Child> {
    let destination = format!("rtp://{destination}?pkt_size=1200");
    let child = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "warning",
            "-f",
            "rawvideo",
            "-pixel_format",
            "yuv420p",
            "-video_size",
            "1920x1080",
            "-framerate",
            &fps.to_string(),
            "-i",
            "pipe:0",
            "-an",
            "-c:v",
            "h264_v4l2m2m",
            "-b:v",
            "1500000",
            "-g",
            &fps.to_string(),
            "-bf",
            "0",
            "-fps_mode",
            "passthrough",
            "-payload_type",
            "96",
            "-ssrc",
            "1337",
            "-f",
            "rtp",
            &destination,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .context("starting FFmpeg h264_v4l2m2m")?;
    Ok(child)
}

fn rtp_timestamp(packet: &[u8]) -> Option<u32> {
    if packet.len() < 12 || packet[0] >> 6 != 2 {
        return None;
    }
    Some(u32::from_be_bytes(packet[4..8].try_into().ok()?))
}

fn synthetic_yuv420(generation: u64) -> Vec<u8> {
    let y_len = WIDTH * HEIGHT;
    let mut frame = vec![128_u8; y_len + y_len / 2];
    let band = usize::try_from(generation % 240).expect("generation remainder fits usize");
    for y in 0..HEIGHT {
        let row = &mut frame[y * WIDTH..(y + 1) * WIDTH];
        for (x, value) in row.iter_mut().enumerate() {
            let gradient = u8::try_from((x * 180) / WIDTH).expect("gradient fits u8");
            let pulse = if (x / 48 + y / 48 + band / 12) % 2 == 0 {
                36
            } else {
                0
            };
            *value = 24_u8.saturating_add(gradient).saturating_add(pulse);
        }
    }
    frame
}

fn raw8_to_mono_yuv420(raw: &[u8]) -> Vec<u8> {
    let y_len = WIDTH * HEIGHT;
    let mut frame = vec![128_u8; y_len + y_len / 2];
    frame[..y_len].copy_from_slice(raw);
    frame
}

/// Converts the ASI662MC's top-left-origin RGGB RAW8 mosaic to full-resolution
/// YUV420 using fixed bilinear interpolation and full-range BT.601 coefficients.
fn raw8_rggb_to_colour_yuv420(raw: &[u8], width: usize, height: usize) -> Vec<u8> {
    debug_assert_eq!(raw.len(), width * height);
    debug_assert!(width.is_multiple_of(2) && height.is_multiple_of(2));
    let y_len = width * height;
    let mut frame = vec![0_u8; y_len + y_len / 2];
    let (y_plane, chroma) = frame.split_at_mut(y_len);
    let (u_plane, v_plane) = chroma.split_at_mut(y_len / 4);
    for y in (0..height).step_by(2) {
        for x in (0..width).step_by(2) {
            let mut red = 0_u16;
            let mut green = 0_u16;
            let mut blue = 0_u16;
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let rgb = demosaic_rggb(raw, width, height, x + dx, y + dy);
                y_plane[(y + dy) * width + x + dx] = rgb_to_y(rgb.0, rgb.1, rgb.2);
                red += u16::from(rgb.0);
                green += u16::from(rgb.1);
                blue += u16::from(rgb.2);
            }
            let chroma_index = (y / 2) * (width / 2) + x / 2;
            let rgb = (
                u8::try_from(red / 4).unwrap_or(u8::MAX),
                u8::try_from(green / 4).unwrap_or(u8::MAX),
                u8::try_from(blue / 4).unwrap_or(u8::MAX),
            );
            u_plane[chroma_index] = rgb_to_u(rgb.0, rgb.1, rgb.2);
            v_plane[chroma_index] = rgb_to_v(rgb.0, rgb.1, rgb.2);
        }
    }
    frame
}

fn demosaic_rggb(raw: &[u8], width: usize, height: usize, x: usize, y: usize) -> (u8, u8, u8) {
    let at = |dx: isize, dy: isize| {
        let reflect = |position: usize, delta: isize, limit: usize| {
            let position = isize::try_from(position).unwrap_or_default() + delta;
            let maximum = isize::try_from(limit - 1).unwrap_or_default();
            let reflected = if position < 0 {
                -position
            } else if position > maximum {
                2 * maximum - position
            } else {
                position
            };
            usize::try_from(reflected).unwrap_or_default()
        };
        let sample_x = reflect(x, dx, width);
        let sample_y = reflect(y, dy, height);
        raw[sample_y * width + sample_x]
    };
    let average = |offsets: &[(isize, isize)]| {
        let sum = offsets
            .iter()
            .map(|&(dx, dy)| u16::from(at(dx, dy)))
            .sum::<u16>();
        u8::try_from(sum / u16::try_from(offsets.len()).unwrap_or(1)).unwrap_or(u8::MAX)
    };
    let horizontal = [(-1, 0), (1, 0)];
    let vertical = [(0, -1), (0, 1)];
    let cross = [(-1, 0), (1, 0), (0, -1), (0, 1)];
    let diagonal = [(-1, -1), (1, -1), (-1, 1), (1, 1)];
    match (y.is_multiple_of(2), x.is_multiple_of(2)) {
        (true, true) => (at(0, 0), average(&cross), average(&diagonal)),
        (true, false) => (average(&horizontal), at(0, 0), average(&vertical)),
        (false, true) => (average(&vertical), at(0, 0), average(&horizontal)),
        (false, false) => (average(&diagonal), average(&cross), at(0, 0)),
    }
}

fn rgb_to_y(red: u8, green: u8, blue: u8) -> u8 {
    let value = 77 * u16::from(red) + 150 * u16::from(green) + 29 * u16::from(blue);
    u8::try_from(value >> 8).unwrap_or(u8::MAX)
}

fn rgb_to_u(red: u8, green: u8, blue: u8) -> u8 {
    clamp_colour((-43 * i32::from(red) - 85 * i32::from(green) + 128 * i32::from(blue)) / 256 + 128)
}

fn rgb_to_v(red: u8, green: u8, blue: u8) -> u8 {
    clamp_colour((128 * i32::from(red) - 107 * i32::from(green) - 21 * i32::from(blue)) / 256 + 128)
}

fn clamp_colour(value: i32) -> u8 {
    u8::try_from(value.clamp(0, 255)).unwrap_or_default()
}

fn unix_time_ns() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_nanos()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rtp_timestamp() {
        let packet = [0x80, 96, 0, 1, 0x12, 0x34, 0x56, 0x78, 0, 0, 0, 1];
        assert_eq!(rtp_timestamp(&packet), Some(0x1234_5678));
    }

    #[test]
    fn rejects_non_rtp_packet() {
        assert_eq!(rtp_timestamp(&[0; 12]), None);
    }

    #[test]
    fn capture_profiles_are_bounded_to_the_production_envelope() {
        assert!(CaptureProfile::new(10_000, 0, Treatment::Mono).is_ok());
        assert!(CaptureProfile::new(30_000_000, 600, Treatment::Colour).is_ok());
        assert!(CaptureProfile::new(9_999, 100, Treatment::Mono).is_err());
        assert!(CaptureProfile::new(30_000_001, 100, Treatment::Mono).is_err());
        assert!(CaptureProfile::new(10_000, 601, Treatment::Mono).is_err());
    }

    #[test]
    fn mono_conversion_preserves_luma_and_neutral_chroma() {
        let raw = vec![37_u8; WIDTH * HEIGHT];
        let frame = raw8_to_mono_yuv420(&raw);
        assert_eq!(&frame[..WIDTH * HEIGHT], raw);
        assert!(frame[WIDTH * HEIGHT..].iter().all(|&value| value == 128));
    }

    #[test]
    fn colour_conversion_preserves_dimensions_and_rggb_channels() {
        let raw = vec![
            200, 100, 200, 100, 100, 20, 100, 20, 200, 100, 200, 100, 100, 20, 100, 20,
        ];
        let frame = raw8_rggb_to_colour_yuv420(&raw, 4, 4);
        assert_eq!(frame.len(), 4 * 4 * 3 / 2);
        assert!(frame[16..20].iter().all(|&value| value < 128));
        assert!(frame[20..].iter().all(|&value| value > 128));
    }

    #[test]
    fn encoder_heartbeat_preserves_source_identity() {
        let frame = RawFrame {
            pixels: vec![0_u8; 6].into(),
            source_generation: 42,
            settings_generation: 7,
            treatment: Treatment::Mono,
            exposure_completed_unix_ns: 123,
        };

        let first = submission_for(&frame, 1_000);
        let heartbeat = submission_for(&frame, 1_500);

        assert_eq!(first.source_generation, heartbeat.source_generation);
        assert_eq!(first.settings_generation, heartbeat.settings_generation);
        assert_eq!(first.treatment, heartbeat.treatment);
        assert_eq!(
            first.exposure_completed_unix_ns,
            heartbeat.exposure_completed_unix_ns
        );
        assert_ne!(first.submitted_unix_ns, heartbeat.submitted_unix_ns);
        assert!(ENCODER_HEARTBEAT_INTERVAL <= Duration::from_millis(500));
    }

    #[test]
    fn rtp_gap_counts_only_whole_bounded_input_steps() {
        assert_eq!(skipped_rtp_inputs(1_000, 10_000, 4_500).unwrap(), 1);
        assert!(skipped_rtp_inputs(1_000, 7_750, 4_500).is_err());
        let oversized_gap = u32::try_from(MAX_PENDING_SUBMISSIONS).unwrap() + 1;
        assert!(skipped_rtp_inputs(1_000, 1_000 + 4_500 * oversized_gap, 4_500).is_err());
    }
}
