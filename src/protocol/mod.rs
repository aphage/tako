use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_FRAME_SIZE: u32 = 4 * 1024 * 1024;
pub const INVALID_REQUEST: &str = "invalid_request";
pub const METHOD_NOT_FOUND: &str = "method_not_found";
pub const DECODE_ERROR: &str = "decode_error";
pub const TIMEOUT: &str = "timeout";
pub const INTERNAL_ERROR: &str = "internal_error";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageType {
    Request,
    Response,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestEnvelope {
    pub version: u16,
    pub message_type: MessageType,
    pub request_id: String,
    pub method: String,
    pub deadline_ms: Option<u64>,
    pub trace_id: Option<String>,
    pub payload: Vec<u8>,
    pub metadata: Option<std::collections::BTreeMap<String, String>>,
}

impl RequestEnvelope {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.version != PROTOCOL_VERSION {
            return Err("request version is not supported");
        }
        if self.message_type != MessageType::Request {
            return Err("request envelope must have request message type");
        }
        if self.request_id.is_empty() {
            return Err("request_id must not be empty");
        }
        if self.method.is_empty() {
            return Err("method must not be empty");
        }
        if self.payload.is_empty() {
            return Err("payload must not be empty");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
    pub details: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResponseEnvelope {
    pub version: u16,
    pub message_type: MessageType,
    pub request_id: String,
    pub ok: bool,
    pub payload: Option<Vec<u8>>,
    pub error: Option<ErrorBody>,
    pub trace_id: Option<String>,
    pub metadata: Option<std::collections::BTreeMap<String, String>>,
}

impl ResponseEnvelope {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.version != PROTOCOL_VERSION {
            return Err("response version is not supported");
        }
        if self.message_type != MessageType::Response {
            return Err("response envelope must have response message type");
        }
        if self.request_id.is_empty() {
            return Err("request_id must not be empty");
        }
        if self.ok && self.error.is_some() {
            return Err("successful response must not contain error");
        }
        if self.ok && self.payload.is_none() {
            return Err("successful response must contain payload");
        }
        if !self.ok && self.error.is_none() {
            return Err("failed response must contain error");
        }
        if !self.ok && self.payload.is_some() {
            return Err("failed response must not contain payload");
        }
        Ok(())
    }
}
