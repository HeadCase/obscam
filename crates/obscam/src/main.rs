use std::error::Error;

use obscam::{Config, RuntimeState};
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

    obscam::serve(listener, RuntimeState::unavailable(runtime_epoch, &config)).await?;
    Ok(())
}

fn init_tracing() -> Result<(), Box<dyn Error + Send + Sync>> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .json()
        .try_init()?;
    Ok(())
}
