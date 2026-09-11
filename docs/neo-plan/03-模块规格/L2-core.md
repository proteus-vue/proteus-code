# L2 · 内核规格 ★

**crate**：`neo-core`
**依赖**：`neo-protocol`(L1)、`neo-sandbox`/`neo-platform`(L0)
**禁止依赖**：L3 capability、L4 orchestration、L5 host

> **这一层不可替换**（ADR-0001）。这是与 DSH 最根本的分歧。

## 组成

### 1. Session 状态机
```
Idle → Planning → Executing → AwaitingApproval → Executing → ... → Idle
```
任一状态可因 `Op::Interrupt` 在**安全点**（工具调用边界）中断。

### 2. run_turn 主循环
见 `02-架构设计/会话状态机与Checkpoint.md`。
关键点：**固定前缀字节稳定** + **预采样 compact** + **CancellationToken 安全取消**。

### 3. ContextManager
| 职责 | 说明 |
|---|---|
| 固定前缀组装 | system + 工具描述，**按固定顺序**，禁 HashMap 随机序 |
| 增量历史 | append，不重写 |
| compact | 预算检测触发，压缩而非截断 |
| fresh-by-default | M4 用 JSONL+BM25；M6 后可加向量索引 |
| AGENTS.md 级联 | 全局 override > 全局 > 项目根 > 目录；上限 32 KiB |

### 4. ToolRouter（依赖注入，不依赖具体工具）
```rust
pub trait Tool {
    fn name(&self) -> &str;
    fn describe(&self) -> ToolSpec;      // 用于生成工具描述
    fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput>;
}
```
L2 只认 trait，**具体工具由 L3 注册进来**。
→ 这是"L2 不依赖 L3"能成立的技术保证。

### 5. ApprovalGate（正交双轴判定）
```rust
fn gate(&self, call: &ToolCall) -> GateDecision {
    // 先判沙箱（技术边界，不可被审批覆盖）
    if !sandbox_allows(call) { return GateDecision::Denied(SandboxViolation); }
    // 再判审批（流程：是否需要问人）
    match approval_policy { ... }
}
```

### 6. ModelProvider（扩展点之一）
```rust
pub trait ModelProvider {
    async fn stream(&self, req: Request) -> impl Stream<Item = Delta>;
}
```
实现：DeepSeek / OpenAI / Anthropic / 自定义 OpenAI 兼容端点。

## 配置层级（吸收 Codex 四级）
```
内置默认 → /etc/neo/config.toml → ~/.neo/config.toml → .neo/config.toml → CLI flag
```
后置覆盖前置。安全敏感键（`model_provider`、`profile`、`notify`）**只允许在用户级设置**，项目级忽略。

## 验收
- [ ] `neo-core` 不依赖 L3/L4/L5（架构守卫）
- [ ] 同一 Op 序列 → 同一 EventMsg 序列（golden 回放）
- [ ] 工具描述顺序在多次运行间字节一致（缓存命中前提）
- [ ] 中断只在工具边界发生，无半写状态
