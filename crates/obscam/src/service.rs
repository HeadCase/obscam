use std::{io, time::Instant};

use axum::{
    Json, Router,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::Response,
    routing::get,
};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::{
    AuthorityCredentials, AuthorityGate, AuthorityRejection, AuthoritySnapshot, assets,
    authority::LEASE_DURATION_MS,
    runtime::{Components, RuntimeState, SCHEMA_VERSION},
};

/// Serves browser-facing contracts until the listener fails or the task is cancelled.
///
/// # Errors
///
/// Returns the listener error when the HTTP server can no longer accept connections.
pub async fn serve(listener: TcpListener, state: RuntimeState) -> io::Result<()> {
    axum::serve(listener, router(state)).await
}

fn router(state: RuntimeState) -> Router {
    Router::new()
        .route("/", get(assets::index))
        .route("/assets/app.js", get(assets::app))
        .route("/assets/control.js", get(assets::control))
        .route("/assets/model.js", get(assets::model))
        .route("/assets/whep.js", get(assets::whep))
        .route("/assets/styles.css", get(assets::styles))
        .route("/api/v1/runtime", get(runtime))
        .route("/api/v1/health", get(health))
        .route("/api/v1/control", get(control))
        .with_state(state)
}

async fn control(ws: WebSocketUpgrade, State(state): State<RuntimeState>) -> Response {
    ws.on_upgrade(move |socket| control_socket(socket, state.authority()))
}

async fn control_socket(mut socket: WebSocket, authority: AuthorityGate) {
    let mut updates = authority.subscribe();
    if let Err(error) =
        send_server(&mut socket, ServerMessage::authority(authority.snapshot())).await
    {
        tracing::debug!(%error, "control connection closed before initial state");
        return;
    }

    loop {
        tokio::select! {
            update = updates.recv() => {
                let snapshot = match update {
                    Ok(snapshot) => snapshot,
                    Err(broadcast::error::RecvError::Lagged(_)) => authority.snapshot(),
                    Err(broadcast::error::RecvError::Closed) => return,
                };
                if let Err(error) = send_server(&mut socket, ServerMessage::authority(snapshot)).await {
                    tracing::debug!(%error, "control connection closed while broadcasting authority");
                    return;
                }
            }
            incoming = socket.recv() => {
                let message = match incoming {
                    Some(Ok(message)) => message,
                    Some(Err(error)) => {
                        tracing::debug!(%error, "control connection receive failed");
                        return;
                    }
                    None => {
                        tracing::debug!("control connection closed");
                        return;
                    }
                };
                let Message::Text(text) = message else {
                    if matches!(message, Message::Close(_)) {
                        return;
                    }
                    continue;
                };
                let response = handle_client_message(&authority, text.as_str());
                if let Some(expiry) = response.expiry {
                    schedule_expiry(authority.clone(), expiry);
                }
                if let Err(error) = send_server(&mut socket, response.message).await {
                    tracing::debug!(%error, "control connection closed while replying");
                    return;
                }
            }
        }
    }
}

struct ControlResponse {
    message: ServerMessage,
    expiry: Option<(u64, Instant)>,
}

fn handle_client_message(authority: &AuthorityGate, text: &str) -> ControlResponse {
    if text.len() > 4_096 {
        return ControlResponse::rejected(RejectionReason::Malformed);
    }
    let Ok(message) = serde_json::from_str::<ClientMessage>(text) else {
        return ControlResponse::rejected(RejectionReason::Malformed);
    };
    if message.schema_version() != SCHEMA_VERSION {
        return ControlResponse::rejected(RejectionReason::UnsupportedSchema);
    }
    let now = Instant::now();
    match message {
        ClientMessage::Take { .. } => {
            let grant = authority.take(now);
            let deadline = grant.deadline();
            ControlResponse {
                message: ServerMessage::Granted {
                    schema_version: SCHEMA_VERSION,
                    generation: grant.generation(),
                    secret: grant.secret().to_owned(),
                    lease_duration_ms: LEASE_DURATION_MS,
                },
                expiry: Some((grant.generation(), deadline)),
            }
        }
        ClientMessage::Renew {
            generation, secret, ..
        } => {
            let Some(credentials) = credentials(generation, secret) else {
                return ControlResponse::rejected(RejectionReason::Malformed);
            };
            match authority.renew(&credentials, now) {
                Ok(lease) => ControlResponse {
                    message: ServerMessage::Renewed {
                        schema_version: SCHEMA_VERSION,
                        generation: lease.generation(),
                        lease_duration_ms: LEASE_DURATION_MS,
                    },
                    expiry: Some((lease.generation(), lease.deadline())),
                },
                Err(rejection) => ControlResponse::rejected(rejection.into()),
            }
        }
        ClientMessage::Resume {
            generation, secret, ..
        } => {
            let Some(credentials) = credentials(generation, secret) else {
                return ControlResponse::rejected(RejectionReason::Malformed);
            };
            match authority.renew(&credentials, now) {
                Ok(lease) => ControlResponse {
                    message: ServerMessage::Resumed {
                        schema_version: SCHEMA_VERSION,
                        generation: lease.generation(),
                        lease_duration_ms: LEASE_DURATION_MS,
                    },
                    expiry: Some((lease.generation(), lease.deadline())),
                },
                Err(rejection) => ControlResponse::rejected(rejection.into()),
            }
        }
        ClientMessage::Release {
            generation, secret, ..
        } => {
            let Some(credentials) = credentials(generation, secret) else {
                return ControlResponse::rejected(RejectionReason::Malformed);
            };
            match authority.release(&credentials, now) {
                Ok(()) => ControlResponse {
                    message: ServerMessage::Released {
                        schema_version: SCHEMA_VERSION,
                        generation,
                    },
                    expiry: None,
                },
                Err(rejection) => ControlResponse::rejected(rejection.into()),
            }
        }
    }
}

fn credentials(generation: u64, secret: String) -> Option<AuthorityCredentials> {
    (generation > 0 && secret.len() == 64 && secret.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| AuthorityCredentials::new(generation, secret))
}

fn schedule_expiry(authority: AuthorityGate, (generation, deadline): (u64, Instant)) {
    tokio::spawn(async move {
        tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
        authority.expire_deadline(generation, deadline);
    });
}

async fn send_server(socket: &mut WebSocket, message: ServerMessage) -> Result<(), axum::Error> {
    let json = serde_json::to_string(&message).expect("static control message serializes");
    socket.send(Message::Text(json.into())).await
}

impl ControlResponse {
    const fn rejected(reason: RejectionReason) -> Self {
        Self {
            message: ServerMessage::Rejected {
                schema_version: SCHEMA_VERSION,
                reason,
            },
            expiry: None,
        }
    }
}

#[derive(Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
enum ClientMessage {
    Take {
        schema_version: u8,
    },
    Renew {
        schema_version: u8,
        generation: u64,
        secret: String,
    },
    Resume {
        schema_version: u8,
        generation: u64,
        secret: String,
    },
    Release {
        schema_version: u8,
        generation: u64,
        secret: String,
    },
}

impl ClientMessage {
    const fn schema_version(&self) -> u8 {
        match self {
            Self::Take { schema_version }
            | Self::Renew { schema_version, .. }
            | Self::Resume { schema_version, .. }
            | Self::Release { schema_version, .. } => *schema_version,
        }
    }
}

#[derive(Serialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
enum ServerMessage {
    Authority {
        schema_version: u8,
        state: crate::AuthorityState,
        generation: u64,
    },
    Granted {
        schema_version: u8,
        generation: u64,
        secret: String,
        lease_duration_ms: u64,
    },
    Renewed {
        schema_version: u8,
        generation: u64,
        lease_duration_ms: u64,
    },
    Resumed {
        schema_version: u8,
        generation: u64,
        lease_duration_ms: u64,
    },
    Released {
        schema_version: u8,
        generation: u64,
    },
    Rejected {
        schema_version: u8,
        reason: RejectionReason,
    },
}

impl ServerMessage {
    const fn authority(snapshot: AuthoritySnapshot) -> Self {
        Self::Authority {
            schema_version: SCHEMA_VERSION,
            state: snapshot.state(),
            generation: snapshot.generation(),
        }
    }
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum RejectionReason {
    NotHolder,
    Expired,
    Malformed,
    UnsupportedSchema,
}

impl From<AuthorityRejection> for RejectionReason {
    fn from(value: AuthorityRejection) -> Self {
        match value {
            AuthorityRejection::NotHolder => Self::NotHolder,
            AuthorityRejection::Expired => Self::Expired,
        }
    }
}

async fn runtime(State(state): State<RuntimeState>) -> Json<crate::runtime::RuntimeSnapshot> {
    Json(state.snapshot())
}

async fn health(State(state): State<RuntimeState>) -> Json<Health> {
    let snapshot = state.snapshot();
    Json(Health {
        schema_version: SCHEMA_VERSION,
        runtime_epoch: snapshot.runtime_epoch,
        service: ServiceState::Ready,
        components: snapshot.components,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Health {
    schema_version: u8,
    runtime_epoch: Uuid,
    service: ServiceState,
    components: Components,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum ServiceState {
    Ready,
}
