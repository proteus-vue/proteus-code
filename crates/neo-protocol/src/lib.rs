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
    /// 行范围（1 基，闭区间）。`None` = 整个文件。
    ///
    /// 单独建模而不是塞进 `target` 字符串：`src/a.rs#12-40` 里的
    /// "#12-40" 是**结构化信息**（起止行），下游（提示词拼装、UI 高亮）
    /// 需要数值而不是再解析一遍字符串。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines: Option<(usize, usize)>,
}

impl ContextRef {
    /// 带行范围的引用是否合法：起止都 >= 1 且 start <= end。
    pub fn valid_range(&self) -> bool {
        match self.lines {
            None => true,
            Some((a, b)) => a >= 1 && a <= b,
        }
    }
}

/// 解析 `path#12-40` / `path#12` 形式的文件引用，返回 (路径, 可选行范围)。
///
/// 只认**末尾**的 `#数字[-数字]`：路径本身可能含 `#`（少见于源码，
/// 但存在），从末尾解析能避免把它误当行号。
pub fn parse_file_ref(raw: &str) -> (String, Option<(usize, usize)>) {
    let Some(hash) = raw.rfind('#') else {
        return (raw.to_string(), None);
    };
    let (path, tail) = (&raw[..hash], &raw[hash + 1..]);
    let spec = tail.trim();
    let parsed = if let Some((a, b)) = spec.split_once('-') {
        match (a.trim().parse::<usize>(), b.trim().parse::<usize>()) {
            (Ok(a), Ok(b)) if a >= 1 && a <= b => Some((a, b)),
            _ => None,
        }
    } else {
        match spec.parse::<usize>() {
            Ok(a) if a >= 1 => Some((a, a)),
            _ => None,
        }
    };
    match parsed {
        Some(r) => (path.to_string(), Some(r)),
        None => (raw.to_string(), None), // `#` 不是行号规格 → 整体当路径
    }
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
    /// 用户直接执行一条 shell 命令（TUI 的 `!cmd`，opencode 同款）。
    ///
    /// 与 UserTurn 的区别：**不经过模型**。命令仍走沙箱（结构性约束），
    /// 结果作为 ToolResult 进入历史，供下一轮模型参考。
    /// 由用户显式输入的命令不再问审批 —— 等价于用户自己在 shell 里敲它。
    Shell { command: String },
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

/// 任务清单的一项（模型通过 `todowrite` 工具维护）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoEntry {
    pub content: String,
    pub status: TodoStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
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
    /// 用户提交的消息（内核回显）。
    ///
    /// 为什么内核要发它：用户**自己的话**是转录里必须可见的事实 —— 否则
    /// 回看会话只看到模型说了什么，不知道当时问的是什么。由内核统一发，
    /// 四个宿主才会一致（T6）；让宿主各自记住用户输入则会分叉。
    UserSubmitted { text: String },
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
    /// 任务清单被更新（整表替换，不是增量）。
    ///
    /// 整表替换而非增量：模型每次给出完整清单，宿主不必维护差量状态，
    /// 也就不会出现"某一步的增量丢了导致清单错乱"。
    TodoUpdated { items: Vec<TodoEntry> },
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
    /// 用户说了什么（转录的第一类事实）
    UserSaid(String),
    /// 助手说了这段话（流式增量已合并为完整消息）。
    AssistantSaid(String),
    /// 某次工具调用结束。
    ToolFinished { name: String, exit_code: i32 },
    /// 需要用户审批。
    ApprovalNeeded { detail: String },
    /// 任务清单（模型自述的进度）。
    TodoList(Vec<TodoEntry>),
    /// 待审批改动的预览（路径 + unified diff）。
    ///
    /// 与 `ApprovalNeeded` 分开建模：一个是"需要你决定"，
    /// 一个是"你要决定的内容是什么"。合并会让没有 diff 的审批
    /// （如网络类）无法表达。
    PatchPreview { path: String, diff: String },
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
            EventMsg::PatchProposed { path, diff } => {
                out.push(Fact::PatchPreview { path: path.clone(), diff: diff.clone() })
            }
            EventMsg::ApprovalRequest { detail, .. } => {
                out.push(Fact::ApprovalNeeded { detail: detail.clone() })
            }
            EventMsg::UserSubmitted { text } => out.push(Fact::UserSaid(text.clone())),
            EventMsg::TodoUpdated { items } => out.push(Fact::TodoList(items.clone())),
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

/// 解析输入里的上下文引用：`@file` / `#session` / `/command` / `$skill`。
///
/// **为什么放在协议层而不是各宿主**：引用符号是界面约定，但解析结果
/// `ContextRef`（"这是一个文件引用"）是协议语义。放在 L0 后，TUI / Desktop / Web
/// 共用同一份解析，不会各自漂移出"某个宿主不识别 `$`"这类分叉。
///
/// 规则：符号后必须有非空目标（单独的 `@` 不算引用）；目标按空白切分，
/// 因此路径带空格需由上层改用引号语法（当前版本不支持）。
pub fn parse_refs(input: &str) -> Vec<ContextRef> {
    let mut out = Vec::new();
    for token in input.split_whitespace() {
        let Some(first) = token.chars().next() else { continue };
        let kind = match first {
            '@' => RefKind::File,
            '#' => RefKind::Session,
            '/' => RefKind::Command,
            '$' => RefKind::Skill,
            _ => continue,
        };
        let target = &token[first.len_utf8()..];
        if target.is_empty() {
            continue;
        }
        // 只有文件引用支持 `#行号`；会话/命令/技能名里的 `#` 是名字的一部分
        if kind == RefKind::File {
            let (path, lines) = parse_file_ref(target);
            let r = ContextRef { kind, target: path, lines };
            if r.valid_range() {
                out.push(r);
            }
        } else {
            out.push(ContextRef { kind, target: target.to_string(), lines: None });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_refs_maps_all_four_sigils() {
        let refs = parse_refs("看下 @src/main.rs 和 #session-1 用 /compact 与 $skill-x");
        assert_eq!(
            refs,
            vec![
                ContextRef { kind: RefKind::File, target: "src/main.rs".into(), lines: None },
                ContextRef { kind: RefKind::Session, target: "session-1".into(), lines: None },
                ContextRef { kind: RefKind::Command, target: "compact".into(), lines: None },
                ContextRef { kind: RefKind::Skill, target: "skill-x".into(), lines: None },
            ]
        );
    }

    #[test]
    fn parse_refs_ignores_lone_sigils_and_plain_words() {
        assert!(parse_refs("hello @ world").is_empty(), "单独的 @ 不算引用");
        assert!(parse_refs("plain text").is_empty());
    }

    #[test]
    fn parses_line_ranges_out_of_file_refs() {
        let (p, l) = parse_file_ref("src/a.rs#12-40");
        assert_eq!(p, "src/a.rs");
        assert_eq!(l, Some((12, 40)));
        let (p, l) = parse_file_ref("src/a.rs#7");
        assert_eq!(p, "src/a.rs");
        assert_eq!(l, Some((7, 7)), "单行应成为 [7,7] 闭区间");
    }

    #[test]
    fn non_range_hash_stays_part_of_the_path() {
        // `#` 后面不是合法行号规格时，整体当路径 —— 不能把文件名切坏
        for raw in ["src/a#b.rs", "weird#name", "a#0", "a#5-2", "a#x-y"] {
            let (p, l) = parse_file_ref(raw);
            assert_eq!(p, raw, "{raw} 应整体作为路径");
            assert_eq!(l, None, "{raw} 不应解析出行范围");
        }
    }

    #[test]
    fn refs_carry_their_line_range() {
        let refs = parse_refs("看下 @src/main.rs#10-20 和 @README.md");
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].lines, Some((10, 20)));
        assert_eq!(refs[0].target, "src/main.rs");
        assert_eq!(refs[1].lines, None);
    }

    #[test]
    fn parse_refs_handles_multibyte_after_sigil() {
        // 首字符是 ASCII 符号，但目标含多字节：不能用 len() 而非 len_utf8() 切片
        let refs = parse_refs("@文件.txt");
        assert_eq!(refs[0].target, "文件.txt");
    }
}
