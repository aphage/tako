use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::Mutex;
use tracing::{info, warn};
use uuid::Uuid;

use crate::api::{CallOptions, Error, IpcAddress, RequestContext, ServiceError};
use crate::codec::{CodecError, decode_cbor, decode_frame, encode_cbor, encode_frame};
use crate::observability;
use crate::protocol::{
    DECODE_ERROR, ErrorBody, INTERNAL_ERROR, INVALID_REQUEST, METHOD_NOT_FOUND, MessageType,
    PROTOCOL_VERSION, RequestEnvelope, ResponseEnvelope, TIMEOUT,
};
#[cfg(unix)]
use crate::transport::unix::{bind_listener, cleanup_socket_file, connect_stream};
#[cfg(windows)]
use crate::transport::windows_named_pipe::{create_server, open_client};
#[cfg(unix)]
use tokio::net::UnixStream;
#[cfg(windows)]
use tokio::net::windows::named_pipe::NamedPipeClient;
type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;
type RawHandler = Arc<
    dyn Fn(RequestContext, &[u8]) -> BoxFuture<Result<Vec<u8>, ServiceError>>
        + Send
        + Sync
        + 'static,
>;

#[derive(Debug, Deserialize)]
struct LooseRequestEnvelope {
    version: Option<u16>,
    message_type: Option<MessageType>,
    request_id: Option<String>,
    method: Option<String>,
    deadline_ms: Option<u64>,
    trace_id: Option<String>,
    payload: Option<Vec<u8>>,
}

enum ParseRequestEnvelopeError {
    Protocol(Error),
    Invalid(RequestEnvelope),
}

struct RequestMeta {
    request_id: String,
    method: String,
    trace_id: Option<String>,
    deadline_ms: Option<u64>,
}

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

        #[cfg(unix)]
        {
            return self.serve_unix(shutdown).await;
        }

        #[cfg(not(any(windows, unix)))]
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
        handle_frame_with_handlers(&self.handlers, frame, "test-connection").await
    }
}

#[cfg(unix)]
impl ServerRuntime {
    async fn serve_unix<S>(self, shutdown: S) -> Result<(), Error>
    where
        S: Future<Output = ()> + Send,
    {
        let path = match &self._addr {
            IpcAddress::UnixSocket(path) => path.clone(),
            _ => {
                return Err(Error::ConnectFailed {
                    message: "unix runtime requires unix socket address".into(),
                });
            }
        };
        let handlers = Arc::new(self.handlers);
        let listener = bind_listener(&path).map_err(io_error_to_local)?;
        tokio::pin!(shutdown);

        loop {
            tokio::select! {
                _ = &mut shutdown => {
                    let _ = cleanup_socket_file(&path);
                    return Ok(());
                }
                accepted = listener.accept() => {
                    let (stream, _) = accepted.map_err(io_error_to_local)?;
                    let handlers = Arc::clone(&handlers);
                    tokio::spawn(async move {
                        let _ = handle_unix_connection(stream, handlers).await;
                    });
                }
            }
        }
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

#[cfg(unix)]
enum ClientState {
    Disconnected,
    Connected(UnixStream),
}

#[cfg(not(any(windows, unix)))]
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
        let meta = decode_request_metadata(&frame)?;
        log_client_call_start(&meta, platform_name());
        let mut state = self.state.lock().await;

        #[cfg(windows)]
        {
            let result = execute_windows_call(&self.addr, &mut state, &frame, timeout).await;
            match result {
                Ok(response) => {
                    let decoded = self.decode_response_frame(&response);
                    match &decoded {
                        Ok(_) => log_client_call_finish(&meta, None, platform_name()),
                        Err(err) => {
                            log_client_call_finish(&meta, error_code_of(err), platform_name())
                        }
                    }
                    return decoded;
                }
                Err(Error::Timeout) => {
                    log_client_call_timeout(&meta, platform_name());
                    return Err(Error::Timeout);
                }
                Err(err) => {
                    log_client_call_finish(&meta, error_code_of(&err), platform_name());
                    return Err(err);
                }
            }
        }

        #[cfg(unix)]
        {
            let result = execute_unix_call(&self.addr, &mut state, &frame, timeout).await;
            match result {
                Ok(response) => {
                    let decoded = self.decode_response_frame(&response);
                    match &decoded {
                        Ok(_) => log_client_call_finish(&meta, None, platform_name()),
                        Err(err) => {
                            log_client_call_finish(&meta, error_code_of(err), platform_name())
                        }
                    }
                    return decoded;
                }
                Err(Error::Timeout) => {
                    log_client_call_timeout(&meta, platform_name());
                    return Err(Error::Timeout);
                }
                Err(err) => {
                    log_client_call_finish(&meta, error_code_of(&err), platform_name());
                    return Err(err);
                }
            }
        }

        #[cfg(not(any(windows, unix)))]
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
            trace_id: Some(
                options
                    .trace_id
                    .unwrap_or_else(|| Uuid::new_v4().to_string()),
            ),
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
        response.validate().map_err(|message| Error::Protocol {
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

    #[cfg(unix)]
    {
        ClientState::Disconnected
    }

    #[cfg(not(any(windows, unix)))]
    {
        ClientState::Unsupported
    }
}

#[cfg(unix)]
async fn execute_unix_call(
    addr: &IpcAddress,
    state: &mut ClientState,
    frame: &[u8],
    timeout: Option<Duration>,
) -> Result<Vec<u8>, Error> {
    let client = ensure_connected_unix_client(addr, state).await?;
    let operation = async {
        crate::transport::write_frame_io(client, frame)
            .await
            .map_err(io_error_to_local)?;
        let response: Vec<u8> = crate::transport::read_frame_io(client)
            .await
            .map_err(io_error_to_local)?;
        Ok::<Vec<u8>, Error>(response)
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

#[cfg(unix)]
async fn ensure_connected_unix_client<'a>(
    addr: &IpcAddress,
    state: &'a mut ClientState,
) -> Result<&'a mut UnixStream, Error> {
    if matches!(state, ClientState::Disconnected) {
        let path = match addr {
            IpcAddress::UnixSocket(path) => path,
            _ => {
                return Err(Error::ConnectFailed {
                    message: "unix runtime requires unix socket address".into(),
                });
            }
        };
        let client = connect_stream(path).await.map_err(io_error_to_local)?;
        *state = ClientState::Connected(client);
    }

    match state {
        ClientState::Connected(client) => Ok(client),
        ClientState::Disconnected => Err(Error::ConnectionClosed),
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
        crate::transport::write_frame_io(client, frame)
            .await
            .map_err(io_error_to_local)?;
        let response: Vec<u8> = crate::transport::read_frame_io(client)
            .await
            .map_err(io_error_to_local)?;
        Ok::<Vec<u8>, Error>(response)
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
async fn open_named_pipe_client_with_retry(
    path: &str,
) -> Result<tokio::net::windows::named_pipe::NamedPipeClient, Error> {
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
    let connection_id = Uuid::new_v4().to_string();
    loop {
        let frame = match crate::transport::read_frame_io(&mut connection).await {
            Ok(frame) => frame,
            Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => {
                log_connection_closed(&connection_id, platform_name());
                return Ok(());
            }
            Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => {
                log_connection_closed(&connection_id, platform_name());
                return Ok(());
            }
            Err(err) => return Err(io_error_to_local(err)),
        };
        let response = handle_frame_with_handlers(&handlers, &frame, &connection_id).await?;
        crate::transport::write_frame_io(&mut connection, &response)
            .await
            .map_err(io_error_to_local)?;
    }
}

#[cfg(unix)]
async fn handle_unix_connection(
    mut connection: UnixStream,
    handlers: Arc<HashMap<String, RegisteredHandler>>,
) -> Result<(), Error> {
    let connection_id = Uuid::new_v4().to_string();
    loop {
        let frame = match crate::transport::read_frame_io(&mut connection).await {
            Ok(frame) => frame,
            Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => {
                log_connection_closed(&connection_id, platform_name());
                return Ok(());
            }
            Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => {
                log_connection_closed(&connection_id, platform_name());
                return Ok(());
            }
            Err(err) => return Err(io_error_to_local(err)),
        };
        let response = handle_frame_with_handlers(&handlers, &frame, &connection_id).await?;
        crate::transport::write_frame_io(&mut connection, &response)
            .await
            .map_err(io_error_to_local)?;
    }
}

async fn handle_frame_with_handlers(
    handlers: &HashMap<String, RegisteredHandler>,
    frame: &[u8],
    connection_id: &str,
) -> Result<Vec<u8>, Error> {
    let payload = decode_frame(frame).map_err(codec_error_to_protocol)?;
    let request = match parse_request_envelope(payload) {
        Ok(request) => request,
        Err(ParseRequestEnvelopeError::Invalid(request)) => {
            log_invalid_request(&request, connection_id, platform_name());
            return invalid_request_response(&request);
        }
        Err(ParseRequestEnvelopeError::Protocol(err)) => return Err(err),
    };

    log_server_request_start(&request, connection_id, platform_name());

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
            request_id: request.request_id.clone(),
            ok: false,
            payload: None,
            error: Some(ErrorBody {
                code: TIMEOUT.into(),
                message: "request deadline has expired".into(),
                details: None,
            }),
            trace_id: request.trace_id.clone(),
            metadata: None,
        };
        log_server_request_finish(&request, Some(TIMEOUT), connection_id, platform_name());
        return encode_response_frame(&response);
    }

    let response = match handlers.get(&request.method) {
        Some(handler) => match handler.call(ctx, &request.payload).await {
            Ok(payload) => ResponseEnvelope {
                version: PROTOCOL_VERSION,
                message_type: MessageType::Response,
                request_id: request.request_id.clone(),
                ok: true,
                payload: Some(payload),
                error: None,
                trace_id: request.trace_id.clone(),
                metadata: None,
            },
            Err(err) => ResponseEnvelope {
                version: PROTOCOL_VERSION,
                message_type: MessageType::Response,
                request_id: request.request_id.clone(),
                ok: false,
                payload: None,
                error: Some(ErrorBody {
                    code: err.code,
                    message: err.message,
                    details: None,
                }),
                trace_id: request.trace_id.clone(),
                metadata: None,
            },
        },
        None => ResponseEnvelope {
            version: PROTOCOL_VERSION,
            message_type: MessageType::Response,
            request_id: request.request_id.clone(),
            ok: false,
            payload: None,
            error: Some(ErrorBody {
                code: METHOD_NOT_FOUND.into(),
                message: "method not registered".into(),
                details: None,
            }),
            trace_id: request.trace_id.clone(),
            metadata: None,
        },
    };

    let response_error_code = response.error.as_ref().map(|error| error.code.as_str());
    if response_error_code == Some(DECODE_ERROR) {
        log_server_decode_error(&request, connection_id, platform_name());
    }
    log_server_request_finish(
        &request,
        response_error_code,
        connection_id,
        platform_name(),
    );

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

fn parse_request_envelope(payload: &[u8]) -> Result<RequestEnvelope, ParseRequestEnvelopeError> {
    let loose: LooseRequestEnvelope = decode_cbor(payload)
        .map_err(codec_error_to_protocol)
        .map_err(ParseRequestEnvelopeError::Protocol)?;

    let request = RequestEnvelope {
        version: loose.version.unwrap_or(PROTOCOL_VERSION),
        message_type: loose.message_type.unwrap_or(MessageType::Request),
        request_id: loose.request_id.unwrap_or_default(),
        method: loose.method.unwrap_or_default(),
        deadline_ms: loose.deadline_ms,
        trace_id: loose.trace_id,
        payload: loose.payload.unwrap_or_default(),
        metadata: None,
    };

    if request.version != PROTOCOL_VERSION
        || request.message_type != MessageType::Request
        || request.request_id.is_empty()
        || request.method.is_empty()
        || request.payload.is_empty()
    {
        return Err(ParseRequestEnvelopeError::Invalid(request));
    }

    Ok(request)
}

fn decode_request_metadata(frame: &[u8]) -> Result<RequestMeta, Error> {
    let payload = decode_frame(frame).map_err(codec_error_to_protocol)?;
    let request: RequestEnvelope = decode_cbor(payload).map_err(codec_error_to_protocol)?;
    Ok(RequestMeta {
        request_id: request.request_id,
        method: request.method,
        trace_id: request.trace_id,
        deadline_ms: request.deadline_ms,
    })
}

fn error_code_of(error: &Error) -> Option<&str> {
    match error {
        Error::Protocol { code, .. } => Some(code.as_str()),
        Error::Remote { code, .. } => Some(code.as_str()),
        _ => None,
    }
}

fn platform_name() -> &'static str {
    #[cfg(windows)]
    {
        "windows"
    }
    #[cfg(unix)]
    {
        "unix"
    }
    #[cfg(not(any(windows, unix)))]
    {
        "unknown"
    }
}

fn log_client_call_start(meta: &RequestMeta, platform: &str) {
    info!(
        event = observability::CLIENT_CALL_START,
        request_id = %meta.request_id,
        trace_id = ?meta.trace_id,
        method = %meta.method,
        deadline_ms = ?meta.deadline_ms,
        platform = %platform,
    );
}

fn log_client_call_finish(meta: &RequestMeta, error_code: Option<&str>, platform: &str) {
    info!(
        event = observability::CLIENT_CALL_FINISH,
        request_id = %meta.request_id,
        trace_id = ?meta.trace_id,
        method = %meta.method,
        error_code = ?error_code,
        platform = %platform,
    );
}

fn log_client_call_timeout(meta: &RequestMeta, platform: &str) {
    warn!(
        event = observability::CLIENT_CALL_TIMEOUT,
        request_id = %meta.request_id,
        trace_id = ?meta.trace_id,
        method = %meta.method,
        deadline_ms = ?meta.deadline_ms,
        platform = %platform,
    );
}

fn log_server_request_start(request: &RequestEnvelope, connection_id: &str, platform: &str) {
    info!(
        event = observability::SERVER_REQUEST_START,
        request_id = %request.request_id,
        trace_id = ?request.trace_id,
        method = %request.method,
        deadline_ms = ?request.deadline_ms,
        connection_id = %connection_id,
        platform = %platform,
    );
}

fn log_server_request_finish(
    request: &RequestEnvelope,
    error_code: Option<&str>,
    connection_id: &str,
    platform: &str,
) {
    info!(
        event = observability::SERVER_REQUEST_FINISH,
        request_id = %request.request_id,
        trace_id = ?request.trace_id,
        method = %request.method,
        error_code = ?error_code,
        connection_id = %connection_id,
        platform = %platform,
    );
}

fn log_server_decode_error(request: &RequestEnvelope, connection_id: &str, platform: &str) {
    warn!(
        event = observability::SERVER_REQUEST_DECODE_ERROR,
        request_id = %request.request_id,
        trace_id = ?request.trace_id,
        method = %request.method,
        connection_id = %connection_id,
        platform = %platform,
    );
}

fn log_invalid_request(request: &RequestEnvelope, connection_id: &str, platform: &str) {
    warn!(
        event = observability::PROTOCOL_INVALID_REQUEST,
        request_id = %request.request_id,
        trace_id = ?request.trace_id,
        method = %request.method,
        connection_id = %connection_id,
        platform = %platform,
    );
}

fn log_connection_closed(connection_id: &str, platform: &str) {
    info!(
        event = observability::CONNECTION_CLOSED,
        connection_id = %connection_id,
        platform = %platform,
    );
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::{
        ClientRuntime, RegisteredHandler, RequestMeta, ServerRuntime, log_client_call_finish,
    };
    use crate::api::{CallOptions, Error, IpcAddress, ServiceError};
    use crate::codec::{decode_cbor, encode_cbor, encode_frame};
    use crate::observability;
    use crate::protocol::{MessageType, PROTOCOL_VERSION, RequestEnvelope};
    use tracing::subscriber::with_default;
    use tracing_subscriber::fmt;

    #[derive(Clone)]
    struct SharedWriter {
        buffer: Arc<Mutex<Vec<u8>>>,
    }

    impl io::Write for SharedWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.buffer
                .lock()
                .expect("buffer lock should succeed")
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn capture_logs<F>(run: F) -> String
    where
        F: FnOnce(),
    {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let subscriber = fmt::Subscriber::builder()
            .with_ansi(false)
            .without_time()
            .with_writer({
                let buffer = Arc::clone(&buffer);
                move || SharedWriter {
                    buffer: Arc::clone(&buffer),
                }
            })
            .finish();

        with_default(subscriber, run);

        String::from_utf8(buffer.lock().expect("buffer lock should succeed").clone())
            .expect("logs should be utf-8")
    }

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
            .register(
                "ping",
                RegisteredHandler::new(|_ctx, req: PingRequest| async move {
                    Ok(PingResponse { value: req.value })
                }),
            )
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
            .register(
                "ping",
                RegisteredHandler::new(|_ctx, req: PingRequest| async move {
                    Ok(PingResponse { value: req.value })
                }),
            )
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
            .register(
                "ping",
                RegisteredHandler::new(|_ctx, _req: PingRequest| async move {
                    Err::<PingResponse, ServiceError>(ServiceError {
                        code: "internal_error".into(),
                        message: "boom".into(),
                    })
                }),
            )
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
            .register(
                "ping",
                RegisteredHandler::new(|_ctx, req: PingRequest| async move {
                    Ok(PingResponse { value: req.value })
                }),
            )
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

    #[test]
    fn server_observability_logs_request_lifecycle() {
        let logs = capture_logs(|| {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime should build");

            runtime.block_on(async {
                let client = ClientRuntime::new(IpcAddress::NamedPipe("demo".into()));
                let mut server = ServerRuntime::new(IpcAddress::NamedPipe("demo".into()));
                server
                    .register(
                        "ping",
                        RegisteredHandler::new(|_ctx, req: PingRequest| async move {
                            Ok(PingResponse { value: req.value })
                        }),
                    )
                    .expect("register should succeed");

                let request = client
                    .build_request_frame(
                        "ping",
                        &PingRequest {
                            value: "hello".into(),
                        },
                        CallOptions {
                            timeout: Some(Duration::from_secs(1)),
                            trace_id: Some("trace-test".into()),
                        },
                    )
                    .expect("request should encode");

                let _ = server
                    .handle_frame(&request)
                    .await
                    .expect("server should respond");
            });
        });

        assert!(logs.contains(observability::SERVER_REQUEST_START));
        assert!(logs.contains(observability::SERVER_REQUEST_FINISH));
        assert!(logs.contains("request_id="));
        assert!(logs.contains("method=ping"));
        assert!(logs.contains("trace_id=Some(\"trace-test\")"));
    }

    #[test]
    fn client_observability_logs_error_code() {
        let meta = RequestMeta {
            request_id: "req-1".into(),
            method: "ping".into(),
            trace_id: Some("trace-1".into()),
            deadline_ms: Some(42),
        };

        let logs = capture_logs(|| {
            log_client_call_finish(&meta, Some("timeout"), "windows");
        });

        assert!(logs.contains(observability::CLIENT_CALL_FINISH));
        assert!(logs.contains("request_id=req-1"));
        assert!(logs.contains("method=ping"));
        assert!(logs.contains("error_code=Some(\"timeout\")"));
        assert!(logs.contains("platform=windows"));
    }
}
