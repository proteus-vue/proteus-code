//! 自绘 **diff 背景带** —— 让"哪些行改了"能一眼扫出来。
//!
//! # 它补的是什么（一个具体的可读性缺口）
//!
//! 两个 GUI 宿主的 diff 正文此前都是**逐行彩色文字**：新增行是绿字、删除行是红字，
//! 但**底子与其它所有内容一样**。逐行彩色文字的问题是：颜色只标注了单行的语义，
//! 没有给出"改动落在哪几段"的形状 —— 而看 diff 时的第一个问题恰恰是那个形状。
//!
//! 真实 diff 查看器（GitHub / Zed / ZCode）都有**行背景带**，这是让 diff 能扫读
//! 的关键层。本模块就是它的自绘实现，经渲染缝出场景（一行一个填充矩形）。
//!
//! # 它为什么走渲染缝，而不是用现成的文本控件加底色
//!
//! 背景带是**面**（填充矩形），与文字是两种绘制产物：
//! - 文字必须留在宿主的 element 树里 —— 那样才有 element diff、脏区剔除、
//!   文本整形与字形缓存（crate 头部的纪律）；
//! - 而"一行一块底色"是纯粹的矩形填充，正是这条缝画得最好的东西。
//!
//! 于是分工是：**缝画底、宿主画字**。这也是 [`crate::GpuiBackend`] 画不出文字
//! 却不影响本组件的原因（见 `tests/conformance.rs` 的"组件只用各后端都画得出的
//! 指令"那条契约）。
//!
//! # 对齐是**构造保证**的，不是调出来的
//!
//! 带子的 y 由 `i * line_height` 给出，宿主把文本容器的行高**设成同一个值**
//! （`Window::line_height()`，两边同源），并把每行设为不折行 ——
//! 于是"一行文字"恰好占 `line_height` 高，与带子严丝合缝。
//! 若靠"大概差不多"去调间距，换字体/换字号就会错开，而错开的背景带
//! 比没有背景带更糟（它会把改动标到错误的行上）。

use crate::scene::{Color, Op, Rect, Scene};

/// 一行的背景带种类（**中立语义**，不含具体色值）。
///
/// 与 [`crate::GutterMark`]（`Plain` / `Add` / `Del`）的关系：那个描述的是
/// "变更条这一格"（只看有没有改），本枚举描述"这一行的底子"（还要区分
/// 上下文行与 hunk 头 —— 它们都**不该**有改动色的底）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffBand {
    /// 上下文行（未改）—— 不画底。
    Context,
    /// 新增行。
    Add,
    /// 删除行。
    Del,
    /// hunk 头（`@@ … @@`）—— 只给极淡的底，作为"这一段从这开始"的标记。
    Hunk,
}

/// 一份背景带的样式常量。
///
/// 单独放一个 `struct` 而不是散落的 `const`：宿主与展示台要从**同一处**取值，
/// 否则"同一份 diff 在两处颜色不同"。
#[derive(Debug, Clone, Copy)]
pub struct DiffBandStyle {
    /// 新增行的底色（半透明，压在文字下面）。
    pub add: Color,
    /// 删除行的底色。
    pub del: Color,
    /// hunk 头的底色（更淡 —— 它是位置标记，不是改动）。
    pub hunk: Color,
}

impl Default for DiffBandStyle {
    fn default() -> Self {
        Self::from_neo_palette()
    }
}

impl DiffBandStyle {
    /// 从 NEO 语义调色板派生。
    ///
    /// # 为什么用**低透明度**而不是不透明色
    ///
    /// 底色要压在文字下面，必须让文字仍然清晰可读。若直接用 `Tone::Success`
    /// 之类的**正文色**当底，浅色主题下会变成"深绿底 + 绿字"，字几乎看不见。
    /// 选 alpha 而不是另取一套"浅绿"色值，也让它在换主题时自动跟随。
    ///
    /// 三个值是按"能扫出形状、但不与文字抢注意力"选的：改动行明显可辨，
    /// hunk 头只是隐约一条 —— 它标位置，不标改动量。
    pub fn from_neo_palette() -> Self {
        let p = &neo_text::palette::NEO;
        let tint = |tone: neo_text::Tone, a: u8| {
            let (r, g, b) = p.rgb(tone);
            Color::rgba(r, g, b, a)
        };
        Self {
            add: tint(neo_text::Tone::Success, 32),
            del: tint(neo_text::Tone::Error, 32),
            hunk: tint(neo_text::Tone::Info, 14),
        }
    }

    /// 某一类带子用什么颜色；`Context` 无底 → `None`。
    pub fn color_for(&self, band: DiffBand) -> Option<Color> {
        match band {
            DiffBand::Context => None,
            DiffBand::Add => Some(self.add),
            DiffBand::Del => Some(self.del),
            DiffBand::Hunk => Some(self.hunk),
        }
    }
}

/// 构建 diff 背景带场景。
///
/// - `bands`：逐行的种类，**顺序与 diff 文本行一一对应**（宿主负责映射）。
/// - `width`：可画宽度（底带铺满）。
/// - `line_height`：**必须与渲染文本用的行高是同一个值**（见模块头部的对齐说明）。
///
/// 场景高度 = `bands.len() * line_height`；越界部分由 `PushClip` 裁掉，
/// 所以即使宿主给的高度更小也不会画到区域之外。
pub fn diff_backdrop(bands: &[DiffBand], width: f32, line_height: f32) -> Scene {
    let mut scene = Scene::new();
    if bands.is_empty() || width <= 0.0 || line_height <= 0.0 {
        return scene;
    }

    let style = DiffBandStyle::from_neo_palette();
    let total_h = bands.len() as f32 * line_height;
    // 裁剪到"带子应有的总高"：宿主给的区域可能更高（比如下方还有别的内容），
    // 不裁的话最后一条会画到不属于它的位置。
    scene.push(Op::PushClip { rect: Rect::new(0.0, 0.0, width, total_h) });

    for (i, band) in bands.iter().enumerate() {
        let Some(color) = style.color_for(*band) else {
            continue; // 上下文行没有底 —— 不画任何东西（不是画一个透明矩形）
        };
        scene.push(Op::FillRect {
            rect: Rect::new(0.0, i as f32 * line_height, width, line_height),
            color,
        });
    }

    scene.push(Op::PopClip);
    scene
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rects(s: &Scene) -> Vec<Rect> {
        s.ops()
            .iter()
            .filter_map(|o| match o {
                Op::FillRect { rect, .. } => Some(*rect),
                _ => None,
            })
            .collect()
    }

    /// **对齐契约**：第 i 条带子必须落在 `[i*lh, (i+1)*lh)`。
    ///
    /// 这是本组件唯一真正要紧的事 —— 带子错位会把"改动"标到错误的行上，
    /// 比不画更糟（用户会据此判断改了哪一行）。
    #[test]
    fn band_i_occupies_exactly_its_own_line_box() {
        let lh = 18.0;
        let bands = [DiffBand::Add, DiffBand::Del, DiffBand::Add];
        let s = diff_backdrop(&bands, 100.0, lh);
        let r = rects(&s);
        assert_eq!(r.len(), 3, "三条带子");
        for (i, rect) in r.iter().enumerate() {
            assert_eq!(rect.origin.y, i as f32 * lh, "第 {i} 条的 y 应为 i*lh");
            assert_eq!(rect.size.h, lh, "每条恰好一行高");
            assert_eq!(rect.origin.x, 0.0, "铺满宽度（起点 0）");
            assert_eq!(rect.size.w, 100.0, "铺满宽度");
        }
    }

    /// 上下文行**不画底**（画一个透明矩形是多余的指令，也让人以为它有底）。
    #[test]
    fn context_lines_draw_nothing() {
        let s = diff_backdrop(&[DiffBand::Context, DiffBand::Context], 50.0, 16.0);
        assert!(rects(&s).is_empty(), "全上下文 → 没有任何底带");
    }

    /// 上下文行**不占位**：它的位置由后面的带子的 y 反映出来，
    /// 而不是靠"画一条透明带"来占位 —— 后者会让指令数随行数线性增长。
    #[test]
    fn context_lines_do_not_shift_the_following_bands() {
        let lh = 10.0;
        let s = diff_backdrop(
            &[DiffBand::Context, DiffBand::Context, DiffBand::Add],
            80.0,
            lh,
        );
        let r = rects(&s);
        assert_eq!(r.len(), 1, "只有新增那条画底");
        assert_eq!(r[0].origin.y, 2.0 * lh, "它的 y 必须体现前两行上下文（2*lh）");
    }

    /// 三类带子的颜色必须**可区分** —— 否则"新增/删除"在界面上是同一个样子。
    #[test]
    fn the_three_kinds_are_visually_distinct() {
        let st = DiffBandStyle::from_neo_palette();
        let add = st.color_for(DiffBand::Add).unwrap();
        let del = st.color_for(DiffBand::Del).unwrap();
        let hunk = st.color_for(DiffBand::Hunk).unwrap();
        assert_ne!(add, del, "新增与删除必须不同色");
        assert_ne!(add, hunk, "hunk 头不能与新增同色");
        assert!(st.color_for(DiffBand::Context).is_none(), "上下文无底");

        // 底色必须是**半透明**的：压在文字下面，不透明会盖掉文字可读性
        for (name, c) in [("add", add), ("del", del), ("hunk", hunk)] {
            assert!(c.a < 128, "{name} 的 alpha 过高（{}）—— 会与文字抢对比度", c.a);
            assert!(c.a > 0, "{name} 的 alpha 为 0 → 等于没画");
        }
    }

    /// 裁剪配平且裁到"带子总高" —— 多余的区域不该被画上底。
    #[test]
    fn clips_the_backdrop_to_its_own_height() {
        let lh = 12.0;
        let s = diff_backdrop(&[DiffBand::Add, DiffBand::Del], 60.0, lh);
        assert!(s.clips_balanced(), "裁剪必须配平（否则后续绘制会被错误裁剪）");
        let clip = s.ops().iter().find_map(|o| match o {
            Op::PushClip { rect } => Some(*rect),
            _ => None,
        });
        let clip = clip.expect("应有裁剪区");
        assert_eq!(clip.size.h, 2.0 * lh, "裁剪高度 = 行数 × 行高");
    }

    /// 退化输入不 panic、不产出无意义指令。
    #[test]
    fn degenerate_inputs_yield_an_empty_scene() {
        assert!(diff_backdrop(&[], 100.0, 16.0).is_empty(), "空输入");
        assert!(diff_backdrop(&[DiffBand::Add], 0.0, 16.0).is_empty(), "零宽");
        assert!(diff_backdrop(&[DiffBand::Add], 100.0, 0.0).is_empty(), "零行高");
        assert!(
            diff_backdrop(&[DiffBand::Add], 100.0, -1.0).is_empty(),
            "负行高（不该悄悄按绝对值画）"
        );
    }

    /// 大 diff 不产生爆炸式的场景（每行至多一条指令 + 裁剪两条）。
    #[test]
    fn a_large_diff_stays_linear_and_bounded() {
        let bands: Vec<DiffBand> = (0..5000)
            .map(|i| if i % 2 == 0 { DiffBand::Add } else { DiffBand::Context })
            .collect();
        let s = diff_backdrop(&bands, 800.0, 18.0);
        assert_eq!(s.len(), 2500 + 2, "2500 条底带 + 收尾两条裁剪");
        assert!(s.clips_balanced());
    }
}
