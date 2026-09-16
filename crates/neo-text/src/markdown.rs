//! Markdown → 带色调的行
//!
//! # 解析与渲染分开：解析交给成熟 crate，渲染决策留在本模块
//!
//! 早期版本手写解析（431 行）。它能跑，但**手写的永远只是 CommonMark 的一个子集**：
//! 嵌套列表、`_强调_`、链接/自动链接、软换行、转义、setext 标题这些都要自己补，
//! 而行内解析还踩过"按字节偏移当字符下标"的多字节坑（`**重点** 与 *次要*` 里的
//! "与"整段消失）。
//!
//! 现在解析交给 `pulldown-cmark`（MIT，`default-features=false` 下只有
//! `bitflags` / `memchr` / `unicase` 三个小依赖，其中 `memchr` 本就在依赖树里）。
//! **但渲染决策仍在本模块** —— 这是有意的产品取舍，不是库能替我们定的：
//!
//! - **剥掉行内标记而不是只改色**：`**重点**` 渲染成带色的 `**重点**` 仍然难读，
//!   星号本身就是噪声。终端没有真正的粗体，加粗只能靠颜色层次表达。
//!   代价是原文不再逐字可见，所以**行内代码的内容一律原样保留**。
//! - **只做五类**：代码块 > 行内代码 > 标题 > 列表 > 强调。表格、脚注、HTML
//!   在窄终端里本来就难读，做了反而占地方。因此用 `Parser::new`（即
//!   `Options::empty()`）——**刻意不打开** tables/footnotes/GFM 那些扩展位。
//! - **有序列表的超长编号按普通文字渲染**：CommonMark 允许 1–9 位编号，
//!   于是 `1234567. 不是列表` 会被解析成列表项。渲染时对 >999 的编号
//!   用普通色调（而非列表强调色）—— 内容与编号都保留，只是不当作列表强调。

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Parser, Tag, TagEnd};
use crate::syntax::{self, Lang};
use crate::Tone;

/// 一行渲染结果：(绝对列号, 文本, 色调)。
pub type Line = Vec<(usize, String, Tone)>;

/// 渲染一段 Markdown 为若干行。`width` 用于换行预算（含前缀缩进）。
pub fn render(text: &str, width: usize) -> Vec<Line> {
    let mut r = Renderer {
        width: width.max(1),
        out: Vec::new(),
        cur: Vec::new(),
        quote: 0,
        list_stack: Vec::new(),
        tones: Vec::new(),
        item_started: false,
        heading: None,
        in_code: false,
        code_lang: Lang::Plain,
        code: String::new(),
    };
    for ev in Parser::new(text) {
        r.event(ev);
    }
    r.flush();
    r.out
}

/// 解析成**逻辑行**（不折行、不带列号）：`Vec<Vec<(文本, 色调)>>`。
///
/// # 为什么需要它（与 [`render`] 的分工）
///
/// `render` 面向**终端**：终端没有文本布局引擎，换行必须由宿主自己按显示宽度
/// 算出来，所以它返回带列号的格子内容。
///
/// GUI 宿主相反 —— egui 有自己的文本布局（字体度量、按像素折行、可选择可滚动）。
/// 若把终端折好的行再喂给它，会**折两遍**：终端已按等宽列断一次，egui 又按
/// 实际字体宽度断一次，结果是断点错位、缩进错乱。
///
/// 所以这里只做"解析 + 语义分层"（前缀缩进、项目符号、色调），**不做折行**，
/// 把换行交给宿主的布局引擎。两者共用同一个解析器，因此**内容与色调必然一致**
/// （有测试钉住），差别只在谁来断行。
pub fn blocks(text: &str) -> Vec<Vec<(String, Tone)>> {
    // 预算给足 = 永不触发折行；用 MAX/4 是为了让内部 `col += 宽度` 那些
    // 累加有充裕余量，不必担心溢出（`saturating_*` 之外的普通加法）。
    let never_wraps = usize::MAX / 4;
    render(text, never_wraps)
        .into_iter()
        // 丢掉列号：布局由宿主做，列号只对终端的格子模型有意义
        .map(|line| line.into_iter().map(|(_, text, tone)| (text, tone)).collect())
        .collect()
}

struct Renderer {
    width: usize,
    out: Vec<Line>,
    /// 当前逻辑行的内联片段（相对，列号在 flush 时计算）。
    cur: Vec<(String, Tone)>,
    /// 引用深度（每层占 2 列：竖条 + 一个空格）。
    quote: usize,
    /// 列表嵌套栈：`None` = 无序，`Some(n)` = 有序的下一个编号。
    list_stack: Vec<Option<u64>>,
    /// 内联色调栈（强调/加粗可嵌套）。
    tones: Vec<Tone>,
    /// 当前列表项的**首行是否已输出过前缀**。
    ///
    /// 用来区分"新的一项"与"同一项内的续行"：列表符号属于**项**，不属于源码里
    /// 的每一行。软换行会反复收口同一项内的多行，若不加这个标记，一项就被画成
    /// 多个项目符号，有序列表还会**逐行递增编号**
    /// （`- a\n  b` → "• a / • b"；`1. a\n   b` → "1. a / 2. b"）。
    item_started: bool,
    /// 当前标题的色调（在标题内时非 None）。
    heading: Option<Tone>,
    in_code: bool,
    code_lang: Lang,
    /// 代码块正文（围栏内逐行累积，收口时统一高亮）。
    code: String,
}

impl Renderer {
    /// 当前内联文字的色调（栈顶，无则正文色）。
    fn tone(&self) -> Tone {
        self.heading
            .or_else(|| self.tones.last().copied())
            .unwrap_or(Tone::Text)
    }

    fn event(&mut self, ev: Event<'_>) {
        match ev {
            // ── 块级 ──
            Event::Start(Tag::Heading { level, .. }) => {
                self.heading = Some(match level {
                    HeadingLevel::H1 | HeadingLevel::H2 => Tone::Accent,
                    _ => Tone::Info,
                });
            }
            Event::End(TagEnd::Heading(_)) => {
                self.heading = None;
                self.flush();
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                self.in_code = true;
                self.code.clear();
                self.code_lang = match &kind {
                    CodeBlockKind::Fenced(info) => {
                        // 围栏信息串可能带额外属性（```rust ignore），取第一个词
                        let first = info.split_whitespace().next().unwrap_or("");
                        Lang::from_fence(first)
                    }
                    CodeBlockKind::Indented => Lang::Plain,
                };
            }
            Event::End(TagEnd::CodeBlock) => {
                self.in_code = false;
                // 代码块：先整体高亮，再**整行排版**。
                // 不能逐 span 换行 —— 那会让每个 span 都从同一列开始、互相覆盖，
                // 代码在终端里会变成残缺的碎片。
                let body = std::mem::take(&mut self.code);
                let indent = self.quote * 2;
                let budget = self.width.saturating_sub(indent + 2);
                for line in body.lines() {
                    let spans = syntax::highlight_line(line, self.code_lang);
                    if spans.is_empty() {
                        self.out.push(self.prefixed_line(indent, Vec::new()));
                        continue;
                    }
                    for seg in layout_styled(&spans, budget) {
                        self.out.push(self.prefixed_line(indent + 2, seg));
                    }
                }
            }
            Event::Start(Tag::List(start)) => {
                // 紧凑列表（tight list）的项内不含 Paragraph 事件，所以父项的
                // 文字会滞留在 cur 里；嵌套列表开始时必须先收口，否则
                // "外层内层"会被拼成同一行。
                self.flush();
                self.list_stack.push(start);
            }
            Event::End(TagEnd::List(_)) => {
                self.flush();
                self.list_stack.pop();
            }
            // 新的一项：前缀归零，下次 flush 重新出符号
            Event::Start(Tag::Item) => self.item_started = false,
            Event::End(TagEnd::Item) => self.flush(),
            Event::End(TagEnd::Paragraph) => self.flush(),
            Event::Start(Tag::BlockQuote(_)) => {
                self.quote += 1;
                // 引用正文压暗（与"▏"竖条形成层次）
                self.tones.push(Tone::Muted);
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                self.flush();
                self.tones.pop();
                self.quote = self.quote.saturating_sub(1);
            }
            Event::Rule => {
                self.flush();
                let indent = self.quote * 2;
                let bar = "─".repeat(self.width.saturating_sub(indent).min(40));
                self.out.push(self.prefixed_line(
                    indent,
                    vec![(bar, Tone::Border)],
                ));
            }

            // ── 行内 ──
            Event::Start(Tag::Emphasis) => self.tones.push(Tone::Muted),
            Event::End(TagEnd::Emphasis) => {
                self.tones.pop();
            }
            Event::Start(Tag::Strong) => self.tones.push(Tone::Primary),
            Event::End(TagEnd::Strong) => {
                self.tones.pop();
            }
            Event::Start(Tag::Strikethrough) => self.tones.push(Tone::Muted),
            Event::End(TagEnd::Strikethrough) => {
                self.tones.pop();
            }
            // 链接/图片：文字照常显示（终端里点不了），色调与正文一致，
            // 以免噪声；URL 不额外渲染 —— 它通常又长又没信息量。
            Event::Start(Tag::Link { .. }) | Event::Start(Tag::Image { .. }) => {}
            Event::End(TagEnd::Link) | Event::End(TagEnd::Image) => {}
            Event::Code(code) => {
                // 行内代码：内容**原样**保留（其中的 * 与 _ 不解释），独立色调
                self.cur.push((code.into_string(), Tone::Success));
            }
            Event::Text(t) => {
                if self.in_code {
                    self.code.push_str(&t);
                } else {
                    let tone = self.tone();
                    push_text(&mut self.cur, &t, tone);
                }
            }
            Event::SoftBreak => {
                // ⚠️ **有意偏离 CommonMark**：规范说软换行渲染成空格，但这里按换行处理。
                //
                // 依据是本项目的场景不是"读文章"而是"看 Agent 的回复"：模型输出的
                // 单个换行通常是有意义的（代码片段、错误输出、条目化的内容），
                // 把它们折叠成空格会改变原文形态 —— `a\nb\nc` 会显示成 "a b c"，
                // 用户看到的东西和模型写的不是一个样子。
                //
                // 代价：一段在源码里被硬折行的长句会显示成多行（换行处通常
                // 会重新排版，观感尚可）。两害相权，**保原文**优先。
                // 这条取舍由 `transcript_line_count_is_nonzero_and_bounded` 钉住。
                self.flush();
            }
            Event::HardBreak => self.flush(),
            // HTML 与其它未启用的事件：原样作为文字，**不丢内容**
            Event::Html(t) | Event::InlineHtml(t) => {
                let tone = self.tone();
                push_text(&mut self.cur, &t, tone);
            }
            Event::FootnoteReference(t) => {
                self.cur.push((format!("[{t}]"), Tone::Muted));
            }
            Event::TaskListMarker(done) => {
                self.cur
                    .push((if done { "[x] ".into() } else { "[ ] ".into() }, Tone::Muted));
            }
            Event::InlineMath(t) | Event::DisplayMath(t) => {
                self.cur.push((t.into_string(), Tone::Success));
            }
            Event::End(_) | Event::Start(_) => {}
        }
    }

    /// 当前行的结构前缀（引用条 + 列表缩进 + 项目符号）。
    ///
    /// 返回 (起始列, 片段列表, 正文起始列)。列表项每行都要带符号（换行时
    /// 续行用等宽空格对齐在符号之下 —— 与旧实现一致）。
    fn prefix_parts(&self) -> (usize, Vec<(String, Tone)>, usize) {
        let mut col = 0usize;
        let mut parts: Vec<(String, Tone)> = Vec::new();
        // 引用：每层占 2 列（竖条 + 空格）
        for _ in 0..self.quote {
            parts.push(("▏".to_string(), Tone::Border));
            col += 2;
        }
        if self.cur.is_empty() {
            return (col, parts, col);
        }
        // 列表缩进：每层 2 列
        let depth = self.list_stack.len();
        if depth > 0 {
            let indent = (depth - 1) * 2;
            if indent > 0 {
                parts.push((" ".repeat(indent), Tone::Text));
                col += indent;
            }
            // 同一项内的续行：只缩进对齐到正文，**不再出符号**（符号属于项，
            // 不属于源码的每一行）。缩进宽度 = 该项符号的显示宽度，
            // 于是续行与首行正文左对齐。
            let marker_w = match self.list_stack.last().copied().flatten() {
                None => 2, // "• "
                Some(n) => crate::width::display_width(&format!("{n}. ")),
            };
            if self.item_started {
                parts.push((" ".repeat(marker_w), Tone::Text));
                col += marker_w;
                return (col, parts, col);
            }
            match self.list_stack.last().copied().flatten() {
                // 无序
                None => {
                    parts.push(("• ".to_string(), Tone::Primary));
                    col += 2;
                }
                // 有序：编号超长（>999）按普通文字，不当列表强调
                Some(n) => {
                    let label = format!("{n}. ");
                    let tone = if n > 999 { Tone::Text } else { Tone::Primary };
                    col += crate::width::display_width(&label);
                    parts.push((label, tone));
                }
            }
        }
        (col, parts, col)
    }

    /// 用给定缩进收口当前行（内部已包含前缀处理）。
    fn prefixed_line(&self, indent: usize, spans: Vec<(String, Tone)>) -> Line {
        let mut line: Line = Vec::new();
        let mut col = 0usize;
        if indent > 0 {
            line.push((0, " ".repeat(indent), Tone::Text));
            col = indent;
        }
        for (text, tone) in spans {
            if text.is_empty() {
                continue;
            }
            line.push((col, text.clone(), tone));
            col += crate::width::display_width(&text);
        }
        if line.is_empty() {
            line.push((0, String::new(), Tone::Text));
        }
        line
    }

    /// 收口当前逻辑行：加前缀、折行、带色调输出。
    fn flush(&mut self) {
        if self.cur.is_empty() {
            return;
        }
        // ⚠️ 顺序要紧：`prefix_parts` 依赖 `cur` 非空来判断"这是不是列表项"，
        // 所以必须在取走 cur **之前**调用（先 take 再算会永远拿不到列表前缀）。
        let in_item = !self.list_stack.is_empty();
        let first_line_of_item = !self.item_started;
        let (_, parts, body_col) = self.prefix_parts();
        let spans = std::mem::take(&mut self.cur);
        if in_item {
            // 标记"本项已出过符号"：后续同项内的续行只缩进、不再出符号。
            self.item_started = true;
            // 编号只在**本项首行**递增 —— 逐行递增会把一项拆成多个编号。
            if first_line_of_item {
                if let Some(Some(n)) = self.list_stack.last_mut() {
                    *n += 1;
                }
            }
        }

        let budget = self.width.saturating_sub(body_col);
        let lines = wrap_spans(&spans, budget);
        let total = lines.len();

        for (i, segments) in lines.into_iter().enumerate() {
            let mut line: Line = Vec::new();
            let mut col = 0usize;
            if i == 0 {
                for (text, tone) in &parts {
                    line.push((col, text.clone(), *tone));
                    col += crate::width::display_width(text);
                }
            } else {
                // 续行：与首行正文对齐（前缀宽度用空格填）
                let padding: usize = parts
                    .iter()
                    .map(|(t, _)| crate::width::display_width(t))
                    .sum();
                if padding > 0 {
                    line.push((0, " ".repeat(padding), Tone::Text));
                    col = padding;
                }
            }
            for (text, tone) in segments {
                if text.is_empty() {
                    continue;
                }
                line.push((col, text.clone(), tone));
                col += crate::width::display_width(&text);
            }
            if line.is_empty() {
                line.push((0, String::new(), Tone::Text));
            }
            let _ = total;
            self.out.push(line);
        }
    }
}

/// 把文字追加到片段序列（同色调相邻合并，减少转义序列）。
fn push_text(spans: &mut Vec<(String, Tone)>, text: &str, tone: Tone) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = spans.last_mut() {
        if last.1 == tone {
            last.0.push_str(text);
            return;
        }
    }
    spans.push((text.to_string(), tone));
}

/// 把一个逻辑行按宽度折行，**保留每段文字的色调**。
///
/// 为什么不直接调 `width::wrap_to_width` 再"切回来"：那个函数按空格分词、
/// 用单个空格重新拼接，于是**连续空格被归一化、首尾空格被丢掉** ——
/// 折行前后的字符数不再一一对应，没法把色调映射回去。
///
/// 这里按字符走：优先在最后一个空格处断行（该空格丢弃），词内没有空格
/// （CJK、长标识符、URL）就按显示宽度硬断。与 `hard_split` 同策略。
fn wrap_spans(spans: &[(String, Tone)], budget: usize) -> Vec<Vec<(String, Tone)>> {
    let budget = budget.max(1);
    // 展平成 (字符, 色调)，折行时只需处理单一序列
    let mut flat: Vec<(char, Tone)> = Vec::new();
    for (text, tone) in spans {
        for ch in text.chars() {
            flat.push((ch, *tone));
        }
    }

    let mut lines: Vec<Vec<(char, Tone)>> = Vec::new();
    let mut cur: Vec<(char, Tone)> = Vec::new();
    let mut used = 0usize;
    let mut last_space: Option<usize> = None;

    for (ch, tone) in flat {
        let cw = crate::width::char_width(ch);
        if used + cw > budget && !cur.is_empty() {
            if let Some(si) = last_space {
                let mut rest = cur.split_off(si);
                // 断点处的空格本身不保留（行尾空白没意义）
                if !rest.is_empty() {
                    rest.remove(0);
                }
                lines.push(std::mem::take(&mut cur));
                cur = rest;
            } else {
                lines.push(std::mem::take(&mut cur));
            }
            used = cur
                .iter()
                .map(|(c, _)| crate::width::char_width(*c))
                .sum();
            last_space = None;
        }
        // 行首空格丢弃；行中空格记为潜在断点
        if ch == ' ' {
            if cur.is_empty() {
                continue;
            }
            last_space = Some(cur.len());
        }
        cur.push((ch, tone));
        used += cw;
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.is_empty() {
        lines.push(Vec::new());
    }

    // 合并同色调相邻字符为片段
    lines
        .into_iter()
        .map(|chars| {
            let mut segs: Vec<(String, Tone)> = Vec::new();
            for (ch, tone) in chars {
                if let Some(last) = segs.last_mut() {
                    if last.1 == tone {
                        last.0.push(ch);
                        continue;
                    }
                }
                segs.push((ch.to_string(), tone));
            }
            segs
        })
        .collect()
}

/// 把高亮片段按宽度硬折（**字符级**，适合代码：路径、长标识符不会在
/// "单词"中间被当作可断点而错位），保留逐段色调。
fn layout_styled(spans: &[(String, Tone)], width: usize) -> Vec<Vec<(String, Tone)>> {
    let width = width.max(1);
    let mut out: Vec<Vec<(String, Tone)>> = Vec::new();
    let mut line: Vec<(String, Tone)> = Vec::new();
    let mut used = 0usize;
    for (text, tone) in spans {
        for ch in text.chars() {
            let cw = crate::width::char_width(ch);
            if used + cw > width && !line.is_empty() {
                out.push(std::mem::take(&mut line));
                used = 0;
            }
            if let Some(last) = line.last_mut() {
                if last.1 == *tone {
                    last.0.push(ch);
                    used += cw;
                    continue;
                }
            }
            line.push((ch.to_string(), *tone));
            used += cw;
        }
    }
    if !line.is_empty() || out.is_empty() {
        out.push(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(lines: &[Line]) -> String {
        lines
            .iter()
            .map(|l| l.iter().map(|(_, t, _)| t.as_str()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn tones_of(lines: &[Line]) -> Vec<Tone> {
        lines.iter().flatten().map(|(_, _, t)| *t).collect()
    }

    #[test]
    fn plain_paragraph_passes_through() {
        let ls = render("这是一段普通文字。", 40);
        assert_eq!(text_of(&ls), "这是一段普通文字。");
    }

    #[test]
    fn heading_loses_its_hashes_and_gets_emphasis() {
        // 星号/# 号本身是噪声：剥掉标记，用颜色表达层次
        let ls = render("## 标题", 40);
        assert_eq!(text_of(&ls), "标题");
        assert_eq!(ls[0][0].2, Tone::Accent, "标题应用强调色");
    }

    #[test]
    fn fenced_code_block_is_syntax_highlighted() {
        let md = "说明：\n```rust\nlet x = \"s\"; // c\n```\n后文";
        let ls = render(md, 60);
        let text = text_of(&ls);
        assert!(text.contains("let x = \"s\"; // c"), "代码内容应保留：{text}");
        // 关键字 / 字符串 / 注释都应有各自的色调
        let tones = tones_of(&ls);
        assert!(tones.contains(&Tone::Accent), "应有关键字色：{tones:?}");
        assert!(tones.contains(&Tone::Success), "应有字符串色：{tones:?}");
        assert!(tones.contains(&Tone::Muted), "应有注释色：{tones:?}");
    }

    #[test]
    fn code_fence_without_language_is_not_highlighted() {
        let ls = render("```\nlet x = 1;\n```", 60);
        let tones = tones_of(&ls);
        assert!(!tones.contains(&Tone::Accent), "未知语言不该高亮：{tones:?}");
    }

    #[test]
    fn inline_code_keeps_its_content_without_backticks() {
        let ls = render("运行 `cargo test` 即可", 60);
        let text = text_of(&ls);
        assert_eq!(text, "运行 cargo test 即可", "反引号应被剥掉：{text}");
        assert!(
            ls[0].iter().any(|(_, t, tone)| t == "cargo test" && *tone == Tone::Success),
            "行内代码应有独立色调：{:?}",
            ls[0]
        );
    }

    #[test]
    fn bold_and_emphasis_markers_are_removed() {
        let ls = render("这是 **重点** 与 *次要* 内容", 60);
        let text = text_of(&ls);
        assert!(!text.contains('*'), "强调标记应被剥掉：{text}");
        assert!(text.contains("重点") && text.contains("次要"), "{text}");
    }

    #[test]
    fn asterisks_inside_inline_code_are_preserved() {
        // 行内代码里 `a * b` 的星号是内容，不能被当强调标记吃掉
        let ls = render("计算 `a * b` 的值", 60);
        let text = text_of(&ls);
        assert!(text.contains("a * b"), "代码内的星号必须保留：{text}");
    }

    #[test]
    fn bullets_become_dots_and_keep_text() {
        let ls = render("- 第一项\n- 第二项", 40);
        let text = text_of(&ls);
        assert!(text.contains("• 第一项"), "{text}");
        assert!(text.contains("• 第二项"), "{text}");
        assert!(!text.contains("- 第一项"), "短横线应换成圆点：{text}");
    }

    /// 列表符号属于**项**，不属于源码里的每一行。
    ///
    /// 回归测试（软换行改动引入的真 bug）：`- a\n  b` 是**一个**列表项被源码
    /// 折行，`b` 是续行 —— 它不该再得到一个项目符号；有序列表更严重，
    /// 逐行递增编号会把一项画成两项（`1. a / 2. b`）。
    #[test]
    fn list_marker_belongs_to_the_item_not_to_every_source_line() {
        let text = text_of(&render("- 甲\n  乙", 40));
        assert_eq!(text.matches('•').count(), 1, "同一项只能有一个符号：{text}");
        assert!(text.contains("甲") && text.contains("乙"), "{text}");

        let text = text_of(&render("1. 甲\n   乙", 40));
        assert!(!text.contains("2."), "同一项不该递增编号：{text}");
        assert_eq!(text.matches("1.").count(), 1, "{text}");

        // 但**不同**的项各自要有符号与递进的编号
        let text = text_of(&render("- 甲\n- 乙", 40));
        assert_eq!(text.matches('•').count(), 2, "两项就要两个符号：{text}");
        let text = text_of(&render("1. 甲\n2. 乙", 40));
        assert!(text.contains("1. 甲") && text.contains("2. 乙"), "{text}");

        // 续行应与首行正文对齐（缩进 = 符号宽度）
        let ls = render("- 甲\n  乙", 40);
        let col_of = |needle: &str| {
            ls.iter()
                .find(|l| l.iter().any(|(_, t, _)| t.contains(needle)))
                .and_then(|l| l.iter().find(|(_, t, _)| t.contains(needle)).map(|(c, _, _)| *c))
                .unwrap_or(0)
        };
        assert_eq!(col_of("甲"), col_of("乙"), "续行应与首行正文左对齐：{ls:?}");
    }

    #[test]
    fn ordered_list_keeps_its_numbering() {
        let ls = render("1. 甲\n2. 乙", 40);
        let text = text_of(&ls);
        assert!(text.contains("1. 甲") && text.contains("2. 乙"), "{text}");
    }

    #[test]
    fn quote_gets_a_left_bar_and_muted_tone() {
        let ls = render("> 引用内容", 40);
        assert!(text_of(&ls).contains("引用内容"));
        let tones = tones_of(&ls);
        assert!(tones.contains(&Tone::Border) && tones.contains(&Tone::Muted), "{tones:?}");
    }

    #[test]
    fn horizontal_rule_is_rendered_as_a_line() {
        let ls = render("---", 40);
        assert!(text_of(&ls).contains('─'), "{:?}", text_of(&ls));
    }

    #[test]
    fn a_long_digit_run_is_not_treated_as_a_list() {
        // CommonMark 允许 1–9 位编号，所以 1234567. 会被解析成列表项；
        // 渲染时对超长编号用普通色调（内容与编号都保留，只是不当列表强调）
        let ls = render("1234567. 不是列表", 60);
        assert!(
            text_of(&ls).contains("1234567. 不是列表"),
            "内容与编号必须保留：{:?}",
            text_of(&ls)
        );
        assert!(
            !tones_of(&ls).contains(&Tone::Primary),
            "超长编号不该用列表强调色：{:?}",
            ls
        );
    }

    #[test]
    fn wraps_long_lines_to_the_width_budget() {
        let long = "字".repeat(100);
        let ls = render(&long, 20);
        assert!(ls.len() > 1, "超宽段落应换行");
        for l in &ls {
            let w: usize = l.iter().map(|(_, t, _)| crate::width::display_width(t)).sum();
            assert!(w <= 20, "行宽 {w} 超预算");
        }
    }

    #[test]
    fn unclosed_code_fence_does_not_lose_content() {
        // 模型输出的围栏可能不闭合；内容不能因此消失
        let ls = render("```rust\nlet x = 1;", 60);
        assert!(text_of(&ls).contains("let x = 1;"), "未闭合围栏也要显示内容");
    }

    #[test]
    fn every_column_is_non_negative_and_ordered() {
        // 列号用于网格放置；倒序或负数会画错位置
        let md = "## 标题\n- 项 `code` 与 **粗体**\n> 引\n```py\nx = 1\n```";
        for l in render(md, 50) {
            let mut prev = 0usize;
            for (col, text, _) in &l {
                assert!(*col >= prev, "列号必须单调不减：{l:?}");
                prev = *col;
                assert!(!text.is_empty() || l.len() == 1);
            }
        }
    }

    // ── 迁移到 pulldown-cmark 后**新支持**的情形 ──
    // 这些在手写解析下做不到，是本次替换的主要收益。

    #[test]
    fn nested_lists_are_rendered_with_increasing_indent() {
        let ls = render("- 外层\n  - 内层", 40);
        let text = text_of(&ls);
        assert!(text.contains("外层") && text.contains("内层"), "{text}");
        // 内层的正文列号必须大于外层（嵌套靠缩进体现）
        let outer = ls
            .iter()
            .find(|l| l.iter().any(|(_, t, _)| t.contains("外层")))
            .expect("应有外层行");
        let inner = ls
            .iter()
            .find(|l| l.iter().any(|(_, t, _)| t.contains("内层")))
            .expect("应有内层行");
        let col_of = |l: &Line, needle: &str| {
            l.iter()
                .find(|(_, t, _)| t.contains(needle))
                .map(|(c, _, _)| *c)
                .unwrap_or(0)
        };
        assert!(
            col_of(inner, "内层") > col_of(outer, "外层"),
            "内层应缩进更多：{ls:?}"
        );
    }

    #[test]
    fn underscore_emphasis_is_recognized_like_asterisk() {
        // 手写解析只认 `*`，`_强调_` 会原样带下划线显示
        let ls = render("这是 _强调_ 内容", 40);
        let text = text_of(&ls);
        assert!(!text.contains('_'), "下划线标记应被剥掉：{text}");
        assert!(text.contains("强调"), "{text}");
    }

    #[test]
    fn links_show_their_text_without_the_url_noise() {
        let ls = render("见 [文档](https://example.com/very/long/path) 说明", 60);
        let text = text_of(&ls);
        assert!(text.contains("文档"), "链接文字应显示：{text}");
        assert!(
            !text.contains("https://example.com"),
            "URL 不该挤进终端正文：{text}"
        );
    }

    #[test]
    fn soft_break_is_preserved_as_a_line_break() {
        // 有意偏离 CommonMark（那里软换行 = 空格）：Agent 输出的单个换行
        // 通常有意义，折叠成空格会让显示与原文不符。
        let ls = render("前半句\n后半句", 60);
        assert_eq!(text_of(&ls), "前半句\n后半句", "{:?}", text_of(&ls));
    }

    #[test]
    fn a_three_line_message_stays_three_lines() {
        // 与 render 同源的行数估算依赖这条（滚动上限、搜索行号都按它算）
        let ls = render("a\nb\nc", 80);
        assert_eq!(ls.len(), 3, "三行文本应渲染成三行：{ls:?}");
        assert_eq!(text_of(&ls), "a\nb\nc");
    }

    #[test]
    fn multibyte_text_is_not_dropped_by_inline_parsing() {
        // 历史 bug：按字节偏移当字符下标，导致多字节文本整段消失
        let ls = render("这是 **重点** 与 *次要* 内容", 60);
        let text = text_of(&ls);
        assert!(text.contains('与'), "多字节字符不能丢：{text}");
        assert!(text.contains("内容"), "{text}");
    }

    #[test]
    fn table_syntax_is_kept_as_plain_text_by_intent() {
        // 刻意不开 tables 扩展：窄终端里表格难读，原样输出比强行排版好
        let md = "| a | b |\n|---|---|\n| 1 | 2 |";
        let ls = render(md, 60);
        let text = text_of(&ls);
        assert!(text.contains("a") && text.contains("1"), "内容不能丢：{text}");
    }

    /// **行内标记跨越折行边界**时，标记必须已被剥掉、色调必须延续。
    ///
    /// 这是老实现的一个真 bug（迁移时实测复现）：老流程是"先按宽度折行、
    /// 再对每个折好的片段跑行内解析"。当 `**加粗**` 跨行时，两边的片段
    /// 各自都不含配对的 `**`，解析失败 → **字面星号留在屏幕上**：
    ///
    /// ```text
    /// 这是
    /// **一段很长的加粗文      ← 星号没被剥掉
    /// 字需要折行** 结束
    /// ```
    ///
    /// 新流程是"先解析拿色调、再带着色调折行"，从根上避免了这个错位。
    /// 这条也是本次替换**不以行数减少为收益**的实证：新实现多出的行
    /// 主要就是这类"先解析后折行"的协调成本，换来的是正确。
    #[test]
    fn inline_markup_spanning_a_wrap_boundary_loses_its_markers() {
        let ls = render("这是 **一段很长的加粗文字需要折行** 结束", 18);
        let text = text_of(&ls);
        assert!(!text.contains('*'), "跨行折行的加粗不该留下字面星号：\n{text}");
        assert!(text.contains("一段很长的加粗文字"), "内容不能丢：\n{text}");
        assert!(text.contains("结束"), "{text}");
        // 色调必须**跨越折行边界延续**（加粗部分整段都是 Primary）
        let bold_tones: Vec<Tone> = ls
            .iter()
            .flatten()
            .filter(|(_, t, _)| t.contains("加粗"))
            .map(|(_, _, tone)| *tone)
            .collect();
        assert!(
            !bold_tones.is_empty() && bold_tones.iter().all(|t| *t == Tone::Primary),
            "加粗文字的色调应在折行后延续：{bold_tones:?}"
        );
        // 每行仍在预算内
        for l in &ls {
            let w: usize = l.iter().map(|(_, t, _)| crate::width::display_width(t)).sum();
            assert!(w <= 18, "行宽 {w} 超预算：{l:?}");
        }
    }
    /// `blocks()` 与 `render()` 必须对**同一段 Markdown 给出相同的内容与色调**。
    ///
    /// 它们服务两个不同的宿主（GUI 自己布局 / 终端按列折行），但共用同一个
    /// 解析器。这条测试是"宿主等价"在这层的落点：如果两者内容漂了，
    /// 同一个模型回复在窗口与终端里就会显示成不同的东西。
    /// 允许的差别只有**断行位置**（终端折行、GUI 交给 egui）。
    #[test]
    fn blocks_and_render_agree_on_content_and_tones() {
        // 覆盖各类块：标题 / 粗体 / 行内代码 / 列表 / 引用 / 代码块 / 链接
        let md = "# 标题\n\n正文 **加粗** 与 `代码`\n\n- 甲\n- 乙\n\n> 引用\n\n```rust\nlet x = 1;\n```\n\n见 [文档](https://e.com)";

        // render：折行后的文字拼回去（去掉折行带来的换行差异，只比字符序列）
        let rendered: String = render(md, 200)
            .iter()
            .flat_map(|l| l.iter().map(|(_, t, _)| t.as_str()))
            .collect::<String>()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();

        let blocked: String = blocks(md)
            .iter()
            .flat_map(|l| l.iter().map(|(t, _)| t.as_str()))
            .collect::<String>()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();

        assert_eq!(rendered, blocked, "两者内容必须一致（只允许断行位置不同）");

        // 色调集合也必须一致 —— 层次表达是语义，不属于布局
        let tones_of = |ls: &[Vec<(String, Tone)>]| {
            let mut v: Vec<Tone> = ls.iter().flatten().map(|(_, t)| *t).collect();
            v.sort_by_key(|t| format!("{t:?}"));
            v.dedup();
            v
        };
        let render_tones = tones_of(
            &render(md, 200)
                .into_iter()
                .map(|l| l.into_iter().map(|(_, t, tone)| (t, tone)).collect())
                .collect::<Vec<_>>(),
        );
        let block_tones = tones_of(&blocks(md));
        assert_eq!(
            render_tones, block_tones,
            "两者的色调集合必须一致（同一份语义分层）"
        );
    }

    /// `blocks()` 不得折行：折行是宿主的责任。
    #[test]
    fn blocks_never_wraps_even_for_very_long_lines() {
        let long = "字".repeat(500);
        let ls = blocks(&long);
        assert_eq!(ls.len(), 1, "不应折行（GUI 用自己的布局引擎）：{ls:?}");
        let text: String = ls[0].iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(text.chars().count(), 500, "内容不能丢");
    }
}

