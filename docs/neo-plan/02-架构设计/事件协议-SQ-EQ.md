# 事件协议：Submission Queue / Event Queue

> 吸收自 Codex。这是"多宿主共享内核"能成立的**唯一技术前提**。

## 核心契约

```rust
// 宿主 → 引擎
pub fn submit(&self, op: Op) -> SubmissionId;

// 引擎 → 宿主
pub async fn next_event(&self) -> EventMsg;
```

宿主不直接调用引擎函数，引擎也不直接调用 UI。
双方只通过两个队列通信：**SQ（Submission Queue）** 与 **EQ（Event Queue）**。

## 为什么必须这样

| 收益 | 说明 |
|---|---|
| **多宿主复用** | Web / TUI / Exec 消费同一条 EventMsg 流，业务逻辑零重复 |
| **可录制可回放** | Op 序列 + EventMsg 序列 = 完整审计日志，也 = 测试用例 |
| **流式 UX** | `AgentMessageDelta` 逐字推送 |
| **可取消 / 可打断** | `Op::Interrupt` 进入 SQ，引擎在安全点处理 |
| **状态线性化** | `submission_loop` 作为独立 tokio 任务，保证状态变更串行 |

## Op（宿主 → 引擎）

```rust
pub enum Op {
    UserTurn { text: String, refs: Vec<ContextRef> }, // refs: @文件 #会话 /命令 $技能
    Interrupt,                                        // 打断当前执行
    Approve { id: ApprovalId, decision: Decision },   // 审批决策
    ConfigureSession { patch: SessionPatch },         // 改执行模式 / 沙箱 / 模型
    Compact,                                          // 手动压缩上下文
    Fork,                                             // 分叉会话
    GoalSet   { goal: String },                       // ZCode Goal Mode
    GoalPause { goal_id: GoalId },
    GoalResume{ goal_id: GoalId },
    Shutdown,
}
```

## EventMsg（引擎 → 宿主）

```rust
pub enum EventMsg {
    SessionConfigured { session_id: String },
    TurnStarted       { turn_id: String },
    AgentMessageDelta { delta: String },
    AgentMessageDone  { text: String },
    ReasoningDelta    { delta: String },        // 独立通道，UI 折叠显示
    ToolCallBegin     { id: ToolCallId, name: String, args: Value },
    ToolCallEnd       { id: ToolCallId, output: ToolOutput },
    ApprovalRequest   { id: ApprovalId, kind: ApprovalKind, detail: String },
    PatchProposed     { path: String, diff: String },   // apply_patch 信封
    CheckpointSaved   { checkpoint_id: String },        // Goal 断点
    GoalProgress      { goal_id: GoalId, done: usize, total: usize },
    Error             { message: String, source: ErrorSource },
    TurnComplete      { usage: Usage },
    ShutdownComplete,
}
```

## 确定性要求（验证点）

> **同一 Op 序列，必须产出同一 EventMsg 序列。**

非确定性只允许出现在两处，且必须显式标注：
1. `AgentMessageDelta` / `ReasoningDelta` 的**分块边界**
2. 模型返回的**内容本身**

**协议骨架必须确定**：事件的**类型、顺序、ID 生成规则**不得因宿主、时序、并发而变化。

这一条由 `05-验证/checks/check_protocol.py` 用 golden 回放强制校验。

## 与 DSH 会话日志的关系

DSH 的 append-only 会话日志理念**完全保留**，并升级：
- 每条 Op / EventMsg 落盘为一行 JSONL
- **JSONL 既是审计日志，也是回放测试的输入与期望输出**（继承 DSH 的无 API Key 测试方案）
