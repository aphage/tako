# IPC Rust 库协议草案（MVP）

## 1. 文档目标

本文档定义 IPC Rust 库 MVP 阶段的线协议约束，用于指导 `protocol`、`codec`、`transport`、`runtime` 模块实现，并作为跨平台一致性的基线。

本文档只覆盖 MVP 必需能力，不覆盖流式传输、多路复用、认证鉴权、自动重试、跨进程取消等非目标能力。

## 2. 设计原则

- 保持请求/响应模型显式化，不做隐式状态同步。
- 协议语义独立于具体传输层，不依赖 Unix Domain Socket 或 Named Pipe 的天然消息边界。
- 优先保证可调试、可验证、可扩展，再考虑高级吞吐优化。
- 保留版本字段与扩展空间，但不在 MVP 中提前引入复杂协商机制。

## 3. 术语

- 帧：传输层上一次完整读写的协议单元。
- 信封：CBOR 编码后的顶层消息对象。
- 请求：由客户端发往服务端的调用消息。
- 响应：由服务端返回给客户端的结果消息。
- 本地错误：未跨线传输、直接在本地 API 暴露的错误。
- 线协议错误：通过响应信封显式返回给调用方的错误。

## 4. 分帧格式

MVP 所有消息均采用统一帧格式：

- `u32_be length`
- `payload[length]`

说明：

- `length` 为 4 字节无符号大端整数，表示后续 payload 的字节数。
- `payload` 必须是一个完整的 CBOR 文档。
- 一个帧只承载一个请求或一个响应。
- 传输层只负责读写字节流，不负责解释 payload 含义。

### 4.1 大小限制

- MVP 默认最大帧大小：`4 MiB`。
- `length == 0` 视为非法请求。
- `length > 4 MiB` 视为协议错误，连接可直接关闭。

### 4.2 非法帧处理

出现以下情况时，接收方应按协议错误处理：

- 帧长度字段非法。
- 帧长度超出上限。
- payload 不是合法 CBOR。
- 顶层对象缺失必填字段。
- `message_type` 取值不合法。

若错误发生在服务端且仍可安全构造响应，则返回错误响应；若无法确定消息边界或连接状态不可信，可直接关闭连接。

MVP 将协议错误处理进一步固定为：

- 可返回错误响应：CBOR 合法、消息边界可信，但顶层字段缺失、字段类型不符、`version` 不兼容、`message_type` 非法。
- 必须直接关闭连接：长度为 0、长度超上限、帧读取不完整、payload 非法 CBOR、任何导致后续帧边界无法信任的错误。

为避免实现分叉，MVP 进一步固定以下行为：

- 对“可返回错误响应”的情形，接收方必须返回 `invalid_request`，并在响应发送完成后关闭该连接。
- 对“必须直接关闭连接”的情形，接收方不得尝试补发错误响应。

## 5. 版本策略

- MVP 固定协议版本为 `1`。
- 每个请求与响应信封都必须携带 `version` 字段。
- 若接收方发现 `version != 1`，且当前帧仍可安全解析，则必须返回 `invalid_request`，并在响应后关闭连接。
- MVP 不定义版本协商握手。

## 6. 顶层消息模型

MVP 采用显式消息类型字段区分请求与响应。

### 6.1 请求信封

请求信封包含以下字段：

- `version: u16`
- `message_type: "request"`
- `request_id: string`
- `method: string`
- `deadline_ms: u64 | null`
- `trace_id: string | null`
- `payload: any`
- `metadata: map<string, value> | null`

字段约束：

- `request_id` 必须在单个连接内唯一。
- `method` 使用稳定字符串命名，例如 `ping`、`system.get_status`。
- `deadline_ms` 固定为 Unix epoch 毫秒时间戳。
- `payload` 为业务请求体，类型固定为单个 CBOR 值，推荐使用结构化对象或数组，而不是嵌套另一层自定义字节封包。
- `metadata` 仅用于扩展字段，不承载 MVP 必需语义。

### 6.2 响应信封

响应信封包含以下字段：

- `version: u16`
- `message_type: "response"`
- `request_id: string`
- `ok: bool`
- `payload: any | null`
- `error: ErrorBody | null`
- `trace_id: string | null`
- `metadata: map<string, value> | null`

字段约束：

- `request_id` 必须回显对应请求的 `request_id`。
- `ok = true` 时，`error` 必须为 `null`。
- `ok = false` 时，`error` 必须存在。
- `payload` 与 `error` 不允许同时表达成功结果；成功响应时 `payload` 为单个 CBOR 值，失败响应时 `payload` 必须为 `null`。

## 7. 错误模型

MVP 将错误分为两层。

### 7.1 本地错误

本地错误不通过协议返回，直接由客户端或服务端 API 暴露：

- 建连失败
- 地址不可达
- 权限不足
- 本地 I/O 读写失败
- 本地等待超时
- 连接已关闭

### 7.2 线协议错误

线协议错误通过响应信封中的 `error` 返回。

`ErrorBody` 最小结构如下：

- `code: string`
- `message: string`
- `details: any | null`

MVP 固定最小错误码集合：

- `method_not_found`
- `invalid_request`
- `decode_error`
- `timeout`
- `internal_error`

建议含义如下：

- `method_not_found`：服务端未注册该方法。
- `invalid_request`：缺少必填字段、字段类型不符、版本不兼容、消息结构非法。
- `decode_error`：业务 payload 解码失败。
- `timeout`：服务端在执行前或执行过程中根据本地期限策略拒绝该请求；MVP 不承诺强制终止已开始的业务逻辑。
- `internal_error`：服务端处理器异常、未分类内部故障、资源耗尽。

客户端错误映射在 MVP 中固定如下：

- 本地检测到的非法帧、非法 CBOR、字段缺失、版本不兼容，映射为本地 `Protocol` 错误。
- 服务端显式返回 `invalid_request`，也映射为客户端 `Protocol` 错误。
- 服务端显式返回 `method_not_found`、`decode_error`、`timeout`、`internal_error`，映射为客户端 `Remote` 错误。
- 建连失败、读写失败、本地等待超时、连接关闭，继续作为本地错误暴露，不通过协议回传。

## 8. 超时与取消语义

- 客户端必须支持调用级超时。
- 客户端超时默认只表示“本地停止等待”，不保证服务端停止处理。
- 请求信封中的 `deadline_ms` 可供服务端在执行前做过期判断。
- 若服务端收到已过期请求，可直接返回 `timeout`。
- 若服务端实现了本地处理期限，也只要求其停止等待并返回 `timeout`；MVP 不要求对正在运行的 handler 提供强制抢占或跨任务中止能力。
- MVP 不定义 cancel 帧。
- future drop 不构成跨进程取消信号。
- 客户端若仅配置相对超时，运行时需在发包前换算出绝对 `deadline_ms` 写入协议。
- 客户端一旦发生本地等待超时，必须关闭当前连接或将其标记为不可复用；MVP 不要求客户端在超时后继续消费晚到响应。
- 后续新调用若继续使用同一个 `Client` 抽象，可在发起前惰性建立新连接；该过程不得重试已经超时的请求。

## 9. 连接与并发语义

- 一个 `Client` 默认维护一个长连接。
- MVP 单连接仅支持串行请求，不支持多个未完成请求同时在途。
- 若客户端 API 允许并发调用，同一 `Client` 上的调用必须在实现内排队串行执行。
- 服务端可同时处理多个客户端连接。
- 单连接串行模型下，响应顺序必须与请求顺序一致。
- 若某请求已在客户端本地超时，则该连接后续不再承载新请求；新的调用必须在新连接上开始。

该约束意味着 MVP 不需要额外定义乱序响应和多路复用相关状态机。

## 10. 地址与平台抽象

协议层不关心具体地址格式，但 API 层需支持统一地址抽象：

- Unix 类系统：socket 文件路径。
- Windows：named pipe 路径。

MVP 要求：

- 地址格式由 API 层统一封装。
- `protocol` 文档不直接引入平台特有字段。
- 权限控制、路径命名、安全描述符作为实现约束，在运行时文档中补充。

## 11. 可观测性字段

MVP 至少保留以下字段用于日志与排障：

- `request_id`
- `trace_id`
- `method`
- `deadline_ms`
- `error.code`

建议日志事件：

- 请求接收
- 请求完成
- 请求失败
- 协议错误
- 连接关闭

## 12. 推荐测试矩阵

协议草案对应的最小测试覆盖应包括：

- 合法请求/合法响应
- 方法不存在
- 非法长度
- 超大帧
- 非法 CBOR
- 缺少必填字段
- 服务端业务解码失败
- 客户端本地超时
- 服务端根据 `deadline_ms` 拒绝过期请求
- 本地超时后旧连接被废弃，后续调用只能在新连接上继续
- 本地超时后同一个 `Client` 的下一次调用会在新连接上开始，且不会隐式重试上一次请求
- `version != 1` 时返回 `invalid_request` 并关闭连接

## 13. 后续扩展点

以下能力保留为未来版本扩展，不进入 MVP：

- 单连接多路复用
- 流式请求/响应
- 认证鉴权
- 取消帧
- 压缩
- 协议握手与能力协商
