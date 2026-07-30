use std::net::SocketAddr;

use futures_util::{SinkExt, StreamExt};
use obscam::{Config, RuntimeState};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio_tungstenite::{WebSocketStream, connect_async, tungstenite::Message};
use uuid::Uuid;

#[tokio::test]
async fn viewers_take_preempt_reject_stale_credentials_and_resume_same_tab() {
    let address = spawn_service().await;
    let (mut first, _) = connect_async(format!("ws://{address}/api/v1/control"))
        .await
        .expect("first control connection");
    let (mut second, _) = connect_async(format!("ws://{address}/api/v1/control"))
        .await
        .expect("second control connection");

    assert_eq!(
        receive_type(&mut first, "authority").await["state"],
        "unheld"
    );
    assert_eq!(
        receive_type(&mut second, "authority").await["state"],
        "unheld"
    );

    send(&mut first, json!({ "schemaVersion": 1, "type": "take" })).await;
    let first_grant = receive_type(&mut first, "granted").await;
    let first_generation = first_grant["generation"].as_u64().expect("generation");
    let first_secret = first_grant["secret"].as_str().expect("secret").to_owned();

    send(&mut second, json!({ "schemaVersion": 1, "type": "take" })).await;
    let second_grant = receive_type(&mut second, "granted").await;
    let second_generation = second_grant["generation"].as_u64().expect("generation");
    assert!(second_generation > first_generation);
    assert_eq!(
        receive_generation(&mut first, "authority", second_generation).await["state"],
        "held"
    );

    send(
        &mut first,
        json!({
            "schemaVersion": 1,
            "type": "renew",
            "generation": first_generation,
            "secret": first_secret
        }),
    )
    .await;
    assert_eq!(
        receive_type(&mut first, "rejected").await["reason"],
        "not_holder"
    );

    let second_secret = second_grant["secret"].as_str().expect("secret").to_owned();
    second.close(None).await.expect("close second connection");
    let (mut reconnected, _) = connect_async(format!("ws://{address}/api/v1/control"))
        .await
        .expect("same-tab reconnect");
    receive_type(&mut reconnected, "authority").await;
    send(
        &mut reconnected,
        json!({
            "schemaVersion": 1,
            "type": "resume",
            "generation": second_generation,
            "secret": second_secret
        }),
    )
    .await;
    assert_eq!(
        receive_type(&mut reconnected, "resumed").await["generation"],
        second_generation
    );
}

async fn spawn_service() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let bind_address = address.to_string();
    let config = Config::parse(&bind_address, "8889", "/obscam/whep").expect("config");
    let state = RuntimeState::unavailable(Uuid::new_v4(), &config);
    tokio::spawn(async move { obscam::serve(listener, state).await.expect("service") });
    address
}

async fn send<S>(socket: &mut WebSocketStream<S>, value: Value)
where
    WebSocketStream<S>: SinkExt<Message> + Unpin,
    <WebSocketStream<S> as futures_util::Sink<Message>>::Error: std::fmt::Debug,
{
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .expect("send control message");
}

async fn receive_type<S>(socket: &mut WebSocketStream<S>, expected: &str) -> Value
where
    WebSocketStream<S>:
        StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        let message = socket
            .next()
            .await
            .expect("message")
            .expect("websocket message");
        if let Message::Text(text) = message {
            let value: Value = serde_json::from_str(&text).expect("server JSON");
            if value["type"] == expected {
                return value;
            }
        }
    }
}

async fn receive_generation<S>(
    socket: &mut WebSocketStream<S>,
    expected: &str,
    generation: u64,
) -> Value
where
    WebSocketStream<S>:
        StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        let value = receive_type(socket, expected).await;
        if value["generation"] == generation {
            return value;
        }
    }
}
