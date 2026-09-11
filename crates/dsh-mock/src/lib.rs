//! SPI Mock/Headless 后端（test-support）
//!
//! SPI-First 纪律：**Mock 必须要有**。单后端 SPI 是假 SPI（AP-01）——
//! 可替换性从未被验证，接口里几乎必然泄漏实现细节。
//!
//! 本 crate 为每个 SPI 提供一个 Mock，使「同一份 conformance 用例跑所有后端」
//! 成为可能（Step 3），并提供故意违规的坏后端作为**负向用例**的被试。

use dsh_core::{
    CallKind, DiffSupport, HostBackend, HostCapabilities, ImageSupport, LoggedRecord,
    ModelDelta, ModelProvider, ModelRequest, ModelStream, PersistenceError, SandboxBackend,
    SandboxOutcome, SessionPersistence, Tool, ToolCtx, ToolInvocation,
};
use dsh_protocol::{SandboxMode, ToolOutput};

// ---------- HostBackend ----------

/// 无头宿主 Mock：只登记事实，不渲染。
pub struct MockHost { facts: Vec<String>, id: &'static str }

impl MockHost {
    /// `id` 用于在等价性断言里定位是哪个宿主。
    pub fn new(id: &'static str) -> Self { Self { facts: Vec::new(), id } }
}

impl HostBackend for MockHost {
    fn id(&self) -> &'static str { self.id }
    fn capabilities(&self) -> HostCapabilities {
        HostCapabilities {
            images: ImageSupport::None,
            rich_text: false,
            interactive_prompt: false,
            diffs: DiffSupport::None,
        }
    }
    fn consume(&mut self, event_json: &str) -> Result<(), String> {
        // Headless 契约：任何事件都必须能"消费"（不渲染 ≠ 不消费）。
        self.facts.push(format!("[{}] {event_json}", self.id));
        Ok(())
    }
    fn rendered_facts(&self) -> Vec<String> { self.facts.clone() }
}

/// 故意违规的坏宿主：遇到无法处理的事件直接报错。
/// **负向用例的被试** —— 证明 conformance 套件有牙齿。
pub struct BrittleHost;

impl HostBackend for BrittleHost {
    fn id(&self) -> &'static str { "brittle" }
    fn capabilities(&self) -> HostCapabilities {
        HostCapabilities { images: ImageSupport::None, rich_text: false, interactive_prompt: false, diffs: DiffSupport::None }
    }
    fn consume(&mut self, event_json: &str) -> Result<(), String> {
        if event_json.contains("\"kind\":\"unsupported\"") { Err("cannot render this event".into()) }
        else { Ok(()) }
    }
    fn rendered_facts(&self) -> Vec<String> { Vec::new() }
}

// ---------- ModelProvider ----------

/// 脚本化 Mock 模型：按**步**给出预定响应，最后一轮之后返回纯文本结束。
///
/// 为什么要脚本化：agent 的核心是"多步 + 工具调用"的控制流。
/// 只有能精确编排每步产出的 provider，才能确定性地测试主循环
/// （进入下一步 / 挂审批 / 结束本轮）。确定性是 T2 可回放的前提。
pub struct ScriptedModelProvider {
    /// 第 n 步的响应脚本。用完后返回固定的收尾文本。
    pub script: Vec<Vec<ModelDelta>>,
    /// 脚本用尽后的收尾文本（默认 "done"）。
    pub tail: String,
}

impl ScriptedModelProvider {
    pub fn new(script: Vec<Vec<ModelDelta>>) -> Self {
        Self { script, tail: "done".into() }
    }
    /// 空脚本：模型从不调用工具，一轮即止。
    pub fn text_only(text: &str) -> Self {
        Self { script: Vec::new(), tail: text.into() }
    }
}

impl ModelProvider for ScriptedModelProvider {
    fn name(&self) -> &str { "scripted" }
    fn stream(&self, request: &ModelRequest) -> ModelStream {
        // 用请求里 assistant 消息的条数推断"第几步"，从而天然确定性：
        // 同一请求序列 ⇒ 同一脚本位置 ⇒ 同一输出。
        let step_index = request
            .messages
            .iter()
            .filter(|m| matches!(m, dsh_core::Message::Assistant { .. }))
            .count();
        let deltas = self
            .script
            .get(step_index)
            .cloned()
            .unwrap_or_else(|| vec![ModelDelta::Text(self.tail.clone())]);
        Box::new(deltas.into_iter())
    }
}

/// 确定性 Mock 模型：**同输入必同输出**（不依赖脚本位置）。
pub struct MockModelProvider;

impl ModelProvider for MockModelProvider {
    fn name(&self) -> &str { "mock" }
    fn stream(&self, request: &ModelRequest) -> ModelStream {
        let n = request.messages.len();
        Box::new(vec![ModelDelta::Text(format!("mock-response:{n}"))].into_iter())
    }
}

/// 便利构造：一次工具调用增量。
pub fn tool_call(id: &str, name: &str, args: serde_json::Value) -> ModelDelta {
    ModelDelta::ToolCall(ToolInvocation {
        id: id.to_string(),
        name: name.to_string(),
        arguments: args,
    })
}

// ---------- SessionPersistence ----------

/// 内存持久化 Mock。强制 append-only 语义。
pub struct InMemoryPersistence { records: Vec<LoggedRecord>, next: u64 }

impl InMemoryPersistence {
    pub fn new() -> Self { Self { records: Vec::new(), next: 1 } }
}
impl Default for InMemoryPersistence { fn default() -> Self { Self::new() } }

impl SessionPersistence for InMemoryPersistence {
    fn append(&mut self, kind: &str, payload: serde_json::Value) -> Result<u64, PersistenceError> {
        let seq = self.next;
        self.next += 1;
        self.records.push(LoggedRecord { seq, kind: kind.to_string(), payload });
        Ok(seq)
    }
    fn load(&self) -> Result<Vec<LoggedRecord>, PersistenceError> { Ok(self.records.clone()) }
}

/// 故意违规的持久化：`load` 会**丢弃首条**记录。
///
/// 这是真实世界最常见的破坏方式 —— 把「压缩日志」误当成「读回日志」。
/// 违规必须对**任意输入**都成立，否则负向用例会因输入不含特定子串而漏判
/// （本实现的第一版就踩了这个坑：只在含 "user" 时改写，导致契约用例的
/// `{"a":1}` 输入触发不了违规，负向用例自己失败了 —— 说明负向用例确实有牙齿）。
/// **负向用例的被试**。
pub struct TamperingPersistence { records: Vec<LoggedRecord>, next: u64 }

impl TamperingPersistence {
    pub fn new() -> Self { Self { records: Vec::new(), next: 1 } }
}
impl Default for TamperingPersistence { fn default() -> Self { Self::new() } }

impl SessionPersistence for TamperingPersistence {
    fn append(&mut self, kind: &str, payload: serde_json::Value) -> Result<u64, PersistenceError> {
        let seq = self.next;
        self.next += 1;
        self.records.push(LoggedRecord { seq, kind: kind.to_string(), payload });
        Ok(seq)
    }
    fn load(&self) -> Result<Vec<LoggedRecord>, PersistenceError> {
        // 违规：静默丢掉首条 —— 对任意输入都破坏 append-only。
        Ok(self.records.iter().skip(1).cloned().collect())
    }
}

// ---------- SandboxBackend ----------

/// Noop 沙箱：只能用于测试。**不提供任何隔离**，能力声明为"仅 danger-full-access"。
pub struct NoopSandbox;

impl SandboxBackend for NoopSandbox {
    fn supports(&self, mode: SandboxMode) -> bool { matches!(mode, SandboxMode::DangerFullAccess) }
    fn execute(&self, mode: SandboxMode, command: &str, _limit: usize) -> SandboxOutcome {
        if self.supports(mode) {
            SandboxOutcome::Ran { stdout: format!("ran:{command}"), truncated: false }
        }
        // 能力不足时**显式拒绝**，不静默放行 —— SPI-First Step 5 能力边界要求。
        else { SandboxOutcome::Denied { reason: format!("noop backend cannot enforce {mode:?}") } }
    }
}

/// 故意违规的沙箱：声称支持 read-only，实际照样执行。
/// **负向用例的被试** —— 这是最危险的一类实现（安全边界形同虚设）。
pub struct LeakySandbox;

impl SandboxBackend for LeakySandbox {
    fn supports(&self, _mode: SandboxMode) -> bool { true }
    fn execute(&self, _mode: SandboxMode, command: &str, _limit: usize) -> SandboxOutcome {
        SandboxOutcome::Ran { stdout: format!("ran:{command}"), truncated: false }
    }
}

// ---------- Tool（SPI-First Step 2：Mock 必须要有）----------

/// Mock 工具：行为可预测，用于验证工具契约。
pub struct MockTool { pub tool_name: String, pub kind: CallKind }

impl MockTool {
    pub fn new(tool_name: &str) -> Self {
        Self { tool_name: tool_name.to_string(), kind: CallKind::Read }
    }
    pub fn writing(tool_name: &str) -> Self {
        Self { tool_name: tool_name.to_string(), kind: CallKind::Write }
    }
}

impl Tool for MockTool {
    fn name(&self) -> &str { &self.tool_name }
    fn describe(&self) -> String { format!("{}(args): mock 工具，用于契约测试。", self.tool_name) }
    fn call_kind(&self, _args: &serde_json::Value) -> CallKind { self.kind }
    fn execute(&self, _args: &serde_json::Value, _ctx: &ToolCtx) -> ToolOutput {
        ToolOutput { exit_code: 0, stdout: "ok".into(), stderr: String::new(), truncated: false }
    }
}

/// 有状态工具：每次执行递增计数，返回计数。用于验证"同一步内多次调用"的顺序。
pub struct CountingTool { pub calls: std::sync::atomic::AtomicUsize }

impl CountingTool {
    pub fn new() -> Self { Self { calls: std::sync::atomic::AtomicUsize::new(0) } }
}
impl Default for CountingTool { fn default() -> Self { Self::new() } }

impl Tool for CountingTool {
    fn name(&self) -> &str { "count" }
    fn describe(&self) -> String { "count(): 返回本会话内第几次调用。".into() }
    fn call_kind(&self, _args: &serde_json::Value) -> CallKind { CallKind::Read }
    fn execute(&self, _args: &serde_json::Value, _ctx: &ToolCtx) -> ToolOutput {
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        ToolOutput { exit_code: 0, stdout: format!("call-{n}"), stderr: String::new(), truncated: false }
    }
}

/// 经沙箱执行的工具：证明"工具想执行命令只能走 ctx"这条结构保证。
pub struct SandboxProbeTool;

impl Tool for SandboxProbeTool {
    fn name(&self) -> &str { "probe" }
    fn describe(&self) -> String { "probe(cmd): 经沙箱执行一条命令并返回结果。".into() }
    fn call_kind(&self, _args: &serde_json::Value) -> CallKind { CallKind::Read }
    fn execute(&self, args: &serde_json::Value, ctx: &ToolCtx) -> ToolOutput {
        let cmd = args.get("cmd").and_then(|v| v.as_str()).unwrap_or("true");
        match ctx.exec(cmd) {
            SandboxOutcome::Ran { stdout, truncated } => {
                ToolOutput { exit_code: 0, stdout, stderr: String::new(), truncated }
            }
            SandboxOutcome::Denied { reason } => ToolOutput { exit_code: -1, stdout: String::new(), stderr: reason, truncated: false },
        }
    }
}

/// 故意违规的工具：**名字为空**。
///
/// 模型无法调用一个无名工具，因此空名工具是"注册了但不可用"的死工具 ——
/// 契约必须抓住它。**负向用例的被试**。
pub struct NamelessTool;

impl Tool for NamelessTool {
    fn name(&self) -> &str { "" }
    fn describe(&self) -> String { String::new() }
    fn call_kind(&self, _args: &serde_json::Value) -> CallKind { CallKind::Read }
    fn execute(&self, _args: &serde_json::Value, _ctx: &ToolCtx) -> ToolOutput {
        ToolOutput { exit_code: 0, stdout: String::new(), stderr: String::new(), truncated: false }
    }
}
