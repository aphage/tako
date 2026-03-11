# IPC Rust 库执行清单

本文档将 [plan.md](D:\alice\tako\spec\plan.md)、[protocol-draft.md](D:\alice\tako\spec\protocol-draft.md) 与 [api-sketch.md](D:\alice\tako\spec\api-sketch.md) 转换为可执行的实施与验收清单。默认用途是项目推进、周会同步、阶段评审和发布前自检。

## 1. 使用规则

- 每个条目必须有明确负责人。
- 每个条目完成后必须留下可复查证据，例如文档链接、PR、测试记录、验证日志或截图。
- 阶段出口条件未全部勾选前，不进入下一阶段。
- 若某条因环境缺失无法完成，必须记录阻塞原因和替代口径，不能默认为“后补”。

## 2. 状态定义

- `TODO`：尚未开始。
- `DOING`：已开始但未完成。
- `DONE`：已完成且有证据。
- `BLOCKED`：因依赖、环境或外部决策阻塞。
- `N/A`：经评审确认当前阶段不适用。

## 3. 阶段 0 清单

### 3.1 技术选型

- [x] 状态：`DONE` 选定异步运行时。
  证据：[spec/tech-research-checklist.md](D:\alice\tako\spec\tech-research-checklist.md)，[Cargo.toml](D:\alice\tako\Cargo.toml)
- [x] 状态：`DONE` 选定 CBOR 编解码库。
  证据：[spec/tech-research-checklist.md](D:\alice\tako\spec\tech-research-checklist.md)，[Cargo.toml](D:\alice\tako\Cargo.toml)
- [x] 状态：`DONE` 选定错误处理方案。
  证据：[spec/tech-research-checklist.md](D:\alice\tako\spec\tech-research-checklist.md)，[Cargo.toml](D:\alice\tako\Cargo.toml)
- [x] 状态：`DONE` 选定日志 / tracing 方案。
  证据：[spec/tech-research-checklist.md](D:\alice\tako\spec\tech-research-checklist.md)，[Cargo.toml](D:\alice\tako\Cargo.toml)
- [x] 状态：`DONE` 输出候选方案对比表，并写明采用 / 不采用原因。
  证据：[spec/tech-research-checklist.md](D:\alice\tako\spec\tech-research-checklist.md)

### 3.2 平台与验证资源

- [ ] 状态：`TODO` 确认主开发平台。
  证据：
- [ ] 状态：`TODO` 确认至少一个额外验证平台。
  证据：
- [ ] 状态：`TODO` 列出本地机器、CI runner、人工验证路径。
  证据：
- [ ] 状态：`TODO` 列出当前缺失的验证资源与影响。
  证据：

### 3.3 阶段 0 出口检查

- [ ] 状态：`TODO` 技术选型记录已完成。
  证据：
- [ ] 状态：`TODO` 平台支持矩阵已完成。
  证据：
- [ ] 状态：`TODO` 验证资源状态已明确，不存在重大未决阻塞。
  证据：

## 4. 阶段 1 清单

### 4.1 API 与协议冻结

- [x] 状态：`DONE` `trace_id` 缺省生成策略已写成明确决议。
  证据：[spec/api-sketch.md](D:\alice\tako\spec\api-sketch.md)，[spec/plan.md](D:\alice\tako\spec\plan.md)，[src/runtime/mod.rs](D:\alice\tako\src\runtime\mod.rs)
- [x] 状态：`DONE` Windows Named Pipe 地址规范化规则已冻结。
  证据：[spec/api-sketch.md](D:\alice\tako\spec\api-sketch.md)，[src/api/mod.rs](D:\alice\tako\src\api\mod.rs)，[tests/api_tests.rs](D:\alice\tako\tests\api_tests.rs)
- [x] 状态：`DONE` `Server::register` 最终签名已冻结为 `&mut self -> &mut Self`。
  证据：[spec/api-sketch.md](D:\alice\tako\spec\api-sketch.md)，[src/api/mod.rs](D:\alice\tako\src\api\mod.rs)
- [x] 状态：`DONE` handler 类型擦除边界已冻结为“原始 CBOR payload bytes -> 编码后响应 bytes / ServiceError”。
  证据：[spec/api-sketch.md](D:\alice\tako\spec\api-sketch.md)，[src/runtime/mod.rs](D:\alice\tako\src\runtime\mod.rs)
- [x] 状态：`DONE` 客户端超时后惰性重连语义已冻结。
  证据：[spec/api-sketch.md](D:\alice\tako\spec\api-sketch.md)，[spec/protocol-draft.md](D:\alice\tako\spec\protocol-draft.md)，[spec/plan.md](D:\alice\tako\spec\plan.md)
- [x] 状态：`DONE` 服务端停服接口已冻结为 `serve_until(shutdown)`。
  证据：[spec/api-sketch.md](D:\alice\tako\spec\api-sketch.md)，[src/api/mod.rs](D:\alice\tako\src\api\mod.rs)
- [x] 状态：`DONE` Unix Domain Socket 默认权限策略已冻结。
  证据：[spec/api-sketch.md](D:\alice\tako\spec\api-sketch.md)，[spec/plan.md](D:\alice\tako\spec\plan.md)
- [x] 状态：`DONE` Windows Named Pipe 默认安全策略已冻结。
  证据：[spec/api-sketch.md](D:\alice\tako\spec\api-sketch.md)，[spec/plan.md](D:\alice\tako\spec\plan.md)

### 4.2 文档一致性

- [x] 状态：`DONE` `plan.md`、`protocol-draft.md`、`api-sketch.md` 不再保留未决项。
  证据：[spec/plan.md](D:\alice\tako\spec\plan.md)，[spec/protocol-draft.md](D:\alice\tako\spec\protocol-draft.md)，[spec/api-sketch.md](D:\alice\tako\spec\api-sketch.md)
- [x] 状态：`DONE` 错误映射规则在三份文档中一致。
  证据：[spec/plan.md](D:\alice\tako\spec\plan.md)，[spec/protocol-draft.md](D:\alice\tako\spec\protocol-draft.md)，[spec/api-sketch.md](D:\alice\tako\spec\api-sketch.md)
- [x] 状态：`DONE` 地址抽象与安全策略在三份文档中一致。
  证据：[spec/plan.md](D:\alice\tako\spec\plan.md)，[spec/protocol-draft.md](D:\alice\tako\spec\protocol-draft.md)，[spec/api-sketch.md](D:\alice\tako\spec\api-sketch.md)
- [x] 状态：`DONE` 阶段 2 出口与最终验收已覆盖最小测试矩阵。
  证据：[spec/plan.md](D:\alice\tako\spec\plan.md)，[spec/protocol-draft.md](D:\alice\tako\spec\protocol-draft.md)，[spec/api-sketch.md](D:\alice\tako\spec\api-sketch.md)

### 4.3 示例与测试设计

- [x] 状态：`DONE` 完成 Hello World 成功调用示例。
  证据：[spec/api-sketch.md](D:\alice\tako\spec\api-sketch.md)
- [x] 状态：`DONE` 完成失败调用与错误分支处理示例。
  证据：[spec/api-sketch.md](D:\alice\tako\spec\api-sketch.md)
- [ ] 状态：`TODO` 将最小测试矩阵转成测试设计清单。
  证据：
- [ ] 状态：`TODO` 标明每条测试是单元测试、集成测试还是跨平台验证。
  证据：

### 4.4 阶段 1 出口检查

- [ ] 状态：`TODO` 示例已按冻结后的 API 重新走通。
  证据：
- [ ] 状态：`TODO` 文档之间不存在二义性接口写法。
  证据：
- [ ] 状态：`TODO` 七项冻结决议全部完成。
  证据：

## 5. 阶段 2 清单

### 5.1 子阶段 A：项目骨架与协议类型

- [x] 状态：`DONE` 建立 crate 基础目录结构。
  证据：[src/lib.rs](D:\alice\tako\src\lib.rs)，[src/api/mod.rs](D:\alice\tako\src\api\mod.rs)，[src/protocol/mod.rs](D:\alice\tako\src\protocol\mod.rs)，[src/codec/mod.rs](D:\alice\tako\src\codec\mod.rs)，[src/transport/mod.rs](D:\alice\tako\src\transport\mod.rs)，[src/runtime/mod.rs](D:\alice\tako\src\runtime\mod.rs)，[src/observability/mod.rs](D:\alice\tako\src\observability\mod.rs)
- [x] 状态：`DONE` 落地请求/响应信封与错误体。
  证据：[src/protocol/mod.rs](D:\alice\tako\src\protocol\mod.rs)
- [x] 状态：`DONE` 落地版本常量与地址抽象。
  证据：[src/protocol/mod.rs](D:\alice\tako\src\protocol\mod.rs)，[src/api/mod.rs](D:\alice\tako\src\api\mod.rs)
- [x] 状态：`DONE` 完成协议类型级单元测试。
  证据：[tests/protocol_tests.rs](D:\alice\tako\tests\protocol_tests.rs)

### 5.2 子阶段 B：codec 与状态机

- [x] 状态：`DONE` 实现长度前缀分帧。
  证据：[src/codec/mod.rs](D:\alice\tako\src\codec\mod.rs)，[tests/codec_tests.rs](D:\alice\tako\tests\codec_tests.rs)
- [x] 状态：`DONE` 实现最大帧大小校验。
  证据：[src/codec/mod.rs](D:\alice\tako\src\codec\mod.rs)，[tests/codec_tests.rs](D:\alice\tako\tests\codec_tests.rs)
- [x] 状态：`DONE` 实现 CBOR 编解码与 payload bytes 适配。
  证据：[src/codec/mod.rs](D:\alice\tako\src\codec\mod.rs)，[src/runtime/mod.rs](D:\alice\tako\src\runtime\mod.rs)
- [x] 状态：`DONE` 实现非法长度处理路径。
  证据：[src/codec/mod.rs](D:\alice\tako\src\codec\mod.rs)，[tests/codec_tests.rs](D:\alice\tako\tests\codec_tests.rs)
- [x] 状态：`DONE` 实现超大帧处理路径。
  证据：[src/codec/mod.rs](D:\alice\tako\src\codec\mod.rs)，[tests/codec_tests.rs](D:\alice\tako\tests\codec_tests.rs)
- [x] 状态：`DONE` 实现非法 CBOR 处理路径。
  证据：[src/codec/mod.rs](D:\alice\tako\src\codec\mod.rs)，[tests/codec_tests.rs](D:\alice\tako\tests\codec_tests.rs)
- [x] 状态：`DONE` 实现缺少必填字段处理路径。
  证据：[src/runtime/mod.rs](D:\alice\tako\src\runtime\mod.rs)，[tests/api_windows_e2e_tests.rs](D:\alice\tako\tests\api_windows_e2e_tests.rs)，[tests/api_unix_e2e_tests.rs](D:\alice\tako\tests\api_unix_e2e_tests.rs)
- [x] 状态：`DONE` 实现版本不兼容处理路径。
  证据：[src/runtime/mod.rs](D:\alice\tako\src\runtime\mod.rs)
- [ ] 状态：`TODO` 实现客户端超时后连接失效与惰性重连状态机。
  证据：

### 5.3 子阶段 C：主开发平台传输与最小链路

- [x] 状态：`DONE` 实现主开发平台 listener / accept / read / write。
  证据：[src/transport/windows_named_pipe.rs](D:\alice\tako\src\transport\windows_named_pipe.rs)，[src/transport/mod.rs](D:\alice\tako\src\transport\mod.rs)，[tests/transport_windows_tests.rs](D:\alice\tako\tests\transport_windows_tests.rs)
- [x] 状态：`DONE` 打通 `Client::connect -> call -> Server::register -> serve_until` 成功链路。
  证据：[tests/api_windows_e2e_tests.rs](D:\alice\tako\tests\api_windows_e2e_tests.rs)，[tests/api_unix_e2e_tests.rs](D:\alice\tako\tests\api_unix_e2e_tests.rs)，`cargo test`
- [x] 状态：`DONE` 实现方法不存在返回路径。
  证据：[src/runtime/mod.rs](D:\alice\tako\src\runtime\mod.rs)，[tests/api_windows_e2e_tests.rs](D:\alice\tako\tests\api_windows_e2e_tests.rs)
- [x] 状态：`DONE` 实现服务端业务解码失败返回路径。
  证据：[src/runtime/mod.rs](D:\alice\tako\src\runtime\mod.rs)，[tests/api_windows_e2e_tests.rs](D:\alice\tako\tests\api_windows_e2e_tests.rs)
- [x] 状态：`DONE` 实现服务端内部错误返回路径。
  证据：[src/runtime/mod.rs](D:\alice\tako\src\runtime\mod.rs)
- [x] 状态：`DONE` 实现过期 `deadline_ms` 返回路径。
  证据：[src/runtime/mod.rs](D:\alice\tako\src\runtime\mod.rs)，[tests/api_windows_e2e_tests.rs](D:\alice\tako\tests\api_windows_e2e_tests.rs)
- [x] 状态：`DONE` 落地 Unix Domain Socket 默认权限策略。
  证据：[src/transport/unix.rs](D:\alice\tako\src\transport\unix.rs)，[tests/transport_unix_tests.rs](D:\alice\tako\tests\transport_unix_tests.rs)，`cargo test`
- [x] 状态：`DONE` 落地 Windows Named Pipe 默认安全策略。
  证据：[src/transport/windows_named_pipe.rs](D:\alice\tako\src\transport\windows_named_pipe.rs)，`cargo test`

### 5.4 子阶段 D：自动化测试与稳定性基线

- [x] 状态：`DONE` 成功调用自动化测试通过。
  证据：[tests/api_windows_e2e_tests.rs](D:\alice\tako\tests\api_windows_e2e_tests.rs)，[tests/api_unix_e2e_tests.rs](D:\alice\tako\tests\api_unix_e2e_tests.rs)，`cargo test`
- [x] 状态：`DONE` 方法不存在自动化测试通过。
  证据：[tests/api_windows_e2e_tests.rs](D:\alice\tako\tests\api_windows_e2e_tests.rs)，[tests/api_unix_e2e_tests.rs](D:\alice\tako\tests\api_unix_e2e_tests.rs)，`cargo test`
- [x] 状态：`DONE` 非法长度自动化测试通过。
  证据：[tests/codec_tests.rs](D:\alice\tako\tests\codec_tests.rs)，`cargo test`
- [x] 状态：`DONE` 超大帧自动化测试通过。
  证据：[tests/codec_tests.rs](D:\alice\tako\tests\codec_tests.rs)，`cargo test`
- [x] 状态：`DONE` 非法 CBOR 自动化测试通过。
  证据：[tests/codec_tests.rs](D:\alice\tako\tests\codec_tests.rs)，`cargo test`
- [x] 状态：`DONE` 缺少必填字段自动化测试通过。
  证据：[tests/api_windows_e2e_tests.rs](D:\alice\tako\tests\api_windows_e2e_tests.rs)，[tests/api_unix_e2e_tests.rs](D:\alice\tako\tests\api_unix_e2e_tests.rs)，`cargo test`
- [x] 状态：`DONE` 服务端业务解码失败自动化测试通过。
  证据：[src/runtime/mod.rs](D:\alice\tako\src\runtime\mod.rs)，`cargo test`
- [x] 状态：`DONE` `deadline_ms` 过期拒绝自动化测试通过。
  证据：[src/runtime/mod.rs](D:\alice\tako\src\runtime\mod.rs)，`cargo test`
- [x] 状态：`DONE` 客户端超时自动化测试通过。
  证据：[tests/api_windows_e2e_tests.rs](D:\alice\tako\tests\api_windows_e2e_tests.rs)，[tests/api_unix_e2e_tests.rs](D:\alice\tako\tests\api_unix_e2e_tests.rs)，`cargo test`
- [x] 状态：`DONE` 本地超时后新连接继续自动化测试通过。
  证据：[tests/api_windows_e2e_tests.rs](D:\alice\tako\tests\api_windows_e2e_tests.rs)，[tests/api_unix_e2e_tests.rs](D:\alice\tako\tests\api_unix_e2e_tests.rs)，`cargo test`
- [x] 状态：`DONE` `version != 1` 自动化测试通过。
  证据：[src/runtime/mod.rs](D:\alice\tako\src\runtime\mod.rs)，`cargo test`
- [x] 状态：`DONE` 服务端内部错误自动化测试通过。
  证据：[src/runtime/mod.rs](D:\alice\tako\src\runtime\mod.rs)，`cargo test`
- [ ] 状态：`TODO` 关键日志字段可用于排障。
  证据：

### 5.5 子阶段 E：非主平台真实验证

- [ ] 状态：`TODO` 至少一个非主平台完成真实链路验证。
  证据：
- [ ] 状态：`TODO` 验证步骤、环境和结果已记录。
  证据：
- [ ] 状态：`TODO` 未验证平台已标记为兼容性目标或未验证。
  证据：

### 5.6 阶段 2 出口检查

- [x] 状态：`DONE` 协议草案最小测试矩阵关键项已自动化覆盖。
  证据：[tests/api_windows_e2e_tests.rs](D:\alice\tako\tests\api_windows_e2e_tests.rs)，[tests/api_unix_e2e_tests.rs](D:\alice\tako\tests\api_unix_e2e_tests.rs)，[tests/codec_tests.rs](D:\alice\tako\tests\codec_tests.rs)，[tests/protocol_tests.rs](D:\alice\tako\tests\protocol_tests.rs)，`cargo test`
- [ ] 状态：`TODO` 至少一个非主平台完成真实链路验证。
  证据：
- [x] 状态：`DONE` 超时、断连、错误映射、`deadline_ms` 规则与文档一致。
  证据：[spec/plan.md](D:\alice\tako\spec\plan.md)，[spec/protocol-draft.md](D:\alice\tako\spec\protocol-draft.md)，[spec/api-sketch.md](D:\alice\tako\spec\api-sketch.md)，[src/runtime/mod.rs](D:\alice\tako\src\runtime\mod.rs)，`cargo test`
- [x] 状态：`DONE` 默认安全策略在实现与文档中一致。
  证据：[spec/plan.md](D:\alice\tako\spec\plan.md)，[spec/api-sketch.md](D:\alice\tako\spec\api-sketch.md)，[src/transport/unix.rs](D:\alice\tako\src\transport\unix.rs)，[src/transport/windows_named_pipe.rs](D:\alice\tako\src\transport\windows_named_pipe.rs)，`cargo test`

## 6. 阶段 3 清单

### 6.1 文档与观测性

- [ ] 状态：`TODO` 接入文档覆盖地址命名、权限策略、超时语义、错误分类、并发排队语义。
  证据：
- [ ] 状态：`TODO` 关键日志事件与字段名称统一。
  证据：
- [ ] 状态：`TODO` 发布检查清单已完成。
  证据：

### 6.2 平台口径与发布

- [ ] 状态：`TODO` 已验证平台列表准确。
  证据：
- [ ] 状态：`TODO` 兼容性目标平台列表准确。
  证据：
- [ ] 状态：`TODO` 未验证平台与风险说明准确。
  证据：

### 6.3 阶段 3 出口检查

- [ ] 状态：`TODO` 发布说明、测试结果和文档口径一致。
  证据：
- [ ] 状态：`TODO` 用户可按文档完成一次成功调用和一次失败排障。
  证据：

## 7. 发布前总检查

- [ ] 状态：`TODO` crate 源码与示例可构建。
  证据：
- [ ] 状态：`TODO` 自动化测试全部通过。
  证据：
- [ ] 状态：`TODO` 已验证平台的真实验证记录齐全。
  证据：
- [ ] 状态：`TODO` 平台支持口径与验证结果一致。
  证据：
- [ ] 状态：`TODO` 已知限制与剩余风险已在发布说明中写明。
  证据：
