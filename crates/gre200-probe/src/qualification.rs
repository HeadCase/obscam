use std::net::SocketAddr;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};

use crate::media::{EncoderCommand, Settings, SourceCommand};

#[derive(Debug, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    ApplySettings { exposure_us: i64, gain: i64 },
    RestartEncoder,
}

#[derive(Debug, Serialize)]
struct Response {
    status: &'static str,
    settings_generation: Option<u64>,
    stream_epoch: Option<u64>,
    error: Option<String>,
}

pub async fn serve(
    address: SocketAddr,
    source_commands: mpsc::Sender<SourceCommand>,
    encoder_commands: mpsc::Sender<EncoderCommand>,
) -> Result<()> {
    if !address.ip().is_loopback() {
        bail!("qualification command listener must use a loopback address");
    }
    let listener = TcpListener::bind(address)
        .await
        .with_context(|| format!("binding qualification command listener at {address}"))?;
    info!(%address, "loopback-only qualification command listener ready");
    loop {
        let (stream, peer) = listener
            .accept()
            .await
            .context("accepting qualification command")?;
        if !peer.ip().is_loopback() {
            warn!(%peer, "rejected non-loopback qualification client");
            continue;
        }
        let source_commands = source_commands.clone();
        let encoder_commands = encoder_commands.clone();
        tokio::spawn(async move {
            if let Err(error) = handle(stream, source_commands, encoder_commands).await {
                warn!(%error, "qualification command failed");
            }
        });
    }
}

async fn handle(
    stream: TcpStream,
    source_commands: mpsc::Sender<SourceCommand>,
    encoder_commands: mpsc::Sender<EncoderCommand>,
) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    while let Some(line) = lines
        .next_line()
        .await
        .context("reading qualification command")?
    {
        let response = match serde_json::from_str::<Command>(&line) {
            Ok(Command::ApplySettings { exposure_us, gain }) => {
                apply_settings(&source_commands, exposure_us, gain).await
            }
            Ok(Command::RestartEncoder) => restart_encoder(&encoder_commands).await,
            Err(error) => Response {
                status: "rejected",
                settings_generation: None,
                stream_epoch: None,
                error: Some(error.to_string()),
            },
        };
        let mut encoded = serde_json::to_vec(&response).context("encoding command response")?;
        encoded.push(b'\n');
        writer
            .write_all(&encoded)
            .await
            .context("writing command response")?;
    }
    Ok(())
}

async fn apply_settings(
    commands: &mpsc::Sender<SourceCommand>,
    exposure_us: i64,
    gain: i64,
) -> Response {
    let settings = match Settings::new(exposure_us, gain) {
        Ok(settings) => settings,
        Err(error) => {
            return Response {
                status: "rejected",
                settings_generation: None,
                stream_epoch: None,
                error: Some(error.to_string()),
            };
        }
    };
    let (reply, receiver) = oneshot::channel();
    if commands
        .send(SourceCommand::ApplySettings { settings, reply })
        .await
        .is_err()
    {
        return Response {
            status: "failed",
            settings_generation: None,
            stream_epoch: None,
            error: Some("camera owner is unavailable".to_owned()),
        };
    }
    match receiver.await {
        Ok(Ok(settings_generation)) => Response {
            status: "applied",
            settings_generation: Some(settings_generation),
            stream_epoch: None,
            error: None,
        },
        Ok(Err(error)) => Response {
            status: "failed",
            settings_generation: None,
            stream_epoch: None,
            error: Some(error),
        },
        Err(_) => Response {
            status: "failed",
            settings_generation: None,
            stream_epoch: None,
            error: Some("camera owner dropped the command".to_owned()),
        },
    }
}

async fn restart_encoder(commands: &mpsc::Sender<EncoderCommand>) -> Response {
    let (reply, receiver) = oneshot::channel();
    if commands
        .send(EncoderCommand::Restart { reply })
        .await
        .is_err()
    {
        return Response {
            status: "failed",
            settings_generation: None,
            stream_epoch: None,
            error: Some("encoder supervisor is unavailable".to_owned()),
        };
    }
    match receiver.await {
        Ok(Ok(stream_epoch)) => Response {
            status: "applied",
            settings_generation: None,
            stream_epoch: Some(stream_epoch),
            error: None,
        },
        Ok(Err(error)) => Response {
            status: "failed",
            settings_generation: None,
            stream_epoch: None,
            error: Some(error),
        },
        Err(_) => Response {
            status: "failed",
            settings_generation: None,
            stream_epoch: None,
            error: Some("encoder supervisor dropped the command".to_owned()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    #[test]
    fn command_schema_rejects_unknown_fields() {
        let json = r#"{"command":"apply_settings","exposure_us":50000,"gain":500,"guess":true}"#;
        assert!(serde_json::from_str::<Command>(json).is_err());
    }

    #[tokio::test]
    async fn refuses_non_loopback_listener() {
        let (sender, _) = mpsc::channel(1);
        let (encoder_sender, _) = mpsc::channel(1);
        let result = serve(
            SocketAddr::new(IpAddr::from([0, 0, 0, 0]), 0),
            sender,
            encoder_sender,
        )
        .await;
        assert!(result.unwrap_err().to_string().contains("loopback"));
    }
}
