//! L1 PROTOCOL —— 线协议层
//!
//! 铁律：本 crate 只依赖 serde。不得引入任何业务 crate。
//! 理由：这是改动最频繁的一层，必须编译最快；也是宿主与引擎的唯一契约。

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

// ---------- 上下文引用（吸收 ZCode 的 @ / # / / / $ 体系） ----------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextRef {
    pub kind: RefKind,
    pub target: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RefKind {
    File,    // @path
    Session, // #session
    Command, // /command
    Skill,   // $skill
}

// ---------- 宿主 -> 引擎 ----------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    UserTurn { text: String, refs: Vec<ContextRef> },
    Interrupt,
    Approve { id: ApprovalId, decision: Decision },
    ConfigureSession { patch: SessionPatch },
    Compact,
    Fork,
    GoalSet { goal: String },
    GoalPause { goal_id: GoalId },
    GoalResume { goal_id: GoalId },
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Allow,
    AllowAlways,
    Deny,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionPatch {
    pub exec_mode: Option<ExecMode>,
    pub sandbox_mode: Option<SandboxMode>,
    pub approval_policy: Option<ApprovalPolicy>,
    pub model: Option<String>,
}

/// ZCode 五档执行模式（UI 档位，映射到沙箱 × 审批双轴）
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecMode {
    Plan,               // 计划模式：先出计划再动手
    ConfirmBefore,      // 变更前确认
    Default,            // 默认（推荐）
    AutoEdit,           // 自动编辑
    FullAccess,         // 完全访问（风险自担）
}

/// 轴二：沙箱（技术边界，OS 强制，agent 无法绕过）
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SandboxMode {
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

/// 轴一：审批（流程边界，何时必须暂停问人）
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPolicy {
    Untrusted,
    OnRequest,
    OnFailure,
    Never,
}

pub type ApprovalId = String;
pub type GoalId = String;
pub type SubmissionId = u64;
pub type ToolCallId = String;
pub type Seq = u64;

// ---------- 引擎 -> 宿主 ----------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventMsg {
    SessionConfigured { session_id: String },
    TurnStarted { turn_id: String },
    AgentMessageDelta { delta: String },
    AgentMessageDone { text: String },
    ReasoningDelta { delta: String },
    ToolCallBegin { id: ToolCallId, name: String },
    ToolCallEnd { id: ToolCallId, exit_code: i32 },
    ApprovalRequest { id: ApprovalId, detail: String },
    PatchProposed { path: String, diff: String },
    CheckpointSaved { checkpoint_id: String },
    GoalProgress { goal_id: GoalId, done: usize, total: usize },
    Error { message: String },
    TurnComplete { input_tokens: u64, output_tokens: u64 },
    ShutdownComplete,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
}
