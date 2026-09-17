//! 组件展示台（storybook）。
//!
//! 运行：
//!
//! ```text
//! cargo run --example storybook -p neo-ui
//! ```
//!
//! # 它有两重身份
//!
//! 1. **调试工具**：改主题或组件时，不必启动整个应用就能看到效果。
//! 2. **开源门面**：使用者想知道"这个库长什么样"时，第一眼看的通常是它 ——
//!    而不是 README 里的文字描述。
//!
//! # 为什么它必须能独立运行
//!
//! 它是"待开源 crate 能否脱离宿主工程"的一个日常检验：若 storybook 需要
//! 引用宿主（或宿主背后的内核）才能跑起来，那说明组件与业务耦合了。
//! 这里只依赖 `neo-ui` 与 `neo-ui-kit` 两个 crate。
//!
//! # 诚实边界
//!
//! 展示台**只列真实存在的组件**。没有占位条目、不预告"即将支持" ——
//! 那种做法会让展示台从"可用的东西"变成"愿望清单"，
//! 而读者无法分辨哪一个是真的。当前条目不多，这是事实。

use neo_text::Tone;
use neo_ui_kit::component::{
    button::Button, h_flex, v_flex, Root, Theme,
};
use neo_ui_kit::component::scroll::ScrollableElement as _;
use neo_ui_kit::gpui::{
    div, prelude::*, px, Context, IntoElement, Render, Window,
};

/// 一次展示：标题 + 说明 + 内容。
struct Story {
    title: &'static str,
    note: &'static str,
    body: Box<dyn FnOnce() -> neo_ui_kit::gpui::AnyElement>,
}

struct Storybook {
    theme_ready: bool,
}

impl Storybook {
    fn stories(&self) -> Vec<Story> {
        vec![
            Story {
                title: "RichText · 语义色调",
                note: "同一段文字按片段给语义色，而不是写死颜色值。\
                       右侧的色名是语义角色，具体色值来自调色板。",
                body: Box::new(|| {
                    let rt = neo_ui::RichText::from_spans(&[
                        ("普通 ".into(), Tone::Text),
                        ("强调 ".into(), Tone::Primary),
                        ("成功 ".into(), Tone::Success),
                        ("警告 ".into(), Tone::Warning),
                        ("失败 ".into(), Tone::Error),
                        ("弱化".into(), Tone::Muted),
                    ]);
                    neo_ui::rich_text(&rt).into_any_element()
                }),
            },
            Story {
                title: "RichText · 搜索命中",
                note: "命中的字节区间加背景高亮，**保留**原有的语义前景色 —— \
                       否则搜索会抹掉'这段是强调/代码/注释'的信息层次。",
                body: Box::new(|| {
                    // "缩进" 出现在第 2、3 处；这里手动给区间（真实场景由搜索给出）
                    let text = "先确认缩进与行号，再看缩进深度";
                    let rt = neo_ui::RichText::new(text)
                        .span(0..text.len(), Tone::Muted)
                        .marks(&[9..15, 27..33]);
                    neo_ui::rich_text(&rt).into_any_element()
                }),
            },
            Story {
                title: "RichText · 无效区间不崩",
                note: "越界或切在多字节字符中间的区间会被忽略并计数（\n\
                       `invalid_ranges()` 可以看到有几个）。文本本身照常完整显示。",
                body: Box::new(|| {
                    let rt = neo_ui::RichText::new("中文区间测试")
                        .span(0..6, Tone::Primary)
                        .span(0..999, Tone::Error)   // 越界：被忽略
                        .mark(1..4);                  // 切在字符中间：被忽略
                    div()
                        .child(neo_ui::rich_text(&rt).into_any_element())
                        .child(
                            div()
                                .text_color(neo_ui::neo_color(Tone::Muted))
                                .child(format!(
                                    "（被忽略的无效区间：{} 个）",
                                    rt.invalid_ranges()
                                )),
                        )
                        .into_any_element()
                }),
            },
            Story {
                title: "背景色",
                note: "底色与正文色是两个独立的入口（`base_bg` / `panel_bg`）。\
                       刻意不提供'背景色调'枚举值 —— 把它塞进语义色调里，\
                       很容易被当作某个具体颜色误用于前景。",
                body: Box::new(|| {
                    // ⚠️ 两块**必须在大底色上并排**展示。
                    // 第一版把它们放进了一个 `panel_bg` 的容器里，于是
                    // "base_bg" 那块的底色被容器盖住 —— 两块看起来几乎同色，
                    // 而实际取值是 #000000 与 #201e2a，差别明显。
                    // 展示台的排布错误会让人怀疑颜色本身错了。
                    h_flex()
                        .gap_3()
                        .child(
                            v_flex()
                                .p_3()
                                .bg(neo_ui::base_bg())
                                .border_1()
                                .border_color(neo_ui::neo_color(Tone::Border))
                                .child(div().text_color(neo_ui::neo_color(Tone::Text)).child("base_bg"))
                                .child(
                                    div()
                                        .text_color(neo_ui::neo_color(Tone::Muted))
                                        .child("最底层背景（#000000）"),
                                ),
                        )
                        .child(
                            v_flex()
                                .p_3()
                                .bg(neo_ui::panel_bg())
                                // 加边框：`panel_bg` 与卡片底色**同值**，
                                // 没边框时这个色块会完全隐形（实测像素采样
                                // 发现两处都是 #201e29，色块看起来"没画出来"）。
                                // 边框让"这是一块样品"成立，而不是靠底色差异。
                                .border_1()
                                .border_color(neo_ui::neo_color(Tone::Border))
                                .child(div().text_color(neo_ui::neo_color(Tone::Text)).child("panel_bg"))
                                .child(
                                    div()
                                        .text_color(neo_ui::neo_color(Tone::Muted))
                                        .child("面板背景（#201e2a，比底色亮一档）"),
                                ),
                        )
                        .into_any_element()
                }),
            },
            Story {
                title: "语义调色板全览",
                note: "所有语义色调的当前取值。改主题时这一页就是对照表。",
                body: Box::new(|| {
                    let tones = [
                        ("Text", Tone::Text),
                        ("Primary", Tone::Primary),
                        ("Accent", Tone::Accent),
                        ("Success", Tone::Success),
                        ("Warning", Tone::Warning),
                        ("Error", Tone::Error),
                        ("Info", Tone::Info),
                        ("Muted", Tone::Muted),
                        ("Border", Tone::Border),
                        ("BorderActive", Tone::BorderActive),
                        ("Dim", Tone::Dim),
                    ];
                    v_flex()
                        .gap_1()
                        .children(tones.into_iter().map(|(name, tone)| {
                            h_flex()
                                .gap_2()
                                .child(
                                    div()
                                        .w(px(96.))
                                        .text_color(neo_ui::neo_color(Tone::Muted))
                                        .child(name),
                                )
                                .child(
                                    div()
                                        .text_color(neo_ui::neo_color(tone))
                                        .child("示例文字 sample text"),
                                )
                        }))
                        .into_any_element()
                }),
            },
            Story {
                title: "UsageBars · 用量条形图（自绘）",
                note: "走渲染缝的自绘表面（不是文字）。柱高按全局最大值缩放、\
                       非零至少 1px（否则小值会被抹掉），柱宽有上限（否则单根\n\
                       占满整行、看起来像色带而不是图表）。",
                body: Box::new(|| {
                    // 展示一组有形状的数据：递增 + 一个陡增
                    let bars = vec![
                        neo_ui_render::UsageBar::new(1200, 300),
                        neo_ui_render::UsageBar::new(1800, 900),
                        neo_ui_render::UsageBar::new(2600, 1500),
                        neo_ui_render::UsageBar::new(11500, 2600),
                        neo_ui_render::UsageBar::new(900, 200),
                        neo_ui_render::UsageBar::new(3000, 1100),
                    ];
                    v_flex()
                        .gap_1()
                        .child(usage_demo_element(bars))
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    div()
                                        .text_color(neo_ui::neo_color(Tone::Info))
                                        .child("■ 输入"),
                                )
                                .child(
                                    div()
                                        .text_color(neo_ui::neo_color(Tone::Accent))
                                        .child("■ 输出"),
                                ),
                        )
                        .into_any_element()
                }),
            },
            Story {
                title: "SegmentedProgress · 分段进度（自绘）",
                note: "每个子任务一段，段内表达走到第几阶段（共 5 步）。\n\
                       零进度只画底色（与'刚开始'区分），完成的段画满（不留空隙）。",
                body: Box::new(|| {
                    let segs = vec![
                        neo_ui_render::ProgressSegment::new(5, 5), // 完成
                        neo_ui_render::ProgressSegment::new(3, 5), // 审查中
                        neo_ui_render::ProgressSegment::new(1, 5), // 刚开始
                        neo_ui_render::ProgressSegment::new(0, 5), // 未开始
                    ];
                    v_flex()
                        .gap_1()
                        .child(progress_demo_element(segs))
                        .child(
                            div()
                                .text_color(neo_ui::neo_color(Tone::Muted))
                                .child("四段：完成 / 审查中 / 刚开始 / 未开始"),
                        )
                        .into_any_element()
                }),
            },
            Story {
                title: "ChangeGutter · 变更条（自绘）",
                note: "diff 左侧的改动分布条：每格按行数比例分配高度、每格至少 1px\n\
                       （否则长 diff 里的小改动会被取整抹掉 —— '有改动却看不见'更糟）。\n\
                       它是渲染缝的**第一个**消费者。",
                body: Box::new(|| {
                    use neo_ui_render::GutterMark;
                    // 一段有形状的改动分布：集中几处 + 稀疏几处
                    let marks = vec![
                        GutterMark::Plain,
                        GutterMark::Add,
                        GutterMark::Add,
                        GutterMark::Plain,
                        GutterMark::Del,
                        GutterMark::Plain,
                        GutterMark::Plain,
                        GutterMark::Add,
                        GutterMark::Del,
                        GutterMark::Del,
                        GutterMark::Del,
                        GutterMark::Plain,
                    ];
                    v_flex()
                        .gap_1()
                        .child(gutter_demo_element(marks))
                        .child(
                            h_flex()
                                .gap_2()
                                .child(div().text_color(neo_ui::neo_color(Tone::Success)).child("■ 新增"))
                                .child(div().text_color(neo_ui::neo_color(Tone::Error)).child("■ 删除"))
                                .child(div().text_color(neo_ui::neo_color(Tone::Muted)).child("□ 未改")),
                        )
                        .into_any_element()
                }),
            },
            Story {
                title: "DiffBackdrop · diff 行背景带（自绘）",
                note: "diff 正文的**行底带**：新增/删除各一色、hunk 头更淡（它是位置标记\n\
                       而非改动）。只给逐行彩色文字时，颜色只标了单行语义、给不出\"改动落在\n\
                       哪几段\"的形状 —— 而看 diff 的第一个问题恰是那个形状。\n\
                       对齐靠\"底带与文字共用同一个行高\"，不是调间距（见模块头部说明）。",
                body: Box::new(|| {
                    use neo_ui_render::DiffBand;
                    let bands = vec![
                        DiffBand::Context,
                        DiffBand::Hunk,
                        DiffBand::Del,
                        DiffBand::Del,
                        DiffBand::Add,
                        DiffBand::Context,
                        DiffBand::Add,
                        DiffBand::Add,
                    ];
                    v_flex()
                        .gap_1()
                        .child(diff_backdrop_demo_element(bands))
                        .child(
                            h_flex()
                                .gap_2()
                                .child(div().text_color(neo_ui::neo_color(Tone::Success)).child("■ 新增"))
                                .child(div().text_color(neo_ui::neo_color(Tone::Error)).child("■ 删除"))
                                .child(div().text_color(neo_ui::neo_color(Tone::Info)).child("■ hunk 头"))
                                .child(div().text_color(neo_ui::neo_color(Tone::Muted)).child("□ 上下文（无底）")),
                        )
                        .into_any_element()
                }),
            },
            Story {
                title: "渲染缝 · 两个后端（同场景 → 不同产物）",
                note: "这条缝有**两个**后端：GpuiBackend（真窗口）与 HeadlessBackend\n\
                       （无 GPU，可跑 CI）。下面列出无头后端对同一个用量场景导出的 SVG ——\n\
                       它不是示意图，是第二个后端**真的产出的东西**。\n\
                       两者对同一场景的画布指令逐项一致（由 conformance 断言）。",
                body: Box::new(|| {
                    use neo_ui_render::{usage_bars, HeadlessBackend, RenderBackend};
                    let scene = usage_bars(
                        &[neo_ui_render::UsageBar::new(900, 200), neo_ui_render::UsageBar::new(400, 120)],
                        96.0,
                        24.0,
                        neo_ui_render::Color::from_tone(&neo_text::palette::NEO, Tone::Info),
                        neo_ui_render::Color::from_tone(&neo_text::palette::NEO, Tone::Accent),
                    );
                    let svg = HeadlessBackend::new().paint(&scene).to_svg(96.0, 24.0);
                    v_flex()
                        .gap_2()
                        .child(
                            div()
                                .text_color(neo_ui::neo_color(Tone::Muted))
                                .child(format!(
                                    "无头后端导出 {} 条绘制指令 · SVG {} 字节",
                                    scene.len(),
                                    svg.len()
                                )),
                        )
                        .child(
                            // 直接显示 SVG 源码前几行 —— 可核对，非杜撰
                            div()
                                .font_family("monospace")
                                .text_color(neo_ui::neo_color(Tone::Text))
                                .child(
                                    svg.lines()
                                        .take(5)
                                        .collect::<Vec<_>>()
                                        .join("\n"),
                                ),
                        )
                        .into_any_element()
                }),
            },
            Story {
                title: "组件库组件（来自依赖）",
                note: "库自己**不重复实现**按钮这类通用控件，直接用组件库的。\
                       展示台把它们也列出来，是为了说明'哪些是自研、哪些是现成'的边界。",
                body: Box::new(|| {
                    h_flex()
                        .gap_2()
                        .child(Button::new("sb-primary").label("主要按钮"))
                        .child(
                            Button::new("sb-secondary")
                                .label("次要按钮")
                                // 组件库的样式由主题驱动，这里演示语义色一致
                                .text_color(neo_ui::neo_color(Tone::Accent)),
                        )
                        .into_any_element()
                }),
            },
        ]
    }
}


/// 展示用的用量图元素（与宿主里的同构：经渲染缝绘制）。
fn usage_demo_element(bars: Vec<neo_ui_render::UsageBar>) -> neo_ui_kit::gpui::AnyElement {
    use neo_ui_render::{usage_bars, RenderBackend};
    let input_color = neo_ui_render::Color::from_tone(&neo_text::palette::NEO, Tone::Info);
    let output_color = neo_ui_render::Color::from_tone(&neo_text::palette::NEO, Tone::Accent);
    let prepaint = move |bounds: neo_ui_kit::gpui::Bounds<neo_ui_kit::gpui::Pixels>,
                         _w: &mut neo_ui_kit::gpui::Window,
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
    neo_ui_kit::gpui::canvas(prepaint, |bounds, (_, paint), window, _cx| {
        paint.draw(bounds.origin, window);
    })
    .w_full()
    .h(px(48.))
    .into_any_element()
}

/// 展示用的分段进度元素。
fn progress_demo_element(segs: Vec<neo_ui_render::ProgressSegment>) -> neo_ui_kit::gpui::AnyElement {
    use neo_ui_render::{segmented_progress, RenderBackend};
    let track = neo_ui_render::Color::from_tone(&neo_text::palette::NEO, Tone::Border);
    let fill = neo_ui_render::Color::from_tone(&neo_text::palette::NEO, Tone::Success);
    let prepaint = move |bounds: neo_ui_kit::gpui::Bounds<neo_ui_kit::gpui::Pixels>,
                         _w: &mut neo_ui_kit::gpui::Window,
                         _cx: &mut neo_ui_kit::gpui::App| {
        let scene = segmented_progress(
            &segs,
            f32::from(bounds.size.width),
            f32::from(bounds.size.height),
            track,
            fill,
        );
        (bounds, neo_ui_render::GpuiBackend::new().paint(&scene))
    };
    neo_ui_kit::gpui::canvas(prepaint, |bounds, (_, paint), window, _cx| {
        paint.draw(bounds.origin, window);
    })
    .w_full()
    .h(px(10.))
    .into_any_element()
}

/// 展示用的变更条元素（与宿主里的同构：经渲染缝绘制）。
fn gutter_demo_element(marks: Vec<neo_ui_render::GutterMark>) -> neo_ui_kit::gpui::AnyElement {
    use neo_ui_render::{change_gutter, RenderBackend};
    let prepaint = move |bounds: neo_ui_kit::gpui::Bounds<neo_ui_kit::gpui::Pixels>,
                         _w: &mut neo_ui_kit::gpui::Window,
                         _cx: &mut neo_ui_kit::gpui::App| {
        let scene = change_gutter(
            &marks,
            f32::from(bounds.size.width),
            f32::from(bounds.size.height),
        );
        (bounds, neo_ui_render::GpuiBackend::new().paint(&scene))
    };
    neo_ui_kit::gpui::canvas(prepaint, |bounds, (_, paint), window, _cx| {
        paint.draw(bounds.origin, window);
    })
    .w_full()
    .h(px(96.))
    .into_any_element()
}

/// 展示用的 diff 行背景带（与宿主里的同构：底带走缝、文字仍在 element 树）。
///
/// 这里用**代表符号 + 固定行高**代替真实 diff 文本：展示台的重点是"底带的形状
/// 与对齐"，而不是再渲染一遍 diff 解析。行高 18pt 与宿主取值的量级一致。
fn diff_backdrop_demo_element(bands: Vec<neo_ui_render::DiffBand>) -> neo_ui_kit::gpui::AnyElement {
    use neo_ui_render::{diff_backdrop, RenderBackend};
    const LH: f32 = 18.0;
    let labels: Vec<&str> = bands
        .iter()
        .map(|b| match b {
            neo_ui_render::DiffBand::Add => "+ 新增的一行",
            neo_ui_render::DiffBand::Del => "- 被删除的一行",
            neo_ui_render::DiffBand::Hunk => "@@ -1,5 +1,6 @@",
            neo_ui_render::DiffBand::Header => "--- a/示例.txt",
            neo_ui_render::DiffBand::Meta => "… 另有 3 处改动未显示",
            neo_ui_render::DiffBand::Fold => "⋯ 未改 12 行（点击展开）",
            neo_ui_render::DiffBand::Context => "  未改的上下文行",
        })
        .collect();
    let tones: Vec<Tone> = bands
        .iter()
        .map(|b| match b {
            neo_ui_render::DiffBand::Add => Tone::Success,
            neo_ui_render::DiffBand::Del => Tone::Error,
            neo_ui_render::DiffBand::Hunk => Tone::Info,
            neo_ui_render::DiffBand::Header => Tone::Muted,
            neo_ui_render::DiffBand::Meta => Tone::Muted,
            neo_ui_render::DiffBand::Fold => Tone::Info,
            neo_ui_render::DiffBand::Context => Tone::Text,
        })
        .collect();

    let prepaint = move |bounds: neo_ui_kit::gpui::Bounds<neo_ui_kit::gpui::Pixels>,
                         _w: &mut neo_ui_kit::gpui::Window,
                         _cx: &mut neo_ui_kit::gpui::App| {
        let scene = diff_backdrop(&bands, f32::from(bounds.size.width), LH);
        (bounds, neo_ui_render::GpuiBackend::new().paint(&scene))
    };
    let mut texts = v_flex().gap_0().line_height(px(LH));
    for (label, tone) in labels.iter().zip(tones) {
        texts = texts.child(
            div()
                .whitespace_nowrap()
                .text_color(neo_ui::neo_color(tone))
                .child(label.to_string()),
        );
    }
    neo_ui_kit::gpui::div()
        .relative()
        .w_full()
        .child(
            neo_ui_kit::gpui::canvas(prepaint, |bounds, (_, paint), window, _cx| {
                paint.draw(bounds.origin, window);
            })
            .absolute()
            .top(px(0.))
            .left(px(0.))
            .right(px(0.))
            .h_full(),
        )
        .child(texts)
        .into_any_element()
}

impl Render for Storybook {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 主题只装一次：反复装会让颜色闪回默认值
        if !self.theme_ready {
            neo_ui::apply_neo_theme(cx);
            self.theme_ready = true;
        }

        let page = v_flex()
            .size_full()
            .bg(neo_ui::base_bg())
            .text_color(neo_ui::neo_color(Tone::Text))
            .p_6()
            .gap_4()
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_color(neo_ui::neo_color(Tone::Accent))
                            .child("neo-ui 组件展示台"),
                    )
                    .child(
                        div()
                            .text_color(neo_ui::neo_color(Tone::Muted))
                            .child(
                                "只列真实存在的组件；下方每一项都是当前可用行为，不是预告。",
                            ),
                    ),
            );

        // 内容必须可滚动：条目会随组件增加而变多，不滚动就只能看到最上面几条
        //（实测：加了 6 条之后，"语义调色板全览"与"组件库组件"两项在窗口外，
        //  且页面滚不动 —— 而后两项恰恰是展示台最该展示的东西）
        let mut body = v_flex().gap_4();
        for story in self.stories() {
            body = body.child(
                v_flex()
                    .gap_2()
                    .p_4()
                    .bg(neo_ui::panel_bg())
                    .child(
                        div()
                            .text_color(neo_ui::neo_color(Tone::Info))
                            .child(story.title),
                    )
                    .child(
                        div()
                            .text_color(neo_ui::neo_color(Tone::Muted))
                            .child(story.note),
                    )
                    .child((story.body)()),
            );
        }

        page.child(
            div()
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scrollbar()
                .child(body),
        )
    }
}

fn main() {
    neo_ui_kit::application()
        .with_assets(neo_ui_kit::assets::Assets)
        .run(move |cx| {
            neo_ui_kit::init(cx);
            // 展示台固定深色：调色板本来就是深色的（见 neo-ui 的说明）
            Theme::change(neo_ui_kit::component::ThemeMode::Dark, None, cx);

            let bounds = neo_ui_kit::gpui::Bounds::centered(
                None,
                neo_ui_kit::gpui::size(px(760.), px(900.)),
                cx,
            );
            let _ = cx.open_window(
                neo_ui_kit::gpui::WindowOptions {
                    window_bounds: Some(neo_ui_kit::gpui::WindowBounds::Windowed(bounds)),
                    titlebar: Some(neo_ui_kit::gpui::TitlebarOptions {
                        title: Some("neo-ui storybook".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |window, cx| {
                    // `cx.new` 收的是**闭包**（它要在实体创建时才有上下文）
                    cx.new(|cx| Root::new(cx.new(|_| Storybook::new()), window, cx))
                },
            );
        });
}

impl Storybook {
    fn new() -> Self {
        Self { theme_ready: false }
    }
}
