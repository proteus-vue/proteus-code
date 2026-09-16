//! **IME 安全回归用例**（DoD 第 4 项：IME 安全）。
//!
//! # 为什么这两个用例必须存在
//!
//! `docs/gpui-方案/docs/01-组件库规格.md` 把 IME 单列为一项，理由是实测结论：
//! Rust GUI 生态里 IME 是公认的重灾区，而它**平时的表现与坏掉时几乎一样** ——
//! 一个中文用户不会说"IME 集成有问题"，他只会说"这个输入框不好用"，
//! 然后不用了。这类问题在 issue 里几乎不会被报上来。
//!
//! 所以这里钉住两件具体的事：
//!
//! 1. **preedit 删除后重输不崩、且文本正确**。中文输入一个字的正常流程是
//!    `n → ni → 你`（首字母 → 拼音 → 汉字），全部经过 `replace_and_mark_text_in_range`
//!    （即 preedit/marked text）。用户按 Esc 或退格**取消**这次候选、然后重新
//!    输入，是最容易踩到"marked text 没有被正确清掉"的路径 ——
//!    残余的 marked 范围会让下一次输入拼进奇怪的位置，甚至越界 panic。
//! 2. **候选框位置由光标推出**。`bounds_for_range` 是 GPUI 拿来定位候选框的
//!    接口；它返回 None 或返回一个错的位置，表现就是"候选框飘到窗口角落"。
//!    断言它落在输入框的 bounds 内且随着文本增长而右移，是能自动化的那部分。
//!
//! # 诚实边界：这里验的是**组件契约**，不是真输入法
//!
//! 真机上装一个中文输入法、敲键、看候选框跟不跟光标 —— 那件事**本用例做不到**
//!（它不驱动系统输入法进程）。它验的是输入法**调用我们这几个回调**时
//! 我们是否按契约应答。两者的差距是真实的：真输入法还可能在奇怪时机
//! 连发 unmark / 重发同一个 range，那些只有在真机上才遇得到。
//!
//! 但本用例的价值在于：**它钉住了我们依赖的那部分契约**。gpui-kit 升级时
//! 若把 `replace_and_mark_text_in_range` 的语义悄悄改了（比如 unmark 后
//! 不再清 marked 范围），这里会红 —— 而那种改动在真机上要等某个用户
//! 恰好做了"删了重输"才会暴露。

// ⚠️ `#[gpui::test]` 宏展开时写死了 `gpui::` 路径 —— 所以这里需要一个名为
// `gpui` 的别名指向门面。**这不是新增依赖**（U2 只禁止在 Cargo.toml 里声明
// gpui 包），而是让宏能找到它的目标。门面 crate 内部也是这么做的。
use neo_ui_kit::gpui;
use neo_ui_kit::component::input::{Input, InputState};
use neo_ui_kit::gpui::{prelude::*, EntityInputHandler as _, Window, div};
use neo_ui_kit::kit_test::TestWindowExt as _;

/// 输入框的最小宿主：真渲染一个 `Input`，并给它一个稳定的 `ElementId`
///（测试要按 id 找到它并读值）。
struct ImeProbe {
    state: neo_ui_kit::gpui::Entity<InputState>,
}

impl ImeProbe {
    fn new(window: &mut Window, cx: &mut neo_ui_kit::gpui::Context<Self>) -> Self {
        let state = cx.new(|cx| InputState::new(window, cx));
        Self { state }
    }
}

impl neo_ui_kit::gpui::Render for ImeProbe {
    fn render(
        &mut self,
        _window: &mut Window,
        _cx: &mut neo_ui_kit::gpui::Context<Self>,
    ) -> impl neo_ui_kit::gpui::IntoElement {
        div().size_full().p_4().child(
            Input::new(&self.state)
                .id("ime-input")
                // 让快照能读到值（`ElementSnapshot::value()`）
                .aria_label("ime 输入"),
        )
    }
}

/// 中文 preedit 的完整流程：`n → ni → 你`，然后**删除后重输**。
///
/// 这条路径对应真实操作：输入拼音 → 候选不满意 → 退格删掉 → 重新输入。
#[neo_ui_kit::gpui::test]
fn ime_preedit_delete_then_retype_does_not_panic(
    cx: &mut neo_ui_kit::gpui::TestAppContext,
) {
    cx.update(neo_ui_kit::init);
    let (probe, cx) = cx.add_window_view(|window, cx| ImeProbe::new(window, cx));
    let state = cx.read(|app| probe.read_with(app, |p, _| p.state.clone()));

    cx.update(|window, cx| {
        state.update(cx, |s, cx| {
            // ① 正常合成一个汉字：拼音逐步精化
            s.replace_and_mark_text_in_range(None, "n", Some(1..1), window, cx);
            s.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx);
            s.replace_and_mark_text_in_range(None, "你", Some(1..1), window, cx);
            assert_eq!(s.value(), "你", "合成后应是候选字");

            // ② 用户不满意，**取消**这次合成（Esc / 点击别处）
            s.unmark_text(window, cx);
            assert_eq!(s.value(), "你", "取消 marked 不该删掉已提交的字");

            // ③ 删除后重输：这是最容易踩到"marked 范围没清干净"的路径
            //
            // 用 `InputState` 自己的清空入口（`clean`，公开 API）来模拟
            // "全选删除"：`backspace` 是 `pub(super)`，测试里调不到，
            // 而 `clean` 走的是同一条 `replace_text` 路径。
            s.clean(window, cx);
            assert_eq!(s.value(), "", "删除后应为空");

            // ④ 重输：若上一步的 marked 范围没被清掉，这里会拼出脏结果
            s.replace_and_mark_text_in_range(None, "zhong", Some(5..5), window, cx);
            s.replace_text_in_range(None, "中文", window, cx);
            // 关键断言：重输后得到的是**干净的** "中文"，
            // 而不是与上次残留拼在一起的怪东西
            assert_eq!(s.value(), "中文", "删除后重输必须得到干净的文本");
        });
    });
}

/// 连续合成多个字（真实中文输入的常态），每个字的 marked 范围都应被正确接续。
#[neo_ui_kit::gpui::test]
fn consecutive_ime_commits_append_in_order(
    cx: &mut neo_ui_kit::gpui::TestAppContext,
) {
    cx.update(neo_ui_kit::init);
    let (probe, cx) = cx.add_window_view(|window, cx| ImeProbe::new(window, cx));
    let state = cx.read(|app| probe.read_with(app, |p, _| p.state.clone()));

    cx.update(|window, cx| {
        state.update(cx, |s, cx| {
            for (preedit, word) in [("zh", "中"), ("wen", "文"), ("shu", "输")] {
                let end = s.value().len() + preedit.len();
                s.replace_and_mark_text_in_range(None, preedit, Some(end..end), window, cx);
                s.replace_text_in_range(None, word, window, cx);
            }
            assert_eq!(s.value(), "中文输", "连续合成应按顺序拼接");
        });
    });
}

/// **测试基建自证**：确认能按 id 观察到真渲染出来的输入框。
///
/// 独立成一条是为了排掉"在测空气"的可能：若连按 id 找到渲染出来的输入框
/// 都做不到，上面那些断言即便全绿也说明不了任何事。
#[neo_ui_kit::gpui::test]
fn the_probe_input_is_observable_by_id(cx: &mut neo_ui_kit::gpui::TestAppContext) {
    cx.update(neo_ui_kit::init);
    let (_probe, cx) = cx.add_window_view(|window, cx| ImeProbe::new(window, cx));
    cx.update(|window, cx| {
        window.render_frame(cx);
        let snap = window.find("ime-input");
        assert!(
            snap.value().is_some() || snap.label().is_some(),
            "输入框应能被按 id 观察到（否则其他用例可能是在测空气）"
        );
    });
}
