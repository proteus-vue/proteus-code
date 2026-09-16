//! 窗口与视图：把事件流画成界面。
//!
//! # 结构（照 egui 版那条分界，但绘制层换成了 gpui）
//!
//! - **数据**：`neo_driver::transcript::Transcript`（共享，不依赖任何 GUI 库，
//!   可在无窗口环境单测）
//! - **绘制**：本模块。它是全仓唯一用 gpui 画 NEO 界面的地方。
//!
//! # 两处必须照做的细节
//!
//! 1. **窗口首层必须是 `Root`**：否则 `open_dialog` / 通知 / tooltip 会 panic
//!    或静默失效（gpui-component 的 `Root::update` 有 `expect`）。
//! 2. **响应式重绘靠唤醒钩子**：gpui 不出帧就不重绘，所以事件到达时必须
//!    `cx.notify()` —— 否则表现为"模型答完了，屏幕上什么都没变"。

use std::sync::Arc;

use neo_driver::transcript::{mode_label, next_mode, Block, Transcript};
use neo_driver::KernelHandle;
use neo_protocol::{Decision, EventMsg, ExecMode, Op};
use neo_text::Tone;
use neo_ui::neo_color;
use neo_ui_kit::component::{
    button::Button, h_flex, v_flex, Root,
};
use neo_ui_kit::component::scroll::ScrollableElement as _;
use neo_ui_kit::gpui::{div, prelude::*, px, Context, Entity, IntoElement, Render, Window};

/// 界面状态（Entity）。
pub struct NeoView {
    handle: KernelHandle,
    transcript: Transcript,
    input: String,
    mode: ExecMode,
    model: String,
    // ⚠️ **待审批不在这里单独存**：`Transcript` 已经有 `pending`
    //（它消费 `ApprovalRequest` 事件时设置）。
    //
    // 曾经这里另存了一份 `PendingApproval`，结果是同一件事有了两处来源：
    // 转录模型按事件设它自己的，视图读自己那份 —— 修一处忘一处就出问题。
    // 共享模型里已经有的状态，宿主不该再存一遍。
    /// 状态行提示
    notice: Option<String>,
    /// 工作区/模式说明
    status: String,
    /// 命令面板是否打开（D9）。
    cmd_open: bool,
    /// 命令面板的搜索串。
    cmd_query: String,
    /// 命令面板的高亮项（过滤后列表的下标）。
    cmd_selected: usize,
    /// 命令台的输入串（D8；与任务输入框是**两条独立通道**）。
    terminal_input: String,
    /// 命令台是否展开（D8）。
    terminal_open: bool,
    /// 会话控制（D1）。`None` = 本装配未接会话库，侧栏如实说明。
    sessions: Option<Box<dyn neo_session::SessionControl>>,
    /// 可选模型（启动时从装配层取；`ShowModels` 在它们之间循环）。
    models: Vec<String>,
    /// 是否折叠思考轨迹（D3）。
    show_reasoning: bool,
    /// 启动时自动提交的任务（一次性）。来源 `NEO_GUI_PROMPT`。
    ///
    /// 与 egui 宿主的同名钩子同源（AGENTS.md 里 `PROTEUS_CODE_SMOKE` 的约定）：
    /// 把「启动 + 提交 + 渲染」合成一次运行，让验证者截图即可。
    /// **gpui 与 egui 的画布都无法靠自动化工具输入文本**，所以这条钩子是
    /// 端到端验证唯一可脚本化的路径。
    auto_prompt: Option<String>,
}

impl NeoView {
    #[allow(clippy::too_many_arguments)]
    fn new(
        handle: KernelHandle,
        sessions: Option<Box<dyn neo_session::SessionControl>>,
        models: Vec<String>,
        status: String,
        mode: ExecMode,
        model: String,
    ) -> Self {
        Self {
            handle,
            sessions,
            models,
            cmd_query: String::new(),
            cmd_selected: 0,
            terminal_input: String::new(),
            show_reasoning: true,
            transcript: Transcript::new(),
            input: String::new(),
            mode,
            model,
            notice: None,
            status,
            // 调试开关：启动即展开某个面板，便于脚本化截图验证
            // （与 NEO_GUI_PROMPT 同一理由：画布无法靠自动化工具输入，
            //   面板的可见性只能由环境变量驱动）
            cmd_open: std::env::var("NEO_GUI_PANEL").ok().as_deref() == Some("cmd"),
            terminal_open: std::env::var("NEO_GUI_PANEL").ok().as_deref() == Some("terminal"),
            auto_prompt: std::env::var("NEO_GUI_PROMPT")
                .ok()
                .filter(|s| !s.trim().is_empty()),
        }
    }

    /// 取回已到达的事件并更新模型。返回是否有变化（决定要不要重绘）。
    fn pump(&mut self) -> bool {
        let events = self.handle.drain();
        if events.is_empty() {
            return false;
        }
        for ev in &events {
            // 只有**宿主自己的**派生状态在这里处理。
            // 事件到"待审批/运行中/目标/转录块"的映射由 `Transcript` 负责
            //（它是共享的，两个 GUI 宿主必须用同一套语义）。
            if let EventMsg::ModelSwitched { model, .. } = ev {
                self.model = model.clone();
            }
        }
        self.transcript.push_batch(&events);
        true
    }

    fn submit(&mut self) {
        let text = self.input.trim().to_string();
        // 审批未决时不接受新任务（避免把下一步排进队列）
        if text.is_empty() || self.transcript.pending.is_some() {
            return;
        }
        self.input.clear();
        let refs = neo_protocol::parse_refs(&text);
        // BeginTurn 而不是 UserTurn：后者一次跑完整轮（界面会卡住）。
        // 推进由每帧的 Pump 完成。
        self.handle.send(Op::BeginTurn { text, refs });
        self.transcript.running = true;
    }

    /// 逐帧推进一轮里的**一步**。
    fn pump_step(&self) {
        self.handle.send(Op::Pump);
    }

    fn approve(&mut self, decision: Decision) {
        let Some(p) = self.transcript.pending.take() else {
            return;
        };
        // `ApproveStep` 而不是 `Approve`：后者会一次跑完剩余往返（界面又冻）
        self.handle.send(Op::ApproveStep {
            id: p.id,
            decision,
            reason: None,
        });
        self.transcript.running = true;
    }

    /// **D1**：切换会话。内核换会话后返回**历史事件流**，用它**替换**转录 ——
    /// 与接收实时事件走同一条渲染路径（`push_batch`），不另写"重画历史"。
    fn switch_session(&mut self, id: String) {
        let Some(sessions) = self.sessions.as_mut() else {
            self.notice = Some("本装配未接会话库，无法切换会话".into());
            return;
        };
        match sessions.switch(&id) {
            Ok(history) => {
                // 换会话 = 换上下文：转录与累计 token 归零，
                // 否则上一条会话的数字会算到这一条上
                self.transcript = Transcript::new();
                self.transcript.push_batch(&history);
                self.notice = Some(format!("已切换到会话 {id}"));
            }
            Err(e) => self.notice = Some(format!("切换失败：{e}")),
        }
    }

    /// **D1**：新建会话（旧会话留在磁盘上，可再切回）。
    fn new_session(&mut self) {
        let Some(sessions) = self.sessions.as_mut() else {
            self.notice = Some("本装配未接会话库，无法新建会话".into());
            return;
        };
        match sessions.create() {
            Ok(id) => {
                self.transcript = Transcript::new();
                self.notice = Some(format!("已新建会话 {id}（旧会话保留）"));
            }
            Err(e) => self.notice = Some(format!("新建失败：{e}")),
        }
    }

    /// **D8**：执行一条用户直输的命令（**不经模型**，走沙箱）。
    fn run_terminal_command(&mut self) {
        let cmd = self.terminal_input.trim().to_string();
        if cmd.is_empty() {
            return;
        }
        self.terminal_input.clear();
        self.handle.send(Op::Shell { command: cmd });
    }

    /// **D9**：执行命令面板选中的命令。
    ///
    /// 每个动作都**真的做点什么** —— 列着却点了没反应比没有更糟。
    fn run_action(&mut self, action: neo_driver::commands::Action) {
        use neo_driver::commands::Action as A;
        match action {
            A::Compact => {
                self.handle.send(Op::Compact);
                self.notice = Some("正在压缩上下文…".into());
            }
            A::Rewind => {
                self.handle.send(Op::Rewind { turns: 1 });
                self.notice = Some("已请求回退一轮".into());
            }
            A::Interrupt => {
                self.handle.send(Op::Interrupt);
                self.notice = Some("已请求打断".into());
            }
            A::ShowModels => {
                self.model = next_model(&self.model, &self.models);
                let m = self.model.clone();
                self.handle.send(Op::ConfigureSession {
                    patch: neo_protocol::SessionPatch {
                        model: Some(m.clone()),
                        ..Default::default()
                    },
                });
                self.notice = Some(format!("已切换到 {m}"));
            }
            A::CycleMode => self.cycle_mode(),
            A::ToggleReasoning => {
                self.show_reasoning = !self.show_reasoning;
                let st = if self.show_reasoning { "展开" } else { "折叠" };
                self.notice = Some(format!("思考轨迹已{st}"));
            }
            // 命令台（D8）：开面板并聚焦它的输入框
            A::ToggleTerminal => {
                self.terminal_open = !self.terminal_open;
                let st = if self.terminal_open { "显示" } else { "隐藏" };
                self.notice = Some(format!("命令台已{st}"));
            }
            // 会话栏（D1）：无独立面板，用"新建/切换"表达
            A::ToggleSidebar => {
                self.notice = Some("会话列表见右侧面板（本宿主未做左侧栏折叠）".into());
            }
            A::NewSession => self.new_session(),
            // 清屏：只清**屏幕上的**转录，不动会话日志
            A::ClearTranscript => {
                self.transcript.clear_view();
                self.notice = Some("已清空屏幕转录（会话日志保留）".into());
            }
            A::Help => {
                self.notice = Some("快捷键：Shift+Tab 切模式 · Cmd+K 命令面板".into());
            }
            A::Quit => {
                self.handle.send(Op::Shutdown);
                self.notice = Some("已请求退出（关闭窗口即可）".into());
            }
        }
    }

    /// 开/关命令面板（`Cmd+K`）。开关式：再按一次关闭。
    fn toggle_cmd(&mut self, cx: &mut Context<Self>) {
        self.cmd_open = !self.cmd_open;
        // 每次打开都清空筛选：残留的搜索串会让下次打开"看起来少了命令"，
        // 而原因（上次输了字）在界面上已经看不见了
        self.cmd_query.clear();
        self.cmd_selected = 0;
        cx.notify();
    }

    /// 命令面板打开时的按键（`↑↓` 选择、`Enter` 执行）。
    fn cmd_key(&mut self, key: &str, cx: &mut Context<Self>) {
        match key {
            "up" => self.cmd_move(false),
            "down" => self.cmd_move(true),
            _ => {
                if let Some(c) = self.cmd_matches().get(self.cmd_selected).copied() {
                    self.run_action(c.action.resolve());
                }
                self.cmd_open = false;
                self.cmd_query.clear();
                self.cmd_selected = 0;
            }
        }
        cx.notify();
    }

    /// 命令面板：按搜索串过滤后的命令。
    fn cmd_matches(&self) -> Vec<&'static neo_driver::commands::Command> {
        neo_driver::commands::filter(&self.cmd_query)
    }

    fn cmd_move(&mut self, down: bool) {
        let n = self.cmd_matches().len();
        if n == 0 {
            self.cmd_selected = 0;
            return;
        }
        if down {
            self.cmd_selected = (self.cmd_selected + 1) % n;
        } else {
            self.cmd_selected = (self.cmd_selected + n - 1) % n;
        }
    }

    fn cycle_mode(&mut self) {
        let next = next_mode(self.mode);
        self.handle.send(Op::ConfigureSession {
            patch: neo_protocol::SessionPatch {
                exec_mode: Some(next),
                ..Default::default()
            },
        });
        self.mode = next;
        // 模式变化**没有内核事件**，宿主必须自记 —— 否则状态行显示旧档位
        self.notice = Some(format!("已切换到 {} 模式", mode_label(next)));
    }
}

/// 在模型列表里循环到下一个（列表空或只有一个时原样返回）。
///
/// 抽成自由函数便于单测：循环逻辑写错会表现为"切了没反应"或"跳到不存在的模型"。
fn next_model(current: &str, models: &[String]) -> String {
    if models.len() <= 1 {
        return current.to_string();
    }
    match models.iter().position(|m| m == current) {
        Some(i) => models[(i + 1) % models.len()].clone(),
        // 当前模型不在列表里（比如注册表变了）：回到第一个
        None => models[0].clone(),
    }
}

/// 把一行带色调的片段渲染成一个元素（复用 `neo-text` 的 Markdown 解析）。
///
/// 用 `StyledText::with_highlights` 而不是逐片段 `div().child()`：
/// 前者把它当**一行文字**参与排版（换行、基线正确），后者是并排的盒子，
/// 中英混排时会各占各的宽度、断行位置全错。
fn styled_line(spans: &[(String, Tone)]) -> impl IntoElement {
    let text: String = spans.iter().map(|(t, _)| t.as_str()).collect();
    let mut highlights = Vec::new();
    let mut offset = 0usize;
    for (seg, tone) in spans {
        let len = seg.len();
        if len > 0 {
            highlights.push((
                offset..offset + len,
                neo_ui_kit::gpui::HighlightStyle {
                    color: Some(neo_color(*tone).into()),
                    ..Default::default()
                },
            ));
        }
        offset += len;
    }
    div().child(
        neo_ui_kit::gpui::StyledText::new(text).with_highlights(highlights),
    )
}

/// 转录区：把 `Block` 画出来。
fn transcript_view(blocks: &[Block]) -> impl IntoElement {
    let mut col = v_flex().gap_1().p_3();
    for b in blocks {
        match b {
            Block::User(t) => {
                col = col.child(div().text_color(neo_color(Tone::Accent)).child(format!("┃ {t}")));
            }
            Block::Assistant(t) => {
                // Markdown：走共享解析器出块，逐行渲染（换行交给布局引擎）
                for line in neo_text::markdown::blocks(t) {
                    col = col.child(styled_line(&line));
                }
            }
            Block::Reasoning(t) => {
                col = col.child(
                    div()
                        .text_color(neo_color(Tone::Muted))
                        .child(format!("▾ 思考（{} 字）：{t}", t.chars().count())),
                );
            }
            Block::Tool(c) => {
                let state = if !c.done {
                    "执行中"
                } else if c.exit_code == Some(0) {
                    "完成"
                } else {
                    "失败"
                };
                let tone = if !c.done {
                    Tone::Info
                } else if c.exit_code == Some(0) {
                    Tone::Success
                } else {
                    Tone::Error
                };
                col = col.child(h_flex().gap_2().child(
                    div()
                        .text_color(neo_color(Tone::Primary))
                        .child(format!("▸ {}", c.name)),
                ).child(div().text_color(neo_color(tone)).child(state)));
                if !c.args.is_empty() {
                    col = col.child(
                        div().text_color(neo_color(Tone::Muted)).child(c.args.clone()),
                    );
                }
                let body = if c.stderr.is_empty() { &c.stdout } else { &c.stderr };
                if !body.trim().is_empty() {
                    col = col.child(
                        div()
                            .text_color(neo_color(if c.stderr.is_empty() {
                                Tone::Text
                            } else {
                                Tone::Error
                            }))
                            .child(body.clone()),
                    );
                }
                if c.truncated {
                    col = col.child(
                        div()
                            .text_color(neo_color(Tone::Warning))
                            .child("（输出已截断）"),
                    );
                }
            }
            Block::Diff { path, diff } => {
                col = col.child(
                    div()
                        .text_color(neo_color(Tone::Info))
                        .child(format!("改动 {path}")),
                );
                for line in diff.lines() {
                    let tone = match neo_driver::transcript::diff_line_kind(line) {
                        neo_driver::transcript::DiffLineKind::Add => Tone::Success,
                        neo_driver::transcript::DiffLineKind::Del => Tone::Error,
                        neo_driver::transcript::DiffLineKind::Hunk => Tone::Info,
                        neo_driver::transcript::DiffLineKind::Meta => Tone::Muted,
                        _ => Tone::Text,
                    };
                    col = col.child(div().text_color(neo_color(tone)).child(line.to_string()));
                }
            }
            Block::TurnSummary { input_tokens, output_tokens } => {
                col = col.child(
                    div()
                        .text_color(neo_color(Tone::Muted))
                        .child(format!("· 本轮完成（{input_tokens} in / {output_tokens} out）")),
                );
            }
            Block::Files(files) => {
                for (p, add, del) in files {
                    col = col.child(
                        div()
                            .text_color(neo_color(Tone::Muted))
                            .child(format!("  {p} +{add} -{del}")),
                    );
                }
            }
            Block::Todos(items) => {
                for it in items {
                    let (mark, tone) = match it.status {
                        neo_protocol::TodoStatus::Completed => ("✓", Tone::Success),
                        neo_protocol::TodoStatus::InProgress => ("▸", Tone::Info),
                        neo_protocol::TodoStatus::Pending => ("·", Tone::Muted),
                    };
                    col = col.child(
                        div()
                            .text_color(neo_color(tone))
                            .child(format!("{mark} {}", it.content)),
                    );
                }
            }
            Block::Notice { text, tone } => {
                col = col.child(div().text_color(neo_color(*tone)).child(text.clone()));
            }
        }
    }
    col
}

/// **D7**：右侧面板 —— 目标 + 会话列表（D1）。
///
/// 两个面板合并成一栏是刻意的：窗口宽度有限，而二者都是"当前上下文"的展示。
/// ZCode 把它们分在左右两侧（各占 240px），那在宽屏上才成立。
fn side_panel(view: &NeoView, cx: &mut Context<NeoView>) -> impl IntoElement {
    let mut col = v_flex()
        .w(px(220.))
        .h_full()
        .gap_2()
        .p_3()
        .child(
            div()
                .text_color(neo_color(Tone::Info))
                .child("目标"),
        );

    match &view.transcript.goal {
        None => {
            col = col.child(
                div()
                    .text_color(neo_color(Tone::Muted))
                    .child("未设定（用 /goal 设定）"),
            );
        }
        Some(g) => {
            // 恒用协议层的 summary()：各宿主自己拼会漂移出"同一个目标长两副样子"
            col = col.child(
                div()
                    .text_color(neo_color(Tone::Accent))
                    .child(g.summary()),
            );
            for s in &g.subtasks {
                let (mark, tone) = match s.phase {
                    neo_protocol::GoalPhase::Done => ("✓", Tone::Success),
                    _ => ("·", Tone::Muted),
                };
                col = col.child(
                    div()
                        .text_color(neo_color(tone))
                        .child(format!("{mark} {}", s.title)),
                );
            }
        }
    }

    // ── D1 会话列表 ──
    col = col.child(
        div()
            .pt_2()
            .text_color(neo_color(Tone::Info))
            .child("会话"),
    );

    let Some(sessions) = view.sessions.as_ref() else {
        return col.child(
            div()
                .text_color(neo_color(Tone::Muted))
                .child("（未接会话库）"),
        );
    };

    let current = sessions.current();
    let mut list = sessions.list();
    // ⚠️ 当前会话必须在列表里，哪怕它还没有文件：
    // `new_id()` 刻意不建文件（首次写入才惰性创建），于是刚点过"新建"的会话
    // 不在 list() 里 —— 用户看不到也点不到自己刚建的那个（egui 版真机踩过）。
    if !current.is_empty() && !list.iter().any(|(id, _, _)| *id == current) {
        list.insert(0, (current.clone(), String::new(), 0));
    }
    if list.is_empty() {
        col = col.child(
            div()
                .text_color(neo_color(Tone::Muted))
                .child("暂无会话"),
        );
    }
    for (id, title, records) in list {
        let is_current = id == current;
        // 标题为空（新会话还没起名）用 id 兜底，否则列表里出现空白行
        let shown = if title.trim().is_empty() { id.clone() } else { title.clone() };
        let click_id = id.clone();
        let v = cx.entity().clone();
        col = col.child(
            div()
                .id(format!("sess-{id}"))
                .text_color(neo_color(if is_current {
                    Tone::Accent
                } else {
                    Tone::Text
                }))
                .child(format!("{shown} ·{records}"))
                .on_click(move |_, _, cx| {
                    if !is_current {
                        let id = click_id.clone();
                        v.update(cx, |this, _| this.switch_session(id));
                    }
                }),
        );
    }

    // 新建按钮
    let v = cx.entity().clone();
    col.child(
        Button::new("new-session")
            .label("+ 新建会话")
            .on_click(move |_, _, cx| {
                v.update(cx, |this, _| this.new_session());
            }),
    )
}

/// **D9**：命令面板（覆盖式）。返回 None 表示未打开。
fn command_palette(view: &NeoView, cx: &mut Context<NeoView>) -> Option<impl IntoElement> {
    if !view.cmd_open {
        return None;
    }
    let matches = view.cmd_matches();
    let mut col = v_flex()
        .w(px(420.))
        .gap_1()
        .p_3()
        .bg(neo_ui::panel_bg())
        .child(
            div()
                .text_color(neo_color(Tone::Text))
                .child(if view.cmd_query.is_empty() {
                    "输入以筛选（可搜命令名或说明）".to_string()
                } else {
                    format!("筛选：{}", view.cmd_query)
                }),
        );

    if matches.is_empty() {
        col = col.child(
            div()
                .text_color(neo_color(Tone::Muted))
                .child("没有匹配的命令"),
        );
    }
    for (i, c) in matches.iter().enumerate() {
        let sel = i == view.cmd_selected;
        let action = c.action.resolve();
        let v = cx.entity().clone();
        col = col.child(
            div()
                .id(format!("cmd-{}", c.name))
                .text_color(neo_color(if sel { Tone::Accent } else { Tone::Text }))
                .child(format!("/{}  {}", c.name, c.desc))
                .on_click(move |_, _, cx| {
                    let a = action.clone();
                    v.update(cx, |this, _| {
                        this.run_action(a);
                        this.cmd_open = false;
                        this.cmd_query.clear();
                        this.cmd_selected = 0;
                    });
                }),
        );
    }
    col = col.child(
        div()
            .text_color(neo_color(Tone::Muted))
            .child("↑↓ 选择 · Enter 执行 · Esc 关闭"),
    );
    Some(col)
}

/// **D8**：命令台（底部面板）。
fn terminal_panel(view: &NeoView, cx: &mut Context<NeoView>) -> Option<impl IntoElement> {
    if !view.terminal_open {
        return None;
    }
    // 输出复用转录里的工具卡片（`Op::Shell` 产出同一对 ToolCall 事件）——
    // 不另存一份终端历史，两份必然漂移（清屏时一份清了一份没清）。
    let cards: Vec<neo_driver::transcript::ToolCard> = view
        .transcript
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Tool(c) => Some(c.clone()),
            _ => None,
        })
        .collect();

    let mut col = v_flex()
        .h(px(160.))
        .gap_1()
        .p_3()
        .child(
            h_flex()
                .gap_2()
                .child(
                    div()
                        .text_color(neo_color(Tone::Info))
                        .child("命令台"),
                )
                .child(
                    div()
                        .text_color(neo_color(Tone::Muted))
                        .child("（每条命令一个进程，走沙箱；不是交互式终端）"),
                ),
        );
    if cards.is_empty() {
        col = col.child(
            div()
                .text_color(neo_color(Tone::Muted))
                .child("还没有执行过命令"),
        );
    }
    for c in cards.iter().rev().take(3) {
        let state = if !c.done {
            "执行中"
        } else if c.exit_code == Some(0) {
            "完成"
        } else {
            "失败"
        };
        col = col.child(
            div()
                .text_color(neo_color(Tone::Muted))
                .child(format!("▸ {} {} · {}", c.name, state, c.args)),
        );
        let body = if c.stderr.is_empty() { &c.stdout } else { &c.stderr };
        for line in body.lines().take(4) {
            col = col.child(div().text_color(neo_color(Tone::Text)).child(line.to_string()));
        }
    }
    let v = cx.entity().clone();
    col = col.child(
        h_flex()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .text_color(neo_color(Tone::Text))
                    .child(if view.terminal_input.is_empty() {
                        "输入命令后回车执行（不经模型）".to_string()
                    } else {
                        view.terminal_input.clone()
                    }),
            )
            .child(Button::new("run-cmd").label("执行").on_click(move |_, _, cx| {
                v.update(cx, |this, _| this.run_terminal_command());
            })),
    );
    Some(col)
}

/// 审批对话框（模态）：三档 Allow / Always / Reject。
fn approval_dialog(view: &NeoView, cx: &mut Context<NeoView>) -> impl IntoElement {
    let p = view.transcript.pending.clone().expect("调用方保证有 pending");
    let v1 = cx.entity().clone();
    let v2 = cx.entity().clone();
    let v3 = cx.entity().clone();
    v_flex()
        .gap_3()
        .p_4()
        .child(div().child("需要审批"))
        .child(div().child(p.detail.clone()))
        .child(
            div()
                .text_color(neo_color(Tone::Muted))
                .child(format!("类别：{}", p.kind)),
        )
        .child(
            h_flex()
                .gap_2()
                .child(
                    Button::new("allow").label("允许").on_click(move |_, _, cx| {
                        v1.update(cx, |this, _| this.approve(Decision::Allow));
                    }),
                )
                .child(
                    Button::new("always").label("总是允许").on_click(move |_, _, cx| {
                        v2.update(cx, |this, _| this.approve(Decision::AllowAlways));
                    }),
                )
                .child(
                    Button::new("reject").label("拒绝").on_click(move |_, _, cx| {
                        v3.update(cx, |this, _| this.approve(Decision::Deny));
                    }),
                ),
        )
}

impl Render for NeoView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 1) 收事件（非阻塞）
        // 冒烟钩子：首帧把 NEO_GUI_PROMPT 当作一次提交（只做一次）
        if let Some(text) = self.auto_prompt.take() {
            self.input = text;
            self.submit();
            cx.notify();
        }

        // 1) 收事件（非阻塞）
        let changed = self.pump();
        // 2) 有变化 → 标记需要重绘（响应式宿主的核心一步）。
        //
        // 注意这里**不消费** wake 信号：信号的作用是"让视图被唤醒一次"，
        // 而唤醒后的重绘由 notify 驱动。消费动作留给宿主入口（见 `run`），
        // 这样信号与 pump 的职责不重叠。
        if changed {
            cx.notify();
        }
        // 3) 轮次进行中：推进一步。
        //
        // 不需要"每帧排一帧"的动画机制：推进本身会产生事件批，
        // 驱动的唤醒钩子（见 `run`）会再触发一次重绘 —— 事件不断则循环自持。
        // 内核忙（暂无事件）时界面停住是对的，那时也没有新内容可画。
        if self.transcript.running && self.transcript.pending.is_none() {
            self.pump_step();
        }

        let blocks = self.transcript.blocks.clone();
        let running = self.transcript.running;
        let mode = self.mode;
        let model = self.model.clone();
        let status = self.status.clone();
        let notice = self.notice.clone();
        let total = (self.transcript.total_in, self.transcript.total_out);
        let input = self.input.clone();
        let blocked = self.transcript.pending.is_some();

        let view_entity = cx.entity().clone();
        let view_for_submit = cx.entity().clone();

        let mut root = v_flex()
            .size_full()
            // ⚠️ 用 base_bg() 而**不是** neo_color(Tone::None)：后者兜底到正文色（近白），
            // 当底色用会画出白底白字（真机截图抓到的）
            .bg(neo_ui::base_bg())
            .text_color(neo_color(Tone::Text))
            // ── 状态行 ──
            .child(
                h_flex()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .child(
                        div()
                            .text_color(neo_color(Tone::Accent))
                            .child("NEO"),
                    )
                    .child(div().text_color(neo_color(Tone::Muted)).child(status))
                    .child(
                        div()
                            .text_color(neo_color(if matches!(
                                mode,
                                ExecMode::AutoEdit | ExecMode::FullAccess
                            ) {
                                Tone::Warning
                            } else {
                                Tone::Info
                            }))
                            .child(mode_label(mode)),
                    )
                    .child(
                        div()
                            .text_color(neo_color(Tone::Muted))
                            .child(format!("模型：{model}")),
                    )
                    // 高风险档位常驻提示（ZCode 语义：风险状态不能只在切档时弹一次）
                    .child(if matches!(mode, ExecMode::AutoEdit | ExecMode::FullAccess) {
                        div()
                            .text_color(neo_color(Tone::Warning))
                            .child("⚠ 写操作可能不经确认")
                    } else {
                        div()
                    })
                    .child(
                        div()
                            .text_color(neo_color(Tone::Muted))
                            .child(format!("{} in / {} out", total.0, total.1)),
                    )
                    .child(if let Some(n) = notice {
                        div().text_color(neo_color(Tone::Success)).child(n)
                    } else {
                        div()
                    })
                    .child(
                        div()
                            .text_color(neo_color(if self.transcript.pending.is_some() {
                                Tone::Warning
                            } else if running {
                                Tone::Info
                            } else {
                                Tone::Muted
                            }))
                            .child(if self.transcript.pending.is_some() {
                                "待审批"
                            } else if running {
                                "运行中"
                            } else {
                                "就绪"
                            }),
                    ),
            )
            // ── 主区：转录（左）+ 目标/会话面板（右，D1/D7）──
            .child(
                h_flex()
                    .flex_1()
                    .min_h(px(0.))
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .overflow_y_scrollbar()
                            .child(transcript_view(&blocks)),
                    )
                    .child(side_panel(self, cx)),
            );

        // ── 审批对话框（模态覆盖）──
        if self.transcript.pending.is_some() {
            root = root.child(approval_dialog(self, cx));
        }

        // ── 命令台（D8）：输入区之上 ──
        if let Some(t) = terminal_panel(self, cx) {
            root = root.child(t);
        }

        // ── 输入区（审批未决时阻塞）──
        root = root.child(
            h_flex()
                .gap_2()
                .px_3()
                .py_2()
                .child(
                    div()
                        .flex_1()
                        .text_color(if blocked {
                            neo_color(Tone::Muted)
                        } else {
                            neo_color(Tone::Text)
                        })
                        .child(if blocked {
                            "待审批：请先在上方选择（避免把下一步排进队列）".to_string()
                        } else if input.is_empty() {
                            "输入任务后回车提交（@文件 / $技能 可用）".to_string()
                        } else {
                            input.clone()
                        }),
                )
                .child(Button::new("send").label("发送").on_click(move |_, _, cx| {
                    view_for_submit.update(cx, |this, _| this.submit());
                })),
        );

        // ── 键盘：模态优先，由应用自己裁决（见 neo-ui-behavior 的 KeyArbiter）──
        //
        // 裁决规则已在 `KeyArbiter` 里用测试钉住（含"模态独占键盘"）。
        // 放在最外层容器上，这样面板与输入区的按键都归它管。
        root = root.on_key_down(move |ev, _window, cx| {
            let key = ev.keystroke.key.to_string();
            let shift = ev.keystroke.modifiers.shift;
            let secondary = ev.keystroke.modifiers.secondary(); // macOS=Cmd / 其它=Ctrl
            let v = view_entity.clone();
            // 模态优先：命令面板打开时，它先接管键盘（见 KeyArbiter 的规则）
            let cmd_open = v.read(cx).cmd_open;

            if secondary && key == "k" {
                v.update(cx, |this, cx| this.toggle_cmd(cx));
            } else if key == "escape" && cmd_open {
                // Esc 的语义："关掉最上面那层"。面板没开时**不作声** ——
                // 不做任何事好过误关别的东西。
                v.update(cx, |this, cx| {
                    this.cmd_open = false;
                    cx.notify();
                });
            } else if cmd_open && (key == "up" || key == "down" || key == "enter") {
                v.update(cx, |this, cx| this.cmd_key(&key, cx));
            } else if shift && key == "tab" {
                v.update(cx, |this, cx| {
                    this.cycle_mode();
                    cx.notify();
                });
            }
        });

        // ── 命令面板（D9）：覆盖在主区之上 ──
        if let Some(p) = command_palette(self, cx) {
            root = root.child(
                div()
                    .absolute()
                    .top(px(48.))
                    .left(px(120.))
                    .child(p),
            );
        }

        root
    }
}

/// 打开窗口并运行到关闭。
#[allow(clippy::too_many_arguments)]
pub fn run(
    handle: KernelHandle,
    sessions: Option<Box<dyn neo_session::SessionControl>>,
    models: Vec<String>,
    title: String,
    status: String,
    mode: ExecMode,
    model: String,
    // 唤醒信号：驱动线程投递，本宿主在渲染路径里消费（gpui 上下文非 Send，
    // 驱动线程不能直接调 cx.notify）。
    wake: neo_driver::WakeSignal,
) -> Result<(), String> {
    let view_holder: Arc<std::sync::Mutex<Option<Entity<NeoView>>>> =
        Arc::new(std::sync::Mutex::new(None));

    // 闭包是 `FnOnce`，但里面要用两次（建视图与后续）——先备好克隆。
    // `Box<dyn SessionControl>` 不可 Clone，所以用 `Arc<Mutex<..>>` 包一层：
    // 它只在建视图时被取走一次，之后为 None。
    let sessions_for_view = Arc::new(std::sync::Mutex::new(sessions));
    let models_for_view = Arc::new(models);

    neo_ui_kit::application()
        .with_assets(neo_ui_kit::assets::Assets)
        .run(move |cx| {
            neo_ui_kit::init(cx);
            // 唤醒循环：信号到达 → 通知视图重绘。
            //
            // 这是响应式宿主的**唯一**重绘触发点（gpui 不出帧就不画）。
            // 用 `cx.spawn` 挂一个后台等待：信号驱动，不轮询、不空转 ——
            // `WakeSignal` 的通道是 async 的，`recv().await` 会真正挂起。
            {
                let wake = wake.clone();
                let holder = view_holder.clone();
                let async_cx = cx.to_async();
                cx.spawn(async move |_| {
                    while wake.recv().await.is_some() {
                        let view = holder
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .clone();
                        if let Some(view) = view {
                            async_cx.update(|cx| cx.notify(view.entity_id()));
                        }
                    }
                })
                .detach();
            }
            // 品牌主题：NEO 的紫（从 `neo-text` 的调色板派生，不写字面量）
            neo_ui::apply_neo_theme(cx);

            let view = cx.new(|_cx| {
                let sess = sessions_for_view
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take();
                NeoView::new(handle.clone(), sess, (*models_for_view).clone(), status, mode, model)
            });
            *view_holder.lock().unwrap_or_else(|e| e.into_inner()) = Some(view.clone());


            let view_for_window = view.clone();
            cx.spawn(async move |cx| {
                // 窗口尺寸要算在**主 App 上**（要有 display 信息），而这里拿到的是
                // `AsyncApp`（异步上下文）—— 所以先用 `update` 借一次主 App。
                let bounds = cx.update(|cx| {
                    neo_ui_kit::gpui::Bounds::centered(
                        None,
                        neo_ui_kit::gpui::size(px(1080.), px(720.)),
                        cx,
                    )
                });
                let _ = cx.open_window(
                    neo_ui_kit::gpui::WindowOptions {
                        window_bounds: Some(neo_ui_kit::gpui::WindowBounds::Windowed(bounds)),
                        titlebar: Some(neo_ui_kit::gpui::TitlebarOptions {
                            title: Some(title.clone().into()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    move |window, cx| {
                        // ⚠️ 窗口首层必须是 `Root`：否则 dialog/通知/tooltip
                        // 会 panic 或静默失效（见 gpui-component 的 Root::update）
                        let inner = view_for_window.clone();
                        cx.new(|cx| Root::new(inner, window, cx))
                    },
                );
            })
            .detach();
        });

    Ok(())
}

/// 供测试与调用方检查工具参数摘要（转发共享实现，避免宿主各写一套）。
pub use neo_driver::transcript::summarize_args as summarize_tool_args;

#[cfg(test)]
mod tests {
    use super::*;

    /// 模型循环：必须真的换到下一个，且**回绕**到第一个。
    ///
    /// 抽成自由函数就是为了能这样测 —— 循环写错的症状是"切了没反应"
    /// 或"跳到不存在的模型"，而两者在真机上都只表现为"模型名没变"。
    #[test]
    fn next_model_cycles_and_wraps() {
        let models: Vec<String> = vec!["a".into(), "b".into(), "c".into()];
        assert_eq!(next_model("a", &models), "b");
        assert_eq!(next_model("b", &models), "c");
        assert_eq!(next_model("c", &models), "a", "应回绕到第一个");
    }

    /// 只有一个模型（或列表为空）时**原样返回**。
    ///
    /// 这条是防"给用户一个切不动的按钮"：单 provider 是常见配置
    /// （离线用 mock/selftest 时就是），那时切换应当是无操作而不是 panic 或跳到空值。
    #[test]
    fn next_model_is_a_noop_with_a_single_model() {
        let one: Vec<String> = vec!["only".into()];
        assert_eq!(next_model("only", &one), "only");
        let empty: Vec<String> = vec![];
        assert_eq!(next_model("x", &empty), "x", "空列表不该 panic");
    }

    /// 当前模型**不在列表里**（注册表变了/是启动时的旧名）时回到第一个。
    ///
    /// 不能返回"下一个"——那要先知道它是第几个，而它根本不在表里。
    #[test]
    fn next_model_falls_back_to_the_first_when_current_is_unknown() {
        let models: Vec<String> = vec!["a".into(), "b".into()];
        assert_eq!(next_model("ghost", &models), "a");
    }
}

