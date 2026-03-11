use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::api::{CallOptions, Error, IpcAddress, RequestContext, ServiceError};
use crate::codec::{decode_cbor, decode_frame, encode_cbor, encode_frame, CodecError};
use crate::protocol::{
    ErrorBody, MessageType, RequestEnvelope, ResponseEnvelope, DECODE_ERROR, INTERNAL_ERROR,
    INVALID_REQUEST, METHOD_NOT_FOUND, PROTOCOL_VERSION, TIMEOUT,
};
#[cfg(windows)]
use tokio::net::windows::named_pipe::NamedPipeClient;
#[cfg(windows)]
use crate::transport::windows_named_pipe::{create_server, open_client};
#[cfg(windows)]
use crate::transport::{read_frame_io, write_frame_io};

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;
type RawHandler = Arc<
    dyn Fn(RequestContext, &[u8]) -> BoxFuture<Result<Vec<u8>, ServiceError>> + Send + Sync + 'static,
>;

#[derive(Clone)]
pub struct RegisteredHandler {
    inner: RawHandler,
}

impl RegisteredHandler {
    pub fn new<F, Fut, Req, Resp>(handler: F) -> Self
    where
        F: Fn(RequestContext, Req) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Resp, ServiceError>> + Send + 'static,
        Req: DeserializeOwned + Send + 'static,
        Resp: Serialize + Send + 'static,
    {
        let handler = Arc::new(handler);
        let inner: RawHandler = Arc::new(move |ctx, payload| {
            let handler = Arc::clone(&handler);
            let payload = payload.to_vec();
            Box::pin(async move {
                let request = decode_cbor::<Req>(&payload).map_err(|_| ServiceError {
                    code: DECODE_ERROR.into(),
                    message: "failed to decode request payload".into(),
                })?;
                let response = handler(ctx, request).await?;
                encode_cbor(&response).map_err(|_| ServiceError {
                    code: INTERNAL_ERROR.into(),
                    message: "failed to encode response payload".into(),
                })
            })
        });
        Self { inner }
    }

    async fn call(&self, ctx: RequestContext, payload: &[u8]) -> Result<Vec<u8>, ServiceError> {
        (self.inner)(ctx, payload).await
    }
}

pub struct ServerRuntime {
    _addr: IpcAddress,
    handlers: HashMap<String, RegisteredHandler>,
}

impl ServerRuntime {
    pub fn new(addr: IpcAddress) -> Self {
        Self {
            _addr: addr,
            handlers: HashMap::new(),
        }
    }

    pub fn register(&mut self, method: &str, handler: RegisteredHandler) -> Result<(), Error> {
        self.handlers.insert(method.to_string(), handler);
        Ok(())
    }

    pub async fn serve(self) -> Result<(), Error> {
        self.serve_until(std::future::pending()).await
    }

    pub async fn serve_until<S>(self, shutdown: S) -> Result<(), Error>
    where
        S: Future<Output = ()> + Send,
    {
        #[cfg(windows)]
        {
            return self.serve_windows(shutdown).await;
        }

        #[cfg(not(windows))]
        {
            shutdown.await;
            let _ = self;
            Err(Error::ConnectFailed {
                message: "transport not implemented for this platform".into(),
            })
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) async fn handle_frame(&self, frame: &[u8]) -> Result<Vec<u8>, Error> {
        handle_frame_with_handlers(&self.handlers, frame).await
    }
}

#[cfg(windows)]
impl ServerRuntime {
    async fn serve_windows<S>(self, shutdown: S) -> Result<(), Error>
    where
        S: Future<Output = ()> + Send,
    {
        let path = self._addr.normalize();
        let handlers = Arc::new(self.handlers);
        let mut first_instance = true;
        let mut listener = create_server(&path, first_instance).map_err(io_error_to_local)?;
        first_instance = false;
        tokio::pin!(shutdown);

        loop {
            tokio::select! {
                _ = &mut shutdown => return Ok(()),
                result = listener.connect() => {
                    result.map_err(io_error_to_local)?;
                    let connected = listener;
                    listener = create_server(&path, first_instance).map_err(io_error_to_local)?;
                    let handlers = Arc::clone(&handlers);
                    tokio::spawn(async move {
                        let _ = handle_named_pipe_connection(connected, handlers).await;
                    });
                }
            }
        }
    }
}

#[derive(Clone)]
pub struct ClientRuntime {
    addr: IpcAddress,
    state: Arc<Mutex<ClientState>>,
}

#[cfg(windows)]
enum ClientState {
    Disconnected,
    Connected(NamedPipeClient),
}

#[cfg(not(windows))]
enum ClientState {
    Unsupported,
}

impl ClientRuntime {
    pub fn new(addr: IpcAddress) -> Self {
        Self {
            addr,
            state: Arc::new(Mutex::new(client_state_default())),
        }
    }

    pub async fn call<Req, Resp>(
        &self,
        method: &str,
        request: Req,
        options: CallOptions,
    ) -> Result<Resp, Error>
    where
        Req: Serialize,
        Resp: DeserializeOwned,
    {
        let timeout = options.timeout;
        let frame = self.build_request_frame(method, &request, options)?;
        let mut state = self.state.lock().await;

        #[cfg(windows)]
        {
            let result = execute_windows_call(&self.addr, &mut state, &frame, timeout).await;
            match result {
                Ok(response) => return self.decode_response_frame(&response),
                Err(Error::Timeout) => return Err(Error::Timeout),
                Err(err) => return Err(err),
            }
        }

        #[cfg(not(windows))]
        {
            let _ = &mut state;
            let _ = frame;
            Err(Error::ConnectFailed {
                message: "transport not implemented for this platform".into(),
            })
        }
    }

    pub(crate) fn build_request_frame<Req>(
        &self,
        method: &str,
        request: &Req,
        options: CallOptions,
    ) -> Result<Vec<u8>, Error>
    where
        Req: Serialize,
    {
        let payload = encode_cbor(request).map_err(codec_error_to_encode)?;
        let request = RequestEnvelope {
            version: PROTOCOL_VERSION,
            message_type: MessageType::Request,
            request_id: Uuid::new_v4().to_string(),
            method: method.to_string(),
            deadline_ms: options.timeout.and_then(deadline_from_now),
            trace_id: Some(options.trace_id.unwrap_or_else(|| Uuid::new_v4().to_string())),
            payload,
            metadata: None,
        };
        let encoded = encode_cbor(&request).map_err(codec_error_to_encode)?;
        encode_frame(&encoded).map_err(codec_error_to_protocol)
    }

    pub(crate) fn decode_response_frame<Resp>(&self, frame: &[u8]) -> Result<Resp, Error>
    where
        Resp: DeserializeOwned,
    {
        let payload = decode_frame(frame).map_err(codec_error_to_protocol)?;
        let response: ResponseEnvelope = decode_cbor(payload).map_err(codec_error_to_protocol)?;
        response
            .validate()
            .map_err(|message| Error::Protocol {
                code: INVALID_REQUEST.into(),
                message: message.into(),
            })?;

        if let Some(error) = response.error {
            if error.code == INVALID_REQUEST {
                return Err(Error::Protocol {
                    code: error.code,
                    message: error.message,
                });
            }
            return Err(Error::Remote {
                code: error.code,
                message: error.message,
            });
        }

        let payload = response.payload.ok_or_else(|| Error::Protocol {
            code: INVALID_REQUEST.into(),
            message: "successful response missing payload".into(),
        })?;

        decode_cbor(&payload).map_err(|_| Error::Decode {
            message: "failed to decode response payload".into(),
        })
    }
}

fn encode_response_frame(response: &ResponseEnvelope) -> Result<Vec<u8>, Error> {
    let encoded = encode_cbor(response).map_err(codec_error_to_encode)?;
    encode_frame(&encoded).map_err(codec_error_to_protocol)
}

fn deadline_from_now(timeout: Duration) -> Option<u64> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
    let deadline = now.checked_add(timeout)?;
    Some(deadline.as_millis() as u64)
}

fn now_epoch_ms() -> Option<u64> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
    Some(now.as_millis() as u64)
}

fn codec_error_to_protocol(err: CodecError) -> Error {
    Error::Protocol {
        code: INVALID_REQUEST.into(),
        message: err.to_string(),
    }
}

fn codec_error_to_encode(err: CodecError) -> Error {
    Error::Encode {
        message: err.to_string(),
    }
}

fn io_error_to_local(err: std::io::Error) -> Error {
    Error::Io {
        message: err.to_string(),
    }
}

fn client_state_default() -> ClientState {
    #[cfg(windows)]
    {
        ClientState::Disconnected
    }

    #[cfg(not(windows))]
    {
        ClientState::Unsupported
    }
}

#[cfg(windows)]
async fn execute_windows_call(
    addr: &IpcAddress,
    state: &mut ClientState,
    frame: &[u8],
    timeout: Option<Duration>,
) -> Result<Vec<u8>, Error> {
    let client = ensure_connected_client(addr, state).await?;
    let operation = async {
        write_frame_io(client, frame).await.map_err(io_error_to_local)?;
        read_frame_io(client).await.map_err(io_error_to_local)
    };

    match timeout {
        Some(timeout) => match tokio::time::timeout(timeout, operation).await {
            Ok(result) => {
                if result.is_err() {
                    *state = ClientState::Disconnected;
                }
                result
            }
            Err(_) => {
                *state = ClientState::Disconnected;
                Err(Error::Timeout)
            }
        },
        None => {
            let result = operation.await;
            if result.is_err() {
                *state = ClientState::Disconnected;
            }
            result
        }
    }
}

#[cfg(windows)]
async fn ensure_connected_client<'a>(
    addr: &IpcAddress,
    state: &'a mut ClientState,
) -> Result<&'a mut NamedPipeClient, Error> {
    if matches!(state, ClientState::Disconnected) {
        let path = addr.normalize();
        let client = open_named_pipe_client_with_retry(&path).await?;
        *state = ClientState::Connected(client);
    }

    match state {
        ClientState::Connected(client) => Ok(client),
        ClientState::Disconnected => Err(Error::ConnectionClosed),
    }
}

#[cfg(windows)]
async fn open_named_pipe_client_with_retry(path: &str) -> Result<tokio::net::windows::named_pipe::NamedPipeClient, Error> {
    const MAX_ATTEMPTS: usize = 50;
    const PIPE_BUSY: i32 = 231;

    for attempt in 0..MAX_ATTEMPTS {
        match open_client(path) {
            Ok(client) => return Ok(client),
            Err(err)
                if err.kind() == std::io::ErrorKind::NotFound
                    || err.raw_os_error() == Some(PIPE_BUSY) =>
            {
                if attempt + 1 == MAX_ATTEMPTS {
                    return Err(io_error_to_local(err));
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(err) => return Err(io_error_to_local(err)),
        }
    }

    Err(Error::ConnectFailed {
        message: "named pipe client retry loop exhausted".into(),
    })
}

#[cfg(windows)]
async fn handle_named_pipe_connection(
    mut connection: tokio::net::windows::named_pipe::NamedPipeServer,
    handlers: Arc<HashMap<String, RegisteredHandler>>,
) -> Result<(), Error> {
    loop {
        let frame = match read_frame_io(&mut connection).await {
            Ok(frame) => frame,
            Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => return Ok(()),
            Err(err) => return Err(io_error_to_local(err)),
        };
        let response = handle_frame_with_handlers(&handlers, &frame).await?;
        write_frame_io(&mut connection, &response)
            .await
            .map_err(io_error_to_local)?;
    }
}

async fn handle_frame_with_handlers(
    handlers: &HashMap<String, RegisteredHandler>,
    frame: &[u8],
) -> Result<Vec<u8>, Error> {
    let payload = decode_frame(frame).map_err(codec_error_to_protocol)?;
    let request: RequestEnvelope = decode_cbor(payload).map_err(codec_error_to_protocol)?;

    if request.version != PROTOCOL_VERSION
        || request.message_type != MessageType::Request
        || request.request_id.is_empty()
        || request.method.is_empty()
        || request.payload.is_empty()
    {
        return invalid_request_response(&request);
    }

    let ctx = RequestContext {
        request_id: request.request_id.clone(),
        method: request.method.clone(),
        trace_id: request.trace_id.clone(),
        deadline_ms: request.deadline_ms,
    };

    if request
        .deadline_ms
        .is_some_and(|deadline_ms| now_epoch_ms().is_some_and(|now| now >= deadline_ms))
    {
        let response = ResponseEnvelope {
            version: PROTOCOL_VERSION,
            message_type: MessageType::Response,
            request_id: request.request_id,
            ok: false,
            payload: None,
            error: Some(ErrorBody {
                code: TIMEOUT.into(),
                message: "request deadline has expired".into(),
                details: None,
            }),
            trace_id: request.trace_id,
            metadata: None,
        };
        return encode_response_frame(&response);
    }

    let response = match handlers.get(&request.method) {
        Some(handler) => match handler.call(ctx, &request.payload).await {
            Ok(payload) => ResponseEnvelope {
                version: PROTOCOL_VERSION,
                message_type: MessageType::Response,
                request_id: request.request_id,
                ok: true,
                payload: Some(payload),
                error: None,
                trace_id: request.trace_id,
                metadata: None,
            },
            Err(err) => ResponseEnvelope {
                version: PROTOCOL_VERSION,
                message_type: MessageType::Response,
                request_id: request.request_id,
                ok: false,
                payload: None,
                error: Some(ErrorBody {
                    code: err.code,
                    message: err.message,
                    details: None,
                }),
                trace_id: request.trace_id,
                metadata: None,
            },
        },
        None => ResponseEnvelope {
            version: PROTOCOL_VERSION,
            message_type: MessageType::Response,
            request_id: request.request_id,
            ok: false,
            payload: None,
            error: Some(ErrorBody {
                code: METHOD_NOT_FOUND.into(),
                message: "method not registered".into(),
                details: None,
            }),
            trace_id: request.trace_id,
            metadata: None,
        },
    };

    encode_response_frame(&response)
}

fn invalid_request_response(request: &RequestEnvelope) -> Result<Vec<u8>, Error> {
    let response = ResponseEnvelope {
        version: PROTOCOL_VERSION,
        message_type: MessageType::Response,
        request_id: request.request_id.clone(),
        ok: false,
        payload: None,
        error: Some(ErrorBody {
            code: INVALID_REQUEST.into(),
            message: "request envelope is invalid".into(),
            details: None,
        }),
        trace_id: request.trace_id.clone(),
        metadata: None,
    };
    encode_response_frame(&response)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{ClientRuntime, RegisteredHandler, ServerRuntime};
    use crate::api::{CallOptions, Error, IpcAddress, ServiceError};
    use crate::codec::{decode_cbor, encode_cbor, encode_frame};
    use crate::protocol::{MessageType, RequestEnvelope, PROTOCOL_VERSION};

    #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    struct PingRequest {
        value: String,
    }

    #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    struct PingResponse {
        value: String,
    }

    #[tokio::test]
    async fn in_memory_roundtrip_succeeds() {
        let client = ClientRuntime::new(IpcAddress::NamedPipe("demo".into()));
        let mut server = ServerRuntime::new(IpcAddress::NamedPipe("demo".into()));
        server
            .register("ping", RegisteredHandler::new(|_ctx, req: PingRequest| async move {
                Ok(PingResponse { value: req.value })
            }))
            .expect("register should succeed");

        let request = client
            .build_request_frame(
                "ping",
                &PingRequest {
                    value: "hello".into(),
                },
                CallOptions {
                    timeout: Some(Duration::from_secs(1)),
                    trace_id: None,
                },
            )
            .expect("request should encode");

        let response = server
            .handle_frame(&request)
            .await
            .expect("server should respond");

        let decoded: PingResponse = client
            .decode_response_frame(&response)
            .expect("client should decode response");

        assert_eq!(
            decoded,
            PingResponse {
                value: "hello".into(),
            }
        );
    }

    #[tokio::test]
    async fn server_returns_remote_decode_error_for_bad_payload() {
        let client = ClientRuntime::new(IpcAddress::NamedPipe("demo".into()));
        let mut server = ServerRuntime::new(IpcAddress::NamedPipe("demo".into()));
        server
            .register("ping", RegisteredHandler::new(|_ctx, req: PingRequest| async move {
                Ok(PingResponse { value: req.value })
            }))
            .expect("register should succeed");

        let request = client
            .build_request_frame("ping", &"wrong-shape", CallOptions::default())
            .expect("request should encode");

        let response = server
            .handle_frame(&request)
            .await
            .expect("server should return decode error");

        let err = client
            .decode_response_frame::<PingResponse>(&response)
            .expect_err("client should see remote decode error");

        assert_eq!(
            err,
            Error::Remote {
                code: "decode_error".into(),
                message: "failed to decode request payload".into(),
            }
        );
    }

    #[tokio::test]
    async fn server_returns_invalid_request_for_unsupported_version() {
        let client = ClientRuntime::new(IpcAddress::NamedPipe("demo".into()));
        let server = ServerRuntime::new(IpcAddress::NamedPipe("demo".into()));
        let request = RequestEnvelope {
            version: 2,
            message_type: MessageType::Request,
            request_id: "req-1".into(),
            method: "ping".into(),
            deadline_ms: None,
            trace_id: Some("trace-1".into()),
            payload: encode_cbor(&PingRequest {
                value: "hello".into(),
            })
            .expect("payload should encode"),
            metadata: None,
        };
        let encoded = encode_cbor(&request).expect("request should encode");
        let frame = encode_frame(&encoded).expect("frame should encode");

        let response = server
            .handle_frame(&frame)
            .await
            .expect("server should return invalid request response");

        let err = client
            .decode_response_frame::<PingResponse>(&response)
            .expect_err("client should see protocol error");

        assert_eq!(
            err,
            Error::Protocol {
                code: "invalid_request".into(),
                message: "request envelope is invalid".into(),
            }
        );
    }

    #[tokio::test]
    async fn service_error_is_returned_as_remote_error() {
        let client = ClientRuntime::new(IpcAddress::NamedPipe("demo".into()));
        let mut server = ServerRuntime::new(IpcAddress::NamedPipe("demo".into()));
        server
            .register("ping", RegisteredHandler::new(|_ctx, _req: PingRequest| async move {
                Err::<PingResponse, ServiceError>(ServiceError {
                    code: "internal_error".into(),
                    message: "boom".into(),
                })
            }))
            .expect("register should succeed");

        let request = client
            .build_request_frame(
                "ping",
                &PingRequest {
                    value: "hello".into(),
                },
                CallOptions::default(),
            )
            .expect("request should encode");

        let response = server
            .handle_frame(&request)
            .await
            .expect("server should respond");

        let err = client
            .decode_response_frame::<PingResponse>(&response)
            .expect_err("client should see remote error");

        assert_eq!(
            err,
            Error::Remote {
                code: "internal_error".into(),
                message: "boom".into(),
            }
        );
    }

    #[tokio::test]
    async fn expired_deadline_returns_remote_timeout() {
        let client = ClientRuntime::new(IpcAddress::NamedPipe("demo".into()));
        let mut server = ServerRuntime::new(IpcAddress::NamedPipe("demo".into()));
        server
            .register("ping", RegisteredHandler::new(|_ctx, req: PingRequest| async move {
                Ok(PingResponse { value: req.value })
            }))
            .expect("register should succeed");

        let request = client
            .build_request_frame(
                "ping",
                &PingRequest {
                    value: "hello".into(),
                },
                CallOptions {
                    timeout: Some(Duration::ZERO),
                    trace_id: None,
                },
            )
            .expect("request should encode");

        let response = server
            .handle_frame(&request)
            .await
            .expect("server should respond");

        let err = client
            .decode_response_frame::<PingResponse>(&response)
            .expect_err("client should see remote timeout");

        assert_eq!(
            err,
            Error::Remote {
                code: "timeout".into(),
                message: "request deadline has expired".into(),
            }
        );
    }

    #[tokio::test]
    async fn request_frame_contains_generated_trace_id() {
        let client = ClientRuntime::new(IpcAddress::NamedPipe("demo".into()));
        let frame = client
            .build_request_frame(
                "ping",
                &PingRequest {
                    value: "hello".into(),
                },
                CallOptions::default(),
            )
            .expect("request should encode");

        let payload = crate::codec::decode_frame(&frame).expect("frame should decode");
        let request: RequestEnvelope = decode_cbor(payload).expect("envelope should decode");

        assert_eq!(request.version, PROTOCOL_VERSION);
        assert!(request.trace_id.is_some());
        assert!(!request.request_id.is_empty());
    }
}
