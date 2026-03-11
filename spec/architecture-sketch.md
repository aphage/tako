# IPC Rust 库架构草图（MVP）

本文档用于把 [plan.md](D:\alice\tako\spec\plan.md)、[protocol-draft.md](D:\alice\tako\spec\protocol-draft.md)、[api-sketch.md](D:\alice\tako\spec\api-sketch.md) 落到可实现的模块边界、关键数据流和状态机设计。目标不是提前设计所有内部细节，而是把阶段 2 会反复摇摆的结构性问题先固定。

## 1. 设计目标

- 保证对外 API、线协议和运行时行为三者一致。
- 让主开发平台最小链路可以尽快打通。
- 把复杂性收敛在 `runtime` 和平台传输适配层，而不是污染 `api`。
- 为异常路径、断连策略和测试夹具预留清晰落点。

## 2. 模块总览

MVP 建议模块如下：

- `api`
- `protocol`
- `codec`
- `transport`
- `runtime`
- `observability`
- `examples`

推荐目录形态：

```text
src/
  api/
  protocol/
  codec/
  transport/
    unix.rs
    windows_named_pipe.rs
  runtime/
  observability/
examples/
tests/
```

## 3. 模块职责

### 3.1 `api`

职责：

- 暴露 `Client`、`Server`、`IpcAddress`、`RequestContext`、`CallOptions`、`Error`。
- 把泛型 handler 和泛型 `call` 请求适配到内部运行时 contract。
- 隐藏传输层与平台差异。

不负责：

- 直接做分帧。
- 直接理解 socket / named pipe 的平台细节。
- 直接维护底层连接状态机细节。

### 3.2 `protocol`

职责：

- 定义请求/响应信封。
- 定义错误体、错误码、版本常量。
- 定义协议字段约束。

不负责：

- 实际的 CBOR 编解码。
- I/O 读写。
- 连接生命周期管理。

### 3.3 `codec`

职责：

- 实现长度前缀分帧。
- 实现信封与字节流之间的编码和解码。
- 执行最大帧大小校验。
- 处理非法长度、超大帧、非法 CBOR、字段缺失等本地协议解析错误。

不负责：

- 打开连接。
- 调度 handler。
- 管理超时与重连。

### 3.4 `transport`

职责：

- 提供统一的 listener / accepted stream / connected stream 抽象。
- 隔离 Unix Domain Socket 与 Windows Named Pipe 差异。
- 承担地址规范化、绑定、连接建立和底层字节流读写。

不负责：

- 解释信封语义。
- 映射业务错误。
- 维护请求级状态。

### 3.5 `runtime`

职责：

- 管理客户端和服务端的请求生命周期。
- 管理客户端串行请求排队。
- 管理本地超时、连接失效和惰性重连。
- 管理服务端请求派发、handler 执行和响应回写。
- 将内部错误映射到对外 `Error` 或远端 `ErrorBody`。

不负责：

- 对外暴露平台细节。
- 定义协议字段本身。

### 3.6 `observability`

职责：

- 定义日志字段名和关键事件。
- 提供请求开始、结束、失败、协议错误、连接关闭的统一打点入口。

不负责：

- 业务层日志聚合。
- 指标系统完整接入。

### 3.7 `examples`

职责：

- 提供最小成功链路示例。
- 提供失败调用与错误处理示例。
- 作为 API 易用性回归样例。

## 4. 核心内部 contract

为降低 runtime 泛型复杂度，MVP 固定以下内部 contract：

- 客户端发包边界：`method + metadata + encoded payload bytes`
- 服务端 handler 边界：`RequestContext + raw payload bytes -> encoded response bytes | ServiceError`
- runtime 与 codec 边界：结构化信封 <-> 字节帧

这意味着：

- `api::Client::call<Req, Resp>` 负责把 `Req` 编码为 payload bytes，并在收到响应后把 payload bytes 解码为 `Resp`。
- `api::Server::register<Req, Resp>` 负责把用户提供的 typed handler 包装成内部 raw-bytes handler。
- `runtime` 不需要理解具体业务类型，只处理 payload bytes 和错误映射。

## 5. 数据流

### 5.1 客户端成功调用

```text
Client::call
  -> api 将 Req 编码为 payload bytes
  -> runtime 获取或建立连接
  -> runtime 生成 request_id / trace_id / deadline_ms
  -> protocol 构造 RequestEnvelope
  -> codec 编码为 frame bytes
  -> transport 写入字节流
  -> transport 读取响应字节流
  -> codec 解码 ResponseEnvelope
  -> runtime 映射错误或返回 payload bytes
  -> api 将 payload bytes 解码为 Resp
```

### 5.2 服务端成功调用

```text
transport accept connection
  -> runtime 读取 frame
  -> codec 解码 RequestEnvelope
  -> runtime 构造 RequestContext
  -> runtime 查找 method 对应 handler
  -> api 包装层将 payload bytes 解码为 Req
  -> 用户 handler 执行
  -> api 包装层将 Resp 编码为 payload bytes
  -> runtime 构造 ResponseEnvelope
  -> codec 编码 frame
  -> transport 回写
```

### 5.3 服务端协议错误

```text
transport read frame
  -> codec 解析失败
  -> runtime 判断是否可安全响应
    -> 可响应：返回 invalid_request，发送完成后关闭连接
    -> 不可响应：直接关闭连接
```

### 5.4 客户端本地超时

```text
Client::call
  -> runtime 等待响应
  -> 本地超时触发
  -> 当前调用返回 Error::Timeout
  -> 当前连接标记失效并关闭
  -> 下一次调用前按需惰性建立新连接
```

## 6. 客户端架构

### 6.1 组成

客户端内部至少需要以下组件：

- 地址配置
- 默认 `CallOptions`
- 当前连接句柄
- 串行调用队列或互斥入口
- 连接状态

### 6.2 连接状态

建议最小状态机：

```text
Disconnected
Connecting
Ready
Busy
DrainingAfterTimeout
Closed
```

语义说明：

- `Disconnected`：当前没有可复用连接。
- `Connecting`：正在建立新连接。
- `Ready`：已有可发送请求的连接。
- `Busy`：当前连接上有一个 in-flight 请求。
- `DrainingAfterTimeout`：本地超时后正在丢弃旧连接，禁止复用。
- `Closed`：客户端被显式关闭或内部不可恢复。

MVP 简化规则：

- `Ready -> Busy -> Ready` 是正常成功路径。
- `Busy -> DrainingAfterTimeout -> Disconnected` 是本地超时路径。
- `Disconnected -> Connecting -> Ready` 是惰性重连路径。
- 不支持一个连接上多个并发 in-flight 请求。

### 6.3 串行调用模型

建议做法：

- 对外保留 `&self`。
- 内部用互斥或单线程任务串行化同一个 `Client` 的请求。
- 所有排队行为都在客户端本地完成，不通过协议表达。

这样做的收益：

- 避免多路复用。
- 避免 request correlation 状态机。
- 降低晚到响应污染后续请求的风险。

## 7. 服务端架构

### 7.1 组成

服务端内部至少需要以下组件：

- 地址配置
- handler 注册表
- listener
- 每连接处理循环
- shutdown 信号入口
- 默认安全策略与资源清理逻辑

### 7.2 handler 注册表

建议形态：

- `HashMap<String, Box<dyn Handler>>`

其中 `Handler` 内部 contract 固定为：

```rust
async fn handle(
    &self,
    ctx: RequestContext,
    payload: &[u8],
) -> Result<Vec<u8>, ServiceError>;
```

说明：

- `api::register` 负责把 typed handler 包装成这个 trait object。
- `runtime` 只依赖这个 trait，不关心 `Req` / `Resp` 泛型。

### 7.3 服务端连接处理模型

MVP 建议：

- 服务端可并发处理多个连接。
- 同一连接上按串行请求处理。
- 每个连接由独立任务负责读帧、派发、写回和清理。

理由：

- 和客户端串行单请求模型一致。
- 实现简单，便于验证断连、超时和协议错误路径。

## 8. 传输抽象

### 8.1 目标

上层只依赖“可靠字节流 + listener + 地址规范化”，不依赖平台细节。

### 8.2 建议抽象

至少需要以下概念：

- `BindableAddress`
- `Listener`
- `Connection`

推荐能力：

- `bind`
- `accept`
- `connect`
- `read_exact`
- `write_all`
- `shutdown`

### 8.3 地址规范化

规则固定如下：

- Unix：使用调用方显式提供或库默认受控目录下的路径。
- Windows：`IpcAddress::NamedPipe(String)` 规范化为 `\\.\pipe\<name>`。

## 9. 安全策略落点

### 9.1 Unix Domain Socket

实现要求：

- 若目录不存在，由库创建受控目录。
- 目录和 socket 文件使用最小权限。
- 绑定前检查旧路径清理策略，避免脏文件影响启动。

### 9.2 Windows Named Pipe

实现要求：

- 默认安全描述符限制为当前用户可访问。
- 若后续支持显式放宽权限，应作为额外配置能力，不进入 MVP 默认路径。

## 10. 错误映射落点

客户端侧：

- 本地建连、I/O、超时、连接关闭：映射为本地 `Error`
- 本地协议解析失败：映射为 `Error::Protocol`
- 远端显式 `invalid_request`：映射为 `Error::Protocol`
- 远端显式 `method_not_found` / `decode_error` / `timeout` / `internal_error`：映射为 `Error::Remote`

服务端侧：

- 业务参数解码失败：返回 `decode_error`
- 方法未注册：返回 `method_not_found`
- 可安全响应的请求结构错误：返回 `invalid_request` 并关闭连接
- 不可安全响应的帧级错误：直接关闭连接
- handler 内部未分类失败：返回 `internal_error`

## 11. 观测性落点

建议统一日志字段：

- `request_id`
- `trace_id`
- `method`
- `deadline_ms`
- `error_code`
- `connection_id`
- `platform`

建议关键事件：

- `client.call.start`
- `client.call.finish`
- `client.call.timeout`
- `server.request.start`
- `server.request.finish`
- `server.request.decode_error`
- `protocol.invalid_request`
- `connection.closed`

## 12. 测试映射

### 12.1 单元测试

适合覆盖：

- 信封字段约束
- 错误码映射
- 分帧边界
- 地址规范化

### 12.2 集成测试

适合覆盖：

- 成功调用
- 方法不存在
- 非法长度
- 超大帧
- 非法 CBOR
- 缺少必填字段
- 服务端业务解码失败
- 本地超时与惰性重连
- `deadline_ms` 过期拒绝
- `version != 1`

### 12.3 跨平台验证

适合覆盖：

- 主开发平台完整链路
- 至少一个非主平台真实链路
- 默认权限 / 安全策略在目标平台上的实际行为

## 13. 开发顺序建议

建议严格按以下顺序推进：

1. `protocol`
2. `codec`
3. `transport` 主开发平台实现
4. `runtime`
5. `api`
6. `examples`
7. 自动化测试补齐
8. 非主平台验证

原因：

- 先锁结构化边界，再接 I/O，最后暴露用户接口，返工最少。

## 14. 明确不做

MVP 架构层面明确不做以下内容：

- 单连接多路复用状态机
- 流式请求/响应通道
- 通用 middleware 框架
- 自动重试器
- 跨进程取消协议
- 动态服务发现
- 抽象到任意 async runtime 的通用适配层

## 15. 架构评审出口

满足以下条件后，可认为架构草图足以支撑阶段 2 开发：

- 模块边界没有未决冲突。
- 客户端与服务端的关键状态机已经写清。
- 错误映射和协议错误处理路径已经落到具体模块。
- 传输抽象足以覆盖 Unix Domain Socket 和 Windows Named Pipe。
- 安全策略、测试映射和开发顺序都已明确。
