//! 变更条：把"哪些行改了"画成一条竖直色带。
//!
//! # 它为什么必须自绘（而不是用文本行渲染）
//!
//! diff 的**文字**用文本渲染是对的（内容要可读、可选中）。但"改动集中在哪里"
//! 这件事，用文字表达不了 —— 200 行里改 3 处，滚动时看不出来。
//! 变更条是真正的**图形**：一列按比例排列的小色块，一眼看出改动分布。
//!
//! 这正是渲染缝存在的理由（方案 §2.2）：常规组件不该走缝（走了会丢 GPUI 的
//! 文本布局与脏区剔除），**只有自绘表面才走**。变更条是第一个真实消费者，
//! 所以它让这条缝从"信仰"变成"有消费者的设计"。
//!
//! # 中立性
//!
//! 本模块只认识 [`GutterMark`]（无/新增/删除）——**不认识 diff 语义**。
//! "哪一行算新增"由调用方（宿主）用 `DiffLineKind` 判定后映射过来。
//! 这样本层不依赖内核轴（门禁 U1），能随 UI 栈一起开源。

use crate::scene::{Color, Op, Rect, Scene};

/// 变更条上的一格（对应 diff 里的一行）。
///
/// 刻意只是"三种态"而不是完整的 diff 语义：本层不需要知道 hunk 头、
/// 上下文行、截断标记 —— 那些对**图形**没有区别（都是"没改"）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GutterMark {
    /// 未改（上下文/无关行）
    Plain,
    /// 新增
    Add,
    /// 删除
    Del,
}

/// 构建变更条场景。
///
/// `width` 是色带宽度、`height` 是可画高度。每一格按**行数比例**分配高度 ——
/// 不这样做的话，200 行的 diff 里改 3 行会看不出有改动（3px 在 200px 里几乎不可见），
/// 而按比例分配至少让"有一处改动"可见。
///
/// # 一个刻意的下限
///
/// 每格高度**至少 1 逻辑像素**：否则长 diff 里的小改动会被取整抹掉，
/// 而"有改动却看不见"比"少显示一格"更糟。代价是极端情况下色带总高会超过
/// `height` —— 那时从底部截断（`Op::PushClip` + 只在范围内画），
/// 保证不越界画出表面之外。
pub fn change_gutter(marks: &[GutterMark], width: f32, height: f32) -> Scene {
    let mut scene = Scene::new();
    if marks.is_empty() || width <= 0.0 || height <= 0.0 {
        return scene;
    }

    // 裁剪到表面范围内：下面的"至少 1px"可能让总高超出 height
    scene.push(Op::PushClip { rect: Rect::new(0.0, 0.0, width, height) });

    let n = marks.len() as f32;
    let per = height / n;
    let mut y = 0.0f32;
    for (i, m) in marks.iter().enumerate() {
        let h = per.max(1.0);
        let color = match m {
            GutterMark::Plain => None,
            GutterMark::Add => Some(Color::from_tone(&neo_text::palette::NEO, neo_text::Tone::Success)),
            GutterMark::Del => Some(Color::from_tone(&neo_text::palette::NEO, neo_text::Tone::Error)),
        };
        if let Some(color) = color {
            scene.fill(Rect::new(0.0, y, width, h), color);
        }
        // 末格贴着底边（避免浮点累计误差留下缝隙）
        y = if i + 1 == marks.len() { height } else { y + per };
    }

    scene.push(Op::PopClip);
    scene
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill_rects(s: &Scene) -> Vec<(f32, f32, f32, f32)> {
        s.ops()
            .iter()
            .filter_map(|op| match op {
                Op::FillRect { rect, .. } => {
                    Some((rect.origin.x, rect.origin.y, rect.size.w, rect.size.h))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn only_changed_lines_produce_rectangles() {
        // 上下文行不该画 —— 它的语义是"没改"，画出来会让色带全是色块，
        // 那就等于没表达"改动在哪"
        let marks = [
            GutterMark::Plain,
            GutterMark::Add,
            GutterMark::Plain,
            GutterMark::Del,
        ];
        let s = change_gutter(&marks, 4.0, 100.0);
        let rects = fill_rects(&s);
        assert_eq!(rects.len(), 2, "只有 Add/Del 该产生矩形：{rects:?}");
        assert!(s.clips_balanced(), "裁剪必须配平（否则后续绘制整片消失）");
    }

    #[test]
    fn marks_are_positioned_proportionally() {
        // 第二条改动在第 3/4 位置 → y 应该落在 50 左右（100px 高、4 行）
        let marks = [GutterMark::Plain, GutterMark::Plain, GutterMark::Add, GutterMark::Plain];
        let s = change_gutter(&marks, 4.0, 100.0);
        let rects = fill_rects(&s);
        assert_eq!(rects.len(), 1);
        let (_, y, _, _) = rects[0];
        assert!((y - 50.0).abs() < 1.0, "第 3 行应落在 50% 处，实际 y={y}");
    }

    /// **每格至少 1px**：长 diff 里的小改动不能被取整抹掉。
    ///
    /// "有改动却看不见"比"少显示一格"更糟 —— 后者用户不会注意，
    /// 前者会让用户以为这个 diff 没改动。这条是那个下限的回归测试。
    #[test]
    fn tiny_marks_keep_at_least_one_pixel() {
        // 1000 行、只有 1 行改动：按比例是 0.1px，必须被抬到 1px
        let mut marks = vec![GutterMark::Plain; 1000];
        marks[500] = GutterMark::Add;
        let s = change_gutter(&marks, 4.0, 100.0);
        let rects = fill_rects(&s);
        assert_eq!(rects.len(), 1);
        let (_, _, _, h) = rects[0];
        assert!(h >= 1.0, "再小的改动也要有 1px，实际 h={h}");
    }

    #[test]
    fn the_last_mark_reaches_the_bottom_edge() {
        // 浮点累计误差不能留缝：末格要贴到底边
        let marks = vec![GutterMark::Add; 7];
        let s = change_gutter(&marks, 4.0, 100.0);
        let rects = fill_rects(&s);
        let (_, y, _, h) = *rects.last().expect("应有矩形");
        assert!((y + h - 100.0).abs() < 0.01, "末格应贴底：y+h={}", y + h);
    }

    #[test]
    fn gutter_is_clipped_so_it_never_paints_outside_the_surface() {
        // 下限是 1px 时总高可能超出 height —— 必须被裁剪兜住，
        // 否则色带会画到相邻的界面元素上
        let marks = vec![GutterMark::Add; 500];
        let s = change_gutter(&marks, 4.0, 50.0);
        assert!(s.clips_balanced(), "必须有配平的裁剪");
        let has_clip = s.ops().iter().any(|o| matches!(o, Op::PushClip { .. }));
        assert!(has_clip, "超出可能时必须压裁剪");
    }

    #[test]
    fn empty_or_degenerate_input_yields_an_empty_scene() {
        assert!(change_gutter(&[], 4.0, 100.0).is_empty(), "没有行就什么都不画");
        // 退化尺寸不该 panic，也不该产出越界矩形
        assert!(change_gutter(&[GutterMark::Add], 0.0, 100.0).is_empty());
        assert!(change_gutter(&[GutterMark::Add], 4.0, 0.0).is_empty());
    }

    /// 颜色走**共享调色板**：新增=Success、删除=Error。
    ///
    /// 这条不只是"颜色对不对"——它保证变更条与 TUI/egui 里的增删色**同源**。
    /// 若这里写死一个绿色，同一处改动在三个宿主的颜色就会漂移。
    #[test]
    fn colors_come_from_the_shared_palette() {
        let s = change_gutter(&[GutterMark::Add, GutterMark::Del], 4.0, 40.0);
        let colors: Vec<Color> = s
            .ops()
            .iter()
            .filter_map(|op| match op {
                Op::FillRect { color, .. } => Some(*color),
                _ => None,
            })
            .collect();
        assert_eq!(colors.len(), 2);
        let success = Color::from_tone(&neo_text::palette::NEO, neo_text::Tone::Success);
        let error = Color::from_tone(&neo_text::palette::NEO, neo_text::Tone::Error);
        assert_eq!(colors[0], success, "新增应取调色板的 Success");
        assert_eq!(colors[1], error, "删除应取调色板的 Error");
        assert_ne!(success, error, "两者必须可区分（否则色带读不出增删）");
    }

    #[test]
    fn the_gutter_scene_paints_through_the_backend() {
        // 端到端（本层内）：中立场景能被后端翻译成绘制产物。
        // 这条证明缝是通的 —— 而不只是"定义了一个 trait"。
        use crate::backend::{GpuiBackend, RenderBackend};
        let marks = [GutterMark::Plain, GutterMark::Add, GutterMark::Del];
        let scene = change_gutter(&marks, 4.0, 60.0);
        let paint = GpuiBackend::new().paint(&scene);
        assert_eq!(paint.quads.len(), 2, "两个改动 → 两个矩形");
        assert_eq!(paint.colors.len(), scene.len(), "颜色与场景逐条对齐");
    }
}
