//! 事件流 → 显示块：两个 GUI 宿主共用的转录模型。
//!
//! # 为什么在驱动层而不是某个宿主里
//!
//! 这段逻辑（吃 `EventMsg`、出可显示的块）**与渲染后端无关** —— egui 与 gpui
//! 两个宿主都需要它，而架构守卫 A3 禁止宿主互相依赖。所以它必须待在宿主之下。
//!
//! 它放在 `neo-driver` 而不是 UI 栈（`neo-ui-*`）里，是因为它**依赖内核轴**
//! （认识 `EventMsg` / `GoalSnapshot` / `TodoEntry`），而 UI 栈被门禁
//! `check_ui_layering.py` 的 U1 要求"不得依赖内核轴"（那样才能整目录开源）。
//! 换句话说：**能脱离内核独立发布的进 UI 栈，不能的进这里**。
//!
//! # 与绘制分开
//!
//! [`Transcript`] 是**纯数据**：不依赖任何 GUI 库，因此能在无显示环境
//! （CI 是 Linux、无窗口）里完整单测。绘制留在各宿主 —— 这条分界是
//! "界面逻辑可测"的前提。
//!
//! # 富界面的数据早就在事件流里
//!
//! 把 D2–D7 需要的信息接上，而不是丢掉：
//!
//! | 显示 | 来源事件 |
//! |---|---|
//! | 正文 | `AgentMessageDelta` / `AgentMessageDone` |
//! | 思考轨迹 | `ReasoningDelta` ← 旧的内置页面直接丢弃 |
//! | 工具卡片 | `ToolCallBegin{name, arguments}` / `ToolCallEnd` |
//! | 改动预览 | `PatchProposed{path, diff}` ← 内核审批前已生成 |
//! | 轮摘要 | `TurnComplete` |
//! | 目标面板 | `GoalUpdated{snapshot}` / `GoalCleared` |
//! | 审批 | `ApprovalRequest` |

use neo_protocol::{EventMsg, GoalSnapshot, TodoEntry};
use neo_text::Tone;

/// 一个工具调用的卡片（D4）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCard {
    pub id: String,
    pub name: String,
    /// 参数摘要：一行、有长度上限（见 [`summarize_args`]）。
    pub args: String,
    pub done: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
}

/// 待审批项（D10）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub id: String,
    pub detail: String,
    /// 内核判定的**调用类别**（read/write/network/interactive）。
    ///
    /// 必须用内核给的值，不能按工具名自己推断 —— 否则"总是允许"放行的范围
    /// 会与界面显示的不一致（`bash` 按命令内容分类）。
    pub kind: String,
}

/// 一个显示块。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// 用户提交的任务。
    User(String),
    /// 助手正文（Markdown 原文，绘制时解析）。
    Assistant(String),
    /// 思考轨迹（可折叠）。
    Reasoning(String),
    Tool(ToolCard),
    /// 审批前的改动预览。
    Diff { path: String, diff: String },
    /// 轮摘要。
    TurnSummary { input_tokens: u64, output_tokens: u64 },
    /// 已改动文件聚合（侧栏 + 转录里各有一份表达）。
    Files(Vec<(String, usize, usize)>),
    /// 待办清单。
    Todos(Vec<TodoEntry>),
    /// 提示/错误等一次性信息。
    Notice { text: String, tone: Tone },
}

/// 转录模型：吃事件、出块。**不依赖 egui**，因此可无窗口测试。
#[derive(Default)]
pub struct Transcript {
    pub blocks: Vec<Block>,
    /// 待审批；非 None 时**输入区被阻塞**（ZCode 语义：权限门触发即暂停当前任务）。
    pub pending: Option<Pending>,
    pub goal: Option<GoalSnapshot>,
    /// 是否有一轮在跑（用于状态行与帧节拍）。
    pub running: bool,
    /// 已折叠的思考块（按块下标）。
    pub collapsed_reasoning: std::collections::HashSet<usize>,
    /// 已**展开**的工具组（按组的起始块下标）。
    ///
    /// 与 `collapsed_reasoning` 同一个地址，理由也同一个：它们都是"按块下标
    /// 记的折叠状态"。放进宿主会漏掉 `clear_view` 的清理 —— 清屏后块下标
    /// 从 0 重新开始，残留的下标会与新块撞上，于是某个组莫名其妙是展开的
    /// （而这种 bug 只在"清除后恰好又生成了同下标的组"时出现，最难查）。
    pub expanded_tool_runs: std::collections::HashSet<usize>,
    /// 已**展开**的 diff 折叠区，键 = `(块下标, 被折区间起点)`。
    ///
    /// 与 `collapsed_reasoning` / `expanded_tool_runs` 同一个地址，理由也同一个：
    /// 它们都是"按块下标记的折叠状态"。放进宿主会漏掉 `clear_view` 的清理 ——
    /// 清屏后块下标从 0 重新开始，残留的键会与新块撞上，于是某个折叠区莫名其妙
    /// 是展开的（只在"清除后恰好又生成了同块下标 + 同区间起点"时出现，最难查）。
    ///
    /// 键里**两个分量都不能省**：一个 diff 块可以有多处被折的未改区，
    /// 只用块下标会让"展开任一处 = 展开全部"（用户点开一处，另一处也开了）。
    pub expanded_diff_folds: std::collections::HashSet<(usize, usize)>,
    /// 累计 token。
    pub total_in: u64,
    pub total_out: u64,
    /// 已经就哪个 (goal_id, iterations) 请求过推进 —— 防重复下发。
    pub advanced_for: Option<(String, usize)>,
}

impl Transcript {
    pub fn new() -> Self {
        Self::default()
    }

    /// 消费一批事件。返回是否应当请求下一帧（有变化就继续画）。
    pub fn push_batch(&mut self, events: &[EventMsg]) -> bool {
        let before = self.blocks.len();
        let mut changed = false;
        for ev in events {
            self.push_one(ev);
            changed = true;
        }
        changed || self.blocks.len() != before
    }

    fn push_one(&mut self, ev: &EventMsg) {
        match ev {
            EventMsg::UserSubmitted { text } => {
                self.push_block(Block::User(text.clone()));
            }
            EventMsg::TurnStarted { .. } => {
                self.running = true;
            }
            // D2：助手正文。**追加到末尾同类块**，否则每个增量都会开一个新块，
            // 一个回复会被画成几百段（真实踩过：TUI 的 pending_text 语义）。
            EventMsg::AgentMessageDelta { delta } => {
                match self.blocks.last_mut() {
                    Some(Block::Assistant(s)) => s.push_str(delta),
                    _ => self.push_block(Block::Assistant(delta.clone())),
                }
            }
            EventMsg::AgentMessageDone { text } => {
                // Done 带完整正文：以它为准（流式增量可能被截断或漏）
                match self.blocks.last_mut() {
                    Some(Block::Assistant(s)) => *s = text.clone(),
                    _ => self.push_block(Block::Assistant(text.clone())),
                }
            }
            // D3：思考轨迹 —— 旧页面把它整个丢掉
            EventMsg::ReasoningDelta { delta } => {
                match self.blocks.last_mut() {
                    Some(Block::Reasoning(s)) => s.push_str(delta),
                    _ => self.push_block(Block::Reasoning(delta.clone())),
                }
            }
            // D4：工具卡片
            EventMsg::ToolCallBegin { id, name, arguments } => {
                self.push_block(Block::Tool(ToolCard {
                    id: id.clone(),
                    name: name.clone(),
                    args: summarize_args(name, arguments),
                    done: false,
                    exit_code: None,
                    stdout: String::new(),
                    stderr: String::new(),
                    truncated: false,
                }));
            }
            EventMsg::ToolCallEnd { id, exit_code, stdout, stderr, truncated } => {
                // 按 id 配对（ToolCallEnd 只带 id，名字在 Begin 里）。
                //
                // 取**最早的那个未完成**卡，而不是最后一个匹配的：同一个 id
                // 出现两次时（provider 给了重复 id），从后往前找会让第一张卡
                // 永远停在"执行中" —— 界面上是一条永远不结束的工具行，
                // 而用户没有任何办法知道它其实早就跑完了。
                //
                // 顺序也天然正确：内核按调用顺序发 End，逐个认领即一一对应。
                if let Some(card) = self.blocks.iter_mut().find_map(|b| match b {
                    Block::Tool(c) if c.id == *id && !c.done => Some(c),
                    _ => None,
                }) {
                    card.done = true;
                    card.exit_code = Some(*exit_code);
                    card.stdout = stdout.clone();
                    card.stderr = stderr.clone();
                    card.truncated = *truncated;
                }
            }
            // D5：审批前把改动画出来（这是"不盲批"的关键）
            EventMsg::PatchProposed { path, diff } => {
                self.push_block(Block::Diff { path: path.clone(), diff: diff.clone() });
            }
            // D10：审批请求 → 挂起 + 阻塞输入
            EventMsg::ApprovalRequest { id, detail, kind } => {
                self.running = false;
                self.pending = Some(Pending {
                    id: id.clone(),
                    detail: detail.clone(),
                    kind: kind.clone(),
                });
            }
            // D6：轮摘要
            EventMsg::TurnComplete { input_tokens, output_tokens } => {
                self.running = false;
                self.total_in += input_tokens;
                self.total_out += output_tokens;
                self.push_block(Block::TurnSummary {
                    input_tokens: *input_tokens,
                    output_tokens: *output_tokens,
                });
            }
            EventMsg::FilesChanged { files } => {
                let list: Vec<(String, usize, usize)> = files
                    .iter()
                    .map(|f| (f.path.clone(), f.additions, f.deletions))
                    .collect();
                // 聚合列表是**覆盖**语义（内核已持有累计状态），
                // 不是追加 —— 逐条追加会让同一文件出现多次
                match self.blocks.iter_mut().rev().find(|b| matches!(b, Block::Files(_))) {
                    Some(Block::Files(slot)) => *slot = list,
                    _ => self.push_block(Block::Files(list)),
                }
            }
            EventMsg::TodoUpdated { items } => {
                match self.blocks.iter_mut().rev().find(|b| matches!(b, Block::Todos(_))) {
                    Some(Block::Todos(slot)) => *slot = items.clone(),
                    _ => self.push_block(Block::Todos(items.clone())),
                }
            }
            // 引用解析结果（`@文件` / `$技能`）：**用户必须看到"引用到了没有"**。
            //
            // 不显示正文：几百行文件正文会把对话挤没，而那是**模型上下文**，
            // 不是人要读的对话（正文已随事件落盘，回放与审计不受影响）。
            // 与 TUI 同一约定、同一来源（`facts_of`）—— 各宿主自己拼摘要
            // 必然漂移出"同一条引用在两个宿主里说法不同"。
            EventMsg::RefsResolved { summary, .. } => {
                for line in summary {
                    self.push_block(Block::Notice { text: format!("  {line}"), tone: Tone::Muted });
                }
            }
            // 项目指令（AGENTS.md 级联）来源：用户需要知道这次会话受哪些约定约束；
            // **截断必须显式告警** —— 静默截断会让用户以为模型看到了完整约定。
            EventMsg::InstructionsLoaded { sources, truncated, .. } => {
                self.push_block(Block::Notice {
                    text: format!(
                        "⚑ 项目指令：{} 个文件{}",
                        sources.len(),
                        if *truncated { "（已按 32 KiB 截断）" } else { "" }
                    ),
                    tone: Tone::Muted,
                });
            }
            // D7：Goal 面板
            EventMsg::GoalUpdated { snapshot } => {
                self.goal = Some(snapshot.clone());
            }
            EventMsg::GoalCleared { .. } => {
                self.goal = None;
                self.advanced_for = None;
            }
            EventMsg::ContextCompacted { removed_messages, summary } => {
                self.push_block(Block::Notice {
                    text: format!("（上下文已压缩：移除 {removed_messages} 条，摘要 {} 字）", summary.chars().count()),
                    tone: Tone::Muted,
                });
            }
            EventMsg::Error { message } => {
                self.running = false;
                self.push_block(Block::Notice { text: message.clone(), tone: Tone::Error });
            }
            EventMsg::Rewound { turns, .. } => {
                self.push_block(Block::Notice {
                    text: format!("（已回退 {turns} 轮对话）"),
                    tone: Tone::Muted,
                });
            }
            _ => {} // 其余事件不改变转录（会话配置、检查点等）
        }
    }

    fn push_block(&mut self, b: Block) {
        self.blocks.push(b);
    }

    /// 是否应当为当前目标请求推进（Goal 编排的驱动源）。
    ///
    /// 与 Web 页面同一约定：**由看到最新快照的一方驱动**，而不是服务端踢一脚。
    /// 去重键是 `(goal_id, iterations)` —— 同一个快照不重复下发，
    /// 每次推进产生新快照（iterations 递增）才会再下发一次。
    pub fn should_advance_goal(&self) -> bool {
        let Some(g) = &self.goal else { return false };
        if g.paused || g.stopped.is_some() || g.turns_remaining == 0 {
            return false;
        }
        self.advanced_for.as_ref() != Some(&(g.goal_id.clone(), g.iterations))
    }

    /// 只清**屏幕上的**转录。
    ///
    /// 刻意不清 `goal` / `total_in` / `total_out` / `pending`：那些是**状态**
    /// 不是显示内容 —— 清掉会让"目标还在跑"变成"目标没了"，
    /// 或让审批挂起时的阻塞状态消失（那就可能把下一步排进队列）。
    /// 用户想清的是"看不过来的一屏文字"，不是把会话状态重置。
    pub fn clear_view(&mut self) {
        self.blocks.clear();
        self.collapsed_reasoning.clear();
        self.expanded_tool_runs.clear();
        // 块下标已从 0 重新开始，折叠键必须一并清掉（见字段说明的"撞上"）
        self.expanded_diff_folds.clear();
    }

    /// 记下"已为该快照请求推进"，避免重复下发。
    pub fn mark_advanced(&mut self) {
        if let Some(g) = &self.goal {
            self.advanced_for = Some((g.goal_id.clone(), g.iterations));
        }
    }
}

/// 执行模式的中文标签（界面用；`mode_short` 是命令行用的英文短名）。
///
/// 与 ZCode 的四档是同一类东西（见 `desktop-parity.md` §1.3）：
/// 它那边的 Ask before changes / Edit automatically / Plan / Full access
/// 可以直接映射到我们内核的 `ExecMode` 五档，不需要发明新概念。
pub fn mode_label(mode: neo_protocol::ExecMode) -> &'static str {
    use neo_protocol::ExecMode as M;
    match mode {
        M::Plan => "计划",
        M::ConfirmBefore => "变更前确认",
        M::Default => "默认",
        M::AutoEdit => "自动编辑",
        M::FullAccess => "完全放行",
    }
}

/// 下一档模式（`Shift+Tab` 循环）。顺序按"限制从紧到松"。
pub fn next_mode(mode: neo_protocol::ExecMode) -> neo_protocol::ExecMode {
    use neo_protocol::ExecMode as M;
    match mode {
        M::Plan => M::ConfirmBefore,
        M::ConfirmBefore => M::Default,
        M::Default => M::AutoEdit,
        M::AutoEdit => M::FullAccess,
        M::FullAccess => M::Plan,
    }
}

/// 该模式是否需要**常驻**风险提示（ZCode 语义：高风险/全自动档位要在工具栏
/// 持续显示，不能只在弹窗里提一次）。依据是"这个档位允许不经确认就写"。
pub fn mode_is_risky(mode: neo_protocol::ExecMode) -> bool {
    use neo_protocol::ExecMode as M;
    matches!(mode, M::AutoEdit | M::FullAccess)
}


/// 工具参数摘要：单行、有长度上限。
///
/// # 为什么必须自己摘要而不是 `to_string()` 整个 JSON
///
/// 参数里可能有 `new`（整份文件内容）这类巨大字段 —— 直接铺开会把转录淹掉。
/// 优先挑**有信息量**的字段（命令、路径、模式），其余折叠成键名列表。
pub fn summarize_args(name: &str, args: &serde_json::Value) -> String {
    const MAX: usize = 160;
    let obj = match args.as_object() {
        Some(o) => o,
        None => return String::new(),
    };
    // 按"信息量"排序的优先字段
    let preferred = ["command", "path", "pattern", "old", "new", "query", "cmd"];
    let mut parts: Vec<String> = Vec::new();
    for key in preferred {
        let Some(v) = obj.get(key) else { continue };
        let s = match v {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        // 多行内容只取首行 + 行数提示，避免铺满
        let shown = if s.contains('\n') {
            let first = s.lines().next().unwrap_or("");
            format!("{}…（共 {} 行）", first, s.lines().count())
        } else {
            s
        };
        parts.push(format!("{key}={}", truncate_chars(&shown, 60)));
    }
    if parts.is_empty() {
        // 没有已知字段：列出键名，至少让用户知道传了什么
        let keys: Vec<&str> = obj.keys().map(|k| k.as_str()).collect();
        if keys.is_empty() {
            return String::new();
        }
        parts.push(format!("({})", keys.join(", ")));
    }
    let _ = name;
    truncate_chars(&parts.join(" "), MAX)
}

/// 按**字符**截断（不是字节 —— 按字节切会切碎中文）。
pub fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// diff 行的种类（用于着色）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    Header,
    Hunk,
    Add,
    Del,
    Context,
    /// 我们自己在截断时追加的说明行（`… 另有 N 处…`、`（改动过大…）`）。
    Meta,
}

/// 判定 unified diff 一行的种类。
///
/// 与**生产者**（`neo-capability::diff::unified_diff`）的约定：文件头 `---`/`+++`、
/// hunk 头 `@@ … @@`、内容行以 `+`/`-`/空格 开头，其余（含中文说明行）按 Meta。
/// 注意顺序：`---`/`+++` 也是以 `-`/`+` 开头的，必须先判文件头。
pub fn diff_line_kind(line: &str) -> DiffLineKind {
    if line.starts_with("---") || line.starts_with("+++") {
        return DiffLineKind::Header;
    }
    if line.starts_with("@@") {
        return DiffLineKind::Hunk;
    }
    if line.starts_with('+') {
        return DiffLineKind::Add;
    }
    if line.starts_with('-') {
        return DiffLineKind::Del;
    }
    if line.starts_with(' ') {
        return DiffLineKind::Context;
    }
    DiffLineKind::Meta
}

/// 单行内**被改动的片段**（字节范围，相对该行原文）。
///
/// 范围以**字节**计，因为渲染层要拿它去切 `&str`。⚠️ 见 [`inline_emphasis`]
/// 里关于"char 索引 ≠ 字节偏移"的说明 —— 中文/emoji 下这两者差得很远。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InlineEmphasis {
    /// 该行内需要强调的片段（按出现顺序，互不重叠、升序）。
    pub ranges: Vec<std::ops::Range<usize>>,
}

impl InlineEmphasis {
    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }
}

/// 参与行内比对的**最长字符数**。超过则放弃行内高亮（整行着色）。
///
/// # 为什么需要上限
///
/// 行内比对是字符级 O(n·m) 的动态规划。一行几万字符（压缩过的 JS、长 base64）
/// 会让它退化成秒级卡顿 —— 而这类行**本来就没有"改了一个词"可言**，
/// 整行着色反而是更诚实的呈现。
///
/// 取 2000：正常源码行远小于它，而 2000×2000 的 DP 仍在毫秒级。
pub const MAX_INLINE_CHARS: usize = 2000;

/// 这一行是不是 `git` 的 **"文件末尾无换行"标记**（`\ No newline at end of file`）。
///
/// # 为什么必须单独识别它（真机实测抓到的）
///
/// 我们自己的 `apply_patch` 就常产出它：旧文件无尾换行时，`similar` 会在
/// `-` 块与 `+` 块**之间**插一行这个标记。而它在 [`diff_line_kind`] 里落在
/// `Meta`（既不是 Del 也不是 Add）—— 于是"Del 块紧跟 Add 块"这个配对条件
/// **被它打断**，行内强调在最常见的一类改动（整文件重写且末尾无换行）上
/// **永远不生效**，而屏幕上完全看不出来（只是"没有强调"）。
///
/// 修法：配对时**跳过**它（它不携带内容，只是相邻性的说明），但仍拒绝跨
/// `Context` 配对 —— 上下文行意味着两段真的不相邻。
fn is_no_newline_marker(line: &str) -> bool {
    line.trim_start().starts_with("\\ No newline")
}

/// 计算一段 diff 的**行内强调区间**，下标与入参 `lines` 一一对应。
///
/// # 它解决什么
///
/// 只按行着色时，一行 200 字符里改了一个词，**整行都是绿的** —— 用户还是得
/// 自己逐字找。GitHub / Zed / ZCode 都会把**行内真正变化的那几个字**再强调一次。
///
/// # 配对规则（照 unified diff 的形状）
///
/// unified diff 把一处替换写成"先 N 行 `-`、再 N 行 `+`"（中间不夹其它种类）。
/// 所以：遇到一段连续 `Del` 紧跟一段连续 `Add`，就**按序配对**
/// （第 1 个 Del 配第 1 个 Add），只比内容（去掉行首的 `-`/`+` 标记）。
/// 配不上的（纯增 / 纯删 / 数量不等）不产生强调 —— 那种情况整行着色已经够了。
///
/// 中间只允许夹 `\ No newline at end of file` 这类**无内容标记**（见
/// [`is_no_newline_marker`]）；夹了上下文行则视为不相邻，不配对。
///
/// # ⚠️ 两个必须做对的地方（都是静默出错的那类）
///
/// 1. **`similar` 的 `from_chars` 返回的是 char 下标，不是字节偏移**。
///    实测 `-中文旧内容`：6 个 char / 16 字节。若把 char 下标直接当字节用，
///    切出来的不是那一段 —— 中文与 emoji 下会切在字符中间（`&str` 切片甚至
///    **直接 panic**）。所以这里用 `char_indices` 建立 char→byte 映射。
/// 2. **必须跳过行首标记**：`-`/`+` 不是内容，把它算进比对会得到
///    "每个 `-` 都变成了 `+`"这种占满全行的假强调。
pub fn inline_emphasis(lines: &[&str]) -> Vec<Option<InlineEmphasis>> {
    let kinds: Vec<DiffLineKind> = lines.iter().map(|l| diff_line_kind(l)).collect();
    let mut out: Vec<Option<InlineEmphasis>> = vec![None; lines.len()];

    let mut i = 0;
    while i < lines.len() {
        if kinds[i] != DiffLineKind::Del {
            i += 1;
            continue;
        }
        let del_start = i;
        while i < lines.len() && kinds[i] == DiffLineKind::Del {
            i += 1;
        }
        let del_end = i;

        // 只在"无内容标记"上向前看 —— 遇到任何别的种类（含 Context）就停，
        // 因为那意味着这两段不相邻。
        let mut j = i;
        while j < lines.len() && is_no_newline_marker(lines[j]) {
            j += 1;
        }
        let add_start = j;
        let mut k = j;
        while k < lines.len() && kinds[k] == DiffLineKind::Add {
            k += 1;
        }
        let add_end = k;
        i = add_end.max(i);

        // 没有紧跟的 Add 块 → 这不是一处替换（纯删除 / 被上下文隔开）
        if add_start == add_end {
            continue;
        }
        let pairs = (del_end - del_start).min(add_end - add_start);
        for k in 0..pairs {
            let d = del_start + k;
            let a = add_start + k;
            if let Some((dr, ar)) = emphasize_pair(lines[d], lines[a]) {
                out[d] = Some(InlineEmphasis { ranges: dr });
                out[a] = Some(InlineEmphasis { ranges: ar });
            }
        }
    }
    out
}

/// 比对一对（旧行, 新行），返回两侧要强调的字节范围。
///
/// 任一侧超长、或内容完全相同（不该发生，但防御）时返回 `None`。
fn emphasize_pair(
    old_line: &str,
    new_line: &str,
) -> Option<(Vec<std::ops::Range<usize>>, Vec<std::ops::Range<usize>>)> {
    // 内容 = 去掉行首标记（`-`/`+`）。标记占 1 字节（都是 ASCII），
    // 但**不假设**它一定存在 —— 防御式取下界。
    let (old_body, old_mark_len) = strip_marker(old_line);
    let (new_body, new_mark_len) = strip_marker(new_line);

    if old_body == new_body {
        return None;
    }
    let old_chars = old_body.chars().count();
    let new_chars = new_body.chars().count();
    if old_chars > MAX_INLINE_CHARS || new_chars > MAX_INLINE_CHARS {
        return None; // 太长不比对（见 MAX_INLINE_CHARS 的说明）
    }

    // ⚠️ `from_chars` 的区间是 **char 下标**；下面一律先经 char→byte 映射，
    //    绝不把 char 下标直接当字节用（中文/emoji 下会切碎甚至 panic）。
    let diff = similar::TextDiff::from_chars(old_body, new_body);

    let old_to_byte = char_to_byte_map(old_body);
    let new_to_byte = char_to_byte_map(new_body);

    let mut old_ranges = Vec::new();
    let mut new_ranges = Vec::new();
    for op in diff.ops() {
        use similar::DiffTag;
        match op.tag() {
            DiffTag::Equal => {}
            DiffTag::Delete => {
                push_mapped(&mut old_ranges, &old_to_byte, op.old_range(), old_mark_len)
            }
            DiffTag::Insert => {
                push_mapped(&mut new_ranges, &new_to_byte, op.new_range(), new_mark_len)
            }
            DiffTag::Replace => {
                push_mapped(&mut old_ranges, &old_to_byte, op.old_range(), old_mark_len);
                push_mapped(&mut new_ranges, &new_to_byte, op.new_range(), new_mark_len);
            }
        }
    }
    // 合并相邻/重叠片段：`similar` 可能把连续替换切成多个 op，
    // 分开渲染会得到若干"本该连成一片"的碎块（视觉上像噪点）。
    coalesce(&mut old_ranges);
    coalesce(&mut new_ranges);
    Some((old_ranges, new_ranges))
}

/// 去掉行首的 diff 标记，返回 `(内容, 标记字节数)`。
fn strip_marker(line: &str) -> (&str, usize) {
    let mut it = line.char_indices();
    match it.next() {
        Some((_, c)) if c == '-' || c == '+' || c == ' ' => {
            let rest_start = it.next().map(|(i, _)| i).unwrap_or(line.len());
            (&line[rest_start..], rest_start)
        }
        _ => (line, 0),
    }
}

/// char 下标 → 字节偏移的映射表（长度 = char 数 + 1，末项是字节总长）。
///
/// 用 `char_indices` 而不是 `c.len_utf8()` 累加：前者由标准库保证与切片边界
/// 一致，后者要自己保证 —— 而这正是 UTF-8 切片的经典出错点。
fn char_to_byte_map(s: &str) -> Vec<usize> {
    let mut map: Vec<usize> = s.char_indices().map(|(b, _)| b).collect();
    map.push(s.len());
    map
}

/// 把一个 char 区间映射成字节区间并追加（含行首标记的偏移修正）。
fn push_mapped(
    out: &mut Vec<std::ops::Range<usize>>,
    map: &[usize],
    char_range: std::ops::Range<usize>,
    mark_bytes: usize,
) {
    // 越界防御：`map` 的末项是字节总长，`char_range.end` 至多等于 char 数。
    let (Some(&start), Some(&end)) = (map.get(char_range.start), map.get(char_range.end)) else {
        return;
    };
    if start >= end {
        return; // 空区间不产生强调
    }
    out.push((start + mark_bytes)..(end + mark_bytes));
}

/// 合并相邻或重叠的区间（原地）。
fn coalesce(ranges: &mut Vec<std::ops::Range<usize>>) {
    if ranges.len() < 2 {
        return;
    }
    ranges.sort_by_key(|r| r.start);
    let mut merged: Vec<std::ops::Range<usize>> = Vec::with_capacity(ranges.len());
    for r in ranges.drain(..) {
        match merged.last_mut() {
            Some(last) if r.start <= last.end => {
                if r.end > last.end {
                    last.end = r.end;
                }
            }
            _ => merged.push(r),
        }
    }
    *ranges = merged;
}

/// 一段**连续的工具调用**（用于分组显示）。
///
/// # 它解决的问题
///
/// 模型连续调 5 次工具时，5 张卡片（每张含参数摘要与输出）会把转录淹掉，
/// 用户反而看不到正文。ZCode 的做法是把它们**分成组**呈现。
///
/// # 判定放在共享层（纯逻辑、可测）
///
/// "哪些块构成一组"是**对 `&[Block]` 的纯计算**，与渲染后端无关 ——
/// 所以它在共享层，两个 GUI 宿主用同一套判定（否则同一个转录在两个宿主里
/// 分组不同，那是最难解释的一类不一致）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolRun {
    /// 该组第一个块在 `blocks` 里的下标。
    pub start: usize,
    /// 该组包含的块数（≥1）。
    pub len: usize,
}

impl ToolRun {
    /// 工具总数（每组都是纯 Tool 块，所以等于 len）。
    pub fn tool_count(&self) -> usize {
        self.len
    }
}

/// 把连续的 `Tool` 块切成组。返回每组的起止。
///
/// 只认**紧邻**的连续（中间夹任何别的块就断开）—— 那正是"一次连续的工具爆发"
/// 的语义：中间有正文说明模型已经回到对话了。
pub fn tool_runs(blocks: &[Block]) -> Vec<ToolRun> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < blocks.len() {
        if !matches!(blocks[i], Block::Tool(_)) {
            i += 1;
            continue;
        }
        let start = i;
        while i < blocks.len() && matches!(blocks[i], Block::Tool(_)) {
            i += 1;
        }
        out.push(ToolRun { start, len: i - start });
    }
    out
}

/// 组内所有工具是否都已完成，且都成功。
///
/// 供渲染层决定组标题的色调：**只要有失败就不能显示成"成功色"** ——
/// 一组绿字里藏着一个失败，用户会漏看（这是信息层次里最容易骗人的一处）。
pub fn run_all_succeeded(blocks: &[Block], run: &ToolRun) -> bool {
    blocks[run.start..run.start + run.len].iter().all(|b| match b {
        Block::Tool(c) => c.done && c.exit_code == Some(0),
        _ => true,
    })
}

/// 组是否还在执行中（有未完成的工具）。
pub fn run_in_progress(blocks: &[Block], run: &ToolRun) -> bool {
    blocks[run.start..run.start + run.len]
        .iter()
        .any(|b| matches!(b, Block::Tool(c) if !c.done))
}

/// 模型清单里的一项（供宿主的模型选择器展示）。
///
/// # 为什么需要具名类型而不是 `(String, String, bool)`
///
/// 三元组在调用处只能写成 `models[0].2`，读的人必须回翻定义才知道那是
/// "是否可用于真实任务"。而这一项**恰好是不能丢的**：`mock` / `selftest`
/// 这类桩 provider 跑不了真实任务，用户切过去会以为模型坏了。
///
/// # 为什么放在共享层
///
/// 两个 GUI 宿主 + TUI 都要展示同一份清单，且"桩要标出来"这条规则必须一致 ——
/// 各宿主自己拼会漂移出"同一个模型在某个宿主里被标成桩、在另一个里没有"
/// 这种最难解释的不一致。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelChoice {
    /// 注册名（切换时用它）。
    pub name: String,
    /// 人类可读说明（可能为空）。
    pub description: String,
    /// 是否可用于真实任务。`false` = 桩（mock/selftest/demo 之类）。
    pub production: bool,
}

impl ModelChoice {
    pub fn new(name: impl Into<String>, description: impl Into<String>, production: bool) -> Self {
        Self { name: name.into(), description: description.into(), production }
    }

    /// 展示名：桩要**显式标出**。
    ///
    /// 只显示名字的话，用户切到 `mock` 会以为模型坏了（它回的是固定文本）；
    /// 标一个"（桩）"就说明了那是在演示链路、不是模型在答。
    pub fn display_name(&self) -> String {
        if self.production {
            self.name.clone()
        } else {
            format!("{}（桩）", self.name)
        }
    }
}

/// 一次搜索命中：块下标 + 该块内的字节区间（**可多个**）。
///
/// 只在**思考块**里搜（D3 的原始需求是"思考轨迹可搜索"）。
/// 不做"搜全文"是因为转录里的工具输出动辄上万行，一搜就是全量扫描 ——
/// 而用户要找的几乎总是自己想不起来的某段推理。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReasoningHit {
    /// 命中所在的块下标。
    pub block: usize,
    /// 该块内所有匹配的字节区间（按位置升序、互不重叠）。
    pub ranges: Vec<std::ops::Range<usize>>,
}

/// 在思考块里查找 `query`（**大小写不敏感**，按字节区间返回，供高亮用）。
///
/// 为什么大小写不敏感：用户回忆某段推理时记不准大小写（`JSON` / `json`、
/// `HashMap` / `hashmap`），区分大小写会让"明明有却搜不到"，
/// 而搜索失败时用户只会以为内容不在那里。
///
/// 匹配按**字节**而不是字符：调用方要拿去给 `HighlightStyle` 用，
/// 那套 API 收的就是字节区间。为此必须保证区间落在 char 边界上 ——
/// 下面用 `find` 在 `&str` 上做，天然满足（不会切在多字节字符中间）。
pub fn search_reasoning(blocks: &[Block], query: &str) -> Vec<ReasoningHit> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    for (idx, b) in blocks.iter().enumerate() {
        let Block::Reasoning(text) = b else { continue };
        let hay = text.to_lowercase();
        // ⚠️ `to_lowercase` 可能**改变字节长度**（如 `İ` → `i̇`），
        // 于是 haystack 上的偏移不能直接当作原串偏移。
        // 折中：只在两者字节长度相同时用小写副本定位，否则退回原串精确匹配。
        // 宁可"大小写敏感地搜"也不要给出**错位的高亮区间**
        // （错位会把高亮画到别的字上，比不亮更糟）。
        let hay = if hay.len() == text.len() { hay.as_str() } else { text.as_str() };
        let mut ranges = Vec::new();
        let mut from = 0usize;
        while from <= hay.len() {
            let Some(rel) = hay[from..].find(&needle) else { break };
            let start = from + rel;
            let end = start + needle.len();
            // 区间必须落在 char 边界上（小写副本非同长时上面已退回原串，
            // 但同一长度下仍可能有非边界情况，这里兜一层）
            if hay.is_char_boundary(start) && hay.is_char_boundary(end) {
                ranges.push(start..end);
            }
            from = end.max(start + 1);
        }
        if !ranges.is_empty() {
            hits.push(ReasoningHit { block: idx, ranges });
        }
    }
    hits
}

/// 把命中的块**整理成要展开的下标集合**。
///
/// 搜索一旦有结果，命中的思考块必须**自动展开** —— 否则用户看到"3 个结果"
/// 却在屏幕上找不到任何一个（折叠的块把命中的字藏起来了）。
/// 这是"搜索结果与可见内容必须对得上"的最小保证。
pub fn blocks_to_expand(hits: &[ReasoningHit]) -> std::collections::HashSet<usize> {
    hits.iter().map(|h| h.block).collect()
}

#[cfg(test)]
mod inline_emphasis_tests {
    use super::*;

    /// 取某行强调出的**文本片段**（测试断言读起来是"强调了哪几个字"，
    /// 而不是一堆字节下标 —— 后者看不出对错）。
    fn emphasized(lines: &[&str], idx: usize) -> Vec<String> {
        let all = inline_emphasis(lines);
        all[idx]
            .as_ref()
            .map(|e| {
                e.ranges
                    .iter()
                    .map(|r| lines[idx][r.clone()].to_string())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 一处行内替换：只强调变化的那几个字，其余不算。
    #[test]
    fn a_single_word_change_is_emphasized_not_the_whole_line() {
        let lines = vec!["-let x = 1;", "+let y = 1;"];
        assert_eq!(emphasized(&lines, 0), vec!["x"]);
        assert_eq!(emphasized(&lines, 1), vec!["y"]);
    }

    /// **CJK 是本节最容易错的地方**：`similar::from_chars` 给的是 **char 下标**，
    /// 而中文一行 6 个 char / 16 字节。若把 char 下标当字节用，切片会切在字符
    /// 中间（`&str` 甚至直接 panic），或强调到完全无关的位置。
    ///
    /// 这条用例就是为那个陷阱写的：断言**切出来的字符串是完整的汉字**。
    #[test]
    fn cjk_change_emphasizes_whole_characters_not_shredded_bytes() {
        let lines = vec!["-中文旧内容", "+中文新内容"];
        assert_eq!(emphasized(&lines, 0), vec!["旧"]);
        assert_eq!(emphasized(&lines, 1), vec!["新"]);
    }

    /// emoji（4 字节）+ 中文 + ASCII 混排 —— 字节/char 差异最大的情形。
    #[test]
    fn mixed_emoji_and_cjk_stays_on_character_boundaries() {
        // ⚠️ 必须一侧 `-` 一侧 `+`：只有 Del→Add 才配对（两侧都是 `+` 不构成替换）
        let lines = vec!["-准备 🚀 发布", "+准备 🚀 上线"];
        assert_eq!(emphasized(&lines, 0), vec!["发布"]);
        assert_eq!(emphasized(&lines, 1), vec!["上线"]);
        // 反向也成立
        let rev = vec!["-准备 🚀 上线", "+准备 🚀 发布"];
        assert_eq!(emphasized(&rev, 0), vec!["上线"]);
        assert_eq!(emphasized(&rev, 1), vec!["发布"]);
    }

    /// 行首的 `-`/`+` 标记**不是内容**，不能被算成"变化"。
    ///
    /// 若不跳过标记，比对会看到"-"→"+"，于是强调一个根本不存在的差异
    /// （表现是整行或行首被点亮）。
    #[test]
    fn the_line_marker_is_not_part_of_the_comparison() {
        let lines = vec!["-same", "+same"];
        assert!(
            emphasized(&lines, 0).is_empty(),
            "内容相同则无强调（标记不算内容）"
        );
        assert!(emphasized(&lines, 1).is_empty());
    }

    /// 纯新增 / 纯删除**不产生行内强调** —— 那种情况整行着色已经表达清楚了。
    #[test]
    fn pure_insertions_and_deletions_have_no_inline_emphasis() {
        let lines = vec!["@@ -1 +1 @@", "-旧", "+新甲乙丙", "+又一行"];
        let all = inline_emphasis(&lines);
        // 「-旧 / +新甲乙丙」配成一对 → 会强调；但第二个 `+又一行` 无配对 → 无强调
        assert!(
            all[3].is_none(),
            "没有配对的 Add（纯新增）不该有行内强调"
        );
        // Hunk 头永远无强调
        assert!(all[0].is_none());
    }

    /// 数量不等时按**序配对到较少的一侧**，多出来的不强行配对。
    #[test]
    fn uneven_delete_add_counts_pair_only_the_minimum() {
        let lines = vec!["-aaa", "-bbb", "+aaa2"];
        let all = inline_emphasis(&lines);
        assert!(all[0].is_some(), "第 1 个 Del 配第 1 个 Add");
        assert!(all[1].is_none(), "第 2 个 Del 没有配对 → 无强调");
        assert!(all[2].is_some(), "Add 侧配上了");
    }

    /// **`\ No newline at end of file` 夹在中间不打断配对。**
    ///
    /// 这条是**真机实测抓到的缺陷**：我们自己的 `apply_patch` 在旧文件无尾换行时
    /// 会产出这个标记，而它在 `diff_line_kind` 里是 `Meta` —— 于是"Del 块紧跟
    /// Add 块"的配对被打断，行内强调在这类**最常见的改动**上永远不生效，
    /// 且屏幕上只表现为"没有强调"，完全看不出是 bug。
    #[test]
    fn the_no_newline_marker_does_not_break_pairing() {
        let lines = vec![
            "--- a/f.txt",
            "+++ b/f.txt",
            "@@ -1 +1 @@",
            "-旧的一整行内容",
            "\\ No newline at end of file",
            "+新的一整行内容",
        ];
        let all = inline_emphasis(&lines);
        assert!(
            all[3].is_some() && all[5].is_some(),
            "中间夹着 `\\ No newline` 标记时，Del/Add 仍应配对（否则整文件重写永不强调）"
        );
        // 标记行自己不该被强调（它不是内容）
        assert!(all[4].is_none(), "标记行不该被强调");
    }

    /// 但**上下文行**仍然打断配对 —— 那种情况两段确实不相邻。
    ///
    /// 与上一条成对：说明"跳过"只针对无内容标记，不是放宽成"随便跨"。
    #[test]
    fn a_context_line_still_breaks_pairing_even_though_a_marker_does_not() {
        let lines = vec!["-旧", " 中间未改", "+新"];
        let all = inline_emphasis(&lines);
        assert!(
            all[0].is_none() && all[2].is_none(),
            "被上下文隔开的两段不是同一处替换"
        );
    }

    /// 中间夹了 Context 就不是同一处替换 —— 不配对。
    #[test]
    fn a_context_line_between_blocks_breaks_the_pairing() {
        let lines = vec!["-旧值", " 未改的上下文", "+新值"];
        let all = inline_emphasis(&lines);
        assert!(all[0].is_none(), "被 Context 隔开就不是同一处替换");
        assert!(all[2].is_none());
    }

    /// 超长行**放弃**行内比对（返回 None），不卡也不猜。
    #[test]
    fn an_overlong_line_skips_inline_comparison() {
        let big_old = format!("-{}", "a".repeat(MAX_INLINE_CHARS + 10));
        let big_new = format!("+{}", "a".repeat(MAX_INLINE_CHARS + 11));
        let lines = vec![big_old.as_str(), big_new.as_str()];
        let all = inline_emphasis(&lines);
        assert!(
            all[0].is_none() && all[1].is_none(),
            "超长行应跳过行内比对（{MAX_INLINE_CHARS} 字符上限），而不是退化成卡顿"
        );
    }

    /// **绝不重叠、绝不超过行长** —— 渲染层拿这些区间去切 `&str`，
    /// 越界或重叠都会 panic 或画出错乱的强调块。
    #[test]
    fn ranges_never_overlap_and_never_exceed_the_line() {
        let lines = vec![
            "-let a = 1; let b = 2;",
            "+let a = 9; let b = 8;",
            "-x",
            "+y",
        ];
        let all = inline_emphasis(&lines);
        for (i, e) in all.iter().enumerate() {
            let Some(e) = e else { continue };
            let mut prev_end = 0usize;
            for r in &e.ranges {
                assert!(r.start < r.end, "空区间不该出现（行 {i}）");
                assert!(r.end <= lines[i].len(), "区间越界（行 {i}）");
                assert!(
                    r.start >= prev_end,
                    "区间重叠或乱序（行 {i}）：{:?} 与上一个的 end={prev_end}",
                    e.ranges
                );
                assert!(
                    lines[i].is_char_boundary(r.start) && lines[i].is_char_boundary(r.end),
                    "区间必须落在字符边界上（行 {i}），否则切片会 panic"
                );
                prev_end = r.end;
            }
        }
    }

    /// 被**未改内容**隔开的改动是两段 —— 不合并（合并会把未改的空格也点亮）。
    #[test]
    fn runs_separated_by_unchanged_text_stay_separate() {
        let lines = vec!["-alpha beta gamma", "+alpha BETA GAMMA"];
        let all = inline_emphasis(&lines);
        let e = all[0].as_ref().expect("应强调");
        assert_eq!(
            e.ranges.len(),
            2,
            "`beta` 与 `gamma` 之间那个未改的空格不该被点亮，故是两段：{:?}",
            e.ranges
        );
        assert_eq!(emphasized(&lines, 0), vec!["beta", "gamma"]);
    }

    /// `coalesce` 把**真正相邻/重叠**的区间合并（防止渲染出碎块）。
    ///
    /// 直接测这个纯函数：`similar` 正常情况下已经把连续替换并成一个 op，
    /// 所以这条是**防御性**的（见函数注释）。用一个会重叠的输入验证它确实生效 ——
    /// 否则它就是一段永远没被执行的代码。
    #[test]
    fn coalesce_merges_adjacent_and_overlapping_ranges() {
        let mut r = vec![0..3, 3..6, 5..9, 20..22];
        coalesce(&mut r);
        assert_eq!(r, vec![0..9, 20..22], "相邻与重叠应并成一段，跳空的不动");

        let mut single = vec![4..5];
        coalesce(&mut single);
        assert_eq!(single, vec![4..5], "单元素不应被改动");

        let mut empty: Vec<std::ops::Range<usize>> = Vec::new();
        coalesce(&mut empty);
        assert!(empty.is_empty());
    }

    /// **真机用例**：整文件重写时只有一个字不同 —— 强调应落在那个字上。
    ///
    /// 这条用真机观察到的**确切字符串**，因为它是"行内强调到底有没有算出来"
    /// 的证据；纯构造的用例可能恰好绕开真实形状。
    #[test]
    fn the_real_selftest_diff_emphasizes_the_changed_character() {
        let lines = vec![
            "--- a/selftest.txt",
            "+++ b/selftest.txt",
            "@@ -1 +1 @@",
            "-由 selftest provider 经 apply_patch 写入？",
            "\\ No newline at end of file",
            "+由 selftest provider 经 apply_patch 写入。",
        ];
        let all = inline_emphasis(&lines);
        for (idx, want) in [(3usize, "？"), (5usize, "。")] {
            let e = all[idx]
                .as_ref()
                .unwrap_or_else(|| panic!("第 {idx} 行应被强调"));
            let got: Vec<String> = e.ranges.iter().map(|r| lines[idx][r.clone()].to_string()).collect();
            assert_eq!(got, vec![want.to_string()], "第 {idx} 行强调内容不对：{got:?}");
        }
    }

    /// 退化与边界输入不 panic。
    #[test]
    fn degenerate_inputs_do_not_panic() {
        assert!(inline_emphasis(&[]).is_empty());
        assert!(inline_emphasis(&[""]).len() == 1);
        // 只有标记、没有内容
        let all = inline_emphasis(&["-", "+"]);
        assert_eq!(all.len(), 2);
        // `-` 与 `---`（文件头）不配对
        let all = inline_emphasis(&["--- a/f", "+++ b/f"]);
        assert!(all.iter().all(|e| e.is_none()), "文件头不该被行内强调");
    }
}

#[cfg(test)]
mod refs_and_instructions_tests {
    use super::*;
    use neo_protocol::EventMsg;

    fn notices(t: &Transcript) -> Vec<String> {
        t.blocks
            .iter()
            .filter_map(|b| match b {
                Block::Notice { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    /// 引用解析结果必须**每行一条**显示 —— 用户要看的是"哪条引用没读到"，
    /// 合并成一行就分不出是哪条失败了。
    #[test]
    fn refs_summary_lines_become_notices() {
        let mut t = Transcript::new();
        t.push_batch(&[EventMsg::RefsResolved {
            summary: vec!["📄 a.rs 已注入".into(), "🧩 $x 未找到".into()],
            block: "（几百行正文，不该进转录）".into(),
        }]);
        let n = notices(&t);
        assert_eq!(n.len(), 2, "两条摘要应各占一行：{n:?}");
        assert!(n[0].contains("a.rs") && n[1].contains("未找到"));
        // 正文不进转录：那是模型上下文，不是对话
        assert!(
            !n.iter().any(|s| s.contains("几百行正文")),
            "注入正文不该出现在转录里"
        );
    }

    #[test]
    fn instructions_loaded_reports_source_count() {
        let mut t = Transcript::new();
        t.push_batch(&[EventMsg::InstructionsLoaded {
            sources: vec!["AGENTS.md".into(), ".neo/AGENTS.md".into()],
            block: "x".into(),
            truncated: false,
        }]);
        let n = notices(&t);
        assert_eq!(n.len(), 1);
        assert!(n[0].contains("2 个文件"), "{:?}", n[0]);
        assert!(!n[0].contains("截断"), "未截断时不该提截断");
    }

    /// 截断**必须显式告警**：静默截断会让用户以为模型看到了完整约定，
    /// 而他的规则可能正好在被切掉的那部分里。
    #[test]
    fn truncated_instructions_are_flagged() {
        let mut t = Transcript::new();
        t.push_batch(&[EventMsg::InstructionsLoaded {
            sources: vec!["AGENTS.md".into()],
            block: "x".into(),
            truncated: true,
        }]);
        let n = notices(&t);
        assert!(n[0].contains("截断"), "截断必须说出来：{:?}", n[0]);
    }
}

#[cfg(test)]
mod reasoning_search_tests {
    use super::*;

    fn blocks(specs: &[(bool, &str)]) -> Vec<Block> {
        specs
            .iter()
            .map(|(is_reasoning, t)| {
                if *is_reasoning {
                    Block::Reasoning((*t).to_string())
                } else {
                    Block::Assistant((*t).to_string())
                }
            })
            .collect()
    }

    #[test]
    fn finds_matches_only_in_reasoning_blocks() {
        let b = blocks(&[
            (true, "先看 JSON 依赖"),
            (false, "正文里也提到 JSON"),
            (true, "然后确认 json 大小写"),
        ]);
        let hits = search_reasoning(&b, "json");
        assert_eq!(hits.len(), 2, "只应命中思考块：{hits:?}");
        assert_eq!(hits[0].block, 0);
        assert_eq!(hits[1].block, 2, "正文块(下标 1)不应命中");
    }

    #[test]
    fn matching_is_case_insensitive() {
        let b = blocks(&[(true, "调用 HashMap 与 hashmap")]);
        let hits = search_reasoning(&b, "HASHMAP");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].ranges.len(), 2, "两种写法都该命中");
    }

    #[test]
    fn ranges_point_at_the_matched_text() {
        let b = blocks(&[(true, "前缀 cfg 后缀")]);
        let hits = search_reasoning(&b, "cfg");
        let r = &hits[0].ranges[0];
        assert_eq!(&"前缀 cfg 后缀"[r.clone()], "cfg");
    }

    /// 中文 query 的区间必须落在 **char 边界**上 ——
    /// 切在多字节字符中间，高亮会画到别的字上（比不亮更糟）。
    #[test]
    fn multibyte_query_yields_char_boundary_ranges() {
        let text = "先确认缩进与行号，再看缩进深度";
        let b = vec![Block::Reasoning(text.to_string())];
        let hits = search_reasoning(&b, "缩进");
        assert_eq!(hits[0].ranges.len(), 2);
        for r in &hits[0].ranges {
            assert!(text.is_char_boundary(r.start), "start 不在 char 边界");
            assert!(text.is_char_boundary(r.end), "end 不在 char 边界");
            assert_eq!(&text[r.clone()], "缩进", "区间应精确框住匹配的字");
        }
    }

    #[test]
    fn empty_or_blank_query_matches_nothing() {
        let b = blocks(&[(true, "有内容")]);
        assert!(search_reasoning(&b, "").is_empty());
        assert!(search_reasoning(&b, "   ").is_empty(), "全空白不该当成搜索");
    }

    /// 命中块必须自动展开 —— 否则用户看到"N 个结果"却在屏幕上找不到一个。
    #[test]
    fn hits_map_to_blocks_that_must_be_expanded() {
        let b = blocks(&[(true, "看 JSON"), (true, "无关"), (true, "又是 JSON")]);
        let hits = search_reasoning(&b, "json");
        let expand = blocks_to_expand(&hits);
        assert!(expand.contains(&0));
        assert!(expand.contains(&2));
        assert!(!expand.contains(&1));
    }
}

#[cfg(test)]
mod tool_group_tests {
    use super::*;
    use neo_protocol::EventMsg;

    fn tool(id: &str, ok: bool) -> Vec<EventMsg> {
        vec![
            EventMsg::ToolCallBegin {
                id: id.into(),
                name: "bash".into(),
                arguments: serde_json::json!({"cmd": "ls"}),
            },
            EventMsg::ToolCallEnd {
                id: id.into(),
                exit_code: if ok { 0 } else { 1 },
                stdout: String::new(),
                stderr: String::new(),
                truncated: false,
            },
        ]
    }

    /// 清屏必须连**按块下标记的**折叠状态一起清 —— 否则清屏后块下标从 0
    /// 重来，残留的下标会与新块撞上，某个组莫名其妙是展开的。
    #[test]
    fn clear_view_forgets_expanded_groups() {
        let mut t = Transcript::new();
        t.push_batch(&tool("a", true));
        t.push_batch(&tool("b", true));
        t.expanded_tool_runs.insert(0);
        t.collapsed_reasoning.insert(0);
        t.expanded_diff_folds.insert((0, 7));
        t.clear_view();
        assert!(t.expanded_tool_runs.is_empty(), "清屏后不应残留展开状态");
        assert!(t.collapsed_reasoning.is_empty(), "清屏后不应残留折叠状态");
        assert!(
            t.expanded_diff_folds.is_empty(),
            "清屏后不应残留 diff 折叠区的展开状态（同样按块下标记）"
        );
    }

    /// 三条 Begin 用**同一个 id**（provider 没给唯一 id）—— 界面上不能出现
    /// "永远执行中"的卡片。
    ///
    /// 这是从真机截图反推出来的：桩把同名调用都发成 `selftest-bash`，
    /// 于是第二、三次的 End 反复认领最后一张卡，前两张停在 ⏳。
    /// 配对改成"认领最早的未完成卡"后，每次 End 都能落到一张真正在等的卡上。
    #[test]
    fn duplicate_ids_each_claim_their_own_card() {
        let mut events = Vec::new();
        for _ in 0..3 {
            events.push(EventMsg::ToolCallBegin {
                id: "dup".into(),
                name: "bash".into(),
                arguments: serde_json::json!({"cmd": "ls"}),
            });
        }
        for _ in 0..3 {
            events.push(EventMsg::ToolCallEnd {
                id: "dup".into(),
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
                truncated: false,
            });
        }
        let t = transcript_with(events);
        let done = t
            .blocks
            .iter()
            .filter(|b| matches!(b, Block::Tool(c) if c.done))
            .count();
        assert_eq!(done, 3, "三条重复 id 的调用应各自收到一次 End");
    }

    fn transcript_with(events: Vec<EventMsg>) -> Transcript {
        let mut t = Transcript::new();
        t.push_batch(&events);
        t
    }

    #[test]
    fn consecutive_tool_calls_form_one_run() {
        let mut ev = Vec::new();
        for i in 0..4 {
            ev.extend(tool(&format!("c{i}"), true));
        }
        let t = transcript_with(ev);
        let runs = tool_runs(&t.blocks);
        assert_eq!(runs.len(), 1, "4 次连续调用应为 1 组：{:?}", t.blocks);
        assert_eq!(runs[0].tool_count(), 4);
    }

    /// **中间夹了别的块就断开** —— 那说明模型回到了对话，
    /// 前后不是"同一次工具爆发"。
    #[test]
    fn a_non_tool_block_splits_the_runs() {
        let mut ev = Vec::new();
        ev.extend(tool("c1", true));
        ev.extend(tool("c2", true));
        ev.push(EventMsg::AgentMessageDone { text: "做完了".into() });
        ev.extend(tool("c3", true));
        let t = transcript_with(ev);
        let runs = tool_runs(&t.blocks);
        assert_eq!(runs.len(), 2, "被正文断成两组：{:?}", t.blocks);
        assert_eq!(runs[0].tool_count(), 2);
        assert_eq!(runs[1].tool_count(), 1);
    }

    #[test]
    fn lone_tool_calls_are_their_own_runs() {
        let mut ev = Vec::new();
        ev.extend(tool("c1", true));
        ev.push(EventMsg::AgentMessageDone { text: "中间".into() });
        ev.extend(tool("c2", true));
        ev.push(EventMsg::AgentMessageDone { text: "再中间".into() });
        ev.extend(tool("c3", true));
        let t = transcript_with(ev);
        assert_eq!(tool_runs(&t.blocks).len(), 3, "三处孤立调用 = 三组");
    }

    #[test]
    fn no_tools_means_no_runs() {
        let t = transcript_with(vec![
            EventMsg::UserSubmitted { text: "问".into() },
            EventMsg::AgentMessageDone { text: "答".into() },
        ]);
        assert!(tool_runs(&t.blocks).is_empty());
    }

    /// 组标题的色调判据：**有失败就不能算成功**。
    ///
    /// 这条防的是"一组绿字里藏着一个失败" —— 信息层次里最容易骗人的一处。
    #[test]
    fn a_run_with_any_failure_is_not_reported_as_succeeded() {
        let mut ev = Vec::new();
        ev.extend(tool("c1", true));
        ev.extend(tool("c2", false)); // 一个失败
        ev.extend(tool("c3", true));
        let t = transcript_with(ev);
        let runs = tool_runs(&t.blocks);
        assert_eq!(runs.len(), 1);
        assert!(
            !run_all_succeeded(&t.blocks, &runs[0]),
            "含失败的组不能报成功：{:?}",
            t.blocks
        );
        assert!(!run_in_progress(&t.blocks, &runs[0]), "都结束了");
    }

    #[test]
    fn a_run_with_an_unfinished_tool_is_in_progress() {
        let t = transcript_with(vec![EventMsg::ToolCallBegin {
            id: "c1".into(),
            name: "bash".into(),
            arguments: serde_json::Value::Null,
        }]);
        let runs = tool_runs(&t.blocks);
        assert_eq!(runs.len(), 1);
        assert!(run_in_progress(&t.blocks, &runs[0]), "未完成的工具 → 组在执行中");
        assert!(!run_all_succeeded(&t.blocks, &runs[0]));
    }

    #[test]
    fn all_successful_run_is_reported_as_succeeded() {
        let mut ev = Vec::new();
        for i in 0..3 {
            ev.extend(tool(&format!("c{i}"), true));
        }
        let t = transcript_with(ev);
        let runs = tool_runs(&t.blocks);
        assert!(run_all_succeeded(&t.blocks, &runs[0]));
        assert!(!run_in_progress(&t.blocks, &runs[0]));
    }
}

