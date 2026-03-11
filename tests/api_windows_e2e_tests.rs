#![cfg(windows)]

use std::io;
use std::time::Duration;

use tako_ipc::{CallOptions, Client, Error, IpcAddress, Server};
use tako_ipc::codec::{decode_cbor, encode_cbor, encode_frame};
use tako_ipc::protocol::{MessageType, RequestEnvelope, ResponseEnvelope, PROTOCOL_VERSION};
use tako_ipc::transport::windows_named_pipe::{create_server, open_client};
use tako_ipc::transport::{read_frame_io, write_frame_io};
use tokio::sync::oneshot;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct PingRequest {
    value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct PingResponse {
    value: String,
}

async fn spawn_test_server() -> io::Result<(
    IpcAddress,
    oneshot::Sender<()>,
    tokio::task::JoinHandle<Result<(), Error>>,
)> {
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

    Ok((addr, shutdown_tx, server_task))
}

async fn open_raw_client(addr: &IpcAddress) -> io::Result<tokio::net::windows::named_pipe::NamedPipeClient> {
    const PIPE_BUSY: i32 = 231;

    for attempt in 0..50 {
        match open_client(&addr.normalize()) {
            Ok(client) => return Ok(client),
            Err(err)
                if err.kind() == io::ErrorKind::NotFound
                    || err.raw_os_error() == Some(PIPE_BUSY) =>
            {
                if attempt == 49 {
                    return Err(err);
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(err) => return Err(err),
        }
    }

    Err(io::Error::other("named pipe client retry loop exhausted"))
}

#[tokio::test]
async fn public_api_named_pipe_roundtrip() -> io::Result<()> {
    let (addr, shutdown_tx, server_task) = spawn_test_server().await?;
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

#[tokio::test]
async fn public_api_returns_method_not_found() -> io::Result<()> {
    let (addr, shutdown_tx, server_task) = spawn_test_server().await?;
    let client = Client::connect(addr)
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

    shutdown_tx
        .send(())
        .map_err(|_| io::Error::other("failed to send shutdown signal"))?;
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

    Ok(())
}

#[tokio::test]
async fn public_api_returns_decode_error() -> io::Result<()> {
    let (addr, shutdown_tx, server_task) = spawn_test_server().await?;
    let client = Client::connect(addr)
        .await
        .map_err(|err| io::Error::other(err.to_string()))?;

    let err = client
        .call::<_, PingResponse>("ping", "wrong-shape")
        .await
        .expect_err("bad payload should fail");

    shutdown_tx
        .send(())
        .map_err(|_| io::Error::other("failed to send shutdown signal"))?;
    server_task
        .await
        .map_err(|err| io::Error::other(err.to_string()))?
        .map_err(|err| io::Error::other(err.to_string()))?;

    assert_eq!(
        err,
        Error::Remote {
            code: "decode_error".into(),
            message: "failed to decode request payload".into(),
        }
    );

    Ok(())
}

#[tokio::test]
async fn public_api_reports_timeout_for_immediate_deadline() -> io::Result<()> {
    let (addr, shutdown_tx, server_task) = spawn_test_server().await?;
    let client = Client::connect(addr)
        .await
        .map_err(|err| io::Error::other(err.to_string()))?;

    let err = client
        .call_with::<_, PingResponse>(
            "ping",
            PingRequest {
                value: "hello".into(),
            },
            CallOptions {
                timeout: Some(Duration::ZERO),
                trace_id: None,
            },
        )
        .await
        .expect_err("expired deadline should fail");

    shutdown_tx
        .send(())
        .map_err(|_| io::Error::other("failed to send shutdown signal"))?;
    server_task
        .await
        .map_err(|err| io::Error::other(err.to_string()))?
        .map_err(|err| io::Error::other(err.to_string()))?;

    assert!(
        matches!(err, Error::Timeout)
            || matches!(
                err,
                Error::Remote { ref code, ref message }
                    if code == "timeout" && message == "request deadline has expired"
            )
    );

    Ok(())
}

#[tokio::test]
async fn client_reuses_connection_for_sequential_calls() -> io::Result<()> {
    let pipe_path = format!(r"\\.\pipe\tako-ipc-reuse-test-{}", Uuid::new_v4());
    let addr = IpcAddress::NamedPipe(pipe_path.trim_start_matches(r"\\.\pipe\").to_string());
    let server = create_server(&pipe_path, true)?;

    let server_task = tokio::spawn(async move {
        let mut server = server;
        server.connect().await?;

        for expected in ["first", "second"] {
            let frame = read_frame_io(&mut server).await?;
            let envelope: RequestEnvelope = decode_cbor(
                tako_ipc::codec::decode_frame(&frame)
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?,
            )
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
            assert_eq!(envelope.message_type, MessageType::Request);
            let request: PingRequest = decode_cbor(&envelope.payload)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
            assert_eq!(request.value, expected);

            let response = ResponseEnvelope {
                version: PROTOCOL_VERSION,
                message_type: MessageType::Response,
                request_id: envelope.request_id,
                ok: true,
                payload: Some(
                    encode_cbor(&PingResponse {
                        value: format!("pong:{expected}"),
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
            write_frame_io(&mut server, &encoded).await?;
        }

        Ok::<_, io::Error>(())
    });

    let client = Client::connect(addr)
        .await
        .map_err(|err| io::Error::other(err.to_string()))?;

    let first: PingResponse = client
        .call(
            "ping",
            PingRequest {
                value: "first".into(),
            },
        )
        .await
        .map_err(|err| io::Error::other(err.to_string()))?;

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

    assert_eq!(first.value, "pong:first");
    assert_eq!(second.value, "pong:second");

    Ok(())
}

#[tokio::test]
async fn client_reconnects_after_local_timeout() -> io::Result<()> {
    let pipe_path = format!(r"\\.\pipe\tako-ipc-reconnect-test-{}", Uuid::new_v4());
    let addr = IpcAddress::NamedPipe(pipe_path.trim_start_matches(r"\\.\pipe\").to_string());

    let server_task = tokio::spawn(async move {
        let mut first = create_server(&pipe_path, true)?;
        first.connect().await?;
        let _first_frame = read_frame_io(&mut first).await?;
        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(first);

        let mut second = create_server(&pipe_path, false)?;
        second.connect().await?;
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

    let client = Client::connect(addr)
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

    Ok(())
}

#[tokio::test]
async fn invalid_length_frame_closes_connection() -> io::Result<()> {
    let (addr, shutdown_tx, server_task) = spawn_test_server().await?;
    let mut client = open_raw_client(&addr).await?;

    client.write_all(&0_u32.to_be_bytes()).await?;

    let err = read_frame_io(&mut client)
        .await
        .expect_err("server should close connection for invalid length");
    assert!(matches!(
        err.kind(),
        io::ErrorKind::UnexpectedEof | io::ErrorKind::BrokenPipe
    ));

    shutdown_tx
        .send(())
        .map_err(|_| io::Error::other("failed to send shutdown signal"))?;
    server_task
        .await
        .map_err(|err| io::Error::other(err.to_string()))?
        .map_err(|err| io::Error::other(err.to_string()))?;

    Ok(())
}

#[tokio::test]
async fn invalid_cbor_frame_closes_connection() -> io::Result<()> {
    let (addr, shutdown_tx, server_task) = spawn_test_server().await?;
    let mut client = open_raw_client(&addr).await?;

    let mut frame = 1_u32.to_be_bytes().to_vec();
    frame.push(0xff);
    write_frame_io(&mut client, &frame).await?;

    let err = read_frame_io(&mut client)
        .await
        .expect_err("server should close connection for invalid cbor");
    assert!(matches!(
        err.kind(),
        io::ErrorKind::UnexpectedEof | io::ErrorKind::BrokenPipe
    ));

    shutdown_tx
        .send(())
        .map_err(|_| io::Error::other("failed to send shutdown signal"))?;
    server_task
        .await
        .map_err(|err| io::Error::other(err.to_string()))?
        .map_err(|err| io::Error::other(err.to_string()))?;

    Ok(())
}

#[tokio::test]
async fn unsupported_version_returns_invalid_request_response() -> io::Result<()> {
    let (addr, shutdown_tx, server_task) = spawn_test_server().await?;
    let mut client = open_raw_client(&addr).await?;

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

    assert_eq!(response.ok, false);
    assert_eq!(
        response.error.expect("error body should exist").code,
        "invalid_request"
    );

    shutdown_tx
        .send(())
        .map_err(|_| io::Error::other("failed to send shutdown signal"))?;
    server_task
        .await
        .map_err(|err| io::Error::other(err.to_string()))?
        .map_err(|err| io::Error::other(err.to_string()))?;

    Ok(())
}
