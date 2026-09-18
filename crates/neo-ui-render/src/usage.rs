//! 用量条形图：把"每一轮花了多少"画成一组柱子。
//!
//! # 它为什么必须自绘
//!
//! 转录里已经逐轮写了 `in / out` 数字（文本渲染，可读、可选中）。但"**趋势**"
//! 用文字表达不了：第 3 轮比第 1 轮贵十倍、或者从某轮开始消耗陡增 ——
//! 这些要横向对比几十个数字才看得出，而人不会那样读。
//!
//! 这是图形问题，所以走渲染缝（方案 §2.2：只有自绘表面走缝，常规组件不走）。
//! 它是缝的第二个真实消费者。
//!
//! # 中立性
//!
//! 本模块不认识"token"这个概念，只认识 [`UsageBar`]（两个非负计数）——
//! 换成"每秒请求数""每轮耗时"同样成立。颜色由调用方从自己的调色板给出
//! （本层不认识语义色调，这样它能随 UI 栈一起开源而不带走主题定义）。
//!
//! # 两个刻意的下限（都与"小值不能被抹掉"有关）
//!
//! 1. **非零数据的柱子至少 1 逻辑像素高**。跨度大时（某轮 100、另一轮 100000）
//!    线性缩放会把小值压成 0 —— 而"有消耗却看不见"比"比例略有失真"更糟。
//! 2. **每根柱子至少 1 逻辑像素宽**（含间隙）。所以能显示的轮数有上限：
//!    宽度不足以逐根画时，**只显示最近的那些轮**（像行情图显示最近 N 日），
//!    而不是把多轮合并成一根 —— 合并会让"某一轮特别贵"这个信息消失。
//!
//! 两处都靠配平的裁剪兜住越界（[`Scene::clips_balanced`] 守着）。

use crate::scene::{Color, Op, Rect, Scene};

/// 一根柱子：某一轮的用量。
///
/// 两个计数都是**非负**的。`input` 画在下方、`output` 画在上方（堆叠），
/// 这样一根柱子的总高就是这一轮的总量，段高表达构成。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UsageBar {
    pub input: u64,
    pub output: u64,
}

impl UsageBar {
    pub fn new(input: u64, output: u64) -> Self {
        Self { input, output }
    }

    pub fn total(&self) -> u64 {
        self.input.saturating_add(self.output)
    }
}

/// 柱间间隙（逻辑像素）。留间隙才能数清有几轮；
/// 宽度不够时会自动降到 0（见 [`usage_bars`] 的说明）。
const GAP: f32 = 1.0;

/// 单根柱子的最大宽度（逻辑像素）。
///
/// # 为什么需要上限
///
/// 柱子宽度原本是"把可用宽度按数量均分"，于是**只有一轮时柱子占满整行** ——
/// 真机上看起来像一条横向色带，既不像图表，也读不出"它占了满高"的含义
/// （因为同时占满了宽）。有上限之后，每一轮都是一根**竖条**，
/// 「竖条越高 = 这一轮用得越多」这个读法才成立。
///
/// 取 12：够宽到看得清两段颜色，又不至于在轮数少时显得像色块。
const MAX_BAR_W: f32 = 12.0;

/// 构建用量条形图场景。
///
/// 参数：
/// - `bars`：按时间顺序的用量（**越靠后越新**）。显示最近的那些轮。
/// - `width` / `height`：可画区域。
/// - `input_color` / `output_color`：两段的颜色（由调用方从调色板取）。
///
/// 返回的场景以左下角为原点：柱子从底部向上长。
pub fn usage_bars(
    bars: &[UsageBar],
    width: f32,
    height: f32,
    input_color: Color,
    output_color: Color,
    // 柱顶圆角半径。由调用方从设计系统取（与颜色同一来源）——
    // 不在渲染层写死：换主题时圆角应与颜色一起变。
    radius: f32,
) -> Scene {
    let mut scene = Scene::new();
    if bars.is_empty() || width <= 0.0 || height <= 0.0 {
        return scene;
    }

    // 能画几根：先按"每根至少 1px + 1px 间隙"算容量，不够就只取最近的。
    // 用 floor 而不是 round —— 宁可少画一根，也不要把某一根压到 0 宽（看不见）。
    let capacity = (width / (1.0 + GAP)).floor().max(1.0) as usize;
    let shown = if bars.len() > capacity {
        &bars[bars.len() - capacity..]
    } else {
        bars
    };
    let n = shown.len() as f32;

    // 宽度够时用 1px 间隙；不够时降为 0（柱子紧挨着也比看不见强）
    let gap = if (width - (n - 1.0) * GAP) / n >= 1.0 { GAP } else { 0.0 };
    let bar_w = ((width - (n - 1.0) * gap) / n).max(1.0).min(MAX_BAR_W);

    // 归一化基准：**所有**数据的最大值，不只是显示的那些 ——
    // 若按可见集归一化，滚动/新增数据时柱子高度会整体跳变（同一份数据
    // 两次渲染长得不一样），那会让趋势看起来在变而实际没变。
    let max_total = bars.iter().map(|b| b.total()).max().unwrap_or(0);
    if max_total == 0 {
        return scene; // 全是零：不画，避免一片"齐平的空柱子"造成误解
    }

    scene.push(Op::PushClip { rect: Rect::new(0.0, 0.0, width, height) });

    for (i, b) in shown.iter().enumerate() {
        let x = i as f32 * (bar_w + gap);
        let total = b.total();
        if total == 0 {
            continue; // 这一轮没有任何消耗：不画柱子（画 1px 会与"极少消耗"混淆）
        }
        // 总高按最大值线性缩放，但**非零至少 1px**
        let h = ((total as f32 / max_total as f32) * height).max(1.0);

        // 段高按 in/out 比例分；某一项为 0 时那一段不画
        let in_frac = b.input as f32 / total as f32;
        let in_h = (h * in_frac).round();
        let out_h = (h - in_h).max(0.0);

        // 从底部向上堆叠：先 output（在下），再 input？——
        // 这里选 **input 在下、output 在上**，因为读图时"底部是输入"更符合
        // "输入决定输出"的直觉；且两根柱子对比时底部对齐的是同类量。
        if in_h > 0.0 {
            // 柱**顶**圆角：让两根堆叠的柱子看起来是一根有圆头的条，
            // 而不是两块硬边色块（这是"图表"与"色块"在观感上的分界）。
            // 半径取 `min(bar_w/2, radius)` —— 超过半宽 gpui 会夹取，
            // 但我们自己先夹能保证各后端结果一致（headless 的 SVG 不夹）。
            scene.fill_rounded(
                Rect::new(x, height - in_h, bar_w, in_h),
                input_color,
                radius.min(bar_w / 2.0),
            );
        }
        if out_h > 0.0 {
            scene.fill_rounded(
                Rect::new(x, height - in_h - out_h, bar_w, out_h),
                output_color,
                radius.min(bar_w / 2.0),
            );
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
        let s = usage_bars(&[], 100.0, 40.0, c(1), c(2), 0.0);
        assert!(s.ops().is_empty());
        assert!(s.clips_balanced());
    }

    #[test]
    fn all_zero_yields_nothing_not_flat_bars() {
        // 全零时画"齐平的空柱子"会让人以为有数据 —— 宁可不画
        let bars = vec![UsageBar::new(0, 0), UsageBar::new(0, 0)];
        let s = usage_bars(&bars, 100.0, 40.0, c(1), c(2), 0.0);
        assert!(rects(&s).is_empty());
    }

    /// 两根柱子的相对高度应反映用量比例（这是图表的全部意义）。
    ///
    /// ⚠️ 分组依据是**柱子的横坐标**（各自唯一的 x），不是"x < 宽度一半" ——
    /// 后者在加了柱子最大宽度之后就不成立了：宽度上限让两根柱子都挤在左侧，
    /// 用"半宽"分组会把它们算进同一根（实测因此测试失败，而组件是对的）。
    #[test]
    fn bar_heights_are_proportional() {
        let bars = vec![UsageBar::new(50, 50), UsageBar::new(25, 25)];
        let s = usage_bars(&bars, 100.0, 100.0, c(1), c(1), 0.0);
        let r = rects(&s);
        // 按横坐标归组（同一根柱子的各段 x 相同）
        let mut by_x: std::collections::BTreeMap<u32, f32> = std::collections::BTreeMap::new();
        for (rect, _) in &r {
            *by_x.entry((rect.origin.x * 100.0) as u32).or_insert(0.0) += rect.size.h;
        }
        let hs: Vec<f32> = by_x.values().copied().collect();
        assert_eq!(hs.len(), 2, "应有两根柱子，实际 {hs:?}");
        assert!((hs[0] - 100.0).abs() < 0.01, "最大值应占满高度，实际 {}", hs[0]);
        assert!((hs[1] - 50.0).abs() < 0.01, "一半用量应占一半高度，实际 {}", hs[1]);
    }

    /// 柱子宽度有上限：只有一轮时若占满整行，看起来像横向色带而不是图表，
    /// 「越高=越多」的读法就不成立。
    #[test]
    fn a_single_bar_does_not_span_the_whole_width() {
        let bars = vec![UsageBar::new(10, 10)];
        let s = usage_bars(&bars, 200.0, 40.0, c(1), c(2), 0.0);
        for (rect, _) in rects(&s) {
            assert!(
                rect.size.w <= MAX_BAR_W + 0.01,
                "单根柱子不该占满 {:?}（看起来像色带）",
                rect.size
            );
        }
    }

    /// **小值不能被抹掉**：跨度大时线性缩放会把小值压成 0 高。
    /// 那会让"有消耗"看起来像"没消耗"。
    #[test]
    fn tiny_values_still_get_a_visible_bar() {
        let bars = vec![UsageBar::new(1_000_000, 0), UsageBar::new(1, 0)];
        let s = usage_bars(&bars, 100.0, 100.0, c(1), c(2), 0.0);
        let r = rects(&s);
        assert_eq!(r.len(), 2, "两根柱子都要画出来");
        let tiny = r.iter().map(|(x, _)| x.size.h).fold(f32::MAX, f32::min);
        assert!(tiny >= 1.0, "极小值也应有至少 1px 可见，实际 {tiny}");
    }

    /// 柱子从**底部**向上长（图表惯例）。原点在左上角是渲染约定，
    /// 所以"底部"意味着 y 更大。
    #[test]
    fn bars_grow_from_the_bottom() {
        let bars = vec![UsageBar::new(10, 0)];
        let s = usage_bars(&bars, 100.0, 40.0, c(1), c(2), 0.0);
        let r = rects(&s);
        assert_eq!(r.len(), 1);
        let (rect, _) = r[0];
        assert!(
            (rect.origin.y + rect.size.h - 40.0).abs() < 0.01,
            "柱子底边应贴住区域底部：{:?}",
            rect
        );
    }

    #[test]
    fn input_and_output_are_drawn_as_two_segments() {
        let bars = vec![UsageBar::new(30, 70)];
        let s = usage_bars(&bars, 100.0, 100.0, c(1), c(2), 0.0);
        let r = rects(&s);
        assert_eq!(r.len(), 2, "两段：输入 + 输出");
        let input_seg = r.iter().find(|(_, col)| *col == c(1)).expect("应有输入段");
        let output_seg = r.iter().find(|(_, col)| *col == c(2)).expect("应有输出段");
        assert!((input_seg.0.size.h - 30.0).abs() < 1.0, "输入段占 30%");
        assert!((output_seg.0.size.h - 70.0).abs() < 1.0, "输出段占 70%");
        // 输入在下（y 更大）、输出在上
        assert!(input_seg.0.origin.y > output_seg.0.origin.y);
    }

    /// 只有一类用量时，另一段不画（画 0 高的段没有意义，也会多出无用绘制指令）。
    #[test]
    fn a_zero_component_draws_no_segment() {
        let bars = vec![UsageBar::new(0, 50)];
        let s = usage_bars(&bars, 100.0, 100.0, c(1), c(2), 0.0);
        let r = rects(&s);
        assert_eq!(r.len(), 1, "只有输出段");
        assert_eq!(r[0].1, c(2));
    }

    /// 宽度不足以逐根画时，**只显示最近的**，不合并 ——
    /// 合并会让"某一轮特别贵"消失，而那正是要看的东西。
    #[test]
    fn narrow_width_keeps_the_most_recent_bars() {
        let bars: Vec<UsageBar> = (1..=20).map(|i| UsageBar::new(i * 10, 0)).collect();
        // 宽度只够约 4 根（每根 1px + 1px 间隙）
        let s = usage_bars(&bars, 8.0, 40.0, c(1), c(2), 0.0);
        let r = rects(&s);
        let count = r.iter().filter(|(_, col)| *col == c(1)).count();
        assert!(count <= 4, "窄区域不该画出 20 根，实际 {count}");
        assert!(count >= 1);
        // 保留的应是**最后**那些（用量最大的几根也在后面）
        let rightmost = r.iter().map(|(x, _)| x.origin.x).fold(f32::MIN, f32::max);
        assert!(rightmost <= 8.0, "不应画出区域之外");
    }

    /// 每根柱子至少 1px 宽 —— 否则窄图里柱子会退化成看不见的竖线。
    #[test]
    fn every_bar_has_at_least_one_pixel_of_width() {
        let bars: Vec<UsageBar> = (1..=10).map(|_| UsageBar::new(10, 0)).collect();
        let s = usage_bars(&bars, 10.0, 40.0, c(1), c(2), 0.0);
        for (rect, _) in rects(&s) {
            assert!(rect.size.w >= 1.0, "柱子宽度不该小于 1px：{:?}", rect);
        }
    }

    /// 归一化基准取**全部数据**的最大值，不只是可见集 ——
    /// 否则新增数据时可见柱子的高度会整体跳变（同一份历史两次渲染不一样）。
    #[test]
    fn normalization_uses_all_data_not_only_visible() {
        // 前面有个巨大的值，后面都是小值；可见的是小值那部分
        let mut bars = vec![UsageBar::new(1_000_000, 0)];
        bars.extend((1..=20).map(|_| UsageBar::new(1000, 0)));
        let s = usage_bars(&bars, 8.0, 100.0, c(1), c(2), 0.0);
        // 可见柱子的高度应远小于满高（因为基准是那个百万级的值）
        for (rect, _) in rects(&s) {
            assert!(
                rect.size.h < 5.0,
                "可见柱高应按全局最大值缩放，实际 {:?}",
                rect.size.h
            );
        }
    }

    #[test]
    fn zero_or_negative_extent_is_handled() {
        for (w, h) in [(0.0, 10.0), (10.0, 0.0), (-1.0, 10.0)] {
            let s = usage_bars(&[UsageBar::new(1, 1)], w, h, c(1), c(2), 0.0);
            assert!(s.ops().is_empty(), "尺寸非法时应返回空场景");
        }
    }

    /// 绘制指令必须配平裁剪 —— 否则裁剪栈会泄漏到后续绘制（画到别的元素上）。
    #[test]
    fn clips_are_always_balanced() {
        let bars = vec![UsageBar::new(10, 20), UsageBar::new(0, 0), UsageBar::new(5, 5)];
        let s = usage_bars(&bars, 50.0, 30.0, c(1), c(2), 0.0);
        assert!(s.clips_balanced());
    }

    /// 单根、极窄等边界组合不该 panic。
    #[test]
    fn degenerate_inputs_do_not_panic() {
        let _ = usage_bars(&[UsageBar::new(1, 0)], 1.0, 1.0, c(1), c(2), 0.0);
        let _ = usage_bars(&[UsageBar::new(u64::MAX, u64::MAX)], 10.0, 10.0, c(1), c(2), 0.0);
        let _ = usage_bars(&[UsageBar::default()], 10.0, 10.0, c(1), c(2), 0.0);
    }
}
