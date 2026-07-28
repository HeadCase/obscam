use std::net::SocketAddr;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};

use crate::contract::Treatment;
use crate::media::{CaptureProfile, EncoderCommand, SourceCommand};

#[derive(Debug, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    ApplyCaptureProfile { profile: ProfileRequest },
    RestartEncoder,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileRequest {
    exposure_us: i64,
    gain: i64,
    treatment: TreatmentRequest,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TreatmentRequest {
    Mono,
    Colour,
}

impl From<TreatmentRequest> for Treatment {
    fn from(value: TreatmentRequest) -> Self {
        match value {
            TreatmentRequest::Mono => Self::Mono,
            TreatmentRequest::Colour => Self::Colour,
        }
    }
}

#[derive(Debug, Serialize)]
struct Response {
    status: &'static str,
    settings_generation: Option<u64>,
    treatment: Option<Treatment>,
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
            Ok(Command::ApplyCaptureProfile { profile }) => {
                apply_profile(&source_commands, profile).await
            }
            Ok(Command::RestartEncoder) => restart_encoder(&encoder_commands).await,
            Err(error) => Response {
                status: "rejected",
                settings_generation: None,
                treatment: None,
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

async fn apply_profile(
    commands: &mpsc::Sender<SourceCommand>,
    request: ProfileRequest,
) -> Response {
    let profile =
        match CaptureProfile::new(request.exposure_us, request.gain, request.treatment.into()) {
            Ok(profile) => profile,
            Err(error) => {
                return Response {
                    status: "rejected",
                    settings_generation: None,
                    treatment: None,
                    stream_epoch: None,
                    error: Some(error.to_string()),
                };
            }
        };
    let (reply, receiver) = oneshot::channel();
    if commands
        .send(SourceCommand::ApplyProfile { profile, reply })
        .await
        .is_err()
    {
        return Response {
            status: "failed",
            settings_generation: None,
            treatment: None,
            stream_epoch: None,
            error: Some("camera owner is unavailable".to_owned()),
        };
    }
    match receiver.await {
        Ok(Ok(applied)) => Response {
            status: "applied",
            settings_generation: Some(applied.settings_generation),
            treatment: Some(applied.treatment),
            stream_epoch: None,
            error: None,
        },
        Ok(Err(error)) => Response {
            status: "failed",
            settings_generation: None,
            treatment: None,
            stream_epoch: None,
            error: Some(error),
        },
        Err(_) => Response {
            status: "failed",
            settings_generation: None,
            treatment: None,
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
            treatment: None,
            stream_epoch: None,
            error: Some("encoder supervisor is unavailable".to_owned()),
        };
    }
    match receiver.await {
        Ok(Ok(stream_epoch)) => Response {
            status: "applied",
            settings_generation: None,
            treatment: None,
            stream_epoch: Some(stream_epoch),
            error: None,
        },
        Ok(Err(error)) => Response {
            status: "failed",
            settings_generation: None,
            treatment: None,
            stream_epoch: None,
            error: Some(error),
        },
        Err(_) => Response {
            status: "failed",
            settings_generation: None,
            treatment: None,
            stream_epoch: None,
            error: Some("encoder supervisor dropped the command".to_owned()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    use crate::media::AppliedProfile;

    #[test]
    fn command_schema_rejects_unknown_fields() {
        let json = r#"{"command":"apply_capture_profile","profile":{"exposure_us":50000,"gain":500,"treatment":"mono","guess":true}}"#;
        assert!(serde_json::from_str::<Command>(json).is_err());
    }

    #[test]
    fn command_schema_requires_explicit_treatment() {
        let missing =
            r#"{"command":"apply_capture_profile","profile":{"exposure_us":50000,"gain":500}}"#;
        assert!(serde_json::from_str::<Command>(missing).is_err());
        let colour = r#"{"command":"apply_capture_profile","profile":{"exposure_us":50000,"gain":500,"treatment":"colour"}}"#;
        assert!(serde_json::from_str::<Command>(colour).is_ok());
    }

    #[tokio::test]
    async fn applied_profile_acknowledges_generation_and_treatment() {
        let (sender, mut receiver) = mpsc::channel(1);
        let owner = tokio::spawn(async move {
            let SourceCommand::ApplyProfile { profile: _, reply } =
                receiver.recv().await.expect("profile command");
            reply
                .send(Ok(AppliedProfile {
                    settings_generation: 7,
                    treatment: Treatment::Colour,
                }))
                .expect("qualification reply receiver");
        });
        let response = apply_profile(
            &sender,
            ProfileRequest {
                exposure_us: 50_000,
                gain: 500,
                treatment: TreatmentRequest::Colour,
            },
        )
        .await;
        owner.await.expect("camera owner task");
        assert_eq!(response.status, "applied");
        assert_eq!(response.settings_generation, Some(7));
        assert_eq!(response.treatment, Some(Treatment::Colour));
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
