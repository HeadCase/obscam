use std::io;

use axum::{Json, Router, extract::State, routing::get};
use serde::Serialize;
use tokio::net::TcpListener;
use uuid::Uuid;

use crate::{
    assets,
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
        .route("/assets/model.js", get(assets::model))
        .route("/assets/whep.js", get(assets::whep))
        .route("/assets/styles.css", get(assets::styles))
        .route("/api/v1/runtime", get(runtime))
        .route("/api/v1/health", get(health))
        .with_state(state)
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
