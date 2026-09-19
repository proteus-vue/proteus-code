//! 界面：事件流 → 显示块 → egui 绘制。
//!
//! # 分界
//!
//! - **转录模型**（事件流 → 显示块）已下沉到 `neo-driver`：两个 GUI 宿主共用，
//!   而 A3 禁止宿主互相依赖。本模块 re-export 它，既有调用点不变。
//! - **绘制**留在本模块：全部 `egui::` 调用都在这里（[`App`] 及其 `draw_*`）。
//!   换成 gpui 时这一半要重写，另一半原样复用。

use std::time::Duration;


use crate::driver::KernelHandle;
use crate::GuiPalette;
use neo_protocol::TodoStatus;
use neo_text::Tone;

// 转录模型来自共享层（episode 语义与渲染无关，两个 GUI 宿主共用一份实现）
pub use neo_driver::transcript::{
    diff_line_kind, mode_is_risky, mode_label, next_mode, summarize_args, truncate_chars, Block,
    DiffLineKind, Pending, ToolCard, Transcript,
};

/// 焦点请求目标（见 [`App::focus_request`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusTarget {
    /// 任务输入框
    Composer,
    /// 命令台输入框
    Terminal,
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
    /// **下一个该拿焦点的控件**（一次性请求）。
    ///
    /// # 为什么用单一请求而不是几个布尔
    ///
    /// 曾经用两个独立布尔（`composer_focused_once` / `terminal_wants_focus`）
    /// 各自在 `draw` 里 `request_focus()`。它们是**竞争关系**而顺序又取决于
    /// 面板绘制次序（composer 画在 terminal 之后，于是它总是赢）——
    /// 真机表现：从命令台敲的命令被当任务发给了模型（日志里是 `begin_turn`
    /// 而不是 `shell`），白花一次模型请求。
    ///
    /// 改成"单一请求 + 谁最后设置谁生效"，竞争就不存在了：每一帧至多一个
    /// 控件去抢焦点。
    focus_request: Option<FocusTarget>,
    /// 上一帧是否处于"被审批阻塞"状态（用于检测"刚恢复可用"）。
    composer_blocked_last: bool,
    /// 当前执行模式。
    ///
    /// **必须由宿主自己记住**：内核的 `ConfigureSession` 只回一条
    /// `SessionConfigured`（不含模式），没有独立的"模式已变"事件 ——
    /// 所以状态行若不本地维护，切完模式界面还会显示旧档位（两处各说一套）。
    mode: neo_protocol::ExecMode,
    /// 当前模型名（同理由宿主维护；`ModelSwitched` 事件会覆盖它）。
    model: String,
    /// 可选模型列表（启动时查一次；运行时切换不改注册表）。
    models: Vec<(String, String, bool)>,
    /// D9 命令面板状态（与上面那个**颜色** `palette` 是两回事，故名字带 cmd_）。
    pub cmd_palette: crate::commands::Palette,
    /// **D8**：底部终端面板是否展开。
    terminal_open: bool,

    /// 终端输入框内容（独立于任务输入：两者语义完全不同 ——
    /// 一个交给模型，一个**不经模型**直接执行）。
    terminal_input: String,
    /// D1：会话控制（列举 / 切换 / 新建 / 删除）。
    ///
    /// `Option` 是刻意的：测试与"未接会话库"的装配可以不给 —— 那时侧栏
    /// 只显示当前会话，不显示列表（而不是 panic 或显示假数据）。
    sessions: Option<Box<dyn neo_session::SessionControl>>,
    /// 侧栏是否展开（D1）。
    sidebar_open: bool,
    /// `raw_input_hook` 摘下了本帧的 `Cmd/Ctrl+K`。
    pending_cmd_k: bool,
    /// `raw_input_hook` 摘下了本帧的 `Esc`（仅在命令面板打开时）。
    pending_esc: bool,
    /// 命令面板的搜索框是否已抢过焦点（每次打开抢一次）。
    cmd_palette_focused_once: bool,
    /// 是否显示帮助面板（命令与快捷键）。
    show_help: bool,
    /// 用户请求退出（由 `draw` 转成窗口关闭命令）。
    ///
    /// 不直接在这里调 `ViewportCommand::Close`：`run_action` 不接触 `Ui`，
    /// 保持"动作与绘制分离"，这样动作能被无头测试直接调用。
    quit_requested: bool,
    /// `raw_input_hook` 摘下了本帧的 `Shift+Tab`，等 `draw` 来执行切换。
    ///
    /// 为什么绕一道：egui 把 `Tab` / `Shift+Tab` 当**焦点导航键**，而它是在
    /// `begin_pass` 里读按键的 —— 那时我们的 `draw` 还没跑，等到帧内再
    /// `consume_key` 已经晚了（焦点已经被挪走）。真机复现过：切完模式焦点
    /// 从输入框跳到了右侧面板的 resize 手柄，**接着敲的字全部丢失**。
    /// 唯一的拦截点是 `raw_input_hook`（eframe 专门留给"阻止 egui 处理某个
    /// 快捷键"的口子），它在 `begin_pass` **之前**跑。
    pending_shift_tab: bool,
    /// 底下状态行的一次性提示（如"已切换到 X"），显示后自行消失。
    notice: Option<(String, std::time::Instant)>,
    /// **仅测试用**：最后一帧的焦点 id（供无头跑帧断言）。
    ///
    /// 焦点存在 `Context` 的 memory 里，而 `draw` 只拿到 `Ui`；测试跑完帧后
    /// 从 ctx 读出来塞回这里，避免为了可测而改 `draw` 的签名。
    #[doc(hidden)]
    pub __test_last_focus: Option<egui::Id>,
    /// 启动时自动提交的任务（一次性）。
    ///
    /// 来源是 `NEO_GUI_PROMPT` 环境变量 —— 供**冒烟验证**用：把
    /// 「启动 + 提交 + 渲染」合成一次运行，验证者截图即可，不必手工点输入框
    /// （egui 自绘画布不通过 accessibility 暴露可写文本，自动化输入进不去）。
    /// 与 AGENTS.md 里 `PROTEUS_CODE_SMOKE` / `PROTEUS_CODE_QUERY` 的约定同源。
    auto_prompt: Option<String>,
}

impl App {
    /// `mode` / `model` / `models` 由调用方（CLI）在启动时告知：
    /// 宿主不持有内核，这些是装配期的已知状态，之后由用户交互更新。
    pub fn new(
        handle: KernelHandle,
        status: String,
        mode: neo_protocol::ExecMode,
        model: String,
        models: Vec<(String, String, bool)>,
        sessions: Option<Box<dyn neo_session::SessionControl>>,
    ) -> Self {
        Self {
            handle,
            transcript: Transcript::new(),
            input: String::new(),
            goal_input: String::new(),
            palette: GuiPalette::neo(),
            status,
            mode,
            model,
            models,
            notice: None,
            pending_shift_tab: false,
            cmd_palette: crate::commands::Palette::default(),
            sessions,
            sidebar_open: true,
            terminal_open: false,

            terminal_input: String::new(),
            cmd_palette_focused_once: false,
            pending_cmd_k: false,
            pending_esc: false,
            show_help: false,
            quit_requested: false,
            show_reasoning: true,
            focus_request: Some(FocusTarget::Composer), // 首帧：输入框拿焦点
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

    /// **D11**：运行时切换执行模式。
    ///
    /// 走 `Op::ConfigureSession`（内核既有能力，不需要新端点 —— 计划里
    /// "D11 需新端点"的判断是过时的）。切换后必须**本地更新 `self.mode`**：
    /// 内核只回 `SessionConfigured`，不会告诉我们模式变成了什么。
    fn set_mode(&mut self, mode: neo_protocol::ExecMode) {
        if mode == self.mode {
            return;
        }
        self.handle.send(neo_protocol::Op::ConfigureSession {
            patch: neo_protocol::SessionPatch {
                exec_mode: Some(mode),
                ..Default::default()
            },
        });
        self.mode = mode;
        self.notice = Some((
            format!("已切换到 {} 模式", crate::ui::mode_label(mode)),
            std::time::Instant::now(),
        ));
    }

    /// 循环切换模式（对齐 ZCode 的 `Shift+Tab`）。
    fn cycle_mode(&mut self) {
        self.set_mode(next_mode(self.mode));
    }

    /// 切换模型。内核会校验名字，失败时不改本地状态（避免界面说谎）。
    fn set_model(&mut self, name: String) {
        if name == self.model {
            return;
        }
        self.handle.send(neo_protocol::Op::ConfigureSession {
            patch: neo_protocol::SessionPatch {
                model: Some(name.clone()),
                ..Default::default()
            },
        });
        // 乐观更新：内核若校验失败会发 Error 事件，届时转录里可见。
        // （TUI 同样按"提交后即认为生效"处理，因为它拿不到同步回执。）
        self.model = name.clone();
        self.notice = Some((format!("已切换到 {name}"), std::time::Instant::now()));
    }

    /// **D8**：执行一条用户直输的命令（不经模型）。
    ///
    /// 走 `Op::Shell` —— 内核把它交给**与模型工具调用同一条**执行路径
    /// （沙箱、输出上限、截断标记、落盘全部一致），所以这里不必也不该
    /// 自己起进程。用户的显式命令不再问审批（等价于用户自己在 shell 里敲它）。
    fn run_terminal_command(&mut self) {
        let cmd = self.terminal_input.trim().to_string();
        if cmd.is_empty() {
            return;
        }
        self.terminal_input.clear();
        self.handle.send(neo_protocol::Op::Shell { command: cmd });
    }

    /// **D1**：切换到另一个会话。
    ///
    /// 关键：内核换会话后返回的是**历史事件流**，宿主用它**替换**转录 ——
    /// 与接收实时事件走同一条渲染路径（`Transcript::push_batch`）。
    /// 不另写"重画历史"的代码，是避免两份画法漂移的关键。
    fn switch_session(&mut self, id: String) {
        let Some(sessions) = self.sessions.as_mut() else {
            self.notice = Some((
                "本装配未接会话库，无法切换会话".into(),
                std::time::Instant::now(),
            ));
            return;
        };
        match sessions.switch(&id) {
            Ok(history) => {
                // 换会话 = 换上下文：转录、累计 token、折叠状态都归零，
                // 否则上一条会话的 token 数会算到这一条上（数字对不上）
                self.transcript = Transcript::new();
                self.transcript.push_batch(&history);
                self.notice = Some((format!("已切换到会话 {id}"), std::time::Instant::now()));
            }
            Err(e) => {
                self.notice = Some((format!("切换失败：{e}"), std::time::Instant::now()));
            }
        }
    }

    /// 新建会话（旧会话留在磁盘上，可再切回）。
    fn new_session(&mut self) {
        let Some(sessions) = self.sessions.as_mut() else {
            // 静默无反应正是"用户以为坏了"的典型；如实说明更诚实
            self.notice = Some((
                "本装配未接会话库，无法新建会话".into(),
                std::time::Instant::now(),
            ));
            return;
        };
        match sessions.create() {
            Ok(id) => {
                self.transcript = Transcript::new();
                self.notice = Some((
                    format!("已新建会话 {id}（旧会话保留）"),
                    std::time::Instant::now(),
                ));
            }
            Err(e) => {
                self.notice = Some((format!("新建失败：{e}"), std::time::Instant::now()));
            }
        }
    }

    /// 执行一条命令（D9 命令面板的落地端）。
    ///
    /// 每个动作都**真的做点什么**：列着却点了没反应比没有更糟
    /// （用户会以为坏了）。能做的只有这些 —— 终端专有动作（主题、星场、
    /// logo）不在表里，所以这里不需要"空实现"分支。
    fn run_action(&mut self, action: crate::commands::Action) {
        use crate::commands::Action as A;
        match action {
            A::ToggleTerminal => {
                self.terminal_open = !self.terminal_open;
                // 开 → 焦点给终端（ZCode 的 Cmd+J 语义）；
                // 关 → 目光回到任务输入框（终端输入框已经不在了，焦点留在它上面
                // 等于键盘输入无处可去）。
                //
                // 用**单一请求**表达，所以不存在"两个控件争焦点"的问题：
                // 后设置的那个生效，与绘制顺序无关。
                self.focus_request = Some(if self.terminal_open {
                    FocusTarget::Terminal
                } else {
                    FocusTarget::Composer
                });
                let st = if self.terminal_open { "显示" } else { "隐藏" };
                self.notice = Some((format!("终端已{st}"), std::time::Instant::now()));
            }
            A::ToggleSidebar => {
                self.sidebar_open = !self.sidebar_open;
                let st = if self.sidebar_open { "显示" } else { "隐藏" };
                self.notice = Some((format!("会话栏已{st}"), std::time::Instant::now()));
            }
            // D12 文件树：**egui 侧不实现**（如实告知，不假装）。
            //
            // 为什么不实现：egui 已冻结（只修 bug、不加功能，见缺口表的
            // "删除前置条件"）—— gpui 是默认宿主且已实现文件树，为即将删除的
            // 宿主再补一个新功能是纯浪费。
            //
            // 为什么**必须**有这个分支而不是留空：留空会让命令表里的 `/files`
            // 在 egui 下"列着却点了没反应"，那正是本仓明确反对的
            // "看起来有功能"的假象（见 commands.rs 的穷尽性守卫）。
            // 所以给一句诚实的提示，并让用户知道去哪找。
            A::ToggleFiles => {
                self.notice = Some((
                    "文件树仅在 gpui 宿主可用（默认宿主；本窗口是 egui 回退）".to_string(),
                    std::time::Instant::now(),
                ));
            }
            // D5 全屏 diff 查看器：egui 侧不实现（同上，给诚实提示）。
            // 注：egui 有自己的审批 diff 预览 + 全屏查看器在 TUI/gpui 两侧，
            // 它是冻结的回退通道，不为它补新功能。
            A::OpenDiff => {
                self.notice = Some((
                    "全屏 diff 查看器在 gpui 宿主与 TUI 里可用（本窗口是 egui 回退）"
                        .to_string(),
                    std::time::Instant::now(),
                ));
            }
            // D12 仓库文档（Repo Wiki）：与文件树同理 —— egui 侧不实现，
            // 但必须给一句诚实的提示（留空 = "列着却点了没反应"）。
            A::ToggleWiki => {
                self.notice = Some((
                    "仓库文档仅在 gpui 宿主可用（默认宿主；本窗口是 egui 回退）".to_string(),
                    std::time::Instant::now(),
                ));
            }
            A::NewSession => self.new_session(),
            A::Compact => {
                self.handle.send(neo_protocol::Op::Compact);
                self.notice = Some(("正在压缩上下文…".into(), std::time::Instant::now()));
            }
            A::Rewind => {
                self.handle.send(neo_protocol::Op::Rewind { turns: 1 });
                self.notice = Some(("已请求回退一轮".into(), std::time::Instant::now()));
            }
            A::Interrupt => {
                self.handle.send(neo_protocol::Op::Interrupt);
                self.notice = Some(("已请求打断".into(), std::time::Instant::now()));
            }
            A::ShowModels => {
                // 模型 picker 常驻在状态栏，这里只提示一下去哪儿找
                self.notice = Some((
                    "模型可在状态栏的模型下拉里切换".into(),
                    std::time::Instant::now(),
                ));
            }
            A::CycleMode => self.cycle_mode(),
            A::ToggleReasoning => {
                self.show_reasoning = !self.show_reasoning;
                let st = if self.show_reasoning { "展开" } else { "折叠" };
                self.notice = Some((format!("思考轨迹已{st}"), std::time::Instant::now()));
            }
            A::ClearTranscript => {
                // 只清屏幕，**不动会话日志**：日志是回放与审计的唯一依据
                self.transcript.clear_view();
                self.notice = Some((
                    "已清空屏幕转录（会话日志保留）".into(),
                    std::time::Instant::now(),
                ));
            }
            A::Help => self.show_help = true,
            A::Quit => {
                self.quit_requested = true;
            }
        }
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

    fn draw_status(&mut self, ui: &mut egui::Ui) {
        let mut pick_mode: Option<neo_protocol::ExecMode> = None;
        let mut pick_model: Option<String> = None;

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

            // ── D11：模式（下拉切换；Shift+Tab 也能循环）──
            let risky = mode_is_risky(self.mode);
            let mode_tone = if risky { Tone::Warning } else { Tone::Info };
            egui::ComboBox::from_id_salt("neo_mode")
                .selected_text(
                    egui::RichText::new(mode_label(self.mode)).color(self.palette.color(mode_tone)),
                )
                .width(120.0)
                .show_ui(ui, |ui| {
                    use neo_protocol::ExecMode as M;
                    for m in [M::Plan, M::ConfirmBefore, M::Default, M::AutoEdit, M::FullAccess] {
                        if ui
                            .selectable_label(m == self.mode, mode_label(m))
                            .clicked()
                        {
                            pick_mode = Some(m);
                        }
                    }
                })
                .response
                .on_hover_text("执行模式（Shift+Tab 循环）");

            // ── 模型 picker ──
            let current = self.model.clone();
            let label = if self.models.len() > 1 {
                format!("模型：{current}")
            } else {
                format!("模型：{current}")
            };
            ui.add_enabled_ui(self.models.len() > 1, |ui| {
                egui::ComboBox::from_id_salt("neo_model")
                    .selected_text(egui::RichText::new(label).color(self.palette.color(Tone::Muted)).small())
                    .width(150.0)
                    .show_ui(ui, |ui| {
                        for (name, desc, production) in &self.models {
                            // 桩 provider（mock/selftest）标出来：它们不能真跑任务
                            let text = if *production {
                                name.clone()
                            } else {
                                format!("{name}（桩）")
                            };
                            let resp = ui.selectable_label(*name == current, text);
                            if resp.clicked() {
                                pick_model = Some(name.clone());
                            }
                            if !desc.is_empty() {
                                resp.on_hover_text(desc);
                            }
                        }
                    });
            });

            // ── ZCode 语义：高风险档位要在**工具栏常驻**提示 ──
            // （不是只在切档那一刻弹一次 —— 否则用户过一会儿就忘了自己在放行模式）
            if risky {
                ui.label(
                    egui::RichText::new("⚠ 写操作可能不经确认")
                        .color(self.palette.color(Tone::Warning))
                        .small(),
                );
            }

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
                // 一次性提示（"已切换到 X"），几秒后自行消失
                if let Some((text, at)) = &self.notice {
                    if at.elapsed() < std::time::Duration::from_secs(4) {
                        ui.label(
                            egui::RichText::new(text)
                                .color(self.palette.color(Tone::Success))
                                .small(),
                        );
                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(300));
                    }
                }
            });
        });

        if let Some(m) = pick_mode {
            self.set_mode(m);
        }
        if let Some(name) = pick_model {
            self.set_model(name);
        }
    }

    /// **D8**：底部终端面板。
    ///
    /// 对标 ZCode 的 `Cmd/Ctrl+J` 内置终端。**与它的关键差别**：ZCode 用
    /// `node-pty` 起一个真 PTY（可跑 vim/htop 这类全屏交互程序），我们走
    /// `Op::Shell` —— **每条命令一个进程、无 PTY**。所以这里**不能**做成
    /// "交互式终端"，只能做"命令执行台"：输入一条、跑完、看输出。
    ///
    /// 这个边界必须如实呈现，不能假装是终端：真的 PTY 需要引入
    /// `portable-pty` 之类的依赖（还有窗口尺寸、信号、全屏应用兼容一堆事），
    /// 不在本轮范围。面板标题写"命令台"而不是"终端"，就是这个原因。
    fn draw_terminal(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("命令台")
                    .color(self.palette.color(Tone::Info))
                    .strong(),
            );
            ui.label(
                egui::RichText::new("（每条命令一个进程，走沙箱；不是交互式终端）")
                    .color(self.palette.color(Tone::Muted))
                    .small(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("收起").clicked() {
                    self.terminal_open = false;
                }
            });
        });

        // 输出：复用转录里的工具卡片（`Op::Shell` 产出的是同一对
        // ToolCallBegin/ToolCallEnd 事件），所以**不另存一份终端历史** ——
        // 两份历史必然漂移（清屏时一份清了一份没清之类）。
        egui::ScrollArea::vertical()
            .id_salt("neo_terminal_out")
            .max_height(180.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let cards: Vec<ToolCard> = self
                    .transcript
                    .blocks
                    .iter()
                    .filter_map(|b| match b {
                        Block::Tool(c) => Some(c.clone()),
                        _ => None,
                    })
                    .collect();
                if cards.is_empty() {
                    ui.label(
                        egui::RichText::new("还没有执行过命令")
                            .color(self.palette.color(Tone::Muted))
                            .small(),
                    );
                }
                for c in cards {
                    self.draw_tool(ui, &c);
                }
            });

        ui.horizontal(|ui| {
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.terminal_input)
                    .hint_text("输入命令后回车执行（不经模型）")
                    .id(egui::Id::new("neo_terminal_in")),
            );
            // 刚打开面板时抢一次焦点（之后不再抢，否则用户没法把焦点移到别处）
            if self.focus_request == Some(FocusTarget::Terminal) {
                resp.request_focus();
                self.focus_request = None;
            }

            // 同 composer 用 `lost_focus()`（回车即放弃焦点）
            if (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                || ui.button("执行").clicked()
            {
                self.run_terminal_command();
            }
        });
    }

    /// **D1**：左侧会话栏。
    ///
    /// 对标 ZCode 的左侧栏，但**只做我们数据支持得住的部分**：会话列表
    /// （标题 / 记录数 / 当前项高亮）+ 新建按钮。ZCode 还有"按工作区分组、
    /// 状态圆点、`+/-` 行数、Grouped/Workspace/Timeline 视图切换、Archive" ——
    /// 那些需要会话元数据里有工作区与运行状态，当前 `SessionStore` 只存
    /// id/标题/记录数。**列出来但显示不出来，是比不列更糟的假象**，故不做。
    fn draw_sidebar(&mut self, ui: &mut egui::Ui) {
        let mut pick: Option<String> = None;
        let mut new_clicked = false;

        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("会话")
                    .color(self.palette.color(Tone::Info))
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("+ 新建").clicked() {
                    new_clicked = true;
                }
            });
        });
        ui.add_space(4.0);

        let Some(sessions) = self.sessions.as_ref() else {
            ui.label(
                egui::RichText::new("（未接会话库）")
                    .color(self.palette.color(Tone::Muted))
                    .small(),
            );
            return;
        };

        let current = sessions.current();
        let mut list = sessions.list();
        // ⚠️ **当前会话必须出现在列表里，哪怕它还没有文件。**
        //
        // `SessionStore::new_id()` 刻意不建文件（首次写入才惰性创建，避免
        // "新建了却没用"留空文件）。于是刚点过"新建"的会话不在 `list()` 里 ——
        // 真机实测：点了新建、界面回了"已新建会话 X"，但侧栏仍显示"暂无会话"，
        // 用户看不到也点不到自己刚建的那个会话。补上这一条。
        if !current.is_empty() && !list.iter().any(|s| s.id == current) {
            list.insert(
                0,
                neo_session::SessionInfo {
                    id: current.clone(),
                    title: String::new(),
                    records: 0,
                    changes: None,
                    state: neo_session::SessionState::Empty,
                },
            );
        }
        if list.is_empty() {
            ui.label(
                egui::RichText::new("暂无会话")
                    .color(self.palette.color(Tone::Muted))
                    .small(),
            );
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for s in list {
                    let id = &s.id;
                    let is_current = *id == current;
                    // 标题可能为空（新会话还没起名）——用 id 兜底，
                    // 否则列表里会出现一行空白，用户不知道那是什么
                    let shown = if s.title.trim().is_empty() {
                        id.clone()
                    } else {
                        s.title.clone()
                    };
                    let text = egui::RichText::new(format!("{shown}  ·{}", s.records))
                        .color(self.palette.color(if is_current { Tone::Accent } else { Tone::Text }));
                    let resp = ui.selectable_label(is_current, text);
                    if resp.clicked() && !is_current {
                        pick = Some(id.clone());
                    }
                    // id 放在悬停提示里：标题可能重复，id 是唯一标识
                    resp.on_hover_text(format!("{id}（{} 条记录）", s.records));
                }
            });

        if new_clicked {
            self.new_session();
        }
        if let Some(id) = pick {
            self.switch_session(id);
        }
    }

    /// **D9**：命令面板（覆盖式，`Cmd/Ctrl+K`）。
    ///
    /// 覆盖式而不是侧边抽屉 —— 对标 ZCode 的语义：它是"随手唤起的动作入口"，
    /// 用完就消失，不该长期占地方。
    fn draw_command_palette(&mut self, ctx: &egui::Context) {
        if !self.cmd_palette.open {
            return;
        }
        let mut close = false;
        let mut run: Option<crate::commands::Action> = None;

        egui::Window::new("命令")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, [0.0, 80.0])
            .fixed_size([460.0, 320.0])
            .show(ctx, |ui| {
                // 搜索框自动聚焦：打开就能打字（否则还得点一下）
                let search = ui.add(
                    egui::TextEdit::singleline(&mut self.cmd_palette.query)
                        .hint_text("输入以筛选（可搜命令名或说明）")
                        .id(egui::Id::new("neo_cmd_search")),
                );
                if !self.cmd_palette_focused_once {
                    search.request_focus();
                    self.cmd_palette_focused_once = true;
                }

                ui.separator();

                let matches = self.cmd_palette.matches();
                if matches.is_empty() {
                    ui.label(
                        egui::RichText::new("没有匹配的命令")
                            .color(self.palette.color(Tone::Muted)),
                    );
                }
                // 高亮夹紧：搜索变化后旧下标可能越界（会显示空行/错行）
                let n = matches.len();
                if n > 0 && self.cmd_palette.selected >= n {
                    self.cmd_palette.selected = n - 1;
                }

                egui::ScrollArea::vertical().max_height(230.0).show(ui, |ui| {
                    for (cat, list) in crate::commands::grouped(&matches) {
                        ui.label(
                            egui::RichText::new(crate::commands::category_title(cat))
                                .color(self.palette.color(Tone::Muted))
                                .small(),
                        );
                        for c in list {
                            let idx = matches
                                .iter()
                                .position(|m| m.name == c.name)
                                .unwrap_or(0);
                            let is_sel = idx == self.cmd_palette.selected;
                            let text = egui::RichText::new(format!("/{}  {}", c.name, c.desc))
                                .color(self.palette.color(if is_sel { Tone::Accent } else { Tone::Text }));
                            let resp = ui.selectable_label(is_sel, text);
                            if resp.clicked() {
                                run = Some(c.action.resolve());
                                close = true;
                            }
                            if resp.hovered() {
                                self.cmd_palette.selected = idx;
                            }
                        }
                    }
                });

                ui.separator();
                ui.label(
                    egui::RichText::new("↑↓ 选择 · Enter 执行 · Esc 关闭")
                        .color(self.palette.color(Tone::Muted))
                        .small(),
                );
            });

        // 键盘：用原始事件（egui 会把方向键/回车用于控件导航，与 Shift+Tab 同理）
        ctx.input(|i| {
            for e in &i.events {
                if let egui::Event::Key { key, pressed: true, .. } = e {
                    match key {
                        egui::Key::ArrowDown => self.cmd_palette.move_selection(true),
                        egui::Key::ArrowUp => self.cmd_palette.move_selection(false),
                        egui::Key::Enter => {
                            if let Some(c) = self.cmd_palette.selected_command() {
                                run = Some(c.action.resolve());
                                close = true;
                            }
                        }
                        _ => {}
                    }
                }
            }
        });

        if let Some(a) = run {
            self.run_action(a);
        }
        if close {
            self.cmd_palette.close();
            self.cmd_palette_focused_once = false;
            // 关闭覆盖层后把焦点还给输入框（不还回去的话键盘输入无处可去
            // —— 真机表现为"打字没反应"）。
            //
            // ⚠️ **只在动作没指定焦点时才还**：`/terminal` 这类命令自己会指定
            // 焦点目标（要交给终端输入框），而无条件归还会把它的请求覆盖掉
            // —— 执行顺序是"先 run_action、后 close"，后写的赢。
            // 这个覆盖曾让"打开命令台后敲的命令仍被当成任务发给模型"。
            if self.focus_request.is_none() {
                self.focus_request = Some(FocusTarget::Composer);
            }
        }
    }

    /// 帮助面板（命令与快捷键总览）。
    fn draw_help(&mut self, ctx: &egui::Context) {
        if !self.show_help {
            return;
        }
        let mut close = false;
        egui::Window::new("帮助")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(egui::RichText::new("快捷键").strong());
                for (k, v) in [
                    ("Cmd/Ctrl+K", "命令面板"),
                    ("Shift+Tab", "循环切换执行模式"),
                    ("Enter", "提交输入框内容"),
                ] {
                    ui.label(
                        egui::RichText::new(format!("  {k:<14} {v}"))
                            .color(self.palette.color(Tone::Text)),
                    );
                }
                ui.add_space(8.0);
                ui.label(egui::RichText::new("命令").strong());
                for c in crate::commands::COMMANDS {
                    ui.label(
                        egui::RichText::new(format!("  /{:<12} {}", c.name, c.desc))
                            .color(self.palette.color(Tone::Text)),
                    );
                }
                ui.add_space(8.0);
                if ui.button("关闭").clicked() {
                    close = true;
                }
            });
        if close {
            self.show_help = false;
            self.focus_request = Some(FocusTarget::Composer); // 焦点还给输入框
        }
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
            if !blocked
                && (self.focus_request == Some(FocusTarget::Composer) || just_unblocked)
            {
                edit.request_focus();
                self.focus_request = None;
            }
            self.composer_blocked_last = blocked;

            // 回车提交。egui 的单行 TextEdit 在按回车时**放弃焦点**，
            // 所以判据是 `lost_focus() && Enter` —— 那是 egui 的标准写法。
            // （不要改成 `has_focus()`：回车那一刻焦点已经交出去了，
            // `has_focus()` 恰好是 false，会变成永远不提交。）
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
    /// **在 egui 的 `begin_pass` 之前**摘掉 `Shift+Tab`。
    ///
    /// 这是唯一能阻止 egui 把它当焦点导航键的地方。若不摘：切了模式的同时
    /// 焦点被挪到下一个控件（真机实测跳到了右侧面板的 resize 手柄），
    /// 用户接着敲的字会落在别处甚至丢失。
    ///
    /// 代价：界面里 Tab 焦点导航基本失效 —— 有意的取舍，"切执行模式"是
    /// 高频操作，而 Tab 导航在这套界面里几乎用不到。
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if self.transcript.pending.is_some() {
            return; // 审批未决时不响应（不该偷偷放宽权限）
        }
        let is_shift_tab = |e: &egui::Event| {
            matches!(
                e,
                egui::Event::Key {
                    key: egui::Key::Tab,
                    pressed: true,
                    modifiers,
                    ..
                } if modifiers.shift
            )
        };
        if raw_input.events.iter().any(is_shift_tab) {
            // 摘掉它：egui 就看不到这个 Tab，也就不会做焦点导航
            raw_input.events.retain(|e| !is_shift_tab(e));
            self.pending_shift_tab = true;
        }

        // `Cmd/Ctrl+K`：开命令面板。用同一个钩子是为了**同一套修饰键判断口径** ——
        // macOS 上要 `Cmd`，其它平台 `Ctrl`（egui 的 `Modifiers::command` 已做了
        // 这个平台适配，不必自己 cfg）。
        let is_cmd_k = |e: &egui::Event| {
            matches!(
                e,
                egui::Event::Key {
                    key: egui::Key::K,
                    pressed: true,
                    modifiers,
                    ..
                } if modifiers.command
            )
        };
        if raw_input.events.iter().any(is_cmd_k) {
            raw_input.events.retain(|e| !is_cmd_k(e));
            self.pending_cmd_k = true;
        }

        // `Esc`：关命令面板。同样必须在**这里**摘掉 —— 真机实测：只在帧内判
        // `Key::Escape` 的话，egui 已经先把它当"结束文本编辑"处理了，
        // 表现是**搜索框被清空、面板却还开着**（用户按 Esc 以为关了，实际没有）。
        if self.cmd_palette.open {
            let is_esc = |e: &egui::Event| {
                matches!(
                    e,
                    egui::Event::Key {
                        key: egui::Key::Escape,
                        pressed: true,
                        ..
                    }
                )
            };
            if raw_input.events.iter().any(is_esc) {
                raw_input.events.retain(|e| !is_esc(e));
                self.pending_esc = true;
            }
        }
    }

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
        // D11：`Shift+Tab` 切执行模式。
        //
        // 按键的**摘除**在 `raw_input_hook` 里做（见那里的说明）—— 这里只负责
        // 执行动作。用一个标记而不是直接判按键，是因为"事件已被摘掉"这件事
        // 只有 hook 知道。
        if self.pending_shift_tab {
            self.pending_shift_tab = false;
            if self.transcript.pending.is_none() {
                self.cycle_mode();
            }
        }
        if self.pending_esc {
            self.pending_esc = false;
            if self.cmd_palette.open {
                self.cmd_palette.close();
                self.cmd_palette_focused_once = false;
                // 同上：关掉覆盖层要把焦点还给输入框。
                // 只在"确实关了一个覆盖层"时做 —— Esc 在别处（比如关对话框）
                // 不该抢焦点。
                self.focus_request = Some(FocusTarget::Composer);
            }
        }
        if self.pending_cmd_k {
            self.pending_cmd_k = false;
            // 再按一次 `Cmd+K` 关闭（与"开关式"命令面板的通行做法一致）
            if self.cmd_palette.open {
                self.cmd_palette.close();
                self.cmd_palette_focused_once = false;
                self.focus_request = Some(FocusTarget::Composer); // 焦点还给输入框
            } else {
                self.cmd_palette.open();
            }
        }

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
        if self.sidebar_open {
            egui::Panel::left("neo_sidebar")
                .default_size(200.0)
                .show(ui, |ui| self.draw_sidebar(ui));
        }
        egui::Panel::right("neo_goal")
            .default_size(240.0)
            .show(ui, |ui| self.draw_goal_panel(ui));
        // 终端在 composer **之上**：先加的先占底部边缘（egui 的 panel 语义），
        // 于是从上到下的视觉顺序是 转录 / 终端 / 输入框 —— 输入框永远贴着底边，
        // 位置稳定，不会因为开合终端而跳动。
        if self.terminal_open {
            egui::Panel::bottom("neo_terminal")
                .default_size(220.0)
                .show(ui, |ui| self.draw_terminal(ui));
        }
        egui::Panel::bottom("neo_composer").show(ui, |ui| self.draw_composer(ui));

        egui::CentralPanel::default().show(ui, |ui| self.draw_transcript(ui));

        self.draw_approval(ui);
        self.draw_command_palette(ui.ctx());
        self.draw_help(ui.ctx());

        // 退出：由 `run_action` 标记，这里转成窗口关闭命令
        if self.quit_requested {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }

        // 回车提交已由 composer 的 lost_focus 路径处理（那里才知道输入框的
        // 真实状态）；这里不再做全局回车判断 —— 两处都判会导致一次回车提交两次。
    }
}

/// 打开窗口并运行到关闭。
///
/// `status` 是状态行文字（工作区 / 模式 / 模型），由调用方（CLI）装配。
pub fn run(
    handle: KernelHandle,
    title: &str,
    status: String,
    mode: neo_protocol::ExecMode,
    model: String,
    models: Vec<(String, String, bool)>,
    sessions: Option<Box<dyn neo_session::SessionControl>>,
) -> Result<(), String> {
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
            Ok(Box::new(App::new(handle, status, mode, model, models, sessions)))
        }),
    )
    .map_err(|e| format!("创建窗口失败：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use neo_protocol::{EventMsg, GoalPhase, GoalSnapshot, GoalSubtask};

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
        (
            App::new(
                handle,
                "测试状态".into(),
                neo_protocol::ExecMode::Default,
                "test-model".into(),
                // 两个可选模型：让 picker 处在"可切换"状态，测到真实路径
                vec![
                    ("test-model".into(), "测试用".into(), true),
                    ("other".into(), "另一个".into(), false),
                ],
                // 渲染测试不接会话库：侧栏走"未接会话库"分支（也要能画）
                None,
            ),
            rx,
            tx,
        )
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
        // ⚠️ 必须走和真实 eframe 一样的入口：`raw_input_hook` 是摘除
        // `Shift+Tab` 的唯一拦截点（它在 `begin_pass` 之前跑）。测试若绕过它，
        // 就测不到"焦点会不会被 Tab 导航带走"这个真机 bug。
        {
            use eframe::App as _;
            app.raw_input_hook(ctx, &mut input);
        }
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
    // ─────────── D11：执行模式切换 ───────────

    /// 模式循环必须**覆盖全部五档并回到起点**。
    ///
    /// 漏一档就会让用户永远切不到那个模式（或卡在某一档出不来）。
    #[test]
    fn mode_cycle_visits_every_mode_and_wraps() {
        use neo_protocol::ExecMode as M;
        let all = [M::Plan, M::ConfirmBefore, M::Default, M::AutoEdit, M::FullAccess];
        let mut seen = vec![M::Default];
        let mut cur = M::Default;
        for _ in 0..all.len() - 1 {
            cur = next_mode(cur);
            assert!(!seen.contains(&cur), "{cur:?} 重复出现，说明漏了别的档位");
            seen.push(cur);
        }
        assert_eq!(next_mode(cur), M::Default, "应回到起点");
        assert_eq!(seen.len(), all.len(), "必须覆盖全部档位");
    }

    /// 高风险档位必须**常驻**提示（ZCode 语义：不是只在切换时弹一次）。
    ///
    /// 判据是"这个档位允许不经确认就写"。少标一个 = 用户在放行模式下没有提醒。
    #[test]
    fn risky_modes_are_flagged_and_safe_ones_are_not() {
        use neo_protocol::ExecMode as M;
        assert!(mode_is_risky(M::AutoEdit), "自动编辑允许不经确认就写");
        assert!(mode_is_risky(M::FullAccess), "完全放行更该提示");
        assert!(!mode_is_risky(M::Plan), "计划模式不动手，不该报风险");
        assert!(!mode_is_risky(M::ConfirmBefore), "变更前确认本来就会问");
        assert!(!mode_is_risky(M::Default), "默认档有审批门");
    }

    /// 切换模式要真的提交 `ConfigureSession` 并更新本地状态。
    ///
    /// 本地状态更新是**必须的**：内核只回 `SessionConfigured`（不含模式），
    /// 没有"模式已变"的独立事件 —— 不本地记的话状态行会一直显示旧档位。
    #[test]
    fn set_mode_submits_and_updates_local_state() {
        let (mut app, _rx, _tx) = render_test_app();
        assert_eq!(app.mode, neo_protocol::ExecMode::Default);
        app.set_mode(neo_protocol::ExecMode::Plan);
        assert_eq!(
            app.mode,
            neo_protocol::ExecMode::Plan,
            "本地模式必须跟着变（否则界面显示旧档位）"
        );
        assert!(app.notice.is_some(), "切换后应给出一次性提示");
    }

    /// 重复设同一个模式不发多余指令（避免每次点都产生一条日志）。
    #[test]
    fn setting_the_same_mode_is_a_noop() {
        let (mut app, _rx, _tx) = render_test_app();
        app.set_mode(neo_protocol::ExecMode::Default);
        assert!(app.notice.is_none(), "同档位切换不该产生提示（也没发指令）");
    }

    /// 切换模型：更新本地名字 + 给提示；同名也是空操作。
    #[test]
    fn set_model_updates_state_and_ignores_a_noop() {
        let (mut app, _rx, _tx) = render_test_app();
        app.set_model("other".into());
        assert_eq!(app.model, "other");
        assert!(app.notice.is_some(), "切换模型应有反馈");

        app.notice = None;
        app.set_model("other".into());
        assert!(app.notice.is_none(), "同名切换是空操作");
    }

    /// `Shift+Tab` 在界面上真的会切模式（真机按键路径）。
    #[test]
    fn shift_tab_cycles_the_mode_in_a_frame() {
        let (mut app, _rx, _tx) = render_test_app();
        let ctx = new_ctx();
        run_frame(&ctx, &mut app, vec![]);
        let before = app.mode;
        run_frame(
            &ctx,
            &mut app,
            vec![egui::Event::Key {
                key: egui::Key::Tab,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers {
                    shift: true,
                    ..Default::default()
                },
            }],
        );
        assert_ne!(app.mode, before, "Shift+Tab 应切换执行模式");
        assert_eq!(app.mode, next_mode(before));
    }

    /// `Shift+Tab` 之后**焦点必须留在输入框**。
    ///
    /// 这是真机抓到的 bug：egui 把 `Shift+Tab` 当焦点导航键，切完模式焦点
    /// 跳到了右侧目标框，**接着敲的字全部丢失**（真机复现：输入框里只有
    /// `Shift+Tab` 之前的 "aaa"，之后的 "bbb" 没了）。
    /// 修法是在**帧初**消费掉这个按键 —— 帧末消费来不及，导航已经发生。
    #[test]
    fn shift_tab_keeps_focus_on_the_composer() {
        let (mut app, _rx, _tx) = render_test_app();
        let ctx = new_ctx();
        // 先跑一帧让输入框拿到焦点
        run_frame(&ctx, &mut app, vec![]);
        assert_eq!(
            app.__test_last_focus,
            Some(egui::Id::new("neo_composer")),
            "前置条件：输入框有焦点"
        );
        let before_mode = app.mode;

        run_frame(
            &ctx,
            &mut app,
            vec![egui::Event::Key {
                key: egui::Key::Tab,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers {
                    shift: true,
                    ..Default::default()
                },
            }],
        );
        assert_ne!(app.mode, before_mode, "应已切换模式");

        // ⚠️ 必须再跑几帧才看得出焦点是否被带走：egui 的焦点导航是**延迟生效**的
        // ——`focus_direction` 在某一帧被设置，焦点实际转移发生在**下一个 pass**
        // 里控件注册时。第一版这条测试只跑一帧就断言，结果无论实现对错都通过
        // （用"帧末才消费"验证过：照样绿）—— 那是**假通过**，比没有测试更糟。
        for _ in 0..3 {
            run_frame(&ctx, &mut app, vec![]);
        }
        assert_eq!(
            app.__test_last_focus,
            Some(egui::Id::new("neo_composer")),
            "Shift+Tab 切模式后焦点必须仍在输入框（否则后续输入会丢）；实际={:?}",
            app.__test_last_focus
        );
    }

    /// 审批未决时 `Shift+Tab` **不得**改档位（不该在等审批时偷偷放宽权限）。
    #[test]
    fn shift_tab_does_not_change_mode_while_awaiting_approval() {
        let (mut app, _rx, _tx) = render_test_app();
        app.transcript.pending = Some(Pending {
            id: "a".into(),
            detail: "写文件".into(),
            kind: "write".into(),
        });
        let ctx = new_ctx();
        run_frame(&ctx, &mut app, vec![]);
        let before = app.mode;
        run_frame(
            &ctx,
            &mut app,
            vec![egui::Event::Key {
                key: egui::Key::Tab,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers {
                    shift: true,
                    ..Default::default()
                },
            }],
        );
        assert_eq!(app.mode, before, "待审批时不该改执行模式");
    }
    // ─────────── D9：命令面板 ───────────

    fn key_event(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    /// `Cmd/Ctrl+K` 打开面板（走 `raw_input_hook`，与真机同一条路径）。
    #[test]
    fn cmd_k_opens_and_closes_the_command_palette() {
        let (mut app, _rx, _tx) = render_test_app();
        let ctx = new_ctx();
        run_frame(&ctx, &mut app, vec![]);
        assert!(!app.cmd_palette.open, "初始应关闭");

        run_frame(
            &ctx,
            &mut app,
            vec![key_event(egui::Key::K, egui::Modifiers::COMMAND)],
        );
        assert!(app.cmd_palette.open, "Cmd+K 应打开命令面板");

        // 再按一次应关闭（开关式）
        run_frame(
            &ctx,
            &mut app,
            vec![key_event(egui::Key::K, egui::Modifiers::COMMAND)],
        );
        assert!(!app.cmd_palette.open, "再按 Cmd+K 应关闭");
    }

    /// `Esc` 关闭面板，且**不执行任何命令**。
    ///
    /// ⚠️ 这条测试原来只断言"面板关了"，而**真机上它其实没关**：
    /// egui 先把 `Esc` 当"结束文本编辑"处理（清空搜索框），面板却还开着 ——
    /// 用户按了 Esc 以为关了，实际没有。断言太弱，所以没抓到。
    ///
    /// 现在断言搜索框内容（Esc 不该清它）与"面板确实关"。修法是把 `Esc`
    /// 也放进 `raw_input_hook` 拦截（与 `Shift+Tab` 同一原因：egui 会先消费它）。
    #[test]
    fn escape_closes_the_palette_without_running_anything() {
        let (mut app, _rx, _tx) = render_test_app();
        let ctx = new_ctx();
        app.cmd_palette.open();
        app.cmd_palette.query = "quit".into();
        run_frame(&ctx, &mut app, vec![]);
        assert_eq!(app.cmd_palette.query, "quit", "前置：搜索串应在");

        let before = app.transcript.blocks.len();
        run_frame(
            &ctx,
            &mut app,
            vec![key_event(egui::Key::Escape, egui::Modifiers::default())],
        );
        assert!(!app.cmd_palette.open, "Esc 应关闭面板");
        assert_eq!(app.transcript.blocks.len(), before, "Esc 不该执行任何动作");
    }

    /// `Enter` 执行高亮的命令，并关闭面板。
    #[test]
    fn enter_runs_the_selected_command_and_closes() {
        let (mut app, _rx, _tx) = render_test_app();
        let ctx = new_ctx();
        app.cmd_palette.open();
        // 过滤到唯一一条，确定会命中哪个动作
        app.cmd_palette.query = "thinking".into();
        run_frame(&ctx, &mut app, vec![]);
        assert_eq!(app.cmd_palette.selected_command().map(|c| c.name), Some("thinking"));
        let before = app.show_reasoning;

        run_frame(
            &ctx,
            &mut app,
            vec![key_event(egui::Key::Enter, egui::Modifiers::default())],
        );
        assert!(!app.cmd_palette.open, "执行后应关闭");
        assert_ne!(app.show_reasoning, before, "thinking 命令应切换思考轨迹显示");
    }

    /// 方向键在列表里移动高亮（循环）。
    #[test]
    fn arrow_keys_move_the_selection() {
        let (mut app, _rx, _tx) = render_test_app();
        let ctx = new_ctx();
        app.cmd_palette.open();
        run_frame(&ctx, &mut app, vec![]);
        assert_eq!(app.cmd_palette.selected, 0);

        run_frame(
            &ctx,
            &mut app,
            vec![key_event(egui::Key::ArrowDown, egui::Modifiers::default())],
        );
        assert_eq!(app.cmd_palette.selected, 1, "↓ 应下移一项");

        run_frame(
            &ctx,
            &mut app,
            vec![key_event(egui::Key::ArrowUp, egui::Modifiers::default())],
        );
        assert_eq!(app.cmd_palette.selected, 0, "↑ 应上移回来");

        // 从 0 往上应绕到最后
        run_frame(
            &ctx,
            &mut app,
            vec![key_event(egui::Key::ArrowUp, egui::Modifiers::default())],
        );
        let n = app.cmd_palette.matches().len();
        assert_eq!(app.cmd_palette.selected, n - 1, "到顶应绕到最后");
    }

    /// **每个命令都必须真的做点什么** —— 列着却点了没反应比没有更糟。
    ///
    /// 这条逐个执行注册表里的全部命令，断言"界面状态确实变了"。
    /// 漏实现一个动作会在这里被抓住。
    #[test]
    fn every_command_has_an_observable_effect() {
        use crate::commands::COMMANDS;
        for c in COMMANDS {
            let (mut app, _rx, _tx) = render_test_app();
            let before = (
                app.show_reasoning,
                app.mode,
                app.show_help,
                app.quit_requested,
                app.transcript.blocks.len(),
                app.notice.clone(),
            );
            app.run_action(c.action.resolve());
            let after = (
                app.show_reasoning,
                app.mode,
                app.show_help,
                app.quit_requested,
                app.transcript.blocks.len(),
                app.notice.clone(),
            );
            assert_ne!(
                before, after,
                "命令 /{} 执行后界面状态毫无变化（用户会以为坏了）",
                c.name
            );
        }
    }

    /// `clear` 只清屏幕，**不动会话状态**。
    ///
    /// 会话状态（目标、token 累计、待审批）不是"显示内容" —— 一起清掉会让
    /// "目标还在跑"变成"目标没了"，或让审批挂起时的阻塞消失。
    #[test]
    fn clear_transcript_keeps_session_state() {
        let (mut app, _rx, _tx) = render_test_app();
        app.transcript.push_batch(&[
            EventMsg::UserSubmitted { text: "问".into() },
            EventMsg::TurnComplete { input_tokens: 7, output_tokens: 3 },
        ]);
        app.transcript.pending = Some(Pending {
            id: "a".into(),
            detail: "d".into(),
            kind: "write".into(),
        });
        app.transcript.push_batch(&[EventMsg::GoalUpdated { snapshot: goal("g", 1, 2) }]);

        app.run_action(crate::commands::Action::ClearTranscript);

        assert!(app.transcript.blocks.is_empty(), "转录应被清空");
        assert_eq!(app.transcript.total_in, 7, "token 累计是状态，不该被清");
        assert!(app.transcript.pending.is_some(), "待审批是状态，不该被清");
        assert!(app.transcript.goal.is_some(), "目标是状态，不该被清");
    }

    // ─────────── D1：会话栏 ───────────

    /// 一个只用于测试的会话控制：可注入"列表"与"当前"。
    struct FakeSessions {
        list: Vec<neo_session::SessionInfo>,
        current: String,
        switched: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl neo_session::SessionControl for FakeSessions {
        fn list(&self) -> Vec<neo_session::SessionInfo> {
            self.list.clone()
        }
        fn switch(&mut self, id: &str) -> Result<Vec<EventMsg>, String> {
            self.switched.lock().unwrap().push(id.to_string());
            self.current = id.to_string();
            // 返回一段"历史"，模拟内核重建后的事件流
            Ok(vec![
                EventMsg::UserSubmitted { text: format!("来自 {id} 的历史") },
                EventMsg::TurnComplete { input_tokens: 1, output_tokens: 1 },
            ])
        }
        fn create(&mut self) -> Result<String, String> {
            let id = "s-new".to_string();
            self.list.push(neo_session::SessionInfo {
                id: id.clone(),
                title: String::new(),
                records: 0,
                changes: None,
                state: neo_session::SessionState::Empty,
            });
            self.current = id.clone();
            Ok(id)
        }
        fn delete(&mut self, _id: &str) -> Result<bool, String> {
            Ok(true)
        }
        fn current(&self) -> String {
            self.current.clone()
        }
    }

    /// 造一个列表项（测试只关心 id/标题/条数，状态与改动留默认）。
    fn session(id: &str, title: &str, records: usize) -> neo_session::SessionInfo {
        neo_session::SessionInfo {
            id: id.into(),
            title: title.into(),
            records,
            changes: None,
            state: neo_session::SessionState::Empty,
        }
    }

    fn app_with_sessions(
        list: Vec<neo_session::SessionInfo>,
        current: &str,
    ) -> (App, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let (handle, rx, tx) = crate::driver::channel();
        let switched = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let app = App::new(
            handle,
            "测试".into(),
            neo_protocol::ExecMode::Default,
            "m".into(),
            vec![("m".into(), "d".into(), true)],
            Some(Box::new(FakeSessions {
                list,
                current: current.into(),
                switched: switched.clone(),
            })),
        );
        std::mem::forget((rx, tx)); // 保住通道两端，避免 drain 塞 Error
        (app, switched)
    }

    /// **当前会话必须在列表里，哪怕它还没有文件。**
    ///
    /// 真机实测的问题：`new_id()` 刻意不建文件（惰性创建），于是刚点过"新建"
    /// 的会话不在 `list()` 里 —— 界面回了"已新建会话 X"，侧栏却仍显示"暂无
    /// 会话"，用户看不到也点不到自己刚建的那个。
    #[test]
    fn the_current_session_is_listed_even_when_it_has_no_file_yet() {
        // 列表里没有当前会话（模拟"新建后尚未落盘"）
        let (app, _sw) = app_with_sessions(vec![session("s-old", "旧的", 3)], "s-fresh");
        let current = app.sessions.as_ref().unwrap().current();
        assert_eq!(current, "s-fresh");
        // 渲染侧应把 current 补进列表
        let ctx = new_ctx();
        let mut app = app;
        run_frame(&ctx, &mut app, vec![]); // 不 panic，且侧栏会显示 current
        let mut list = app.sessions.as_ref().unwrap().list();
        if !list.iter().any(|s| s.id == current) {
            list.insert(
                0,
                neo_session::SessionInfo {
                    id: current.clone(),
                    title: String::new(),
                    records: 0,
                    changes: None,
                    state: neo_session::SessionState::Empty,
                },
            );
        }
        assert!(
            list.iter().any(|s| s.id == "s-fresh"),
            "当前会话必须出现在列表里：{list:?}"
        );
    }

    /// 切换会话要真的调 `switch`，并用返回的历史**替换**转录。
    #[test]
    fn switching_session_replaces_the_transcript_with_history() {
        let (mut app, switched) = app_with_sessions(
            vec![
                session("s-a", "甲", 2),
                session("s-b", "乙", 5),
            ],
            "s-a",
        );
        // 先塞一点当前会话的内容
        app.transcript.push_batch(&[EventMsg::UserSubmitted { text: "旧内容".into() }]);
        app.transcript.total_in = 99;

        app.switch_session("s-b".into());

        assert_eq!(
            switched.lock().unwrap().as_slice(),
            &["s-b".to_string()],
            "应调用 SessionControl::switch"
        );
        // 转录被历史替换（旧内容不见了）
        let text = format!("{:?}", app.transcript.blocks);
        assert!(text.contains("来自 s-b 的历史"), "应显示新会话的历史：{text}");
        assert!(!text.contains("旧内容"), "旧会话内容应被替换掉：{text}");
        // token 累计**从零重新起算**：换会话 = 换上下文，旧的 99 不该带过来。
        // 注意不是断言 0 —— 新会话的历史里有一条 TurnComplete（1 token），
        // 它会被重放进去。所以判据是"旧的 99 没了、只剩历史里那一条"。
        assert_eq!(
            app.transcript.total_in, 1,
            "换会话后 token 应只反映新会话的历史（旧的 99 必须丢掉）实际={}",
            app.transcript.total_in
        );
    }

    /// 新建会话也走同一条路：转录清空 + 给出可读反馈。
    #[test]
    fn new_session_clears_the_transcript_and_reports() {
        let (mut app, _sw) = app_with_sessions(vec![], "s-a");
        app.transcript.push_batch(&[EventMsg::UserSubmitted { text: "旧内容".into() }]);
        app.new_session();
        assert!(app.transcript.blocks.is_empty(), "新会话应是空转录");
        assert!(app.notice.is_some(), "应有反馈");
    }

    /// **未接会话库时必须有可见反馈**，不能静默无反应。
    ///
    /// 静默无反应正是"用户以为坏了"的典型。这条由
    /// `every_command_has_an_observable_effect` 先发现（`/new` 什么也没做）。
    #[test]
    fn session_commands_are_honest_when_no_store_is_wired() {
        let (mut app, _rx, _tx) = render_test_app(); // sessions = None
        app.new_session();
        assert!(
            app.notice.is_some(),
            "未接会话库时新建应给出可见反馈，而不是静默无反应"
        );
        app.notice = None;
        app.switch_session("x".into());
        assert!(app.notice.is_some(), "切换同理");
    }

    /// 侧栏开关命令（`/sessions`）。
    #[test]
    fn toggle_sidebar_flips_visibility() {
        let (mut app, _rx, _tx) = render_test_app();
        let before = app.sidebar_open;
        app.run_action(crate::commands::Action::ToggleSidebar);
        assert_ne!(app.sidebar_open, before, "开关命令应切换侧栏");
    }

    /// 面板打开时跑帧不 panic（含空搜索结果）。
    #[test]
    fn drawing_the_open_palette_does_not_panic() {
        let (mut app, _rx, _tx) = render_test_app();
        let ctx = new_ctx();
        app.cmd_palette.open();
        run_frame(&ctx, &mut app, vec![]);
        // 搜不到任何命令
        app.cmd_palette.query = "zzzz-no-such-command".into();
        run_frame(&ctx, &mut app, vec![]);
        assert!(app.cmd_palette.matches().is_empty());
        // 帮助面板
        app.show_help = true;
        run_frame(&ctx, &mut app, vec![]);
    }
    // ─────────── D8：底部命令台 ───────────

    /// 命令台执行：`Op::Shell`，且**输入框随后清空**（便于连着敲下一条）。
    #[test]
    fn terminal_command_submits_and_clears_input() {
        let (mut app, _rx, _tx) = render_test_app();
        app.terminal_input = "ls -la".into();
        app.run_terminal_command();
        assert!(app.terminal_input.is_empty(), "执行后应清空，便于连敲下一条");
    }

    /// 空命令不上报（避免产生一条空命令的工具事件）。
    #[test]
    fn empty_terminal_command_is_a_noop() {
        let (mut app, _rx, _tx) = render_test_app();
        app.terminal_input = "   ".into();
        app.run_terminal_command();
        assert_eq!(app.terminal_input.trim(), "", "空命令应被忽略");
    }

    /// `/terminal` 开关面板。
    #[test]
    fn toggle_terminal_flips_visibility() {
        let (mut app, _rx, _tx) = render_test_app();
        let before = app.terminal_open;
        app.run_action(crate::commands::Action::ToggleTerminal);
        assert_ne!(app.terminal_open, before, "开关命令应切换终端面板");
    }

    /// **命令台与任务输入框是两条独立通道**：
    /// 终端的命令不经模型（`Op::Shell`），任务才交给模型（`BeginTurn`）。
    /// 混淆会让"我想直接跑条命令"变成"花一次模型请求"。
    #[test]
    fn terminal_and_composer_are_separate_channels() {
        let (mut app, _rx, _tx) = render_test_app();
        // 往终端里敲不影响任务输入框
        app.terminal_input = "git status".into();
        app.input = "帮我看看代码".into();
        app.run_terminal_command();
        assert_eq!(app.input, "帮我看看代码", "执行终端命令不该动任务输入框");
        assert!(app.terminal_input.is_empty(), "只清终端自己的输入框");
    }

    /// 终端面板打开时跑帧不 panic（含"还没跑过任何命令"）。
    #[test]
    fn drawing_the_open_terminal_does_not_panic() {
        let (mut app, _rx, _tx) = render_test_app();
        let ctx = new_ctx();
        app.terminal_open = true;
        run_frame(&ctx, &mut app, vec![]);
        // 有工具卡片时也要能画
        app.transcript.push_batch(&[
            EventMsg::ToolCallBegin {
                id: "shell-0".into(),
                name: "bash".into(),
                arguments: serde_json::json!({"cmd": "ls"}),
            },
            EventMsg::ToolCallEnd {
                id: "shell-0".into(),
                exit_code: 0,
                stdout: "a.txt".into(),
                stderr: String::new(),
                truncated: false,
            },
        ]);
        run_frame(&ctx, &mut app, vec![]);
    }
    /// 关闭命令面板后，**焦点必须回到输入框**。
    ///
    /// 真机抓到的缺陷：`Cmd+K` 开面板 → `Esc` 关掉，之后**两个输入框都没有焦点**，
    /// 键盘输入无处可去（表现为"打字没反应"，用户完全不知道发生了什么）。
    /// 覆盖层拿走过焦点，关掉时必须还回去 —— 这是模态 UI 的基本礼节。
    #[test]
    fn closing_the_palette_returns_focus_to_the_composer() {
        let (mut app, _rx, _tx) = render_test_app();
        let ctx = new_ctx();
        // 先让输入框拿到焦点
        run_frame(&ctx, &mut app, vec![]);
        assert_eq!(app.__test_last_focus, Some(egui::Id::new("neo_composer")));

        // 开面板（焦点转到搜索框）
        app.cmd_palette.open();
        run_frame(&ctx, &mut app, vec![]);
        for _ in 0..3 {
            run_frame(&ctx, &mut app, vec![]);
        }
        assert_ne!(
            app.__test_last_focus,
            Some(egui::Id::new("neo_composer")),
            "前置：面板打开时焦点在搜索框"
        );

        // 关掉 → 焦点应回到输入框
        app.cmd_palette.close();
        app.focus_request = Some(FocusTarget::Composer); // 与真实关闭路径一致
        for _ in 0..3 {
            run_frame(&ctx, &mut app, vec![]);
        }
        assert_eq!(
            app.__test_last_focus,
            Some(egui::Id::new("neo_composer")),
            "关掉覆盖层后焦点必须回到输入框（否则打字没反应）"
        );
    }
    /// 打开命令台后，**焦点应在终端输入框**（ZCode 的 `Cmd+J` 语义）。
    ///
    /// 真机实测：打开面板后焦点仍在任务输入框 —— 用户得再点一下终端输入框
    /// 才能敲命令（而我用 AXPress 点击时 egui 甚至收不到焦点转移，
    /// 于是"在终端里敲的命令"被当成任务发给了模型）。
    /// 打开即聚焦可以消掉这一整类问题。
    #[test]
    fn opening_the_terminal_moves_focus_to_its_input() {
        let (mut app, _rx, _tx) = render_test_app();
        let ctx = new_ctx();
        run_frame(&ctx, &mut app, vec![]); // composer 先拿焦点

        app.run_action(crate::commands::Action::ToggleTerminal);
        assert!(app.terminal_open);
        for _ in 0..3 {
            run_frame(&ctx, &mut app, vec![]);
        }
        assert_eq!(
            app.__test_last_focus,
            Some(egui::Id::new("neo_terminal_in")),
            "打开命令台后焦点应在终端输入框，否则敲的命令会进错通道"
        );
    }

    /// 关掉命令台后焦点回到任务输入框（否则打字又没反应）。
    #[test]
    fn closing_the_terminal_returns_focus_to_the_composer() {
        let (mut app, _rx, _tx) = render_test_app();
        let ctx = new_ctx();
        app.run_action(crate::commands::Action::ToggleTerminal);
        for _ in 0..3 {
            run_frame(&ctx, &mut app, vec![]);
        }
        app.run_action(crate::commands::Action::ToggleTerminal); // 关
        for _ in 0..3 {
            run_frame(&ctx, &mut app, vec![]);
        }
        assert_eq!(
            app.__test_last_focus,
            Some(egui::Id::new("neo_composer")),
            "关掉命令台后焦点应回到任务输入框"
        );
    }
    /// **端到端**：经命令面板打开终端后，在输入框里敲的东西必须走 `Op::Shell`。
    ///
    /// # 为什么这条要断言"发出去的 Op"而不是"焦点 id"
    ///
    /// 断言焦点是**间接指标**，而且很容易测错：我第一版就是直接调
    /// `run_action(ToggleTerminal)` + `close()`，**绕过了面板真实的按键处理路径**
    /// ——于是"关闭面板时无条件归还焦点"这个 bug 在测试里根本不出现
    /// （用旧行为跑那条测试照样绿）。
    ///
    /// 真正要保证的是**用户能感知的结果**：在命令台里敲的命令**不该发给模型**。
    /// 所以这里直接看驱动通道上发出去的 `Op` —— 真机验证时我也是这么判断的
    /// （会话日志里出现 `op {"shell": ...}` 而不是 `op {"begin_turn": ...}`）。
    #[test]
    fn a_command_typed_in_the_terminal_goes_to_shell_not_the_model() {
        let (mut app, rx, _tx) = render_test_app();
        let ctx = new_ctx();

        // 1) 用户真实路径：`Cmd+K` → 搜 terminal → `Enter` 执行该命令
        app.cmd_palette.open();
        app.cmd_palette.query = "terminal".into();
        run_frame(&ctx, &mut app, vec![]);
        assert_eq!(
            app.cmd_palette.selected_command().map(|c| c.name),
            Some("terminal"),
            "过滤器应把 terminal 排在第一位"
        );
        // 用**回车事件**走面板自己的处理（`draw_command_palette` 读 ctx.input），
        // 而不是替他调 run_action —— 那正是上一版漏掉的那段代码
        run_frame(
            &ctx,
            &mut app,
            vec![key_event(egui::Key::Enter, egui::Modifiers::default())],
        );
        assert!(app.terminal_open, "面板里的 terminal 命令应打开命令台");
        // 焦点请求在**下一帧**才生效（egui 的 request_focus 语义），
        // 所以要多跑一帧再断言 —— 这是上一版测试的另一个盲点：
        // 它执行完动作立刻断言，那时终端输入框还没画出来。
        for _ in 0..3 {
            run_frame(&ctx, &mut app, vec![]);
        }
        assert_eq!(
            app.__test_last_focus,
            Some(egui::Id::new("neo_terminal_in")),
            "打开后焦点应在终端输入框"
        );

        // 2) 敲一条命令并回车 → 必须是 Op::Shell（不是 BeginTurn）。
        // 注意 `render_test_app` 的句柄是**发到同一通道**的，所以下面
        // 直接从 rx 收 Op 就能看到 UI 侧发了什么。
        app.terminal_input = "echo hi".into();
        run_frame(&ctx, &mut app, vec![key_event(egui::Key::Enter, egui::Modifiers::default())]);

        // 3) 看通道上到底发出去了什么
        let mut sent = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            if let crate::driver::DriverMsg::Op(op) = msg {
                sent.push(op);
            }
        }
        assert!(
            sent.iter().any(|op| matches!(op, neo_protocol::Op::Shell { .. })),
            "命令台里敲的命令必须走 Op::Shell；实际发出：{sent:?}"
        );
        assert!(
            !sent.iter().any(|op| matches!(op, neo_protocol::Op::BeginTurn { .. })),
            "命令台里的命令**绝不能**发给模型（那会白花一次请求）：{sent:?}"
        );
    }
}

