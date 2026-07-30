use std::{io, path::Path, sync::Arc, thread, time::Duration};

use zwo_asi::{CameraOwner, CameraSource, CaptureError, Settings};

#[cfg(feature = "camera-substitute")]
use zwo_asi::{DeterministicCamera, DeterministicScenario};

use crate::{
    ComponentReadiness, FfmpegEncoder, LatestFrameMailbox, MonochromeProcessor, RuntimeState,
};

/// Detached continuously warm capture, processing, and publication workers.
pub struct MediaPipeline {
    _capture: thread::JoinHandle<()>,
    _encoder: thread::JoinHandle<()>,
}

impl MediaPipeline {
    /// Starts the production camera owner and the sole hardware encoder.
    ///
    /// # Errors
    ///
    /// Returns an operating-system error when a worker thread cannot be created.
    pub fn start(runtime: RuntimeState) -> io::Result<Self> {
        let mailbox = Arc::new(LatestFrameMailbox::new());
        let encoder = spawn_encoder(Arc::clone(&mailbox), runtime.clone())?;
        let capture = thread::Builder::new()
            .name("obscam-capture".into())
            .spawn(move || match CameraOwner::connect() {
                Ok(source) => capture(source, &runtime, &mailbox),
                Err(error) => tracing::error!(%error, "camera owner unavailable"),
            })?;
        Ok(Self {
            _capture: capture,
            _encoder: encoder,
        })
    }

    /// Starts the explicit development/acceptance camera substitute.
    ///
    /// # Errors
    ///
    /// Returns an operating-system error when a worker thread cannot be created.
    #[cfg(feature = "camera-substitute")]
    pub fn start_deterministic(runtime: RuntimeState) -> io::Result<Self> {
        let mailbox = Arc::new(LatestFrameMailbox::new());
        let encoder = spawn_encoder(Arc::clone(&mailbox), runtime.clone())?;
        let capture = thread::Builder::new()
            .name("obscam-deterministic-capture".into())
            .spawn(
                move || match DeterministicCamera::connect(DeterministicScenario::new([])) {
                    Ok(source) => capture(source, &runtime, &mailbox),
                    Err(error) => tracing::error!(%error, "deterministic camera unavailable"),
                },
            )?;
        Ok(Self {
            _capture: capture,
            _encoder: encoder,
        })
    }
}

fn capture(mut source: impl CameraSource, runtime: &RuntimeState, mailbox: &LatestFrameMailbox) {
    let settings = Settings::new(10_000, 100).expect("default settings are validated constants");
    if let Err(error) = source.configure(settings).and_then(|()| source.start()) {
        tracing::error!(%error, "camera capture could not start");
        return;
    }
    runtime.set_capture_readiness(ComponentReadiness::Ready);
    let mut processor = MonochromeProcessor::new();

    loop {
        match source.capture_next(100) {
            Ok(frame) => mailbox.publish(&processor.process(&frame)),
            Err(CaptureError::Timeout | CaptureError::Interrupted) => {}
            Err(error) => {
                runtime.set_capture_readiness(ComponentReadiness::Unavailable);
                tracing::error!(%error, "camera capture stopped");
                return;
            }
        }
    }
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
