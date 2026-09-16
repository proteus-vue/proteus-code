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

use crate::scene::{Color, Scene, Size};

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

/// GPUI 后端的绘制产物：一批可绘制的矩形。
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
    /// 待绘制的矩形（顺序即绘制顺序）。
    pub quads: Vec<GpuiQuad>,
    /// 文字指令的条数。文字由宿主用自己的文本系统画（不是 quad），
    /// 这里只记数，便于断言"文字确实被翻译了、没被丢掉"。
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

    fn paint(&self, scene: &Scene) -> Self::Paint {
        self.painted.set(self.painted.get() + 1);
        let mut colors = Vec::with_capacity(scene.len());
        let mut quads = Vec::new();
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
                crate::scene::Op::StrokeRect { rect, color, .. } => {
                    // 阶段 2 的自绘消费者只有填充矩形（变更条）。
                    // 描边先按填充处理会让它"看起来对但粗一档"，不如显式不画 ——
                    // 等真有描边需求时再接，那时才知道正确的线宽语义。
                    // 颜色仍进 colors 以保持顺序契约。
                    let _ = rect;
                    colors.push(color.to_rgba_u32());
                }
                crate::scene::Op::FillText { color, .. } => {
                    text_ops += 1;
                    colors.push(color.to_rgba_u32());
                }
                // 裁剪指令没有颜色：留 0 占位以保持"顺序与场景一致"的契约
                crate::scene::Op::PushClip { .. } | crate::scene::Op::PopClip => colors.push(0),
            }
        }
        GpuiPaint { quads, text_ops, colors }
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
}
