use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::Serialize;
use thiserror::Error as ThisError;

use crate::runtime::{ClientRuntime, RegisteredHandler, ServerRuntime};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IpcAddress {
    UnixSocket(PathBuf),
    NamedPipe(String),
}

impl IpcAddress {
    pub fn normalize(&self) -> String {
        match self {
            Self::UnixSocket(path) => path.display().to_string(),
            Self::NamedPipe(name) => format!(r"\\.\pipe\{name}"),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CallOptions {
    pub timeout: Option<Duration>,
    pub trace_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestContext {
    pub request_id: String,
    pub method: String,
    pub trace_id: Option<String>,
    pub deadline_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, ThisError, Clone, PartialEq, Eq)]
pub enum Error {
    #[error("connect failed: {message}")]
    ConnectFailed { message: String },
    #[error("permission denied: {message}")]
    PermissionDenied { message: String },
    #[error("i/o error: {message}")]
    Io { message: String },
    #[error("timeout")]
    Timeout,
    #[error("connection closed")]
    ConnectionClosed,
    #[error("protocol error ({code}): {message}")]
    Protocol { code: String, message: String },
    #[error("remote error ({code}): {message}")]
    Remote { code: String, message: String },
    #[error("decode error: {message}")]
    Decode { message: String },
    #[error("encode error: {message}")]
    Encode { message: String },
}

pub struct Server {
    runtime: ServerRuntime,
}

impl Server {
    pub async fn bind(addr: IpcAddress) -> Result<Self, Error> {
        Ok(Self {
            runtime: ServerRuntime::new(addr),
        })
    }

    pub fn register<F, Fut, Req, Resp>(
        &mut self,
        method: &str,
        handler: F,
    ) -> Result<&mut Self, Error>
    where
        F: Fn(RequestContext, Req) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Resp, ServiceError>> + Send + 'static,
        Req: DeserializeOwned + Send + 'static,
        Resp: Serialize + Send + 'static,
    {
        let registered = RegisteredHandler::new(handler);
        self.runtime.register(method, registered)?;
        Ok(self)
    }

    pub async fn serve(self) -> Result<(), Error> {
        self.runtime.serve().await
    }

    pub async fn serve_until<S>(self, shutdown: S) -> Result<(), Error>
    where
        S: Future<Output = ()> + Send,
    {
        self.runtime.serve_until(shutdown).await
    }
}

#[derive(Clone)]
pub struct Client {
    runtime: ClientRuntime,
}

impl Client {
    pub async fn connect(addr: IpcAddress) -> Result<Self, Error> {
        Ok(Self {
            runtime: ClientRuntime::new(addr),
        })
    }

    pub async fn call<Req, Resp>(&self, method: &str, request: Req) -> Result<Resp, Error>
    where
        Req: Serialize,
        Resp: DeserializeOwned,
    {
        self.call_with(method, request, CallOptions::default()).await
    }

    pub async fn call_with<Req, Resp>(
        &self,
        method: &str,
        request: Req,
        options: CallOptions,
    ) -> Result<Resp, Error>
    where
        Req: Serialize,
        Resp: DeserializeOwned,
    {
        self.runtime.call(method, request, options).await
    }
}

