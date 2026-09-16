//! 界面：事件流 → 显示块 → egui 绘制。
//!
//! # 数据全部来自事件流（零后端改动）
//!
//! 富界面所需的信息**早就在事件流里流着**，只是旧的内置页面把大部分扔掉了。
//! 本模块把 D2–D7 需要的都接上：
//!
//! | 显示 | 来源事件 |
//! |---|---|
//! | D2 Markdown 正文 | `AgentMessageDelta` / `AgentMessageDone` |
//! | D3 思考轨迹（可折叠） | `ReasoningDelta` ← **旧页面直接丢弃** |
//! | D4 工具卡片（含参数摘要） | `ToolCallBegin{name, arguments}` / `ToolCallEnd` |
//! | D5 diff 渲染 | `PatchProposed{path, diff}` ← **内核审批前已生成** |
//! | D6 轮摘要（token） | `TurnComplete` |
//! | D7 Goal 面板 | `GoalUpdated{snapshot}` / `GoalCleared` |
//! | D10 审批（阻塞 composer） | `ApprovalRequest` |
//!
//! # 结构：模型与绘制分开
//!
//! [`Transcript`] 是**纯数据**（吃事件、出块），不依赖 egui —— 因此可以在
//! 没有窗口的环境里测（CI 是 Linux 无显示环境，GUI 测试跑不起来）。
//! 绘制在 [`App`] 里，只读模型。这条分界是"界面逻辑可测"的前提。
//!
//! # 为什么 Markdown 交给 egui 布局
//!
//! 用 `neo_text::markdown::blocks()` 而不是 `render()`：后者面向终端、按等宽
//! 列折行；egui 有自己的文本布局。两者都折一遍会**折两次**（断点错位、缩进
//! 错乱）。`blocks()` 只做解析与语义分层，换行交给 egui —— 内容与色调两边
//! 由 `neo-text` 的测试保证一致。

use std::time::Duration;

use neo_protocol::{EventMsg, GoalSnapshot, TodoEntry, TodoStatus};
use neo_text::Tone;

use crate::driver::KernelHandle;
use crate::GuiPalette;

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
                // 按 id 配对（ToolCallEnd 只带 id，名字在 Begin 里）
                if let Some(card) = self.blocks.iter_mut().rev().find_map(|b| match b {
                    Block::Tool(c) if c.id == *id => Some(c),
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

    /// 记下"已为该快照请求推进"，避免重复下发。
    pub fn mark_advanced(&mut self) {
        if let Some(g) = &self.goal {
            self.advanced_for = Some((g.goal_id.clone(), g.iterations));
        }
    }
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
fn truncate_chars(s: &str, max: usize) -> String {
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

// ─────────────────────────── egui 界面 ───────────────────────────

/// GUI 应用状态。
pub struct App {
    handle: KernelHandle,
    pub transcript: Transcript,
    input: String,
    /// Goal 输入框（独立于任务输入：两者语义不同，共用会互相清空）。
    goal_input: String,
    palette: GuiPalette,
    /// 状态行文字（工作区/模型/模式）。
    status: String,
    /// 是否折叠思考块（全局开关，与逐块折叠叠加）
    show_reasoning: bool,
    /// 输入框是否曾经取得过焦点（首帧要抢一次）。
    composer_focused_once: bool,
    /// 上一帧是否处于"被审批阻塞"状态（用于检测"刚恢复可用"）。
    /// **仅测试用**：最后一帧的焦点 id（供无头跑帧断言）。
    ///
    /// 焦点存在 `Context` 的 memory 里，而 `draw` 只拿到 `Ui`；测试跑完帧后
    /// 从 ctx 读出来塞回这里，避免为了可测而改 `draw` 的签名。
    #[doc(hidden)]
    pub __test_last_focus: Option<egui::Id>,
    composer_blocked_last: bool,
    /// 启动时自动提交的任务（一次性）。
    ///
    /// 来源是 `NEO_GUI_PROMPT` 环境变量 —— 供**冒烟验证**用：把
    /// 「启动 + 提交 + 渲染」合成一次运行，验证者截图即可，不必手工点输入框
    /// （egui 自绘画布不通过 accessibility 暴露可写文本，自动化输入进不去）。
    /// 与 AGENTS.md 里 `PROTEUS_CODE_SMOKE` / `PROTEUS_CODE_QUERY` 的约定同源。
    auto_prompt: Option<String>,
}

impl App {
    pub fn new(handle: KernelHandle, status: String) -> Self {
        Self {
            handle,
            transcript: Transcript::new(),
            input: String::new(),
            goal_input: String::new(),
            palette: GuiPalette::neo(),
            status,
            show_reasoning: true,
            composer_focused_once: false,
            __test_last_focus: None,
            composer_blocked_last: false,
            auto_prompt: std::env::var("NEO_GUI_PROMPT").ok().filter(|s| !s.trim().is_empty()),
        }
    }

    /// 一帧的推进：收事件 → 更新模型 → （必要时）驱动 Goal。
    fn pump(&mut self) {
        let events = self.handle.drain();
        if !events.is_empty() {
            self.transcript.push_batch(&events);
        }
        // D7：Goal 由"看到最新快照的一方"驱动（与 Web 页面同一约定）
        if self.transcript.should_advance_goal() {
            self.transcript.mark_advanced();
            self.handle.send(neo_protocol::Op::GoalAdvance);
        }
    }

    /// 提交任务。**审批未决时不接受**（ZCode 语义：权限门暂停当前任务）。
    fn submit(&mut self) {
        let text = self.input.trim().to_string();
        if text.is_empty() || self.transcript.pending.is_some() {
            return;
        }
        self.input.clear();
        // 引用解析复用协议层共享实现（宿主不自造语义）；必须在 move 之前算
        let refs = neo_protocol::parse_refs(&text);
        // BeginTurn 而不是 UserTurn：后者一次跑完整轮（界面冻结）。
        // 推进由每帧的 Pump 完成。
        self.handle.send(neo_protocol::Op::BeginTurn { text, refs });
        self.transcript.running = true;
    }

    fn approve(&mut self, decision: neo_protocol::Decision) {
        let Some(p) = self.transcript.pending.take() else { return };
        // 逐帧宿主用 ApproveStep（Approve 会一次跑完剩余往返，界面又冻）
        self.handle.send(neo_protocol::Op::ApproveStep {
            id: p.id,
            decision,
            reason: None,
        });
        self.transcript.running = true;
    }

    fn set_goal(&mut self) {
        let text = self.goal_input.trim().to_string();
        if text.is_empty() {
            return;
        }
        self.handle.send(neo_protocol::Op::GoalSet { goal: text });
    }

    fn goal_action(&mut self, op: neo_protocol::Op) {
        self.handle.send(op);
    }

    fn draw_status(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("NEO")
                    .color(self.palette.color(Tone::Accent))
                    .strong(),
            );
            ui.label(
                egui::RichText::new(&self.status)
                    .color(self.palette.color(Tone::Muted))
                    .small(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (label, tone) = if self.transcript.pending.is_some() {
                    ("待审批", Tone::Warning)
                } else if self.transcript.running {
                    ("运行中", Tone::Info)
                } else {
                    ("就绪", Tone::Muted)
                };
                ui.label(egui::RichText::new(label).color(self.palette.color(tone)));
                ui.label(
                    egui::RichText::new(format!(
                        "{} in / {} out",
                        self.transcript.total_in, self.transcript.total_out
                    ))
                    .color(self.palette.color(Tone::Muted))
                    .small(),
                );
            });
        });
    }

    /// D7：右侧 Goal 面板。
    fn draw_goal_panel(&mut self, ui: &mut egui::Ui) {
        ui.label(
            egui::RichText::new("目标")
                .color(self.palette.color(Tone::Info))
                .strong(),
        );
        ui.add_space(4.0);
        match self.transcript.goal.clone() {
            None => {
                ui.label(
                    egui::RichText::new("未设定")
                        .color(self.palette.color(Tone::Muted))
                        .small(),
                );
                ui.add_space(6.0);
                ui.add(
                    egui::TextEdit::multiline(&mut self.goal_input)
                        .desired_rows(3)
                        .hint_text("每行一个子任务"),
                );
                if ui.button("开始").clicked() {
                    self.set_goal();
                }
            }
            Some(g) => {
                // 恒用协议层的 summary()：各宿主自己拼摘要必然漂移出
                // "同一目标在 TUI 与 GUI 长两副样子"
                ui.label(
                    egui::RichText::new(g.summary())
                        .color(self.palette.color(Tone::Accent)),
                );
                ui.add_space(4.0);
                for s in &g.subtasks {
                    let (mark, tone) = match s.phase {
                        neo_protocol::GoalPhase::Done => ("✓", Tone::Success),
                        _ => ("·", Tone::Muted),
                    };
                    ui.label(
                        egui::RichText::new(format!("{mark} {}", s.title))
                            .color(self.palette.color(tone)),
                    );
                }
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    if ui.button("暂停").clicked() {
                        self.goal_action(neo_protocol::Op::GoalPause { goal_id: g.goal_id.clone() });
                    }
                    if ui.button("恢复").clicked() {
                        self.goal_action(neo_protocol::Op::GoalResume { goal_id: g.goal_id.clone() });
                    }
                    if ui.button("清除").clicked() {
                        self.goal_action(neo_protocol::Op::GoalClear);
                    }
                });
            }
        }
    }

    /// D10：审批对话框 —— **模态**，且在未决时阻塞输入区。
    fn draw_approval(&mut self, ui: &mut egui::Ui) {
        let Some(p) = self.transcript.pending.clone() else { return };
        egui::Window::new("需要审批")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ui.ctx(), |ui| {
                ui.label(
                    egui::RichText::new(&p.detail)
                        .color(self.palette.color(Tone::Text)),
                );
                if !p.kind.is_empty() {
                    ui.label(
                        egui::RichText::new(format!("类别：{}", p.kind))
                            .color(self.palette.color(Tone::Muted))
                            .small(),
                    );
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .button(egui::RichText::new("允许").color(self.palette.color(Tone::Success)))
                        .clicked()
                    {
                        self.approve(neo_protocol::Decision::Allow);
                    }
                    if ui
                        .button("总是允许")
                        .on_hover_text(format!("对「{}」类别不再询问", p.kind))
                        .clicked()
                    {
                        self.approve(neo_protocol::Decision::AllowAlways);
                    }
                    if ui
                        .button(egui::RichText::new("拒绝").color(self.palette.color(Tone::Error)))
                        .clicked()
                    {
                        self.approve(neo_protocol::Decision::Deny);
                    }
                });
            });
    }

    fn draw_reasoning(&mut self, ui: &mut egui::Ui, idx: usize, text: &str) {
        let collapsed = !self.show_reasoning || self.transcript.collapsed_reasoning.contains(&idx);
        let arrow = if collapsed { "▸" } else { "▾" };
        let head = format!("{} 思考（{} 字）", arrow, text.chars().count());
        let resp = ui.add(
            egui::Label::new(
                egui::RichText::new(head)
                    .color(self.palette.color(Tone::Muted))
                    .small(),
            )
            .sense(egui::Sense::click()),
        );
        if resp.clicked() {
            if collapsed {
                self.transcript.collapsed_reasoning.remove(&idx);
            } else {
                self.transcript.collapsed_reasoning.insert(idx);
            }
        }
        if !collapsed {
            ui.label(
                egui::RichText::new(text)
                    .color(self.palette.color(Tone::Muted))
                    .monospace()
                    .small(),
            );
        }
    }

    /// D2：Markdown 正文 —— 用共享解析器出块，交给 egui 布局。
    fn draw_markdown(&self, ui: &mut egui::Ui, text: &str) {
        for line in neo_text::markdown::blocks(text) {
            if line.is_empty() {
                ui.add_space(6.0);
                continue;
            }
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                for (seg, tone) in line {
                    if seg.is_empty() {
                        continue;
                    }
                    // 前导空白（缩进）用等宽空间保留，否则层次会塌掉
                    let seg = seg.replace(' ', "\u{2002}");
                    ui.label(
                        egui::RichText::new(seg)
                            .color(self.palette.color(tone))
                            .monospace(),
                    );
                }
            });
        }
    }

    /// D5：diff 渲染 —— 按行着色。
    fn draw_diff(&self, ui: &mut egui::Ui, path: &str, diff: &str) {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("改动")
                    .color(self.palette.color(Tone::Info))
                    .strong(),
            );
            ui.label(
                egui::RichText::new(path)
                    .color(self.palette.color(Tone::Muted))
                    .monospace(),
            );
        });
        egui::ScrollArea::vertical()
            .id_salt(("diff", path))
            .max_height(320.0)
            .show(ui, |ui| {
                for line in diff.lines() {
                    let (tone, text) = match diff_line_kind(line) {
                        DiffLineKind::Header => (Tone::Muted, line.to_string()),
                        DiffLineKind::Hunk => (Tone::Info, line.to_string()),
                        DiffLineKind::Add => (Tone::Success, line.to_string()),
                        DiffLineKind::Del => (Tone::Error, line.to_string()),
                        DiffLineKind::Context => (Tone::Text, line.to_string()),
                        DiffLineKind::Meta => (Tone::Muted, line.to_string()),
                    };
                    ui.label(
                        egui::RichText::new(text)
                            .color(self.palette.color(tone))
                            .monospace(),
                    );
                }
            });
    }

    /// D4：工具卡片。
    fn draw_tool(&self, ui: &mut egui::Ui, card: &ToolCard) {
        let (state, tone) = if !card.done {
            ("执行中", Tone::Info)
        } else if card.exit_code == Some(0) {
            ("完成", Tone::Success)
        } else {
            ("失败", Tone::Error)
        };
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("▸ {}", card.name))
                    .color(self.palette.color(Tone::Primary))
                    .strong(),
            );
            ui.label(
                egui::RichText::new(state)
                    .color(self.palette.color(tone))
                    .small(),
            );
            if let Some(code) = card.exit_code {
                ui.label(
                    egui::RichText::new(format!("exit {code}"))
                        .color(self.palette.color(Tone::Muted))
                        .small(),
                );
            }
        });
        if !card.args.is_empty() {
            ui.label(
                egui::RichText::new(&card.args)
                    .color(self.palette.color(Tone::Muted))
                    .monospace()
                    .small(),
            );
        }
        // 输出**不丢**：这是"这步到底做了什么"的依据
        let body = if card.stderr.is_empty() { &card.stdout } else { &card.stderr };
        if !body.trim().is_empty() {
            egui::ScrollArea::vertical()
                .id_salt(("tool", &card.id))
                .max_height(200.0)
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(body)
                            .color(self.palette.color(if card.stderr.is_empty() {
                                Tone::Text
                            } else {
                                Tone::Error
                            }))
                            .monospace()
                            .small(),
                    );
                });
        }
        if card.truncated {
            ui.label(
                egui::RichText::new("（输出已截断）")
                    .color(self.palette.color(Tone::Warning))
                    .small(),
            );
        }
    }

    fn draw_transcript(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                // 借出后用 to_vec 避开"循环里改 self"的借用冲突；
                // 转录规模有上限（内核侧有限制），代价可接受。
                let blocks = std::mem::take(&mut self.transcript.blocks);
                for (i, b) in blocks.iter().enumerate() {
                    match b {
                        Block::User(t) => {
                            ui.label(
                                egui::RichText::new(format!("┃ {t}"))
                                    .color(self.palette.color(Tone::Text)),
                            );
                        }
                        Block::Assistant(t) => self.draw_markdown(ui, t),
                        Block::Reasoning(t) => self.draw_reasoning(ui, i, t),
                        Block::Tool(c) => self.draw_tool(ui, c),
                        Block::Diff { path, diff } => self.draw_diff(ui, path, diff),
                        Block::TurnSummary { input_tokens, output_tokens } => {
                            ui.label(
                                egui::RichText::new(format!(
                                    "· 本轮完成（{input_tokens} in / {output_tokens} out）"
                                ))
                                .color(self.palette.color(Tone::Muted))
                                .small(),
                            );
                        }
                        Block::Files(files) => {
                            for (p, add, del) in files {
                                ui.label(
                                    egui::RichText::new(format!("  {p} +{add} -{del}"))
                                        .color(self.palette.color(Tone::Muted))
                                        .small(),
                                );
                            }
                        }
                        Block::Todos(items) => {
                            for it in items {
                                let (mark, tone) = match it.status {
                                    TodoStatus::Completed => ("✓", Tone::Success),
                                    TodoStatus::InProgress => ("▸", Tone::Info),
                                    TodoStatus::Pending => ("·", Tone::Muted),
                                };
                                ui.label(
                                    egui::RichText::new(format!("{mark} {}", it.content))
                                        .color(self.palette.color(tone))
                                        .small(),
                                );
                            }
                        }
                        Block::Notice { text, tone } => {
                            ui.label(
                                egui::RichText::new(text)
                                    .color(self.palette.color(*tone)),
                            );
                        }
                    }
                    ui.add_space(2.0);
                }
                self.transcript.blocks = blocks;
            });
    }

    /// 底部输入区。审批未决时**阻塞**并说明原因。
    fn draw_composer(&mut self, ui: &mut egui::Ui) {
        let blocked = self.transcript.pending.is_some();
        let mut submit_now = false;
        ui.horizontal(|ui| {
            let hint = if blocked {
                "待审批：请先在上方对话框中选择（避免把下一步排进队列）"
            } else {
                "输入任务，回车提交（@文件 / $技能 可用）"
            };
            let edit = ui.add_enabled(
                !blocked,
                egui::TextEdit::singleline(&mut self.input)
                    .hint_text(hint)
                    // 稳定 id 是自动聚焦的前提：egui 靠 id 记住"谁有焦点"，
                    // 不给 id 时每帧可能生成不同的 id，聚焦会掉。
                    .id(egui::Id::new("neo_composer")),
            );

            // **自动聚焦输入框**：这是聊天式界面，打开就该能直接打字。
            // 不聚焦则用户要先点一下输入框 —— 纯键盘流里很别扭。
            // 时机：首帧；以及从"被阻塞"恢复时（审批刚答完，接着就该输入）。
            // 注意只在**恢复那一刻**抢一次焦点：每帧都抢会让用户没法把焦点
            // 移到别处（比如右侧目标输入框）。
            let just_unblocked = self.composer_blocked_last && !blocked;
            if !blocked && (!self.composer_focused_once || just_unblocked) {
                edit.request_focus();
                self.composer_focused_once = true;
            }
            self.composer_blocked_last = blocked;

            // 回车提交：egui 在单行 TextEdit 里按回车会 lost_focus
            if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                submit_now = true;
            }
            ui.add_enabled_ui(!blocked, |ui| {
                if ui.button("发送").clicked() {
                    submit_now = true;
                }
            });
        });
        if submit_now {
            self.submit();
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // 只做转发：真正的绘制在 `App::draw`。
        // 这样拆开的理由是**可测**：`eframe::Frame` 没有公开构造函数，
        // 若绘制逻辑长在 trait 方法里，测试就永远没法无头跑一帧 —— 而
        // "输入框有没有自动聚焦""回车会不会提交"这类问题恰恰只有跑帧才知道。
        self.draw(ui);
    }
}

impl App {
    /// 绘制一帧。**不依赖 `eframe::Frame`**，因此可在无窗口环境测试
    /// （`Context::run_ui` + 本方法即可，不需要显示器）。
    pub fn draw(&mut self, ui: &mut egui::Ui) {
        // 冒烟钩子：首帧把 NEO_GUI_PROMPT 的内容当作一次提交（只做一次）
        if let Some(text) = self.auto_prompt.take() {
            self.input = text;
            self.submit();
        }

        // 每帧推进：收事件（非阻塞）+ 推进一轮（Pump）
        self.pump();
        if self.transcript.running {
            // 驱动每帧发一个 Pump —— **这正是"能看见正在进行"的机制**
            self.handle.send(neo_protocol::Op::Pump);
        }
        // 动画节拍：即使没有事件也要继续出帧（spinner / 流式追加）
        if self.transcript.running || self.transcript.pending.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }

        egui::Panel::top("neo_status").show(ui, |ui| self.draw_status(ui));
        egui::Panel::right("neo_goal")
            .default_size(240.0)
            .show(ui, |ui| self.draw_goal_panel(ui));
        egui::Panel::bottom("neo_composer").show(ui, |ui| self.draw_composer(ui));

        egui::CentralPanel::default().show(ui, |ui| self.draw_transcript(ui));

        self.draw_approval(ui);

        // 回车提交已由 composer 的 lost_focus 路径处理（那里才知道输入框的
        // 真实状态）；这里不再做全局回车判断 —— 两处都判会导致一次回车提交两次。
    }
}

/// 打开窗口并运行到关闭。
///
/// `status` 是状态行文字（工作区 / 模式 / 模型），由调用方（CLI）装配。
pub fn run(handle: KernelHandle, title: &str, status: String) -> Result<(), String> {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1080.0, 720.0])
            .with_min_inner_size([640.0, 420.0])
            .with_title(title),
        ..Default::default()
    };
    eframe::run_native(
        title,
        opts,
        Box::new(move |cc| {
            crate::theme::install(&cc.egui_ctx);
            // ⚠️ 中文字体必须在界面出第一帧**之前**装上：egui 默认字体不含
            // CJK 字形，漏装的表现是"界面能开、中文全是豆腐块"（真机截图发现）。
            // 状态行只承载"工作区/模式/模型"；字体名进去会挤，故仅在**失败**时提示
            // （成功的路径不需要用户关心，失败却必须说 —— 否则是"能开但看不懂"）。
            let status = if crate::fonts::install(&cc.egui_ctx).is_some() {
                status
            } else {
                format!("{status} · ⚠ 未找到中文字体，中文可能显示为方块")
            };
            Ok(Box::new(App::new(handle, status)))
        }),
    )
    .map_err(|e| format!("创建窗口失败：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use neo_protocol::{GoalPhase, GoalSubtask};

    fn goal(id: &str, iterations: usize, remaining: usize) -> GoalSnapshot {
        GoalSnapshot {
            goal_id: id.into(),
            goal: "把 X 做完".into(),
            paused: false,
            stopped: None,
            subtasks: vec![GoalSubtask {
                id: 0,
                title: "第一步".into(),
                phase: GoalPhase::Plan,
                retries: 0,
            }],
            iterations,
            consecutive_failures: 0,
            turns_remaining: remaining,
            budget_used: 0,
        }
    }

    #[test]
    fn assistant_deltas_accumulate_into_one_block() {
        // 一个回复被画成几百段是真实的踩坑（TUI 的 pending_text 语义）
        let mut t = Transcript::new();
        t.push_batch(&[
            EventMsg::AgentMessageDelta { delta: "你".into() },
            EventMsg::AgentMessageDelta { delta: "好".into() },
            EventMsg::AgentMessageDelta { delta: "呀".into() },
        ]);
        assert_eq!(t.blocks.len(), 1, "同类增量必须并入同一块：{:?}", t.blocks);
        match &t.blocks[0] {
            Block::Assistant(s) => assert_eq!(s, "你好呀"),
            other => panic!("期望 Assistant，实际 {other:?}"),
        }
    }

    #[test]
    fn agent_message_done_overrides_the_deltas() {
        // Done 带完整正文，应以它为准（增量可能被截断）
        let mut t = Transcript::new();
        t.push_batch(&[
            EventMsg::AgentMessageDelta { delta: "半".into() },
            EventMsg::AgentMessageDone { text: "完整正文".into() },
        ]);
        match &t.blocks[0] {
            Block::Assistant(s) => assert_eq!(s, "完整正文"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn reasoning_is_kept_and_grouped() {
        // 旧页面把 ReasoningDelta 整个丢弃 —— 这里必须留下
        let mut t = Transcript::new();
        t.push_batch(&[
            EventMsg::ReasoningDelta { delta: "先".into() },
            EventMsg::ReasoningDelta { delta: "想".into() },
        ]);
        match &t.blocks[0] {
            Block::Reasoning(s) => assert_eq!(s, "先想"),
            other => panic!("思考轨迹必须保留：{other:?}"),
        }
    }

    #[test]
    fn tool_calls_pair_begin_with_end_by_id() {
        let mut t = Transcript::new();
        t.push_batch(&[
            EventMsg::ToolCallBegin {
                id: "c1".into(),
                name: "bash".into(),
                arguments: serde_json::json!({"command": "cargo test"}),
            },
            EventMsg::ToolCallEnd {
                id: "c1".into(),
                exit_code: 0,
                stdout: "ok".into(),
                stderr: String::new(),
                truncated: false,
            },
        ]);
        match &t.blocks[0] {
            Block::Tool(c) => {
                assert!(c.done, "结束事件应标完成");
                assert_eq!(c.exit_code, Some(0));
                assert_eq!(c.stdout, "ok");
                assert!(c.args.contains("cargo test"), "应含参数摘要：{}", c.args);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn tool_end_with_unknown_id_does_not_panic_or_invent_a_card() {
        // 事件可能乱序/丢失（截断的日志回放）—— 不能因此崩
        let mut t = Transcript::new();
        t.push_batch(&[EventMsg::ToolCallEnd {
            id: "ghost".into(),
            exit_code: 1,
            stdout: String::new(),
            stderr: "e".into(),
            truncated: false,
        }]);
        assert!(t.blocks.is_empty(), "没有对应 Begin 就不该造卡片：{:?}", t.blocks);
    }

    #[test]
    fn approval_sets_pending_and_stops_running() {
        let mut t = Transcript::new();
        t.push_batch(&[
            EventMsg::TurnStarted { turn_id: "t".into() },
            EventMsg::ApprovalRequest {
                id: "a1".into(),
                detail: "写文件".into(),
                kind: "write".into(),
            },
        ]);
        assert!(!t.running, "审批挂起时不是运行中（状态行要显示待审批）");
        let p = t.pending.as_ref().expect("应挂起审批");
        assert_eq!(p.id, "a1");
        assert_eq!(p.kind, "write", "类别用内核给的值，不按工具名推断");
    }

    #[test]
    fn files_changed_replaces_instead_of_appending() {
        // 内核发的是聚合表（覆盖语义）；逐条追加会让同一文件出现多次
        let mut t = Transcript::new();
        let f = |p: &str| neo_protocol::FileChange {
            path: p.into(),
            additions: 1,
            deletions: 0,
        };
        t.push_batch(&[EventMsg::FilesChanged { files: vec![f("a.rs")] }]);
        t.push_batch(&[EventMsg::FilesChanged { files: vec![f("a.rs"), f("b.rs")] }]);
        let files: Vec<&Block> = t.blocks.iter().filter(|b| matches!(b, Block::Files(_))).collect();
        assert_eq!(files.len(), 1, "聚合表应覆盖，不该追加成两块：{:?}", t.blocks);
        match files[0] {
            Block::Files(list) => assert_eq!(list.len(), 2, "应以最后一次为准"),
            _ => unreachable!(),
        }
    }

    #[test]
    fn turn_complete_accumulates_tokens_and_ends_the_turn() {
        let mut t = Transcript::new();
        t.push_batch(&[EventMsg::TurnStarted { turn_id: "t".into() }]);
        assert!(t.running);
        t.push_batch(&[EventMsg::TurnComplete { input_tokens: 10, output_tokens: 5 }]);
        assert!(!t.running);
        t.push_batch(&[EventMsg::TurnComplete { input_tokens: 1, output_tokens: 2 }]);
        assert_eq!((t.total_in, t.total_out), (11, 7), "token 应累计");
    }

    #[test]
    fn goal_snapshot_round_trips_and_clears() {
        let mut t = Transcript::new();
        t.push_batch(&[EventMsg::GoalUpdated { snapshot: goal("goal-1", 0, 2) }]);
        assert_eq!(t.goal.as_ref().map(|g| g.goal_id.clone()), Some("goal-1".into()));
        t.push_batch(&[EventMsg::GoalCleared { goal_id: "goal-1".into() }]);
        assert!(t.goal.is_none(), "清除后不该再有目标");
    }

    #[test]
    fn goal_advance_is_requested_once_per_snapshot() {
        // 每帧都会问一次 should_advance_goal；不去重就会反复下发 GoalAdvance
        let mut t = Transcript::new();
        t.push_batch(&[EventMsg::GoalUpdated { snapshot: goal("g", 0, 2) }]);
        assert!(t.should_advance_goal(), "有待执行子任务就该推进");
        t.mark_advanced();
        assert!(!t.should_advance_goal(), "同一快照不得重复推进");

        // 推进后快照前进（iterations+1）→ 允许下一次
        t.push_batch(&[EventMsg::GoalUpdated { snapshot: goal("g", 1, 1) }]);
        assert!(t.should_advance_goal());
    }

    #[test]
    fn goal_advance_stops_when_paused_stopped_or_finished() {
        let mut t = Transcript::new();
        let mut g = goal("g", 0, 1);
        g.paused = true;
        t.push_batch(&[EventMsg::GoalUpdated { snapshot: g }]);
        assert!(!t.should_advance_goal(), "暂停时不该推进");

        let mut t = Transcript::new();
        let mut g = goal("g", 0, 1);
        g.stopped = Some("预算用尽".into());
        t.push_batch(&[EventMsg::GoalUpdated { snapshot: g }]);
        assert!(!t.should_advance_goal(), "已停止不该推进");

        let mut t = Transcript::new();
        t.push_batch(&[EventMsg::GoalUpdated { snapshot: goal("g", 0, 0) }]);
        assert!(!t.should_advance_goal(), "没有剩余子任务不该推进");
    }

    #[test]
    fn error_ends_running_and_is_visible() {
        let mut t = Transcript::new();
        t.push_batch(&[
            EventMsg::TurnStarted { turn_id: "t".into() },
            EventMsg::Error { message: "网络断了".into() },
        ]);
        assert!(!t.running);
        assert!(
            t.blocks.iter().any(|b| matches!(b, Block::Notice { text, .. } if text == "网络断了")),
            "错误必须可见：{:?}",
            t.blocks
        );
    }

    #[test]
    fn summarize_args_prefers_informative_fields_and_stays_one_line() {
        let args = serde_json::json!({"command": "ls -la", "note": "x"});
        let s = summarize_args("bash", &args);
        assert!(s.contains("command=ls -la"), "{s}");

        // 多行内容只取首行 + 行数
        let args = serde_json::json!({"new": "line1\nline2\nline3"});
        let s = summarize_args("write", &args);
        assert!(!s.contains('\n'), "摘要必须是单行：{s}");
        assert!(s.contains("共 3 行"), "{s}");

        // 未知字段：至少列出键名
        let s = summarize_args("x", &serde_json::json!({"weird": 1}));
        assert!(s.contains("weird"), "{s}");

        // 非对象参数不崩
        assert_eq!(summarize_args("x", &serde_json::json!(null)), "");
    }

    #[test]
    fn summarize_args_truncates_huge_values_by_chars() {
        // 按字节截断会切碎中文（本项目踩过这个坑）
        let big = "字".repeat(500);
        let s = summarize_args("write", &serde_json::json!({"new": big}));
        assert!(s.chars().count() <= 200, "摘要要短：{} 字", s.chars().count());
        // 不能出现替换字符（说明切在了字符中间）
        assert!(!s.contains('\u{FFFD}'), "截断切碎了字符：{s}");
    }

    /// diff 行分类必须与**生产者**（`unified_diff`）的输出对得上。
    ///
    /// 这是跨模块往返：`neo-capability` 产出 diff 文本，本宿主流式着色。
    /// 分层方向 L5→L3 允许，所以能拿到真实生产者来测（不是自己造样例）。
    #[test]
    fn diff_line_kinds_match_what_the_capability_layer_produces() {
        use neo_capability::diff::unified_diff;
        let (text, _) = unified_diff(
            "fn a() {\n    let x = 1;\n}\n",
            "fn a() {\n    let x = 2;\n    log(x);\n}\n",
            "src/demo.rs",
        );
        let kinds: Vec<DiffLineKind> = text.lines().map(diff_line_kind).collect();
        assert_eq!(kinds[0], DiffLineKind::Header, "首行是 --- a/...");
        assert_eq!(kinds[1], DiffLineKind::Header, "次行是 +++ b/...");
        assert_eq!(kinds[2], DiffLineKind::Hunk, "第三行是 @@");
        assert!(
            kinds.contains(&DiffLineKind::Del) && kinds.contains(&DiffLineKind::Add),
            "应同时有删除与新增行：{kinds:?}"
        );
        assert!(kinds.contains(&DiffLineKind::Context), "应有上下文行");
        // 关键：`---`/`+++` 也是以 -/+ 开头，必须先判文件头，不能算成增删
        assert_ne!(kinds[0], DiffLineKind::Del);
        assert_ne!(kinds[1], DiffLineKind::Add);
    }

    /// 超限摘要路径的输出也要能被分类（它有中文说明行）。
    #[test]
    fn oversized_summary_lines_are_classified_as_meta_not_add_or_del() {
        use neo_capability::diff::unified_diff;
        let n = 2600; // 超过 MAX_ALIGN_LINES
        let old: String = (0..n).map(|i| format!("old {i}\n")).collect();
        let new: String = (0..n).map(|i| format!("new {i}\n")).collect();
        let (text, truncated) = unified_diff(&old, &new, "huge.txt");
        assert!(truncated);
        // 中文说明行不含前导 +/- 与空格 → Meta（不该被染成增删色）
        let metas = text
            .lines()
            .filter(|l| diff_line_kind(l) == DiffLineKind::Meta)
            .count();
        assert!(metas > 0, "摘要应有说明行：{}", &text[..text.len().min(200)]);
        // 且 @@ 行始终是 Hunk（供界面识别分段）
        assert_eq!(
            text.lines().filter(|l| l.starts_with("@@")).count(),
            1,
            "摘要应恰有一个 hunk 头"
        );
    }
    // ─────────── 无头跑帧：界面行为可测（不需要显示器） ───────────
    //
    // 这一组解决的是个真实困境：egui 是自绘画布，不通过 accessibility 暴露
    // 可写文本，自动化工具**没法往输入框里打字**（真机验证时 set_value / type
    // 都报 target_verification_status: mismatched）。既然"点一下再打字"这条路
    // 走不通，就把"输入框该不该有焦点""回车会不会提交"变成**可断言的行为**，
    // 用 run_ui 跑真帧来验。

    /// 造一个不接内核的 App（渲染测试不需要真内核）。
    ///
    /// 保留 rx/tx 不 drop：通道若两端都关闭，`drain()` 会塞一条 Error 事件，
    /// 干扰渲染断言。
    fn render_test_app() -> (App, std::sync::mpsc::Receiver<crate::driver::DriverMsg>,
                             std::sync::mpsc::Sender<Vec<EventMsg>>) {
        let (handle, rx, tx) = crate::driver::channel();
        (App::new(handle, "测试状态".into()), rx, tx)
    }

    fn raw_input() -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 600.0),
            )),
            ..Default::default()
        }
    }

    /// 跑一帧。**Context 必须跨帧复用**（焦点存在它的 memory 里）。
    ///
    /// 第一版这里每帧新建 Context，于是"上一帧有焦点、这一帧回车触发
    /// lost_focus"永远不成立 —— 测试失败怪到了产品头上，实际是测试台错了。
    /// 真实应用只有一个 Context，测试必须照做。
    fn run_frame(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) {
        let mut input = raw_input();
        input.events = events;
        let mut out = ctx.run_ui(input, |ui| app.draw(ui));
        out.textures_delta.clear();
        // 焦点状态记在 ctx 的 memory 里；带出来供断言
        app.__test_last_focus = ctx.memory(|m| m.focused());
    }

    fn new_ctx() -> egui::Context {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        ctx
    }

    /// **输入框必须自动聚焦** —— 否则用户打开窗口后得先点一下才能打字。
    ///
    /// 这条在真机验证时是**卡住验收**的那个问题：辅助功能已授权，但 egui
    /// 画布不接受注入文本，唯一的活路是"先用真键盘打字"—— 而真键盘打字
    /// 需要输入框已有焦点。加了自动聚焦后，真机 `keystroke` 一次就把文字送
    /// 进去了（日志里看到 `begin_turn` 带着输入的文本）。
    #[test]
    fn composer_takes_focus_on_the_first_frame() {
        let (mut app, _rx, _tx) = render_test_app();
        let ctx = new_ctx();
        run_frame(&ctx, &mut app, vec![]);
        assert_eq!(
            app.__test_last_focus,
            Some(egui::Id::new("neo_composer")),
            "首帧应把焦点交给输入框（打开就能打字）"
        );
    }

    /// 回车提交：单行输入框按回车会 `lost_focus`，由 composer 捕获并提交。
    ///
    /// 真机验证过（osascript `key code 36` → 会话日志出现 `begin_turn`），
    /// 这里把它钉在 CI 上。注意提交后输入框要清空。
    #[test]
    fn pressing_enter_submits_the_composer_text() {
        let (mut app, _rx, _tx) = render_test_app();
        let ctx = new_ctx();
        app.input = "列出文件".into();
        // 先跑一帧拿到焦点（真实交互顺序：聚焦 → 打字 → 回车）
        run_frame(&ctx, &mut app, vec![]);
        // 回车：egui 把它作为事件送进来
        run_frame(
            &ctx,
            &mut app,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::default(),
            }],
        );
        assert!(app.input.is_empty(), "提交后输入框应清空：{:?}", app.input);
    }

    /// 审批未决时输入框**不可用**（ZCode 语义：权限门暂停当前任务）。
    #[test]
    fn composer_is_disabled_while_an_approval_is_pending() {
        let (mut app, _rx, _tx) = render_test_app();
        app.transcript.pending = Some(Pending {
            id: "a1".into(),
            detail: "写文件".into(),
            kind: "write".into(),
        });
        let ctx = new_ctx();
        run_frame(&ctx, &mut app, vec![]);
        // 阻塞时不该抢焦点（焦点应留给审批对话框）
        assert_ne!(
            app.__test_last_focus,
            Some(egui::Id::new("neo_composer")),
            "待审批时输入框不应获得焦点"
        );
        // 且此时提交是空操作（不会被排队）
        app.input = "下一步".into();
        app.submit();
        assert!(!app.input.is_empty(), "待审批时提交应被忽略，不排进队列");
    }

    /// 跑帧不该 panic（覆盖空转录、有内容、审批挂起、Goal 面板四种状态）。
    #[test]
    fn drawing_every_panel_state_does_not_panic() {
        // 每种状态单独跑一帧；用独立的 App 避免状态互相影响
        let cases: Vec<Box<dyn Fn(&mut App)>> = vec![
            Box::new(|_a| {}),
            Box::new(|a| {
                a.transcript.push_batch(&[
                    EventMsg::UserSubmitted { text: "问".into() },
                    EventMsg::AgentMessageDelta { delta: "**答**\n\n- 项".into() },
                    EventMsg::ReasoningDelta { delta: "想".into() },
                    EventMsg::ToolCallBegin {
                        id: "c".into(),
                        name: "bash".into(),
                        arguments: serde_json::json!({"cmd": "ls"}),
                    },
                    EventMsg::PatchProposed {
                        path: "a.rs".into(),
                        diff: "@@ -1 +1 @@\n-a\n+b".into(),
                    },
                    EventMsg::TurnComplete { input_tokens: 1, output_tokens: 2 },
                ]);
            }),
            Box::new(|a| {
                a.transcript.pending = Some(Pending {
                    id: "a".into(),
                    detail: "d".into(),
                    kind: "write".into(),
                });
            }),
            Box::new(|a| {
                a.transcript.push_batch(&[EventMsg::GoalUpdated { snapshot: goal("g1", 1, 2) }]);
            }),
        ];
        for setup in cases {
            let (mut app, _rx, _tx) = render_test_app();
            let ctx = new_ctx();
            setup(&mut app);
            run_frame(&ctx, &mut app, vec![]);
        }
    }
}
