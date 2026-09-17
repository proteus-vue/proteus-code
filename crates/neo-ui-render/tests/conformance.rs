//! 渲染缝的 conformance —— **同一份用例跑所有后端**。
//!
//! # 为什么这条缝需要 conformance（它此前没有）
//!
//! `neo-ui-render` 的头部一直标注着"只有一个实现，所以这不是成熟抽象"。
//! 抽象是否中立，没有别的验证方式：**必须有两个后端、并用同一份断言去跑它们**。
//! 否则"可替换"就只是宣称（方法论文档称之为假 SPI，AP-01/AP-03）。
//!
//! 本文件把那条宣称变成可执行的契约。两个后端的能力矩阵**故意不同**
//! （`GpuiBackend` 画不出文字、`HeadlessBackend` 画得出），所以断言分两类：
//!
//! - **必须一致**：度量、颜色序列、几何、零线宽语义 —— 这些是"同一场景在
//!   不同后端表现相同"的支点；
//! - **允许不同**：`supported()` 的能力边界 —— 那是**后端自己的事实**。
//!
//! # 用真实消费者的场景，不用手搓的玩具场景
//!
//! 下面的场景由 `usage_bars` / `segmented_progress` / `change_gutter`
//! （三个已在用的自绘组件）产生。手搓一个只有两个矩形的场景，会让 conformance
//! 在两个后端"都很容易通过"，从而失去意义。

use neo_text::palette::NEO;
use neo_text::Tone;
use neo_ui_render::{
    change_gutter, color_seq_of_scene, segmented_progress, unsupported_ops, usage_bars, Color,
    GpuiBackend, GutterMark, HeadlessBackend, ProgressSegment, RenderBackend, Scene, UsageBar,
};

/// 现实中会用到的场景集合（取自三个真实自绘消费者 + 两个边界场景）。
fn real_world_scenes() -> Vec<(&'static str, Scene)> {
    let input = Color::from_tone(&NEO, Tone::Primary);
    let output = Color::from_tone(&NEO, Tone::Accent);
    vec![
        (
            "用量条形图（三个组件的真实数据）",
            usage_bars(
                &[UsageBar::new(1200, 340), UsageBar::new(890, 210), UsageBar::new(45, 12)],
                240.0,
                40.0,
                input,
                output,
            ),
        ),
        (
            "分段进度条（目标子任务）",
            segmented_progress(
                &[
                    ProgressSegment::new(1, 5),
                    ProgressSegment::new(5, 5),
                    ProgressSegment::new(3, 5),
                ],
                300.0,
                6.0,
                input,
                output,
            ),
        ),
        (
            "变更条（含新增/删除/未改三种标记）",
            change_gutter(&[GutterMark::Add, GutterMark::Del, GutterMark::Plain], 12.0, 60.0),
        ),
        (
            "diff 背景带（第四个消费者：新增/删除/上下文/hunk 混排）",
            neo_ui_render::diff_backdrop(
                &[
                    neo_ui_render::DiffBand::Hunk,
                    neo_ui_render::DiffBand::Context,
                    neo_ui_render::DiffBand::Del,
                    neo_ui_render::DiffBand::Add,
                    neo_ui_render::DiffBand::Context,
                    neo_ui_render::DiffBand::Add,
                ],
                320.0,
                18.0,
            ),
        ),
        ("空场景（消费者拿到空列表）", Scene::new()),
        (
            "退化输入（全零 / 零尺寸）",
            usage_bars(&[UsageBar::new(0, 0)], 0.0, 0.0, input, output),
        ),
    ]
}

/// 契约 1：**度量必须逐字一致**。
///
/// 度量决定换行位置与行高。各后端各算一套的话，同一段文字在两个后端上
/// 会断在不同地方 —— 这是"渲染缝"最核心的一条，没有它其余都是装饰。
#[test]
fn every_backend_measures_text_identically() {
    let gpui = GpuiBackend::new();
    let headless = HeadlessBackend::new();

    // 含 CJK（双宽）、emoji、中英混排、空串等真实内容
    let samples = [
        ("", 14.0f32),
        ("hello", 14.0),
        ("中文渲染测试", 14.0),
        ("中英 mixed 混排", 12.0),
        ("emoji 🚀 也算宽度", 16.0),
        ("a", 1.0),
        ("a", 96.0),
    ];
    for (text, size) in samples {
        let a = gpui.measure_text(text, size);
        let b = headless.measure_text(text, size);
        assert_eq!(
            a, b,
            "两个后端的度量必须一致：{text:?} @ {size}（gpui={a:?} headless={b:?}）"
        );
    }
}

/// 契约 2：**颜色序列逐项相等** —— 同一场景在不同后端必须"哪一步是什么色"一致。
///
/// 这条比"画出来像不像"更根本：颜色错位是静默的视觉 bug（红蓝互换看着像主题变了），
/// 而顺序对齐能被机器判定。
#[test]
fn every_backend_translates_the_same_color_sequence() {
    let gpui = GpuiBackend::new();
    let headless = HeadlessBackend::new();

    for (name, scene) in real_world_scenes() {
        let expected = color_seq_of_scene(&scene);

        // 无头后端直接给出语义序列（`None` = 裁剪，本就无颜色）
        let got = headless.paint(&scene).color_sequence();
        assert_eq!(
            got, expected,
            "[{name}] 无头后端的颜色序列应与场景一一对应"
        );

        // GPUI 后端用 `0` 表示裁剪占位，其余按 `0xRRGGBBAA` 打包 —— 转成
        // 同一口径再比，避免拿"两种表示"直接对比而得出假结论。
        let gpui_colors = gpui.paint(&scene).colors;
        let gpui_as_semantic: Vec<Option<neo_ui_render::Color>> = gpui_colors
            .iter()
            .map(|packed| {
                if *packed == 0 {
                    None
                } else {
                    let [r, g, b, a] = packed.to_be_bytes();
                    Some(neo_ui_render::Color::rgba(r, g, b, a))
                }
            })
            .collect();
        assert_eq!(
            gpui_as_semantic, expected,
            "[{name}] gpui 后端的颜色序列应与场景一一对应（0 = 裁剪占位）"
        );
    }
}

/// 契约 3：**几何一致** —— 同一场景产出的矩形，位置与尺寸必须相同。
///
/// 只比颜色会漏掉"位置算错了"这类问题（颜色全对、图形错位）。两个后端都得
/// 报出自己画了哪些矩形，这里逐项比。
#[test]
fn every_backend_places_the_same_rectangles() {
    let gpui = GpuiBackend::new();
    let headless = HeadlessBackend::new();

    for (name, scene) in real_world_scenes() {
        let g = gpui.paint(&scene);
        let h = headless.paint(&scene);

        // gpui：填充矩形在 `quads`
        let gpui_rects: Vec<(f32, f32, f32, f32)> =
            g.quads.iter().map(|q| (q.x, q.y, q.w, q.h)).collect();
        // headless：填充矩形在 `HeadlessCmd::Fill`
        let headless_rects: Vec<(f32, f32, f32, f32)> = h
            .cmds
            .iter()
            .filter_map(|c| match c {
                neo_ui_render::HeadlessCmd::Fill { rect, .. } => {
                    Some((rect.origin.x, rect.origin.y, rect.size.w, rect.size.h))
                }
                _ => None,
            })
            .collect();

        assert_eq!(
            gpui_rects, headless_rects,
            "[{name}] 两个后端画出的矩形必须逐一相同"
        );
    }
}

/// 契约 4：**能力边界由后端自己申报，且必须诚实**。
///
/// 两个后端的能力矩阵**不同**（文字：gpui 否 / headless 是）。这条用例的断言
/// 不是"两边一样"，而是两件更根本的事：
/// 1. `supported()` 报 `true` 的指令，后端**真的**产出了绘制产物；
/// 2. 报 `false` 的，`unsupported_ops` 能把它指出来（不静默）。
#[test]
fn capability_claims_are_honest_and_observed() {
    // 构造一个含全部四种 op 的场景
    let mut scene = Scene::new();
    scene.push(neo_ui_render::Op::FillRect {
        rect: neo_ui_render::Rect::new(0.0, 0.0, 4.0, 4.0),
        color: neo_ui_render::Color::rgb(1, 1, 1),
    });
    scene.push(neo_ui_render::Op::StrokeRect {
        rect: neo_ui_render::Rect::new(0.0, 0.0, 4.0, 4.0),
        color: neo_ui_render::Color::rgb(2, 2, 2),
        width: 1.0,
    });
    scene.push(neo_ui_render::Op::FillText {
        text: "文字".into(),
        origin: neo_ui_render::Point::new(0.0, 0.0),
        color: neo_ui_render::Color::rgb(3, 3, 3),
        size: 12.0,
    });
    scene.push(neo_ui_render::Op::PushClip {
        rect: neo_ui_render::Rect::new(0.0, 0.0, 8.0, 8.0),
    });
    scene.push(neo_ui_render::Op::PopClip);

    let gpui = GpuiBackend::new();
    let headless = HeadlessBackend::new();

    // --- 无头：声称全支持 → 必须每条都进显示列表（Fill/Stroke/Text/Clip） ---
    assert!(
        unsupported_ops(&headless, &scene).is_empty(),
        "无头后端声称支持全部 op，就不该有画不出的"
    );
    let h = headless.paint(&scene);
    assert_eq!(h.cmds.len(), 4, "四条可见指令（PopClip 不产出）");
    assert!(
        h.cmds.iter().any(|c| matches!(c, neo_ui_render::HeadlessCmd::Text { .. })),
        "声称支持文字 → 必须真的有文字指令"
    );
    assert!(
        h.cmds.iter().any(|c| matches!(c, neo_ui_render::HeadlessCmd::Stroke { .. })),
        "声称支持描边 → 必须真的有描边指令"
    );

    // --- gpui：文字画不出 → 必须被 `unsupported_ops` 指出来，且**不得**
    //     偷偷变成矩形（那是最糟的"静默替代"） ---
    let bad = unsupported_ops(&gpui, &scene);
    assert_eq!(bad, vec![2], "应且只应指出下标 2 的文字指令");
    let g = gpui.paint(&scene);
    assert_eq!(g.quads.len(), 1, "只该有 1 个填充矩形（文字不许变成矩形）");
    assert_eq!(g.strokes.len(), 1, "描边这一版真的画（见 supported 的实现）");
    assert_eq!(g.text_ops, 1, "画不出也要计数，以区分'翻译了'与'没翻译'");
}

/// 契约 5：**我们的自绘组件不得产生任何后端画不出的指令**。
///
/// 这是把这条缝和真实组件绑起来的那一条。前面几条验的是"后端之间一致"，
/// 这条验的是"**组件不会在某个后端上静默少东西**"：组件产生的场景若含后端
/// 画不出的指令，那个组件在那个后端上就会缺一块 —— 而屏幕上不会报错
/// （这正是最该被机器抓住的一类问题）。
///
/// ⚠️ 它当前**有实际约束力**：`GpuiBackend` 画不出文字（`FillText`），
/// 所以哪天有人给变更条 / 用量图 / 进度条加上文字标签，却没同时给 gpui 后端
/// 实现文字绘制，这条会立刻红 —— 而不是等到真机截图才发现"标签不见了"。
#[test]
fn our_self_drawn_components_use_only_ops_every_backend_can_paint() {
    let gpui = GpuiBackend::new();
    let headless = HeadlessBackend::new();

    for (name, scene) in real_world_scenes() {
        for (backend_name, bad) in [
            ("gpui", unsupported_ops(&gpui, &scene)),
            ("headless", unsupported_ops(&headless, &scene)),
        ] {
            assert!(
                bad.is_empty(),
                "[{name}] 在 {backend_name} 后端上有 {} 条画不出的指令（下标 {bad:?}）—— \
                 组件在那个后端上会静默少一块",
                bad.len()
            );
        }
    }
}

/// 契约 6（负向，必须有牙齿）：**"声称支持却不画"必须被抓住**。
///
/// 本仓的纪律：每个 seam 都要有一个"坏后端"当负向被试 —— 抓不住反例的套件
/// 没有牙齿。这里用一个**说谎的后端**：对 `FillRect` 报 `true`，实际什么都不画。
/// 若 conformance 抓不住它，那么它对真实后端也没有约束力。
#[test]
fn a_backend_that_lies_about_fill_rect_is_caught() {
    /// 说谎的后端：声称支持填充，`paint` 却产出空列表。
    struct LyingBackend;
    struct EmptyPaint {
        drawn: usize,
    }

    impl RenderBackend for LyingBackend {
        type Paint = EmptyPaint;
        fn name(&self) -> &'static str {
            "lying"
        }
        fn measure_text(&self, text: &str, size: f32) -> neo_ui_render::Size {
            let cols = neo_text::width::display_width(text) as f32;
            neo_ui_render::Size::new(cols * size * 0.6, size * 1.35)
        }
        fn supported(&self, _op: &neo_ui_render::Op) -> bool {
            true // 声称支持一切
        }
        fn paint(&self, _scene: &Scene) -> EmptyPaint {
            EmptyPaint { drawn: 0 } // 实际上什么都不画
        }
    }

    let mut scene = Scene::new();
    scene.push(neo_ui_render::Op::FillRect {
        rect: neo_ui_render::Rect::new(0.0, 0.0, 4.0, 4.0),
        color: neo_ui_render::Color::rgb(1, 1, 1),
    });

    let liar = LyingBackend;
    // 说谎点：它说全支持，于是 unsupported_ops 也说"没问题" ——
    // 所以这条必须靠**比对真实产物**来抓，不能只问 supported()。
    assert!(
        unsupported_ops(&liar, &scene).is_empty(),
        "它确实声称全支持（这正是谎言本身）"
    );

    // 真实产物为空 → 与诚实后端对比即可暴露。
    // 把"谎报支持"定义为：声称支持的 op 数量 > 实际产出的指令数。
    let honest = HeadlessBackend::new();
    let honest_cmds = honest.paint(&scene).cmds.len();
    let liar_cmds = liar.paint(&scene).drawn;
    assert_ne!(
        liar_cmds, honest_cmds,
        "说谎后端必须与诚实后端的产物数量不同 —— 若相同则这条负向用例失效"
    );
    assert_eq!(honest_cmds, 1, "诚实后端对同场景产出 1 条填充");
    assert_eq!(liar_cmds, 0, "说谎后端产出 0 条 —— 套件靠这个差异抓住它");
}
