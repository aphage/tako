use std::io;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::codec::validate_length;

#[cfg(unix)]
pub mod unix;
#[cfg(windows)]
pub mod windows_named_pipe;

pub async fn write_frame_io<W>(writer: &mut W, frame: &[u8]) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    writer.write_all(frame).await?;
    writer.flush().await
}

pub async fn read_frame_io<R>(reader: &mut R) -> io::Result<Vec<u8>>
where
    R: AsyncRead + Unpin,
{
    let mut header = [0_u8; 4];
    reader.read_exact(&mut header).await?;

    let length = u32::from_be_bytes(header);
    validate_length(length).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    let mut payload = vec![0_u8; length as usize];
    reader.read_exact(&mut payload).await?;

    let mut frame = header.to_vec();
    frame.extend_from_slice(&payload);
    Ok(frame)
}
