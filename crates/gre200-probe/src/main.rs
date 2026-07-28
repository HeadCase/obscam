mod contract;
mod media;
mod qualification;
mod server;
mod state;

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use tokio::sync::watch;
use tracing::info;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

use crate::server::AppState;
use crate::state::ProbeState;

#[derive(Debug, Parser)]
#[command(about = "GRE-200 exact H.264 source-to-browser correlation probe")]
struct Args {
    #[arg(long, default_value = "0.0.0.0:8200")]
    listen: SocketAddr,
    #[arg(long, default_value = "127.0.0.1:15001")]
    rtp_observe: SocketAddr,
    #[arg(long, default_value = "238.0.0.1:15000")]
    rtp_forward: SocketAddr,
    #[arg(long, default_value_t = 20)]
    fps: u32,
    #[arg(long, value_enum, default_value_t = SourceKind::Synthetic)]
    source: SourceKind,
    #[arg(long, default_value_t = 10_000)]
    exposure_us: i64,
    #[arg(long, default_value_t = 100)]
    gain: i64,
    #[arg(long, default_value_t = 18_889)]
    whep_port: u16,
    #[arg(long, default_value = "/obscam/whep", value_parser = parse_absolute_path)]
    whep_path: String,
    #[arg(long, default_value = "127.0.0.1:8201")]
    qualification_listen: SocketAddr,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum SourceKind {
    Synthetic,
    Asi,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("gre200_probe=info".parse()?))
        .init();
    let args = Args::parse();
    let state = ProbeState::new(Uuid::new_v4().simple().to_string(), 1);
    let (frame_tx, frame_rx) = watch::channel(None);
    let (command_tx, command_rx) = tokio::sync::mpsc::channel(8);
    let (encoder_command_tx, encoder_command_rx) = tokio::sync::mpsc::channel(4);
    let source_state = state.clone();

    let source = match args.source {
        SourceKind::Synthetic => tokio::spawn(async move {
            media::run_synthetic_source(frame_tx, source_state, args.fps).await;
            Ok(())
        }),
        SourceKind::Asi => tokio::spawn(media::run_asi_source(
            frame_tx,
            source_state,
            command_rx,
            args.exposure_us,
            args.gain,
        )),
    };
    let encoder = tokio::spawn(media::run_encoder(
        frame_rx,
        state.clone(),
        args.fps,
        args.rtp_observe,
        encoder_command_rx,
    ));
    let observer = tokio::spawn(media::run_rtp_observer(
        args.rtp_observe,
        args.rtp_forward,
        state.clone(),
        args.fps,
    ));
    let qualification = tokio::spawn(qualification::serve(
        args.qualification_listen,
        command_tx,
        encoder_command_tx,
    ));

    let app = server::router(AppState {
        probe: state,
        whep_port: args.whep_port,
        whep_path: Arc::from(args.whep_path),
    });
    let listener = tokio::net::TcpListener::bind(args.listen)
        .await
        .with_context(|| format!("binding HTTP server at {}", args.listen))?;
    info!(address = %args.listen, "GRE-200 probe ready");

    tokio::select! {
        result = axum::serve(listener, app).with_graceful_shutdown(shutdown()) => {
            result.context("serving probe")?;
        }
        result = encoder => {
            result.context("joining encoder task")??;
        }
        result = observer => {
            result.context("joining RTP observer task")??;
        }
        result = source => {
            result.context("joining frame source task")??;
        }
        result = qualification => {
            result.context("joining qualification command task")??;
        }
    }
    Ok(())
}

fn parse_absolute_path(value: &str) -> Result<String, String> {
    if !value.starts_with('/') || value.starts_with("//") || value.contains(['?', '#']) {
        return Err(
            "WHEP path must be an absolute path without an authority, query, or fragment"
                .to_owned(),
        );
    }
    Ok(value.to_owned())
}

async fn shutdown() {
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::parse_absolute_path;

    #[test]
    fn accepts_absolute_whep_path() {
        assert_eq!(
            parse_absolute_path("/obscam/whep").as_deref(),
            Ok("/obscam/whep")
        );
    }

    #[test]
    fn rejects_values_that_can_supply_an_authority() {
        for value in [
            "obscam/whep",
            "//10.164.190.1/whep",
            "/whep?host=lan",
            "/whep#lan",
        ] {
            assert!(parse_absolute_path(value).is_err(), "accepted {value}");
        }
    }
}
