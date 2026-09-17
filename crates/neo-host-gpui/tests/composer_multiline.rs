//! composer **多行语义**的回归用例。
//!
//! # 为什么必须有这一层（它替代了什么）
//!
//! 这个缺陷此前记在缺口表里："输入框是单行（`Input`）。`Shift+Enter` 当前
//! **什么都不做**（既不换行也不提交）"。它的表现与坏掉时几乎一样：用户按
//! `Shift+Enter` 期待换行，屏幕上没反应 —— 于是**不会有人报这个 bug**，
//! 他只会觉得"这个输入框不好用"，然后少用。
//!
//! 真机验它也不划算：启动窗口、手工按键、截图数行数。而语义本身是**可判定的**，
//! 所以用无头窗口 + **真按键派发**断言（不是直接调内部方法 —— 那样绕过了
//! keymap，恰好是这类 bug 的藏身处）。
//!
//! # 关键：用的是**生产那份配置**，不是测试里手写的一份
//!
//! 状态经 `neo_host_gpui::composer_state_for_test` 构造 —— 与 `app.rs` 里
//! `ensure_input` 走同一个 `new_composer_state`。否则产品代码哪天去掉
//! `submit_on_enter(true)`，用例照样绿，而真机上"回车永远提交不了"。
//!
//! # 诚实边界：这里验的是「组件契约 + 我们的配置」，不是「真机手感」
//!
//! 输入框长到 6 行时转录区被顶成什么样、发送按钮贴底好不好看 —— 那需要人看窗口。
//! 本文件只钉住可判定的部分。

// ⚠️ `#[gpui::test]` 宏展开时写死了 `gpui::` 路径 —— 所以这里需要一个名为
// `gpui` 的别名指向门面。**这不是新增依赖**（门禁 U2 只禁止在 Cargo.toml 里
// 声明 gpui 包），而是让宏能找到它的目标。与 `neo-ui/tests/ime_regression.rs`
// 同一做法。
use neo_ui_kit::gpui;
use neo_ui_kit::component::input::{InputEvent, TextareaState};
use neo_ui_kit::gpui::{Entity, TestAppContext, Window, div, prelude::*};
use std::cell::RefCell;
use std::rc::Rc;

/// 一个最小宿主：真渲染一个 composer。
///
/// **复用生产的 `neo_host_gpui::composer_input`**（而不是在测试里拼一份等价
/// 元素）—— 否则"生产接线被删掉"这类退化测试抓不到：用例测的是它自己那份。
struct ComposerProbe {
    state: Entity<TextareaState>,
}

impl ComposerProbe {
    fn new(window: &mut Window, cx: &mut neo_ui_kit::gpui::Context<Self>) -> Self {
        Self { state: cx.new(|cx| neo_host_gpui::composer_state_for_test(window, cx)) }
    }
}

impl neo_ui_kit::gpui::Render for ComposerProbe {
    fn render(
        &mut self,
        _window: &mut Window,
        _cx: &mut neo_ui_kit::gpui::Context<Self>,
    ) -> impl IntoElement {
        // 无模态：与生产里"没有任何面板打开"的正常态一致。
        div().size_full().p_4().child(neo_host_gpui::composer_input(&self.state, false))
    }
}

/// 建一个无头窗口里的 composer，焦点给到它。
///
/// 返回 `(窗口上下文, 状态, 收到的 PressEnter 记录)` —— 第三条用来断言
/// "Enter 确实报出了提交"（生产里宿主据此调 `submit()`）。
///
/// 焦点必须显式给：`simulate_keystrokes` 派发给**当前焦点元素**，
/// 没焦点则按键掉在地上（测试会以一种"什么都没发生"的形式失败）。
fn setup<'a>(
    cx: &'a mut TestAppContext,
) -> (
    &'a mut neo_ui_kit::gpui::VisualTestContext,
    Entity<TextareaState>,
    Rc<RefCell<Vec<bool>>>, // 每次 PressEnter 的 shift 值
) {
    cx.update(neo_ui_kit::init);
    let (probe, cx) = cx.add_window_view(|window, cx| ComposerProbe::new(window, cx));
    let state = cx.read(|app| probe.read_with(app, |p, _| p.state.clone()));
    let submitted: Rc<RefCell<Vec<bool>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = submitted.clone();
    cx.update(|window, cx| {
        // 与生产同一个订阅位置（`ensure_input` 里订阅 PressEnter）。
        // 用 `App::subscribe`（无 window 版）：测试的接收端不需要 window，
        // 而 `&mut App` 上没有 `subscribe_in`。
        cx.subscribe(&state, move |_state, ev: &InputEvent, _cx| {
            if let InputEvent::PressEnter { shift, .. } = ev {
                sink.borrow_mut().push(*shift);
            }
        })
        .detach();
        state.update(cx, |s, cx| s.focus(window, cx));
    });
    (cx, state, submitted)
}

/// **契约 1：Shift+Enter 插入换行（而不是提交）。**
///
/// 这是本用例存在的直接理由 —— 此前它"什么都不做"。修好之后，
/// 一个多行提示词（贴代码、列要点）应当能在框里真的换行。
///
/// 判定用**文本内容**而不是视觉行数：值里出现换行符就是"插入了换行"的直接证据，
/// 不依赖任何渲染细节。
#[neo_ui_kit::gpui::test]
fn shift_enter_inserts_a_newline_instead_of_submitting(cx: &mut TestAppContext) {
    let (cx, state, submitted) = setup(cx);
    cx.simulate_input("第一行");
    cx.simulate_keystrokes("shift-enter");
    cx.simulate_input("第二行");

    cx.read(|app| {
        state.read_with(app, |s, _| {
            assert_eq!(
                s.value(),
                "第一行\n第二行",
                "Shift+Enter 必须插入换行 —— 此前它什么都不做（多行提示词写不了）"
            );
        });
    });
    // 上报的 shift 必须是 true —— 生产据它分流"换行 or 提交"。
    // 若这里变成 false，真机上 Shift+Enter 会**直接提交**（换行变发送）。
    assert_eq!(
        submitted.borrow().as_slice(),
        [true],
        "Shift+Enter 上报的 shift 必须为 true"
    );
}

/// **契约 2：Enter 既不插入换行、又确实报了提交。**
///
/// 两条断言必须**同时**成立，因为修法容易只顾一头：
/// - 只让组件 `submit_on_enter(true)`：`PressEnter` 有了，但平台的 `\n`
///   仍会插进来（**实测就是这个 bug**：回车既提交、又留下一个换行，
///   输入框看着没清干净）。故 `composer_input` 里补了 `stop_propagation`。
/// - 只 `stop_propagation` 而不配 `submit_on_enter`：换行没了，可 Enter 也
///   不再提交 —— 输入框变成"按回车没反应"。
///
/// 所以这条用例的价值在**两边一起断言**：它守的是那两行配置的**配合**。
#[neo_ui_kit::gpui::test]
fn plain_enter_commits_without_leaving_a_stray_newline(cx: &mut TestAppContext) {
    let (cx, state, submitted) = setup(cx);
    cx.simulate_input("任务");
    cx.simulate_keystrokes("enter");

    cx.read(|app| {
        state.read_with(app, |s, _| {
            assert_eq!(
                s.value(),
                "任务",
                "Enter 不得插入换行 —— 否则提交后输入框里留下一个空行"
            );
        });
    });
    assert!(
        !submitted.borrow().is_empty(),
        "Enter 必须报出 PressEnter（宿主据此提交）—— 否则按回车没反应"
    );
}

/// **契约 3：确实是多行组件。**
///
/// 单行 `Input` 上 `Shift+Enter` 无处可换，所以"多行"是契约 1 的前提。
/// 用上游公开的 `is_multi_line()` 断言我们**用对了组件类型**。
#[test]
fn the_composer_is_a_multiline_field() {
    let mut cx = TestAppContext::single();
    cx.update(neo_ui_kit::init);
    let (probe, cx) = cx.add_window_view(|window, cx| ComposerProbe::new(window, cx));
    let state = cx.read(|app| probe.read_with(app, |p, _| p.state.clone()));
    cx.read(|app| {
        state.read_with(app, |s, _| {
            assert!(s.is_multi_line(), "composer 必须是多行（单行即 Shift+Enter 无处可换）");
        });
    });
}

/// **契约 4：多行文本原样保留**（换行与缩进逐字不丢）。
///
/// `value()` 是宿主 `input` 镜像的来源（见 `ensure_input` 的 `Change` 分支），
/// 所以它必须**逐字**保留 —— 模型侧收到的是同一串文本。
#[neo_ui_kit::gpui::test]
fn multiline_text_survives_verbatim(cx: &mut TestAppContext) {
    let (cx, state, _submitted) = setup(cx);
    let text = "第一行\n  缩进的一行\n\n结束";
    cx.simulate_input(text);

    cx.read(|app| {
        state.read_with(app, |s, _| {
            assert_eq!(s.value(), text, "换行与缩进必须逐字保留（模型要收到同一串文本）");
        });
    });
}
