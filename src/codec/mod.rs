use std::io::Cursor;

use ciborium::de::from_reader;
use ciborium::ser::into_writer;
use serde::de::DeserializeOwned;
use serde::Serialize;
use thiserror::Error;

use crate::protocol::MAX_FRAME_SIZE;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CodecError {
    #[error("invalid frame length: {0}")]
    InvalidLength(u32),
    #[error("frame exceeds max size: {0}")]
    FrameTooLarge(u32),
    #[error("cbor decode failed")]
    InvalidCbor,
    #[error("cbor encode failed")]
    EncodeFailed,
    #[error("trailing bytes after cbor document")]
    TrailingBytes,
}

pub fn encode_frame(payload: &[u8]) -> Result<Vec<u8>, CodecError> {
    let len = payload.len() as u32;
    validate_length(len)?;
    let mut frame = Vec::with_capacity(payload.len() + 4);
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(payload);
    Ok(frame)
}

pub fn decode_frame(frame: &[u8]) -> Result<&[u8], CodecError> {
    if frame.len() < 4 {
        return Err(CodecError::InvalidLength(frame.len() as u32));
    }
    let declared = u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]);
    validate_length(declared)?;
    if frame.len() != declared as usize + 4 {
        return Err(CodecError::InvalidLength(declared));
    }
    Ok(&frame[4..])
}

pub fn validate_length(length: u32) -> Result<(), CodecError> {
    if length == 0 {
        return Err(CodecError::InvalidLength(length));
    }
    if length > MAX_FRAME_SIZE {
        return Err(CodecError::FrameTooLarge(length));
    }
    Ok(())
}

pub fn encode_cbor<T>(value: &T) -> Result<Vec<u8>, CodecError>
where
    T: Serialize,
{
    let mut bytes = Vec::new();
    into_writer(value, &mut bytes).map_err(|_| CodecError::EncodeFailed)?;
    validate_length(bytes.len() as u32)?;
    Ok(bytes)
}

pub fn decode_cbor<T>(bytes: &[u8]) -> Result<T, CodecError>
where
    T: DeserializeOwned,
{
    validate_length(bytes.len() as u32)?;
    let mut cursor = Cursor::new(bytes);
    let value = from_reader(&mut cursor).map_err(|_| CodecError::InvalidCbor)?;
    if cursor.position() != bytes.len() as u64 {
        return Err(CodecError::TrailingBytes);
    }
    Ok(value)
}
