use std::net::SocketAddr;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::mpsc as std_mpsc;
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

#[derive(Debug)]
pub struct RawFrame {
    pixels: RecycledPixels,
    pub source_generation: u64,
    pub settings_generation: u64,
    pub treatment: Treatment,
    pub exposure_completed_unix_ns: u128,
}

#[derive(Debug)]
struct RecycledPixels {
    bytes: Option<Vec<u8>>,
    recycler: std_mpsc::Sender<Vec<u8>>,
}

impl RecycledPixels {
    fn new(bytes: Vec<u8>, recycler: std_mpsc::Sender<Vec<u8>>) -> Self {
        Self {
            bytes: Some(bytes),
            recycler,
        }
    }
}

impl AsRef<[u8]> for RecycledPixels {
    fn as_ref(&self) -> &[u8] {
        self.bytes.as_deref().unwrap_or_default()
    }
}

impl Drop for RecycledPixels {
    fn drop(&mut self) {
        if let Some(bytes) = self.bytes.take() {
            let _ = self.recycler.send(bytes);
        }
    }
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
    sender: watch::Sender<Option<Arc<RawFrame>>>,
    state: ProbeState,
    fps: u32,
) {
    let (recycle_tx, recycle_rx) = std_mpsc::channel();
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
        let mut pixels = take_frame_buffer(&recycle_rx);
        synthetic_yuv420_into(generation, &mut pixels);
        let settings_generation = generation / 200 + 1;
        let treatment = if settings_generation.is_multiple_of(2) {
            Treatment::Colour
        } else {
            Treatment::Mono
        };
        let frame = Arc::new(RawFrame {
            pixels: RecycledPixels::new(pixels, recycle_tx.clone()),
            source_generation: generation,
            settings_generation,
            treatment,
            exposure_completed_unix_ns: unix_time_ns(),
        });
        sender.send_replace(Some(frame));
    }
}

pub async fn run_asi_source(
    sender: watch::Sender<Option<Arc<RawFrame>>>,
    state: ProbeState,
    mut commands: mpsc::Receiver<SourceCommand>,
    exposure_us: i64,
    gain: i64,
) -> Result<()> {
    tokio::task::spawn_blocking(move || -> Result<()> {
        let mut profile = CaptureProfile::new(exposure_us, gain, Treatment::Mono)?;
        let mut raw = vec![0_u8; WIDTH * HEIGHT];
        let (recycle_tx, recycle_rx) = std_mpsc::channel();
        let mut generation = 0_u64;
        let mut settings_generation = 0_u64;
        loop {
            if sender.is_closed() {
                return Ok(());
            }
            let mut camera = match open_camera(profile) {
                Ok(camera) => camera,
                Err(error) => {
                    warn!(%error, "ASI662MC unavailable; retrying exact camera ownership");
                    std::thread::sleep(Duration::from_millis(250));
                    continue;
                }
            };
            settings_generation = settings_generation
                .checked_add(1)
                .context("settings generation exhausted")?;
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
                "ASI662MC capture owner ready"
            );
            let result = run_camera_session(
                &mut camera,
                &sender,
                &state,
                &mut commands,
                &mut profile,
                &mut generation,
                &mut settings_generation,
                &mut raw,
                &recycle_tx,
                &recycle_rx,
            );
            match result {
                Ok(()) => return Ok(()),
                Err(error) => {
                    sender.send_replace(None);
                    warn!(%error, settings_generation, "camera owner failed; recovering component");
                    std::thread::sleep(Duration::from_millis(250));
                }
            }
        }
    })
    .await
    .context("joining ASI662MC capture thread")?
}

trait CameraDevice {
    fn configure(&self, exposure_us: i64, gain: i64) -> Result<()>;
    fn start(&mut self) -> Result<()>;
    fn stop(&mut self) -> Result<()>;
    fn capture(&self, buffer: &mut [u8]) -> Result<bool>;
}

impl CameraDevice for zwo_asi::Camera {
    fn configure(&self, exposure_us: i64, gain: i64) -> Result<()> {
        self.configure(exposure_us, gain).map_err(Into::into)
    }

    fn start(&mut self) -> Result<()> {
        self.start().map_err(Into::into)
    }

    fn stop(&mut self) -> Result<()> {
        self.stop().map_err(Into::into)
    }

    fn capture(&self, buffer: &mut [u8]) -> Result<bool> {
        match self.capture(buffer, 100) {
            Ok(()) => Ok(true),
            Err(zwo_asi::CameraError::Timeout) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }
}

fn open_camera(profile: CaptureProfile) -> Result<zwo_asi::Camera> {
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
    Ok(camera)
}

#[allow(clippy::too_many_arguments)]
fn run_camera_session<C: CameraDevice>(
    camera: &mut C,
    sender: &watch::Sender<Option<Arc<RawFrame>>>,
    state: &ProbeState,
    commands: &mut mpsc::Receiver<SourceCommand>,
    profile: &mut CaptureProfile,
    generation: &mut u64,
    settings_generation: &mut u64,
    raw: &mut [u8],
    recycle_tx: &std_mpsc::Sender<Vec<u8>>,
    recycle_rx: &std_mpsc::Receiver<Vec<u8>>,
) -> Result<()> {
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
                            .context("applying ASI662MC qualification profile")?;
                        camera.start().context("restarting ASI662MC capture")?;
                        *profile = requested;
                        *settings_generation = settings_generation
                            .checked_add(1)
                            .context("settings generation exhausted")?;
                        state.record_capture_started(
                            *settings_generation,
                            *generation + 1,
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
                            settings_generation: *settings_generation,
                            treatment: profile.treatment,
                        })
                    })();
                    match result {
                        Ok(applied) => {
                            let _ = reply.send(Ok(applied));
                        }
                        Err(error) => {
                            let message = error.to_string();
                            let _ = reply.send(Err(message));
                            return Err(error).context("applying capture profile");
                        }
                    }
                }
            }
        }
        if !camera
            .capture(raw)
            .context("capturing ASI662MC RAW8 frame")?
        {
            continue;
        }
        *generation = generation.wrapping_add(1);
        let mut pixels = take_frame_buffer(recycle_rx);
        match profile.treatment {
            Treatment::Mono => raw8_rggb_to_mono_yuv420_into(raw, WIDTH, HEIGHT, &mut pixels),
            Treatment::Colour => {
                raw8_rggb_to_colour_yuv420_into(raw, WIDTH, HEIGHT, &mut pixels);
            }
        }
        let frame = Arc::new(RawFrame {
            pixels: RecycledPixels::new(pixels, recycle_tx.clone()),
            source_generation: *generation,
            settings_generation: *settings_generation,
            treatment: profile.treatment,
            exposure_completed_unix_ns: unix_time_ns(),
        });
        if sender.send(Some(frame)).is_err() {
            return Ok(());
        }
        state.record_capture_started(
            *settings_generation,
            *generation + 1,
            profile.exposure_us,
            unix_time_ns(),
        );
    }
}

pub async fn run_encoder(
    mut receiver: watch::Receiver<Option<Arc<RawFrame>>>,
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
    receiver: &mut watch::Receiver<Option<Arc<RawFrame>>>,
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
                    let latest = receiver.borrow_and_update().clone();
                    match latest {
                        Some(frame) => {
                            submit_frame(&mut stdin, state, &frame).await?;
                            last_frame = Some(frame);
                            heartbeat.as_mut().reset(tokio::time::Instant::now() + ENCODER_HEARTBEAT_INTERVAL);
                        }
                        None => {
                            last_frame = None;
                        }
                    }
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
    if let Err(error) = stdin.write_all(frame.pixels.as_ref()).await {
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

fn take_frame_buffer(recycler: &std_mpsc::Receiver<Vec<u8>>) -> Vec<u8> {
    let mut frame = recycler
        .try_recv()
        .unwrap_or_else(|_| Vec::with_capacity(WIDTH * HEIGHT * 3 / 2));
    frame.resize(WIDTH * HEIGHT * 3 / 2, 128);
    frame
}

fn synthetic_yuv420_into(generation: u64, frame: &mut [u8]) {
    let y_len = WIDTH * HEIGHT;
    frame[y_len..].fill(128);
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
}

/// Reconstructs neutral full-resolution luminance from the ASI662MC's RGGB
/// mosaic without allocating or copying an intermediate RGB frame.
fn raw8_rggb_to_mono_yuv420_into(raw: &[u8], width: usize, height: usize, frame: &mut [u8]) {
    debug_assert_eq!(raw.len(), width * height);
    debug_assert!(width.is_multiple_of(2) && height.is_multiple_of(2));
    let y_len = width * height;
    debug_assert_eq!(frame.len(), y_len + y_len / 2);
    let (y_plane, chroma) = frame.split_at_mut(y_len);
    for x in 0..width {
        let (red, green, blue) = demosaic_rggb(raw, width, height, x, 0);
        y_plane[x] = rgb_to_y(red, green, blue);
        let (red, green, blue) = demosaic_rggb(raw, width, height, x, height - 1);
        y_plane[(height - 1) * width + x] = rgb_to_y(red, green, blue);
    }
    for y in 1..height - 1 {
        let (red, green, blue) = demosaic_rggb(raw, width, height, 0, y);
        y_plane[y * width] = rgb_to_y(red, green, blue);
        for x in 1..width - 1 {
            y_plane[y * width + x] = demosaiced_luma_rggb_interior(raw, width, x, y);
        }
        let (red, green, blue) = demosaic_rggb(raw, width, height, width - 1, y);
        y_plane[(y + 1) * width - 1] = rgb_to_y(red, green, blue);
    }
    chroma.fill(128);
}

#[inline]
fn demosaiced_luma_rggb_interior(raw: &[u8], width: usize, x: usize, y: usize) -> u8 {
    let index = y * width + x;
    let horizontal = u16::midpoint(u16::from(raw[index - 1]), u16::from(raw[index + 1]));
    let vertical = u16::midpoint(u16::from(raw[index - width]), u16::from(raw[index + width]));
    let cross = (u16::from(raw[index - 1])
        + u16::from(raw[index + 1])
        + u16::from(raw[index - width])
        + u16::from(raw[index + width]))
        / 4;
    let diagonal = (u16::from(raw[index - width - 1])
        + u16::from(raw[index - width + 1])
        + u16::from(raw[index + width - 1])
        + u16::from(raw[index + width + 1]))
        / 4;
    let centre = u16::from(raw[index]);
    let (red, green, blue) = match (y.is_multiple_of(2), x.is_multiple_of(2)) {
        (true, true) => (centre, cross, diagonal),
        (true, false) => (horizontal, centre, vertical),
        (false, true) => (vertical, centre, horizontal),
        (false, false) => (diagonal, cross, centre),
    };
    u8::try_from((77 * red + 150 * green + 29 * blue) >> 8).unwrap_or(u8::MAX)
}

/// Converts the ASI662MC's top-left-origin RGGB RAW8 mosaic to full-resolution
/// YUV420 using fixed bilinear interpolation and full-range BT.601 coefficients.
fn raw8_rggb_to_colour_yuv420_into(raw: &[u8], width: usize, height: usize, frame: &mut [u8]) {
    debug_assert_eq!(raw.len(), width * height);
    debug_assert!(width.is_multiple_of(2) && height.is_multiple_of(2));
    let y_len = width * height;
    debug_assert_eq!(frame.len(), y_len + y_len / 2);
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

    struct ScriptedCamera {
        capturing: bool,
        fail_configure: bool,
    }

    impl CameraDevice for ScriptedCamera {
        fn configure(&self, _exposure_us: i64, _gain: i64) -> Result<()> {
            if self.fail_configure {
                bail!("injected configure failure");
            }
            Ok(())
        }

        fn start(&mut self) -> Result<()> {
            self.capturing = true;
            Ok(())
        }

        fn stop(&mut self) -> Result<()> {
            self.capturing = false;
            Ok(())
        }

        fn capture(&self, _buffer: &mut [u8]) -> Result<bool> {
            Ok(false)
        }
    }

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
    fn production_mono_removes_detectable_bayer_grid() {
        let (width, height) = (8_usize, 8_usize);
        let raw = (0..height)
            .flat_map(|y| {
                (0..width).map(move |x| match (y.is_multiple_of(2), x.is_multiple_of(2)) {
                    (true, true) => 200,
                    (false, false) => 20,
                    _ => 100,
                })
            })
            .collect::<Vec<_>>();
        let mut mono = vec![0; width * height * 3 / 2];
        raw8_rggb_to_mono_yuv420_into(&raw, width, height, &mut mono);

        let expected_luma = rgb_to_y(200, 100, 20);
        assert!(
            mono[..width * height]
                .iter()
                .all(|&value| value == expected_luma)
        );
        assert!(mono[width * height..].iter().all(|&value| value == 128));
    }

    #[test]
    fn production_mono_preserves_full_resolution_edge_contrast() {
        let (width, height) = (8_usize, 8_usize);
        let raw = (0..height)
            .flat_map(|_| (0..width).map(|x| if x < width / 2 { 24 } else { 224 }))
            .collect::<Vec<_>>();
        let mut frame = vec![0; width * height * 3 / 2];

        raw8_rggb_to_mono_yuv420_into(&raw, width, height, &mut frame);

        assert_eq!(frame.len(), width * height * 3 / 2);
        for row in frame[..width * height].chunks_exact(width) {
            assert_eq!(row[0], 24);
            assert_eq!(row[width - 1], 224);
            assert!(row.windows(2).all(|pair| pair[0] <= pair[1]));
        }
    }

    #[test]
    fn production_mono_matches_reference_bilinear_demosaic() {
        let (width, height) = (12_usize, 10_usize);
        let raw = (0..width * height)
            .map(|index| u8::try_from((index * 73 + 19) % 256).expect("sample fits u8"))
            .collect::<Vec<_>>();
        let mut frame = vec![0; width * height * 3 / 2];

        raw8_rggb_to_mono_yuv420_into(&raw, width, height, &mut frame);

        for y in 0..height {
            for x in 0..width {
                let (red, green, blue) = demosaic_rggb(&raw, width, height, x, y);
                assert_eq!(frame[y * width + x], rgb_to_y(red, green, blue));
            }
        }
    }

    #[test]
    fn colour_conversion_preserves_dimensions_and_rggb_channels() {
        let raw = vec![
            200, 100, 200, 100, 100, 20, 100, 20, 200, 100, 200, 100, 100, 20, 100, 20,
        ];
        let mut frame = vec![0; 4 * 4 * 3 / 2];
        raw8_rggb_to_colour_yuv420_into(&raw, 4, 4, &mut frame);
        assert_eq!(frame.len(), 4 * 4 * 3 / 2);
        assert!(frame[16..20].iter().all(|&value| value < 128));
        assert!(frame[20..].iter().all(|&value| value > 128));
    }

    #[test]
    fn frame_buffer_returns_to_the_camera_owner_after_last_reader() {
        let (recycle_tx, recycle_rx) = std_mpsc::channel();
        let bytes = vec![0_u8; WIDTH * HEIGHT * 3 / 2];
        let pointer = bytes.as_ptr();
        drop(RecycledPixels::new(bytes, recycle_tx));
        let recycled = take_frame_buffer(&recycle_rx);
        assert_eq!(recycled.as_ptr(), pointer);
    }

    #[test]
    fn failed_profile_transition_stops_session_and_preserves_generation() {
        let mut camera = ScriptedCamera {
            capturing: true,
            fail_configure: true,
        };
        let (frame_tx, _) = watch::channel(None);
        let state = ProbeState::new("epoch".into(), 1);
        let (command_tx, mut command_rx) = mpsc::channel(1);
        let (reply_tx, reply_rx) = oneshot::channel();
        command_tx
            .blocking_send(SourceCommand::ApplyProfile {
                profile: CaptureProfile::new(50_000, 500, Treatment::Colour).unwrap(),
                reply: reply_tx,
            })
            .unwrap();
        let mut profile = CaptureProfile::new(50_000, 500, Treatment::Mono).unwrap();
        let mut generation = 10;
        let mut settings_generation = 3;
        let mut raw = vec![0_u8; WIDTH * HEIGHT];
        let (recycle_tx, recycle_rx) = std_mpsc::channel();
        let result = run_camera_session(
            &mut camera,
            &frame_tx,
            &state,
            &mut command_rx,
            &mut profile,
            &mut generation,
            &mut settings_generation,
            &mut raw,
            &recycle_tx,
            &recycle_rx,
        );
        assert!(result.is_err());
        assert!(!camera.capturing);
        assert_eq!(profile.treatment, Treatment::Mono);
        assert_eq!(settings_generation, 3);
        assert!(reply_rx.blocking_recv().unwrap().is_err());
    }

    #[test]
    fn encoder_heartbeat_preserves_source_identity() {
        let (recycle_tx, _) = std_mpsc::channel();
        let frame = RawFrame {
            pixels: RecycledPixels::new(vec![0_u8; 6], recycle_tx),
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
