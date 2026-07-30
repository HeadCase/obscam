use std::{
    fmt::Display,
    io,
    path::Path,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use zwo_asi::{CameraOwner, CameraSource, CaptureError, HEIGHT, Settings, WIDTH};

#[cfg(feature = "camera-substitute")]
use zwo_asi::{DeterministicCamera, DeterministicScenario};

use crate::{
    ComponentReadiness, FfmpegEncoder, LatestFrameMailbox, MonochromeProcessor, RuntimeState,
    latest::LatestBufferMailbox,
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
        let raw = Arc::new(LatestBufferMailbox::new(RAW8_BYTES));
        let processed = Arc::new(LatestFrameMailbox::new());
        let encoder = spawn_encoder(Arc::clone(&processed), runtime.clone())?;
        let processing = spawn_processing(Arc::clone(&raw), Arc::clone(&processed))?;
        let capture = thread::Builder::new()
            .name(capture_thread_name.into())
            .spawn(move || match connect() {
                Ok(source) => capture(source, &runtime, &raw, minimum_capture_interval),
                Err(error) => tracing::error!(%error, "camera source unavailable"),
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
    let settings = Settings::new(10_000, 100).expect("default settings are validated constants");
    if let Err(error) = source.configure(settings).and_then(|()| source.start()) {
        tracing::error!(%error, "camera capture could not start");
        return;
    }
    runtime.set_capture_readiness(ComponentReadiness::Ready);

    loop {
        let started = Instant::now();
        match source.capture_next(100) {
            Ok(frame) => raw.publish(frame.generation(), frame.data()),
            Err(CaptureError::Timeout | CaptureError::Interrupted) => {}
            Err(error) => {
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

fn spawn_processing(
    raw: Arc<LatestBufferMailbox>,
    processed: Arc<LatestFrameMailbox>,
) -> io::Result<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("obscam-processing".into())
        .spawn(move || {
            let mut processor = MonochromeProcessor::new();
            loop {
                let Some(source) = raw.wait_take(Duration::from_secs(1)) else {
                    continue;
                };
                let generation = source.generation();
                let output = processor.process_validated(generation, source.data());
                if !raw.is_obsolete(generation) {
                    processed.publish(&output);
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
                if mailbox.is_obsolete(frame.generation()) {
                    mailbox.recycle(frame);
                    continue;
                }
                if let Err(error) = encoder.publish(&frame) {
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
