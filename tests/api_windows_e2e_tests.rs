#![cfg(windows)]

use std::io;

use tako_ipc::{Client, IpcAddress, Server};
use tokio::sync::oneshot;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct PingRequest {
    value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct PingResponse {
    value: String,
}

#[tokio::test]
async fn public_api_named_pipe_roundtrip() -> io::Result<()> {
    let pipe_name = format!("tako-ipc-api-test-{}", Uuid::new_v4());
    let addr = IpcAddress::NamedPipe(pipe_name);

    let mut server = Server::bind(addr.clone())
        .await
        .map_err(|err| io::Error::other(err.to_string()))?;
    server
        .register("ping", |_ctx, req: PingRequest| async move {
            Ok(PingResponse {
                value: format!("pong:{}", req.value),
            })
        })
        .map_err(|err| io::Error::other(err.to_string()))?;

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server_task = tokio::spawn(async move {
        server
            .serve_until(async {
                let _ = shutdown_rx.await;
            })
            .await
    });

    let client = Client::connect(addr)
        .await
        .map_err(|err| io::Error::other(err.to_string()))?;

    let response: PingResponse = client
        .call(
            "ping",
            PingRequest {
                value: "hello".into(),
            },
        )
        .await
        .map_err(|err| io::Error::other(err.to_string()))?;

    shutdown_tx
        .send(())
        .map_err(|_| io::Error::other("failed to send shutdown signal"))?;
    server_task
        .await
        .map_err(|err| io::Error::other(err.to_string()))?
        .map_err(|err| io::Error::other(err.to_string()))?;

    assert_eq!(
        response,
        PingResponse {
            value: "pong:hello".into(),
        }
    );

    Ok(())
}
