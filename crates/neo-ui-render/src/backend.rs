//! `RenderBackend` 契据与它的 GPUI 实现。
//!
//! # 契约的形状
//!
//! 后端只做两件事：**度量文字**、把**中立场景**翻译成自己的绘制产物。
//! 契据本身不出现任何 GUI 类型（`Paint` 是关联类型），所以换后端时
//! 消费方（自绘组件）不需要改 —— 它们只依赖 [`Scene`] 与 [`RenderBackend`]。
//!
//! # 为什么只有 `measure_text` 是"必须立刻做对"的
//!
//! 度量决定布局（换行位置、行高、命中区），一旦各后端各算一套，
//! 同一个界面在不同后端上**排版就会不一致**。所以这个方法从一开始就在契约里，
//! 而不是等第二个后端出现才补。

use crate::scene::{Color, Op, Scene, Size};

/// 一个渲染后端。
///
/// `Send + 'static`：场景可能从任意线程提交（内核事件在驱动线程上到达）。
pub trait RenderBackend: Send + 'static {
    /// 后端自己的绘制产物。
    ///
    /// 关联类型是刻意的：契据本身不认识 GPUI 的 `AnyElement`，也不认识 Vello 的
    /// `Scene` —— 只有各自实现里才出现具体类型。
    type Paint: 'static;

    /// 后端名（诊断与测试用）。
    fn name(&self) -> &'static str;

    /// 度量一段文字在给定字号下占多大。
    ///
    /// 各后端必须给出**一致**的结果，否则同一场景在不同后端排版不同。
    fn measure_text(&self, text: &str, size: f32) -> Size;

    /// 把中立场景翻译成该后端的绘制产物。
    fn paint(&self, scene: &Scene) -> Self::Paint;

    /// 本后端**画不画得出**这条指令。
    ///
    /// # 为什么把它做成契约的一部分
    ///
    /// `paint()` 遇到画不出的 op 只有两种做法，都不好：
    /// 静默丢掉 —— 表现为"某块东西在界面上不见了"，最难查；
    /// panic —— 把整帧搞崩。
    ///
    /// 所以把"支持哪些"变成**可查询的事实**，再由 [`unsupported_ops`] 统一报告。
    /// 于是"场景里有后端画不出的东西"从一个隐形事实，变成**可断言、可进门禁**的
    /// 命题 —— 反过来也逼着中立指令集（[`Op`]）不要长出没人能画的词汇。
    ///
    /// 实现里**必须诚实**：报 `true` 就真的会画出来。
    fn supported(&self, op: &Op) -> bool;
}

/// 列出场景里**该后端画不出**的指令下标。
///
/// 用于两处：
/// 1. 组件自测 —— 断言自己产生的场景**全部可画**（否则那个组件在某个后端上
///    会静默少一块）；
/// 2. conformance —— 中立词汇表里若有"任何后端都画不出"的指令，这里会暴露。
///
/// 取泛型而不是 `&dyn RenderBackend`：契据带关联类型 `Paint`（刻意如此，
/// 见 [`RenderBackend`]），而 `dyn` 必须把它钉死 —— 那会白拿一个只在
/// 这个辅助函数里出现的类型参数。泛型在这里零成本。
pub fn unsupported_ops<B: RenderBackend + ?Sized>(backend: &B, scene: &Scene) -> Vec<usize> {
    scene
        .ops()
        .iter()
        .enumerate()
        .filter(|(_, op)| !backend.supported(op))
        .map(|(i, _)| i)
        .collect()
}


/// GPUI 后端。
///
/// 它把中立场景翻译成 GPUI 的绘制调用。**这是全仓唯一允许把中立类型
/// 与 GPUI 类型接起来的地方** —— 缝的边界就在这一层。
pub struct GpuiBackend {
    /// 被翻译过的场景数（仅用于测试断言"真的走了缝"）。
    painted: std::cell::Cell<usize>,
}

impl GpuiBackend {
    pub fn new() -> Self {
        Self { painted: std::cell::Cell::new(0) }
    }

    /// 已翻译的场景数（测试用）。
    pub fn painted_count(&self) -> usize {
        self.painted.get()
    }
}

impl Default for GpuiBackend {
    fn default() -> Self {
        Self::new()
    }
}

/// 一个待绘制的矩形（中立坐标，相对所在表面的左上角）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GpuiQuad {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub color: Color,
}

/// 一条待描边的矩形（线宽 > 0）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GpuiStroke {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub width: f32,
    pub color: Color,
}

/// GPUI 后端的绘制产物：一批填充矩形 + 一批描边 + 文字指令计数。
///
/// # 为什么是"数据 + 一个 draw 方法"，而不是构造 `AnyElement`
///
/// 构造 element 需要 `&mut Window`，那会把契据污染成"只能从渲染帧内部调用" ——
/// 中立层就不中立了。所以 `paint()` 只做**坐标与颜色的翻译**（纯数据、可测），
/// 真正的绘制留给 [`GpuiPaint::draw`]，由宿主在 gpui 的 `canvas` 回调里调用
/// （那里才有 `Window`）。
///
/// 这条分工也让"翻译对不对"能被单测覆盖 —— 而绘制本身要靠真机看。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GpuiPaint {
    /// 待填充的矩形（顺序即绘制顺序）。
    pub quads: Vec<GpuiQuad>,
    /// 待描边的矩形 —— 与填充分开存：gpui 的 `quad` 用 `Edges` 表达边框，
    /// 与"填充一块面"是不同的绘制参数，混在一个列表里会丢掉线宽。
    pub strokes: Vec<GpuiStroke>,
    /// 文字指令的条数。
    ///
    /// ⚠️ **本后端画不出文字**（`supported` 对 `FillText` 报 `false`）：gpui 的
    /// 文字要经 `TextSystem::shape_line` 整形后逐个 `paint_glyph`，需要字体上下文，
    /// 而本后端的产物是**纯数据**（见上）。这里只记数，是为了让
    /// "文字确实被翻译到了、没被悄悄丢掉"仍可被断言 —— 配合 `unsupported_ops`
    /// 就能区分"计数了但画不出"与"根本没翻译"。
    pub text_ops: usize,
    /// 每条指令对应的颜色（`0xRRGGBBAA`），顺序与场景一致（含裁剪占位）。
    /// 保留它是为了"顺序对齐"这条契约仍可被断言（见下方测试）。
    pub colors: Vec<u32>,
}

impl GpuiPaint {
    /// 把矩形画到窗口上。`origin` 是所在表面的左上角（画布 bounds 的原点）。
    ///
    /// 坐标在这里从"表面局部坐标"加上 `origin` 变成窗口坐标 —— 这一步是必要的：
    /// 中立场景不知道自己在窗口的哪个位置。
    pub fn draw(&self, origin: neo_ui_kit::gpui::Point<neo_ui_kit::gpui::Pixels>, window: &mut neo_ui_kit::gpui::Window) {
        use neo_ui_kit::gpui::{Bounds, Corners, Edges, point, px, size};
        for q in &self.quads {
            let bounds = Bounds::new(
                point(origin.x + px(q.x), origin.y + px(q.y)),
                size(px(q.w), px(q.h)),
            );
            // 无边框、直角：变更条是"面"不是"卡片"（本项目一贯的视觉纪律：
            // 底色层是面、边框只是线）
            window.paint_quad(neo_ui_kit::gpui::quad(
                bounds,
                Corners::default(),
                to_gpui_rgba(q.color),
                Edges::default(),
                neo_ui_kit::gpui::transparent_black(),
                neo_ui_kit::gpui::BorderStyle::default(),
            ));
        }
        for s in &self.strokes {
            let bounds = Bounds::new(
                point(origin.x + px(s.x), origin.y + px(s.y)),
                size(px(s.w), px(s.h)),
            );
            // 描边 = 四条边等宽 + 透明填充。用 `quad` 的 border 通道而不是四周画
            // 四条实心条：后者在拐角会重叠（半透明色下看得见"角更亮"）。
            window.paint_quad(neo_ui_kit::gpui::quad(
                bounds,
                Corners::default(),
                neo_ui_kit::gpui::transparent_black(),
                Edges::all(px(s.width)),
                to_gpui_rgba(s.color),
                neo_ui_kit::gpui::BorderStyle::default(),
            ));
        }
    }
}

impl RenderBackend for GpuiBackend {
    type Paint = GpuiPaint;

    fn name(&self) -> &'static str {
        "gpui"
    }

    fn measure_text(&self, text: &str, size: f32) -> Size {
        // 用显示宽度（`neo-text` 的 width 模块）而不是 `chars().count()`：
        // 中日韩是**双宽**字符，按字符数算会低估一半，换行位置随之错。
        // 与 TUI 用同一份宽度实现，是"同一场景在不同宿主排版一致"的支点。
        let cols = neo_text::width::display_width(text) as f32;
        Size::new(cols * size * 0.6, size * 1.35)
    }

    /// 诚实地报出画得出来什么。
    ///
    /// **`FillText` 报 `false`** —— 不是遗漏，是这一版的真实边界：gpui 的文字要
    /// 经 `TextSystem::shape_line` 整形再 `paint_glyph`，需要字体上下文，而本后端的
    /// 产物是纯数据（见 `GpuiPaint`）。现在**没有任何自绘消费者产生文字指令**
    /// （`FillRect` + `PushClip` 足够画变更条 / 用量条 / 分段进度），
    /// 所以这条边界当前不影响任何界面 —— 但它是真的，必须报出来。
    ///
    /// 报 `true` 的必须真的画得出：`paint()` 里 `StrokeRect` 走 `strokes`
    /// 并在 `draw()` 里经 `quad` 的 border 通道画出来。
    fn supported(&self, op: &Op) -> bool {
        match op {
            Op::FillRect { .. } | Op::StrokeRect { .. } | Op::PushClip { .. } | Op::PopClip => true,
            Op::FillText { .. } => false,
        }
    }

    fn paint(&self, scene: &Scene) -> Self::Paint {
        self.painted.set(self.painted.get() + 1);
        let mut colors = Vec::with_capacity(scene.len());
        let mut quads = Vec::new();
        let mut strokes = Vec::new();
        let mut text_ops = 0;
        for op in scene.ops() {
            match op {
                crate::scene::Op::FillRect { rect, color } => {
                    quads.push(GpuiQuad {
                        x: rect.origin.x,
                        y: rect.origin.y,
                        w: rect.size.w,
                        h: rect.size.h,
                        color: *color,
                    });
                    colors.push(color.to_rgba_u32());
                }
                crate::scene::Op::StrokeRect { rect, color, width } => {
                    // 线宽为 0 的描边画不出任何东西 —— 按"不产生绘制指令"处理，
                    // 但仍占一个颜色位（顺序契约），否则后续 op 的颜色会整体错位。
                    if *width > 0.0 {
                        strokes.push(GpuiStroke {
                            x: rect.origin.x,
                            y: rect.origin.y,
                            w: rect.size.w,
                            h: rect.size.h,
                            width: *width,
                            color: *color,
                        });
                    }
                    colors.push(color.to_rgba_u32());
                }
                crate::scene::Op::FillText { color, .. } => {
                    // 计数但不产出绘制指令：本后端画不出文字（见 `supported`）。
                    // 留计数是为了让"翻译到了但画不出"与"根本没翻译"可区分。
                    text_ops += 1;
                    colors.push(color.to_rgba_u32());
                }
                // 裁剪指令没有颜色：留 0 占位以保持"顺序与场景一致"的契约
                crate::scene::Op::PushClip { .. } | crate::scene::Op::PopClip => colors.push(0),
            }
        }
        GpuiPaint { quads, strokes, text_ops, colors }
    }
}

/// 供测试与调用方使用的便利：把 `Color` 直接翻成 GPUI 的 `Rgba`。
///
/// 只在后端实现里用得到，所以不放进中立层。
pub fn to_gpui_rgba(c: Color) -> neo_ui_kit::gpui::Rgba {
    neo_ui_kit::gpui::rgba(c.to_rgba_u32())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Op, Rect};

    fn one_rect_scene() -> Scene {
        let mut s = Scene::new();
        s.push(Op::FillRect {
            rect: Rect::new(0.0, 0.0, 10.0, 10.0),
            color: Color::rgb(0xa7, 0x8b, 0xfa),
        });
        s
    }

    #[test]
    fn backend_names_itself() {
        assert_eq!(GpuiBackend::new().name(), "gpui");
    }

    #[test]
    fn painting_translates_neutral_colors_to_backend_values() {
        let b = GpuiBackend::new();
        let paint = b.paint(&one_rect_scene());
        assert_eq!(paint.colors, vec![0xa7_8b_fa_ff], "颜色要按后端布局打包");
        assert_eq!(b.painted_count(), 1, "应记录翻译过一次");
    }

    #[test]
    fn painting_counts_text_ops() {
        let mut s = Scene::new();
        s.push(Op::FillRect {
            rect: Rect::new(0.0, 0.0, 1.0, 1.0),
            color: Color::rgb(0, 0, 0),
        });
        s.push(Op::FillText {
            text: "中文 abc".into(),
            origin: crate::scene::Point::new(0.0, 0.0),
            color: Color::rgb(255, 255, 255),
            size: 14.0,
        });
        let paint = GpuiBackend::new().paint(&s);
        assert_eq!(paint.text_ops, 1);
        assert_eq!(paint.colors.len(), 2, "顺序与场景一致：每个 op 一个位置");
    }

    #[test]
    fn clip_ops_keep_a_placeholder_so_indices_stay_aligned() {
        // 契约：colors[i] 对应 scene.ops()[i] —— 裁剪指令不能把顺序挤歪，
        // 否则后续 op 的颜色会整体错位（表现为"颜色莫名其妙"）。
        let mut s = Scene::new();
        s.push(Op::PushClip { rect: Rect::new(0.0, 0.0, 5.0, 5.0) });
        s.push(Op::FillRect {
            rect: Rect::new(0.0, 0.0, 1.0, 1.0),
            color: Color::rgb(1, 2, 3),
        });
        s.push(Op::PopClip);
        let paint = GpuiBackend::new().paint(&s);
        assert_eq!(paint.colors.len(), 3);
        assert_eq!(paint.colors[0], 0, "裁剪占位");
        assert_eq!(paint.colors[1], 0x01_02_03_ff, "填充色仍在下标 1");
    }

    #[test]
    fn measuring_uses_display_width_not_char_count() {
        let b = GpuiBackend::new();
        // 中日韩是双宽：6 个汉字 ≈ 12 列，而 chars().count() 只有 6。
        // 按字符数算会让换行位置整体偏一半。
        let cjk = b.measure_text("中文渲染测试", 10.0);
        let ascii = b.measure_text("abcdef", 10.0);
        assert!(
            cjk.w > ascii.w * 1.5,
            "双宽文字应显著更宽（CJK {} vs ASCII {}）",
            cjk.w,
            ascii.w
        );
    }

    #[test]
    fn measuring_scales_with_font_size() {
        let b = GpuiBackend::new();
        let small = b.measure_text("abc", 10.0);
        let big = b.measure_text("abc", 20.0);
        assert!(big.w > small.w && big.h > small.h, "字号翻倍应让尺寸翻倍");
        assert!((big.w - small.w * 2.0).abs() < 0.01, "应为线性");
    }

    /// **契约：`supported` 必须诚实** —— 报 `true` 的就真的产出绘制指令。
    ///
    /// 这条是"静默丢东西"的反面保险：若有人把 `StrokeRect` 改成不画，却忘了把
    /// `supported` 改成 `false`，场景里就会出现"声称能画、实际不见"的指令 ——
    /// 那正是本契约要防的事。
    #[test]
    fn supported_claims_match_what_paint_actually_produces() {
        let b = GpuiBackend::new();
        let ops = [
            Op::FillRect { rect: Rect::new(0.0, 0.0, 4.0, 4.0), color: Color::rgb(1, 1, 1) },
            Op::StrokeRect {
                rect: Rect::new(0.0, 0.0, 4.0, 4.0),
                color: Color::rgb(2, 2, 2),
                width: 1.0,
            },
            Op::FillText {
                text: "x".into(),
                origin: crate::scene::Point::new(0.0, 0.0),
                color: Color::rgb(3, 3, 3),
                size: 12.0,
            },
        ];
        let mut s = Scene::new();
        for op in &ops {
            s.push(op.clone());
        }
        let paint = b.paint(&s);

        assert!(b.supported(&ops[0]), "填充矩形必须支持");
        assert_eq!(paint.quads.len(), 1, "声称支持填充 → 必须真的有填充指令");

        assert!(b.supported(&ops[1]), "描边必须支持（这一版真的画）");
        assert_eq!(paint.strokes.len(), 1, "声称支持描边 → 必须真的有描边指令");
        assert_eq!(paint.strokes[0].width, 1.0, "线宽要原样带过去，不能被吞");

        assert!(!b.supported(&ops[2]), "文字本后端画不出，必须诚实报 false");
        assert!(paint.quads.is_empty() || paint.text_ops == 1, "文字不得变成矩形");
        assert_eq!(paint.text_ops, 1, "画不出也要计数，以区分'翻译了'与'没翻译'");
    }

    /// **零线宽的描边不该产出绘制指令**（画不出任何东西），
    /// 但**必须仍占一个颜色位** —— 否则后续 op 的颜色整体错位。
    #[test]
    fn zero_width_stroke_is_skipped_without_breaking_color_alignment() {
        let mut s = Scene::new();
        s.push(Op::StrokeRect {
            rect: Rect::new(0.0, 0.0, 4.0, 4.0),
            color: Color::rgb(9, 9, 9),
            width: 0.0,
        });
        s.push(Op::FillRect { rect: Rect::new(0.0, 0.0, 1.0, 1.0), color: Color::rgb(1, 2, 3) });
        let paint = GpuiBackend::new().paint(&s);

        assert!(paint.strokes.is_empty(), "零线宽画不出东西，不该产生描边指令");
        assert_eq!(paint.colors.len(), 2, "顺序契约：每个 op 一个位置");
        assert_eq!(paint.colors[1], 0x01_02_03_ff, "填充色仍在下标 1，没被挤歪");
    }

    /// `unsupported_ops`：把"后端画不出什么"变成可查询的事实。
    #[test]
    fn unsupported_ops_reports_the_text_boundary() {
        let mut s = Scene::new();
        s.push(Op::FillRect { rect: Rect::new(0.0, 0.0, 1.0, 1.0), color: Color::rgb(0, 0, 0) });
        s.push(Op::FillText {
            text: "画不出的文字".into(),
            origin: crate::scene::Point::new(0.0, 0.0),
            color: Color::rgb(255, 255, 255),
            size: 12.0,
        });
        let bad = unsupported_ops(&GpuiBackend::new(), &s);
        assert_eq!(bad, vec![1], "应指出下标 1 的文字指令画不出");
    }
}
