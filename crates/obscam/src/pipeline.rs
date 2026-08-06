use std::{
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream},
    path::Path,
    sync::{Arc, Condvar, Mutex, mpsc},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use zwo_asi::{CameraOwner, CameraSource, CaptureError, HEIGHT, WIDTH};

#[cfg(feature = "camera-substitute")]
use zwo_asi::{DeterministicCamera, DeterministicScenario};

use crate::{
    CameraRecoveryBackoff, ColourProcessor, ComponentReadiness, FfmpegEncoder, LatestFrameMailbox,
    MonochromeProcessor, RuntimeState, Treatment, ValidationFailure, ValidationWindow,
    correlation::CapturedFrameMetadata,
    latest::{EpochFence, LatestBufferMailbox},
    recovery::CaptureWatchdogMonitor,
    validate_processing_output,
};

const RAW8_BYTES: usize = WIDTH * HEIGHT;
const MAX_ENCODER_RECOVERY_DELAY: Duration = Duration::from_secs(5);
const ENCODER_PUBLICATION_TIMEOUT: Duration = Duration::from_secs(2);
const ENCODER_PUBLICATION_INTERVAL: Duration = Duration::from_millis(50);
const RELAY_METRICS_ADDRESS: &str = "169.254.218.2:9998";
const RELAY_PROBE_INTERVAL: Duration = Duration::from_millis(500);
const RELAY_PROBE_TIMEOUT: Duration = Duration::from_millis(250);
const CAPTURE_INTERRUPT_POLL_MS: i32 = 25;
const MAX_SENSOR_TRANSITION_FRAMES: u8 = 3;

#[derive(Debug, Default)]
struct SettingsFrameTrust {
    untrusted_remaining: u8,
}

impl SettingsFrameTrust {
    const fn begin_transition(&mut self, untrusted_frames: u8) {
        self.untrusted_remaining = untrusted_frames;
    }

    const fn permits_capture_progress(&self) -> bool {
        self.untrusted_remaining == 0
    }

    const fn permits_exact_metadata(&mut self) -> bool {
        if self.untrusted_remaining == 0 {
            true
        } else {
            self.untrusted_remaining -= 1;
            false
        }
    }
}

#[derive(Debug, Default)]
struct EncoderRecoveryBackoff {
    failures: u32,
}

impl EncoderRecoveryBackoff {
    const fn new() -> Self {
        Self { failures: 0 }
    }

    fn next_delay_with_jitter(&mut self, jitter_ms: u64) -> Duration {
        let delay = if self.failures == 0 {
            Duration::ZERO
        } else {
            Duration::from_millis(250_u64.saturating_mul(1_u64 << (self.failures - 1).min(5)))
                .min(MAX_ENCODER_RECOVERY_DELAY)
        };
        self.failures = self.failures.saturating_add(1);
        if delay.is_zero() {
            return delay;
        }
        delay
            .saturating_add(Duration::from_millis(jitter_ms))
            .min(MAX_ENCODER_RECOVERY_DELAY)
    }

    const fn reset(&mut self) {
        self.failures = 0;
    }
}

fn encoder_recovery_jitter_ms() -> u64 {
    u64::from(getrandom::u32().unwrap_or(0) % 251)
}

fn camera_recovery_jitter_ms() -> i64 {
    i64::from(getrandom::u32().unwrap_or(0) % 501) - 250
}

#[derive(Debug)]
struct ProcessingGate {
    state: Mutex<ProcessingGateState>,
    changed: Condvar,
}

#[derive(Debug)]
struct ProcessingGateState {
    paused: bool,
    active: bool,
    reset_requested: bool,
}

impl ProcessingGate {
    fn paused() -> Self {
        Self {
            state: Mutex::new(ProcessingGateState {
                paused: true,
                active: false,
                reset_requested: false,
            }),
            changed: Condvar::new(),
        }
    }

    fn enter(&self) -> ProcessingPermit<'_> {
        let mut state = self.state.lock().expect("processing gate mutex poisoned");
        while state.paused {
            state = self
                .changed
                .wait(state)
                .expect("processing gate mutex poisoned while paused");
        }
        assert!(!state.active, "processing gate has one worker");
        state.active = true;
        ProcessingPermit { gate: self }
    }

    fn pause_and_wait(&self) {
        let mut state = self.state.lock().expect("processing gate mutex poisoned");
        state.paused = true;
        while state.active {
            state = self
                .changed
                .wait(state)
                .expect("processing gate mutex poisoned while draining");
        }
    }

    fn resume(&self) {
        self.state
            .lock()
            .expect("processing gate mutex poisoned")
            .paused = false;
        self.changed.notify_all();
    }

    fn request_reset(&self) {
        self.state
            .lock()
            .expect("processing gate mutex poisoned")
            .reset_requested = true;
    }

    fn take_reset(&self) -> bool {
        let mut state = self.state.lock().expect("processing gate mutex poisoned");
        std::mem::take(&mut state.reset_requested)
    }
}

struct ProcessingPermit<'a> {
    gate: &'a ProcessingGate,
}

impl Drop for ProcessingPermit<'_> {
    fn drop(&mut self) {
        let mut state = self
            .gate
            .state
            .lock()
            .expect("processing gate mutex poisoned");
        state.active = false;
        self.gate.changed.notify_all();
    }
}

/// Detached continuously warm capture, processing, and publication workers.
pub struct MediaPipeline {
    _capture: thread::JoinHandle<()>,
    _processing: thread::JoinHandle<()>,
    _encoder: thread::JoinHandle<()>,
    _relay: thread::JoinHandle<()>,
}

impl MediaPipeline {
    /// Starts the production camera owner and the sole hardware encoder.
    ///
    /// # Errors
    ///
    /// Returns an operating-system error when a worker thread cannot be created.
    pub fn start(runtime: RuntimeState) -> io::Result<Self> {
        Self::start_with_source("obscam-capture", runtime, None, CameraOwner::connect)
    }

    /// Starts the explicit development/acceptance camera substitute.
    ///
    /// # Errors
    ///
    /// Returns an operating-system error when a worker thread cannot be created.
    #[cfg(feature = "camera-substitute")]
    pub fn start_deterministic(runtime: RuntimeState) -> io::Result<Self> {
        Self::start_with_source(
            "obscam-deterministic-capture",
            runtime,
            Some(Duration::from_millis(50)),
            || DeterministicCamera::connect(DeterministicScenario::new([])),
        )
    }

    fn start_with_source<S>(
        capture_thread_name: &str,
        runtime: RuntimeState,
        minimum_capture_interval: Option<Duration>,
        connect: impl Fn() -> Result<S, zwo_asi::CameraError> + Send + 'static,
    ) -> io::Result<Self>
    where
        S: CameraSource,
    {
        let epoch_fence = EpochFence::new();
        let raw = Arc::new(LatestBufferMailbox::with_epoch_fence(
            RAW8_BYTES,
            epoch_fence.clone(),
        ));
        let processed = Arc::new(LatestFrameMailbox::with_epoch_fence(epoch_fence));
        let processing_gate = Arc::new(ProcessingGate::paused());
        let (processing_recovery, recovery_requests) = mpsc::channel();
        let watchdog = CaptureWatchdogMonitor::spawn()?;
        let encoder = spawn_encoder(Arc::clone(&processed), runtime.clone())?;
        let relay = spawn_relay_observer(runtime.clone())?;
        let processing = spawn_processing(
            Arc::clone(&raw),
            Arc::clone(&processed),
            Arc::clone(&processing_gate),
            processing_recovery,
            runtime.clone(),
        )?;
        let capture = thread::Builder::new()
            .name(capture_thread_name.into())
            .spawn(move || {
                supervise_capture(
                    connect,
                    &runtime,
                    &raw,
                    &processing_gate,
                    &recovery_requests,
                    minimum_capture_interval,
                    &watchdog,
                );
            })?;
        Ok(Self {
            _capture: capture,
            _processing: processing,
            _encoder: encoder,
            _relay: relay,
        })
    }
}

fn spawn_relay_observer(runtime: RuntimeState) -> io::Result<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("obscam-relay-observer".into())
        .spawn(move || {
            let mut previous = None;
            loop {
                let ready = probe_relay();
                runtime.set_relay_readiness(if ready {
                    ComponentReadiness::Ready
                } else {
                    ComponentReadiness::Unavailable
                });
                if previous != Some(ready) {
                    if ready {
                        tracing::info!(
                            "MediaMTX relay path is ready; RTP publication is available"
                        );
                    } else {
                        tracing::warn!(
                            "MediaMTX relay path is unavailable; RTP publication continues"
                        );
                    }
                    previous = Some(ready);
                }
                thread::sleep(RELAY_PROBE_INTERVAL);
            }
        })
}

fn probe_relay() -> bool {
    let Ok(address) = RELAY_METRICS_ADDRESS.parse::<SocketAddr>() else {
        return false;
    };
    let Ok(mut stream) = TcpStream::connect_timeout(&address, RELAY_PROBE_TIMEOUT) else {
        return false;
    };
    if stream.set_read_timeout(Some(RELAY_PROBE_TIMEOUT)).is_err()
        || stream.set_write_timeout(Some(RELAY_PROBE_TIMEOUT)).is_err()
        || stream
            .write_all(
                b"GET /metrics?type=paths&path=obscam HTTP/1.1\r\nHost: mediamtx\r\nConnection: close\r\n\r\n",
            )
            .is_err()
    {
        return false;
    }
    let mut response = [0_u8; 4_096];
    let mut length = 0;
    loop {
        match stream.read(&mut response[length..]) {
            Ok(0) => return relay_path_is_ready(&response[..length]),
            Ok(read) => {
                length += read;
                if relay_path_is_ready(&response[..length]) {
                    return true;
                }
                if length == response.len() {
                    return false;
                }
            }
            Err(_) => return false,
        }
    }
}

fn relay_path_is_ready(response: &[u8]) -> bool {
    let Some(boundary) = response.windows(4).position(|window| window == b"\r\n\r\n") else {
        return false;
    };
    if !response.starts_with(b"HTTP/1.1 200 ") {
        return false;
    }
    response[boundary + 4..]
        .split(|byte| *byte == b'\n')
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
        .any(|line| line == b"paths{name=\"obscam\",state=\"ready\"} 1")
}

fn supervise_capture<S>(
    connect: impl Fn() -> Result<S, zwo_asi::CameraError>,
    runtime: &RuntimeState,
    raw: &LatestBufferMailbox,
    processing_gate: &ProcessingGate,
    processing_recovery: &mpsc::Receiver<()>,
    minimum_capture_interval: Option<Duration>,
    watchdog: &CaptureWatchdogMonitor,
) where
    S: CameraSource,
{
    let mut backoff = CameraRecoveryBackoff::new();
    let mut recovering = false;
    let mut source_generation = 0_u64;
    loop {
        thread::sleep(backoff.next_delay_with_jitter(camera_recovery_jitter_ms()));
        let mut source = match connect() {
            Ok(source) => source,
            Err(error) => {
                if error.ownership_uncertain() {
                    tracing::error!(%error, "camera connection cleanup failed; terminating");
                    std::process::exit(70);
                }
                runtime.begin_camera_recovery();
                processing_gate.pause_and_wait();
                tracing::error!(%error, "camera source unavailable; retrying");
                recovering = true;
                continue;
            }
        };
        let settings = runtime.settings();
        settings.install_interrupter(source.interrupter());
        let applied = settings.snapshot().applied().settings();
        if let Err(error) = source
            .configure(applied.camera_settings())
            .and_then(|()| source.start())
        {
            runtime.begin_camera_recovery();
            processing_gate.pause_and_wait();
            tracing::error!(%error, "camera capture could not start; retrying");
            let teardown_token = watchdog.arm(Duration::ZERO, source.interrupter());
            if let Err(close_error) = source.close() {
                tracing::error!(%close_error, "camera startup cleanup failed; terminating");
                std::process::exit(70);
            }
            watchdog.complete(teardown_token);
            recovering = true;
            continue;
        }
        runtime.require_source_generation(source_generation.saturating_add(1));
        runtime.complete_camera_recovery();
        processing_gate.resume();
        if recovering {
            runtime.record_camera_restart();
            tracing::info!("camera backend reopened and exact identity revalidated");
        }

        let mut context = CaptureSessionContext {
            runtime,
            raw,
            processing_gate,
            processing_recovery,
            minimum_capture_interval,
            watchdog,
            backoff: &mut backoff,
            source_generation: &mut source_generation,
        };
        let end = capture_session(&mut source, &mut context);
        runtime.begin_camera_recovery();
        let teardown_token = end
            .watchdog_token
            .unwrap_or_else(|| watchdog.arm(Duration::ZERO, source.interrupter()));
        if let Err(error) = source.stop() {
            tracing::error!(%error, "camera stop failed during recovery; terminating");
            std::process::exit(70);
        }
        processing_gate.pause_and_wait();
        runtime.record_pipeline_skips(raw.begin_new_epoch());
        if let Err(error) = source.close() {
            tracing::error!(%error, "camera close failed during recovery; terminating");
            std::process::exit(70);
        }
        watchdog.complete(teardown_token);
        recovering = true;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CaptureSessionEnd {
    watchdog_token: Option<u64>,
}

struct CaptureSessionContext<'a> {
    runtime: &'a RuntimeState,
    raw: &'a LatestBufferMailbox,
    processing_gate: &'a ProcessingGate,
    processing_recovery: &'a mpsc::Receiver<()>,
    minimum_capture_interval: Option<Duration>,
    watchdog: &'a CaptureWatchdogMonitor,
    backoff: &'a mut CameraRecoveryBackoff,
    source_generation: &'a mut u64,
}

#[allow(
    clippy::too_many_lines,
    reason = "the single linear camera-owner state machine keeps recovery ordering auditable"
)]
fn capture_session(
    source: &mut impl CameraSource,
    context: &mut CaptureSessionContext<'_>,
) -> CaptureSessionEnd {
    let runtime = context.runtime;
    let raw = context.raw;
    let processing_gate = context.processing_gate;
    let processing_recovery = context.processing_recovery;
    let minimum_capture_interval = context.minimum_capture_interval;
    let watchdog = context.watchdog;
    let backoff = &mut *context.backoff;
    let source_generation = &mut *context.source_generation;
    let settings = runtime.settings();
    let mut capture_in_progress = false;
    let mut watchdog_token = None;
    let session_started = Instant::now();
    let mut validation = ValidationWindow::new(Duration::from_secs(10));
    let mut last_generation = 0;
    let mut settings_frame_trust = SettingsFrameTrust::default();

    loop {
        if processing_recovery.try_recv().is_ok() {
            if let Some(token) = watchdog_token
                && watchdog.cancellation_requested(token)
            {
                return CaptureSessionEnd {
                    watchdog_token: watchdog_token.take(),
                };
            }
            processing_gate.pause_and_wait();
            runtime.record_pipeline_skips(raw.begin_new_epoch());
            processing_gate.request_reset();
            runtime.require_source_generation(source_generation.saturating_add(1));
            processing_gate.resume();
            tracing::info!("processing component recovered without restarting capture");
            continue;
        }
        match apply_pending_settings(source, &settings, |generation| {
            raw.begin_epoch(generation);
        }) {
            Ok(SettingsTransition::Applied { untrusted_frames }) => {
                settings_frame_trust.begin_transition(untrusted_frames);
                if let Some(token) = watchdog_token.take() {
                    if watchdog.cancellation_requested(token) {
                        return CaptureSessionEnd {
                            watchdog_token: Some(token),
                        };
                    }
                    watchdog.complete(token);
                }
                capture_in_progress = false;
                continue;
            }
            Ok(SettingsTransition::Idle) => {}
            Ok(SettingsTransition::Restored(error)) => {
                if let Some(token) = watchdog_token.take() {
                    if watchdog.cancellation_requested(token) {
                        return CaptureSessionEnd {
                            watchdog_token: Some(token),
                        };
                    }
                    watchdog.complete(token);
                }
                capture_in_progress = false;
                tracing::warn!(%error, "camera settings transition failed; applied tuple restored");
            }
            Err(restore_error) => {
                tracing::error!(%restore_error, "camera settings recovery failed");
                return CaptureSessionEnd {
                    watchdog_token: watchdog_token.take(),
                };
            }
        }
        if !capture_in_progress {
            let applied = settings.snapshot().applied();
            if settings_frame_trust.permits_capture_progress() {
                runtime.capture_started(
                    applied.generation(),
                    applied.settings().exposure_ms(),
                    unix_time_us(),
                );
            }
            watchdog_token = Some(watchdog.arm(
                Duration::from_millis(u64::from(applied.settings().exposure_ms())),
                source.interrupter(),
            ));
            capture_in_progress = true;
        }
        let capture_wait_started = Instant::now();
        let epoch = raw.current_epoch();
        match source.capture_next(CAPTURE_INTERRUPT_POLL_MS) {
            Ok(frame) => {
                let token = watchdog_token.expect("watchdog armed");
                if watchdog.cancellation_requested(token) {
                    return CaptureSessionEnd {
                        watchdog_token: Some(token),
                    };
                }
                if frame.generation() <= last_generation {
                    if validation_failure(
                        runtime,
                        &mut validation,
                        ValidationFailure::InvalidGeneration,
                        session_started.elapsed(),
                    ) {
                        return CaptureSessionEnd {
                            watchdog_token: watchdog_token.take(),
                        };
                    }
                    watchdog.complete(watchdog_token.take().expect("watchdog armed"));
                    capture_in_progress = false;
                    continue;
                }
                let applied = settings.snapshot().applied();
                let metadata =
                    settings_frame_trust
                        .permits_exact_metadata()
                        .then(|| CapturedFrameMetadata {
                            settings_generation: applied.generation(),
                            treatment: applied.settings().treatment(),
                            exposure_completed_at_unix_us: unix_time_us(),
                        });
                *source_generation = source_generation.saturating_add(1);
                let skipped =
                    raw.publish_with_metadata(epoch, *source_generation, frame.data(), metadata);
                runtime.record_pipeline_skips(skipped);
                last_generation = frame.generation();
                backoff.reset();
                watchdog.complete(watchdog_token.take().expect("watchdog armed"));
                capture_in_progress = false;
            }
            Err(CaptureError::Timeout) => {}
            Err(CaptureError::Interrupted) => {
                let token = watchdog_token.take().expect("watchdog armed");
                if watchdog.cancellation_requested(token) {
                    return CaptureSessionEnd {
                        watchdog_token: Some(token),
                    };
                }
                watchdog.complete(token);
                capture_in_progress = false;
            }
            Err(CaptureError::MalformedDimensions { .. }) => {
                if validation_failure(
                    runtime,
                    &mut validation,
                    ValidationFailure::InvalidDimensions,
                    session_started.elapsed(),
                ) {
                    return CaptureSessionEnd {
                        watchdog_token: watchdog_token.take(),
                    };
                }
                watchdog.complete(watchdog_token.take().expect("watchdog armed"));
                capture_in_progress = false;
            }
            Err(CaptureError::MalformedLength { .. }) => {
                if validation_failure(
                    runtime,
                    &mut validation,
                    ValidationFailure::InvalidBufferLength,
                    session_started.elapsed(),
                ) {
                    return CaptureSessionEnd {
                        watchdog_token: watchdog_token.take(),
                    };
                }
                watchdog.complete(watchdog_token.take().expect("watchdog armed"));
                capture_in_progress = false;
            }
            Err(CaptureError::MalformedGeneration { .. }) => {
                if validation_failure(
                    runtime,
                    &mut validation,
                    ValidationFailure::InvalidGeneration,
                    session_started.elapsed(),
                ) {
                    return CaptureSessionEnd {
                        watchdog_token: watchdog_token.take(),
                    };
                }
                watchdog.complete(watchdog_token.take().expect("watchdog armed"));
                capture_in_progress = false;
            }
            Err(error) => {
                tracing::error!(%error, "camera capture stopped");
                return CaptureSessionEnd {
                    watchdog_token: watchdog_token.take(),
                };
            }
        }
        if let Some(interval) = minimum_capture_interval {
            thread::sleep(interval.saturating_sub(capture_wait_started.elapsed()));
        }
    }
}

fn validation_failure(
    runtime: &RuntimeState,
    validation: &mut ValidationWindow,
    failure: ValidationFailure,
    now: Duration,
) -> bool {
    runtime.record_validation_failure(failure);
    validation.record(failure, now)
}

/// Applies at most one claimed settings target through the camera-owner lifecycle.
///
/// The boundary callback is invoked exactly once after live controls change and
/// before the target becomes authoritative as Applied.
///
/// # Errors
///
/// Returns a camera lifecycle error without marking the target Applied.
///
/// # Panics
///
/// Panics if the settings coordinator's internal invariants are violated.
fn apply_pending_settings(
    source: &mut impl CameraSource,
    controller: &crate::SettingsController,
    begin_epoch: impl FnOnce(u64),
) -> Result<SettingsTransition, zwo_asi::CameraError> {
    let Some(target) = controller.claim_latest() else {
        return Ok(SettingsTransition::Idle);
    };
    let previous = controller.snapshot().applied().settings();
    let exposure_changed = previous.exposure_ms() != target.settings().exposure_ms();
    let exposure_shortened = target.settings().exposure_ms() < previous.exposure_ms();
    let gain_changed = previous.gain() != target.settings().gain();
    let camera_controls_changed = previous.camera_settings() != target.settings().camera_settings();
    if camera_controls_changed
        && let Err(error) = source.apply_live_settings(target.settings().camera_settings())
    {
        controller.begin_recovery();
        source.stop()?;
        source.configure(previous.camera_settings())?;
        source.start()?;
        controller.mark_camera_ready();
        return Ok(SettingsTransition::Restored(error));
    }
    let untrusted_frames = if gain_changed || exposure_shortened {
        MAX_SENSOR_TRANSITION_FRAMES
    } else {
        u8::from(exposure_changed)
    };
    begin_epoch(target.generation());
    controller.mark_applied(target);
    Ok(SettingsTransition::Applied { untrusted_frames })
}

/// Outcome of one bounded settings transition attempt.
#[derive(Debug)]
enum SettingsTransition {
    /// No settings target was pending.
    Idle,
    /// The target became Applied, with transitional frames explicitly fenced.
    Applied {
        /// Completed frames that remain visible but cannot carry exact target metadata.
        untrusted_frames: u8,
    },
    /// Applying failed, the target was failed, and the prior tuple was restored.
    Restored(zwo_asi::CameraError),
}

fn spawn_processing(
    raw: Arc<LatestBufferMailbox>,
    processed: Arc<LatestFrameMailbox>,
    gate: Arc<ProcessingGate>,
    recovery: mpsc::Sender<()>,
    runtime: RuntimeState,
) -> io::Result<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("obscam-processing".into())
        .spawn(move || {
            let settings = runtime.settings();
            let mut processor = MonochromeProcessor::new();
            let mut colour_processor = ColourProcessor::new();
            let started = Instant::now();
            let mut validation = ValidationWindow::new(Duration::from_secs(10));
            let mut recovery_pending = false;
            loop {
                let permit = gate.enter();
                if gate.take_reset() {
                    processor = MonochromeProcessor::new();
                    colour_processor = ColourProcessor::new();
                    validation.reset();
                    recovery_pending = false;
                }
                let Some(source) = raw.wait_take(Duration::from_secs(1)) else {
                    drop(permit);
                    continue;
                };
                let generation = source.generation();
                let epoch = source.epoch();
                let output = match settings.applied_treatment() {
                    Treatment::Monochrome => processor.process_validated(generation, source.data()),
                    Treatment::Colour => {
                        colour_processor.process_validated(generation, source.data())
                    }
                };
                let valid = validate_processing_output(
                    generation,
                    output.generation(),
                    output.width(),
                    output.height(),
                    output.data().len(),
                );
                if !valid {
                    request_processing_recovery(
                        &runtime,
                        &mut validation,
                        started.elapsed(),
                        &recovery,
                        &mut recovery_pending,
                    );
                } else if raw.is_current_epoch(epoch) {
                    let skipped = if let Some(metadata) = source.metadata() {
                        processed.publish_captured(epoch, &output, metadata)
                    } else {
                        processed.publish_in_epoch(epoch, &output)
                    };
                    runtime.record_pipeline_skips(skipped);
                }
                raw.recycle(source);
                drop(permit);
            }
        })
}

fn request_processing_recovery(
    runtime: &RuntimeState,
    validation: &mut ValidationWindow,
    now: Duration,
    recovery: &mpsc::Sender<()>,
    recovery_pending: &mut bool,
) {
    let threshold_reached = validation_failure(
        runtime,
        validation,
        ValidationFailure::InvalidProcessingOutput,
        now,
    );
    if !*recovery_pending && threshold_reached && recovery.send(()).is_ok() {
        *recovery_pending = true;
    }
}

pub(crate) fn unix_time_us() -> u64 {
    let micros = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros();
    u64::try_from(micros).unwrap_or(u64::MAX)
}

fn spawn_encoder(
    mailbox: Arc<LatestFrameMailbox>,
    runtime: RuntimeState,
) -> io::Result<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("obscam-encoder".into())
        .spawn(move || {
            let correlation = runtime.correlation();
            let mut encoder = None;
            let mut completed = None;
            let mut recovering = false;
            let mut backoff = EncoderRecoveryBackoff::new();
            let mut next_publication_at: Option<Instant> = None;
            loop {
                if encoder.is_none() {
                    if recovering {
                        thread::sleep(
                            backoff.next_delay_with_jitter(encoder_recovery_jitter_ms()),
                        );
                    }
                    match FfmpegEncoder::start_correlated(
                        Path::new("ffmpeg"),
                        correlation.clone(),
                    ) {
                        Ok(replacement) => {
                            if recovering {
                                runtime.record_encoder_replacement();
                            }
                            encoder = Some(replacement);
                            next_publication_at = None;
                        }
                        Err(error) => {
                            runtime.set_encoder_readiness(ComponentReadiness::Unavailable);
                            tracing::error!(%error, "qualified FFmpeg hardware encoder unavailable; retrying");
                            recovering = true;
                            continue;
                        }
                    }
                }
                let next = if completed.is_some() {
                    if let Some(deadline) = next_publication_at {
                        thread::sleep(deadline.saturating_duration_since(Instant::now()));
                    }
                    mailbox.take()
                } else {
                    mailbox.wait_take(Duration::from_secs(1))
                };
                let publication = if let Some(frame) = next {
                    if let Some(previous) = completed.take() {
                        mailbox.recycle(previous);
                    }
                    let publication = mailbox.commit_owned_if_current(frame, |current| {
                        encoder
                            .as_mut()
                            .expect("current media epoch has an encoder")
                            .publish_owned(current, false, ENCODER_PUBLICATION_TIMEOUT)
                    });
                    match publication {
                        Ok(publication) => publication,
                        Err(frame) => {
                            mailbox.recycle(frame);
                            continue;
                        }
                    }
                } else if let Some(frame) = completed.take() {
                    if mailbox.is_current(&frame) {
                        encoder
                            .as_mut()
                            .expect("completed frame has an encoder")
                            .publish_owned(frame, true, ENCODER_PUBLICATION_TIMEOUT)
                    } else {
                        mailbox.recycle(frame);
                        next_publication_at = None;
                        continue;
                    }
                } else {
                    if replace_exited_encoder(&mut encoder, &runtime, &mailbox) {
                        recovering = true;
                    }
                    continue;
                };
                match publication {
                    Ok(frame) => {
                        runtime.set_encoder_readiness(ComponentReadiness::Ready);
                        completed = Some(frame);
                        recovering = false;
                        backoff.reset();
                        next_publication_at = Some(next_encoder_publication_at(
                            next_publication_at.unwrap_or_else(Instant::now),
                            Instant::now(),
                        ));
                    }
                    Err(failure) => {
                        let (frame, error) = failure.into_parts();
                        runtime.set_encoder_readiness(ComponentReadiness::Unavailable);
                        tracing::error!(%error, "FFmpeg hardware publication stopped; replacing encoder");
                        completed = Some(frame);
                        encoder.take();
                        recovering = true;
                        runtime.record_pipeline_skips(mailbox.discard_pending());
                    }
                }
            }
        })
}

fn next_encoder_publication_at(scheduled_at: Instant, now: Instant) -> Instant {
    scheduled_at
        .checked_add(ENCODER_PUBLICATION_INTERVAL)
        .filter(|next| *next > now)
        .unwrap_or_else(|| now + ENCODER_PUBLICATION_INTERVAL)
}

fn replace_exited_encoder(
    encoder: &mut Option<FfmpegEncoder>,
    runtime: &RuntimeState,
    mailbox: &LatestFrameMailbox,
) -> bool {
    let Err(error) = encoder
        .as_mut()
        .expect("started encoder remains present")
        .verify_running()
    else {
        return false;
    };
    runtime.set_encoder_readiness(ComponentReadiness::Unavailable);
    tracing::error!(%error, "FFmpeg hardware encoder exited; replacing encoder");
    encoder.take();
    runtime.record_pipeline_skips(mailbox.discard_pending());
    true
}

#[cfg(all(test, feature = "camera-substitute"))]
mod tests {
    use super::*;
    use crate::{CameraSettings, Config, SettingsController};
    use std::{
        process::Command,
        sync::atomic::{AtomicUsize, Ordering},
    };
    use uuid::Uuid;
    use zwo_asi::{CapturePlan, DeterministicCamera, DeterministicScenario, Settings};

    #[test]
    fn repeated_encoder_failures_back_off_with_a_five_second_cap() {
        let mut backoff = EncoderRecoveryBackoff::new();

        assert_eq!(backoff.next_delay_with_jitter(250), Duration::ZERO);
        assert_eq!(
            backoff.next_delay_with_jitter(0),
            Duration::from_millis(250)
        );
        assert_eq!(
            backoff.next_delay_with_jitter(0),
            Duration::from_millis(500)
        );
        assert_eq!(backoff.next_delay_with_jitter(0), Duration::from_secs(1));
        assert_eq!(backoff.next_delay_with_jitter(0), Duration::from_secs(2));
        assert_eq!(backoff.next_delay_with_jitter(0), Duration::from_secs(4));
        assert_eq!(backoff.next_delay_with_jitter(0), Duration::from_secs(5));
        assert_eq!(backoff.next_delay_with_jitter(0), Duration::from_secs(5));
    }

    #[test]
    fn encoder_publication_deadline_holds_twenty_fps_without_catch_up_bursts() {
        let start = Instant::now();

        assert_eq!(
            next_encoder_publication_at(start, start + Duration::from_millis(10)),
            start + ENCODER_PUBLICATION_INTERVAL
        );
        assert_eq!(
            next_encoder_publication_at(start, start + ENCODER_PUBLICATION_INTERVAL),
            start + ENCODER_PUBLICATION_INTERVAL * 2
        );
        assert_eq!(
            next_encoder_publication_at(start, start + Duration::from_millis(80)),
            start + Duration::from_millis(130)
        );
    }

    #[test]
    fn relay_probe_accepts_only_a_ready_obscam_path() {
        let ready = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\npaths{name=\"obscam\",state=\"ready\"} 1\n";
        let unavailable = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\npaths{name=\"obscam\",state=\"notReady\"} 1\n";

        assert!(relay_path_is_ready(ready));
        assert!(!relay_path_is_ready(unavailable));
        assert!(!relay_path_is_ready(b"HTTP/1.1 404 Not Found\r\n\r\n{}"));
    }

    #[test]
    fn gain_changes_keep_three_transition_frames_visible_but_untrusted() {
        let mut trust = SettingsFrameTrust::default();

        trust.begin_transition(3);

        assert!(!trust.permits_capture_progress());
        assert!(!trust.permits_exact_metadata());
        assert!(!trust.permits_capture_progress());
        assert!(!trust.permits_exact_metadata());
        assert!(!trust.permits_capture_progress());
        assert!(!trust.permits_exact_metadata());
        assert!(trust.permits_capture_progress());
        assert!(trust.permits_exact_metadata());
    }

    #[test]
    fn a_lengthened_exposure_discards_only_the_old_in_flight_frame() {
        let mut trust = SettingsFrameTrust::default();

        trust.begin_transition(1);

        assert!(!trust.permits_exact_metadata());
        assert!(trust.permits_exact_metadata());
    }

    #[test]
    fn a_shortened_exposure_keeps_three_transition_frames_visible_but_untrusted() {
        let mut trust = SettingsFrameTrust::default();

        trust.begin_transition(3);

        assert!(!trust.permits_exact_metadata());
        assert!(!trust.permits_exact_metadata());
        assert!(!trust.permits_exact_metadata());
        assert!(trust.permits_exact_metadata());
    }

    #[test]
    fn browser_only_treatment_changes_do_not_touch_sensor_controls_or_need_a_fence() {
        let mut inner =
            DeterministicCamera::connect(DeterministicScenario::new([])).expect("camera present");
        inner
            .configure(Settings::new(500_000, 100).expect("defaults"))
            .expect("configure");
        inner.start().expect("start");
        let stops = Arc::new(AtomicUsize::new(0));
        let mut camera = StopCountingCamera::new(inner, Arc::clone(&stops));
        let controller = ready_controller();
        controller.install_interrupter(camera.interrupter());
        controller
            .accept(
                CameraSettings::new(500, 100, Treatment::Colour).expect("treatment-only target"),
            )
            .expect("camera ready");

        let transition = apply_pending_settings(&mut camera, &controller, |_| {})
            .expect("treatment-only transition");

        assert!(matches!(
            transition,
            SettingsTransition::Applied {
                untrusted_frames: 0
            }
        ));
        assert_eq!(camera.live_settings_applications, 0);
        assert_eq!(stops.load(Ordering::SeqCst), 0);
        let mut trust = SettingsFrameTrust::default();
        trust.begin_transition(0);
        assert!(trust.permits_exact_metadata());
    }

    #[test]
    fn shortening_an_exposure_keeps_warm_acquisition_running() {
        let mut inner =
            DeterministicCamera::connect(DeterministicScenario::new([])).expect("camera present");
        inner
            .configure(Settings::new(500_000, 100).expect("defaults"))
            .expect("configure");
        inner.start().expect("start");
        let stops = Arc::new(AtomicUsize::new(0));
        let mut camera = StopCountingCamera::new(inner, Arc::clone(&stops));
        let controller = ready_controller();
        controller.install_interrupter(camera.interrupter());
        let target = controller
            .accept(CameraSettings::new(50, 200, Treatment::Colour).expect("target"))
            .expect("camera ready");
        assert_eq!(
            camera.capture_next(100).err(),
            Some(CaptureError::Interrupted)
        );
        let mut boundaries = Vec::new();

        let transition = apply_pending_settings(&mut camera, &controller, |generation| {
            boundaries.push(generation);
        })
        .expect("transition");
        assert!(matches!(
            transition,
            SettingsTransition::Applied {
                untrusted_frames: 3
            }
        ));

        assert_eq!(boundaries, [target.generation()]);
        assert_eq!(
            stops.load(Ordering::SeqCst),
            0,
            "live settings do not restart warm acquisition"
        );
        assert_eq!(controller.snapshot().applied(), target);
        let frame = camera
            .capture_next(100)
            .expect("matching-generation capture");
        assert_eq!(frame.data()[258 * WIDTH + 258], 171, "gain 200 applied");
    }

    #[test]
    fn failed_apply_restores_only_the_previous_fully_applied_tuple() {
        let mut inner =
            DeterministicCamera::connect(DeterministicScenario::new([])).expect("camera present");
        inner
            .configure(Settings::new(500_000, 100).expect("defaults"))
            .expect("configure");
        inner.start().expect("start");
        let mut camera = FailNextLiveSettings::new(inner);
        let controller = ready_controller();
        controller.install_interrupter(camera.interrupter());
        let target = controller
            .accept(CameraSettings::new(50, 600, Treatment::Colour).expect("target"))
            .expect("camera ready");
        let mut boundary_count = 0;

        let transition = apply_pending_settings(&mut camera, &controller, |_| boundary_count += 1)
            .expect("previous tuple restores");

        assert!(matches!(transition, SettingsTransition::Restored(_)));
        assert_eq!(
            controller.last_failure().map(|(generation, _)| generation),
            Some(target.generation())
        );
        assert_eq!(boundary_count, 0, "failed target establishes no boundary");
        assert_eq!(controller.snapshot().applied().generation(), 0);
        let frame = loop {
            match camera.capture_next(100) {
                Ok(frame) => break frame,
                Err(CaptureError::Timeout) => {}
                Err(error) => panic!("restored capture failed: {error}"),
            }
        };
        assert_eq!(frame.data()[258 * WIDTH + 258], 166, "gain 100 restored");
    }

    #[test]
    fn three_malformed_dimensions_recover_capture_and_increment_only_their_counter() {
        let plans = [
            CapturePlan::MalformedDimensions {
                width: 1280,
                height: 720,
            },
            CapturePlan::MalformedDimensions {
                width: 1280,
                height: 720,
            },
            CapturePlan::MalformedDimensions {
                width: 1280,
                height: 720,
            },
        ];
        let (end, snapshot) = run_capture_faults(plans);

        assert!(end.watchdog_token.is_some());
        assert_eq!(snapshot["mediaRecovery"]["invalidDimensions"], 3);
        assert_eq!(snapshot["mediaRecovery"]["invalidBufferLengths"], 0);
        assert_eq!(snapshot["mediaRecovery"]["invalidGenerationMetadata"], 0);
    }

    #[test]
    fn malformed_lengths_and_generations_have_independent_recovery_thresholds() {
        let plans = [
            CapturePlan::MalformedLength { length: 1 },
            CapturePlan::MalformedGeneration { generation: 0 },
            CapturePlan::MalformedLength { length: 1 },
            CapturePlan::MalformedGeneration { generation: 0 },
            CapturePlan::MalformedLength { length: 1 },
        ];
        let (end, snapshot) = run_capture_faults(plans);

        assert!(end.watchdog_token.is_some());
        assert_eq!(snapshot["mediaRecovery"]["invalidBufferLengths"], 3);
        assert_eq!(snapshot["mediaRecovery"]["invalidGenerationMetadata"], 2);
    }

    #[test]
    fn sdk_and_disconnect_faults_recover_capture_immediately() {
        for plan in [CapturePlan::Sdk { code: 17 }, CapturePlan::Disconnect] {
            let (end, snapshot) = run_capture_faults([plan]);
            assert!(end.watchdog_token.is_some());
            assert_eq!(snapshot["mediaRecovery"]["invalidDimensions"], 0);
            assert_eq!(snapshot["mediaRecovery"]["invalidBufferLengths"], 0);
            assert_eq!(snapshot["mediaRecovery"]["invalidGenerationMetadata"], 0);
        }
    }

    #[test]
    fn processing_validation_requests_only_processing_component_recovery() {
        let runtime = capture_test_runtime();
        let stops = Arc::new(AtomicUsize::new(0));
        let mut camera = StopCountingCamera::new(
            configured_camera([CapturePlan::Sdk { code: 17 }]),
            Arc::clone(&stops),
        );
        let raw = LatestBufferMailbox::new(RAW8_BYTES);
        let initial_epoch = raw.current_epoch();
        let gate = ProcessingGate::paused();
        gate.resume();
        let (recovery, requests) = mpsc::channel();
        recovery.send(()).expect("processing recovery request");
        let watchdog = CaptureWatchdogMonitor::spawn().expect("watchdog");
        let mut backoff = CameraRecoveryBackoff::new();
        let mut source_generation = 10;

        let mut context = CaptureSessionContext {
            runtime: &runtime,
            raw: &raw,
            processing_gate: &gate,
            processing_recovery: &requests,
            minimum_capture_interval: None,
            watchdog: &watchdog,
            backoff: &mut backoff,
            source_generation: &mut source_generation,
        };
        let end = capture_session(&mut camera, &mut context);

        if let Some(token) = end.watchdog_token {
            watchdog.complete(token);
        }
        assert_eq!(stops.load(Ordering::SeqCst), 0);
        assert_ne!(raw.current_epoch(), initial_epoch);
        let snapshot = serde_json::to_value(runtime.snapshot()).expect("runtime snapshot");
        assert_eq!(snapshot["minimumSourceGeneration"], 11);
        assert_eq!(snapshot["components"]["capture"]["state"], "ready");
    }

    #[test]
    fn invalid_processing_output_requests_one_recovery_at_the_threshold() {
        let runtime = capture_test_runtime();
        let mut validation = ValidationWindow::new(Duration::from_secs(10));
        let (recovery, requests) = mpsc::channel();
        let mut recovery_pending = false;

        for second in 0..5 {
            request_processing_recovery(
                &runtime,
                &mut validation,
                Duration::from_secs(second),
                &recovery,
                &mut recovery_pending,
            );
        }

        assert!(requests.try_recv().is_ok());
        assert!(
            requests.try_recv().is_err(),
            "one threshold crossing queues once"
        );
        let snapshot = serde_json::to_value(runtime.snapshot()).expect("runtime snapshot");
        assert_eq!(snapshot["mediaRecovery"]["invalidProcessingOutput"], 5);
    }

    #[test]
    fn capture_watchdog_monitor_interrupts_the_owner() {
        let mut camera = configured_camera([]);
        let watchdog = CaptureWatchdogMonitor::spawn().expect("watchdog");
        let token = watchdog.arm(Duration::ZERO, camera.interrupter());

        thread::sleep(Duration::from_millis(2_100));

        assert_eq!(
            camera.capture_next(100).err(),
            Some(CaptureError::Interrupted)
        );
        watchdog.complete(token);
    }

    #[test]
    fn capture_watchdog_monitor_terminates_when_cancellation_does_not_complete() {
        const CHILD: &str = "OBSCAM_WATCHDOG_TERMINATION_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let camera = configured_camera([]);
            let watchdog = CaptureWatchdogMonitor::spawn().expect("watchdog");
            let _token = watchdog.arm(Duration::ZERO, camera.interrupter());
            thread::sleep(Duration::from_secs(6));
            panic!("watchdog did not terminate the process");
        }

        let status = Command::new(std::env::current_exe().expect("current test executable"))
            .args([
                "--exact",
                "pipeline::tests::capture_watchdog_monitor_terminates_when_cancellation_does_not_complete",
            ])
            .env(CHILD, "1")
            .status()
            .expect("watchdog child process");

        assert_eq!(status.code(), Some(70));
    }

    #[test]
    fn failed_teardown_terminates_before_reopen() {
        const CHILD: &str = "OBSCAM_TEARDOWN_FAILURE_CHILD";
        if let Some(mode) = std::env::var_os(CHILD) {
            let mode = mode.to_string_lossy().into_owned();
            let runtime = capture_test_runtime();
            let raw = LatestBufferMailbox::new(RAW8_BYTES);
            let gate = ProcessingGate::paused();
            let (_processing, requests) = mpsc::channel();
            let watchdog = CaptureWatchdogMonitor::spawn().expect("watchdog");
            let fail_stop = mode == "stop";
            supervise_capture(
                move || {
                    if mode == "connect" {
                        return Err(zwo_asi::CameraError::OwnershipUncertain { close_code: 5 });
                    }
                    let camera = DeterministicCamera::connect(DeterministicScenario::new([
                        CapturePlan::Sdk { code: 17 },
                    ]))?;
                    Ok::<_, zwo_asi::CameraError>(FailingTeardownCamera::new(camera, fail_stop))
                },
                &runtime,
                &raw,
                &gate,
                &requests,
                None,
                &watchdog,
            );
            panic!("teardown failure reopened in-process");
        }

        for mode in ["connect", "stop", "close"] {
            let status = Command::new(std::env::current_exe().expect("current test executable"))
                .args([
                    "--exact",
                    "pipeline::tests::failed_teardown_terminates_before_reopen",
                ])
                .env(CHILD, mode)
                .status()
                .expect("teardown child process");
            assert_eq!(status.code(), Some(70), "{mode} failure terminates");
        }
    }

    #[test]
    fn camera_supervisor_drops_the_old_owner_before_reopening_and_restores_the_applied_tuple() {
        let runtime = capture_test_runtime();
        let raw = Arc::new(LatestBufferMailbox::new(RAW8_BYTES));
        let gate = ProcessingGate::paused();
        let (_processing, requests) = mpsc::channel();
        let watchdog = CaptureWatchdogMonitor::spawn().expect("watchdog");
        let attempts = Arc::new(AtomicUsize::new(0));
        let active = Arc::new(AtomicUsize::new(0));
        let maximum_active = Arc::new(AtomicUsize::new(0));
        let connect_attempts = Arc::clone(&attempts);
        let connect_active = Arc::clone(&active);
        let connect_maximum = Arc::clone(&maximum_active);
        let worker_runtime = runtime.clone();
        let worker_raw = Arc::clone(&raw);

        let worker = thread::spawn(move || {
            supervise_capture(
                move || {
                    let attempt = connect_attempts.fetch_add(1, Ordering::SeqCst) + 1;
                    assert!(attempt <= 2, "test ends after the recovered source fails");
                    assert_eq!(
                        connect_active.load(Ordering::SeqCst),
                        0,
                        "the old camera owner must be dropped before reconnect"
                    );
                    let plans = vec![
                        CapturePlan::Frame {
                            additional_delay_us: 0,
                        },
                        CapturePlan::Sdk { code: 17 },
                    ];
                    let camera = DeterministicCamera::connect(DeterministicScenario::new(plans))?;
                    Ok::<_, zwo_asi::CameraError>(TrackedCamera::new(
                        camera,
                        Arc::clone(&connect_active),
                        connect_maximum.as_ref(),
                    ))
                },
                &worker_runtime,
                &worker_raw,
                &gate,
                &requests,
                None,
                &watchdog,
            );
        });

        assert!(
            worker.join().is_err(),
            "bounded test connector stops the supervisor"
        );
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
        assert_eq!(maximum_active.load(Ordering::SeqCst), 1);
        assert_eq!(runtime.settings().snapshot().applied().generation(), 0);
        let snapshot = serde_json::to_value(runtime.snapshot()).expect("runtime snapshot");
        assert_eq!(snapshot["mediaRecovery"]["cameraRestarts"], 1);
    }

    #[test]
    fn supervisor_retries_absence_and_startup_failure_until_capture_recovers() {
        let runtime = capture_test_runtime();
        let raw = LatestBufferMailbox::new(RAW8_BYTES);
        let gate = ProcessingGate::paused();
        let (_processing, requests) = mpsc::channel();
        let watchdog = CaptureWatchdogMonitor::spawn().expect("watchdog");
        let attempts = Arc::new(AtomicUsize::new(0));
        let connector_attempts = Arc::clone(&attempts);
        let worker_runtime = runtime.clone();

        let worker = thread::spawn(move || {
            supervise_capture(
                move || match connector_attempts.fetch_add(1, Ordering::SeqCst) + 1 {
                    1 => Err(zwo_asi::CameraError::IdentityCount { found: 0 }),
                    2 => DeterministicCamera::connect(DeterministicScenario::new([]))
                        .map(FailNextConfiguration::new),
                    3 => DeterministicCamera::connect(DeterministicScenario::new([
                        CapturePlan::Frame {
                            additional_delay_us: 0,
                        },
                        CapturePlan::Sdk { code: 17 },
                    ]))
                    .map(FailNextConfiguration::without_failure),
                    _ => panic!("test ends after recovered capture fails"),
                },
                &worker_runtime,
                &raw,
                &gate,
                &requests,
                None,
                &watchdog,
            );
        });

        assert!(worker.join().is_err());
        assert_eq!(attempts.load(Ordering::SeqCst), 4);
        let snapshot = serde_json::to_value(runtime.snapshot()).expect("runtime snapshot");
        assert_eq!(snapshot["mediaRecovery"]["cameraRestarts"], 1);
    }

    #[test]
    fn source_generation_remains_runtime_monotonic_across_camera_owners() {
        let runtime = capture_test_runtime();
        let raw = LatestBufferMailbox::new(RAW8_BYTES);
        let gate = ProcessingGate::paused();
        gate.resume();
        let (_recovery, requests) = mpsc::channel();
        let watchdog = CaptureWatchdogMonitor::spawn().expect("watchdog");
        let mut backoff = CameraRecoveryBackoff::new();
        let mut source_generation = 0;
        for owner in 0..2 {
            let mut camera = configured_camera([
                CapturePlan::Frame {
                    additional_delay_us: 0,
                },
                CapturePlan::Sdk { code: 17 },
            ]);
            let mut context = CaptureSessionContext {
                runtime: &runtime,
                raw: &raw,
                processing_gate: &gate,
                processing_recovery: &requests,
                minimum_capture_interval: None,
                watchdog: &watchdog,
                backoff: &mut backoff,
                source_generation: &mut source_generation,
            };
            let end = capture_session(&mut camera, &mut context);
            if let Some(token) = end.watchdog_token {
                watchdog.complete(token);
            }
            if owner == 0 {
                raw.begin_new_epoch();
            }
        }

        assert_eq!(
            raw.take()
                .expect("latest recovered generation")
                .generation(),
            2
        );
    }

    fn run_capture_faults<const N: usize>(
        plans: [CapturePlan; N],
    ) -> (CaptureSessionEnd, serde_json::Value) {
        let runtime = capture_test_runtime();
        let mut camera = configured_camera(plans);
        let raw = LatestBufferMailbox::new(RAW8_BYTES);
        let gate = ProcessingGate::paused();
        gate.resume();
        let (_recovery, requests) = mpsc::channel();
        let watchdog = CaptureWatchdogMonitor::spawn().expect("watchdog");
        let mut backoff = CameraRecoveryBackoff::new();
        let mut source_generation = 0;

        let mut context = CaptureSessionContext {
            runtime: &runtime,
            raw: &raw,
            processing_gate: &gate,
            processing_recovery: &requests,
            minimum_capture_interval: None,
            watchdog: &watchdog,
            backoff: &mut backoff,
            source_generation: &mut source_generation,
        };
        let end = capture_session(&mut camera, &mut context);
        if let Some(token) = end.watchdog_token {
            watchdog.complete(token);
        }
        let snapshot = serde_json::to_value(runtime.snapshot()).expect("runtime snapshot");
        (end, snapshot)
    }

    fn configured_camera(plans: impl IntoIterator<Item = CapturePlan>) -> DeterministicCamera {
        let mut camera = DeterministicCamera::connect(DeterministicScenario::new(plans))
            .expect("camera present");
        camera
            .configure(Settings::new(500_000, 100).expect("defaults"))
            .expect("configure");
        camera.start().expect("start");
        camera
    }

    fn capture_test_runtime() -> RuntimeState {
        let config =
            Config::parse("127.0.0.1:8080", "8889", "/obscam/whep").expect("configuration");
        let runtime = RuntimeState::with_readiness(
            Uuid::from_u128(1),
            &config,
            ComponentReadiness::Ready,
            ComponentReadiness::Ready,
            ComponentReadiness::Ready,
        );
        runtime.settings().mark_camera_ready();
        runtime
    }

    struct TrackedCamera {
        inner: Option<DeterministicCamera>,
        active: Arc<AtomicUsize>,
    }

    impl TrackedCamera {
        fn new(
            inner: DeterministicCamera,
            active: Arc<AtomicUsize>,
            maximum_active: &AtomicUsize,
        ) -> Self {
            let current = active.fetch_add(1, Ordering::SeqCst) + 1;
            maximum_active.fetch_max(current, Ordering::SeqCst);
            Self {
                inner: Some(inner),
                active,
            }
        }

        fn inner(&self) -> &DeterministicCamera {
            self.inner.as_ref().expect("tracked camera remains present")
        }

        fn inner_mut(&mut self) -> &mut DeterministicCamera {
            self.inner.as_mut().expect("tracked camera remains present")
        }
    }

    impl CameraSource for TrackedCamera {
        fn interrupter(&self) -> zwo_asi::CaptureInterrupter {
            self.inner().interrupter()
        }

        fn configure(&mut self, settings: Settings) -> Result<(), zwo_asi::CameraError> {
            self.inner_mut().configure(settings)
        }

        fn apply_live_settings(&mut self, settings: Settings) -> Result<(), zwo_asi::CameraError> {
            self.inner_mut().apply_live_settings(settings)
        }

        fn start(&mut self) -> Result<(), zwo_asi::CameraError> {
            self.inner_mut().start()
        }

        fn capture_next(
            &mut self,
            wait_ms: i32,
        ) -> Result<zwo_asi::FrameGeneration<'_>, CaptureError> {
            self.inner_mut().capture_next(wait_ms)
        }

        fn stop(&mut self) -> Result<(), zwo_asi::CameraError> {
            self.inner_mut().stop()
        }
    }

    impl Drop for TrackedCamera {
        fn drop(&mut self) {
            drop(self.inner.take());
            self.active.fetch_sub(1, Ordering::SeqCst);
        }
    }

    struct StopCountingCamera {
        inner: DeterministicCamera,
        stops: Arc<AtomicUsize>,
        live_settings_applications: usize,
    }

    impl StopCountingCamera {
        fn new(inner: DeterministicCamera, stops: Arc<AtomicUsize>) -> Self {
            Self {
                inner,
                stops,
                live_settings_applications: 0,
            }
        }
    }

    impl CameraSource for StopCountingCamera {
        fn interrupter(&self) -> zwo_asi::CaptureInterrupter {
            self.inner.interrupter()
        }

        fn configure(&mut self, settings: Settings) -> Result<(), zwo_asi::CameraError> {
            self.inner.configure(settings)
        }

        fn apply_live_settings(&mut self, settings: Settings) -> Result<(), zwo_asi::CameraError> {
            self.live_settings_applications += 1;
            self.inner.apply_live_settings(settings)
        }

        fn start(&mut self) -> Result<(), zwo_asi::CameraError> {
            self.inner.start()
        }

        fn capture_next(
            &mut self,
            wait_ms: i32,
        ) -> Result<zwo_asi::FrameGeneration<'_>, CaptureError> {
            self.inner.capture_next(wait_ms)
        }

        fn stop(&mut self) -> Result<(), zwo_asi::CameraError> {
            self.stops.fetch_add(1, Ordering::SeqCst);
            self.inner.stop()
        }
    }

    struct FailingTeardownCamera {
        inner: DeterministicCamera,
        fail_stop: bool,
    }

    impl FailingTeardownCamera {
        const fn new(inner: DeterministicCamera, fail_stop: bool) -> Self {
            Self { inner, fail_stop }
        }
    }

    impl CameraSource for FailingTeardownCamera {
        fn interrupter(&self) -> zwo_asi::CaptureInterrupter {
            self.inner.interrupter()
        }

        fn configure(&mut self, settings: Settings) -> Result<(), zwo_asi::CameraError> {
            self.inner.configure(settings)
        }

        fn apply_live_settings(&mut self, settings: Settings) -> Result<(), zwo_asi::CameraError> {
            self.inner.apply_live_settings(settings)
        }

        fn start(&mut self) -> Result<(), zwo_asi::CameraError> {
            self.inner.start()
        }

        fn capture_next(
            &mut self,
            wait_ms: i32,
        ) -> Result<zwo_asi::FrameGeneration<'_>, CaptureError> {
            self.inner.capture_next(wait_ms)
        }

        fn stop(&mut self) -> Result<(), zwo_asi::CameraError> {
            if self.fail_stop {
                return Err(zwo_asi::CameraError::InvalidState {
                    operation: "injected stop failure",
                });
            }
            self.inner.stop()
        }

        fn close(self) -> Result<(), zwo_asi::CameraError> {
            Err(zwo_asi::CameraError::InvalidState {
                operation: "injected close failure",
            })
        }
    }

    fn ready_controller() -> SettingsController {
        let controller = SettingsController::new(CameraSettings::default());
        controller.mark_camera_ready();
        controller
    }

    struct FailNextConfiguration {
        inner: DeterministicCamera,
        fail_next: bool,
    }

    impl FailNextConfiguration {
        const fn new(inner: DeterministicCamera) -> Self {
            Self {
                inner,
                fail_next: true,
            }
        }

        const fn without_failure(inner: DeterministicCamera) -> Self {
            Self {
                inner,
                fail_next: false,
            }
        }
    }

    impl CameraSource for FailNextConfiguration {
        fn interrupter(&self) -> zwo_asi::CaptureInterrupter {
            self.inner.interrupter()
        }

        fn configure(&mut self, settings: Settings) -> Result<(), zwo_asi::CameraError> {
            if self.fail_next {
                self.fail_next = false;
                return Err(zwo_asi::CameraError::InvalidState {
                    operation: "injected configuration failure",
                });
            }
            self.inner.configure(settings)
        }

        fn apply_live_settings(&mut self, settings: Settings) -> Result<(), zwo_asi::CameraError> {
            self.inner.apply_live_settings(settings)
        }

        fn start(&mut self) -> Result<(), zwo_asi::CameraError> {
            self.inner.start()
        }

        fn capture_next(
            &mut self,
            wait_ms: i32,
        ) -> Result<zwo_asi::FrameGeneration<'_>, CaptureError> {
            self.inner.capture_next(wait_ms)
        }

        fn stop(&mut self) -> Result<(), zwo_asi::CameraError> {
            self.inner.stop()
        }
    }

    struct FailNextLiveSettings {
        inner: DeterministicCamera,
        fail_next: bool,
    }

    impl FailNextLiveSettings {
        const fn new(inner: DeterministicCamera) -> Self {
            Self {
                inner,
                fail_next: true,
            }
        }
    }

    impl CameraSource for FailNextLiveSettings {
        fn interrupter(&self) -> zwo_asi::CaptureInterrupter {
            self.inner.interrupter()
        }

        fn configure(&mut self, settings: Settings) -> Result<(), zwo_asi::CameraError> {
            self.inner.configure(settings)
        }

        fn apply_live_settings(&mut self, settings: Settings) -> Result<(), zwo_asi::CameraError> {
            if self.fail_next {
                self.fail_next = false;
                return Err(zwo_asi::CameraError::InvalidState {
                    operation: "injected live settings failure",
                });
            }
            self.inner.apply_live_settings(settings)
        }

        fn start(&mut self) -> Result<(), zwo_asi::CameraError> {
            self.inner.start()
        }

        fn capture_next(
            &mut self,
            wait_ms: i32,
        ) -> Result<zwo_asi::FrameGeneration<'_>, CaptureError> {
            self.inner.capture_next(wait_ms)
        }

        fn stop(&mut self) -> Result<(), zwo_asi::CameraError> {
            self.inner.stop()
        }
    }
}
