# L0 · 协议层规格 ★ 基石

**crate**：`dsh-protocol`
**依赖**：仅 `serde` / `serde_json`（**禁止依赖任何业务 crate**）
**职责**：纯数据类型定义，无业务逻辑，无副作用

> 为什么单独一层且零业务依赖：**这是改动最频繁、编译必须最快的地方**。
> 也是宿主与引擎之间唯一的契约，必须能独立版本化。

## 类型清单

```rust
// 宿主 → 引擎
pub struct Submission { pub id: SubmissionId, pub op: Op }
pub enum Op { /* 见 02-架构设计/事件协议-SQ-EQ.md */ }

// 引擎 → 宿主
pub enum EventMsg { /* 同见 */ }

// 共享值对象
pub struct ContextRef { pub kind: RefKind, pub target: String }  // @文件 #会话 /命令 $技能
pub enum  RefKind { File, Session, Command, Skill }

pub struct ToolOutput { pub exit_code: i32, pub stdout: String, pub stderr: String, pub truncated: bool }
pub struct Usage { pub input_tokens: u64, pub output_tokens: u64, pub cached_tokens: u64 }
pub enum  ErrorSource { Model, Tool, Sandbox, Config, Internal }
```

## 版本化与兼容

- 所有枚举带 `#[serde(rename_all = "snake_case")]`
- 新增变体用 `#[non_exhaustive]` 或提供 `Unknown` 兜底，**旧宿主不得因新事件崩溃**
- 每个事件带 `schema_version`

## 序列化落盘格式（JSONL，每行一个事件）

```json
{"v":1,"ts":"2026-09-10T10:00:00Z","seq":42,"kind":"tool_call_begin","payload":{...}}
```
字段固定：**`v`（schema 版本）、`ts`、`seq`（单调递增序号）、`kind`、`payload`**。
`seq` 必须严格递增 —— 这是"append-only 未破坏"的校验依据（见 `check_session_schema.py`）。

## 验收
- [ ] `cargo build -p dsh-protocol` 不引入任何业务 crate
- [ ] JSONL 每条事件通过 schema 校验
- [ ] `seq` 严格递增，无重复、无空洞
