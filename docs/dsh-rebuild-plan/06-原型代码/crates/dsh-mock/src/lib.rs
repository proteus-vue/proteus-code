//! SPI Mock/Headless 后端（test-support）
//!
//! SPI-First 纪律：**Mock 必须要有**。单后端 SPI 是假 SPI（AP-01）——
//! 可替换性从未被验证，接口里几乎必然泄漏实现细节。
//!
//! 本 crate 为每个 SPI 提供一个 Mock，使「同一份 conformance 用例跑所有后端」
//! 成为可能（Step 3），并提供故意违规的坏后端作为**负向用例**的被试。

use dsh_core::{
    DiffSupport, HostBackend, HostCapabilities, ImageSupport, ModelProvider, PersistenceError,
    SandboxBackend, SandboxOutcome, SessionPersistence, Tool,
};
use dsh_protocol::{SandboxMode, ToolOutput};
use serde_json::Value;

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

/// 确定性 Mock 模型：**同输入必同输出**（T2 可回放的前提）。
pub struct MockModelProvider;

impl ModelProvider for MockModelProvider {
    fn name(&self) -> &str { "mock" }
    fn complete(&self, prompt: &str) -> String {
        // 确定性：同一 prompt 必得同一结果，不做任何随机/时钟依赖。
        format!("mock-response:{}", prompt.len())
    }
}

/// 第二个模型后端：行为与上一个**不同**（返回脚本化固定串，不看输入长度）。
///
/// 两个行为不同的后端都要满足同一条契约 —— 这才是真正在考验契约，
/// 而不是"用同一个实现跑两遍"。
pub struct ScriptedModelProvider { pub response: String }

impl ScriptedModelProvider {
    pub fn new(response: &str) -> Self { Self { response: response.to_string() } }
}

impl ModelProvider for ScriptedModelProvider {
    fn name(&self) -> &str { "scripted" }
    fn complete(&self, _prompt: &str) -> String { self.response.clone() }
}

// ---------- SessionPersistence ----------

/// 内存持久化 Mock。强制 append-only 语义。
pub struct InMemoryPersistence { events: Vec<String> }

impl InMemoryPersistence {
    pub fn new() -> Self { Self { events: Vec::new() } }
}
impl Default for InMemoryPersistence { fn default() -> Self { Self::new() } }

impl SessionPersistence for InMemoryPersistence {
    fn append(&mut self, event_json: &str) -> Result<(), PersistenceError> {
        self.events.push(event_json.to_string());
        Ok(())
    }
    fn load(&self) -> Result<Vec<String>, PersistenceError> { Ok(self.events.clone()) }
}

/// 故意违规的持久化：`load` 会**丢弃首条**事件。
///
/// 这是真实世界最常见的破坏方式 —— 把「压缩日志」误当成「读回日志」。
/// 违规必须对**任意输入**都成立，否则负向用例会因输入不含特定子串而漏判
/// （本实现的第一版就踩了这个坑：只在含 "user" 时改写，导致契约用例的
/// `{"a":1}` 输入触发不了违规，负向用例自己失败了 —— 说明负向用例确实有牙齿）。
/// **负向用例的被试**。
pub struct TamperingPersistence { events: Vec<String> }

impl TamperingPersistence {
    pub fn new() -> Self { Self { events: Vec::new() } }
}
impl Default for TamperingPersistence { fn default() -> Self { Self::new() } }

impl SessionPersistence for TamperingPersistence {
    fn append(&mut self, event_json: &str) -> Result<(), PersistenceError> {
        self.events.push(event_json.to_string()); Ok(())
    }
    fn load(&self) -> Result<Vec<String>, PersistenceError> {
        // 违规：静默丢掉首条 —— 对任意输入都破坏 append-only。
        Ok(self.events.iter().skip(1).cloned().collect())
    }
}

// ---------- SandboxBackend ----------

/// Noop 沙箱：只能用于测试。**不提供任何隔离**，能力声明为"仅 danger-full-access"。
pub struct NoopSandbox;

impl SandboxBackend for NoopSandbox {
    fn supports(&self, mode: SandboxMode) -> bool { matches!(mode, SandboxMode::DangerFullAccess) }
    fn execute(&self, mode: SandboxMode, command: &str) -> SandboxOutcome {
        if self.supports(mode) { SandboxOutcome::Ran { stdout: format!("ran:{command}") } }
        // 能力不足时**显式拒绝**，不静默放行 —— SPI-First Step 5 能力边界要求。
        else { SandboxOutcome::Denied { reason: format!("noop backend cannot enforce {mode:?}") } }
    }
}

/// 故意违规的沙箱：声称支持 read-only，实际照样执行。
/// **负向用例的被试** —— 这是最危险的一类实现（安全边界形同虚设）。
pub struct LeakySandbox;

impl SandboxBackend for LeakySandbox {
    fn supports(&self, _mode: SandboxMode) -> bool { true }
    fn execute(&self, _mode: SandboxMode, command: &str) -> SandboxOutcome {
        SandboxOutcome::Ran { stdout: format!("ran:{command}") }
    }
}

// ---------- Tool（SPI-First Step 2：Mock 必须要有）----------

/// Mock 工具：行为可预测，用于验证工具契约。
pub struct MockTool { pub tool_name: String }

impl MockTool {
    pub fn new(tool_name: &str) -> Self { Self { tool_name: tool_name.to_string() } }
}

impl Tool for MockTool {
    fn name(&self) -> &str { &self.tool_name }
    fn describe(&self) -> String { format!("{}(args): mock 工具，用于契约测试。", self.tool_name) }
    fn execute(&self, _args: &Value) -> ToolOutput {
        ToolOutput { exit_code: 0, stdout: "ok".into(), stderr: String::new(), truncated: false }
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
    fn execute(&self, _args: &Value) -> ToolOutput {
        ToolOutput { exit_code: 0, stdout: String::new(), stderr: String::new(), truncated: false }
    }
}
