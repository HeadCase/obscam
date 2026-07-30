use std::error::Error;

use obscam::{CameraSourceKind, Config, MediaPipeline, RuntimeState};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    init_tracing()?;
    let config = Config::from_environment().inspect_err(|error| {
        tracing::error!(%error, "configuration rejected");
    })?;
    let runtime_epoch = Uuid::new_v4();
    let listener = TcpListener::bind(config.bind_address()).await?;
    tracing::info!(
        %runtime_epoch,
        bind_address = %config.bind_address(),
        whep_port = config.whep_port(),
        whep_path = config.whep_path(),
        "ObsCam browser service ready"
    );

    let runtime = RuntimeState::unavailable(runtime_epoch, &config);
    let _pipeline = start_pipeline(config.camera_source(), runtime.clone())?;
    obscam::serve(listener, runtime).await?;
    Ok(())
}

fn start_pipeline(
    camera_source: CameraSourceKind,
    runtime: RuntimeState,
) -> Result<MediaPipeline, std::io::Error> {
    match camera_source {
        CameraSourceKind::Production => MediaPipeline::start(runtime),
        CameraSourceKind::Deterministic => {
            #[cfg(feature = "camera-substitute")]
            {
                MediaPipeline::start_deterministic(runtime)
            }
            #[cfg(not(feature = "camera-substitute"))]
            {
                let _ = runtime;
                Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "deterministic camera requires the camera-substitute build feature",
                ))
            }
        }
    }
}

fn init_tracing() -> Result<(), Box<dyn Error + Send + Sync>> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .json()
        .try_init()?;
    Ok(())
}
