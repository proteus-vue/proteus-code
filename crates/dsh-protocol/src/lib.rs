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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
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

// ---------- 用户可见事实：宿主无关的语义抽取 ----------
//
// 为什么放在协议层而不是宿主层：
//   「哪些事实是用户必须知道的」是**事件语义**，不是呈现细节。
//   TUI 画成彩色文本、exec 打成日志、Web 渲染成卡片 —— 但三者必须
//   传达**同一组事实**。把抽取放在协议层，T6（宿主语义等价）才有可比较的基准：
//   否则"等价"只能靠人工比对渲染结果，无法机器判定。
//
// 与「渲染」的分工：
//   本函数回答"发生了什么"（Fact），宿主回答"怎么画"（文本/颜色/布局）。

/// 一条用户必须知道的事实。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fact {
    /// 助手说了这段话（流式增量已合并为完整消息）。
    AssistantSaid(String),
    /// 某次工具调用结束。
    ToolFinished { name: String, exit_code: i32 },
    /// 需要用户审批。
    ApprovalNeeded { detail: String },
    /// 出错。
    Failed(String),
    /// 一轮结束（含用量）。
    TurnFinished { input_tokens: u64, output_tokens: u64 },
    /// 会话已就绪。
    SessionReady { session_id: String },
}

/// 从事件序列抽取用户可见事实。
///
/// 两个刻意的语义决定：
/// 1. **流式增量合并为一条**：`AgentMessageDelta` 是传输细节，用户看到的是
///    一句完整的话。若把每个增量当一条事实，宿主间就无法比较（切片点可能不同）。
/// 2. **未收尾的增量也要落成事实**：一轮被中断时可能只收到 delta 没有 done，
///    此时若丢弃，用户在界面上看过的文字就从"事实"里消失了。
pub fn facts_of(events: &[EventMsg]) -> Vec<Fact> {
    let mut out = Vec::new();
    let mut pending_text = String::new();
    // ToolCallEnd 只带 id，名字来自对应的 ToolCallBegin —— 需逐个关联。
    let mut tool_names: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();

    for e in events {
        match e {
            EventMsg::AgentMessageDelta { delta } => pending_text.push_str(delta),
            EventMsg::AgentMessageDone { text } => {
                // 以 Done 的完整文本为准（权威），清掉增量累积
                pending_text.clear();
                if !text.is_empty() {
                    out.push(Fact::AssistantSaid(text.clone()));
                }
            }
            EventMsg::ToolCallBegin { id, name } => {
                tool_names.insert(id.as_str(), name.as_str());
            }
            EventMsg::ToolCallEnd { id, exit_code } => {
                let name = tool_names
                    .get(id.as_str())
                    .map(|n| n.to_string())
                    // 没有 begin 的 end（截断的事件流）也要成事实，名字诚实留空
                    .unwrap_or_default();
                out.push(Fact::ToolFinished { name, exit_code: *exit_code })
            }
            EventMsg::ApprovalRequest { detail, .. } => {
                out.push(Fact::ApprovalNeeded { detail: detail.clone() })
            }
            EventMsg::Error { message } => out.push(Fact::Failed(message.clone())),
            EventMsg::TurnComplete { input_tokens, output_tokens } => {
                out.push(Fact::TurnFinished {
                    input_tokens: *input_tokens,
                    output_tokens: *output_tokens,
                })
            }
            EventMsg::SessionConfigured { session_id } => {
                out.push(Fact::SessionReady { session_id: session_id.clone() })
            }
            // 其余事件不承载"必须知道"的事实：
            // TurnStarted/ToolCallBegin 是过程提示；ReasoningDelta 是思考过程；
            // PatchProposed/CheckpointSaved/GoalProgress/ShutdownComplete 属状态推进。
            // 若要晋升为事实，在此显式添加（并让 T6 覆盖）。
            _ => {}
        }
    }

    // 收尾：中断导致只有增量没有 Done 时，也要把用户看过的文字落成事实
    if !pending_text.is_empty() {
        out.push(Fact::AssistantSaid(pending_text));
    }
    out
}
