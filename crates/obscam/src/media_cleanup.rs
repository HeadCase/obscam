use std::{io, net::SocketAddr, time::Duration};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::timeout,
};
use uuid::Uuid;

const MEDIAMTX_WHEP_ADDRESS: &str = "169.254.218.2:8889";
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) fn spawn(whep_path: String, session_id: Uuid) {
    tokio::spawn(async move {
        let address = MEDIAMTX_WHEP_ADDRESS
            .parse::<SocketAddr>()
            .expect("fixed MediaMTX WHEP address");
        match timeout(
            CLEANUP_TIMEOUT,
            delete_session(address, &whep_path, session_id),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::debug!(%error, %session_id, "WHEP session cleanup failed");
            }
            Err(_) => {
                tracing::debug!(%session_id, "WHEP session cleanup timed out");
            }
        }
    });
}

async fn delete_session(address: SocketAddr, whep_path: &str, session_id: Uuid) -> io::Result<()> {
    let resource = format!("{}/{session_id}", whep_path.trim_end_matches('/'));
    let mut stream = TcpStream::connect(address).await?;
    let request = format!(
        "DELETE {resource} HTTP/1.1\r\nHost: mediamtx\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await?;

    let mut response = [0_u8; 512];
    let length = stream.read(&mut response).await?;
    let head = &response[..length];
    if head.starts_with(b"HTTP/1.1 2") || head.starts_with(b"HTTP/1.1 404 ") {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "MediaMTX rejected WHEP session cleanup",
        ))
    }
}

#[cfg(test)]
mod tests {
    use tokio::net::TcpListener;

    use super::*;

    #[tokio::test]
    async fn deletes_only_the_validated_session_resource() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind relay");
        let address = listener.local_addr().expect("relay address");
        let session_id = Uuid::from_u128(42);
        let relay = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept cleanup");
            let mut request = [0_u8; 512];
            let length = stream.read(&mut request).await.expect("read cleanup");
            stream
                .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
                .await
                .expect("write response");
            String::from_utf8(request[..length].to_vec()).expect("HTTP request")
        });

        delete_session(address, "/obscam/whep", session_id)
            .await
            .expect("cleanup succeeds");

        let request = relay.await.expect("relay task");
        assert!(
            request.starts_with(&format!("DELETE /obscam/whep/{session_id} HTTP/1.1\r\n")),
            "{request}"
        );
    }
}
