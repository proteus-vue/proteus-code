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
