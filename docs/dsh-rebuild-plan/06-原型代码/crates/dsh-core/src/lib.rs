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
