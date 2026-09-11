//! 显示宽度计算（UAX #11 East Asian Width 的实用子集）
//!
//! # 为什么不能按"字符个数"算
//!
//! 终端里一行能放多少**列**取决于字符的显示宽度：CJK、全角标点、多数 emoji
//! 占 **2 列**，ASCII 占 1 列。按字符个数换行会让中文行在视觉上超出一半，
//! 中文界面下换行位置全错——这是终端 UI 最常见的可见缺陷。
//!
//! # 为什么手写而不引 unicode-width
//!
//! 与本项目一贯立场一致：调试链要浅。这里覆盖的是**实用子集**：
//! 常见 CJK 区段 + 全角形式 + 主要 emoji 区段。这不是完整 UAX #11 实现，
//! 未覆盖的冷门区段按 1 列处理（诚实边界，见文件末）。
//!
//! # 组合字符
//!
//! 组合记号（U+0300–U+036F 等）宽度为 0：它们叠在前一个字符上，
//! 若算 1 列会让行看起来比实际短，光标位置与换行位置都会偏。

/// 单个字符占的列数：0（组合/零宽）、1（半角）、2（全角/宽）。
pub fn char_width(c: char) -> usize {
    let cp = c as u32;

    // 控制字符不该出现在正文里；若出现按 0 处理，避免撑破布局
    if cp < 0x20 || cp == 0x7f {
        return 0;
    }

    // 组合记号 / 零宽：叠在前一个字符上
    if is_zero_width(cp) {
        return 0;
    }

    if is_wide(cp) {
        2
    } else {
        1
    }
}

fn is_zero_width(cp: u32) -> bool {
    matches!(cp,
        0x0300..=0x036F   // Combining Diacritical Marks
        | 0x0483..=0x0489 // Cyrillic combining
        | 0x0591..=0x05BD // Hebrew points
        | 0x0610..=0x061A // Arabic marks
        | 0x064B..=0x065F // Arabic marks
        | 0x0E31 | 0x0E34..=0x0E3A // Thai marks
        | 0x1AB0..=0x1AFF // Combining Extended
        | 0x1DC0..=0x1DFF // Combining Supplement
        | 0x20D0..=0x20FF // Combining for Symbols
        | 0xFE00..=0xFE0F // Variation Selectors
        | 0xFE20..=0xFE2F // Combining Half Marks
        | 0x200B..=0x200F // Zero-width space/joiners, direction marks
        | 0x2060..=0x2064 // Word joiner, invisible operators
        | 0xFEFF          // BOM / zero-width no-break space
    )
}

fn is_wide(cp: u32) -> bool {
    matches!(cp,
        // ── 东亚宽字符（East Asian Wide / Fullwidth）──
        0x1100..=0x115F   // Hangul Jamo (initial)
        | 0x2E80..=0x2EFF // CJK Radicals Supplement
        | 0x2F00..=0x2FDF // Kangxi Radicals
        | 0x2FF0..=0x2FFF // Ideographic Description Characters
        | 0x3000..=0x303E // CJK Symbols and Punctuation（含全角空格 3000）
        | 0x3041..=0x309F // Hiragana
        | 0x30A0..=0x30FF // Katakana
        | 0x3100..=0x312F // Bopomofo
        | 0x3130..=0x318F // Hangul Compatibility Jamo
        | 0x3190..=0x319F // Kanbun
        | 0x31A0..=0x31BF // Bopomofo Extended
        | 0x31C0..=0x31EF // CJK Strokes
        | 0x3200..=0x32FF // Enclosed CJK Letters and Months
        | 0x3300..=0x33FF // CJK Compatibility
        | 0x3400..=0x4DBF // CJK Unified Ideographs Extension A
        | 0x4E00..=0x9FFF // CJK Unified Ideographs（最常用）
        | 0xA000..=0xA4CF // Yi Syllables / Radicals
        | 0xA960..=0xA97F // Hangul Jamo Extended-A
        | 0xAC00..=0xD7A3 // Hangul Syllables（韩文常用）
        | 0xF900..=0xFAFF // CJK Compatibility Ideographs
        | 0xFE10..=0xFE19 // Vertical Forms
        | 0xFE30..=0xFE6F // CJK Compatibility Forms + Small Form Variants
        | 0xFF00..=0xFF60 // Fullwidth Forms（全角 ASCII）
        | 0xFFE0..=0xFFE6 // Fullwidth signs
        // ── 宽 emoji ──
        | 0x1F300..=0x1F64F // Misc Symbols and Pictographs + Emoticons
        | 0x1F680..=0x1F6FF // Transport and Map
        | 0x1F900..=0x1F9FF // Supplemental Symbols and Pictographs
        | 0x1FA70..=0x1FAFF // Symbols and Pictographs Extended-A
        | 0x1F004 | 0x1F0CF | 0x1F18E | 0x1F191..=0x1F19A // 麻将/扑克等
        // ── CJK 扩展（星形平面）──
        | 0x20000..=0x2FFFD
        | 0x30000..=0x3FFFD
    )
}

/// 字符串的显示宽度（列数）。
pub fn display_width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

/// 按显示宽度截断，超出时追加省略号（省略号本身占 1 列）。
pub fn truncate_to_width(s: &str, max_cols: usize) -> String {
    if display_width(s) <= max_cols {
        return s.to_string();
    }
    // 为省略号留 1 列
    let budget = max_cols.saturating_sub(1);
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = char_width(c);
        if used + w > budget {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

/// 按显示宽度右侧补空格到指定列数（用于对齐）。
pub fn pad_to_width(s: &str, cols: usize) -> String {
    let w = display_width(s);
    if w >= cols {
        return s.to_string();
    }
    format!("{s}{}", " ".repeat(cols - w))
}

/// 按显示宽度换行。
///
/// 两遍法，比"边扫边记断点"清楚得多（后者会把空格宽度算进断点，本实现第一版
/// 就栽在这上面）：
///  1. **硬切超长词**：单个词比整行还宽时（长路径、长 URL、连续 CJK），
///     先按宽度切成块 —— 否则它会溢出。
///  2. **贪心合并**：把块用空格拼行，放不下就换行。
///
/// 于是英文优先在空格断（不从词中间切），而超长 token 也不会撑破布局。
pub fn wrap_to_width(s: &str, max_cols: usize) -> Vec<String> {
    if max_cols == 0 {
        return vec![s.to_string()];
    }

    // 第一遍：把每个词切成不超过 max_cols 的块
    let mut chunks: Vec<String> = Vec::new();
    for word in s.split(' ') {
        if word.is_empty() {
            continue;
        }
        chunks.extend(hard_split(word, max_cols));
    }
    if chunks.is_empty() {
        return vec![String::new()];
    }

    // 第二遍：贪心合并成行
    let mut lines = Vec::new();
    let mut cur = String::new();
    let mut used = 0usize;
    for chunk in chunks {
        let cw = display_width(&chunk);
        if cur.is_empty() {
            cur = chunk;
            used = cw;
            continue;
        }
        if used + 1 + cw <= max_cols {
            cur.push(' ');
            cur.push_str(&chunk);
            used += 1 + cw;
        } else {
            lines.push(std::mem::take(&mut cur));
            cur = chunk;
            used = cw;
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

/// 把一个词硬切成若干不超过 `max_cols` 的块。**不切开字符**。
fn hard_split(word: &str, max_cols: usize) -> Vec<String> {
    if display_width(word) <= max_cols {
        return vec![word.to_string()];
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut used = 0;
    for c in word.chars() {
        let w = char_width(c);
        if used + w > max_cols && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
            used = 0;
        }
        cur.push(c);
        used += w;
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_is_one_column() {
        assert_eq!(char_width('a'), 1);
        assert_eq!(char_width('Z'), 1);
        assert_eq!(char_width('1'), 1);
        assert_eq!(display_width("hello"), 5);
    }

    #[test]
    fn cjk_is_two_columns() {
        assert_eq!(char_width('中'), 2);
        assert_eq!(char_width('好'), 2);
        assert_eq!(char_width('あ'), 2);
        assert_eq!(char_width('한'), 2);
        assert_eq!(display_width("中文"), 4, "两个汉字应占 4 列");
        assert_eq!(display_width("你好世界"), 8);
    }

    #[test]
    fn fullwidth_forms_are_two_columns() {
        assert_eq!(char_width('，'), 2, "全角逗号");
        assert_eq!(char_width('。'), 2, "全角句号");
        assert_eq!(char_width('：'), 2);
        assert_eq!(char_width('\u{3000}'), 2, "全角空格");
        assert_eq!(char_width('Ａ'), 2, "全角字母 A");
    }

    #[test]
    fn combining_marks_are_zero_width() {
        // e + combining acute
        assert_eq!(char_width('\u{0301}'), 0);
        assert_eq!(display_width("e\u{0301}"), 1, "组合记号不占列");
        assert_eq!(char_width('\u{200B}'), 0, "零宽空格");
    }

    #[test]
    fn emoji_are_two_columns() {
        assert_eq!(char_width('🚀'), 2);
        assert_eq!(char_width('😀'), 2);
    }

    #[test]
    fn mixed_width_is_summed_correctly() {
        assert_eq!(display_width("a中b"), 1 + 2 + 1);
        assert_eq!(display_width("OK 完成"), 2 + 1 + 4);
    }

    #[test]
    fn wrap_uses_display_width_not_char_count() {
        // 4 列预算：两个汉字刚好占满
        assert_eq!(wrap_to_width("中文字", 4), vec!["中文", "字"]);
        // ASCII 则能放 4 个
        assert_eq!(wrap_to_width("abcd", 4), vec!["abcd"]);
        // 混排
        assert_eq!(wrap_to_width("a中b中", 4), vec!["a中b", "中"]);
    }

    #[test]
    fn wrap_prefers_breaking_at_spaces() {
        let lines = wrap_to_width("hello world foo", 11);
        assert_eq!(lines, vec!["hello world", "foo"], "应在空格处断而不是切词");
        // 放不下时整词下移，绝不从词中间切
        let lines = wrap_to_width("alpha beta gamma", 10);
        assert!(lines.iter().all(|l| !l.starts_with(' ') && display_width(l) <= 10));
        for w in ["alpha", "beta", "gamma"] {
            assert!(lines.iter().any(|l| l.split(' ').any(|t| t == w)), "{w} 不应被切开：{lines:?}");
        }
    }

    #[test]
    fn wrap_hard_splits_a_word_longer_than_the_line() {
        // 长路径/URL 放不下时必须硬切，否则会溢出
        let lines = wrap_to_width("averylongpath/with/segments/here", 8);
        assert!(lines.iter().all(|l| display_width(l) <= 8), "不得溢出：{lines:?}");
        assert_eq!(lines.join(""), "averylongpath/with/segments/here", "不得丢字符");

        // 连续 CJK（无空格）同样硬切
        let lines = wrap_to_width("这是一段连续的中文没有任何空格", 6);
        assert!(lines.iter().all(|l| display_width(l) <= 6), "不得溢出：{lines:?}");
        assert_eq!(lines.join(""), "这是一段连续的中文没有任何空格");
    }

    #[test]
    fn wrap_never_splits_a_char() {
        // 1 列预算下，宽字符只能独占一行（不能切成半个）
        let lines = wrap_to_width("中中", 1);
        assert!(lines.iter().all(|l| display_width(l) <= 2 || l.is_empty() || l == "中"));
        let joined: String = lines.join("");
        assert_eq!(joined, "中中", "换行不得丢失字符");
    }

    #[test]
    fn truncate_accounts_for_width_and_adds_ellipsis() {
        assert_eq!(truncate_to_width("中文测试", 5), "中文…", "5 列 = 2+2+1(省略号)");
        assert_eq!(truncate_to_width("abcde", 5), "abcde", "刚好放得下不截断");
        assert_eq!(truncate_to_width("abcdef", 5), "abcd…");
        assert_eq!(display_width(&truncate_to_width("中文测试中文", 3)), 3);
    }

    #[test]
    fn pad_aligns_columns() {
        assert_eq!(pad_to_width("中", 4), "中  ");
        assert_eq!(display_width(&pad_to_width("中", 4)), 4);
        assert_eq!(pad_to_width("abcd", 4), "abcd");
        assert_eq!(pad_to_width("abcdef", 4), "abcdef", "超出不裁剪");
    }
}
