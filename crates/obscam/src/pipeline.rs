use std::{
    fmt::Display,
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream},
    path::Path,
    sync::Arc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use zwo_asi::{CameraOwner, CameraSource, CaptureError, HEIGHT, WIDTH};

#[cfg(feature = "camera-substitute")]
use zwo_asi::{DeterministicCamera, DeterministicScenario};

use crate::{
    ColourProcessor, ComponentReadiness, FfmpegEncoder, LatestFrameMailbox, MonochromeProcessor,
    RuntimeState, Treatment,
    correlation::CapturedFrameMetadata,
    latest::{EpochFence, LatestBufferMailbox},
};

const RAW8_BYTES: usize = WIDTH * HEIGHT;
const SETTINGS_RECOVERY_RETRY_DELAY: Duration = Duration::from_secs(1);
const MAX_ENCODER_RECOVERY_DELAY: Duration = Duration::from_secs(5);
const ENCODER_PUBLICATION_TIMEOUT: Duration = Duration::from_secs(2);
const RELAY_API_ADDRESS: &str = "169.254.218.2:9997";
const RELAY_PROBE_INTERVAL: Duration = Duration::from_millis(500);
const RELAY_PROBE_TIMEOUT: Duration = Duration::from_millis(250);

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

    fn start_with_source<S, E>(
        capture_thread_name: &str,
        runtime: RuntimeState,
        minimum_capture_interval: Option<Duration>,
        connect: impl FnOnce() -> Result<S, E> + Send + 'static,
    ) -> io::Result<Self>
    where
        S: CameraSource,
        E: Display,
    {
        let epoch_fence = EpochFence::new();
        let raw = Arc::new(LatestBufferMailbox::with_epoch_fence(
            RAW8_BYTES,
            epoch_fence.clone(),
        ));
        let processed = Arc::new(LatestFrameMailbox::with_epoch_fence(epoch_fence));
        let encoder = spawn_encoder(Arc::clone(&processed), runtime.clone())?;
        let relay = spawn_relay_observer(runtime.clone())?;
        let processing =
            spawn_processing(Arc::clone(&raw), Arc::clone(&processed), runtime.clone())?;
        let capture = thread::Builder::new()
            .name(capture_thread_name.into())
            .spawn(move || match connect() {
                Ok(source) => capture(source, &runtime, &raw, minimum_capture_interval),
                Err(error) => {
                    runtime.settings().begin_recovery();
                    runtime.set_capture_readiness(ComponentReadiness::Unavailable);
                    tracing::error!(%error, "camera source unavailable");
                }
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
    let Ok(address) = RELAY_API_ADDRESS.parse::<SocketAddr>() else {
        return false;
    };
    let Ok(mut stream) = TcpStream::connect_timeout(&address, RELAY_PROBE_TIMEOUT) else {
        return false;
    };
    if stream.set_read_timeout(Some(RELAY_PROBE_TIMEOUT)).is_err()
        || stream.set_write_timeout(Some(RELAY_PROBE_TIMEOUT)).is_err()
        || stream
            .write_all(
                b"GET /v3/paths/get/obscam HTTP/1.1\r\nHost: mediamtx\r\nConnection: close\r\n\r\n",
            )
            .is_err()
    {
        return false;
    }
    let mut response = Vec::with_capacity(2_048);
    stream.read_to_end(&mut response).is_ok() && relay_path_is_ready(&response)
}

fn relay_path_is_ready(response: &[u8]) -> bool {
    let Some(boundary) = response.windows(4).position(|window| window == b"\r\n\r\n") else {
        return false;
    };
    if !response.starts_with(b"HTTP/1.1 200 ") {
        return false;
    }
    serde_json::from_slice::<serde_json::Value>(&response[boundary + 4..]).is_ok_and(|body| {
        body.get("name").and_then(serde_json::Value::as_str) == Some("obscam")
            && body.get("ready").and_then(serde_json::Value::as_bool) == Some(true)
    })
}

fn capture(
    mut source: impl CameraSource,
    runtime: &RuntimeState,
    raw: &LatestBufferMailbox,
    minimum_capture_interval: Option<Duration>,
) {
    let settings = runtime.settings();
    settings.install_interrupter(source.interrupter());
    let defaults = settings.snapshot().applied().settings();
    if let Err(error) = source
        .configure(defaults.camera_settings())
        .and_then(|()| source.start())
    {
        settings.begin_recovery();
        runtime.set_capture_readiness(ComponentReadiness::Unavailable);
        tracing::error!(%error, "camera capture could not start");
        return;
    }
    settings.mark_camera_ready();
    runtime.set_capture_readiness(ComponentReadiness::Ready);
    let mut capture_in_progress = false;

    loop {
        match apply_pending_settings(&mut source, &settings, |generation| {
            raw.begin_epoch(generation);
        }) {
            Ok(SettingsTransition::Applied) => continue,
            Ok(SettingsTransition::Idle) => {}
            Ok(SettingsTransition::Restored(error)) => {
                tracing::warn!(%error, "camera settings transition failed; applied tuple restored");
            }
            Err(restore_error) => {
                runtime.set_capture_readiness(ComponentReadiness::Unavailable);
                tracing::error!(%restore_error, "camera settings recovery failed");
                restore_applied_settings(&mut source, &settings);
                runtime.set_capture_readiness(ComponentReadiness::Ready);
            }
        }
        if !capture_in_progress {
            let applied = settings.snapshot().applied();
            runtime.capture_started(
                applied.generation(),
                applied.settings().exposure_ms(),
                unix_time_us(),
            );
            capture_in_progress = true;
        }
        let started = Instant::now();
        let epoch = raw.current_epoch();
        match source.capture_next(100) {
            Ok(frame) => {
                let applied = settings.snapshot().applied();
                let skipped = raw.publish_with_metadata(
                    epoch,
                    frame.generation(),
                    frame.data(),
                    Some(CapturedFrameMetadata {
                        settings_generation: applied.generation(),
                        treatment: applied.settings().treatment(),
                        exposure_completed_at_unix_us: unix_time_us(),
                    }),
                );
                runtime.record_pipeline_skips(skipped);
                capture_in_progress = false;
            }
            Err(CaptureError::Timeout) => {}
            Err(CaptureError::Interrupted) => capture_in_progress = false,
            Err(error) => {
                settings.begin_recovery();
                runtime.set_capture_readiness(ComponentReadiness::Unavailable);
                tracing::error!(%error, "camera capture stopped");
                return;
            }
        }
        if let Some(interval) = minimum_capture_interval {
            thread::sleep(interval.saturating_sub(started.elapsed()));
        }
    }
}

/// Applies at most one claimed settings target through the camera-owner lifecycle.
///
/// The boundary callback is invoked exactly once after capture restarts and before
/// the target becomes authoritative as Applied.
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
    let transition = source
        .stop()
        .and_then(|()| source.configure(target.settings().camera_settings()))
        .and_then(|()| source.start());
    if let Err(error) = transition {
        controller.begin_recovery();
        source.stop()?;
        source.configure(previous.camera_settings())?;
        source.start()?;
        controller.mark_camera_ready();
        return Ok(SettingsTransition::Restored(error));
    }
    begin_epoch(target.generation());
    controller.mark_applied(target);
    Ok(SettingsTransition::Applied)
}

/// Outcome of one bounded settings transition attempt.
#[derive(Debug)]
enum SettingsTransition {
    /// No settings target was pending.
    Idle,
    /// The target restarted capture and became Applied.
    Applied,
    /// Applying failed, the target was failed, and the prior tuple was restored.
    Restored(zwo_asi::CameraError),
}

fn restore_applied_settings(
    source: &mut impl CameraSource,
    controller: &crate::SettingsController,
) {
    loop {
        let applied = controller.snapshot().applied().settings();
        let restoration = source
            .stop()
            .and_then(|()| source.configure(applied.camera_settings()))
            .and_then(|()| source.start());
        match restoration {
            Ok(()) => {
                controller.mark_camera_ready();
                tracing::info!("camera settings recovery completed");
                return;
            }
            Err(error) => {
                tracing::error!(%error, "camera settings recovery retry failed");
                thread::sleep(SETTINGS_RECOVERY_RETRY_DELAY);
            }
        }
    }
}

fn spawn_processing(
    raw: Arc<LatestBufferMailbox>,
    processed: Arc<LatestFrameMailbox>,
    runtime: RuntimeState,
) -> io::Result<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("obscam-processing".into())
        .spawn(move || {
            let settings = runtime.settings();
            let mut processor = MonochromeProcessor::new();
            let mut colour_processor = ColourProcessor::new();
            loop {
                let Some(source) = raw.wait_take(Duration::from_secs(1)) else {
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
                if raw.is_current_epoch(epoch) {
                    let skipped = if let Some(metadata) = source.metadata() {
                        processed.publish_captured(epoch, &output, metadata)
                    } else {
                        processed.publish_in_epoch(epoch, &output)
                    };
                    runtime.record_pipeline_skips(skipped);
                }
                raw.recycle(source);
            }
        })
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
                        }
                        Err(error) => {
                            runtime.set_encoder_readiness(ComponentReadiness::Unavailable);
                            tracing::error!(%error, "qualified FFmpeg hardware encoder unavailable; retrying");
                            recovering = true;
                            continue;
                        }
                    }
                }
                let next = mailbox.wait_take(if completed.is_some() {
                    Duration::from_millis(500)
                } else {
                    Duration::from_secs(1)
                });
                if let Some(frame) = next {
                    if let Some(previous) = completed.take() {
                        mailbox.recycle(previous);
                    }
                    let publication = mailbox.commit_owned_if_current(frame, |current| {
                        encoder
                            .as_mut()
                            .expect("current media epoch has an encoder")
                            .publish_owned(current, false, ENCODER_PUBLICATION_TIMEOUT)
                    });
                    let publication = match publication {
                        Ok(publication) => publication,
                        Err(frame) => {
                            mailbox.recycle(frame);
                            continue;
                        }
                    };
                    match publication {
                        Ok(frame) => {
                            runtime.set_encoder_readiness(ComponentReadiness::Ready);
                            completed = Some(frame);
                            recovering = false;
                            backoff.reset();
                        }
                        Err(failure) => {
                            let (frame, error) = failure.into_parts();
                            runtime.set_encoder_readiness(ComponentReadiness::Unavailable);
                            tracing::error!(%error, "FFmpeg hardware publication stopped; replacing encoder");
                            completed = Some(frame);
                            encoder.take();
                            recovering = true;
                            runtime.record_pipeline_skips(mailbox.begin_new_epoch());
                        }
                    }
                } else if let Some(frame) = completed.take() {
                    match encoder
                        .as_mut()
                        .expect("completed frame has an encoder")
                        .publish_owned(frame, true, ENCODER_PUBLICATION_TIMEOUT)
                    {
                        Ok(frame) => {
                            runtime.set_encoder_readiness(ComponentReadiness::Ready);
                            completed = Some(frame);
                            recovering = false;
                            backoff.reset();
                        }
                        Err(failure) => {
                            let (frame, error) = failure.into_parts();
                            runtime.set_encoder_readiness(ComponentReadiness::Unavailable);
                            tracing::error!(%error, "FFmpeg hardware repeat stopped; replacing encoder");
                            completed = Some(frame);
                            encoder.take();
                            recovering = true;
                            runtime.record_pipeline_skips(mailbox.begin_new_epoch());
                        }
                    }
                }
            }
        })
}

#[cfg(all(test, feature = "camera-substitute"))]
mod tests {
    use super::*;
    use crate::{CameraSettings, SettingsController};
    use zwo_asi::{DeterministicCamera, DeterministicScenario, Settings};

    #[test]
    fn repeated_encoder_failures_back_off_with_a_five_second_cap() {
        let mut backoff = EncoderRecoveryBackoff::new();

        assert_eq!(backoff.next_delay_with_jitter(0), Duration::ZERO);
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
    fn relay_probe_accepts_only_a_ready_obscam_path() {
        let ready = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"name\":\"obscam\",\"ready\":true}";
        let unavailable = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"name\":\"obscam\",\"ready\":false}";

        assert!(relay_path_is_ready(ready));
        assert!(!relay_path_is_ready(unavailable));
        assert!(!relay_path_is_ready(b"HTTP/1.1 404 Not Found\r\n\r\n{}"));
    }

    #[test]
    fn pending_tuple_runs_the_complete_camera_and_generation_transition() {
        let mut camera =
            DeterministicCamera::connect(DeterministicScenario::new([])).expect("camera present");
        camera
            .configure(Settings::new(500_000, 100).expect("defaults"))
            .expect("configure");
        camera.start().expect("start");
        let controller = ready_controller();
        controller.install_interrupter(camera.interrupter());
        let target = controller
            .accept(CameraSettings::new(20, 200, Treatment::Colour).expect("target"))
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
        assert!(matches!(transition, SettingsTransition::Applied));

        assert_eq!(boundaries, [target.generation()]);
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
        let mut camera = FailNextConfiguration::new(inner);
        let controller = ready_controller();
        controller.install_interrupter(camera.interrupter());
        let target = controller
            .accept(CameraSettings::new(20, 600, Treatment::Colour).expect("target"))
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
    fn failed_compensating_restoration_retries_until_the_previous_tuple_is_live() {
        let mut inner =
            DeterministicCamera::connect(DeterministicScenario::new([])).expect("camera present");
        inner
            .configure(Settings::new(500_000, 100).expect("defaults"))
            .expect("configure");
        inner.start().expect("start");
        let mut camera = FailNextConfigurations::new(inner, 3);
        let controller = ready_controller();
        controller.install_interrupter(camera.interrupter());
        let _target = controller
            .accept(CameraSettings::new(20, 600, Treatment::Colour).expect("target"))
            .expect("camera ready");

        assert!(apply_pending_settings(&mut camera, &controller, |_| {}).is_err());
        restore_applied_settings(&mut camera, &controller);

        assert!(
            controller
                .accept(CameraSettings::new(50, 200, Treatment::Monochrome).expect("next target"))
                .is_ok(),
            "successful restoration makes the camera ready for new intent"
        );
        let frame = loop {
            match camera.capture_next(100) {
                Ok(frame) => break frame,
                Err(CaptureError::Interrupted | CaptureError::Timeout) => {}
                Err(error) => panic!("restored capture failed: {error}"),
            }
        };
        assert_eq!(frame.data()[258 * WIDTH + 258], 166, "gain 100 restored");
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

    struct FailNextConfigurations {
        inner: DeterministicCamera,
        remaining: usize,
    }

    impl FailNextConfigurations {
        const fn new(inner: DeterministicCamera, remaining: usize) -> Self {
            Self { inner, remaining }
        }
    }

    impl CameraSource for FailNextConfigurations {
        fn interrupter(&self) -> zwo_asi::CaptureInterrupter {
            self.inner.interrupter()
        }

        fn configure(&mut self, settings: Settings) -> Result<(), zwo_asi::CameraError> {
            if self.remaining > 0 {
                self.remaining -= 1;
                return Err(zwo_asi::CameraError::InvalidState {
                    operation: "injected repeated settings failure",
                });
            }
            self.inner.configure(settings)
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

    impl FailNextConfiguration {
        const fn new(inner: DeterministicCamera) -> Self {
            Self {
                inner,
                fail_next: true,
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
                    operation: "injected settings failure",
                });
            }
            self.inner.configure(settings)
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
