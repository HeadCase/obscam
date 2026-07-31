use std::net::SocketAddr;
use std::time::{SystemTime, UNIX_EPOCH};

use obscam::{ComponentReadiness, Config, RuntimeState};
use serde_json::Value;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use uuid::Uuid;

const EPOCH: Uuid = Uuid::from_u128(0x8d4c_c9fd_b91f_4d0c_a2e3_0f38_3860_8262);

#[tokio::test]
async fn unavailable_runtime_contract_omits_unproven_frame_facts() {
    let address = spawn_service().await;

    let response = get_json(address, "/api/v1/runtime").await;

    assert_eq!(response["schemaVersion"], 1);
    assert_eq!(response["runtimeEpoch"], EPOCH.to_string());
    assert_eq!(response["media"]["whepPort"], 8889);
    assert_eq!(response["media"]["whepPath"], "/obscam/whep");
    assert_eq!(response["latestFrame"], Value::Null);
    assert_eq!(response["capture"], Value::Null);
    assert_eq!(response["components"]["capture"]["state"], "unavailable");
    assert_eq!(response["components"]["encoder"]["state"], "unavailable");
    assert_eq!(response["components"]["relay"]["state"], "unavailable");
}

#[tokio::test]
async fn health_is_ready_without_claiming_component_readiness() {
    let address = spawn_service().await;

    let response = get_json(address, "/api/v1/health").await;

    assert_eq!(response["schemaVersion"], 1);
    assert_eq!(response["runtimeEpoch"], EPOCH.to_string());
    assert_eq!(response["service"], "ready");
    assert_eq!(response["components"]["capture"]["state"], "unavailable");
    assert_eq!(response["components"]["encoder"]["state"], "unavailable");
    assert_eq!(response["components"]["relay"]["state"], "unavailable");
}

#[tokio::test]
async fn component_readiness_is_exposed_independently() {
    let config =
        Config::parse("127.0.0.1:8080", "8889", "/obscam/whep").expect("test configuration");
    let state = RuntimeState::with_readiness(
        EPOCH,
        &config,
        ComponentReadiness::Ready,
        ComponentReadiness::Unavailable,
        ComponentReadiness::Ready,
    );
    let address = spawn_state(state).await;

    let response = get_json(address, "/api/v1/runtime").await;

    assert_eq!(response["components"]["capture"]["state"], "ready");
    assert_eq!(response["components"]["encoder"]["state"], "unavailable");
    assert_eq!(response["components"]["relay"]["state"], "ready");
}

#[tokio::test]
async fn browser_reports_unknown_presentation_into_authoritative_scoped_evidence() {
    let address = spawn_service().await;
    let client_id = Uuid::from_u128(42);
    let connection = post_json(
        address,
        "/api/v1/service-quality/connections",
        serde_json::json!({
            "schemaVersion": 1,
            "clientId": client_id,
            "runtimeEpoch": EPOCH,
        }),
    )
    .await;
    assert_eq!(connection["connectionGeneration"], 1);
    assert_eq!(connection["reconnects"], 0);

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("current time")
        .as_micros();
    let report = serde_json::json!({
        "schemaVersion": 1,
        "clientId": client_id,
        "runtimeEpoch": EPOCH,
        "connectionGeneration": 1,
        "samples": [{
            "streamEpoch": null,
            "presentedFrames": 1,
            "presentedAtUnixUs": now,
            "clockUncertaintyUs": 1_000,
            "visibility": "visible",
            "correlation": "unknown",
        }],
    });
    let (head, body) = request_json(address, "POST", "/api/v1/service-quality", &report).await;
    assert!(head.starts_with("HTTP/1.1 204 No Content"), "{head}");
    assert!(body.is_empty());

    let evidence = get_json(
        address,
        &format!("/api/v1/service-quality?clientId={client_id}"),
    )
    .await;
    assert_eq!(evidence["limits"]["clients"], 16);
    assert_eq!(evidence["limits"]["samplesPerClient"], 512);
    assert_eq!(evidence["clients"].as_array().expect("clients").len(), 1);
    assert_eq!(evidence["clients"][0]["sampleCount"], 1);
    assert_eq!(evidence["clients"][0]["exactCorrelation"], 0);
    assert_eq!(evidence["clients"][0]["unknownCorrelation"], 1);
    assert_eq!(
        evidence["clients"][0]["samples"][0]["sourceGeneration"],
        Value::Null
    );
    assert_eq!(
        evidence["clients"][0]["samples"][0]["latencyUs"],
        Value::Null
    );
    assert_eq!(evidence["combined"]["sampleCount"], 1);
}

#[tokio::test]
async fn media_connection_generation_is_explicit_and_stale_reports_are_rejected() {
    let address = spawn_service().await;
    let client_id = Uuid::from_u128(42);
    let connection_request = serde_json::json!({
        "schemaVersion": 1,
        "clientId": client_id,
        "runtimeEpoch": EPOCH,
    });
    post_json(
        address,
        "/api/v1/service-quality/connections",
        connection_request.clone(),
    )
    .await;
    let second = post_json(
        address,
        "/api/v1/service-quality/connections",
        connection_request,
    )
    .await;
    assert_eq!(second["connectionGeneration"], 2);
    assert_eq!(second["reconnects"], 1);

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("current time")
        .as_micros();
    let stale = serde_json::json!({
        "schemaVersion": 1,
        "clientId": client_id,
        "runtimeEpoch": EPOCH,
        "connectionGeneration": 1,
        "samples": [{
            "streamEpoch": null,
            "presentedFrames": 1,
            "presentedAtUnixUs": now,
            "clockUncertaintyUs": 1_000,
            "visibility": "visible",
            "correlation": "unknown",
        }],
    });
    let (head, _) = request_json(address, "POST", "/api/v1/service-quality", &stale).await;
    assert!(head.starts_with("HTTP/1.1 409 Conflict"), "{head}");
}

#[tokio::test]
async fn clock_contract_uses_the_runtime_schema() {
    let address = spawn_service().await;
    let response = get_json(address, "/api/v1/clock").await;
    assert_eq!(response["schemaVersion"], 1);
    assert!(response["serverUnixUs"].as_u64().is_some());
}

#[tokio::test]
async fn production_assets_expose_the_complete_unavailable_viewer_shell() {
    let address = spawn_service().await;

    let (head, html) = get(address, "/").await;
    assert!(head.contains("content-type: text/html"), "{head}");
    assert!(html.contains("data-viewer-status>Unavailable"));
    assert!(html.contains("data-viewer-frame"));
    assert!(html.contains("data-viewer-video"));
    assert!(html.contains("data-service-status"));
    assert!(html.contains("data-service-detail"));
    assert!(html.contains("data-control=\"take-control\""));
    assert!(html.contains("data-control-status"));
    assert!(html.contains("data-control=\"snapshot\""));
    assert!(html.contains("data-control=\"treatment-monochrome\""));
    assert!(html.contains("data-control=\"treatment-colour\""));
    for exposure in [
        "30 s", "20 s", "15 s", "10 s", "5 s", "2 s", "1 s", "500 ms", "300 ms", "200 ms",
        "100 ms", "50 ms", "20 ms", "10 ms",
    ] {
        assert!(html.contains(exposure), "missing exposure {exposure}");
    }
    assert_eq!(html.matches("data-exposure-ms=").count(), 14);
    assert!(html.contains("data-gain"));

    let (script_head, script) = get(address, "/assets/app.js").await;
    assert!(
        script_head.contains("content-type: text/javascript"),
        "{script_head}"
    );
    assert!(script.contains("parseRuntimeContract"));

    let (control_head, control) = get(address, "/assets/control.js").await;
    assert!(
        control_head.contains("content-type: text/javascript"),
        "{control_head}"
    );
    assert!(control.contains("ControlClient"));

    let (presentation_head, presentation) = get(address, "/assets/presentation.js").await;
    assert!(
        presentation_head.contains("content-type: text/javascript"),
        "{presentation_head}"
    );
    assert!(presentation.contains("reducePresentation"));

    let (quality_head, quality) = get(address, "/assets/service-quality.js").await;
    assert!(
        quality_head.contains("content-type: text/javascript"),
        "{quality_head}"
    );
    assert!(quality.contains("ServiceQualityClient"));

    let (style_head, style) = get(address, "/assets/styles.css").await;
    assert!(
        style_head.contains("content-type: text/css"),
        "{style_head}"
    );
    assert!(style.contains(".viewer"));
    assert!(style.contains("object-fit: contain"));

    let (whep_head, whep) = get(address, "/assets/whep.js").await;
    assert!(
        whep_head.contains("content-type: text/javascript"),
        "{whep_head}"
    );
    assert!(whep.contains("RTCPeerConnection"));

    let (viewer_head, viewer) = get(address, "/assets/viewer.js").await;
    assert!(
        viewer_head.contains("content-type: text/javascript"),
        "{viewer_head}"
    );
    assert!(viewer.contains("reduceViewer"));
}

async fn spawn_service() -> SocketAddr {
    let config =
        Config::parse("127.0.0.1:8080", "8889", "/obscam/whep").expect("test configuration");
    let state = RuntimeState::unavailable(EPOCH, &config);
    spawn_state(state).await
}

async fn spawn_state(state: RuntimeState) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    tokio::spawn(async move {
        obscam::serve(listener, state)
            .await
            .expect("test service remains available");
    });
    address
}

async fn get_json(address: SocketAddr, path: &str) -> Value {
    let (head, body) = get(address, path).await;
    assert!(head.starts_with("HTTP/1.1 200 OK"), "{head}");
    serde_json::from_str(&body).expect("JSON response body")
}

async fn post_json(address: SocketAddr, path: &str, body: Value) -> Value {
    let (head, body) = request_json(address, "POST", path, &body).await;
    assert!(head.starts_with("HTTP/1.1 200 OK"), "{head}");
    serde_json::from_str(&body).expect("JSON response body")
}

async fn request_json(
    address: SocketAddr,
    method: &str,
    path: &str,
    body: &Value,
) -> (String, String) {
    let body = serde_json::to_string(body).expect("serialize request");
    let mut stream = TcpStream::connect(address)
        .await
        .expect("connect to test service");
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write request");
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .expect("read response");
    let response = String::from_utf8(response).expect("UTF-8 HTTP response");
    let (head, body) = response.split_once("\r\n\r\n").expect("HTTP response");
    (head.to_owned(), body.to_owned())
}

async fn get(address: SocketAddr, path: &str) -> (String, String) {
    let mut stream = TcpStream::connect(address)
        .await
        .expect("connect to test service");
    let request = format!("GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write request");
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .expect("read response");
    let response = String::from_utf8(response).expect("UTF-8 HTTP response");
    let (head, body) = response.split_once("\r\n\r\n").expect("HTTP response");
    (head.to_owned(), body.to_owned())
}
