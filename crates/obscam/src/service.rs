use std::{
    io,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    extract::{
        Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::{
    AuthorityCredentials, AuthorityGate, AuthorityRejection, AuthoritySnapshot, CameraSettings,
    CorrelationMapping, SettingsController, SettingsFailure, SettingsSnapshot, Treatment, assets,
    authority::LEASE_DURATION_MS,
    runtime::{
        CaptureProgress, Components, LifecycleSnapshot, RecoveryComponent, RuntimeState,
        SCHEMA_VERSION,
    },
    service_quality::{
        ConnectionRequest, ConnectionResponse, PresentationBatch, ServiceQualityError,
        ServiceQualityResponse,
    },
    settings::SettingsEvent,
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
        .route("/assets/presentation.js", get(assets::presentation))
        .route("/assets/service-quality.js", get(assets::service_quality))
        .route("/assets/whep.js", get(assets::whep))
        .route("/assets/viewer.js", get(assets::viewer))
        .route("/assets/styles.css", get(assets::styles))
        .route("/api/v1/runtime", get(runtime))
        .route("/api/v1/health", get(health))
        .route("/api/v1/clock", get(clock))
        .route(
            "/api/v1/service-quality",
            get(service_quality).post(report_presentation),
        )
        .route(
            "/api/v1/service-quality/connections",
            post(begin_media_connection),
        )
        .route("/api/v1/control", get(control))
        .with_state(state)
}

async fn control(ws: WebSocketUpgrade, State(state): State<RuntimeState>) -> Response {
    ws.on_upgrade(move |socket| control_socket(socket, state))
}

async fn control_socket(mut socket: WebSocket, state: RuntimeState) {
    let authority = state.authority();
    let settings = state.settings();
    let mut updates = authority.subscribe();
    let mut settings_updates = settings.subscribe();
    let mut correlation_updates = state.correlation().subscribe();
    let mut lifecycle_updates = state.subscribe_lifecycle();
    if let Err(error) = send_initial_control_state(&mut socket, &state, &authority, &settings).await
    {
        tracing::debug!(%error, "control connection closed before initial facts");
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
            update = settings_updates.recv() => {
                let message = match update {
                    Ok(event) => settings_event_message(event, &settings),
                    Err(broadcast::error::RecvError::Lagged(_)) => ServerMessage::Settings {
                        schema_version: SCHEMA_VERSION,
                        state: settings.snapshot(),
                    },
                    Err(broadcast::error::RecvError::Closed) => return,
                };
                if let Err(error) = send_server(&mut socket, message).await {
                    tracing::debug!(%error, "control connection closed while broadcasting settings");
                    return;
                }
            }
            update = correlation_updates.recv() => {
                let mapping = match update {
                    Ok(mapping) => mapping,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return,
                };
                let message = ServerMessage::FrameMapping {
                    schema_version: SCHEMA_VERSION,
                    mapping,
                };
                if let Err(error) = send_server(&mut socket, message).await {
                    tracing::debug!(%error, "control connection closed while broadcasting frame mapping");
                    return;
                }
            }
            update = lifecycle_updates.recv() => {
                match update {
                    Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return,
                }
                if let Err(error) = send_server(
                    &mut socket,
                    ServerMessage::lifecycle(state.lifecycle_snapshot()),
                ).await {
                    tracing::debug!(%error, "control connection closed while broadcasting lifecycle facts");
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
                let response = handle_client_message(&authority, &settings, text.as_str());
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

async fn send_initial_control_state(
    socket: &mut WebSocket,
    state: &RuntimeState,
    authority: &AuthorityGate,
    settings: &SettingsController,
) -> Result<(), axum::Error> {
    send_server(socket, ServerMessage::authority(authority.snapshot())).await?;
    send_server(
        socket,
        ServerMessage::Settings {
            schema_version: SCHEMA_VERSION,
            state: settings.snapshot(),
        },
    )
    .await?;
    send_server(socket, ServerMessage::lifecycle(state.lifecycle_snapshot())).await
}

struct ControlResponse {
    message: ServerMessage,
    expiry: Option<(u64, Instant)>,
}

fn handle_client_message(
    authority: &AuthorityGate,
    settings: &SettingsController,
    text: &str,
) -> ControlResponse {
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
        ClientMessage::SetSettings {
            generation,
            secret,
            settings: requested,
            ..
        } => handle_settings_mutation(authority, settings, now, generation, secret, requested),
    }
}

fn handle_settings_mutation(
    authority: &AuthorityGate,
    controller: &SettingsController,
    now: Instant,
    generation: u64,
    secret: String,
    requested: RequestedSettings,
) -> ControlResponse {
    let Some(credentials) = credentials(generation, secret) else {
        return ControlResponse::rejected(RejectionReason::Malformed);
    };
    let Ok(requested) =
        CameraSettings::new(requested.exposure_ms, requested.gain, requested.treatment)
    else {
        return ControlResponse::rejected(RejectionReason::InvalidSettings);
    };
    match authority.accept(&credentials, now, || controller.accept(requested)) {
        Ok(Ok(target)) => ControlResponse {
            message: ServerMessage::Accepted {
                schema_version: SCHEMA_VERSION,
                target_generation: target.generation(),
                settings: target.settings(),
            },
            expiry: None,
        },
        Ok(Err(_)) => ControlResponse::rejected(RejectionReason::CameraUnavailable),
        Err(rejection) => ControlResponse::rejected(rejection.into()),
    }
}

fn settings_event_message(event: SettingsEvent, settings: &SettingsController) -> ServerMessage {
    match event {
        SettingsEvent::Accepted => ServerMessage::Settings {
            schema_version: SCHEMA_VERSION,
            state: settings.snapshot(),
        },
        SettingsEvent::Applied(target) => ServerMessage::Applied {
            schema_version: SCHEMA_VERSION,
            settings_generation: target.generation(),
            settings: target.settings(),
        },
        SettingsEvent::Failed { generation, reason } => ServerMessage::Failed {
            schema_version: SCHEMA_VERSION,
            target_generation: generation,
            reason,
        },
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
    SetSettings {
        schema_version: u8,
        generation: u64,
        secret: String,
        settings: RequestedSettings,
    },
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequestedSettings {
    exposure_ms: u32,
    gain: u16,
    treatment: Treatment,
}

impl ClientMessage {
    const fn schema_version(&self) -> u8 {
        match self {
            Self::Take { schema_version }
            | Self::Renew { schema_version, .. }
            | Self::Resume { schema_version, .. }
            | Self::Release { schema_version, .. }
            | Self::SetSettings { schema_version, .. } => *schema_version,
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
    Lifecycle {
        schema_version: u8,
        runtime_epoch: Uuid,
        components: Components,
        recovery: Option<RecoveryComponent>,
        capture: Option<CaptureProgress>,
    },
    FrameMapping {
        schema_version: u8,
        #[serde(flatten)]
        mapping: CorrelationMapping,
    },
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
    Settings {
        schema_version: u8,
        state: SettingsSnapshot,
    },
    Accepted {
        schema_version: u8,
        target_generation: u64,
        settings: CameraSettings,
    },
    Applied {
        schema_version: u8,
        settings_generation: u64,
        settings: CameraSettings,
    },
    Failed {
        schema_version: u8,
        target_generation: u64,
        reason: SettingsFailure,
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

    const fn lifecycle(snapshot: LifecycleSnapshot) -> Self {
        Self::Lifecycle {
            schema_version: SCHEMA_VERSION,
            runtime_epoch: snapshot.runtime_epoch,
            components: snapshot.components,
            recovery: snapshot.recovery,
            capture: snapshot.capture,
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
    InvalidSettings,
    CameraUnavailable,
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

async fn clock() -> Json<ClockResponse> {
    Json(ClockResponse {
        schema_version: SCHEMA_VERSION,
        server_unix_us: unix_time_us(),
    })
}

async fn begin_media_connection(
    State(state): State<RuntimeState>,
    Json(request): Json<ConnectionRequest>,
) -> Result<Json<ConnectionResponse>, ServiceQualityApiError> {
    state
        .service_quality()
        .begin_connection(request)
        .map(Json)
        .map_err(Into::into)
}

async fn report_presentation(
    State(state): State<RuntimeState>,
    Json(report): Json<PresentationBatch>,
) -> Result<StatusCode, ServiceQualityApiError> {
    state
        .service_quality()
        .record_batch(report, unix_time_us())
        .map(|()| StatusCode::NO_CONTENT)
        .map_err(Into::into)
}

async fn service_quality(
    State(state): State<RuntimeState>,
    Query(query): Query<ServiceQualityQuery>,
) -> Json<ServiceQualityResponse> {
    Json(state.service_quality().response(query.client_id))
}

fn unix_time_us() -> u64 {
    let micros = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros();
    u64::try_from(micros).unwrap_or(u64::MAX)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ClockResponse {
    schema_version: u8,
    server_unix_us: u64,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServiceQualityQuery {
    client_id: Option<Uuid>,
}

struct ServiceQualityApiError(ServiceQualityError);

impl From<ServiceQualityError> for ServiceQualityApiError {
    fn from(value: ServiceQualityError) -> Self {
        Self(value)
    }
}

impl IntoResponse for ServiceQualityApiError {
    fn into_response(self) -> Response {
        let status = match self.0 {
            ServiceQualityError::UnknownClient => StatusCode::NOT_FOUND,
            ServiceQualityError::StaleConnection => StatusCode::CONFLICT,
            ServiceQualityError::UnsupportedSchema
            | ServiceQualityError::RuntimeMismatch
            | ServiceQualityError::InvalidReport => StatusCode::BAD_REQUEST,
        };
        (
            status,
            Json(ErrorResponse {
                schema_version: SCHEMA_VERSION,
                error: self.0.to_string(),
            }),
        )
            .into_response()
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorResponse {
    schema_version: u8,
    error: String,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CorrelationTracker, FrameSubmission};

    #[test]
    fn frame_mapping_is_flattened_into_the_browser_websocket_contract() {
        let runtime_epoch = Uuid::from_u128(1);
        let mut tracker = CorrelationTracker::new(4, 4_500);
        tracker.submit(FrameSubmission::new(
            runtime_epoch,
            2,
            81,
            7,
            Treatment::Monochrome,
            1920,
            1080,
            10_000,
            20_000,
            false,
        ));
        let mapping = tracker.anchor(0, 55_000).expect("exact mapping");

        let value = serde_json::to_value(ServerMessage::FrameMapping {
            schema_version: SCHEMA_VERSION,
            mapping,
        })
        .expect("serialize mapping");

        assert_eq!(value["type"], "frame_mapping");
        assert_eq!(value["runtimeEpoch"], runtime_epoch.to_string());
        assert_eq!(value["streamEpoch"], 2);
        assert_eq!(value["rtpTimestamp"], 55_000);
        assert_eq!(value["sourceGeneration"], 81);
        assert_eq!(value["settingsGeneration"], 7);
    }
}
