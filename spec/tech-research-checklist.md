# IPC Rust 库技术调研清单

本文档用于支撑 [plan.md](D:\alice\tako\spec\plan.md) 阶段 0 的技术选型工作。目标不是罗列所有可能方案，而是把 MVP 真正会影响实现、排期和风险的依赖与技术决策收敛成可评审清单。

## 1. 调研目标

- 选定 MVP 所需的最小依赖集合。
- 排除会显著放大复杂度但不能直接提升 MVP 交付价值的方案。
- 给阶段 1 和阶段 2 提供足够稳定的实现前提。

## 2. 调研范围

本轮调研只覆盖以下主题：

1. 异步运行时
2. 本地 IPC 传输实现
3. CBOR 编解码
4. 错误处理
5. 日志与 tracing
6. 测试与跨平台验证
7. UUID 与时间处理

以下内容不纳入本轮调研：

- IDL / 代码生成
- 流式 RPC
- 多路复用
- 认证鉴权框架
- 自动重试框架
- 跨机器网络传输

## 3. 选型原则

- 优先选稳定、主流、维护活跃的 Rust 生态方案。
- 优先选能直接服务 MVP 的依赖，不为未来扩展提前引入重框架。
- 优先选跨平台能力明确的方案。
- 若某能力标准库已足够，则不额外引入第三方依赖。
- 同一层职责尽量只保留一个主方案，避免“双轨并存”。

## 4. 候选主题与评审项

每个主题统一从以下维度评估：

- 是否满足 MVP 需求
- 跨平台能力是否清晰
- API 是否简洁
- 社区成熟度与维护风险
- 与计划中的协议 / API / 测试口径是否兼容
- 是否会引入额外状态机复杂度

## 5. 候选库清单

### 5.1 异步运行时

候选：

- `tokio`
- `async-std`
- `smol`

建议结论：

- 首选 `tokio`

理由：

- Windows Named Pipe、Unix Domain Socket、超时控制、任务模型和测试工具链都更容易围绕 `tokio` 收敛。
- 文档、示例和社区资料充足，更适合作为 MVP 基线。
- 若后续需要扩展 observability、graceful shutdown 或更复杂运行时行为，`tokio` 生态衔接更平滑。

结论状态：

- `ADOPT`

### 5.2 Unix Domain Socket

候选：

- `tokio::net::UnixListener` / `tokio::net::UnixStream`
- 自行封装标准库阻塞 I/O

建议结论：

- 首选 `tokio::net`

理由：

- 与运行时保持一致，避免在 MVP 引入阻塞线程桥接和双模型调度。
- 足以满足当前长度前缀分帧和串行请求模型。

结论状态：

- `ADOPT`

### 5.3 Windows Named Pipe

候选：

- `tokio::net::windows::named_pipe`
- 第三方 Windows pipe crate
- 自行调用 Win32 API

建议结论：

- 优先评估并采用 `tokio::net::windows::named_pipe`

理由：

- 运行时模型统一。
- 能减少额外封装层与维护面。
- 若能力不足，再单独评估补充 crate 或少量 Win32 封装，而不是一开始就自建一套。

结论状态：

- `ADOPT_IF_SUFFICIENT`

### 5.4 CBOR 编解码

候选：

- `ciborium`
- `serde_cbor`

建议结论：

- 优先 `ciborium`

理由：

- 更适合作为 `serde` 生态下的通用 CBOR 方案。
- 能满足 MVP 对结构化消息、错误体和 payload 编解码的需求。

保留关注点：

- 是否方便处理“顶层信封 + 原始 payload bytes 边界”的实现方式。

结论状态：

- `ADOPT`

### 5.5 错误处理

候选：

- 手写错误枚举 + `thiserror`
- `anyhow`
- `eyre`

建议结论：

- 对外错误类型使用手写枚举，内部配合 `thiserror`

理由：

- 对外 API 需要稳定、结构化的错误分类，不适合直接暴露 `anyhow`/`eyre`。
- `thiserror` 适合内部错误分层与映射。

结论状态：

- `ADOPT`

### 5.6 日志与 tracing

候选：

- `tracing`
- 标准日志接口 + `log`

建议结论：

- 首选 `tracing`

理由：

- 更适合请求生命周期、结构化字段和异步上下文。
- 与计划中的 `request_id`、`trace_id`、`method`、`error.code` 字段天然契合。

结论状态：

- `ADOPT`

### 5.7 UUID

候选：

- `uuid`
- 自定义随机字符串方案

建议结论：

- 首选 `uuid`

理由：

- `trace_id` 已冻结为 UUID v4 字符串生成策略，直接使用成熟库即可。

结论状态：

- `ADOPT`

### 5.8 时间处理

候选：

- 标准库 `std::time`
- `time`
- `chrono`

建议结论：

- 优先标准库 `std::time`

理由：

- MVP 只需要本地超时换算和 `deadline_ms` 的 Unix epoch 毫秒表示，不需要额外日历能力。

结论状态：

- `ADOPT`

### 5.9 序列化边界测试

候选：

- 标准 `#[test]`
- `tokio::test`
- 属性测试框架

建议结论：

- 基础单元测试使用 `#[test]`
- 异步与集成测试使用 `tokio::test`
- 属性测试暂不纳入 MVP 必需项

理由：

- 当前测试矩阵主要是协议分支和状态机路径，不需要一开始引入更重的测试框架。

结论状态：

- `ADOPT`

## 6. 依赖建议清单

MVP 建议最小依赖集合如下：

- `tokio`
- `serde`
- `ciborium`
- `thiserror`
- `tracing`
- `uuid`

可选依赖：

- `tracing-subscriber`

暂不建议默认引入：

- `anyhow`
- `eyre`
- `chrono`
- 代码生成相关依赖
- 多路复用 / 流式框架

## 7. 待验证问题

以下问题需要在真正拍板前做一次最小验证：

1. `tokio::net::windows::named_pipe` 是否足以承载当前所需的 listener / connect / read / write / shutdown 模型。
2. `ciborium` 在“信封结构 + 原始 payload bytes 边界”下的实现是否顺手。
3. Unix Domain Socket 默认权限控制是否需要额外平台分支处理。
4. Windows Named Pipe 默认安全策略是否需要少量平台专有封装。

## 8. 最小验证任务

建议在阶段 0 末尾前完成以下验证：

1. 用选定运行时与传输 API 建立最小双端 echo 原型。
2. 用候选 CBOR 库完成一次请求/响应信封编码和解码。
3. 验证 `trace_id` UUID v4 生成与 `deadline_ms` 写入逻辑。
4. 验证主开发平台的本地链路可在测试中稳定启动和停止。
5. 明确额外验证平台是本地机、CI runner 还是人工验证路径。

## 9. 调研结论模板

每个主题最终应按以下格式补齐：

- 主题：
- 候选：
- 结论：
- 原因：
- 风险：
- 后续动作：
- 负责人：
- 日期：

## 10. 阶段 0 评审出口

满足以下条件后，可认为阶段 0 调研完成：

- MVP 最小依赖集合已确定。
- 所有关键依赖都已有采用 / 不采用结论。
- 待验证问题已收敛到可执行的小实验，而不是开放式讨论。
- 平台验证资源和验证路径已明确。
- 没有会直接阻断阶段 1 / 2 的重大未决项。
