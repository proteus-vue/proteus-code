//! 分段进度条：把"每个子任务走到哪一步了"画成一条分段色带。
//!
//! # 它为什么必须自绘
//!
//! 目标面板里已经逐行写了子任务文字（`· 标题`，文本渲染对了）。但"**整条目标
//! 推进到什么程度**"用文字表达不了：五个子任务、每个都在不同阶段，
//! 读五行字得自己在脑子里数。
//!
//! 一条分段色带把这件事变成一眼可见：**看有多少比例已经到末段**。
//! 这是图形问题（连续几何 + 比例），所以走渲染缝。
//! 它是缝的第三个真实消费者。
//!
//! # 中立性
//!
//! 本模块不认识"目标""子任务"这些概念，只认识 [`ProgressSegment`]
//! （一个非负的"已完成步数 / 总步数"）。换成"构建流水线""下载分片"
//! 同样成立。这样它能随 UI 栈一起开源（门禁 U1 守着）。
//!
//! # 三个刻意的处理
//!
//! 1. **零段的进度不画**（用底色表示），而不是画一段 1px 高的 ——
//!    "没开始"与"刚开始"在视觉上必须能区分。
//! 2. **已完成的段画满**。`done == total` 时整段填实，不再按比例留一点空隙 ——
//!    那一点点空隙会让人怀疑"是不是还差一步"。
//! 3. **每段至少 1 逻辑像素宽**，哪怕子任务很多、可用宽度很窄。
//!    窄到看不清时，**均匀铺满**（每段 1px）比"挤在一起"更能表达"有几段"。

use crate::scene::{Color, Op, Rect, Scene};

/// 一个子任务的进度（已完成步数 / 总步数）。
///
/// 两个都是**非负**整数；`done > total` 会被夹到 `total`（调用方传错时
/// 不至于画出超过 100% 的段）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProgressSegment {
    pub done: u32,
    pub total: u32,
}

impl ProgressSegment {
    pub fn new(done: u32, total: u32) -> Self {
        Self { done, total }
    }

    /// 这一段是否已完成（画满）。
    pub fn is_complete(&self) -> bool {
        self.total > 0 && self.done >= self.total
    }

    /// 完成比例（0.0–1.0）。`total == 0` 时视为 0（没有可完成的事，
    /// 而不是"全部完成"—— 后者会让空输入显示成满进度）。
    pub fn fraction(&self) -> f32 {
        if self.total == 0 {
            return 0.0;
        }
        (self.done.min(self.total) as f32) / (self.total as f32)
    }
}

/// 段间间隙（逻辑像素）。留间隙才能数清有几段。
const GAP: f32 = 2.0;

/// 构建分段进度条场景。
///
/// - `segments`：按顺序（左到右）的子任务进度。
/// - `width` / `height`：可画区域。
/// - `track_color`：未完成部分的底色（画整条）。
/// - `fill_color`：已完成部分的颜色。
pub fn segmented_progress(
    segments: &[ProgressSegment],
    width: f32,
    height: f32,
    track_color: Color,
    fill_color: Color,
    // 段的圆角半径（同样由调用方从设计系统取）。
    radius: f32,
) -> Scene {
    let mut scene = Scene::new();
    if segments.is_empty() || width <= 0.0 || height <= 0.0 {
        return scene;
    }

    let n = segments.len() as f32;
    // 段宽：按数量均分，再扣掉间隙。窄到 1px 以下就铺满（间隙降为 0）——
    // "紧挨着但看得见几段"比"挤成一团看不清"好。
    let total_gaps = (n - 1.0) * GAP;
    let (seg_w, gap) = if (width - total_gaps) / n >= 1.0 {
        ((width - total_gaps) / n, GAP)
    } else {
        (width / n, 0.0)
    };

    scene.push(Op::PushClip { rect: Rect::new(0.0, 0.0, width, height) });

    for (i, seg) in segments.iter().enumerate() {
        let x = i as f32 * (seg_w + gap);

        // 1) 先画整段的底色（表示"这一段存在，但还没走完"）
        //
        // 圆角夹到"半宽/半高"之内：超过之后 gpui 会自己夹取，但 headless
        // 的 SVG 不夹 —— 两个后端会给不同结果。**在这里夹一次**，
        // 让"中立场景"本身就只有一种解释（跨后端一致性由此保证）。
        let r = radius.min(seg_w / 2.0).min(height / 2.0);
        scene.fill_rounded(Rect::new(x, 0.0, seg_w, height), track_color, r);

        // 2) 再按比例覆盖已完成部分
        if seg.total == 0 {
            // 没有可完成的步数：只留底色（见模块注释第 1 条）
            continue;
        }
        if seg.is_complete() {
            // 完成时画满整段，不留空隙（第 2 条）
            scene.fill_rounded(Rect::new(x, 0.0, seg_w, height), fill_color, r);
        } else {
            let w = seg_w * seg.fraction();
            if w >= 1.0 {
                // 已走部分：右端也要圆角（否则"走了一半"的截断处是硬边，
                // 看起来像被切掉而不是"进行中"）。半径按**实际宽度**再夹一次。
                scene.fill_rounded(
                    Rect::new(x, 0.0, w, height),
                    fill_color,
                    r.min(w / 2.0),
                );
            }
            // 比例不足 1px 就不画那一小条：它会退化成"几乎看不见"，
            // 而底色已经表达了"未完成"。
        }
    }

    scene.push(Op::PopClip);
    scene
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(v: u8) -> Color {
        Color::rgb(v, v, v)
    }

    fn rects(s: &Scene) -> Vec<(Rect, Color)> {
        s.ops()
            .iter()
            .filter_map(|o| match o {
                Op::FillRect { rect, color, .. } => Some((*rect, *color)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn empty_input_yields_empty_scene() {
        let s = segmented_progress(&[], 100.0, 8.0, c(1), c(2), 0.0);
        assert!(s.ops().is_empty());
        assert!(s.clips_balanced());
    }

    #[test]
    fn zero_size_yields_empty_scene() {
        for (w, h) in [(0.0, 8.0), (100.0, 0.0), (-5.0, 8.0)] {
            let s = segmented_progress(&[ProgressSegment::new(1, 2)], w, h, c(1), c(2), 0.0);
            assert!(s.ops().is_empty(), "尺寸 {w}x{h} 应返回空场景");
        }
    }

    /// **零进度的段只画底色** —— "没开始"与"刚开始一点"必须能区分。
    /// 画一段 1px 会让人以为已经动过了。
    #[test]
    fn a_segment_with_no_progress_draws_only_the_track() {
        let s = segmented_progress(&[ProgressSegment::new(0, 3)], 100.0, 8.0, c(1), c(2), 0.0);
        let r = rects(&s);
        assert_eq!(r.len(), 1, "只应有底色");
        assert_eq!(r[0].1, c(1), "是底色而不是填充色");
    }

    /// **完成的段画满**，不留按比例算出的空隙 ——
    /// 那一点点空隙会让人怀疑"是不是还差一步"。
    #[test]
    fn a_complete_segment_fills_its_whole_width() {
        let s = segmented_progress(&[ProgressSegment::new(3, 3)], 100.0, 8.0, c(1), c(2), 0.0);
        let r = rects(&s);
        let fill = r.iter().find(|(_, col)| *col == c(2)).expect("应有填充");
        assert!((fill.0.size.w - 100.0).abs() < 0.01, "完成段应画满，实际 {:?}", fill.0);
    }

    /// 部分完成：填充宽度 = 段宽 × 比例。
    #[test]
    fn partial_progress_is_proportional() {
        let s = segmented_progress(&[ProgressSegment::new(1, 4)], 100.0, 8.0, c(1), c(2), 0.0);
        let r = rects(&s);
        let fill = r.iter().find(|(_, col)| *col == c(2)).expect("应有填充");
        assert!((fill.0.size.w - 25.0).abs() < 0.01, "1/4 应占 25%，实际 {:?}", fill.0);
    }

    /// 每段先画底色再覆盖填充 —— 顺序反了填充会被底色盖住。
    #[test]
    fn track_is_drawn_before_fill() {
        let s = segmented_progress(&[ProgressSegment::new(1, 2)], 100.0, 8.0, c(1), c(2), 0.0);
        let r = rects(&s);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].1, c(1), "底色先画");
        assert_eq!(r[1].1, c(2), "填充后画（覆盖在上面）");
    }

    /// `total == 0` 表示"没有可完成的事"，不是"全部完成" ——
    /// 后者会让空输入显示成满进度。
    #[test]
    fn zero_total_is_not_treated_as_complete() {
        let seg = ProgressSegment::new(0, 0);
        assert!(!seg.is_complete());
        assert_eq!(seg.fraction(), 0.0);
        let s = segmented_progress(&[seg], 100.0, 8.0, c(1), c(2), 0.0);
        let r = rects(&s);
        assert_eq!(r.len(), 1, "只画底色");
        assert_eq!(r[0].1, c(1));
    }

    /// `done > total`（调用方传错）夹到 100%，不画出超过整段的宽度。
    #[test]
    fn done_beyond_total_is_clamped() {
        let seg = ProgressSegment::new(99, 3);
        assert!(seg.is_complete());
        assert_eq!(seg.fraction(), 1.0);
        let s = segmented_progress(&[seg], 100.0, 8.0, c(1), c(2), 0.0);
        let fill = rects(&s).into_iter().find(|(_, col)| *col == c(2)).unwrap();
        assert!(fill.0.size.w <= 100.01);
    }

    /// 多段：按顺序从左到右排布，段间有间隙。
    #[test]
    fn segments_are_laid_out_left_to_right_with_gaps() {
        let segs = vec![
            ProgressSegment::new(2, 2),
            ProgressSegment::new(0, 2),
            ProgressSegment::new(1, 2),
        ];
        let s = segmented_progress(&segs, 100.0, 8.0, c(1), c(2), 0.0);
        let r = rects(&s);
        // 每段一条底色，共 3 条；填充：第 1、3 段各 1 条
        let tracks: Vec<f32> = r.iter().filter(|(_, col)| *col == c(1)).map(|(x, _)| x.origin.x).collect();
        assert_eq!(tracks.len(), 3);
        assert!(tracks[0] < tracks[1] && tracks[1] < tracks[2], "应按顺序排布");
    }

    /// 段很多、宽度很窄时**均匀铺满**（间隙降为 0），每段仍 >= 1px ——
    /// "紧挨着但数得出几段"比"挤成一团"好。
    #[test]
    fn narrow_width_gives_every_segment_at_least_one_pixel() {
        let segs: Vec<ProgressSegment> = (0..10).map(|_| ProgressSegment::new(1, 2)).collect();
        let s = segmented_progress(&segs, 12.0, 8.0, c(1), c(2), 0.0);
        let tracks: Vec<Rect> = rects(&s)
            .into_iter()
            .filter(|(_, col)| *col == c(1))
            .map(|(r, _)| r)
            .collect();
        assert_eq!(tracks.len(), 10, "十段都要画出来");
        for t in tracks {
            assert!(t.size.w >= 0.99, "每段至少 1px，实际 {t:?}");
        }
    }

    /// 宽度不足以逐段画时也不该画出区域之外（裁剪兜住）。
    #[test]
    fn never_draws_outside_the_given_width() {
        let segs: Vec<ProgressSegment> = (0..50).map(|_| ProgressSegment::new(1, 1)).collect();
        let s = segmented_progress(&segs, 20.0, 8.0, c(1), c(2), 0.0);
        for (rect, _) in rects(&s) {
            assert!(
                rect.origin.x + rect.size.w <= 20.01,
                "画到区域外了：{:?}",
                rect
            );
        }
    }

    #[test]
    fn clips_are_always_balanced() {
        let segs = vec![ProgressSegment::new(1, 3), ProgressSegment::new(0, 0)];
        let s = segmented_progress(&segs, 50.0, 6.0, c(1), c(2), 0.0);
        assert!(s.clips_balanced());
    }

    #[test]
    fn degenerate_inputs_do_not_panic() {
        let _ = segmented_progress(&[ProgressSegment::new(u32::MAX, u32::MAX)], 1.0, 1.0, c(1), c(2), 0.0);
        let _ = segmented_progress(&[ProgressSegment::default()], 1.0, 1.0, c(1), c(2), 0.0);
    }
}
