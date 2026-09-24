//! L1 PROTOCOL —— 线协议层
//!
//! 铁律：本 crate 只依赖 serde。不得引入任何业务 crate。
//! 理由：这是改动最频繁的一层，必须编译最快；也是宿主与引擎的唯一契约。

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

// ---------- 上下文引用（吸收 ZCode 的 @ / # / / / $ 体系） ----------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema, ts_rs::TS))]
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

/// 把一条文件引用**格式化成可写进输入框的文本** —— [`parse_file_ref`] 的逆。
///
/// # 为什么必须有它（此前只有解析、没有格式化）
///
/// 解析早就有了（`parse_refs`），但反向一直是空的。于是"给输入框插入一条文件引用"
/// 这件事没有任何正确做法：调用方只能自己拼 `format!("@{path}")` —— 而**路径含
/// 空格时这就错了**（`@my file.txt` 会被解析成 `@my` 加一个普通词 `file.txt`）。
/// 文件树做"点一下把文件加进上下文"时正是这个场景，所以这一轮把它补上。
///
/// # 引号规则（与 `ref_tokens` 的解析严格对偶）
///
/// | 情形 | 输出 |
/// |---|---|
/// | 路径无空白、不以引号开头 | `@src/main.rs`（直接写，可读性最好） |
/// | 含空白 | `@"my file.txt"` |
/// | 含 `"` | `@'a"b.txt'`（换单引号，免得转义） |
/// | 含 `'` 也含 `"` | `@"a\"b'c.txt"`（用双引号 + 转义 `\"`） |
/// | 含换行 | `None` —— **无法安全表示**，不猜 |
///
/// 含行范围时拼成 `@"a b.rs"#12-40`（行号跟在右引号后，与解析一致）。
///
/// 返回 `None` 表示"这条路径无法被安全表示"，调用方应**如实告知**而不是
/// 硬拼一个会解析错的字符串（后者会在下次解析时静默丢引用）。
pub fn format_file_ref(path: &str, lines: Option<(usize, usize)>) -> Option<String> {
    if path.is_empty() {
        return None;
    }
    // 换行/回车无法在单条引用里表示：解析器把空白当分隔符，引号体内虽可含换行，
    // 但那样一条引用会跨两行 —— 在输入框里既看不出也容易误删。不猜，直接拒绝。
    if path.contains('\n') || path.contains('\r') {
        return None;
    }
    let needs_quote = path.chars().any(char::is_whitespace)
        || path.starts_with('"')
        || path.starts_with('\'');
    let body = if !needs_quote {
        path.to_string()
    } else if !path.contains('"') {
        // 无 `"` → 用双引号（最常见的含空格情形）
        format!("\"{path}\"")
    } else if !path.contains('\'') {
        // 含 `"` 但不含 `'` → 用单引号，免转义
        format!("'{path}'")
    } else {
        // 两种引号都有 → 双引号 + 把 `"` 转义（解析器认 `\"`）
        format!("\"{}\"", path.replace('"', "\\\""))
    };
    let range = match lines {
        Some((a, b)) if a >= 1 && a <= b => {
            if a == b {
                format!("#{a}")
            } else {
                format!("#{a}-{b}")
            }
        }
        // 非法行范围（0 起、逆序）当作没有范围 —— 与 `valid_range` 的判定一致，
        // 不产出解析回来会被丢弃的东西。
        _ => String::new(),
    };
    Some(format!("@{body}{range}"))
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum RefKind {
    File,    // @path
    Session, // #session
    Command, // /command
    Skill,   // $skill
}

// ---------- 宿主 -> 引擎 ----------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum Op {
    UserTurn { text: String, refs: Vec<ContextRef> },
    /// 开始一轮但**不驱动**：只做回显/引用解析/推入历史。配合 [`Op::Pump`]
    /// 逐帧推进用。
    ///
    /// # 为什么需要它（而不是只用 UserTurn）
    ///
    /// `UserTurn` 在一次 `submit` 里跑完整轮（可能多次模型往返 + 工具执行），
    /// 宿主只能等它全部结束才拿到事件 —— 期间界面**完全冻结**（真实反馈：
    /// "回车后像卡死，退出后才显示一堆内容"）。逐帧推进让宿主在每步之后
    /// 重绘一次，至少能看到"正在请求模型 / 正在执行工具"。
    BeginTurn { text: String, refs: Vec<ContextRef> },
    /// 推进一轮里的**一步**（一次模型请求 + 它要求的工具执行）。
    /// 收到 `TurnComplete` 或 `ApprovalRequest` 即到边界。
    Pump,
    /// 用户直接执行一条 shell 命令（TUI 的 `!cmd`，opencode 同款）。
    ///
    /// 与 UserTurn 的区别：**不经过模型**。命令仍走沙箱（结构性约束），
    /// 结果作为 ToolResult 进入历史，供下一轮模型参考。
    /// 由用户显式输入的命令不再问审批 —— 等价于用户自己在 shell 里敲它。
    Shell { command: String },
    Interrupt,
    /// 落实一次审批。
    ///
    /// `reason` 仅在 `decision = Deny` 时有意义：用户拒绝时**为什么拒**。
    /// 它会进入模型可见的工具结果（"用户拒绝了该调用：<理由>"）——
    /// 模型因此知道该换个做法，而不是原样重试（真机实测过这个差别）。
    /// 没有理由输入的宿主（exec / Web）传 `None`，行为与之前完全一致。
    Approve { id: ApprovalId, decision: Decision, #[serde(default)] reason: Option<String> },
    /// 与 `Approve` 相同，但**只执行本步剩余的调用**，不驱动后续步骤。
    ///
    /// 供逐帧宿主（TUI）用：`Approve` 会在一次调用里把整轮剩下的
    /// 模型往返全部跑完 —— 那里可能又有多次网络请求，界面再次冻结
    /// （用户曾因此在冻结期间敲键，解冻后那些键被逐个处理，误触退出）。
    /// 逐帧宿主用这个变体，然后自己 `Pump` 逐步推进。
    ApproveStep { id: ApprovalId, decision: Decision, #[serde(default)] reason: Option<String> },
    /// 应答一次 `request_user_input` 挂起（对齐 Codex `item/tool/requestUserInput`）。
    ///
    /// `response` 是用户原话，作为该工具调用的 stdout 进入模型可见历史。
    RespondUserInput { id: ApprovalId, response: String },
    ConfigureSession { patch: SessionPatch },
    Compact,
    Fork,
    /// 回退对话到之前某轮：删掉最近 `turns` 个用户轮次的全部消息。
    ///
    /// **只回退对话，不还原文件** —— 见 `EventMsg::Rewound` 的说明。
    /// 用途：某个方向做错了想重新问一次，而不必开新会话丢掉前面有用的上下文。
    Rewind { turns: usize },
    GoalSet { goal: String },
    GoalPause { goal_id: GoalId },
    GoalResume { goal_id: GoalId },
    /// 推进目标：执行**一个**子任务轮（一次完整 turn），跑完自动推进引擎。
    /// 与 `Pump` 同理是有界的 —— 一次只跑一轮，宿主控制节奏；
    /// 是否还有下一轮看最新 `GoalUpdated` 快照的 `turns_remaining`。
    GoalAdvance,
    /// 清除目标（目标与其进度一并丢弃，会话保留）。
    GoalClear,
    /// 运行中插入用户指令（Codex `turn/steer`）。
    ///
    /// 与 `UserTurn` 的区别：**不新开轮**——把文本推入当前轮历史并继续
    /// `drive_steps`。流式在飞时拒绝（等边界或先 Interrupt）；轮已结束时
    /// 拒绝（应走 `turn/start`）。
    Steer { text: String },
    Shutdown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum Decision {
    Allow,
    AllowAlways,
    Deny,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct SessionPatch {
    pub exec_mode: Option<ExecMode>,
    pub sandbox_mode: Option<SandboxMode>,
    pub approval_policy: Option<ApprovalPolicy>,
    pub model: Option<String>,
    /// 会话级 token 预算（护栏 #9）。`Some(0)` = 清除预算（不限）。
    ///
    /// 缺省 `None` = **不改**现有预算（与其它字段同语义）。
    /// 要"不限"必须显式传 `0`，不能靠缺省——否则桌面端漏传会静默抹掉用户设过的预算。
    #[serde(default)]
    pub token_budget: Option<u64>,
}

/// ZCode 五档执行模式（UI 档位，映射到沙箱 × 审批双轴）
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema, ts_rs::TS))]
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
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum SandboxMode {
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

/// 轴一：审批（流程边界，何时必须暂停问人）
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum ApprovalPolicy {
    Untrusted,
    OnRequest,
    OnFailure,
    Never,
}

/// 一个文件的改动统计（供侧栏"已修改文件"面板）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct FileChange {
    pub path: String,
    pub additions: usize,
    pub deletions: usize,
}

/// 任务清单的一项（模型通过 `todowrite` 工具维护）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct TodoEntry {
    pub content: String,
    pub status: TodoStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

// ── 目标编排（Goal）────────────────────────────────────────────────

/// 目标子任务所处的阶段（Plan→Code→Review→Learn 的四阶段闭环 + 完成）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum GoalPhase {
    Plan,
    Code,
    Review,
    Learn,
    Done,
}

/// 一个子任务在快照里的形态。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct GoalSubtask {
    pub id: usize,
    pub title: String,
    pub phase: GoalPhase,
    pub retries: u32,
}

/// 目标编排状态的**完整快照**。
///
/// 每次状态变化都发整份快照而不是增量：回放重建 = 取最后一条即可，
/// 宿主展示 = 直接读，不需要自己累计差量 —— 状态单一事实源在编排器，
/// 快照即它当时的全部。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct GoalSnapshot {
    /// 确定性编号（`goal-N`，按编排器内计数器派生，不用随机/时钟）
    pub goal_id: GoalId,
    /// 用户设定的目标原文
    pub goal: String,
    /// 暂停中（不推进，但目标保留）
    pub paused: bool,
    /// 引擎触发停止条件后的原因（非空时不再推进）
    pub stopped: Option<String>,
    pub subtasks: Vec<GoalSubtask>,
    /// 已消耗的编排步数
    pub iterations: usize,
    /// 连续失败计数（审查判停的依据 —— 回放重建后判停行为必须一致，
    /// 所以它必须在快照里，而不能只活在内存里）
    pub consecutive_failures: u32,
    /// 还需要多少个子任务轮（**下限估计**：审查失败的重试会让实际更多）。
    /// 宿主据此决定要不要继续 `GoalAdvance`。
    pub turns_remaining: usize,
    /// 目标已消耗的 token 预算
    pub budget_used: u64,
}

impl GoalSnapshot {
    /// 已完成子任务数。
    pub fn done_count(&self) -> usize {
        self.subtasks.iter().filter(|s| s.phase == GoalPhase::Done).count()
    }

    /// 用户可见的单行摘要（宿主渲染的单一事实源 —— 各宿主拼各的
    /// 会漂移出"同一个目标在 TUI 和 exec 长两副样子"）。
    pub fn summary(&self) -> String {
        let current = self.subtasks.iter().find(|s| s.phase != GoalPhase::Done);
        let cur = match (self.stopped.as_deref(), current) {
            (Some(reason), _) => format!("已停止：{reason}"),
            (None, Some(s)) => format!("当前：{}（{}）", s.title, phase_name(s.phase)),
            (None, None) => "全部完成".to_string(),
        };
        let paused = if self.paused { " · 已暂停" } else { "" };
        format!(
            "🎯 {}: {}/{} 完成 · {}{}",
            self.goal_id,
            self.done_count(),
            self.subtasks.len(),
            cur,
            paused
        )
    }
}

/// 阶段的中文名（摘要用）。
fn phase_name(p: GoalPhase) -> &'static str {
    match p {
        GoalPhase::Plan => "计划",
        GoalPhase::Code => "执行",
        GoalPhase::Review => "审查",
        GoalPhase::Learn => "复盘",
        GoalPhase::Done => "完成",
    }
}

pub type ApprovalId = String;
pub type GoalId = String;
pub type SubmissionId = u64;
pub type ToolCallId = String;
pub type Seq = u64;

// ---------- 引擎 -> 宿主 ----------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum EventMsg {
    /// 用户提交的消息（内核回显）。
    ///
    /// 为什么内核要发它：用户**自己的话**是转录里必须可见的事实 —— 否则
    /// 回看会话只看到模型说了什么，不知道当时问的是什么。由内核统一发，
    /// 四个宿主才会一致（T6）；让宿主各自记住用户输入则会分叉。
    UserSubmitted { text: String },
    /// 用户输入里的 `@` / `$` 引用已解析成具体内容并注入本轮请求。
    ///
    /// # 为什么必须发事件、且 `block` 必须落日志
    ///
    /// `block` 是**模型可见**的内容（拼在用户消息之后进入请求）。若不落盘，
    /// 回放时只能拿到引用符号（`@src/main.rs`），重建出的历史与真实请求不符 ——
    /// 直接违反 AGENTS.md「模型可见即已落日志」。
    ///
    /// 反过来，回放时**不能重新解析**：文件内容可能已经变了，
    /// 重解析会得到与当时不同的字节。所以内容必须"当时定格、随日志走"。
    RefsResolved {
        /// 人类可读的一行摘要（用户可见事实）：解析成功了几条、哪条失败。
        summary: Vec<String>,
        /// 注入模型请求的完整上下文块。
        #[serde(default)]
        block: String,
    },
    SessionConfigured { session_id: String },
    /// 项目指令（`AGENTS.md` 级联）已并入系统提示词。
    ///
    /// # 为什么系统提示词的组成也要落日志
    ///
    /// 系统提示词**每一轮都进模型请求**，属于最典型的"模型可见"内容。
    /// 只落会话消息而不落它，审计时无法回答"模型当时到底被要求遵守什么约定" ——
    /// 而这恰恰是解释模型行为的关键（它按 AGENTS.md 做了某件事，日志里却看不到那份文件）。
    ///
    /// `block` 是注入提示词的实际文本；`sources` 是它来自哪些文件。
    /// `truncated` 如实标注是否被 32 KiB 上限截断。
    InstructionsLoaded {
        sources: Vec<String>,
        block: String,
        #[serde(default)]
        truncated: bool,
    },
    /// 模型已切换（运行时换模型）。
    ///
    /// 单独发事件而不是只发 SessionConfigured：模型切换是用户**会关心**的
    /// 状态变化（直接影响后续回答的风格与成本），而 SessionConfigured 是
    /// 一次笼统的"配置更新"。
    ModelSwitched { model: String, context_limit: u64 },
    TurnStarted { turn_id: String },
    AgentMessageDelta { delta: String },
    AgentMessageDone { text: String },
    ReasoningDelta { delta: String },
    ToolCallBegin {
        id: ToolCallId,
        name: String,
        /// 调用参数。**必须落日志** —— 模型下次请求要带完整的 assistant
        /// tool_call（含 arguments），否则会话无法从日志重建、
        /// 跨进程续聊时请求非法。这也正是 AGENTS.md「模型可见即已落日志」
        /// 那条约束的要求。
        #[serde(default)]
        #[cfg_attr(feature = "schema", ts(type = "unknown"))]
        arguments: serde_json::Value,
    },
    ToolCallEnd {
        id: ToolCallId,
        exit_code: i32,
        /// 工具输出。**必须进事件流** —— 否则用户只看到 `✓ bash exit 0`
        /// 而看不到命令打印了什么，等于无法判断这步到底做了什么。
        /// 内核已按上限截断，`truncated` 如实标注。
        #[serde(default)]
        stdout: String,
        #[serde(default)]
        stderr: String,
        #[serde(default)]
        truncated: bool,
    },
    /// 需要用户审批。
    ///
    /// `kind` 是内核判定的**调用类别**（read / write / network / interactive /
    /// **loop**），不是工具名 —— "总是允许"将放行的范围由它定义。
    /// `loop` = doom-loop 循环检测闸门（护栏 #6）：**不参与类别 granted**，
    /// 单次放行不得被映射成「总是允许 read/write」。
    /// 宿主若按工具名自行推断，bash 这类"按命令内容分类"的工具就会
    /// 显示成与内核实际放行范围不一致的类别（显示"只读"、实际放行"写入"）。
    ApprovalRequest { id: ApprovalId, detail: String, #[serde(default)] kind: String },
    /// 内核请求宿主向用户提问（`request_user_input` / Codex ServerRequest 形态）。
    ///
    /// 宿主用 `Op::RespondUserInput` 以**同一 id** 回写答复。
    /// 与 `ApprovalRequest` 分开：审批是 Allow/Deny，提问要的是自由文本。
    UserInputRequest { id: ApprovalId, prompt: String },
    /// 一次图片附件已进入模型上下文（宿主可按 path 预览；**不含 base64**）。
    ///
    /// 与 `ToolCallEnd` 分开：End 是工具收尾事实，本事件专供「有一张图
    /// 要展示/回放」——载荷保持小，JSONL 不被像素撑爆。
    ImageAttached {
        id: ToolCallId,
        path: String,
        mime: String,
        bytes: u64,
    },
    PatchProposed { path: String, diff: String },
    CheckpointSaved { checkpoint_id: String },
    /// 单个文件发生改动（**由工具上报**，内核据此累计）。
    FileChanged { path: String, additions: usize, deletions: usize },
    /// 已修改文件的**完整聚合列表**（内核广播，侧栏据此渲染）。
    ///
    /// 为什么发聚合而不是只发增量：侧栏需要的是"当前全部改动"，
    /// 让每个宿主自己维护增量状态，一旦某条增量丢了就会显示错乱。
    /// 内核已经持有累计状态，广播整表最省事也最不容易错。
    FilesChanged { files: Vec<FileChange> },
    /// 上下文已压缩：前缀 `removed_messages` 条被替换为一条 `summary`。
    ///
    /// 摘要本身必须进事件流 —— 它是**模型可见**的内容（后续请求会带上），
    /// 不落日志会让回放出的历史与真实请求不符。
    ContextCompacted { removed_messages: usize, summary: String },
    /// 对话已回退。
    ///
    /// **明确区分"对话"与"文件"**：回退只动对话历史，磁盘上的改动**不会**
    /// 被撤销（我们没有 git 快照）。宿主必须把这一点告诉用户 ——
    /// 让人以为"undo 了文件也回去了"是最危险的那种错觉。
    Rewound { turns: usize, removed_messages: usize, files_kept: usize },
    /// 任务清单被更新（整表替换，不是增量）。
    ///
    /// 整表替换而非增量：模型每次给出完整清单，宿主不必维护差量状态，
    /// 也就不会出现"某一步的增量丢了导致清单错乱"。
    TodoUpdated { items: Vec<TodoEntry> },
    GoalProgress { goal_id: GoalId, done: usize, total: usize },
    /// 目标编排状态变化（设定 / 阶段推进 / 暂停 / 恢复 / 停止）。
    ///
    /// 快照是**完整**状态：回放重建取最后一条即可（kill 进程后续跑的核心）。
    /// 目标文本会经子任务轮进入模型上下文 —— 模型可见，故必须落日志。
    GoalUpdated { snapshot: GoalSnapshot },
    /// 目标已被清除（清除不是快照 —— 没有目标就没有快照可发；
    /// 宿主与回放以此显式撤销Goal 显示与状态）。
    GoalCleared { goal_id: GoalId },
    Error { message: String },
    TurnComplete { input_tokens: u64, output_tokens: u64 },
    /// `fs/watch` 订阅的路径发生变化（Codex `fs/changed` 通知）。
    ///
    /// 由宿主侧监听器产生，**不经内核状态机** —— 与 `ApprovalRequest` 不同，
    /// 它不改变会话真值，只是"磁盘上动了"的提示；`facts_of` 刻意忽略它
    ///（过程提示，不是必须记住的事实）。
    FsChanged {
        watch_id: String,
        /// 变化涉及的路径（截断至前 32 条，防止风暴撑爆事件行）。
        paths: Vec<String>,
        /// 合并后的一次提示（true）或逐条（false）—— 由监听器有界通道决定。
        #[serde(default)]
        coalesced: bool,
    },
    ShutdownComplete,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct ToolOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
}

/// 一张已读入内存的图片（`view_image` → 模型上下文）。
///
/// `data_base64` **不进事件流/会话日志**（会把 JSONL 撑爆）：日志只记
/// [`EventMsg::ImageAttached`] 的路径元数据；像素在内存 `Message` 里，
/// provider 编码时再发给模型。跨进程回放时若文件仍在则重读，否则降级为文字说明。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct ImageAttachment {
    /// `image/png` 等
    pub mime: String,
    /// 标准 base64（无 data: 前缀）
    pub data_base64: String,
    /// 工作区内绝对路径（回放重读用）
    pub path: String,
    /// Codex 同名：`high` / `original` / 缺省
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// 源文件字节数（未编码前；有界性审计用）
    pub bytes: u64,
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
///
/// **可序列化**：app-server 的 `thread/history` 直接把它推上线，
/// 线格式 = T6 比较对象，没有第二套「投影类型」。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum Fact {
    /// 用户说了什么（转录的第一类事实）
    UserSaid(String),
    /// 用户引用（`@file` / `$skill`）被解析成内容的**一行摘要**。
    ///
    /// 只把摘要纳入事实、不把注入内容纳入：用户需要知道"引用的文件到底读到了没有、
    /// 哪条失败"，但不需要在转录里看到几百行文件正文（那是模型上下文，
    /// 不是人要读的对话）。内容本身仍随事件落盘，回放与审计不受影响。
    RefsResolved(Vec<String>),
    /// 助手说了这段话（流式增量已合并为完整消息）。
    AssistantSaid(String),
    /// 模型的推理过程（`ReasoningDelta` 合并而成）。
    ///
    /// 与 `AssistantSaid` **分开**：推理是"模型怎么想的"，答复是"模型说了什么"。
    /// 混在一起会让宿主无法独立控制显隐（`/thinking` 就做不到）。
    AssistantThought(String),
    /// 某次工具调用结束（含输出）。
    ///
    /// 输出是**用户必须知道的**：没有它，工具调用只是一行"成功"，
    /// 用户无法判断这次执行到底做了什么。
    ToolFinished {
        name: String,
        exit_code: i32,
        stdout: String,
        stderr: String,
        truncated: bool,
        /// 调用参数原文（来自 ToolCallBegin.arguments 的 JSON 文本）。
        /// 宿主据此在工具行上显示"执行了什么"（如 bash 的命令），
        /// 不用展开详情。派生视图，非事件流内容。
        args: Option<String>,
    },
    /// 需要用户审批。
    ApprovalNeeded { detail: String },
    /// 任务清单（模型自述的进度）。
    TodoList(Vec<TodoEntry>),
    /// 上下文已压缩。
    ContextCompacted { removed_messages: usize },
    /// 对话已回退（回退了 N 轮，删掉 M 条消息，有 K 个文件改动被保留）。
    Rewound { turns: usize, removed_messages: usize, files_kept: usize },
    /// 本次会话已修改的文件（含增删行数）。
    FilesChanged(Vec<FileChange>),
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
    /// 项目指令已加载（`AGENTS.md` 级联）。
    ///
    /// 纳入事实的理由：用户应当知道"这个会话受哪些约定约束"，
    /// 尤其是**截断**时 —— 否则会以为整套约定都在生效。
    InstructionsLoaded { sources: Vec<String>, truncated: bool },
    /// 模型已切换（含新模型的上下文窗口，0 = 未知）。
    ModelSwitched { model: String, context_limit: u64 },
    /// 目标编排状态（单行摘要，由 `GoalSnapshot::summary` 生成 ——
    /// 摘要在协议层拼好，宿主直接展示，不各自拼一套）。
    Goal(String),
    /// 目标已清除（宿主应撤掉目标显示，而不是读最后一条快照 ——
    /// 快照是历史，清除是现在时）。
    GoalCleared(String),
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
    let mut pending_thought = String::new();
    // ToolCallEnd 只带 id，名字来自对应的 ToolCallBegin —— 需逐个关联。
    let mut tool_names: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    // Begin 的调用参数（JSON 文本）：End 只带 id，参数靠这里关联
    let mut tool_args: std::collections::HashMap<&str, String> = std::collections::HashMap::new();

    for e in events {
        match e {
            EventMsg::AgentMessageDelta { delta } => pending_text.push_str(delta),
            EventMsg::ReasoningDelta { delta } => {
                // 与文本增量同构：先累积，Done 时（或流结束时）落成一条事实
                pending_thought.push_str(delta);
            }
            EventMsg::AgentMessageDone { text } => {
                // 以 Done 的完整文本为准（权威），清掉增量累积
                pending_text.clear();
                // 推理在答复落定时一并交出（顺序：先想后说）
                if !pending_thought.is_empty() {
                    out.push(Fact::AssistantThought(std::mem::take(&mut pending_thought)));
                }
                if !text.is_empty() {
                    out.push(Fact::AssistantSaid(text.clone()));
                }
            }
            EventMsg::ToolCallBegin { id, name, arguments, .. } => {
                tool_names.insert(id.as_str(), name.as_str());
                // 无参调用（arguments 为 null）不记摘要，否则渲染成 "null"
                if !arguments.is_null() {
                    tool_args.insert(id.as_str(), arguments.to_string());
                }
            }
            EventMsg::ToolCallEnd { id, exit_code, stdout, stderr, truncated } => {
                let name = tool_names
                    .get(id.as_str())
                    .map(|n| n.to_string())
                    // 没有 begin 的 end（截断的事件流）也要成事实，名字诚实留空
                    .unwrap_or_default();
                out.push(Fact::ToolFinished {
                    name,
                    exit_code: *exit_code,
                    stdout: stdout.clone(),
                    stderr: stderr.clone(),
                    truncated: *truncated,
                    args: tool_args.get(id.as_str()).cloned(),
                })
            }
            EventMsg::PatchProposed { path, diff } => {
                out.push(Fact::PatchPreview { path: path.clone(), diff: diff.clone() })
            }
            EventMsg::ApprovalRequest { detail, .. } => {
                out.push(Fact::ApprovalNeeded { detail: detail.clone() })
            }
            // 提问也是「需要用户介入」的事实；复用 ApprovalNeeded 避免扩 Fact 枚举
            // 波及全部宿主 match（detail 前缀区分，history 投影仍可读）。
            EventMsg::UserInputRequest { prompt, .. } => {
                out.push(Fact::ApprovalNeeded { detail: format!("需要用户输入：{prompt}") })
            }
            EventMsg::ImageAttached { path, mime, .. } => {
                out.push(Fact::ApprovalNeeded {
                    detail: format!("已附加图片 {path}（{mime}）"),
                })
            }
            EventMsg::UserSubmitted { text } => out.push(Fact::UserSaid(text.clone())),
            EventMsg::RefsResolved { summary, .. } => {
                if !summary.is_empty() {
                    out.push(Fact::RefsResolved(summary.clone()))
                }
            }
            EventMsg::TodoUpdated { items } => out.push(Fact::TodoList(items.clone())),
            EventMsg::ContextCompacted { removed_messages, .. } => {
                out.push(Fact::ContextCompacted { removed_messages: *removed_messages })
            }
            EventMsg::Rewound { turns, removed_messages, files_kept } => out.push(Fact::Rewound {
                turns: *turns,
                removed_messages: *removed_messages,
                files_kept: *files_kept,
            }),
            EventMsg::FilesChanged { files } => out.push(Fact::FilesChanged(files.clone())),
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
            EventMsg::InstructionsLoaded { sources, truncated, .. } => {
                if !sources.is_empty() {
                    out.push(Fact::InstructionsLoaded {
                        sources: sources.clone(),
                        truncated: *truncated,
                    })
                }
            }
            EventMsg::ModelSwitched { model, context_limit } => {
                out.push(Fact::ModelSwitched {
                    model: model.clone(),
                    context_limit: *context_limit,
                })
            }
            EventMsg::GoalUpdated { snapshot } => out.push(Fact::Goal(snapshot.summary())),
            EventMsg::GoalCleared { goal_id } => out.push(Fact::GoalCleared(goal_id.clone())),
            // 其余事件不承载"必须知道"的事实：
            // TurnStarted/ToolCallBegin 是过程提示；ReasoningDelta 是思考过程；
            // PatchProposed/CheckpointSaved/GoalProgress/ShutdownComplete 属状态推进。
            // 若要晋升为事实，在此显式添加（并让 T6 覆盖）。
            _ => {}
        }
    }

    // 收尾：中断导致只有增量没有 Done 时，也要把用户看过的文字落成事实
    if !pending_thought.is_empty() {
        out.push(Fact::AssistantThought(pending_thought));
    }
    if !pending_text.is_empty() {
        out.push(Fact::AssistantSaid(pending_text));
    }
    out
}

// ── 目标事件流的宿主侧判定（两个宿主各写一套必然漂移，收敛到 L0）────

/// 目标是否还有待推进的子任务轮（宿主在 TurnComplete 后据此继续
/// `Op::GoalAdvance`）。倒序找**最近一条**目标状态事件：快照给判断，
/// `GoalCleared` 直接否 —— 清除之后旧快照不再是"当前目标"。
///
/// 有界性由编排器的停止条件保证（停止后 `stopped` 非空，此处返回 false）；
/// 这个函数只读事件流，不推进任何东西。
pub fn goal_awaiting_advance(events: &[EventMsg]) -> bool {
    for e in events.iter().rev() {
        match e {
            EventMsg::GoalUpdated { snapshot } => {
                return !snapshot.paused
                    && snapshot.stopped.is_none()
                    && snapshot.turns_remaining > 0;
            }
            EventMsg::GoalCleared { .. } => return false,
            _ => {}
        }
    }
    false
}

/// 最后一条目标快照的 id（pause / resume 要作用于**当前**目标，
/// 不能从更早的快照里拿旧的）。
pub fn latest_goal_id(events: &[EventMsg]) -> Option<GoalId> {
    for e in events.iter().rev() {
        match e {
            EventMsg::GoalUpdated { snapshot } => return Some(snapshot.goal_id.clone()),
            EventMsg::GoalCleared { .. } => return None,
            _ => {}
        }
    }
    None
}

/// 目标单行状态（最后一次快照的摘要；清除后为 None）。
pub fn latest_goal_line(events: &[EventMsg]) -> Option<String> {
    for e in events.iter().rev() {
        match e {
            EventMsg::GoalUpdated { snapshot } => return Some(snapshot.summary()),
            EventMsg::GoalCleared { .. } => return None,
            _ => {}
        }
    }
    None
}

/// 解析输入里的上下文引用：`@file` / `#session` / `/command` / `$skill`。
///
/// **为什么放在协议层而不是各宿主**：引用符号是界面约定，但解析结果
/// `ContextRef`（"这是一个文件引用"）是协议语义。放在 L0 后，TUI / Desktop / Web
/// 共用同一份解析，不会各自漂移出"某个宿主不识别 `$`"这类分叉。
///
/// 规则：符号后必须有非空目标（单独的 `@` 不算引用）。目标按空白切分；
/// **路径带空格用引号语法** —— `@"my file.txt"`、`$'我的技能'`，
/// 引号体内空白是目标的一部分，`\"` 表示字面引号；行范围紧跟右引号
/// （`@"a b.rs"#12-40`）。空引号体 = 无引用；未闭合的引号整体按
/// 普通词处理（不猜意图）。
pub fn parse_refs(input: &str) -> Vec<ContextRef> {
    let mut out = Vec::new();
    for token in ref_tokens(input) {
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

/// 把输入切成候选引用 token。
///
/// 与 `split_whitespace` 的唯一差别：符号（@ # / $）后紧跟引号时，
/// **引号体（可含空白）整体作为 token 的目标** —— `@"my file.txt"` 的
/// target 是 `my file.txt` 而不是 `my`。其余行为与空白切分完全一致：
/// 不猜意图 —— 未闭合/空引号体按普通词处理（与历史行为兼容）。
fn ref_tokens(input: &str) -> Vec<String> {
    let chars: Vec<char> = input.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        let sigil = matches!(c, '@' | '#' | '/' | '$');
        let start = i;
        i += 1;
        // 引号体只在"符号后紧跟引号"时特殊处理
        if sigil && i < chars.len() && (chars[i] == '"' || chars[i] == '\'') {
            let quote = chars[i];
            i += 1;
            let mut body = String::new();
            let mut closed = false;
            while i < chars.len() {
                let ch = chars[i];
                if ch == '\\' && i + 1 < chars.len() && chars[i + 1] == quote {
                    body.push(quote);
                    i += 2;
                    continue;
                }
                if ch == quote {
                    i += 1;
                    closed = true;
                    break;
                }
                body.push(ch);
                i += 1;
            }
            if closed {
                if body.is_empty() {
                    // 空目标 = 没有引用（与"单独的 @ 不算引用"同一条规则）
                    continue;
                }
                // 行范围紧跟右引号：@"a b.rs"#12-40 —— 与非引号路径的
                // `#行号` 语义一致（是否合法范围由 parse_file_ref 判定）
                if i < chars.len() && chars[i] == '#' {
                    while i < chars.len() && !chars[i].is_whitespace() {
                        body.push(chars[i]);
                        i += 1;
                    }
                }
                out.push(format!("{c}{body}"));
                continue;
            }
            // 未闭合：回退为普通词（从符号起吃到下一个空白，含引号字符
            // —— 与历史行为一致，不猜意图）
            i = start;
            while i < chars.len() && !chars[i].is_whitespace() {
                i += 1;
            }
            out.push(chars[start..i].iter().collect());
            continue;
        }
        while i < chars.len() && !chars[i].is_whitespace() {
            i += 1;
        }
        out.push(chars[start..i].iter().collect());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goal_helpers_find_the_latest_state_and_honour_clear() {
        let snap = |id: &str, paused: bool| GoalSnapshot {
            goal_id: id.into(),
            goal: "g".into(),
            paused,
            stopped: None,
            subtasks: vec![GoalSubtask {
                id: 1,
                title: "t".into(),
                phase: GoalPhase::Code,
                retries: 0,
            }],
            iterations: 1,
            consecutive_failures: 0,
            turns_remaining: 3,
            budget_used: 0,
        };
        let events = vec![
            EventMsg::GoalUpdated { snapshot: snap("goal-1", false) },
            EventMsg::UserSubmitted { text: "x".into() }, // 中间的普通事件不该挡住判定
            EventMsg::GoalUpdated { snapshot: snap("goal-1", true) },
        ];
        assert!(latest_goal_id(&events).as_deref() == Some("goal-1"));
        assert!(!goal_awaiting_advance(&events), "暂停中不该推进");
        assert!(latest_goal_line(&events).unwrap().contains("已暂停"));

        // 清除显式截断：清除之后旧快照不再是当前目标
        let events2 = vec![
            EventMsg::GoalUpdated { snapshot: snap("goal-1", false) },
            EventMsg::GoalCleared { goal_id: "goal-1".into() },
        ];
        assert_eq!(latest_goal_id(&events2), None);
        assert!(!goal_awaiting_advance(&events2));
        assert_eq!(latest_goal_line(&events2), None);
    }

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

    /// **往返契约**：格式化出来的文本，必须能被**真正的解析器**（`parse_refs`）
    /// 还原成同一条引用。
    ///
    /// 为什么用 `parse_refs` 而不是 `parse_file_ref`：前者才是链路上的真实消费者
    /// （输入框文本 → 引用列表）。只测 `parse_file_ref` 会漏掉分词那一步 ——
    /// 而"路径含空格"的 bug 恰恰发生在分词（`@my file.txt` 被切成两个词）。
    #[test]
    fn formatted_file_refs_round_trip_through_the_real_parser() {
        let cases: &[&str] = &[
            "src/main.rs",
            "my file.txt",          // 含空格
            "a b/c d.rs",           // 多段含空格
            "中文 路径/文件.rs",     // 含空格的中文路径
            "a\"b.txt",             // 含双引号
            "a'b.txt",              // 含单引号
            "a\"b'c.txt",           // 两种引号都有
            "src/weird\u{3000}space.rs", // 全角空格（也属 whitespace）
            "#hash.rs",             // 以 # 开头（路径本身含 #）
            "dir/with#hash/x.rs",
        ];
        for path in cases {
            for lines in [None, Some((1usize, 1usize)), Some((12usize, 40usize))] {
                let formatted = format_file_ref(path, lines)
                    .unwrap_or_else(|| panic!("应能格式化 {path:?}"));
                let refs = parse_refs(&formatted);
                assert_eq!(
                    refs.len(),
                    1,
                    "格式化结果 {formatted:?} 应解析出**恰好一条**引用（{path:?} lines={lines:?}）：{refs:?}"
                );
                assert_eq!(refs[0].kind, RefKind::File, "应是文件引用：{formatted:?}");
                assert_eq!(
                    refs[0].target, *path,
                    "路径往返不一致：{path:?} → {formatted:?} → {:?}",
                    refs[0].target
                );
                assert_eq!(
                    refs[0].lines,
                    lines.map(|(a, b)| (a, b)),
                    "行范围往返不一致：{formatted:?}"
                );
            }
        }
    }

    /// 不可表示的路径**返回 None**，不硬拼一个会被解析错的字符串。
    #[test]
    fn unrepresentable_paths_are_rejected_instead_of_mangled() {
        assert!(format_file_ref("", None).is_none(), "空路径");
        assert!(format_file_ref("a\nb.txt", None).is_none(), "含换行（无法单行表示）");
        assert!(format_file_ref("a\rb.txt", None).is_none(), "含回车");
        // 非法行范围 → 当作没有范围（而不是产出解析回来会被丢弃的东西）
        assert_eq!(format_file_ref("a.rs", Some((0, 5))).as_deref(), Some("@a.rs"));
        assert_eq!(format_file_ref("a.rs", Some((9, 3))).as_deref(), Some("@a.rs"));
    }

    /// 常见路径**不加引号**（可读性优先：输入框里 `@src/main.rs` 比 `@"src/main.rs"` 好读）。
    #[test]
    fn plain_paths_are_not_unnecessarily_quoted() {
        assert_eq!(format_file_ref("src/main.rs", None).as_deref(), Some("@src/main.rs"));
        assert_eq!(
            format_file_ref("src/main.rs", Some((12, 40))).as_deref(),
            Some("@src/main.rs#12-40")
        );
        assert_eq!(
            format_file_ref("src/main.rs", Some((7, 7))).as_deref(),
            Some("@src/main.rs#7")
        );
        // 只在必要时加引号
        assert_eq!(format_file_ref("my file.txt", None).as_deref(), Some("@\"my file.txt\""));
    }

    #[test]
    fn quoted_paths_keep_whitespace_in_the_target() {
        // 引号语法的核心价值：带空格的路径不再被空白切分切坏
        let refs = parse_refs(r##"看下 @"my file.txt"#2-3 和 @'单 字.md'"##);
        assert_eq!(
            refs,
            vec![
                ContextRef {
                    kind: RefKind::File,
                    target: "my file.txt".into(),
                    lines: Some((2, 3)),
                },
                ContextRef {
                    kind: RefKind::File,
                    target: "单 字.md".into(),
                    lines: None,
                },
            ]
        );
    }

    #[test]
    fn quoted_target_supports_escaped_quote_and_all_sigils() {
        let refs = parse_refs(r#"@"a\"b.md" $'skill 一'"#);
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].target, "a\"b.md", r#"\" 应为字面引号"#);
        assert_eq!(refs[1].kind, RefKind::Skill);
        assert_eq!(refs[1].target, "skill 一");
    }

    #[test]
    fn unclosed_or_empty_quotes_fall_back_to_plain_words() {
        // 不猜意图：未闭合引号整体按普通词 —— 与历史行为一致
        let refs = parse_refs("@\"没闭合 file");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].target, "\"没闭合", "未闭合引号按普通词（含引号字符）");
        // 空引号体 = 目标为空 = 不算引用
        assert!(parse_refs("@\"\"").is_empty());
        // 引号不在符号后：与老行为一致（普通词）
        let refs2 = parse_refs("echo \"hi there\"");
        assert!(refs2.is_empty());
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
