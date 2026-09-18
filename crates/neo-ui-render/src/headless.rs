//! 无头后端：渲染缝的**第二个实现**，也是 conformance 的另一半。
//!
//! # 它为什么必须存在（这是本 crate 从「信仰」走向「设计」的那一步）
//!
//! 只有一个实现的抽象，无法回答"这条缝到底中立不中立" —— 因为没有任何东西
//! 逼它不偏向那个唯一实现。本 crate 的头部一直标注着这句话，本模块就是把它
//! 兑现：**同一份场景，两个后端必须给出语义一致的结果**。
//!
//! # 它与 `GpuiBackend` **故意相反**，这才有验证力
//!
//! | | `GpuiBackend` | `HeadlessBackend` |
//! |---|---|---|
//! | 输出 | gpui 绘制参数（数据） | 可自查的显示列表 + SVG |
//! | 需要 GPU | 是（真实窗口） | **否**（纯计算） |
//! | 文字 | **画不出**（需字体上下文） | **画得出**（转成 `<text>`） |
//! | 描边 | 画得出 | 画得出 |
//!
//! 两者**能力矩阵不同**（文字正好相反）这件事本身，就是
//! [`RenderBackend::supported`] 该属于后端、而不属于中立层的最好证据 ——
//! 若把它放进中立层，就必然要按某一个后端写死。
//!
//! # 它不是"Vello 的替代品"
//!
//! 方案文档明确要求**不要**为了凑数提前写第二个 GPU 后端（"在只有一个实现时，
//! 第二个实现的所有假设都会错"）。本后端不是那个 —— 它不试图成为一条并行的
//! 生产渲染路径，而是**可判定的对照物 + 无 GPU 的测试替身**。真正的第二个
//! 生产后端（VelloBackend）仍等 Phase 4 的触发条件。
//!
//! 但也正因为它便宜，它能立刻回答一个贵问题：**这条缝的形状对不对**。

use crate::backend::RenderBackend;
use crate::scene::{Color, Op, Point, Rect, Scene, Size};

/// 无头后端。无状态（除计数），可在任意环境跑（CI、无 GPU 容器）。
#[derive(Default)]
pub struct HeadlessBackend {
    painted: std::cell::Cell<usize>,
}

impl HeadlessBackend {
    pub fn new() -> Self {
        Self::default()
    }

    /// 已翻译的场景数（测试断言"真的走了缝"）。
    pub fn painted_count(&self) -> usize {
        self.painted.get()
    }
}

/// 无头后端的绘制产物的单条指令。
///
/// 与 [`crate::GpuiQuad`] 的区别是刻意的：这里是**可自查的显示列表**
/// （保留了几何、颜色、文字与裁剪），而不是"喂给某个绘制 API 的参数包"。
#[derive(Debug, Clone, PartialEq)]
pub enum HeadlessCmd {
    /// 填充（`radius` 见 `scene::Op::FillRect` 的说明）。
    Fill { rect: Rect, color: Color, radius: f32 },
    Stroke { rect: Rect, color: Color, width: f32, radius: f32 },
    /// 文字 —— **本后端真的画得出**（见模块头部的能力对照表）。
    Text { text: String, origin: Point, color: Color, size: f32 },
    /// 裁剪区（后续指令受它约束）。
    Clip { rect: Rect },
}

impl HeadlessCmd {
    /// 该指令的颜色（裁剪没有颜色 → `None`）。
    ///
    /// 用于"顺序与颜色必须与场景一致"这条跨后端契约 —— 两个后端都要能
    /// 按同一顺序报出颜色，否则"同一场景在两处长得不同"就无从检查。
    pub fn color(&self) -> Option<Color> {
        match self {
            HeadlessCmd::Fill { color, .. }
            | HeadlessCmd::Stroke { color, .. }
            | HeadlessCmd::Text { color, .. } => Some(*color),
            HeadlessCmd::Clip { .. } => None,
        }
    }
}

/// 无头后端的绘制产物：显示列表 + 可导出 SVG。
///
/// # 为什么除了 `cmds` 还要单独存 `colors`
///
/// `cmds` 是**显示列表**：只有真画得出的东西才在里面（`PopClip` 不在 —— 它由
/// `Clip` 那条自己闭合）。但跨后端契约要求"**每个场景 op 一个颜色位**"，
/// 后端的颜色序列必须与场景**逐项对齐**（`GpuiBackend` 对裁剪/零线宽都留占位）。
///
/// 这个字段就是那份被本文件第一版漏掉的占位 —— 是 `tests/conformance.rs`
/// 的跨后端比对抓出来的：同一个场景，无头后端比场景**少一位**（尾部 `PopClip`），
/// 而 gpui 后端留着。**两个后端对同一个契约有两种实现，正是这条缝要防的事**，
/// 所以补齐它，而不是把断言改宽松。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HeadlessPaint {
    pub cmds: Vec<HeadlessCmd>,
    /// 每个场景 op 一位（裁剪与"画不出"的 op 为 `None`），与 `cmds` **不是**
    /// 一一对应 —— 这是刻意的，见上。
    pub colors: Vec<Option<Color>>,
}

impl HeadlessPaint {
    /// 每条场景 op 的颜色位（与场景顺序一一对应）。裁剪为 `None`。
    ///
    /// 这是跨后端可比的**语义指纹**：两个后端对同一场景翻译出的颜色序列
    /// 必须逐项相等（见 `tests/conformance.rs`）。
    pub fn color_sequence(&self) -> Vec<Option<Color>> {
        self.colors.clone()
    }

    /// 导出 SVG —— 让这条缝的产**可以被看一眼**。
    ///
    /// 为什么给它一个可视化出口：无头后端的价值一半在"能在无 GPU 环境断言"，
    /// 另一半在"可以把它画出来的东西存成文件看"——否则一个自绘组件在某后端上
    /// 排错了，只能靠读坐标数字猜。
    pub fn to_svg(&self, width: f32, height: f32) -> String {
        let mut out = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" \
             viewBox=\"0 0 {width} {height}\">\n"
        );
        // 裁剪用嵌套 <g clip-path>，但**只支持一层**（够当前消费者用）。
        // 多层裁剪需要真正的栈式输出，现在直接报错而不是画错 ——
        // 悄悄画错是最难查的那类问题。
        let clip_count = self
            .cmds
            .iter()
            .filter(|c| matches!(c, HeadlessCmd::Clip { .. }))
            .count();
        assert!(
            clip_count <= 1,
            "SVG 导出目前只支持一层裁剪（场景里有 {clip_count} 层）；\
             要支持嵌套请先实现栈式输出，不要静默画错"
        );
        let mut clipped = false;
        for cmd in &self.cmds {
            match cmd {
                HeadlessCmd::Clip { rect } => {
                    assert!(!clipped, "SVG 导出不支持重复 PushClip（未实现 PopClip 的嵌套语义）");
                    out.push_str(&format!(
                        "<g clip-path=\"url(#clip0)\">\n<clipPath id=\"clip0\"><rect x=\"{}\" y=\"{}\" \
                         width=\"{}\" height=\"{}\"/></clipPath>\n",
                        rect.origin.x, rect.origin.y, rect.size.w, rect.size.h
                    ));
                    clipped = true;
                }
                HeadlessCmd::Fill { rect, color, radius } => {
                    // SVG 用 `rx` 表示圆角。**不夹取**：这里输出的就是场景里
                    // 的那个值 —— 夹取该由产生场景的一方负责（见 progress.rs
                    // 的说明），否则两个后端会给出不同结果。
                    let rx = if *radius > 0.0 { format!(" rx=\"{radius}\"") } else { String::new() };
                    out.push_str(&format!(
                        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"{rx} fill=\"{}\"/>\n",
                        rect.origin.x,
                        rect.origin.y,
                        rect.size.w,
                        rect.size.h,
                        color_hex(*color)
                    ))
                }
                HeadlessCmd::Stroke { rect, color, width, radius } => {
                    let rx = if *radius > 0.0 { format!(" rx=\"{radius}\"") } else { String::new() };
                    out.push_str(&format!(
                        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"{rx} fill=\"none\" \
                         stroke=\"{}\" stroke-width=\"{}\"/>\n",
                        rect.origin.x,
                        rect.origin.y,
                        rect.size.w,
                        rect.size.h,
                        color_hex(*color),
                        width
                    ))
                }
                HeadlessCmd::Text { text, origin, color, size } => out.push_str(&format!(
                    "<text x=\"{}\" y=\"{}\" fill=\"{}\" font-size=\"{}\">{}</text>\n",
                    origin.x,
                    origin.y,
                    color_hex(*color),
                    size,
                    escape_xml(text)
                )),
            }
        }
        if clipped {
            out.push_str("</g>\n");
        }
        out.push_str("</svg>\n");
        out
    }
}

fn color_hex(c: Color) -> String {
    if c.a == 255 {
        format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
    } else {
        // SVG 用 fill-opacity 更标准，但这里保持单一属性便于 diff。
        format!("#{:02x}{:02x}{:02x}{:02x}", c.r, c.g, c.b, c.a)
    }
}

/// XML 转义。**必须做**：文字可能含 `<` / `&`（diff 里就是代码），
/// 不转义会生成非法 SVG —— 而"导出的图打不开"会让人以为是渲染错了。
fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

impl RenderBackend for HeadlessBackend {
    type Paint = HeadlessPaint;

    fn name(&self) -> &'static str {
        "headless"
    }

    fn measure_text(&self, text: &str, size: f32) -> Size {
        // ⚠️ **必须与 `GpuiBackend` 逐字一致**：度量决定换行与布局，各后端各算
        // 一套的话，同一场景在不同后端上排版不同 —— 那正是这条缝存在的理由。
        // 所以这里刻意复用同一份实现（`neo-text` 的显示宽度），而不是"自己也能
        // 算一个差不多的"。跨后端一致性由 `tests/conformance.rs` 断言。
        let cols = neo_text::width::display_width(text) as f32;
        Size::new(cols * size * 0.6, size * 1.35)
    }

    fn supported(&self, op: &Op) -> bool {
        // **与 GpuiBackend 不同**：文字这里画得出。能力矩阵不同是允许的
        // （见模块头部的对照表）——`supported` 报的是**本后端**的事实。
        match op {
            Op::FillRect { .. }
            | Op::StrokeRect { .. }
            | Op::FillText { .. }
            | Op::PushClip { .. }
            | Op::PopClip => true,
        }
    }

    fn paint(&self, scene: &Scene) -> Self::Paint {
        self.painted.set(self.painted.get() + 1);
        let mut cmds = Vec::with_capacity(scene.len());
        // 与 `cmds` 分开记：每个 scene op 一位，裁剪与"画不出"的 op 留 `None`。
        // 这条占位规则必须与 `GpuiBackend` 一致（见 `HeadlessPaint` 的说明）。
        let mut colors = Vec::with_capacity(scene.len());
        for op in scene.ops() {
            match op {
                Op::FillRect { rect, color, radius } => {
                    cmds.push(HeadlessCmd::Fill { rect: *rect, color: *color, radius: *radius });
                    colors.push(Some(*color));
                }
                Op::StrokeRect { rect, color, width, radius } => {
                    // 与 GpuiBackend 同一语义：零线宽画不出东西 → 不产出指令，
                    // **但仍占一个颜色位**（它是一条 op，只是画不出）。
                    // 两个后端在这点上必须一致，否则"描边在 A 看得见、在 B 不见"。
                    if *width > 0.0 {
                        cmds.push(HeadlessCmd::Stroke {
                            rect: *rect,
                            color: *color,
                            width: *width,
                            radius: *radius,
                        });
                    }
                    colors.push(Some(*color));
                }
                Op::FillText { text, origin, color, size } => {
                    cmds.push(HeadlessCmd::Text {
                        text: text.clone(),
                        origin: *origin,
                        color: *color,
                        size: *size,
                    });
                    colors.push(Some(*color));
                }
                Op::PushClip { rect } => {
                    cmds.push(HeadlessCmd::Clip { rect: *rect });
                    colors.push(None); // 裁剪没有颜色，占位以保持与场景对齐
                }
                Op::PopClip => {
                    // 不产出显示指令（裁剪区间由 `Clip` 那条自己表达，见 `to_svg`），
                    // **但要占一个颜色位** —— 占位规则是"每个 op 一位"，
                    // `PushClip`/`PopClip` 都算（见 `color_seq_of_scene`）。
                    // 漏掉这一位会让颜色序列比场景短，跨后端比对错位；
                    // 本文件第一版就漏了，被下面的 debug_assert 当场抓住。
                    colors.push(None);
                }
            }
        }
        // 用共享的规则函数兜底校验：若"哪些 op 占色位"的规则变了，这里会立刻
        // 暴露，而不是等到跨后端比对时以一个难读的数组 diff 出现。
        debug_assert_eq!(
            colors.len(),
            scene.len(),
            "颜色位必须与场景 op 一一对应（占位规则见 color_seq_of_scene）"
        );
        HeadlessPaint { cmds, colors }
    }
}

/// 场景的**期望颜色序列**（供跨后端比对）。
///
/// 单独一个函数，而不是让测试各自复述一遍规则：两个后端都必须在
/// "哪些 op 占颜色位"这个细节上一致，而这条规则只该有一份来源。
/// 裁剪占位（`None`）、零线宽描边**仍占位**（它是一条指令，只是画不出）。
pub fn color_seq_of_scene(scene: &Scene) -> Vec<Option<Color>> {
    use crate::scene::Op;
    scene
        .ops()
        .iter()
        .map(|op| match op {
            Op::FillRect { color, .. }
            | Op::StrokeRect { color, .. }
            | Op::FillText { color, .. } => Some(*color),
            Op::PushClip { .. } | Op::PopClip => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene_with(ops: Vec<Op>) -> Scene {
        let mut s = Scene::new();
        for op in ops {
            s.push(op);
        }
        s
    }

    #[test]
    fn it_names_itself_and_counts_paints() {
        let b = HeadlessBackend::new();
        assert_eq!(b.name(), "headless");
        b.paint(&Scene::new());
        assert_eq!(b.painted_count(), 1);
    }

    /// **与 GPUI 相反**：文字在本后端画得出 —— 这正是"能力属于后端"的证据。
    #[test]
    fn text_is_supported_here_unlike_the_gpui_backend() {
        let op = Op::FillText {
            text: "中".into(),
            origin: Point::new(0.0, 0.0),
            color: Color::rgb(1, 2, 3),
            size: 12.0,
        };
        let headless = HeadlessBackend::new();
        assert!(headless.supported(&op), "无头后端画得出文字");
        assert!(
            !crate::GpuiBackend::new().supported(&op),
            "gpui 后端画不出文字 —— 两者能力不同是刻意的，也是 supported 属于后端的证明"
        );

        let paint = headless.paint(&scene_with(vec![op]));
        assert_eq!(paint.cmds.len(), 1);
        assert!(matches!(paint.cmds[0], HeadlessCmd::Text { .. }), "文字要真的进显示列表");
    }

    /// 度量必须与 GPUI 后端**逐字一致** —— 否则同一场景两处排版不同。
    #[test]
    fn measurement_agrees_with_the_gpui_backend() {
        let h = HeadlessBackend::new();
        let g = crate::GpuiBackend::new();
        for (text, size) in [("abc", 12.0f32), ("中文渲染", 14.0), ("a中b", 10.0), ("", 12.0)] {
            assert_eq!(
                h.measure_text(text, size),
                g.measure_text(text, size),
                "度量必须一致：{text:?} @ {size}"
            );
        }
    }

    /// 零线宽描边：与 GPUI 后端同一语义（不产出指令），但**仍占颜色位**。
    #[test]
    fn zero_width_stroke_matches_the_gpui_semantics() {
        let s = scene_with(vec![
            Op::StrokeRect {
                rect: Rect::new(0.0, 0.0, 4.0, 4.0),
                color: Color::rgb(9, 9, 9),
                width: 0.0,
                radius: 0.0,
            },
            Op::FillRect { rect: Rect::new(0.0, 0.0, 1.0, 1.0), color: Color::rgb(1, 2, 3), radius: 0.0 },
        ]);
        let paint = HeadlessBackend::new().paint(&s);
        assert_eq!(paint.cmds.len(), 1, "零线宽不该产出描边指令（只剩那条填充）");
        assert_eq!(
            paint.color_sequence(),
            vec![Some(Color::rgb(9, 9, 9)), Some(Color::rgb(1, 2, 3))],
            "颜色序列按'每个 op 一位'与场景对齐：零线宽那条**仍占位**（它画不出，但它是一条 op）"
        );
    }

    #[test]
    fn svg_export_escapes_text_that_looks_like_markup() {
        // diff 里真的会出现 `<` 与 `&`；不转义会生成打不开的 SVG
        let s = scene_with(vec![Op::FillText {
            text: "a < b && c".into(),
            origin: Point::new(1.0, 2.0),
            color: Color::rgb(0, 0, 0),
            size: 12.0,
        }]);
        let svg = HeadlessBackend::new().paint(&s).to_svg(100.0, 50.0);
        assert!(svg.contains("a &lt; b &amp;&amp; c"), "必须转义：{svg}");
        assert!(!svg.contains("a < b"), "不得出现未转义的小于号");
        assert!(svg.starts_with("<svg") && svg.trim_end().ends_with("</svg>"));
    }

    #[test]
    fn svg_export_closes_the_clip_group_it_opens() {
        let s = scene_with(vec![
            Op::PushClip { rect: Rect::new(0.0, 0.0, 10.0, 10.0) },
            Op::FillRect { rect: Rect::new(0.0, 0.0, 5.0, 5.0), color: Color::rgb(1, 1, 1), radius: 0.0 },
            Op::PopClip,
        ]);
        let svg = HeadlessBackend::new().paint(&s).to_svg(20.0, 20.0);
        assert_eq!(svg.matches("<g ").count(), 1, "应开一个裁剪组");
        assert_eq!(svg.matches("</g>").count(), 1, "并且要闭合它（否则 SVG 非法）");
    }

    /// 嵌套裁剪在 SVG 导出里**明确报错**，而不是画出错误的结果。
    #[test]
    #[should_panic(expected = "只支持一层裁剪")]
    fn svg_export_refuses_nested_clips_instead_of_drawing_them_wrong() {
        let s = scene_with(vec![
            Op::PushClip { rect: Rect::new(0.0, 0.0, 10.0, 10.0) },
            Op::PushClip { rect: Rect::new(1.0, 1.0, 5.0, 5.0) },
        ]);
        let _ = HeadlessBackend::new().paint(&s).to_svg(20.0, 20.0);
    }

    /// 空场景导出仍是合法 SVG（消费者可能拿到空列表）。
    #[test]
    fn empty_scene_exports_valid_svg() {
        let svg = HeadlessBackend::new().paint(&Scene::new()).to_svg(10.0, 10.0);
        assert!(svg.starts_with("<svg") && svg.trim_end().ends_with("</svg>"));
    }
}
