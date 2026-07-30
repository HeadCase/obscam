use std::{
    fmt::Display,
    io,
    path::Path,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use zwo_asi::{CameraOwner, CameraSource, CaptureError, HEIGHT, WIDTH};

#[cfg(feature = "camera-substitute")]
use zwo_asi::{DeterministicCamera, DeterministicScenario};

use crate::{
    ColourProcessor, ComponentReadiness, FfmpegEncoder, LatestFrameMailbox, MonochromeProcessor,
    RuntimeState, SettingsTarget, Treatment,
    latest::{EpochFence, LatestBufferMailbox},
};

const RAW8_BYTES: usize = WIDTH * HEIGHT;

/// Detached continuously warm capture, processing, and publication workers.
pub struct MediaPipeline {
    _capture: thread::JoinHandle<()>,
    _processing: thread::JoinHandle<()>,
    _encoder: thread::JoinHandle<()>,
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
        let processing =
            spawn_processing(Arc::clone(&raw), Arc::clone(&processed), runtime.settings())?;
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
        })
    }
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

    loop {
        match apply_pending_settings(&mut source, &settings, |generation| {
            raw.begin_epoch(generation);
        }) {
            Ok(SettingsTransition::Applied(_)) => continue,
            Ok(SettingsTransition::Idle) => {}
            Ok(SettingsTransition::Restored { error, .. }) => {
                tracing::warn!(%error, "camera settings transition failed; applied tuple restored");
            }
            Err(restore_error) => {
                runtime.set_capture_readiness(ComponentReadiness::Unavailable);
                tracing::error!(%restore_error, "camera settings recovery failed");
                return;
            }
        }
        let started = Instant::now();
        let epoch = raw.current_epoch();
        match source.capture_next(100) {
            Ok(frame) => raw.publish(epoch, frame.generation(), frame.data()),
            Err(CaptureError::Timeout | CaptureError::Interrupted) => {}
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
pub fn apply_pending_settings(
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
        return Ok(SettingsTransition::Restored { target, error });
    }
    begin_epoch(target.generation());
    controller.mark_applied(target);
    Ok(SettingsTransition::Applied(target))
}

/// Outcome of one bounded settings transition attempt.
#[derive(Debug)]
pub enum SettingsTransition {
    /// No settings target was pending.
    Idle,
    /// The target restarted capture and became Applied.
    Applied(SettingsTarget),
    /// Applying failed, the target was failed, and the prior tuple was restored.
    Restored {
        /// Target that failed before becoming Applied.
        target: SettingsTarget,
        /// Camera error that triggered restoration.
        error: zwo_asi::CameraError,
    },
}

fn spawn_processing(
    raw: Arc<LatestBufferMailbox>,
    processed: Arc<LatestFrameMailbox>,
    settings: crate::SettingsController,
) -> io::Result<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("obscam-processing".into())
        .spawn(move || {
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
                    processed.publish_in_epoch(epoch, &output);
                }
                raw.recycle(source);
            }
        })
}

fn spawn_encoder(
    mailbox: Arc<LatestFrameMailbox>,
    runtime: RuntimeState,
) -> io::Result<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("obscam-encoder".into())
        .spawn(move || {
            let mut encoder = match FfmpegEncoder::start(Path::new("ffmpeg")) {
                Ok(encoder) => encoder,
                Err(error) => {
                    tracing::error!(%error, "qualified FFmpeg hardware encoder unavailable");
                    return;
                }
            };
            loop {
                let Some(frame) = mailbox.wait_take(Duration::from_secs(1)) else {
                    continue;
                };
                let publication =
                    mailbox.commit_if_current(&frame, |current| encoder.publish(current));
                let Some(publication) = publication else {
                    mailbox.recycle(frame);
                    continue;
                };
                if let Err(error) = publication {
                    runtime.set_encoder_readiness(ComponentReadiness::Unavailable);
                    tracing::error!(%error, "FFmpeg hardware publication stopped");
                    mailbox.recycle(frame);
                    return;
                }
                runtime.set_encoder_readiness(ComponentReadiness::Ready);
                mailbox.recycle(frame);
            }
        })
}
