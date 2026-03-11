# IPC Rust 库 API 草图（MVP）

## 1. 文档目标

本文档定义 IPC Rust 库在 MVP 阶段的对外 API 方向、使用方式和边界约束。目标不是一次性定稿所有 Rust trait 细节，而是先冻结用户可见的接入模型。

本文档与 [plan.md](D:\alice\tako\spec\plan.md) 和 [protocol-draft.md](D:\alice\tako\spec\protocol-draft.md) 保持一致。

## 2. 设计目标

- 让 Rust 用户在 10 分钟内完成 Hello World。
- 默认 API 少而直接，不把平台差异暴露给上层。
- 先满足单请求单响应，再考虑 typed stub 与中间件。
- 错误分类清晰，便于调用方做超时、重试和排障。

## 3. 用户视角核心对象

MVP 核心对象建议如下：

- `IpcAddress`：统一地址抽象。
- `Server`：服务端监听与路由注册入口。
- `Client`：客户端连接与调用入口。
- `RequestContext`：服务端处理请求时的上下文。
- `CallOptions`：单次调用的超时等参数。
- `Error`：统一错误类型。

## 4. 地址抽象草图

```rust
pub enum IpcAddress {
    UnixSocket(std::path::PathBuf),
    NamedPipe(String),
}
```

约束：

- `api` 层直接接收 `IpcAddress`，不暴露底层传输实现类型。
- `IpcAddress::NamedPipe(String)` 固定承载逻辑名称而不是完整系统路径；运行时统一规范化为 `\\.\pipe\<name>`。
- Unix Domain Socket 路径必须位于调用方显式指定或库默认创建的受控目录下；MVP 默认采用最小权限创建目录和 socket 文件。

## 5. Server API 草图

```rust
pub struct Server { /* omitted */ }

pub struct ServiceError {
    pub code: String,
    pub message: String,
}

impl Server {
    pub async fn bind(addr: IpcAddress) -> Result<Self, Error>;

    pub fn register<F, Fut, Req, Resp>(&mut self, method: &str, handler: F) -> Result<&mut Self, Error>
    where
        F: Fn(RequestContext, Req) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Resp, ServiceError>> + Send + 'static,
        Req: serde::de::DeserializeOwned + Send + 'static,
        Resp: serde::Serialize + Send + 'static;

    pub async fn serve(self) -> Result<(), Error>;

    pub async fn serve_until<S>(self, shutdown: S) -> Result<(), Error>
    where
        S: Future<Output = ()> + Send;
}
```

设计说明：

- `bind` 与 `serve` 分离，便于测试和生命周期控制。
- `serve_until` 作为 MVP 最小停服入口，确保集成测试和嵌入式场景不依赖 task abort 才能回收服务端。
- `register` 采用按方法名注册处理器的模型，符合计划书中的最小入口。
- MVP 不要求用户先定义 service trait。
- `register` 最终固定为 `&mut self -> &mut Self`，避免在初始化阶段引入不必要的 move 语义与二义性。
- handler 只返回服务端业务/领域错误，不直接暴露客户端建连、I/O、协议解析等本地错误类型；这些错误由 runtime 负责产生并映射。
- 对外保留泛型 `register`，内部类型擦除 contract 固定为“原始 CBOR payload bytes -> 编码后响应 bytes / ServiceError”；泛型解码与编码包装发生在 `register` 适配层，而不是 runtime 核心路径。

## 6. Client API 草图

```rust
pub struct Client { /* omitted */ }

impl Client {
    pub async fn connect(addr: IpcAddress) -> Result<Self, Error>;

    pub async fn call<Req, Resp>(
        &self,
        method: &str,
        request: Req,
    ) -> Result<Resp, Error>
    where
        Req: serde::Serialize,
        Resp: serde::de::DeserializeOwned;

    pub async fn call_with<Req, Resp>(
        &self,
        method: &str,
        request: Req,
        options: CallOptions,
    ) -> Result<Resp, Error>
    where
        Req: serde::Serialize,
        Resp: serde::de::DeserializeOwned;
}
```

设计说明：

- `call` 提供最简默认路径。
- `call_with` 提供超时、trace_id 等扩展入口。
- MVP 默认单连接串行调用，因此 `&self` 的并发语义固定为“可并发发起、库内排队串行执行”。
- 一旦单次调用在客户端本地超时，当前连接必须视为不可复用；后续新调用开始前，实现可惰性重建连接，但不得隐式重试已经超时的那次调用。
- MVP 不提供自动重试，也不做后台保活式自动重连；允许仅为后续新调用执行惰性重连。
- `trace_id` 若未显式提供，由客户端在发包前自动生成 UUID v4 字符串。

## 7. RequestContext 草图

```rust
pub struct RequestContext {
    pub request_id: String,
    pub method: String,
    pub trace_id: Option<String>,
    pub deadline_ms: Option<u64>,
}
```

建议能力：

- 读取请求元信息。
- 为日志打字段。
- 后续可扩展 peer 信息，但 MVP 不强制暴露。

## 8. CallOptions 草图

```rust
#[derive(Default)]
pub struct CallOptions {
    pub timeout: Option<std::time::Duration>,
    pub trace_id: Option<String>,
}
```

约束：

- `timeout` 只影响客户端本地等待上限。
- `trace_id` 为空时由库自动生成 UUID v4 字符串。
- 运行时在发包前根据 `timeout` 计算 `deadline_ms`，并以 Unix epoch 毫秒写入请求信封。
- MVP 不在 `CallOptions` 中放入重试配置，避免误导用户认为库会自动重试。

## 9. Error 草图

```rust
pub enum Error {
    ConnectFailed { message: String },
    PermissionDenied { message: String },
    Io { message: String },
    Timeout,
    ConnectionClosed,
    Protocol { code: String, message: String },
    Remote { code: String, message: String },
    Decode { message: String },
    Encode { message: String },
}
```

设计说明：

- `Protocol` 表示线协议或消息结构错误。
- `Remote` 表示服务端业务失败或服务端显式返回的错误码。
- `Timeout` 默认表示客户端本地等待超时。
- `ConnectionClosed` 表示连接在超时清理、对端关闭或本地状态失效后不可继续使用。
- 实现时可以用更细粒度内部错误，再映射到对外 `Error`。
- 服务端 handler 不直接返回该 `Error`；handler 产出的 `ServiceError` 由 runtime 映射成响应信封中的 `error` 字段。
- 服务端返回的 `invalid_request` 默认映射为 `Error::Protocol`；服务端返回的 `method_not_found`、`decode_error`、`timeout`、`internal_error` 默认映射为 `Error::Remote`。
- 本地检测到的非法帧、非法 CBOR、字段缺失、版本不兼容映射为 `Error::Protocol`。
- 客户端本地超时后若内部立即断开连接，本次调用仍返回 `Error::Timeout`；随后发起的新调用可触发惰性重连，若重连失败或连接仍不可用，可返回 `Error::ConnectFailed` 或 `Error::ConnectionClosed`。

## 10. Hello World 伪代码

### 10.1 服务端

```rust
#[derive(serde::Deserialize)]
struct PingRequest {
    value: String,
}

#[derive(serde::Serialize)]
struct PingResponse {
    value: String,
}

let addr = IpcAddress::UnixSocket("/tmp/demo.sock".into());

let mut server = Server::bind(addr).await?;
server.register("ping", |_ctx, req: PingRequest| async move {
    Ok(PingResponse { value: req.value })
})?;

server.serve().await?;
```

### 10.2 客户端

```rust
#[derive(serde::Serialize)]
struct PingRequest {
    value: String,
}

#[derive(serde::Deserialize)]
struct PingResponse {
    value: String,
}

let addr = IpcAddress::UnixSocket("/tmp/demo.sock".into());
let client = Client::connect(addr).await?;

let resp: PingResponse = client
    .call("ping", PingRequest { value: "hello".into() })
    .await?;
```

## 11. 失败调用示例

```rust
let result: Result<PingResponse, Error> = client
    .call_with(
        "missing_method",
        PingRequest { value: "hello".into() },
        CallOptions {
            timeout: Some(std::time::Duration::from_secs(1)),
            trace_id: Some("trace-123".into()),
        },
    )
    .await;

match result {
    Err(Error::Remote { code, .. }) if code == "method_not_found" => {
        // expected
    }
    other => {
        // handle unexpected branch
    }
}
```

## 12. MVP 明确不做

- 不暴露 typed generated client。
- 不暴露 middleware/interceptor API。
- 不支持流式 request/response。
- 不支持单连接多路复用。
- 不支持自动重试。
- 不支持跨进程取消。

## 13. 已冻结决议

- `trace_id` 缺省时自动生成 UUID v4 字符串。
- `IpcAddress::NamedPipe(String)` 承载逻辑名称，运行时统一规范化为 `\\.\pipe\<name>`。
- `Server::register` 固定为 `&mut self -> &mut Self`。
- handler 类型擦除边界固定为“原始 CBOR payload bytes -> 编码后响应 bytes / ServiceError”。
- `Client` 本地超时后连接失效，后续新调用允许惰性重连，但不得隐式重试前一次请求。
- 服务端最小停服入口固定为 `serve_until(shutdown)`；`serve()` 仅作为永久运行包装。
- 默认安全策略固定为：Unix Domain Socket 使用受控目录与最小权限；Windows Named Pipe 默认限制为当前用户可访问。

## 14. 最小测试草图

- `ping` 成功调用。
- 方法不存在返回 `method_not_found`。
- 超大帧触发断连，不返回错误响应。
- 非法 CBOR 触发断连，不返回错误响应。
- 缺少必填字段返回 `invalid_request`，并在响应后关闭连接。
- 客户端超时返回 `Error::Timeout`。
- 服务端根据 `deadline_ms` 拒绝过期请求并返回 `timeout`。
- 客户端本地超时后，同一个 `Client` 的下一次调用在新连接上继续，且不会隐式重试前一次请求。
- 服务端返回非法 payload 时客户端得到解码错误。
- 非法帧触发协议错误并关闭连接。
- `version != 1` 返回 `invalid_request` 并关闭连接。
- 主开发平台完成真实端到端调用，且至少一个额外已具备验证环境的平台完成真实链路验证；其余平台仅在发布说明中标注为兼容性目标或未验证。
