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
        t.clear_view();
        assert!(t.expanded_tool_runs.is_empty(), "清屏后不应残留展开状态");
        assert!(t.collapsed_reasoning.is_empty(), "清屏后不应残留折叠状态");
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

