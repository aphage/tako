#![cfg(windows)]

use std::io;

use tako_ipc::codec::{decode_cbor, encode_cbor, encode_frame};
use tako_ipc::transport::windows_named_pipe::{create_server, open_client};
use tako_ipc::transport::{read_frame_io, write_frame_io};
use tokio::task::JoinHandle;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct DemoValue {
    value: String,
}

#[tokio::test]
async fn named_pipe_transport_roundtrip_frame() -> io::Result<()> {
    let pipe_name = format!(r"\\.\pipe\tako-ipc-test-{}", Uuid::new_v4());
    let server = create_server(&pipe_name, true)?;

    let server_task: JoinHandle<io::Result<DemoValue>> = tokio::spawn(async move {
        server.connect().await?;
        let mut server = server;
        let frame = read_frame_io(&mut server).await?;
        let payload = tako_ipc::codec::decode_frame(&frame)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        let request: DemoValue =
            decode_cbor(payload).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

        let response = DemoValue {
            value: format!("pong:{}", request.value),
        };
        let encoded = encode_cbor(&response)
            .and_then(|payload| encode_frame(&payload))
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        write_frame_io(&mut server, &encoded).await?;
        Ok(request)
    });

    let client_task: JoinHandle<io::Result<DemoValue>> = tokio::spawn(async move {
        let mut client = open_client(&pipe_name)?;
        let encoded = encode_cbor(&DemoValue {
            value: "ping".into(),
        })
        .and_then(|payload| encode_frame(&payload))
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        write_frame_io(&mut client, &encoded).await?;

        let response_frame = read_frame_io(&mut client).await?;
        let payload = tako_ipc::codec::decode_frame(&response_frame)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        decode_cbor(payload).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    });

    let (server_result, client_result) = tokio::try_join!(server_task, client_task)
        .map_err(|err| io::Error::other(err.to_string()))?;

    assert_eq!(
        server_result?,
        DemoValue {
            value: "ping".into(),
        }
    );
    assert_eq!(
        client_result?,
        DemoValue {
            value: "pong:ping".into(),
        }
    );

    Ok(())
}
