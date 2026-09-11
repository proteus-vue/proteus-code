//! L2 CORE —— 内核（不可替换，见 ADR-0001）
//!
//! 关键约束：本 crate 只认 `Tool` trait，不认任何具体工具实现。
//! 具体工具由 L3 通过 `ToolRegistry::register` 注入 —— 这是"L2 不依赖 L3"的技术保证。

use dsh_config::Config;
use dsh_protocol::*;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

// ---------- 扩展点 1/4：Tool ----------

pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn describe(&self) -> String;
    fn execute(&self, args: &Value) -> ToolOutput;
}

/// 用 BTreeMap 而非 HashMap —— 保证工具描述顺序稳定（提示词缓存命中的前提）
pub struct ToolRegistry {
    tools: BTreeMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self { Self { tools: BTreeMap::new() } }
    pub fn register(&mut self, t: Arc<dyn Tool>) { self.tools.insert(t.name().to_string(), t); }

    /// 固定顺序输出描述。**顺序必须字节稳定**，否则 prompt cache 全 miss。
    pub fn render_prompt(&self) -> String {
        self.tools.values().map(|t| t.describe()).collect::<Vec<_>>().join("\n")
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> { self.tools.get(name).cloned() }
}

// ---------- 扩展点 2/4：ModelProvider ----------

pub trait ModelProvider: Send + Sync {
    fn name(&self) -> &str;
    fn complete(&self, prompt: &str) -> String;
}

// ---------- 审批闸门：沙箱 × 审批 正交双轴 ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateDecision {
    Allowed,
    NeedApproval(ApprovalPolicy),
    Denied(&'static str),
}

pub struct ApprovalGate;

impl ApprovalGate {
    /// 先判沙箱（技术边界，不可被审批覆盖），再判审批（流程：何时问人）
    pub fn gate(call: &str, want_network: bool, cfg: &Config) -> GateDecision {
        match cfg.sandbox_mode {
            SandboxMode::ReadOnly => {
                if call == "bash" { return GateDecision::NeedApproval(cfg.approval_policy); }
                if call == "apply_patch" { return GateDecision::NeedApproval(cfg.approval_policy); }
                GateDecision::Allowed
            }
            SandboxMode::WorkspaceWrite => {
                if want_network { return GateDecision::NeedApproval(cfg.approval_policy); }
                match cfg.approval_policy {
                    ApprovalPolicy::Never => GateDecision::Allowed,
                    p => GateDecision::NeedApproval(p),
                }
            }
            SandboxMode::DangerFullAccess => GateDecision::Allowed,
        }
    }
}

// ---------- Session 状态机 ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State { Idle, Planning, Executing, AwaitingApproval }

pub struct Session {
    pub state: State,
    pub cfg: Config,
    pub registry: ToolRegistry,
}

impl Session {
    pub fn new(cfg: Config) -> Self {
        Self { state: State::Idle, cfg, registry: ToolRegistry::new() }
    }

    /// 状态转移。**Interrupt 只在工具调用边界生效**（安全点），杜绝半写状态。
    pub fn transition(&mut self, op: &Op) -> State {
        match (self.state, op) {
            (_, Op::Shutdown) => State::Idle,
            // 中断在任意状态生效（安全点由调用方保证：工具调用边界）
            (_, Op::Interrupt) => State::Idle,
            (State::Idle, Op::UserTurn { .. }) | (State::Idle, Op::GoalSet { .. }) => State::Planning,
            (State::Planning, _) => State::Executing,
            (State::Executing, _) => State::Executing,
            (State::AwaitingApproval, Op::Approve { .. }) => State::Executing,
            (State::AwaitingApproval, _) => State::AwaitingApproval,
            (_, _) => self.state,
        }
    }
}

// ---------- 宿主 SPI：契据在 L2，实现在 L5 ----------
//
// 方法论依据（见 02-架构设计/Proteus方法论-语义核心与后端SPI.md）：
// 宿主是"后端实现细节"。只给一种实现时，"换 UI 不动内核"是未经证实的宣称，
// 故要求 >=2 个真实后端（tui + desktop）并由 T6 铁律机器校验。

/// 宿主能力自描述。存在的意义：**让降级变成数据驱动**。
/// 没有它，工具就得写 `if is_tui {..} else {..}` —— 那正是 Proteus 反对的
/// "业务代码里出现平台分支"。有了它，工具只输出规范值 + 能力，
/// 由宿主自行挑选最合适的呈现方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostCapabilities {
    pub images: ImageSupport,
    pub rich_text: bool,
    pub interactive_prompt: bool,
    pub diffs: DiffSupport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageSupport { None, Inline, External }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffSupport { None, Text, Hunk }

/// 宿主后端契据。实现在 L5；宿主之间不得互相依赖（check_architecture 强制）。
pub trait HostBackend: Send {
    /// 宿主标识（用于 T6 等价性断言定位）。
    fn id(&self) -> &'static str;

    /// 能力自描述。
    fn capabilities(&self) -> HostCapabilities;

    /// 消费一条事件（JSON）。返回 Err 表示该事件不被本宿主支持 —— 这是
    /// T6 断言 (a) "都能消费完" 的判据。
    fn consume(&mut self, event_json: &str) -> Result<(), String>;

    /// 宿主已渲染的"用户可见事实"集合，用于 T6 断言 (b) 语义等价。
    fn rendered_facts(&self) -> Vec<String>;
}

/// 宿主注册表：内核只认契据，不认具体后端。
pub struct HostRegistry { hosts: Vec<Box<dyn HostBackend>> }

impl HostRegistry {
    pub fn new() -> Self { Self { hosts: Vec::new() } }
    pub fn register(&mut self, h: Box<dyn HostBackend>) { self.hosts.push(h); }
    pub fn ids(&self) -> Vec<&'static str> { self.hosts.iter().map(|h| h.id()).collect() }

    /// 把同一事件流喂给所有宿主。T6 的运行时形态。
    pub fn broadcast(&mut self, event_json: &str) -> Vec<(&'static str, Result<(), String>)> {
        self.hosts.iter_mut().map(|h| (h.id(), h.consume(event_json))).collect()
    }
}
