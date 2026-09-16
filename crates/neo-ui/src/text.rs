//! 富文本：一段文字 + 语义色调区间 + 匹配高亮区间。
//!
//! # 为什么它把两件事合在一起
//!
//! "按语义色调给不同片段上色"（Markdown 渲染、语法高亮）与"把命中的片段标出来"
//! （搜索）看起来是两个需求，底层却是**同一件事**：给一段文字的若干字节区间
//! 指定绘制样式。分开做会得到两份几乎一样的区间归一化与边界检查代码，
//! 而边界处理（字节 vs 字符、区间越界、区间重叠）恰恰是最容易写错的部分。
//!
//! # 为什么用 `StyledText` 而不是拼一串 `div`
//!
//! 逐片段拼盒子看起来更直观，实际上会让排版坏掉：并排的盒子各自参与布局，
//! 中英混排时每一段各占各的宽度，**断行位置与基线全错**。
//! `StyledText` 把整段当一个文字块交给文本系统，区间只影响绘制样式。
//! 这个区别在纯英文短句上看不出来，在中文长句上立刻可见。
//!
//! # 区间是**字节**偏移，且必须落在字符边界上
//!
//! 因为底层 API 收的就是字节区间。切在字符中间（多字节字符的一半）会
//! 产生无效样式范围。本组件**不 panic**：无效区间被忽略并如实计数，
//! 调用方可以据此发现自己的 bug。理由见 [`RichText::invalid_ranges`]。

use crate::neo_color;
use neo_text::Tone;

/// 一段富文本。
///
/// ```
/// use neo_text::Tone;
/// use neo_ui::RichText;
///
/// // 按片段构造（每段一个语义色调）
/// let rt = RichText::from_spans(&[
///     ("普通 ".to_string(), Tone::Text),
///     ("强调".to_string(), Tone::Primary),
/// ]);
/// assert_eq!(rt.text(), "普通 强调");
/// assert_eq!(rt.invalid_ranges(), 0);
///
/// // 再把搜索命中的区间加上去
/// let rt = rt.mark(0..6);
/// assert_eq!(rt.mark_count(), 1);
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RichText {
    text: String,
    /// 语义色调区间（前景色）。后加的同位置区间覆盖先加的。
    spans: Vec<(std::ops::Range<usize>, Tone)>,
    /// 匹配高亮区间（背景色）。与 `spans` 独立：命中处的前景色仍由 `spans` 决定。
    marks: Vec<std::ops::Range<usize>>,
    /// 被忽略的无效区间数（越界 / 不在字符边界）。
    invalid: usize,
}

impl RichText {
    /// 空文本。
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into(), ..Default::default() }
    }

    /// 按「片段 + 色调」构造。片段首尾相接，不校验也不重叠 ——
    /// 这是给"解析器已经切好的片段"用的入口（如 Markdown 行）。
    pub fn from_spans(spans: &[(String, Tone)]) -> Self {
        let mut out = Self::default();
        for (seg, tone) in spans {
            let start = out.text.len();
            out.text.push_str(seg);
            out.spans.push((start..start + seg.len(), *tone));
        }
        out
    }

    /// 加一个语义色调区间。无效区间被忽略（见 [`Self::invalid_ranges`]）。
    pub fn span(mut self, range: std::ops::Range<usize>, tone: Tone) -> Self {
        if self.valid(&range) {
            self.spans.push((range, tone));
        }
        self
    }

    /// 加一个匹配高亮区间。
    pub fn mark(mut self, range: std::ops::Range<usize>) -> Self {
        if self.valid(&range) {
            self.marks.push(range);
        }
        self
    }

    /// 一次加多个匹配高亮区间（搜索结果通常是一批）。
    pub fn marks(mut self, ranges: &[std::ops::Range<usize>]) -> Self {
        for r in ranges {
            self = self.mark(r.clone());
        }
        self
    }

    /// 文本内容。
    pub fn text(&self) -> &str {
        &self.text
    }

    /// 高亮区间个数。
    pub fn mark_count(&self) -> usize {
        self.marks.len()
    }

    /// 被忽略的无效区间数。
    ///
    /// **为什么要暴露它**：无效区间几乎总是调用方算错了偏移（比如把字符下标
    /// 当字节下标，或用了过期的高亮结果）。静默丢弃会让"高亮没出现"变成一个
    /// 无从查起的问题；如实计数则让调用方（与它的测试）能立刻发现。
    /// 但界面**不该**因此崩溃或报错弹窗 —— 文本本身仍然完整显示。
    pub fn invalid_ranges(&self) -> usize {
        self.invalid
    }

    /// 把 spans 与 marks 归并成**有序且互不重叠**的绘制区间。
    ///
    /// 为什么要归并：底层文本系统（gpui 的 `compute_runs`）按顺序消费高亮
    /// 区间并把「上一个的结束」当作「下一个的开始」—— 它**假设区间有序
    /// 且不重叠**。直接把它俩倒进去，重叠处会算错长度、甚至 panic。
    /// 而归并需要同时处理"前景色来自 spans、背景色来自 marks"两个来源，
    /// 这正是调用方最不该自己重写一遍的东西。
    ///
    /// 归并规则（按优先级）：
    /// - 把两个来源切成**边界点分割的最小段**，每段只属于一个 span 与一个 mark；
    /// - 段的色调取覆盖它的 span（同位置多个 span 时取**后者**，即"后加者生效"）；
    /// - 段是否高亮，取决于它是否落在任一 mark 内。
    pub fn runs(&self) -> Vec<(std::ops::Range<usize>, Option<Tone>, bool)> {
        // 收集所有边界点
        let mut cuts: Vec<usize> = vec![0, self.text.len()];
        for (r, _) in &self.spans {
            cuts.push(r.start);
            cuts.push(r.end);
        }
        for r in &self.marks {
            cuts.push(r.start);
            cuts.push(r.end);
        }
        cuts.retain(|c| *c <= self.text.len() && self.text.is_char_boundary(*c));
        cuts.sort_unstable();
        cuts.dedup();

        let mut out = Vec::new();
        for w in cuts.windows(2) {
            let (start, end) = (w[0], w[1]);
            if start == end {
                continue;
            }
            // 该段所属的 span：后加者优先（与 span() 的文档一致）
            let tone = self
                .spans
                .iter()
                .rev()
                .find(|(r, _)| r.start <= start && end <= r.end)
                .map(|(_, t)| *t);
            let marked = self.marks.iter().any(|r| r.start <= start && end <= r.end);
            out.push((start..end, tone, marked));
        }
        out
    }

    fn valid(&mut self, r: &std::ops::Range<usize>) -> bool {
        let ok = r.start <= r.end
            && r.end <= self.text.len()
            // 必须落在字符边界上：切在多字节字符中间会产生无效范围
            && self.text.is_char_boundary(r.start)
            && self.text.is_char_boundary(r.end)
            // 空区间没有可绘制的范围，不算错，但也不记录
            && r.start != r.end;
        if !ok && r.start < r.end {
            self.invalid += 1;
        }
        ok && r.start != r.end
    }
}

/// 把 [`RichText`] 渲染成一个元素。
///
/// # 为什么需要它而不是让调用方自己拼
///
/// 调用方要自己做三件事：把 span 与 mark 归并成有序不重叠的区间、
/// 把语义色调翻成具体颜色、把区间喂给文本系统。前两件各有各的错法
/// （归并错了会丢字，颜色取错了会串色），第三件还依赖具体框架的
/// 区间语义（必须有序不重叠）。三件都在这里做一次，调用方只给内容。
///
/// # 命中高亮的样式
///
/// 命中处**保留**语义色调作为前景色，只加背景色 —— 若用固定前景色覆盖，
/// 搜索命中会让"这段是代码/这段是强调"的信息全部丢失。这是刻意的取舍：
/// 用户搜到某处时，仍需要看懂那段文字的语义层次。
pub fn rich_text(rt: &RichText) -> impl neo_ui_kit::gpui::IntoElement {
    use neo_ui_kit::gpui::prelude::*;
    use neo_ui_kit::gpui::HighlightStyle;

    let text = rt.text().to_string();
    let runs = rt.runs();

    // 全空白的文本不产生任何高亮（也不该因为 runs 为空而画出空元素）
    let highlights: Vec<(std::ops::Range<usize>, HighlightStyle)> = runs
        .into_iter()
        .filter_map(|(range, tone, marked)| {
            // 既没有色调也没有高亮 → 用默认样式，不必进高亮列表
            if tone.is_none() && !marked {
                return None;
            }
            let mut style = HighlightStyle::default();
            if let Some(t) = tone {
                style.color = Some(neo_color(t).into());
            }
            if marked {
                style.background_color = Some(neo_color(Tone::Primary).into());
            }
            Some((range, style))
        })
        .collect();

    if highlights.is_empty() {
        return neo_ui_kit::gpui::div().child(text).into_any_element();
    }
    neo_ui_kit::gpui::StyledText::new(text)
        .with_highlights(highlights)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_spans_concatenates_and_offsets_are_correct() {
        let rt = RichText::from_spans(&[
            ("ab".into(), Tone::Text),
            ("中文".into(), Tone::Primary),
        ]);
        assert_eq!(rt.text(), "ab中文");
        // 第二段起点是 2（"ab" 的字节数），长度是 6（"中文" 两个三字节字符）
        assert_eq!(rt.spans[1].0, 2..8);
    }

    #[test]
    fn invalid_ranges_are_counted_not_panicked() {
        let mut rt = RichText::new("abc");
        // 越界
        assert!(!rt.valid(&(0..99)));
        assert_eq!(rt.invalid, 1);
        // 合理的区间不该被计入
        assert!(rt.valid(&(0..3)));
        assert_eq!(rt.invalid, 1, "合法区间不该增加无效计数");
    }

    /// 切在多字节字符中间必须被拒 —— 否则底层文本系统会拿到无效范围。
    #[test]
    fn ranges_must_fall_on_char_boundaries() {
        let rt = RichText::new("中文");
        // "中" 占 0..3，"文" 占 3..6。取 1..4 切在字符中间。
        assert_eq!(rt.clone().span(1..4, Tone::Primary).invalid_ranges(), 1);
        // 边界正确
        assert_eq!(rt.clone().mark(0..3).invalid_ranges(), 0);
        assert_eq!(rt.mark(3..6).invalid_ranges(), 0);
    }

    #[test]
    fn empty_range_is_ignored_but_not_an_error() {
        let rt = RichText::new("abc").mark(1..1);
        assert_eq!(rt.mark_count(), 0, "空区间没有可绘制范围");
        assert_eq!(rt.invalid_ranges(), 0, "空区间不是错误");
    }

    #[test]
    fn marks_and_spans_are_independent() {
        // 命中处的前景色仍由 span 决定，mark 只加背景
        let rt = RichText::new("hello")
            .span(0..5, Tone::Muted)
            .mark(1..3);
        assert_eq!(rt.mark_count(), 1);
        assert_eq!(rt.spans.len(), 1, "mark 不该影响 span");
        assert_eq!(rt.invalid_ranges(), 0);
    }

    #[test]
    fn marks_helper_accepts_a_batch() {
        let rt = RichText::new("abcdef").marks(&[0..1, 2..3, 4..5]);
        assert_eq!(rt.mark_count(), 3);
    }

    /// 归并出来的段必须**首尾相接、覆盖全文且不重叠** ——
    /// 底层文本系统按顺序消费这些段，缺一段就会丢掉那段文字。
    #[test]
    fn runs_tile_the_whole_text_without_gaps_or_overlaps() {
        let rt = RichText::new("abcdef")
            .span(0..3, Tone::Primary)
            .mark(2..5);
        let runs = rt.runs();
        // 覆盖全文
        assert_eq!(runs.first().unwrap().0.start, 0);
        assert_eq!(runs.last().unwrap().0.end, 6);
        // 首尾相接
        for w in runs.windows(2) {
            assert_eq!(w[0].0.end, w[1].0.start, "段之间不能有缝或重叠：{runs:?}");
        }
        // 全文长度守恒
        let total: usize = runs.iter().map(|(r, _, _)| r.len()).sum();
        assert_eq!(total, 6);
    }

    /// span 与 mark 交叠处：**两个属性都要保住** ——
    /// 命中处的前景色该是 span 给的，背景色该是高亮给的。
    /// 归并时若只取其中一个来源，会出现"高亮把字色冲掉"或反之。
    #[test]
    fn overlapping_span_and_mark_keep_both_attributes() {
        let rt = RichText::new("abcdef")
            .span(0..6, Tone::Muted)
            .mark(2..4);
        let runs = rt.runs();
        let hit: Vec<_> = runs.iter().filter(|(_, _, m)| *m).collect();
        assert_eq!(hit.len(), 1);
        assert_eq!(hit[0].0, 2..4, "高亮段应精确等于 mark 区间");
        assert_eq!(hit[0].1, Some(Tone::Muted), "高亮段仍应带上 span 的色调");
    }

    /// span 与 mark 边界错开时（真实场景：按高亮搜索定位命中），
    /// 归并要切出正确的段数。
    #[test]
    fn staggered_boundaries_are_split_correctly() {
        // span 0..4 / 4..8，mark 2..6 → 边界 {0,2,4,6,8}
        let rt = RichText::from_spans(&[
            ("abcd".into(), Tone::Primary),
            ("efgh".into(), Tone::Muted),
        ])
        .mark(2..6);
        let runs = rt.runs();
        let marked_ranges: Vec<_> = runs
            .iter()
            .filter(|(_, _, m)| *m)
            .map(|(r, _, _)| r.clone())
            .collect();
        // 2..4 与 4..6 两段（4 是 span 边界，必须切开）
        assert_eq!(marked_ranges, vec![2..4, 4..6]);
        // 两段各自保留自己那半的色调
        let tones: Vec<_> = runs
            .iter()
            .filter(|(_, _, m)| *m)
            .map(|(_, t, _)| *t)
            .collect();
        assert_eq!(tones, vec![Some(Tone::Primary), Some(Tone::Muted)]);
    }

    /// 同位置多个 span：**后加者生效**（与 `span()` 的文档一致）。
    #[test]
    fn later_span_wins_at_the_same_range() {
        let rt = RichText::new("abc")
            .span(0..3, Tone::Error)
            .span(0..3, Tone::Success);
        let runs = rt.runs();
        assert_eq!(runs[0].1, Some(Tone::Success));
    }

    #[test]
    fn plain_text_has_a_single_untinted_run() {
        let runs = RichText::new("abc").runs();
        assert_eq!(runs, vec![(0..3, None, false)]);
    }

    /// 空文本不该产生任何段（也不该 panic）。
    #[test]
    fn empty_text_yields_no_runs() {
        assert!(RichText::new("").runs().is_empty());
    }

    /// 中文长文本的归并必须落在字节边界上 ——
    /// 与 `valid()` 的检查配合，这是"高亮不会画到半个字上"的保证。
    #[test]
    fn multibyte_runs_stay_on_char_boundaries() {
        let text = "先看依赖方向，再确认缩进";
        let rt = RichText::new(text).mark(6..18); // 落在汉字之间
        let runs = rt.runs();
        for (r, _, _) in &runs {
            assert!(text.is_char_boundary(r.start));
            assert!(text.is_char_boundary(r.end));
        }
        // 命中段切出来的文字应当还是完整的汉字
        let marked: String = runs
            .iter()
            .filter(|(_, _, m)| *m)
            .map(|(r, _, _)| &text[r.clone()])
            .collect();
        assert_eq!(marked, &text[6..18]);
    }

    #[test]
    fn a_bad_range_among_good_ones_keeps_the_good_ones() {
        // 一批命中里混进一个坏区间：好的仍要生效，坏的只计数
        let rt = RichText::new("abc").marks(&[0..1, 0..99, 1..2]);
        assert_eq!(rt.mark_count(), 2);
        assert_eq!(rt.invalid_ranges(), 1);
    }
}
