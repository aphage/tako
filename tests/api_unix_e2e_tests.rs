#![cfg(unix)]

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use tako_ipc::{CallOptions, Client, Error, IpcAddress, Server};
use tako_ipc::codec::{decode_cbor, encode_cbor, encode_frame};
use tako_ipc::protocol::{MessageType, RequestEnvelope, ResponseEnvelope, PROTOCOL_VERSION};
use tako_ipc::transport::unix::{bind_listener, cleanup_socket_file, connect_stream};
use tako_ipc::transport::{read_frame_io, write_frame_io};
use tokio::io::AsyncWriteExt;
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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct MissingMethodEnvelope {
    version: u16,
    message_type: MessageType,
    request_id: String,
    trace_id: Option<String>,
    payload: Vec<u8>,
}

fn temp_socket_path(label: &str) -> PathBuf {
    std::env::temp_dir()
        .join(format!("tako-ipc-{label}-{}", Uuid::new_v4()))
        .join("ipc.sock")
}

async fn spawn_test_server() -> io::Result<(
    IpcAddress,
    oneshot::Sender<()>,
    tokio::task::JoinHandle<Result<(), Error>>,
)> {
    let path = temp_socket_path("api");
    let addr = IpcAddress::UnixSocket(path.clone());

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

    Ok((addr, shutdown_tx, server_task))
}

#[tokio::test]
async fn unix_public_api_roundtrip() -> io::Result<()> {
    let (addr, shutdown_tx, server_task) = spawn_test_server().await?;
    let client = Client::connect(addr.clone())
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

    shutdown_tx.send(()).map_err(|_| io::Error::other("failed to send shutdown"))?;
    server_task
        .await
        .map_err(|err| io::Error::other(err.to_string()))?
        .map_err(|err| io::Error::other(err.to_string()))?;

    assert_eq!(response.value, "pong:hello");
    if let IpcAddress::UnixSocket(path) = addr {
        let _ = cleanup_socket_file(&path);
    }
    Ok(())
}

#[tokio::test]
async fn unix_public_api_returns_method_not_found() -> io::Result<()> {
    let (addr, shutdown_tx, server_task) = spawn_test_server().await?;
    let client = Client::connect(addr.clone())
        .await
        .map_err(|err| io::Error::other(err.to_string()))?;

    let err = client
        .call::<_, PingResponse>(
            "missing",
            PingRequest {
                value: "hello".into(),
            },
        )
        .await
        .expect_err("missing method should fail");

    shutdown_tx.send(()).map_err(|_| io::Error::other("failed to send shutdown"))?;
    server_task
        .await
        .map_err(|err| io::Error::other(err.to_string()))?
        .map_err(|err| io::Error::other(err.to_string()))?;

    assert_eq!(
        err,
        Error::Remote {
            code: "method_not_found".into(),
            message: "method not registered".into(),
        }
    );
    if let IpcAddress::UnixSocket(path) = addr {
        let _ = cleanup_socket_file(&path);
    }
    Ok(())
}

#[tokio::test]
async fn unix_client_reconnects_after_local_timeout() -> io::Result<()> {
    let path = temp_socket_path("reconnect");
    let addr = IpcAddress::UnixSocket(path.clone());

    let server_task = tokio::spawn(async move {
        let listener = bind_listener(&path)?;
        let (mut first, _) = listener.accept().await?;
        let _ = read_frame_io(&mut first).await?;
        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(first);

        let (mut second, _) = listener.accept().await?;
        let frame = read_frame_io(&mut second).await?;
        let envelope: RequestEnvelope = decode_cbor(
            tako_ipc::codec::decode_frame(&frame)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?,
        )
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        let request: PingRequest = decode_cbor(&envelope.payload)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

        let response = ResponseEnvelope {
            version: PROTOCOL_VERSION,
            message_type: MessageType::Response,
            request_id: envelope.request_id,
            ok: true,
            payload: Some(
                encode_cbor(&PingResponse {
                    value: format!("pong:{}", request.value),
                })
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?,
            ),
            error: None,
            trace_id: envelope.trace_id,
            metadata: None,
        };
        let encoded = encode_cbor(&response)
            .and_then(|payload| encode_frame(&payload))
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        write_frame_io(&mut second, &encoded).await?;
        Ok::<_, io::Error>(())
    });

    let client = Client::connect(addr.clone())
        .await
        .map_err(|err| io::Error::other(err.to_string()))?;

    let timeout_err = client
        .call_with::<_, PingResponse>(
            "ping",
            PingRequest {
                value: "first".into(),
            },
            CallOptions {
                timeout: Some(Duration::from_millis(10)),
                trace_id: None,
            },
        )
        .await
        .expect_err("first call should time out");

    assert_eq!(timeout_err, Error::Timeout);

    let second: PingResponse = client
        .call(
            "ping",
            PingRequest {
                value: "second".into(),
            },
        )
        .await
        .map_err(|err| io::Error::other(err.to_string()))?;

    server_task
        .await
        .map_err(|err| io::Error::other(err.to_string()))??;

    assert_eq!(second.value, "pong:second");
    if let IpcAddress::UnixSocket(path) = addr {
        let _ = cleanup_socket_file(&path);
    }
    Ok(())
}

#[tokio::test]
async fn unix_invalid_length_closes_connection() -> io::Result<()> {
    let (addr, shutdown_tx, server_task) = spawn_test_server().await?;
    let path = match &addr {
        IpcAddress::UnixSocket(path) => path.clone(),
        _ => unreachable!(),
    };
    let mut client = connect_stream(&path).await?;

    client.write_all(&0_u32.to_be_bytes()).await?;

    let err = read_frame_io(&mut client)
        .await
        .expect_err("server should close connection for invalid length");
    assert!(matches!(
        err.kind(),
        io::ErrorKind::UnexpectedEof | io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset
    ));

    shutdown_tx.send(()).map_err(|_| io::Error::other("failed to send shutdown"))?;
    server_task
        .await
        .map_err(|err| io::Error::other(err.to_string()))?
        .map_err(|err| io::Error::other(err.to_string()))?;
    let _ = cleanup_socket_file(&path);
    Ok(())
}

#[tokio::test]
async fn unix_invalid_cbor_closes_connection() -> io::Result<()> {
    let (addr, shutdown_tx, server_task) = spawn_test_server().await?;
    let path = match &addr {
        IpcAddress::UnixSocket(path) => path.clone(),
        _ => unreachable!(),
    };
    let mut client = connect_stream(&path).await?;

    let mut frame = 1_u32.to_be_bytes().to_vec();
    frame.push(0xff);
    write_frame_io(&mut client, &frame).await?;

    let err = read_frame_io(&mut client)
        .await
        .expect_err("server should close connection for invalid cbor");
    assert!(matches!(
        err.kind(),
        io::ErrorKind::UnexpectedEof | io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset
    ));

    shutdown_tx.send(()).map_err(|_| io::Error::other("failed to send shutdown"))?;
    server_task
        .await
        .map_err(|err| io::Error::other(err.to_string()))?
        .map_err(|err| io::Error::other(err.to_string()))?;
    let _ = cleanup_socket_file(&path);
    Ok(())
}

#[tokio::test]
async fn unix_unsupported_version_returns_invalid_request() -> io::Result<()> {
    let (addr, shutdown_tx, server_task) = spawn_test_server().await?;
    let path = match &addr {
        IpcAddress::UnixSocket(path) => path.clone(),
        _ => unreachable!(),
    };
    let mut client = connect_stream(&path).await?;

    let request = RequestEnvelope {
        version: 2,
        message_type: MessageType::Request,
        request_id: "req-version".into(),
        method: "ping".into(),
        deadline_ms: None,
        trace_id: Some("trace-version".into()),
        payload: encode_cbor(&PingRequest {
            value: "hello".into(),
        })
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?,
        metadata: None,
    };
    let frame = encode_cbor(&request)
        .and_then(|payload| encode_frame(&payload))
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    write_frame_io(&mut client, &frame).await?;
    let response_frame = read_frame_io(&mut client).await?;
    let response: ResponseEnvelope = decode_cbor(
        tako_ipc::codec::decode_frame(&response_frame)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?,
    )
    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    assert!(!response.ok);
    assert_eq!(
        response.error.expect("error body should exist").code,
        "invalid_request"
    );

    shutdown_tx.send(()).map_err(|_| io::Error::other("failed to send shutdown"))?;
    server_task
        .await
        .map_err(|err| io::Error::other(err.to_string()))?
        .map_err(|err| io::Error::other(err.to_string()))?;
    let _ = cleanup_socket_file(&path);
    Ok(())
}

#[tokio::test]
async fn unix_missing_required_field_returns_invalid_request() -> io::Result<()> {
    let (addr, shutdown_tx, server_task) = spawn_test_server().await?;
    let path = match &addr {
        IpcAddress::UnixSocket(path) => path.clone(),
        _ => unreachable!(),
    };
    let mut client = connect_stream(&path).await?;

    let request = MissingMethodEnvelope {
        version: PROTOCOL_VERSION,
        message_type: MessageType::Request,
        request_id: "req-missing-method".into(),
        trace_id: Some("trace-missing-method".into()),
        payload: encode_cbor(&PingRequest {
            value: "hello".into(),
        })
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?,
    };
    let frame = encode_cbor(&request)
        .and_then(|payload| encode_frame(&payload))
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    write_frame_io(&mut client, &frame).await?;
    let response_frame = read_frame_io(&mut client).await?;
    let response: ResponseEnvelope = decode_cbor(
        tako_ipc::codec::decode_frame(&response_frame)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?,
    )
    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    assert!(!response.ok);
    assert_eq!(
        response.error.expect("error body should exist").code,
        "invalid_request"
    );

    shutdown_tx.send(()).map_err(|_| io::Error::other("failed to send shutdown"))?;
    server_task
        .await
        .map_err(|err| io::Error::other(err.to_string()))?
        .map_err(|err| io::Error::other(err.to_string()))?;
    let _ = cleanup_socket_file(&path);
    Ok(())
}
