use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::Router;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use serde::Serialize;
use tower_http::trace::TraceLayer;
use tracing::{debug, warn};

use crate::contract::{
    BrowserPresentation, CorrelationResult, CorrelationStatus, RuntimeDescription, SCHEMA_VERSION,
    WhepEndpoint,
};
use crate::state::ProbeState;

const CLIENT_HTML: &str = include_str!("client.html");

#[derive(Clone)]
pub struct AppState {
    pub probe: ProbeState,
    pub whep_port: u16,
    pub whep_path: Arc<str>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/api/runtime", get(runtime))
        .route("/api/clock", get(clock))
        .route("/api/presentations", post(presentation))
        .route("/api/evidence", get(evidence))
        .route("/ws/telemetry", get(telemetry))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn index() -> Html<&'static str> {
    Html(CLIENT_HTML)
}

async fn runtime(State(state): State<AppState>) -> Json<RuntimeDescription> {
    Json(RuntimeDescription {
        schema_version: SCHEMA_VERSION,
        runtime_epoch: state.probe.runtime_epoch().to_owned(),
        stream_epoch: state.probe.stream_epoch(),
        whep: WhepEndpoint {
            port: state.whep_port,
            path: state.whep_path.to_string(),
        },
        capture: state.probe.capture(),
    })
}

#[derive(Serialize)]
struct Clock {
    server_receive_unix_ns: u128,
    server_send_unix_ns: u128,
}

async fn clock() -> Json<Clock> {
    let receive = unix_time_ns();
    Json(Clock {
        server_receive_unix_ns: receive,
        server_send_unix_ns: unix_time_ns(),
    })
}

async fn presentation(
    State(state): State<AppState>,
    Json(presentation): Json<BrowserPresentation>,
) -> Result<Json<CorrelationResult>, (StatusCode, &'static str)> {
    if !presentation.is_valid() {
        return Err((StatusCode::UNPROCESSABLE_ENTITY, "invalid presentation"));
    }
    let result = state.probe.resolve(&presentation);
    if result.status == CorrelationStatus::Correlated {
        debug!(
            client_id = presentation.client_id,
            presented_frames = presentation.presented_frames,
            width = presentation.width,
            height = presentation.height,
            rtp_timestamp = ?result.observed_rtp_timestamp,
            "browser presented correlated frame"
        );
    } else {
        warn!(
            client_id = presentation.client_id,
            presented_frames = presentation.presented_frames,
            width = presentation.width,
            height = presentation.height,
            status = ?result.status,
            rtp_timestamp = ?result.observed_rtp_timestamp,
            "browser frame identity unavailable"
        );
    }
    Ok(Json(result))
}

#[derive(Serialize)]
struct Evidence {
    schema_version: u8,
    runtime_epoch: String,
    mappings: Vec<crate::contract::FrameMapping>,
}

async fn evidence(State(state): State<AppState>) -> Json<Evidence> {
    Json(Evidence {
        schema_version: SCHEMA_VERSION,
        runtime_epoch: state.probe.runtime_epoch().to_owned(),
        mappings: state.probe.snapshot(),
    })
}

async fn telemetry(State(state): State<AppState>, upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(move |socket| telemetry_socket(socket, state))
}

async fn telemetry_socket(mut socket: WebSocket, state: AppState) {
    let initial = serde_json::json!({
        "type": "runtime",
        "schema_version": SCHEMA_VERSION,
        "runtime_epoch": state.probe.runtime_epoch(),
        "stream_epoch": state.probe.stream_epoch(),
        "frames": state.probe.snapshot(),
        "capture": state.probe.capture(),
    });
    if socket
        .send(Message::Text(initial.to_string().into()))
        .await
        .is_err()
    {
        return;
    }
    let mut receiver = state.probe.subscribe();
    let mut capture_receiver = state.probe.subscribe_capture();
    loop {
        tokio::select! {
        mapping = receiver.recv() => match mapping {
            Ok(frame) => {
                let message = serde_json::json!({"type": "frame", "frame": frame});
                if socket
                    .send(Message::Text(message.to_string().into()))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                warn!(skipped, "telemetry client replaced obsolete mappings");
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
        },
        capture = capture_receiver.recv() => match capture {
            Ok(capture) => {
                let message = serde_json::json!({"type": "capture", "capture": capture});
                if socket.send(Message::Text(message.to_string().into())).await.is_err() {
                    return;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                warn!(skipped, "telemetry client replaced obsolete capture progress");
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
        }
        }
    }
}

fn unix_time_ns() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_nanos()
}

impl IntoResponse for RuntimeDescription {
    fn into_response(self) -> Response {
        Json(self).into_response()
    }
}
