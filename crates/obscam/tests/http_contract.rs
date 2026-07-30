use std::net::SocketAddr;

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
async fn production_assets_expose_the_complete_unavailable_viewer_shell() {
    let address = spawn_service().await;

    let (head, html) = get(address, "/").await;
    assert!(head.contains("content-type: text/html"), "{head}");
    assert!(html.contains("data-viewer-status>Unavailable"));
    assert!(html.contains("data-viewer-frame"));
    assert!(html.contains("data-viewer-video"));
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
