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
    button::Button,
    h_flex,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    v_flex, Root,
};
use neo_ui_kit::component::scroll::ScrollableElement as _;
use neo_ui_kit::gpui::{div, prelude::*, px, Context, Entity, IntoElement, Render, Window};

/// 焦点目标（`FocusIntent` 的载荷）。
///
/// 与 egui 宿主**同名同义** —— 两边指的是同一件事，改名会让人以为语义不同。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusTarget {
    /// 任务输入框
    Composer,
}

/// 任务输入框（composer）的**唯一构造点**。
///
/// 抽成函数而不是写在 `ensure_input` 里，是为了让回归用例能钉住**我们的配置**
/// （`tests/composer_multiline.rs`）—— 只测组件默认值证明不了我们用对了它。
/// 这里的每一行都是一条语义，改动即改行为：
///
/// - `submit_on_enter(true)`：**Enter 提交、Shift+Enter 换行**。组件据此决定
///   "插不插换行"，并把两者都报成 `InputEvent::PressEnter { shift }` ——
///   宿主只按 `shift` 分流，不自己解析按键（否则就有了第二套判定）。
///   不设它（默认 `false`）时 Enter 也换行，于是**永远提交不了**。
/// - `auto_grow(1, 6)`：从 1 行长到 6 行，超出后内部滚动 —— 贴一段代码
///   不该把转录区顶没，而只给 1 行又让多行输入失去意义。
fn new_composer_state(
    window: &mut Window,
    cx: &mut neo_ui_kit::gpui::Context<TextareaState>,
) -> TextareaState {
    TextareaState::new(window, cx)
        .submit_on_enter(true)
        .auto_grow(1, 6)
        .placeholder("输入任务后回车提交（Shift+Enter 换行；@文件 / $技能 可用）")
}

/// composer 的输入元素（**生产唯一构造点**，`pub` 以便回归用例复用同一份接线）。
///
/// # 为什么要拦住 Enter 的"插入字符"通道（端到端实测抓到的真缺陷）
///
/// `submit_on_enter(true)`（见 `new_composer_state`）让 Enter 走**提交**语义：
/// 组件不插换行，而是把 `InputEvent::PressEnter { shift }` 报给宿主。但平台还会
/// 把同一个 Enter **再当作文本输入送一遍** —— macOS 平台层给 Enter 设的
/// `key_char` 就是 `"\n"`（`gpui-pre-macos` 的 `events.rs`；测试里
/// `with_simulated_ime` 同理），而组件在"提交"分支里会 `cx.propagate()`，
/// 于是这个 `\n` 照样被插进框里。
///
/// 现象：**回车既提交、又留下一个换行** —— 输入框看起来没清干净（光标停在第二
/// 行），下一个任务接着往下写。屏幕不报错、不崩溃，最难发现的那类退化。
///
/// 修法：在冒泡经过这里时 `stop_propagation()`，于是 `dispatch_keystroke` 跳过
/// key_char 分支。`PressEnter` 在**那之前**就已派发，所以提交不受影响 ——
/// 回归用例同时钉住这两条（`tests/composer_multiline.rs`）。
/// Shift+Enter **不拦**：它本就该插换行，且组件在换行分支里不 propagate。
///
/// `modals_open`：命令面板/帮助/模型选择器开着时不拦 —— 那些面板自己要拿 Enter
/// 确认（见根容器的 `KeyArbiter`）。由调用方在渲染时算好传进来，这样本函数
/// 是纯接线、可被测试直接复用。
pub fn composer_input(
    state: &Entity<TextareaState>,
    modals_open: bool,
) -> impl IntoElement {
    div()
        .flex_1()
        .on_key_down(move |ev, _window, cx| {
            if modals_open {
                return;
            }
            if ev.keystroke.key.as_str() == "enter" && !ev.keystroke.modifiers.shift {
                cx.stop_propagation();
            }
        })
        .child(
            // 多行：高度由 `auto_grow(1, 6)` 决定（见 `new_composer_state`），
            // 所以**不设固定 `.h()`** —— 设了就固定住不再长。
            Textarea::new(state)
                // 无边框外观：它是应用里唯一的输入区，套一个框反而像网页表单，
                // 与转录区的连续排版割裂。
                .appearance(false)
                .aria_label("任务输入"),
        )
}

/// 界面状态（Entity）。
pub struct NeoView {
    handle: KernelHandle,
    transcript: Transcript,
    /// 任务输入框当前文本（与 `input_state` 同步）。
    ///
    /// 保留这个镜像而不是每次去读 `InputState`：`submit()` 与冒烟钩子
    /// （`NEO_GUI_PROMPT` 直接写字符串）都走它，读起来是普通 `String`，
    /// 不必到处传 `window`。
    input: String,
    /// **真实文本输入框**（gpui 的 `TextareaState`，多行）。
    ///
    /// ⚠️ 曾经这里是**纯展示**的一行 `div` —— 于是 gpui 宿主根本没法用键盘
    /// 输入任务，只能靠 `NEO_GUI_PROMPT` 喂（egui 侧一直是真 `TextEdit`）。
    /// 这是 gpui 转正最主要的拦路石，比任何 D 项都硬。
    ///
    /// ⚠️ 又曾经是**单行** `InputState`：`Shift+Enter` 什么都不做（既不换行
    /// 也不提交），而 composer 天然要能写多行提示词（贴代码、列要点）。
    /// 多行的语义由 `submit_on_enter` 一处决定，见 `ensure_input`。
    ///
    /// 惰性创建：`TextareaState::new` 要 `&mut Window`，而视图实体在窗口之前
    /// 就建好了 —— 所以第一次渲染时补上。
    input_state: Option<Entity<TextareaState>>,
    /// 输入框事件订阅。**必须持有**：`Subscription` 一 drop 就退订，
    /// 表现为"打字没反应"或"回车不提交"。
    input_subs: Vec<neo_ui_kit::gpui::Subscription>,
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
    /// 侧栏（目标 + 会话）是否展开。
    ///
    /// 默认展开。与 egui 侧的 `sidebar_open` 同名同义 —— 窄窗口下把它收起
    /// 能把宽度让给转录区（这是转录区唯一可用的横向空间来源）。
    sidebar_open: bool,
    /// 帮助面板是否打开。
    ///
    /// 与命令面板同构。**内容从共享命令表读**（`neo_driver::commands`），
    /// 不手写一份清单 —— 手写的会在加命令时忘记更新，而"面板里列了
    /// 但实际没有"和"实际有但没列"都是错误信息。
    help_open: bool,
    /// 模型选择器是否打开。
    ///
    /// 与命令面板同构（覆盖式面板 + 可点选），但**内容不同**：它要显示
    /// 每个模型的说明与"是否桩"。此前 gpui 只有一个左右切换的按钮 ——
    /// 用户看不到有哪些可选、更看不出 `mock` 是桩（切过去以为模型坏了）。
    model_picker_open: bool,
    /// 命令面板是否打开（D9）。
    cmd_open: bool,
    /// **D7** 目标输入框内容（每行一个子任务）。
    ///
    /// 缺了它，gpui 宿主**根本没有设定目标的入口** —— 面板上写着
    /// "未设定（用 /goal 设定）"，而界面上并不存在能设定它的地方（`/goal`
    /// 是 TUI 的命令行语法），用户被告知了一条走不通的路。
    goal_input: String,
    /// 目标输入框（惰性建，理由同任务输入框）。**多行**（`TextareaState`）：
    /// 目标天然是多行的（每行一个子任务），单行框会把换行吃掉。
    goal_state: Option<Entity<neo_ui_kit::component::input::TextareaState>>,
    goal_subs: Vec<neo_ui_kit::gpui::Subscription>,
    /// **D3 思考轨迹搜索**串。
    ///
    /// 搜的是"想不起来的某段推理"，所以范围**只到思考块**：
    /// 工具输出动辄上万行，全量搜一遍既慢又几乎不是用户想要的。
    reasoning_query: String,
    /// 搜索输入框（惰性建，理由同任务输入框）。
    search_state: Option<Entity<InputState>>,
    search_subs: Vec<neo_ui_kit::gpui::Subscription>,
    /// 每轮的 token 用量（按 `TurnComplete` 出现顺序）。
    ///
    /// 与 `turn_durations` 同一处收集、同样的理由：这是**宿主的派生状态**，
    /// 不进共享转录模型（模型里有 `TurnSummary` 块，但那是给转录区逐行显示
    /// 数字用的；画图需要的是序列，重复解析块列表既慢又容易错位）。
    turn_usages: Vec<neo_ui_render::UsageBar>,
    /// 转录区的滚动句柄。
    ///
    /// ⚠️ 用 `track_scroll` + `overflow_y_scroll` + `vertical_scrollbar`
    /// **三项组合**，而不是组件库的 `overflow_y_scrollbar()`：
    /// 后者内部自己创建并持有句柄，调用方拿不到它 —— 而我们需要读
    /// "用户是不是在底部"来决定要不要自动跟随。三种写法等价（组件库内部
    /// 也是这个组合），区别只在于**句柄归谁**。
    transcript_scroll: neo_ui_kit::gpui::ScrollHandle,
    /// 自动跟随状态（逻辑在 `neo-ui-behavior::scroll`，可无头测试）。
    follow_tail: neo_ui_behavior::FollowTail,
    /// 脚本化验证：每帧报告滚动位置与跟随判定（`NEO_GUI_SCROLL`）。
    ///
    /// 为什么需要它：自动跟随的正确性有一半在**符号约定**上 ——
    /// gpui 的 `offset()` 是负值，若直接喂进判定会让"是否在底部"恒为假，
    /// 而纯逻辑单测全绿（测试里的输入都是正的）。这类错只有把**真实的
    /// 框架返回值**打出来才能确认。它与 `NEO_GUI_FOCUS` 同一个理由：
    /// 滚动状态在截图里看不出来（截图是静止的）。
    scroll_report: bool,
    /// 脚本化验证：每帧报告输入框焦点状态（`NEO_GUI_FOCUS`）。
    focus_report: bool,
    /// 上一帧是否处于"审批未决"（用于识别"刚刚解除阻塞"这一刻）。
    composer_blocked_last: bool,
    /// **焦点意图**（`neo-ui-behavior::FocusIntent`）。
    ///
    /// 用共享层的单一槽位而不是两个布尔：
    /// 曾经（egui 宿主）用"面板要不要焦点"+"输入框要不要焦点"两个标志，
    /// 两个都置位时**谁赢取决于代码顺序** —— 真机上表现为"命令被当成任务
    /// 发给模型"（见 `neo-ui-behavior::focus` 的注释）。单一槽位就没有这个问题。
    focus_intent: neo_ui_behavior::FocusIntent<FocusTarget>,
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
    models: Vec<neo_driver::transcript::ModelChoice>,
    /// 是否折叠思考轨迹（D3）。
    show_reasoning: bool,
    /// **D6** 轮次计时。
    ///
    /// 计时在**宿主**做，不进共享转录模型 —— 挂钟时间会破坏回放确定性
    /// （同一份日志回放必须得到同一个转录）。见 `neo_ui_behavior::clock`。
    clock: neo_ui_behavior::TurnClock,
    /// 每轮耗时（按 `TurnComplete` 出现顺序追加，与 `Block::TurnSummary` 对齐）。
    turn_durations: Vec<Option<std::time::Duration>>,
    /// 脚本化验证用：强制展开所有工具组（`NEO_GUI_EXPAND`）。
    ///
    /// 为什么需要它：分组标题是自绘的行，**不进无障碍树**，而 gpui 窗口在
    /// WindowServer 里没有稳定身份（`bundle_id` 为空）→ 自动化点击这条路
    /// 走不通（实测报 "no stable WindowServer app/window identity"）。
    /// 于是展开态的渲染就只能靠人眼点、或者靠这个开关。与
    /// `NEO_GUI_PROMPT` / `NEO_GUI_PANEL` 同一个理由：canvas 收不到
    /// 合成输入，脚本化验证必须留一个入口。
    force_expand_groups: bool,
    /// 冒烟截图用的就绪信号（`NEO_GUI_SHOT`）。
    ///
    /// 外部脚本无法直接问"这一轮跑完了吗"，只能用固定 `sleep` 猜 —— 猜短了
    /// 截到半截画面，猜长了白等。改成：**这一轮真正结束时把窗口标题改成
    /// `NEO-SMOKE-READY`**，标题可以从系统里读到，于是等待变成有条件的。
    /// 值 = 是否已经观察到"跑起来过"（否则首帧的状态就满足"不在运行"）。
    shot_armed: Option<bool>,
    // **D4** 工具组的折叠状态**不在这里** —— 它按块下标记账，必须与
    // `clear_view` 一起被清理，所以和 `collapsed_reasoning` 同放共享层
    // （见 `neo_driver::transcript::Transcript::expanded_tool_runs`）。
    //
    // 默认**折叠**大于 1 的组：连续 5 次工具调用各带参数与输出会把转录淹掉，
    // 而用户此刻要看的是正文。单个调用不做分组外壳（套一层反而多一次点击）。
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
        models: Vec<neo_driver::transcript::ModelChoice>,
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
            clock: neo_ui_behavior::TurnClock::new(),
            turn_durations: Vec::new(),
            turn_usages: Vec::new(),
            force_expand_groups: std::env::var("NEO_GUI_EXPAND").is_ok(),
            shot_armed: std::env::var("NEO_GUI_SHOT").ok().map(|_| false),
            transcript: Transcript::new(),
            input: String::new(),
            reasoning_query: String::new(),
            goal_input: String::new(),
            goal_state: None,
            goal_subs: Vec::new(),
            focus_report: std::env::var("NEO_GUI_FOCUS").is_ok(),
            scroll_report: std::env::var("NEO_GUI_SCROLL").is_ok(),
            transcript_scroll: neo_ui_kit::gpui::ScrollHandle::new(),
            follow_tail: neo_ui_behavior::FollowTail::new(),
            composer_blocked_last: false,
            focus_intent: neo_ui_behavior::FocusIntent::new(),
            search_state: None,
            search_subs: Vec::new(),
            input_state: None,
            input_subs: Vec::new(),
            mode,
            model,
            notice: None,
            status,
            // 调试开关：启动即展开某个面板，便于脚本化截图验证
            // （与 NEO_GUI_PROMPT 同一理由：画布无法靠自动化工具输入，
            //   面板的可见性只能由环境变量驱动）
            model_picker_open: std::env::var("NEO_GUI_PANEL").ok().as_deref() == Some("models"),
            help_open: std::env::var("NEO_GUI_PANEL").ok().as_deref() == Some("help"),
            // 脚本化验证用：启动即折叠侧栏（`NEO_GUI_SIDEBAR=off`）。
            // 与 `NEO_GUI_PANEL` 同族 —— 自绘元素收不到合成点击时，
            // 需要环境变量驱动一次以便截图核对（理由同 NEO_GUI_EXPAND）。
            sidebar_open: std::env::var("NEO_GUI_SIDEBAR").ok().as_deref() != Some("off"),
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
            match ev {
                EventMsg::ModelSwitched { model, .. } => self.model = model.clone(),
                // **D6 计时**：边界由宿主自己判定（它才知道哪个事件算一轮的开始/结束）。
                // 这里刻意不用 `transcript.running` —— 那是共享模型的状态，
                // 而计时的起点要精确到"看到 TurnStarted 的那一刻"。
                EventMsg::TurnStarted { .. } => {
                    self.clock.start(elapsed_now());
                }
                EventMsg::TurnComplete { input_tokens, output_tokens } => {
                    self.turn_durations.push(self.clock.elapsed_since_start(elapsed_now()));
                    self.turn_usages
                        .push(neo_ui_render::UsageBar::new(*input_tokens, *output_tokens));
                    self.clock.finish();
                }
                _ => {}
            }
        }
        self.transcript.push_batch(&events);
        // **D7 目标自动推进**：由"看到最新快照的一方"驱动（与 egui/Web 同一约定）。
        //
        // 缺了这一步的后果很隐蔽：目标能设定、子任务清单也画出来了，
        // 但**永远只有第一个子任务会跑** —— 界面看起来一切正常，
        // 只是停在那里不动。来源放在 pump（而非 render）是为了
        // "只有真有事件时才可能推进"，与 egui 宿主一致。
        //
        // `should_advance_goal` 自带去重（按 (goal_id, iterations) 记账）：
        // 否则每收到一批事件都会再下发一次 GoalAdvance，把同一个子任务
        // 跑很多遍。
        if self.transcript.should_advance_goal() {
            self.transcript.mark_advanced();
            self.handle.send(Op::GoalAdvance);
        }
        true
    }

    /// 惰性建输入框并订阅它的事件。
    ///
    /// 只在第一次渲染时做 —— `TextareaState::new` 需要 `&mut Window`，
    /// 而视图实体先于窗口存在。
    fn ensure_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.input_state.is_some() {
            return;
        }
        let state = cx.new(|cx| new_composer_state(window, cx));
        // 订阅必须在**持有 Subscription** 的前提下才有意义（drop 即退订）
        let sub = cx.subscribe_in(
            &state,
            window,
            |this, state, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change => {
                    this.input = state.read(cx).value().to_string();
                    cx.notify();
                }
                InputEvent::PressEnter { shift, .. } => {
                    // `shift` 为真 = **组件已经插入了换行**（见 `submit_on_enter`），
                    // 该换行就留在框里，不提交。
                    if *shift {
                        return;
                    }
                    this.submit();
                    this.clear_input_box(window, cx);
                }
                _ => {}
            },
        );
        self.input_state = Some(state);
        self.input_subs = vec![sub];
        // **首帧自动聚焦**：这是聊天式界面，打开就该能直接打字。
        // 不聚焦则用户得先点一下输入框 —— 纯键盘流里很别扭。
        self.focus_intent.request(FocusTarget::Composer);
    }

    /// 清空输入框（**两条提交路径都必须调它**）。
    ///
    /// 回车与"发送"按钮是两个入口，各清一半是典型的"改一处忘一处"：
    /// 点按钮提交后输入框里还留着上一句，用户会以为没发出去、再点一次，
    /// 于是同一句话被提交两遍。
    fn clear_input_box(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.clear();
        if let Some(state) = self.input_state.clone() {
            state.update(cx, |s, cx| s.set_value("", window, cx));
        }
    }

    /// 惰性建"搜索思考"输入框并订阅（理由同 `ensure_input`）。
    ///
    /// 与命令面板一样，搜索是**面板级**的：只要有命中就跳出一行"N 处匹配"，
    /// 没有命中就说没有 —— 不悄悄什么都不显示（"搜了没反应"会被当成坏掉）。
    fn ensure_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.search_state.is_some() {
            return;
        }
        let state = cx.new(|cx| {
            InputState::new(window, cx).placeholder("搜索思考轨迹（不区分大小写）")
        });
        let sub = cx.subscribe_in(
            &state,
            window,
            |this, state, ev: &InputEvent, _window, cx| {
                if matches!(ev, InputEvent::Change) {
                    this.reasoning_query = state.read(cx).value().to_string();
                    cx.notify();
                }
            },
        );
        self.search_state = Some(state);
        self.search_subs = vec![sub];
    }

    /// 切换模型（选择器点选时调用）。
    fn pick_model(&mut self, name: &str) {
        self.model = name.to_string();
        self.model_picker_open = false;
        self.handle.send(Op::ConfigureSession {
            patch: neo_protocol::SessionPatch {
                model: Some(name.to_string()),
                ..Default::default()
            },
        });
        self.notice = Some(format!("已切换到 {name}"));
    }

    /// 目标控制（暂停/恢复/清除）。
    ///
    /// 与 egui 宿主同一个做法：都经 `Op` 走内核，不在宿主侧臆测目标状态
    ///（宿主自己改 `goal` 会与内核的快照漂移）。
    fn goal_action(&mut self, op: Op) {
        self.handle.send(op);
    }

    /// **D7** 设定目标（多行输入：每行一个子任务，由内核拆解）。
    fn set_goal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.goal_input.trim().to_string();
        if text.is_empty() {
            return;
        }
        self.handle.send(Op::GoalSet { goal: text });
        self.notice = Some("已提交目标（等待内核拆解）".into());
        // 清空输入框，理由同任务输入框（否则同一目标会被再提交一次）
        self.goal_input.clear();
        if let Some(state) = self.goal_state.clone() {
            state.update(cx, |s, cx| s.set_value("", window, cx));
        }
        cx.notify();
    }

    /// 惰性建目标输入框（多行：目标天然是多行的）。
    fn ensure_goal_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.goal_state.is_some() {
            return;
        }
        let state = cx.new(|cx| {
            neo_ui_kit::component::input::TextareaState::new(window, cx)
                .placeholder("每行一个子任务")
        });
        let sub = cx.subscribe_in(
            &state,
            window,
            |this, state, ev: &InputEvent, _window, cx| {
                if matches!(ev, InputEvent::Change) {
                    this.goal_input = state.read(cx).value().to_string();
                    cx.notify();
                }
            },
        );
        self.goal_state = Some(state);
        self.goal_subs = vec![sub];
    }

    fn submit(&mut self) {
        let text = self.input.trim().to_string();
        // 审批未决时不接受新任务（避免把下一步排进队列）
        if text.is_empty() || self.transcript.pending.is_some() {
            return;
        }
        // **不在这里清输入** —— 清理由 `clear_input_box` 一处负责。
        // 各清各的正是"改一处忘一处"的来源（见该方法的注释）。
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
                // 打开**选择器**而不是直接切到下一个模型。
                //
                // 原先是"循环到下一个"（照 egui 的按钮语义），现在改成列出
                // 全部可选：盲切的问题是想切到第 3 个得按 2 次，且途中会经过
                // 桩模型（真按下去就切过去了）。两个入口做同一件事、行为不同
                // 比只有一个入口更糟，所以统一到这里。
                self.model_picker_open = !self.model_picker_open;
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
                self.sidebar_open = !self.sidebar_open;
            }
            A::NewSession => self.new_session(),
            // 清屏：只清**屏幕上的**转录，不动会话日志
            A::ClearTranscript => {
                self.transcript.clear_view();
                self.notice = Some("已清空屏幕转录（会话日志保留）".into());
            }
            A::Help => {
                // 打开帮助**面板**而不是弹一行提示：快捷键与命令列表放在
                // 提示行里会被下一条提示冲掉，用户来不及看完。
                self.help_open = !self.help_open;
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

/// 进程启动至今的时长。
///
/// `TurnClock` 用的是 `Duration`（从零起算的时长）而不是 `Instant` ——
/// 这样它是**纯数据**、可注入假值做确定性测试。
/// 这里把真实的单调时钟折算成同一个表示。
fn elapsed_now() -> std::time::Duration {
    use std::sync::OnceLock;
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed()
}

/// 变更条元素：**经渲染缝**绘制（不直接用 gpui 的绘制 API）。
///
/// # 为什么这里值得绕一层
///
/// 变更条是"自绘表面"（图形，不是文字），正是渲染缝存在的理由。
/// 直接调 `window.paint_quad` 也能画出来 —— 但那样这条缝就永远没有消费者，
/// 也就永远验证不了它是否可用（方案称之为"只有一个实现的抽象是信仰"）。
///
/// 分工：本函数负责**把 diff 文本映射成中立标记**（这一步依赖 diff 语义，
/// 只能由认识 diff 的层做），`neo-ui-render` 负责"标记 → 中立场景 → 后端产物"。
fn gutter_element(diff: &str) -> impl IntoElement {
    use neo_ui_render::{change_gutter, GutterMark, RenderBackend};

    // diff 文本 → 中立标记（渲染层不认识 diff 语义，这一步必须在宿主做）
    let marks: Vec<GutterMark> = diff
        .lines()
        .map(|line| match neo_driver::transcript::diff_line_kind(line) {
            neo_driver::transcript::DiffLineKind::Add => GutterMark::Add,
            neo_driver::transcript::DiffLineKind::Del => GutterMark::Del,
            // 文件头/hunk 头/说明行都不是"改动内容"，归为 Plain ——
            // 把它们画进色带会让"改动在哪"失真
            _ => GutterMark::Plain,
        })
        .collect();

    // 场景与后端产物在 prepaint 里算（纯计算，不需要 window）
    let prepaint = move |bounds: neo_ui_kit::gpui::Bounds<neo_ui_kit::gpui::Pixels>,
                         _window: &mut neo_ui_kit::gpui::Window,
                         _cx: &mut neo_ui_kit::gpui::App| {
        let h = f32::from(bounds.size.height);
        let scene = change_gutter(&marks, 4.0, h);
        (bounds, neo_ui_render::GpuiBackend::new().paint(&scene))
    };
    // 绘制在 paint 里做（那里才有 Window）
    neo_ui_kit::gpui::canvas(
        prepaint,
        |bounds, (_, paint), window, _cx| {
            paint.draw(bounds.origin, window);
        },
    )
    .w(px(4.))
    .h_full()
}

/// diff 文本 → **显示行**：分类（diff 语法 → 中立行种类）+ 折叠（收起成片未改）。
///
/// # 为什么抽成函数
///
/// 这是"宿主接线"里唯一有判断的部分（其余是把结果塞进 element）。抽出来就能
/// 无头断言三件事：分类是否**完整**（`Header`/`Meta` 不能被当成 `Context`，
/// 否则折叠会把文件头与"被截断"的说明藏掉）、折叠是否被真的调用、以及
/// **底带序列是否与显示行同长**（不同长就会错位）。
///
/// 渲染回调里做不了这些断言 —— 那里只能靠真机截图。
///
/// 返回 `(每行的中立种类, 显示行, 与显示行一一对应的底带种类)`。
/// 最后一个与 `rows` 等长是**硬不变量**：底带按它铺、文字按 `rows` 排，
/// 两者必须同长才不会错位。
fn diff_display_rows(
    lines: &[&str],
    expanded: &std::collections::HashSet<usize>,
) -> (
    Vec<neo_ui_render::DiffBand>,
    Vec<neo_ui_behavior::FoldRow>,
    Vec<neo_ui_render::DiffBand>,
) {
    // 分类必须**完整**：行为层的折叠靠它区分"未改的上下文"（可折）与
    // "文件头 / 截断说明"（不可折）。写成 `_ => Context` 就会把后两者混进去。
    let bands: Vec<neo_ui_render::DiffBand> = lines
        .iter()
        .map(|line| match neo_driver::transcript::diff_line_kind(line) {
            neo_driver::transcript::DiffLineKind::Add => neo_ui_render::DiffBand::Add,
            neo_driver::transcript::DiffLineKind::Del => neo_ui_render::DiffBand::Del,
            neo_driver::transcript::DiffLineKind::Hunk => neo_ui_render::DiffBand::Hunk,
            neo_driver::transcript::DiffLineKind::Header => neo_ui_render::DiffBand::Header,
            neo_driver::transcript::DiffLineKind::Meta => neo_ui_render::DiffBand::Meta,
            neo_driver::transcript::DiffLineKind::Context => neo_ui_render::DiffBand::Context,
        })
        .collect();

    let rows = neo_ui_behavior::fold_rows(&bands, expanded);

    // 折叠**改变了显示行数**，所以底带必须按 `rows` 重算，不能沿用 `bands` ——
    // 否则底带还按原始行数铺，会与前缀被折掉的文字错位。
    let display_bands: Vec<neo_ui_render::DiffBand> = rows
        .iter()
        .map(|r| match r {
            neo_ui_behavior::FoldRow::Line(i) => bands[*i],
            neo_ui_behavior::FoldRow::Fold { .. } => neo_ui_render::DiffBand::Fold,
        })
        .collect();

    debug_assert_eq!(
        rows.len(),
        display_bands.len(),
        "底带必须与显示行同长，否则两者错位"
    );
    (bands, rows, display_bands)
}

/// 把一行按**强调区间**切成若干段，返回 `(文本, 是否强调)`。
///
/// # 为什么需要它（行内高亮的最后一步）
///
/// 只按行着色时，一行 200 字符里改了一个词，**整行都是绿的** —— 用户还得自己
/// 逐字找。行内强调就是把真正变化的那几个字再点亮一次。
///
/// "哪些字变了"由共享层（`neo_driver::transcript::inline_emphasis`）算出，
/// 两个 GUI 宿主共用同一套判定（否则同一份 diff 在两处强调的位置不同）；
/// 这里只负责**把它切成可直接渲染的段**。
///
/// # 切分按**字节边界**，且不信任输入
///
/// 区间来自共享层（那边有测试保证落在 char 边界上）。这里仍做越界/乱序检查 ——
/// 不是不信任，而是这段代码跑在**渲染路径**上：一个越界切片会 panic 掉整帧，
/// 表现为"窗口突然空白"，比"少高亮一处"严重得多。所以防御地退回整行普通显示。
fn split_by_emphasis<'a>(
    line: &'a str,
    emphasis: Option<&neo_driver::transcript::InlineEmphasis>,
) -> Vec<(&'a str, bool)> {
    let Some(e) = emphasis else {
        return vec![(line, false)];
    };
    if e.is_empty() {
        return vec![(line, false)];
    }
    let mut out = Vec::with_capacity(e.ranges.len() * 2 + 1);
    let mut cursor = 0usize;
    for r in &e.ranges {
        // 越界 / 乱序 / 空区间 → 放弃强调（整行普通显示），绝不 panic 整帧。
        //
        // ⚠️ `r.start >= r.end` 这一条**不能省**：只判越界的话，`3..1` 这种倒序
        // 区间会通过检查，然后在下面 `&line[r.clone()]` 上 panic（切片要求
        // start <= end）。本仓库的测试就是这么抓出它的 —— 缺了这条，
        // 一个上游手滑就能崩掉整个窗口。
        if r.start >= r.end || r.start < cursor || r.end > line.len() {
            return vec![(line, false)];
        }
        if r.start > cursor {
            out.push((&line[cursor..r.start], false));
        }
        out.push((&line[r.clone()], true));
        cursor = r.end;
    }
    if cursor < line.len() {
        out.push((&line[cursor..], false));
    }
    out
}

/// **自绘 diff 背景带**（渲染缝的第四个消费者）：让"哪些行改了"一眼扫得出。
///
/// # 分工：**缝画底、宿主画字**
///
/// 底带是矩形填充（缝的强项），文字仍在 element 树里（保住 gpui 的文本整形、
/// 字形缓存与 element diff）。所以本函数只负责底，正文由调用方用普通
/// `div` 叠在上面。
///
/// # 入参是**中立行种类**，不是 diff 文本
///
/// 由调用方分类（宿主认识 diff 语法，渲染层不认识）。而且折叠**改变了显示行数** ——
/// 调用方传进来的必须是"折叠后的显示行"，不是原始 diff 行；否则底带会与前缀
/// 被折掉的文字错位。
///
/// # 对齐：靠**同一个行高**，不靠调间距
///
/// gpui 里 `absolute` 定位的元素**不参与父容器的布局**，所以这层底垫在下面
/// 不会把文字顶走。但"底的一行"与"字的一行"要严丝合缝，靠的是两边共用
/// 一个行高：
/// - 底：`i * lh`（由 `diff_backdrop` 保证）；
/// - 字：文本容器设 `.line_height(lh)` 且每行 `.whitespace_nowrap()`
///   —— 那样"一行文字"恰好占 `lh` 高。
///
/// `lh` 取 `Window::line_height()`（当前文本样式的真实行高），两边同源。
/// **不要**改成"看起来差不多"的魔数：换字号/换字体立刻错开，
/// 而错位的背景带会把改动标到错误的行上，比不画更糟。
fn diff_backdrop_element(
    bands: &[neo_ui_render::DiffBand],
    line_height: f32,
) -> impl IntoElement {
    use neo_ui_render::{diff_backdrop, RenderBackend};

    let bands = bands.to_vec();
    // 场景与后端产物在 prepaint 里算（纯计算，不需要 window）
    let prepaint = move |bounds: neo_ui_kit::gpui::Bounds<neo_ui_kit::gpui::Pixels>,
                         _window: &mut neo_ui_kit::gpui::Window,
                         _cx: &mut neo_ui_kit::gpui::App| {
        let scene = diff_backdrop(&bands, f32::from(bounds.size.width), line_height);
        (bounds, neo_ui_render::GpuiBackend::new().paint(&scene))
    };
    // 绘制在 paint 里做（那里才有 Window）
    neo_ui_kit::gpui::canvas(
        prepaint,
        |bounds, (_, paint), window, _cx| {
            paint.draw(bounds.origin, window);
        },
    )
    .absolute()
    .top(px(0.))
    .left(px(0.))
    .right(px(0.))
    .h_full()
}

/// 目标阶段 → 进度段（`0..=5` 步中的第几步）。
///
/// 五个阶段（Plan→Code→Review→Learn→Done）天然是一个有序的推进过程，
/// 所以"走到第几步了"就是进度。映射放在宿主：渲染层不认识 `GoalPhase`
/// （那是协议类型，会破坏 UI 栈的可提取性）。
fn goal_progress_segments(snapshot: &neo_protocol::GoalSnapshot) -> Vec<neo_ui_render::ProgressSegment> {
    snapshot
        .subtasks
        .iter()
        .map(|st| {
            let done = match st.phase {
                neo_protocol::GoalPhase::Plan => 1,
                neo_protocol::GoalPhase::Code => 2,
                neo_protocol::GoalPhase::Review => 3,
                neo_protocol::GoalPhase::Learn => 4,
                neo_protocol::GoalPhase::Done => 5,
            };
            // 总步数固定为 5：这是阶段**枚举的基数**，不是"这个子任务要几步"。
            // 用固定基数才能让不同子任务的段长一致（否则段长随阶段数变化，
            // 一眼看不出谁更靠前）。
            neo_ui_render::ProgressSegment::new(done, 5)
        })
        .collect()
}

/// 分段进度条元素：**渲染缝的第三个真实消费者**。
///
/// 与变更条、用量图同构：宿主持有原始数据 → 映射成中立类型 →
/// 经 `RenderBackend` 出场景 → 在 canvas 的 paint 回调里提交。
fn progress_chart_element(segments: Vec<neo_ui_render::ProgressSegment>) -> impl IntoElement {
    use neo_ui_render::{segmented_progress, RenderBackend};

    let track_color = neo_ui_render::Color::from_tone(&neo_text::palette::NEO, Tone::Border);
    let fill_color = neo_ui_render::Color::from_tone(&neo_text::palette::NEO, Tone::Success);

    let prepaint = move |bounds: neo_ui_kit::gpui::Bounds<neo_ui_kit::gpui::Pixels>,
                         _window: &mut neo_ui_kit::gpui::Window,
                         _cx: &mut neo_ui_kit::gpui::App| {
        let scene = segmented_progress(
            &segments,
            f32::from(bounds.size.width),
            f32::from(bounds.size.height),
            track_color,
            fill_color,
        );
        (bounds, neo_ui_render::GpuiBackend::new().paint(&scene))
    };
    neo_ui_kit::gpui::canvas(
        prepaint,
        |bounds, (_, paint), window, _cx| {
            paint.draw(bounds.origin, window);
        },
    )
    .w_full()
    .h(px(6.))
}

/// 用量条形图元素：**渲染缝的第二个真实消费者**。
///
/// 与变更条（第一个消费者）同构：宿主持有原始数据 → 映射成中立类型 →
/// 经 `RenderBackend` 出场景 → 在 canvas 的 paint 回调里提交。
/// 区别只在于数据来源（这里来自事件流而不是 diff 文本）。
fn usage_chart_element(bars: Vec<neo_ui_render::UsageBar>) -> impl IntoElement {
    use neo_ui_render::{usage_bars, RenderBackend};

    // 颜色在**宿主**决定（渲染层不认识语义色调，这样它能随 UI 栈开源）。
    //
    // 直接用 `Color::from_tone`：它是"语义色调 → RGB"的那一处实现，
    // 不必绕道 gpui 的 `Rgba` 再解回来（绕一圈要处理 0-1 浮点与字节序，
    // 而那条路上任何一步写错都只表现为"颜色不对"，不会有编译错误）。
    let input_color = neo_ui_render::Color::from_tone(&neo_text::palette::NEO, Tone::Info);
    let output_color = neo_ui_render::Color::from_tone(&neo_text::palette::NEO, Tone::Accent);

    let prepaint = move |bounds: neo_ui_kit::gpui::Bounds<neo_ui_kit::gpui::Pixels>,
                         _window: &mut neo_ui_kit::gpui::Window,
                         _cx: &mut neo_ui_kit::gpui::App| {
        let scene = usage_bars(
            &bars,
            f32::from(bounds.size.width),
            f32::from(bounds.size.height),
            input_color,
            output_color,
        );
        (bounds, neo_ui_render::GpuiBackend::new().paint(&scene))
    };
    neo_ui_kit::gpui::canvas(
        prepaint,
        |bounds, (_, paint), window, _cx| {
            paint.draw(bounds.origin, window);
        },
    )
    .w_full()
    .h(px(36.))
}

/// 把一行带色调的片段渲染成一个元素（复用 `neo-text` 的 Markdown 解析）。
///
/// 用 `StyledText::with_highlights` 而不是逐片段 `div().child()`：
/// 前者把它当**一行文字**参与排版（换行、基线正确），后者是并排的盒子，
/// 中英混排时会各占各的宽度、断行位置全错。
fn styled_line(spans: &[(String, Tone)]) -> impl IntoElement {
    // 归并、色调翻译、区间语义这三件事都在 `neo_ui::rich_text` 里做 ——
    // 宿主只负责"给内容"。原先这里是手写的偏移累加，与
    // `reasoning_highlighted` 各写了一遍同样的区间逻辑。
    neo_ui::rich_text(&neo_ui::RichText::from_spans(spans)).into_any_element()
}

/// 思考块带搜索高亮：把匹配区间标成项目主色。
///
/// 用 `StyledText::with_highlights` 而不是拼多个 `div` —— 与 `styled_line`
/// 同一个理由：它要作为**一行文字**参与排版（中文里搜索时，逐片段拼盒子会
/// 让断行位置全错）。区间是**字节**偏移（`search_reasoning` 已保证落在
/// char 边界上，否则这里会 panic 或错位）。
fn reasoning_highlighted(
    text: &str,
    ranges: &[std::ops::Range<usize>],
) -> neo_ui_kit::gpui::AnyElement {
    // 思考块正文用 Muted，命中区间加高亮背景。
    // 命中处**保留** Muted 前景（不是换成固定色）—— 见 `rich_text` 的说明。
    let rt = neo_ui::RichText::new(text)
        .span(0..text.len(), Tone::Muted)
        .marks(ranges);
    neo_ui::rich_text(&rt).into_any_element()
}

/// 单个工具调用的卡片。
///
/// 抽成自由函数，是为了让"组内展开"与"零散单次调用"复用同一份渲染 ——
/// 两处各画一遍，迟早会画出两种样子（同一个工具在折叠与展开时颜色不同）。
fn tool_card(c: &neo_driver::transcript::ToolCard) -> impl IntoElement {
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
    // 左缩进：让"属于同一组"这件事在视觉上成立，而不只靠上面那行标题
    let mut col = v_flex().gap_1().pl_3().child(
        h_flex()
            .gap_2()
            .child(
                div()
                    .text_color(neo_color(Tone::Primary))
                    .child(format!("▸ {}", c.name)),
            )
            .child(div().text_color(neo_color(tone)).child(state)),
    );
    if !c.args.is_empty() {
        col = col.child(
            div()
                .text_color(neo_color(Tone::Muted))
                .child(c.args.clone()),
        );
    }
    // stderr 非空时**只**显示 stderr：两者拼在一起，用户分不清哪句是失败的
    // 原因、哪句是之前的正常输出。内核已按此约定填这两个字段。
    let body = if c.stderr.is_empty() { &c.stdout } else { &c.stderr };
    // 裁掉**尾部**空白再显示：命令行输出几乎总以换行结尾，原样画出来就是
    // 卡片底部多一个空行，一张张叠起来节奏全乱（真机截图看出来的）。
    // 只裁尾部 —— 前导缩进是输出内容的一部分（缩进的日志/JSON 有意义）。
    let body = body.trim_end();
    if !body.is_empty() {
        col = col.child(
            div()
                .text_color(neo_color(if c.stderr.is_empty() {
                    Tone::Text
                } else {
                    Tone::Error
                }))
                .child(body.to_string()),
        );
    }
    if c.truncated {
        // 截断必须**说出来**：内核如实截了，界面不说，用户就以为那是全部。
        // 内存有界是内核义务，如实上报是界面义务。
        col = col.child(
            div()
                .text_color(neo_color(Tone::Warning))
                .child("（输出已截断）"),
        );
    }
    col
}

/// 转录区：把 `Block` 画出来。
/// 转录区。**是方法而不是自由函数** —— 它需要访问折叠状态、并且要拿实体
/// 来挂点击回调（思考块的折叠/展开）。自由函数只能拿到数据快照。
impl NeoView {
    /// `line_height`：当前文本样式的真实行高，由 `render` 传入（那里有 `Window`）。
    /// diff 背景带的 y 与文字容器的 `.line_height()` 都必须用它 —— 两边同源才
    /// 对得齐（见 `diff_backdrop_element` 的对齐说明）。
    fn transcript_view(&self, cx: &mut Context<Self>, line_height: f32) -> impl IntoElement {
        let blocks = &self.transcript.blocks;
        // **D4 工具分组**：连续的工具调用合成一组。
        //
        // 判定逻辑在共享层（`neo_driver::transcript::tool_runs`），宿主只做展示决策 ——
        // 两个宿主必须给出同一套分组，否则同一个转录在两个窗口里长得不一样，
        // 那是最难向用户解释的一类不一致。
        //
        // **只有 ≥2 个调用的组才有外壳**：单个调用套一层"1 次工具调用"，
        // 只是多一次点击，没有半点信息增量。
        // **D3 搜索**：命中块 → 下标 → 字节区间（高亮用）。
        //
        // 判定在共享层（`search_reasoning`）：两个宿主必须给出同一套命中，
        // 否则"同一个查询在两边结果数不同"是最难解释的一类不一致。
        let hits = neo_driver::transcript::search_reasoning(blocks, &self.reasoning_query);
        let hit_map: std::collections::HashMap<usize, Vec<std::ops::Range<usize>>> = hits
            .iter()
            .map(|h| (h.block, h.ranges.clone()))
            .collect();
        let mut group_of: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
        let mut group_runs: std::collections::HashMap<usize, neo_driver::transcript::ToolRun> =
            std::collections::HashMap::new();
        for run in neo_driver::transcript::tool_runs(blocks) {
            if run.len >= 2 {
                for i in run.start..run.start + run.len {
                    group_of.insert(i, run.start);
                }
                group_runs.insert(run.start, run);
            }
        }
        let mut col = v_flex().gap_1().p_3();
        for (idx, b) in blocks.iter().enumerate() {
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
                    // 折叠语义与 egui 宿主**完全一致**（同一份状态）：
                    //   折叠 = 全局开关关掉 **或** 这一块被单独折叠
                    // 全局关掉是"我现在不想看思考"，逐块折叠是"这一块太长"——
                    // 两个独立的意图，必须叠加而不是互相覆盖。
                    let has_hit = hit_map.contains_key(&idx);
                    // 命中块强制展开（见上面 must_expand 的理由）
                    let collapsed = !has_hit
                        && (!self.show_reasoning
                            || self.transcript.collapsed_reasoning.contains(&idx));
                    let arrow = if collapsed { "▸" } else { "▾" };
                    let head = format!("{arrow} 思考（{} 字）", t.chars().count());
                    let v = cx.entity().clone();
                    // 标题可点击：切换**这一块**的折叠
                    let header = div()
                        .id(("reasoning-head", idx))
                        .text_color(neo_color(Tone::Muted))
                        .child(head)
                        .on_click(move |_, _, cx| {
                            v.update(cx, |this, cx| {
                                if this.transcript.collapsed_reasoning.contains(&idx) {
                                    this.transcript.collapsed_reasoning.remove(&idx);
                                } else {
                                    this.transcript.collapsed_reasoning.insert(idx);
                                }
                                cx.notify();
                            });
                        });
                    col = col.child(header);
                    if !collapsed {
                        // 命中时走**高亮渲染**（把匹配区间标成项目色），
                        // 未命中时保持原来的单色 —— 不给普通内容加视觉噪音。
                        let el = match hit_map.get(&idx) {
                            Some(ranges) => reasoning_highlighted(t, ranges),
                            None => div()
                                .text_color(neo_color(Tone::Muted))
                                .child(t.clone())
                                .into_any_element(),
                        };
                        col = col.child(el);
                    }
                }
                // 组内非首块：跳过（已由组的首块代表整组渲染）。
                //
                // 在渲染层做而不是把 `Block` 预先合并：合并会让"展开某一组"
                // 变成要改数据，而分组是纯展示决策。
                Block::Tool(_) if group_of.get(&idx).is_some_and(|s| *s != idx) => {}

                Block::Tool(_) if group_runs.contains_key(&idx) => {
                    let run = &group_runs[&idx];
                    let expanded =
                        self.force_expand_groups || self.transcript.expanded_tool_runs.contains(&idx);
                    let all_ok = neo_driver::transcript::run_all_succeeded(blocks, run);
                    let in_progress = neo_driver::transcript::run_in_progress(blocks, run);
                    // 整组的色调取决于**最坏的那个**：一组绿字里藏着一个失败，
                    // 用户必然漏看 —— 这是信息层次里最容易骗人的一处。
                    let (mark, tone) = if in_progress {
                        ("⏳", Tone::Info)
                    } else if all_ok {
                        ("✓", Tone::Success)
                    } else {
                        ("✗", Tone::Error)
                    };
                    let arrow = if expanded { "▾" } else { "▸" };
                    let v = cx.entity().clone();
                    col = col.child(
                        div()
                            .id(("tool-group", idx))
                            .text_color(neo_color(tone))
                            .child(format!("{arrow} {mark} {} 次工具调用", run.tool_count()))
                            .on_click(move |_, _, cx| {
                                v.update(cx, |this, cx| {
                                    // `HashSet::remove` 返回"是否真的移除了"，
                                    // 正好当作折叠/展开的切换，不必先查再插。
                                    if !this.transcript.expanded_tool_runs.remove(&idx) {
                                        this.transcript.expanded_tool_runs.insert(idx);
                                    }
                                    cx.notify();
                                });
                            }),
                    );
                    if expanded {
                        for b in &blocks[run.start..run.start + run.len] {
                            if let Block::Tool(c) = b {
                                col = col.child(tool_card(c));
                            }
                        }
                    }
                }

                // 单个调用：直接一张卡，不套组外壳（套一层反而多一次点击）。
                Block::Tool(c) => {
                    col = col.child(tool_card(c));
                }

                Block::Diff { path, diff } => {
                    col = col.child(
                        div()
                            .text_color(neo_color(Tone::Info))
                            .child(format!("改动 {path}")),
                    );
                    // **渲染缝的第一个真实消费者**：变更条是自绘表面（不是文字），
                    // 所以它走 `RenderBackend`。
                    //
                    // ⚠️ 它必须与 diff **正文**并排、且占满正文的高度 ——
                    // 第一版把它放进了标题行，于是 `h_full()` 只等于一行文本高，
                    // 变更条被压成 4×14px 的一小块（真机截图看出来的）：
                    // 那个尺寸表达不了"改动分布"这个唯一的用途。
                    //
                    // 行高从**当前文本样式**取（不是魔数）：底带的 y 由它算，
                    // 文字容器的 `.line_height(lh)` 也用它 —— 两边同源才对得齐。
                    let lh = line_height;
                    let lines: Vec<&str> = diff.lines().collect();

                    // 折叠状态按 `(块下标, 区间起点)` 记 —— 一个 diff 可以有多处被折的
                    // 未改区，只用块下标会让"展开任一处 = 全部展开"。
                    let expanded: std::collections::HashSet<usize> = self
                        .transcript
                        .expanded_diff_folds
                        .iter()
                        .filter(|(b, _)| *b == idx)
                        .map(|(_, s)| *s)
                        .collect();

                    // diff 文本 → 显示行（分类 + 折叠）抽成纯函数，见其说明。
                    let (bands, rows, display_bands) = diff_display_rows(&lines, &expanded);

                    // **行内强调**：把"真正变化的字"再点亮一次。判定在共享层
                    // （`inline_emphasis`），两个 GUI 宿主共用同一套 —— 否则同一份
                    // diff 在两处强调的位置不同（那是最难解释的一类不一致）。
                    //
                    // ⚠️ 入参是**全部 diff 行**（配对要跨行看），不是折叠后的显示行；
                    // 下标与 `lines` 一一对应，所以下面用原始行下标取。
                    let emphases = neo_driver::transcript::inline_emphasis(&lines);
                    // 强调样式从**统一来源**取（与自绘底带同一份调色板派生）
                    let emph_style = neo_ui_render::DiffBandStyle::from_neo_palette();

                    let mut body = v_flex()
                        .gap_0()
                        // 文字容器的行高 = 底带用的行高（对齐靠这一句）
                        .line_height(px(lh));
                    for row in &rows {
                        match row {
                            neo_ui_behavior::FoldRow::Line(i) => {
                                let line = lines[*i];
                                let tone = match bands[*i] {
                                    neo_ui_render::DiffBand::Add => Tone::Success,
                                    neo_ui_render::DiffBand::Del => Tone::Error,
                                    neo_ui_render::DiffBand::Hunk => Tone::Info,
                                    neo_ui_render::DiffBand::Meta => Tone::Muted,
                                    _ => Tone::Text,
                                };
                                // 切段：改动处用**粗体**强调。为什么不用另一种颜色 ——
                                // 那会让"这一行是绿的（新增）"这条信息被冲淡；忽明忽暗的
                                // 色相也让"哪行是增、哪行是删"更难扫。粗体只加强、不改语义。
                                //
                                // ⚠️ **必须放在 `h_flex` 里**：外层 `body` 是**纵向** flex，
                                // 直接把各段塞进去会让每一段各占一行（真机上看到的正是
                                // "行首的 `-` 单独一行、内容另起一行"）。段是**同一行的
                                // 片段**，所以要用横向容器让它们并排。
                                let mut row_el = h_flex()
                                    .gap_0()
                                    // 不折行：折行会让"一行文字"占两行高，底带立刻错位。
                                    // 长行由裁切/横向滚动处理。
                                    .whitespace_nowrap()
                                    .text_color(neo_color(tone));
                                // 强调色：**更浓的行底**，压在变更片段下面。
                                // 用 `.bg()` 而不是自己算宽度再画矩形 —— div 会按
                                // 文本自身尺寸撑开，不必测量（少一处会随字体/字号
                                // 漂移的计算）。行高由父容器统一给定，所以这块底
                                // 与底带同高。
                                let emph_bg = emph_style.emphasis_for(bands[*i]);
                                for (text, is_emph) in
                                    split_by_emphasis(line, emphases[*i].as_ref())
                                {
                                    let mut seg = div().child(text.to_string());
                                    if is_emph {
                                        if let Some(bg) = emph_bg {
                                            seg = seg.bg(neo_ui_render::to_gpui_rgba(bg));
                                        }
                                    }
                                    row_el = row_el.child(seg);
                                }
                                body = body.child(row_el);
                            }
                            neo_ui_behavior::FoldRow::Fold { range } => {
                                // 把手：点一下展开（再点收起）。文案给出**确切行数** ——
                                // "⋯" 这种含糊提示会让人不知道折了多少。
                                let n = neo_ui_behavior::folded_line_count(range);
                                let start = range.start;
                                let v = cx.entity().clone();
                                body = body.child(
                                    div()
                                        // 元素 id 要唯一：同一块 diff 里可能有多处折叠区。
                                        // `ElementId` 只接受 `(&str, usize)` 这类形状，所以把
                                        // 两个下标**无冲突地**打包进 u64（块下标占高 32 位）。
                                        // 不用 `a*n+b` 那种乘加：它会在某些取值上撞号，
                                        // 而 id 撞号的表现是"点一处展开、另一处也动"。
                                        .id(("diff-fold", ((idx as u64) << 32) | (start as u64)))
                                        .whitespace_nowrap()
                                        .text_color(neo_color(Tone::Info))
                                        .child(format!("⋯ 未改 {n} 行（点击展开）"))
                                        .on_click(move |_, _, cx| {
                                            v.update(cx, |this, cx| {
                                                let key = (idx, start);
                                                if !this
                                                    .transcript
                                                    .expanded_diff_folds
                                                    .remove(&key)
                                                {
                                                    this.transcript
                                                        .expanded_diff_folds
                                                        .insert(key);
                                                }
                                                cx.notify();
                                            });
                                        }),
                                );
                            }
                        }
                    }
                    // 正文叠在**底带之上**：底带 `absolute` 不参与布局（不顶走文字），
                    // 且它没有落在无障碍树里 —— 底带只是视觉层，不承载信息，
                    // 语义由文字与变更条承担。
                    let body_with_backdrop = div()
                        .relative()
                        .child(diff_backdrop_element(&display_bands, lh))
                        .child(body);
                    col = col.child(
                        h_flex()
                            .items_start()
                            .gap_0()
                            .child(gutter_element(diff))
                            .child(body_with_backdrop),
                    );
                }
                Block::TurnSummary { input_tokens, output_tokens } => {
                    // **D6 耗时**：把第 n 个轮摘要与第 n 个记录的耗时配对。
                    // 用**计数**而不是块下标对齐：转录里还夹着别的块，
                    // 按下标索引会在任何一次"块类型变化"后错位（悄悄显示错的耗时）。
                    let nth = self
                        .transcript
                        .blocks
                        .iter()
                        .take(idx + 1)
                        .filter(|b| matches!(b, Block::TurnSummary { .. }))
                        .count()
                        - 1;
                    let dur = self
                        .turn_durations
                        .get(nth)
                        .copied()
                        .flatten()
                        .map(|d| format!(" · 耗时 {}", neo_ui_behavior::format_duration(d)))
                        .unwrap_or_default();
                    col = col.child(
                        div()
                            .text_color(neo_color(Tone::Muted))
                            .child(format!(
                                "· 本轮完成（{input_tokens} in / {output_tokens} out{dur}）"
                            )),
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
}


/// **D7**：右侧面板 —— 目标 + 会话列表（D1）。
///
/// 两个面板合并成一栏是刻意的：窗口宽度有限，而二者都是"当前上下文"的展示。
/// ZCode 把它们分在左右两侧（各占 240px），那在宽屏上才成立。
fn side_panel(view: &NeoView, cx: &mut Context<NeoView>) -> impl IntoElement {
    let mut col = v_flex()
        // 260 而不是 220：220 会把"标题 ·条数 +3 -1"里最右侧的 `-1` 裁掉
        //（真机截图看出来的 —— 那一行**恰好**在改动行数出现时超宽，
        //  而"改动行数"正是这个列表新加的信息，等于新功能自己不可见）
        .w(px(260.))
        .h_full()
        .gap_2()
        .p_3()
        // ── 用量趋势（渲染缝的第二个自绘消费者）──
        //
        // 放在最上面：它是**全局**信息（这条会话总共花了多少、有没有陡增），
        // 而下面的目标与会话列表是"当前上下文"。
        //
        // 只在有数据时画：一条会话刚开始就摆一个空图表框，会让人以为
        // 功能坏了（而不是"还没有数据"）。
        .child({
            let total: u64 = view.turn_usages.iter().map(|b| b.total()).sum();
            if view.turn_usages.is_empty() {
                div().into_any_element()
            } else if total == 0 {
                // **有轮次但全为 0** 是一个真实且常见的情况：桩 provider
                // 不报 token。此时不能只显示标题 + 空白 —— 那看起来像图表坏了。
                // 说明原因，用户才知道"不是坏了，是这个 provider 不报"。
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_color(neo_color(Tone::Info))
                            .child(format!("用量（{} 轮）", view.turn_usages.len())),
                    )
                    .child(
                        div()
                            .text_color(neo_color(Tone::Muted))
                            .child("本次未报告 token 用量（桩 provider 不报）"),
                    )
                    .into_any_element()
            } else {
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_color(neo_color(Tone::Info))
                            .child(format!("用量（{} 轮）", view.turn_usages.len())),
                    )
                    .child(usage_chart_element(view.turn_usages.clone()))
                    .child(
                        // 图例：两条色带必须有说明，否则读者只能猜哪根是哪根
                        h_flex()
                            .gap_2()
                            .child(div().text_color(neo_color(Tone::Info)).child("■ 输入"))
                            .child(div().text_color(neo_color(Tone::Accent)).child("■ 输出")),
                    )
                    .into_any_element()
            }
        })
        .child(
            div()
                .text_color(neo_color(Tone::Info))
                .child("目标"),
        );

    match &view.transcript.goal {
        None => {
            // **必须在这里能设定**：面板写着"未设定（用 /goal 设定）"却
            // 不给输入框，用户就被指去了一条 GUI 里走不通的路
            //（`/goal` 是 TUI 的命令行语法）。
            col = col.child(
                div()
                    .text_color(neo_color(Tone::Muted))
                    .child("未设定：填子任务后开始"),
            );
            if let Some(state) = view.goal_state.clone() {
                let v = cx.entity().clone();
                col = col
                    .child(div().child(
                        neo_ui_kit::component::input::Textarea::new(&state)
                            .appearance(false)
                            .h(px(72.)),
                    ))
                    .child(Button::new("goal-start").label("开始").on_click(
                        move |_, window, cx| {
                            v.update(cx, |this, cx| this.set_goal(window, cx));
                        },
                    ));
            }
        }
        Some(g) => {
            // 恒用协议层的 summary()：各宿主自己拼会漂移出"同一个目标长两副样子"
            col = col.child(
                div()
                    .text_color(neo_color(Tone::Accent))
                    .child(g.summary()),
            );
            // 分段进度条：每个子任务一段，段内表达走到第几阶段。
            // 文字列表说得出"每个子任务在哪个阶段"，说不出"整体推进了多少" ——
            // 后者要读完全部行再自己数，而色带一眼可见。
            col = col.child(progress_chart_element(goal_progress_segments(g)));
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
            // **暂停 / 恢复 / 清除**：目标一旦立起来，没有这三个按钮就只能靠
            // 重开窗口才能停下它 —— 而目标会持续自动推进（花真钱）。
            // 此前 gpui 侧只有"设定"没有"控制"，是 D7 只做了一半。
            {
                let goal_id = g.goal_id.clone();
                let v_pause = cx.entity().clone();
                let v_clear = cx.entity().clone();
                let paused = g.paused;
                col = col.child(
                    h_flex()
                        .gap_2()
                        .pt_1()
                        .child(
                            Button::new("goal-pause")
                                // 已暂停时按钮文案换成"恢复"，避免用户点了
                                // 一个语义相反却看不出来的按钮
                                .label(if paused { "恢复" } else { "暂停" })
                                .on_click(move |_, _, cx| {
                                    let id = goal_id.clone();
                                    v_pause.update(cx, |this, _| {
                                        this.goal_action(if paused {
                                            Op::GoalResume { goal_id: id }
                                        } else {
                                            Op::GoalPause { goal_id: id }
                                        });
                                    });
                                }),
                        )
                        .child(Button::new("goal-clear").label("清除").on_click(
                            move |_, _, cx| {
                                v_clear.update(cx, |this, _| {
                                    this.goal_action(Op::GoalClear);
                                });
                            },
                        )),
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
        col = col.child(
            div()
                .text_color(neo_color(Tone::Muted))
                .child("暂无会话"),
        );
    }
    for si in list {
        let id = si.id.clone();
        let is_current = id == current;
        // 标题为空（新会话还没起名）用 id 兜底，否则列表里出现空白行
        let shown = if si.title.trim().is_empty() { id.clone() } else { si.title.clone() };
        // **状态圆点**：只对"需要用户注意"的两种状态上色。
        // 正常结束不画圆点（全是绿点等于没有信息）；空会话也不画。
        let (dot, dot_tone) = match si.state {
            neo_session::SessionState::Failed => ("●", Some(Tone::Error)),
            neo_session::SessionState::Interrupted => ("◐", Some(Tone::Warning)),
            _ => ("", None),
        };
        // **改动行数**：只显示增删，不带"改动"之类的字（列表里每行都要短）
        let changes = match si.changes {
            Some((a, d)) if a > 0 || d > 0 => format!(" +{a} -{d}"),
            _ => String::new(),
        };
        let click_id = id.clone();
        let v = cx.entity().clone();
        col = col.child(
            h_flex()
                .gap_1()
                .id(format!("sess-{id}"))
                .child(if let Some(t) = dot_tone {
                    div().text_color(neo_color(t)).child(dot)
                } else {
                    div()
                })
                .child(
                    div()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .text_color(neo_color(if is_current { Tone::Accent } else { Tone::Text }))
                        .child(format!("{shown} ·{}{changes}", si.records)),
                )
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
/// 帮助面板：快捷键 + 全部命令。
///
/// 命令清单**从共享命令表读**（`neo_driver::commands::COMMANDS`），
/// 不在这里手写。手写清单必然在加命令时忘记更新 ——
/// 而"列了但按不出来"和"能按但没列"都是错误信息。
fn help_panel(view: &NeoView, cx: &mut Context<NeoView>) -> Option<impl IntoElement> {
    if !view.help_open {
        return None;
    }
    let mut col = v_flex()
        .w(px(440.))
        .gap_1()
        .p_3()
        .bg(neo_ui::panel_bg())
        .child(
            div()
                .text_color(neo_ui::neo_color(Tone::Info))
                .child("快捷键"),
        );

    // 快捷键：**只列真的实现了的**。列一条按不出来的比不列更糟。
    for (k, v) in [
        ("Cmd/Ctrl+K", "命令面板"),
        ("Shift+Tab", "循环切换执行模式"),
        ("Enter", "提交输入框内容"),
        ("Esc", "关闭最上层面板"),
        ("↑ / ↓", "命令面板内选择"),
    ] {
        col = col.child(
            h_flex()
                .gap_2()
                .child(
                    div()
                        .w(px(96.))
                        .text_color(neo_ui::neo_color(Tone::Muted))
                        .child(k),
                )
                .child(div().text_color(neo_ui::neo_color(Tone::Text)).child(v)),
        );
    }

    col = col.child(
        div()
            .pt_2()
            .text_color(neo_ui::neo_color(Tone::Info))
            .child(format!("命令（{} 条）", neo_driver::commands::COMMANDS.len())),
    );
    for c in neo_driver::commands::COMMANDS {
        col = col.child(
            h_flex()
                .gap_2()
                .child(
                    div()
                        .w(px(96.))
                        .text_color(neo_ui::neo_color(Tone::Accent))
                        .child(format!("/{}", c.name)),
                )
                .child(
                    div()
                        .text_color(neo_ui::neo_color(Tone::Muted))
                        .child(c.desc),
                ),
        );
    }

    let v = cx.entity().clone();
    col = col.child(
        div().pt_2().child(
            Button::new("help-close").label("关闭").on_click(move |_, _, cx| {
                v.update(cx, |this, cx| {
                    this.help_open = false;
                    cx.notify();
                });
            }),
        ),
    );
    Some(col)
}

/// 模型选择器：列出全部模型，带说明与"是否桩"的标记。
///
/// 与命令面板同构（覆盖式、可点选）。**不显示模型名列表就让人左右切换**
/// 是 gpui 侧此前的做法 —— 用户看不到有哪些可选，更看不出某个是桩。
fn model_picker(view: &NeoView, cx: &mut Context<NeoView>) -> Option<impl IntoElement> {
    if !view.model_picker_open {
        return None;
    }
    let mut col = v_flex()
        .w(px(420.))
        .gap_1()
        .p_3()
        .bg(neo_ui::panel_bg())
        .child(
            div()
                .text_color(neo_color(Tone::Text))
                .child("选择模型（点选切换）"),
        );

    if view.models.is_empty() {
        col = col.child(
            div()
                .text_color(neo_color(Tone::Muted))
                .child("没有可用模型"),
        );
    }

    for m in &view.models {
        let is_current = m.name == view.model;
        let name = m.name.clone();
        let v = cx.entity().clone();
        // 桩标记用**文字**而不是只靠颜色：颜色可能被主题吃掉，
        // 而"这个模型不能真跑任务"是必须传达到的信息。
        let label = m.display_name();
        let mut row = h_flex()
            .gap_2()
            .id(format!("model-{}", m.name))
            .child(div().text_color(neo_color(if is_current {
                Tone::Accent
            } else {
                Tone::Text
            }))
            .child(if is_current { format!("● {label}") } else { format!("  {label}") }))
            .on_click(move |_, _, cx| {
                let name = name.clone();
                v.update(cx, |this, cx| {
                    this.pick_model(&name);
                    cx.notify();
                });
            });
        if !m.description.is_empty() {
            row = row.child(
                div()
                    .text_color(neo_color(Tone::Muted))
                    .child(m.description.clone()),
            );
        }
        col = col.child(row);
    }
    Some(col)
}

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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 当前文本样式的真实行高：diff 背景带的 y 与文字行高都取自它。
        // 在 render 里取一次并向下传，而不是在深层函数里各取一次 ——
        // 后者要求每个函数都拿到 `Window`，会把签名污染到整条链路。
        let line_height = f32::from(window.line_height());
        // 0) 输入框（惰性；第一次渲染时 window 才可用）
        self.ensure_input(window, cx);
        self.ensure_search(window, cx);
        self.ensure_goal_input(window, cx);

        // 审批刚答完 → 把焦点还回输入框（用户接着就要打字）。
        // 只在**恢复那一刻**抢一次：每帧都抢会让用户没法把焦点移到搜索框。
        let blocked_now = self.transcript.pending.is_some();
        if self.composer_blocked_last && !blocked_now {
            self.focus_intent.request(FocusTarget::Composer);
        }
        self.composer_blocked_last = blocked_now;

        // 消费焦点意图：**每帧只消费一次**，且只认自己那一个目标。
        // 消费（take）而不是读（peek）是关键 —— 否则每帧都会重新聚焦，
        // 用户一移开焦点就被拽回来。
        if let Some(FocusTarget::Composer) = self.focus_intent.take() {
            if !blocked_now {
                if let Some(state) = self.input_state.clone() {
                    state.update(cx, |s, cx| s.focus(window, cx));
                }
            }
        }

        // 脚本化验证用：报告输入框是否真的拿到了焦点（`NEO_GUI_FOCUS=1`）。
        //
        // 为什么需要它：焦点在界面上**唯一的可见证据是光标**，而光标会闪 ——
        // 单张截图可能正好catch在熄灭帧，据此判"没聚焦"会得出错误结论
        // （反过来，catch到亮帧也不能证明它**一直是**聚焦的）。
        // 直接读 `FocusHandle::is_focused` 是确定性的，且能自动断言。
        if self.focus_report {
            if let Some(state) = self.input_state.clone() {
                // 用 `Focusable` trait（`focus_handle()` 与同名字段并存，
                // 字段遮蔽了方法 → 必须经 trait 调用）
                let focus = neo_ui_kit::gpui::Focusable::focus_handle(
                    &*state.read(cx),
                    cx,
                );
                let focused = focus.is_focused(window);
                eprintln!("[neo] composer_focused={focused}");
            }
        }


        // 1) 收事件（非阻塞）
        // 冒烟钩子：首帧把 NEO_GUI_PROMPT 当作一次提交（只做一次）
        if let Some(text) = self.auto_prompt.take() {
            self.input = text;
            self.submit();
            self.clear_input_box(window, cx);
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
        // 冒烟就绪：先等到"确实运行过"，再等运行结束 —— 只判断后者的话，
        // 提交与 `TurnStarted` 到达之间的那一帧就会被误判成已完成。
        if let Some(armed) = self.shot_armed {
            if self.transcript.running {
                self.shot_armed = Some(true);
            } else if armed && self.transcript.pending.is_none() {
                self.shot_armed = None;
                window.set_window_title("NEO-SMOKE-READY");
            }
        }
        // 3) 轮次进行中：推进一步。
        //
        // 不需要"每帧排一帧"的动画机制：推进本身会产生事件批，
        // 驱动的唤醒钩子（见 `run`）会再触发一次重绘 —— 事件不断则循环自持。
        // 内核忙（暂无事件）时界面停住是对的，那时也没有新内容可画。
        if self.transcript.running && self.transcript.pending.is_none() {
            self.pump_step();
        }

        let running = self.transcript.running;
        let mode = self.mode;
        let model = self.model.clone();
        let status = self.status.clone();
        let notice = self.notice.clone();
        let total = (self.transcript.total_in, self.transcript.total_out);
        let blocked = self.transcript.pending.is_some();

        let view_entity = cx.entity().clone();
        let view_for_submit = cx.entity().clone();
        let view_for_model = cx.entity().clone();

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
                            .id("model-open")
                            .child(format!("模型：{model} ▾"))
                            .on_click(move |_, _, cx| {
                                view_for_model.update(cx, |this, cx| {
                                    this.model_picker_open = !this.model_picker_open;
                                    cx.notify();
                                });
                            }),
                    )
                    // 侧栏开关：窄窗口下把宽度让给转录区。
                    // 它与 `/sessions` 命令等价（同一个 Action），有可点入口
                    // 才不用每次都走命令面板。
                    .child({
                        let v = cx.entity().clone();
                        let label = if self.sidebar_open { "侧栏 ◀" } else { "侧栏 ▶" };
                        div()
                            .id("sidebar-toggle")
                            .text_color(neo_color(Tone::Muted))
                            .child(label)
                            .on_click(move |_, _, cx| {
                                v.update(cx, |this, cx| {
                                    this.run_action(neo_driver::commands::Action::ToggleSidebar);
                                    cx.notify();
                                });
                            })
                    })
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
                    // ⚠️ `min_w(0)` 不能省：转录区是 `flex_1`，而 flex 项的
                    // 默认最小宽度是"内容宽度" —— 于是长转录（不换行的长行）
                    // 会把整个横向布局撑开，**把右侧栏挤出窗口**（真机看到
                    // 侧栏连标题都不见了，而它的数据是好的）。
                    // `min_w(0)` 允许它被压缩到可用宽度以内，内容由滚动条处理。
                    .min_w(px(0.))
                    .min_h(px(0.))
                    .child({
                        // 搜索行在转录**上方**（它作用于转录内容，放侧栏会
                        // 让人以为它只搜侧栏）
                        let search = self.search_state.clone().expect("搜索框应已建好");
                        let n_hits = neo_driver::transcript::search_reasoning(
                            &self.transcript.blocks,
                            &self.reasoning_query,
                        )
                        .len();
                        let searching = !self.reasoning_query.trim().is_empty();
                        v_flex()
                            .flex_1()
                            .h_full()
                            // ⚠️ 与外层 `h_flex` 同一个坑：flex 项默认
                            // `min-width: auto`，不压到 0 的话它会撑到
                            // **内容宽度**（转录里那些不换行的长行），
                            // 从而把侧栏挤出窗口。
                            .min_w(px(0.))
                            .min_h(px(0.))
                            .gap_1()
                            .child(
                                v_flex()
                                    .px_3()
                                    .pt_2()
                                    .gap_1()
                                    .child(
                                        Input::new(&search)
                                            .appearance(false)
                                            .aria_label("搜索思考轨迹"),
                                    )
                                    // 有查询就必须给出结果数：搜了没反应会被当成坏掉
                                    .child(if searching {
                                        div()
                                            .text_color(neo_color(if n_hits > 0 {
                                                Tone::Muted
                                            } else {
                                                Tone::Warning
                                            }))
                                            .child(if n_hits > 0 {
                                                format!("思考轨迹命中 {n_hits} 处")
                                            } else {
                                                "思考轨迹里没有匹配".to_string()
                                            })
                                    } else {
                                        div()
                                    }),
                            )
                            .child({
                                // 自动跟随（流式输出时视图跟着走）。语义是
                                // "**只有用户本来就在底部才跟随**" —— 他往上翻
                                // 就是在读历史，那时自动滚动是干扰。
                                // 判定在 `neo-ui-behavior::scroll`（有测试），
                                // 这里只把当前滚动位置喂给它。
                                let handle = self.transcript_scroll.clone();
                                // ⚠️ 必须走 `from_raw`：gpui 的 `offset()`
                                // 是**负值**（往下滚为负），直接喂给判定会让
                                // `offset >= max_offset` 恒为假 —— 自动跟随
                                // 永远不生效，而纯逻辑单测全绿。
                                let pos = neo_ui_behavior::ScrollPos::from_raw(
                                    handle.offset().y.into(),
                                    handle.max_offset().y.into(),
                                );
                                let follow = self.follow_tail.should_follow(pos);
                                if follow {
                                    handle.scroll_to_bottom();
                                }
                                self.follow_tail.observe(pos);
                                if self.scroll_report {
                                    eprintln!(
                                        "[neo] scroll raw_offset={} raw_max={} -> offset={} max={} at_bottom={} follow={}",
                                        handle.offset().y, handle.max_offset().y,
                                        pos.offset, pos.max_offset, pos.at_bottom(), follow
                                    );
                                }
                                div()
                                    .id("transcript-scroll")
                                    .flex_1()
                                    .min_h(px(0.))
                                    .track_scroll(&handle)
                                    .overflow_y_scroll()
                                    .vertical_scrollbar(&handle)
                                    .child(self.transcript_view(cx, line_height))
                            })
                    })
                    .child(if self.sidebar_open {
                        // 侧栏内容会随"用量图 + 目标 + 进度条 + 会话列表"变长，
                        // 而它**必须有滚动容器** —— 我加第三个自绘组件时
                        // 真机发现目标区块被挤出可视区且滚不到（侧栏此前没有
                        // 滚动，因为内容少到没暴露这个问题）。
                        div()
                            .id("side-scroll")
                            // 宽度写在外层（内层 side_panel 自己的 w 会被
                            // 滚动容器影响 —— 显式给出更可靠）
                            .w(px(260.))
                            .h_full()
                            .min_h(px(0.))
                            .overflow_y_scrollbar()
                            .child(side_panel(self, cx))
                            .into_any_element()
                    } else {
                        div().into_any_element()
                    }),
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
        //
        // 审批未决时**整行换成提示**而不是把输入框置灰：置灰的输入框仍占着
        // 视觉焦点，用户会先去点它、发现打不了字，才回读那行小字。
        let input_row = if blocked {
            h_flex()
                .gap_2()
                .px_3()
                .py_2()
                .child(
                    div()
                        .flex_1()
                        .text_color(neo_color(Tone::Muted))
                        .child("待审批：请先在上方选择（避免把下一步排进队列）"),
                )
        } else {
            let state = self
                .input_state
                .clone()
                .expect("输入框应在第一次渲染时建好（见 ensure_input）");
            let modals_open = self.cmd_open || self.help_open || self.model_picker_open;
            h_flex()
                .gap_2()
                .px_3()
                .py_2()
                .items_end() // 发送按钮贴底：输入框长高时按钮不该跟着浮到中间
                .child(composer_input(&state, modals_open))
                .child(Button::new("send").label("发送").on_click(
                    move |_, window, cx| {
                        view_for_submit.update(cx, |this, cx| {
                            this.submit();
                            // 与回车同一条清理路径 —— 见 `clear_input_box`
                            this.clear_input_box(window, cx);
                        });
                    },
                ))
        };
        root = root.child(input_row);

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
            } else if key == "escape" && v.read(cx).help_open {
                v.update(cx, |this, cx| {
                    this.help_open = false;
                    cx.notify();
                });
            } else if key == "escape" && v.read(cx).model_picker_open {
                v.update(cx, |this, cx| {
                    this.model_picker_open = false;
                    cx.notify();
                });
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
        if let Some(p) = help_panel(self, cx) {
            root = root.child(
                div()
                    .absolute()
                    .top(px(48.))
                    .left(px(120.))
                    .child(p),
            );
        }
        if let Some(p) = model_picker(self, cx) {
            root = root.child(
                div()
                    .absolute()
                    .top(px(48.))
                    .left(px(120.))
                    .child(p),
            );
        }
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
    models: Vec<neo_driver::transcript::ModelChoice>,
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

/// 供回归用例构造**与生产同一份**的 composer 状态。
///
/// 为什么必须走这个出口，而不是让测试自己 `TextareaState::new(...)`：
/// 那样测的是"我在测试里手写的那份配置"，产品代码哪天把
/// `submit_on_enter(true)` 去掉，用例照样绿 —— 于是最关键的
/// "Enter 到底提不提交"没人守。
pub fn composer_state_for_test(
    window: &mut Window,
    cx: &mut neo_ui_kit::gpui::Context<TextareaState>,
) -> TextareaState {
    new_composer_state(window, cx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use neo_protocol::{GoalPhase, GoalSnapshot, GoalSubtask};

    fn snapshot(phases: &[GoalPhase]) -> GoalSnapshot {
        GoalSnapshot {
            goal_id: "goal-1".into(),
            goal: "测试目标".into(),
            paused: false,
            stopped: None,
            subtasks: phases
                .iter()
                .enumerate()
                .map(|(i, ph)| GoalSubtask {
                    id: i,
                    title: format!("子任务 {i}"),
                    phase: *ph,
                    retries: 0,
                })
                .collect(),
            iterations: 0,
            consecutive_failures: 0,
            turns_remaining: 0,
            budget_used: 0,
        }
    }

    /// 切段：强调区间之外的部分仍要原样输出（不能只输出强调部分）。
    #[test]
    fn splitting_keeps_the_whole_line_and_marks_only_the_emphasis() {
        use neo_driver::transcript::InlineEmphasis;
        let line = "-let x = 1;";
        // 「x」在 `-let ` 之后：字节 5..6
        let e = InlineEmphasis { ranges: vec![5..6] };
        let parts = split_by_emphasis(line, Some(&e));
        assert_eq!(
            parts,
            vec![("-let ", false), ("x", true), (" = 1;", false)],
            "整行必须完整保留，只有中间那段被强调"
        );
        // 拼回去必须与原文**逐字相等**（切段不该增删任何字符）
        let joined: String = parts.iter().map(|(t, _)| *t).collect();
        assert_eq!(joined, line);
    }

    /// 没有强调 → 整行一段（不产生多余元素）。
    #[test]
    fn no_emphasis_yields_one_plain_segment() {
        assert_eq!(split_by_emphasis("-x", None), vec![("-x", false)]);
        let empty = neo_driver::transcript::InlineEmphasis { ranges: vec![] };
        assert_eq!(split_by_emphasis("-x", Some(&empty)), vec![("-x", false)]);
    }

    /// **越界 / 乱序的区间必须被拒**（退回整行），绝不能在渲染路径上 panic。
    ///
    /// 这条防的是"一帧崩掉整窗口"：切段跑在 render 里，一个越界切片会 panic
    /// 整个界面，远比"少高亮一处"严重。区间来自共享层、本该合法，
    /// 但渲染路径上的代码不该信任上游（防御式）。
    #[test]
    fn invalid_ranges_are_rejected_instead_of_panicking() {
        use neo_driver::transcript::InlineEmphasis;
        let line = "-abc"; // 4 字节
        for bad in [
            InlineEmphasis { ranges: vec![2..99] },      // 越界
            InlineEmphasis { ranges: vec![3..1] },       // 倒序
            InlineEmphasis { ranges: vec![2..3, 1..2] }, // 乱序
            InlineEmphasis { ranges: vec![99..100] },    // 完全在外
        ] {
            let parts = split_by_emphasis(line, Some(&bad));
            assert_eq!(
                parts,
                vec![(line, false)],
                "非法区间应退回整行普通显示，而不是 panic：{bad:?}"
            );
        }
    }

    /// 强调片段在行首 / 行尾：两侧不留空段。
    #[test]
    fn emphasis_at_the_edges_produces_no_empty_segments() {
        use neo_driver::transcript::InlineEmphasis;
        let line = "abc";
        let head = split_by_emphasis(line, Some(&InlineEmphasis { ranges: vec![0..1] }));
        assert_eq!(head, vec![("a", true), ("bc", false)]);
        let tail = split_by_emphasis(line, Some(&InlineEmphasis { ranges: vec![2..3] }));
        assert_eq!(tail, vec![("ab", false), ("c", true)]);
        let all = split_by_emphasis(line, Some(&InlineEmphasis { ranges: vec![0..3] }));
        assert_eq!(all, vec![("abc", true)], "全是强调则只有一个段");
        // 任何情况下都没有空文本段（渲染空 div 是浪费，也让断言读起来含糊）
        for parts in [head, tail, all] {
            assert!(parts.iter().all(|(t, _)| !t.is_empty()), "不该产生空段");
        }
    }

    /// 切段必须落在**字符边界**上（中文/emoji）—— 否则 `&line[..]` 直接 panic。
    #[test]
    fn splitting_respects_character_boundaries_for_cjk() {
        let line = "-中文旧内容";
        // 用共享层的真实判定拿区间（它保证落在 char 边界）
        let lines = vec!["-中文旧内容", "+中文新内容"];
        let em = neo_driver::transcript::inline_emphasis(&lines);
        let parts = split_by_emphasis(line, em[0].as_ref());
        let joined: String = parts.iter().map(|(t, _)| *t).collect();
        assert_eq!(joined, line, "中文行切段后必须逐字还原");
        assert!(
            parts.iter().any(|(t, e)| *e && *t == "旧"),
            "应强调「旧」这个完整的汉字：{parts:?}"
        );
    }

    /// **接线契约 1**：分类必须**完整** —— 文件头与截断说明不能被当成上下文。
    ///
    /// 这条守的是折叠的正确性：`Header` / `Meta` 若被归成 `Context`（曾经的
    /// `_ => Context` 就是这么写的），折叠会把它们一起藏掉 —— 而"这段 diff 属于
    /// 哪个文件""这个 diff 被截断过"是**结构信息**，藏了会误导。
    #[test]
    fn diff_classification_keeps_header_and_meta_distinct() {
        let lines = vec![
            "--- a/f.txt",   // Header
            "+++ b/f.txt",   // Header
            "@@ -1,3 +1,3 @@", // Hunk
            " ctx",          // Context
            "-gone",         // Del
            "+new",          // Add
            "… 另有 2 处改动未展示", // Meta
        ];
        let (bands, rows, display) = diff_display_rows(&lines, &Default::default());

        assert_eq!(bands[0], neo_ui_render::DiffBand::Header);
        assert_eq!(bands[1], neo_ui_render::DiffBand::Header);
        assert_eq!(bands[6], neo_ui_render::DiffBand::Meta);
        assert_eq!(bands[3], neo_ui_render::DiffBand::Context);

        // 短 diff 不折 → 显示行与原行一一对应
        assert_eq!(rows.len(), lines.len());
        assert_eq!(display.len(), rows.len(), "底带必须与显示行同长");
    }

    /// **接线契约 2**：折叠**真的被调用**了 —— 长片上下文被折起，
    /// 且底带随之同步缩短（不是只折文字、底带还按原行数铺）。
    ///
    /// 用**人工构造**的带长上下文的 diff：我们自己的 `apply_patch` 每 hunk 只带
    /// 3 行上下文，不会触发折叠（见 `neo-ui-behavior::fold` 头部的说明），
    /// 所以这条必须绕过生产者、直接喂形状。
    #[test]
    fn a_long_context_run_is_folded_and_the_backdrop_stays_aligned() {
        let mut lines: Vec<String> = vec!["--- a/f.txt".into(), "+++ b/f.txt".into(), "@@ -1,30 +1,30 @@".into()];
        lines.push("-gone".into());
        for i in 0..25 {
            lines.push(format!(" ctx {i}"));
        }
        lines.push("+added".into());
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();

        let (_bands, rows, display) = diff_display_rows(&refs, &Default::default());

        assert!(
            rows.iter().any(|r| matches!(r, neo_ui_behavior::FoldRow::Fold { .. })),
            "25 行连续上下文应被折起（阈值 {}）",
            neo_ui_behavior::FOLD_THRESHOLD
        );
        assert_eq!(
            display.len(),
            rows.len(),
            "**底带必须与显示行同长** —— 不同长会让底带与文字错位"
        );
        assert!(
            display.len() < refs.len(),
            "折叠后显示行应少于原行数：{} vs {}",
            display.len(),
            refs.len()
        );
        // 折叠行的底带种类必须是 Fold（它要看起来像个可展开的把手）
        let fold_pos = rows
            .iter()
            .position(|r| matches!(r, neo_ui_behavior::FoldRow::Fold { .. }))
            .unwrap();
        assert_eq!(display[fold_pos], neo_ui_render::DiffBand::Fold);
    }

    /// **接线契约 3**：展开态被正确读到 —— 同一个折叠区展开后全部铺开，
    /// 且**仍保留一个把手**（否则没有收起的入口）。
    #[test]
    fn expanding_the_fold_reads_the_expanded_key() {
        let mut lines: Vec<String> = vec!["@@ -1,30 +1,30 @@".into(), "-gone".into()];
        for i in 0..25 {
            lines.push(format!(" ctx {i}"));
        }
        lines.push("+added".into());
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();

        let (_b, collapsed, _d) = diff_display_rows(&refs, &Default::default());
        let start = collapsed
            .iter()
            .find_map(|r| match r {
                neo_ui_behavior::FoldRow::Fold { range } => Some(range.start),
                _ => None,
            })
            .expect("应有折叠区");

        let mut expanded = std::collections::HashSet::new();
        expanded.insert(start);
        let (_b2, rows, display) = diff_display_rows(&refs, &expanded);

        assert!(
            rows.len() > collapsed.len(),
            "展开后显示行应变多：{} vs {}",
            rows.len(),
            collapsed.len()
        );
        assert!(
            rows.iter().any(|r| matches!(r, neo_ui_behavior::FoldRow::Fold { .. })),
            "展开态仍要保留把手，否则没法收起"
        );
        assert_eq!(display.len(), rows.len(), "底带仍须与显示行同长");
    }

    /// 每个子任务一段，且**总步数固定为 5**（阶段枚举的基数）——
    /// 否则段长会随阶段数变化，一眼看不出谁更靠前。
    #[test]
    fn each_subtask_becomes_one_segment_with_a_fixed_total() {
        let g = snapshot(&[GoalPhase::Plan, GoalPhase::Done, GoalPhase::Review]);
        let segs = goal_progress_segments(&g);
        assert_eq!(segs.len(), 3, "三个子任务三段");
        for s in &segs {
            assert_eq!(s.total, 5, "总步数恒为 5");
        }
        assert_eq!(segs[0].done, 1, "Plan 是第 1 步");
        assert_eq!(segs[1].done, 5, "Done 是第 5 步");
        assert_eq!(segs[2].done, 3, "Review 是第 3 步");
    }

    /// 阶段顺序必须**严格递增**地映射到步数 ——
    /// 若两个阶段映射到同一步，进度条上就分不出先后（而那是它的唯一用途）。
    #[test]
    fn phases_map_to_strictly_increasing_steps() {
        let order = [
            GoalPhase::Plan,
            GoalPhase::Code,
            GoalPhase::Review,
            GoalPhase::Learn,
            GoalPhase::Done,
        ];
        let g = snapshot(&order);
        let segs = goal_progress_segments(&g);
        for w in segs.windows(2) {
            assert!(w[1].done > w[0].done, "阶段步数必须严格递增：{segs:?}");
        }
    }

    #[test]
    fn done_phase_maps_to_a_complete_segment() {
        let g = snapshot(&[GoalPhase::Done]);
        let segs = goal_progress_segments(&g);
        assert!(segs[0].is_complete(), "Done 应是完成态（画满整段）");
    }

    #[test]
    fn a_goal_without_subtasks_yields_no_segments() {
        let g = snapshot(&[]);
        assert!(goal_progress_segments(&g).is_empty());
    }
}
