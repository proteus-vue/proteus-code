//! Markdown → 带色调的行
//!
//! # 范围（有意不做完整 Markdown）
//!
//! 终端里渲染 Markdown 的价值排序很清楚：**代码块 > 行内代码 > 标题 >
//! 列表 > 强调**。表格、脚注、HTML、嵌套列表这些在窄终端里本来就难读，
//! 做了反而占地方。所以这里只做前五类。
//!
//! # 为什么行内标记要"剥掉"而不是只改色
//!
//! `**重点**` 渲染成带色的 `**重点**` 仍然很难读 —— 星号本身就是噪声。
//! 这里剥掉标记只留文字（终端没有真正的粗体，加粗只能靠颜色层次表达）。
//! 代价是**原文不再逐字可见**，所以行内代码里的反引号内容一律原样保留。

use crate::syntax::{self, Lang};
use crate::Tone;

/// 一行渲染结果：(缩进, 文本, 色调)。
pub type Line = Vec<(usize, String, Tone)>;

/// 渲染一段 Markdown 为若干行。
///
/// `width` 用于换行预算（含缩进）。
pub fn render(text: &str, width: usize) -> Vec<Line> {
    let mut out: Vec<Line> = Vec::new();
    let mut in_code = false;
    let mut lang = Lang::Plain;

    for raw in text.lines() {
        let line = raw.trim_end();

        // ── 代码围栏 ──
        if let Some(rest) = line.trim_start().strip_prefix("```") {
            if in_code {
                in_code = false;
                continue;
            }
            in_code = true;
            lang = Lang::from_fence(rest);
            continue;
        }
        if in_code {
            // 代码块：先整体高亮，再**整行排版**。
            // 不能逐 span 换行 —— 那会让每个 span 都从同一列开始，互相覆盖，
            // 代码在终端里会变成残缺的碎片。
            let spans = syntax::highlight_line(line, lang);
            out.extend(layout_styled(&spans, 2, width.saturating_sub(4)));
            continue;
        }

        if line.is_empty() {
            out.push(Vec::new());
            continue;
        }

        // ── 标题：去掉 # 号，用强调色 ──
        let trimmed = line.trim_start();
        if let Some(hashes) = trimmed.split_whitespace().next() {
            if !hashes.is_empty() && hashes.chars().all(|c| c == '#') {
                let title = trimmed[hashes.len()..].trim();
                let tone = if hashes.len() <= 2 { Tone::Accent } else { Tone::Info };
                for w in crate::width::wrap_to_width(title, width) {
                    out.push(vec![(0, w, tone)]);
                }
                continue;
            }
        }

        // ── 引用：左侧竖条 + 压暗 ──
        if let Some(q) = trimmed.strip_prefix('>') {
            let body = q.trim_start();
            for w in crate::width::wrap_to_width(body, width.saturating_sub(2)) {
                out.push(vec![(0, "▏".to_string(), Tone::Border), (2, w, Tone::Muted)]);
            }
            continue;
        }

        // ── 无序列表：`- ` / `* ` / `+ ` → 圆点 ──
        if let Some(rest) = strip_bullet(trimmed) {
            let indent = leading_spaces(line);
            for (j, w) in crate::width::wrap_to_width(rest, width.saturating_sub(indent + 2))
                .into_iter()
                .enumerate()
            {
                if j == 0 {
                    let mut seg = vec![(indent, "• ".to_string(), Tone::Primary)];
                    seg.extend(inline(&w, indent + 2));
                    out.push(seg);
                } else {
                    out.push(vec![(indent, "  ".to_string(), Tone::Text), (indent + 2, w, Tone::Text)]);
                }
            }
            continue;
        }

        // ── 有序列表：保持原编号 ──
        if let Some((num, rest)) = strip_ordered(trimmed) {
            let indent = leading_spaces(line);
            let label = format!("{num}. ");
            let mut first = true;
            for w in crate::width::wrap_to_width(&rest, width.saturating_sub(indent + label.len())) {
                if first {
                    let mut seg = vec![(indent, label.clone(), Tone::Primary)];
                    seg.extend(inline(&w, indent + label.len()));
                    out.push(seg);
                    first = false;
                } else {
                    out.push(vec![(indent + label.len(), w, Tone::Text)]);
                }
            }
            continue;
        }

        // ── 水平线 ──
        if trimmed.chars().all(|c| c == '-' || c == '*' || c == '_') && trimmed.len() >= 3 {
            out.push(vec![(0, "─".repeat(width.min(40)), Tone::Border)]);
            continue;
        }

        // ── 普通段落 ──
        for w in crate::width::wrap_to_width(trimmed, width) {
            out.push(inline(&w, 0));
        }
    }
    out
}

/// 行内标记：`` `code` `` / `**bold**` / `*em*`。
///
/// 返回带绝对列号的片段（调用方给定起始缩进）。
///
/// **全程按 char 索引**：曾用 `String::find` 取偏移后当字符数用，
/// 遇到中文（多字节）就会跳过后续文本 —— `**重点** 与 *次要*` 里的
/// "与" 整段消失。字面量在下标运算里必须统一口径。
fn inline(s: &str, base: usize) -> Line {
    let chars: Vec<char> = s.chars().collect();
    let mut spans: Vec<(String, Tone)> = Vec::new();
    let mut buf = String::new();
    let mut i = 0;

    // 把 buf 以指定色调推出（同色调相邻合并）
    fn flush(spans: &mut Vec<(String, Tone)>, buf: &mut String, tone: Tone) {
        if buf.is_empty() {
            return;
        }
        if let Some(last) = spans.last_mut() {
            if last.1 == tone {
                last.0.push_str(buf);
                buf.clear();
                return;
            }
        }
        spans.push((std::mem::take(buf), tone));
    }

    while i < chars.len() {
        // 行内代码：内容原样保留（不解释其中的 * 与 _）
        if chars[i] == '`' {
            if let Some(end) = chars[i + 1..].iter().position(|c| *c == '`') {
                flush(&mut spans, &mut buf, Tone::Text);
                let code: String = chars[i + 1..i + 1 + end].iter().collect();
                spans.push((code, Tone::Success));
                i += end + 2;
                continue;
            }
        }
        // 粗体 / 强调：剥掉标记，只留文字
        if chars[i] == '*' {
            let is_bold = chars.get(i + 1) == Some(&'*');
            let mlen = if is_bold { 2 } else { 1 };
            // 在 **字符** 序列里找配对的标记，偏移天然是字符数
            let find_from = i + mlen;
            let close = chars[find_from..]
                .windows(mlen)
                .position(|w| w.iter().all(|c| *c == '*'));
            if let Some(rel) = close {
                flush(&mut spans, &mut buf, Tone::Text);
                let inner: String = chars[find_from..find_from + rel].iter().collect();
                let tone = if is_bold { Tone::Primary } else { Tone::Muted };
                for (c, t) in inline_simple(&inner, tone) {
                    spans.push((c, t));
                }
                i = find_from + rel + mlen;
                continue;
            }
        }
        buf.push(chars[i]);
        i += 1;
    }
    flush(&mut spans, &mut buf, Tone::Text);
    if spans.is_empty() {
        return vec![(base, String::new(), Tone::Text)];
    }
    // 相对片段 → 绝对列号
    let mut col = base;
    let mut line: Line = Vec::new();
    for (text, tone) in spans {
        let w = crate::width::display_width(&text);
        line.push((col, text, tone));
        col += w;
    }
    line
}

/// 按显示宽度把带色调的片段排成多行（**字符级**换行，适合代码）。
///
/// 与 `wrap_to_width` 的区别：那个按词换行（适合散文），
/// 这个按宽度硬换（适合代码/路径/长标识符）且保留逐段色调。
fn layout_styled(spans: &[(String, Tone)], indent: usize, width: usize) -> Vec<Line> {
    let width = width.max(1);
    let mut out: Vec<Line> = Vec::new();
    let mut line: Line = Vec::new();
    let mut col = indent;
    for (text, tone) in spans {
        for ch in text.chars() {
            let cw = crate::width::char_width(ch);
            if col + cw > indent + width {
                out.push(std::mem::take(&mut line));
                col = indent;
            }
            // 同色调相邻合并，减少转义序列
            if let Some(last) = line.last_mut() {
                if last.2 == *tone {
                    last.1.push(ch);
                    col += cw;
                    continue;
                }
            }
            line.push((col, ch.to_string(), *tone));
            col += cw;
        }
    }
    out.push(line);
    out
}

/// 强调内部的极小渲染：只识别 `` `code` ``，其余整体用给定色调。
fn inline_simple(s: &str, tone: Tone) -> Vec<(String, Tone)> {
    let mut out = Vec::new();
    let mut rest = s;
    loop {
        let Some(a) = rest.find('`') else {
            if !rest.is_empty() {
                out.push((rest.to_string(), tone));
            }
            break;
        };
        let Some(b) = rest[a + 1..].find('`') else {
            if !rest.is_empty() {
                out.push((rest.to_string(), tone));
            }
            break;
        };
        if a > 0 {
            out.push((rest[..a].to_string(), tone));
        }
        out.push((rest[a + 1..a + 1 + b].to_string(), Tone::Success));
        rest = &rest[a + 1 + b + 1..];
    }
    out
}

fn strip_bullet(s: &str) -> Option<&str> {
    for m in ["- ", "* ", "+ "] {
        if let Some(rest) = s.strip_prefix(m) {
            return Some(rest);
        }
    }
    None
}

fn strip_ordered(s: &str) -> Option<(String, String)> {
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    // 防御：超长数字串不是列表（避免把一大串数字误当编号）
    if digits.len() > 3 {
        return None;
    }
    let rest = s[digits.len()..].strip_prefix(". ")?;
    Some((digits, rest.to_string()))
}

fn leading_spaces(s: &str) -> usize {
    s.chars().take_while(|c| *c == ' ').count()
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
        let tones: Vec<Tone> = ls.iter().flatten().map(|(_, _, t)| *t).collect();
        assert!(tones.contains(&Tone::Accent), "应有关键字色：{tones:?}");
        assert!(tones.contains(&Tone::Success), "应有字符串色：{tones:?}");
        assert!(tones.contains(&Tone::Muted), "应有注释色：{tones:?}");
    }

    #[test]
    fn code_fence_without_language_is_not_highlighted() {
        let ls = render("```\nlet x = 1;\n```", 60);
        let tones: Vec<Tone> = ls.iter().flatten().map(|(_, _, t)| *t).collect();
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
        let tones: Vec<Tone> = ls.iter().flatten().map(|(_, _, t)| *t).collect();
        assert!(tones.contains(&Tone::Border) && tones.contains(&Tone::Muted), "{tones:?}");
    }

    #[test]
    fn horizontal_rule_is_rendered_as_a_line() {
        let ls = render("---", 40);
        assert!(text_of(&ls).contains('─'), "{:?}", text_of(&ls));
    }

    #[test]
    fn a_long_digit_run_is_not_treated_as_a_list() {
        // 防御：`1234567. x` 不是有序列表（避免误判噪声）
        let ls = render("1234567. 不是列表", 60);
        assert!(text_of(&ls).contains("1234567. 不是列表"), "不该被拆成编号");
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
}
