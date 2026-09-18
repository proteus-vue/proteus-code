//! 后端中立的绘制描述：颜色 / 几何 / 绘制指令。
//!
//! 这些类型**不引用任何 GUI 库**，因此能被任意后端消费（GPUI、Vello、Skia…）。
//! 它们是这条缝的全部词汇表 —— 刻意保持很小：词汇表越大，后端越难对齐，
//! 而"中立"的意义恰恰在于**没有后端特有的概念**。

use neo_text::{Palette, Tone};

/// 颜色（sRGB 分量，0–255）。与 `neo-text` 调色板的表示一致，
/// 因此语义色调可以直接派生，不必做归一化换算。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    /// 不透明色。
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// 带透明度。
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// 从语义色调派生 —— **这是跨宿主观感一致的支点**：
    /// TUI 把 `Tone::Accent` 翻成 ANSI 色、egui 翻成 `Color32`、gpui 翻成 `Rgba`，
    /// 但"Accent 是哪一个 RGB"只有一份来源（`neo-text` 的调色板）。
    pub fn from_tone(palette: &Palette, tone: Tone) -> Self {
        let (r, g, b) = palette.rgb(tone);
        Self::rgb(r, g, b)
    }

    /// `0xRRGGBBAA` 打包（GPUI 的 `rgba()` 就用这个布局）。
    ///
    /// 单开一个方法而不是让后端各自拼：拼错字节序是个静默的视觉 bug
    /// （红蓝互换看着像"主题变了"，很难联想到打包顺序）。
    pub const fn to_rgba_u32(self) -> u32 {
        ((self.r as u32) << 24) | ((self.g as u32) << 16) | ((self.b as u32) << 8) | (self.a as u32)
    }
}

/// 点（逻辑坐标，左上为原点）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// 尺寸（逻辑像素）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Size {
    pub w: f32,
    pub h: f32,
}

impl Size {
    pub const fn new(w: f32, h: f32) -> Self {
        Self { w, h }
    }
}

/// 矩形区域。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub origin: Point,
    pub size: Size,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { origin: Point::new(x, y), size: Size::new(w, h) }
    }

    /// 右边界 / 下边界（不含）。
    pub fn right(&self) -> f32 {
        self.origin.x + self.size.w
    }
    pub fn bottom(&self) -> f32 {
        self.origin.y + self.size.h
    }
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.origin.x && p.x < self.right() && p.y >= self.origin.y && p.y < self.bottom()
    }
}

/// 一条绘制指令。**只包含各后端都有的概念**（填充、描边、文字、裁剪）。
///
/// 刻意不做"通用 2D 图形 API"（渐变、滤镜、混合模式…）：
/// 那些在各后端语义差异很大，塞进来会让"中立"变成"以某一个后端为准"。
/// 自绘表面需要什么，等它真的需要时再往这里加。
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// 填充矩形（可带圆角，`radius = 0` 即直角）。
    ///
    /// # 圆角为什么是**这里的属性**而不是另开一条指令
    ///
    /// 它就是"这块矩形有多圆"，与颜色一样是矩形的**属性**（Zed 的场景模型
    /// 也是这么放的：`Quad { bounds, corner_radii, background, … }`）。
    /// 另开一条 `FillRoundedRect` 会让"同一件事有两种说法" —— 后端要处理
    /// 两遍、`supported()` 要报两条，而它们永远该一起变。
    ///
    /// **默认 0（直角）**是刻意的：`radius` 是"要圆角的人显式要"的属性，
    /// 而不是"所有人都会继承的默认"。这一点很重要 —— 像 diff 底色带那种
    /// **整行通铺**的面必须保持直角（圆角会让行与行之间出现缝隙，
    /// 一眼看去像"改动的行数不对"）。
    FillRect { rect: Rect, color: Color, radius: f32 },
    /// 描边矩形（可带圆角，语义同上）。
    StrokeRect { rect: Rect, color: Color, width: f32, radius: f32 },
    /// 画一行文字（基线左上角对齐）。
    FillText { text: String, origin: Point, color: Color, size: f32 },
    /// 压入裁剪区（后续绘制只在该区内可见）。
    PushClip { rect: Rect },
    /// 弹出最近一次裁剪。
    PopClip,
}

/// 一个中立场景 = 一串绘制指令。
///
/// 设计上刻意**廉价克隆**（`Vec<Op>` + `Clone`）：将来做增量绘制时，
/// "这一帧和上一帧的场景差异"是脏区剔除的依据。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scene {
    ops: Vec<Op>,
}

impl Scene {
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }

    /// 追加一条指令（链式，便于拼装）。
    pub fn push(&mut self, op: Op) -> &mut Self {
        self.ops.push(op);
        self
    }

    /// 方角填充 —— **最常见**的情形，所以单列一个入口。
    ///
    /// 有它之后调用点读起来是 `scene.fill(rect, color)` 而不是
    /// `push(Op::FillRect { rect, color, radius: 0.0 })` ——
    /// 后者会让"我没要圆角"淹没在一堆字段里。
    pub fn fill(&mut self, rect: Rect, color: Color) -> &mut Self {
        self.push(Op::FillRect { rect, color, radius: 0.0 })
    }

    /// 圆角填充。`radius` 由调用方给（**不要**在渲染层写死具体数值：
    /// 半径属于设计系统，应当与颜色一样由上层传入）。
    pub fn fill_rounded(&mut self, rect: Rect, color: Color, radius: f32) -> &mut Self {
        self.push(Op::FillRect { rect, color, radius })
    }

    pub fn ops(&self) -> &[Op] {
        &self.ops
    }

    pub fn len(&self) -> usize {
        self.ops.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// 清空（复用同一块内存，避免每帧重新分配）。
    pub fn clear(&mut self) {
        self.ops.clear();
    }

    /// 裁剪区是否配平（PushClip/PopClip 数量相等）。
    ///
    /// 后端翻译裁剪时依赖配平：不配平的场景会让裁剪栈错位，
    /// 表现为"后面画的东西整片消失"——很难查。所以做成可断言的不变量。
    pub fn clips_balanced(&self) -> bool {
        let mut depth: i32 = 0;
        for op in &self.ops {
            match op {
                Op::PushClip { .. } => depth += 1,
                Op::PopClip => {
                    depth -= 1;
                    if depth < 0 {
                        return false;
                    }
                }
                _ => {}
            }
        }
        depth == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use neo_text::palette::NEO;

    #[test]
    fn color_packs_as_rrggbbaa() {
        let c = Color::rgb(0xa7, 0x8b, 0xfa);
        assert_eq!(c.to_rgba_u32(), 0xa7_8b_fa_ff);
        let c = Color::rgba(0x00, 0x11, 0x22, 0x33);
        assert_eq!(c.to_rgba_u32(), 0x00_11_22_33);
    }

    #[test]
    fn tone_derives_from_the_shared_palette() {
        // 跨宿主一致：Tone → Color 只有一份来源
        let c = Color::from_tone(&NEO, Tone::Primary);
        assert_eq!((c.r, c.g, c.b, c.a), (0xa7, 0x8b, 0xfa, 255), "品牌紫");
        let c = Color::from_tone(&NEO, Tone::Accent);
        assert_eq!((c.r, c.g, c.b), (0xe8, 0x79, 0xf9), "强调品红");
    }

    #[test]
    fn rect_geometry_is_half_open() {
        let r = Rect::new(10.0, 10.0, 100.0, 50.0);
        assert_eq!(r.right(), 110.0);
        assert_eq!(r.bottom(), 60.0);
        assert!(r.contains(Point::new(10.0, 10.0)), "左上角含");
        assert!(r.contains(Point::new(109.9, 59.9)));
        assert!(!r.contains(Point::new(110.0, 30.0)), "右边界不含（半开）");
        assert!(!r.contains(Point::new(30.0, 60.0)), "下边界不含");
    }

    #[test]
    fn scene_builder_accumulates_and_clears() {
        let mut s = Scene::new();
        assert!(s.is_empty());
        s.push(Op::FillRect { rect: Rect::new(0.0, 0.0, 1.0, 1.0), color: Color::rgb(1, 2, 3), radius: 0.0 });
        s.push(Op::PopClip);
        assert_eq!(s.len(), 2);
        s.clear();
        assert!(s.is_empty(), "clear 后应回到空（复用内存）");
    }

    #[test]
    fn clip_balance_is_checkable() {
        let mut ok = Scene::new();
        ok.push(Op::PushClip { rect: Rect::new(0.0, 0.0, 10.0, 10.0) });
        ok.push(Op::PopClip);
        assert!(ok.clips_balanced());

        let mut unbalanced = Scene::new();
        unbalanced.push(Op::PushClip { rect: Rect::new(0.0, 0.0, 10.0, 10.0) });
        assert!(!unbalanced.clips_balanced(), "缺 PopClip 必须被查出");

        let mut underflow = Scene::new();
        underflow.push(Op::PopClip);
        assert!(!underflow.clips_balanced(), "多出的 PopClip 也要被查出");
    }
}
