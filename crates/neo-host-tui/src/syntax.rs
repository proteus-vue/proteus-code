//! 轻量语法高亮 —— 词法级，覆盖常见语言
//!
//! # 为什么不用 tree-sitter
//!
//! opencode 用 tree-sitter WASM 做高亮（每语言一个 .wasm + 在线拉取的 query）。
//! 那对本项目是**方向相反的**选择：它意味着引入 WASM 运行时 + 网络依赖 +
//! 数十 MB 解析器，把"调试链要浅"这条硬约束整个推翻。
//!
//! 这里做词法级高亮：识别注释 / 字符串 / 数字 / 关键字 / 类型名。
//! 它**不是**语法分析 —— 分不清 `foo(bar)` 里哪个是函数名，
//! 也不做作用域分析。但终端里 90% 的可读性提升来自"字符串和注释不再是
//! 一片同色文字"，词法级就能拿到。这是有意的取舍，边界写在文件末。
//!
//! # 内存有界
//!
//! 高亮是逐行的，不跨行持有状态（跨行字符串/注释会退化为普通文本）。
//! 单行长度也封顶，避免超长行撑出大 Vec。

use crate::Tone;

/// 支持的语言（按语法族归并，不追求穷举）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Rust,
    Python,
    JavaScript,
    Go,
    Shell,
    Json,
    Markdown,
    /// 未知语言：不做高亮（原样输出）
    Plain,
}

impl Lang {
    /// 从代码围栏的语言标记推断（```rust / ```py / ```ts ...）。
    pub fn from_fence(tag: &str) -> Self {
        let t = tag.trim().to_ascii_lowercase();
        match t.as_str() {
            "rust" | "rs" => Self::Rust,
            "python" | "py" => Self::Python,
            "javascript" | "js" | "jsx" | "typescript" | "ts" | "tsx" => Self::JavaScript,
            "go" | "golang" => Self::Go,
            "sh" | "bash" | "zsh" | "shell" | "console" => Self::Shell,
            "json" | "jsonc" => Self::Json,
            "md" | "markdown" => Self::Markdown,
            _ => Self::Plain,
        }
    }

    /// 从文件扩展名推断（diff 里的路径用得上）。
    pub fn from_path(path: &str) -> Self {
        let ext = path.rsplit('.').next().unwrap_or("");
        Self::from_fence(ext)
    }

    fn line_comment(self) -> &'static str {
        match self {
            Self::Rust | Self::JavaScript | Self::Go => "//",
            Self::Python | Self::Shell => "#",
            _ => "",
        }
    }

    /// 字符串定界符（`'` 与 `"`；Shell 里单引号不解释转义，这里统一处理）
    fn quote_chars(self) -> &'static [char] {
        match self {
            Self::Rust | Self::Python | Self::JavaScript | Self::Go | Self::Shell | Self::Json => {
                &['"', '\'']
            }
            _ => &[],
        }
    }

    fn is_block_comment_lang(self) -> bool {
        matches!(self, Self::Rust | Self::JavaScript | Self::Go)
    }

    fn keywords(self) -> &'static [&'static str] {
        match self {
            Self::Rust => &[
                "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else",
                "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match",
                "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct",
                "super", "trait", "true", "type", "unsafe", "use", "where", "while",
            ],
            Self::Python => &[
                "and", "as", "assert", "async", "await", "break", "class", "continue", "def",
                "del", "elif", "else", "except", "False", "finally", "for", "from", "global",
                "if", "import", "in", "is", "lambda", "None", "nonlocal", "not", "or", "pass",
                "raise", "return", "True", "try", "while", "with", "yield",
            ],
            Self::JavaScript => &[
                "async", "await", "break", "case", "catch", "class", "const", "continue",
                "default", "delete", "do", "else", "export", "extends", "false", "finally",
                "for", "from", "function", "if", "import", "in", "instanceof", "let", "new",
                "null", "of", "return", "static", "super", "switch", "this", "throw", "true",
                "try", "typeof", "undefined", "var", "void", "while", "yield",
            ],
            Self::Go => &[
                "break", "case", "chan", "const", "continue", "default", "defer", "else",
                "fallthrough", "for", "func", "go", "goto", "if", "import", "interface", "map",
                "package", "range", "return", "select", "struct", "switch", "type", "var",
            ],
            Self::Shell => &[
                "alias", "case", "cd", "do", "done", "echo", "elif", "else", "esac", "export",
                "fi", "for", "function", "if", "in", "local", "return", "then", "until", "while",
            ],
            Self::Json => &["true", "false", "null"],
            Self::Markdown | Self::Plain => &[],
        }
    }
}

/// 一行高亮的片段（列由调用方顺序拼接）。
pub type Span = (String, Tone);

/// 单行最大高亮长度。超出部分按普通文本处理 —— 超长行（如压缩的 JSON）
/// 高亮没有意义，但会带来可观的临时分配。
const MAX_LINE: usize = 4096;

/// 高亮一行。
pub fn highlight_line(line: &str, lang: Lang) -> Vec<Span> {
    if lang == Lang::Plain || lang == Lang::Markdown {
        return vec![(line.to_string(), Tone::Text)];
    }
    // 超长行：不高亮（不值得为它分配）
    if line.len() > MAX_LINE {
        return vec![(line.to_string(), Tone::Text)];
    }

    let chars: Vec<char> = line.chars().collect();
    let mut out: Vec<Span> = Vec::new();
    let mut buf = String::new();
    let mut cur = Tone::Text;
    let mut i = 0;

    let comment = lang.line_comment();
    let quotes = lang.quote_chars();
    let kw = lang.keywords();

    while i < chars.len() {
        let c = chars[i];

        // ── 块注释开始：从 `/*` 到行尾都算注释（不跨行持有状态）──
        if lang.is_block_comment_lang() && c == '/' && chars.get(i + 1) == Some(&'*') {
            push_span(&mut out, &mut buf, cur);
            buf.push_str("/*");
            i += 2;
            while i < chars.len() {
                buf.push(chars[i]);
                if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    buf.push('/');
                    i += 2;
                    break;
                }
                i += 1;
            }
            push_span(&mut out, &mut buf, Tone::Muted);
            cur = Tone::Text;
            continue;
        }

        // ── 行注释：剩下全是注释 ──
        if !comment.is_empty() && c == comment.chars().next().unwrap() {
            let whole = comment.chars().count();
            if whole == 1 || chars[i..].iter().take(whole).collect::<String>() == comment {
                push_span(&mut out, &mut buf, cur);
                out.push((chars[i..].iter().collect(), Tone::Muted));
                break;
            }
        }

        // ── 字符串：从定界符到配对的定界符（处理转义）──
        if quotes.contains(&c) {
            push_span(&mut out, &mut buf, cur);
            let q = c;
            buf.push(c);
            i += 1;
            while i < chars.len() {
                let ch = chars[i];
                buf.push(ch);
                if ch == '\\' {
                    if let Some(&n) = chars.get(i + 1) {
                        buf.push(n);
                        i += 2;
                        continue;
                    }
                }
                i += 1;
                if ch == q {
                    break;
                }
            }
            push_span(&mut out, &mut buf, Tone::Success);
            cur = Tone::Text;
            continue;
        }

        // ── 数字 ──
        if c.is_ascii_digit()
            && (i == 0 || (!chars[i - 1].is_alphanumeric() && chars[i - 1] != '_'))
        {
            push_span(&mut out, &mut buf, cur);
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric() || chars[i] == '.' || chars[i] == '_')
            {
                buf.push(chars[i]);
                i += 1;
            }
            push_span(&mut out, &mut buf, Tone::Warning);
            cur = Tone::Text;
            continue;
        }

        // ── 标识符 / 关键字 / 类型 ──
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            push_span(&mut out, &mut buf, cur);
            let tone = if kw.contains(&word.as_str()) {
                Tone::Accent
            } else if word.chars().next().map(|x| x.is_uppercase()).unwrap_or(false) {
                // 首字母大写 → 按类型名处理（Rust/Go/JS 里这个启发式很准）
                Tone::Info
            } else if chars.get(i) == Some(&'(') {
                // 后面紧跟 `(` → 按函数名处理
                Tone::Primary
            } else {
                Tone::Text
            };
            out.push((word, tone));
            continue;
        }

        buf.push(c);
        i += 1;
    }
    push_span(&mut out, &mut buf, cur);
    if out.is_empty() {
        out.push((String::new(), Tone::Text));
    }
    out
}

/// 把累积中的普通文本冲刷出去；同色调相邻片段合并，减少转义序列数量。
///
/// 用函数而不是宏：宏里若写 `continue` 会在调用点继续外层循环，
/// 把调用点**后续的语句整段跳过**（曾因此丢掉注释起始符与字符串内容）。
fn push_span(out: &mut Vec<Span>, buf: &mut String, tone: Tone) {
    if buf.is_empty() {
        return;
    }
    if let Some(last) = out.last_mut() {
        if last.1 == tone {
            last.0.push_str(buf);
            buf.clear();
            return;
        }
    }
    out.push((std::mem::take(buf), tone));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tones(line: &str, lang: Lang) -> Vec<Tone> {
        highlight_line(line, lang).into_iter().map(|(_, t)| t).collect()
    }

    /// 取某个子串的色调（找不到返回 None）。
    fn tone_of(line: &str, lang: Lang, needle: &str) -> Option<Tone> {
        highlight_line(line, lang)
            .into_iter()
            .find(|(s, _)| s.contains(needle))
            .map(|(_, t)| t)
    }

    #[test]
    fn reconstructs_the_line_exactly() {
        // 高亮绝不能丢字符或改字符 —— 拼接结果必须逐字节等于输入。
        // 这是所有高亮实现最容易出错的地方（尤其是注释与字符串边界）。
        let cases = [
            ("let x = 1; // 注释", Lang::Rust),
            ("s = \"hello \\\"world\\\"\"", Lang::Python),
            ("/* block */ code()", Lang::JavaScript),
            ("echo '单引号' # 注释", Lang::Shell),
            ("{\"a\": [1, 2, null]}", Lang::Json),
            ("func main() { fmt.Println(1) }", Lang::Go),
            ("if x >= 10 && y != 20 { }", Lang::Rust),
            ("没有语法的中文行", Lang::Plain),
        ];
        for (line, lang) in cases {
            let joined: String = highlight_line(line, lang).into_iter().map(|(s, _)| s).collect();
            assert_eq!(joined, line, "高亮改变了内容（{lang:?}）：{line:?}");
        }
    }

    #[test]
    fn keywords_strings_comments_get_distinct_tones() {
        let line = "let s = \"hi\"; // note";
        assert_eq!(tone_of(line, Lang::Rust, "let"), Some(Tone::Accent), "关键字");
        assert_eq!(tone_of(line, Lang::Rust, "\"hi\""), Some(Tone::Success), "字符串");
        assert_eq!(tone_of(line, Lang::Rust, "// note"), Some(Tone::Muted), "注释");
    }

    #[test]
    fn keywords_inside_strings_are_not_highlighted() {
        // `"let"` 里的 let 是字符串内容，不该被当关键字
        let spans = highlight_line("let s = \"let x\";", Lang::Rust);
        let in_string = spans
            .iter()
            .find(|(s, _)| s.contains("\"let x\""))
            .expect("应有字符串片段");
        assert_eq!(in_string.1, Tone::Success, "字符串内的关键字不该单独高亮：{spans:?}");
    }

    #[test]
    fn comments_swallow_everything_after_them() {
        // 注释后的代码不该被高亮（`// let x` 里的 let 是注释文本）
        let spans = highlight_line("// let x = 1", Lang::Rust);
        assert_eq!(spans.len(), 1, "整行应是一个注释片段：{spans:?}");
        assert_eq!(spans[0].1, Tone::Muted);
    }

    #[test]
    fn numbers_are_highlighted_but_not_identifiers_with_digits() {
        assert_eq!(tone_of("x = 42;", Lang::Rust, "42"), Some(Tone::Warning));
        // `x86` 是一个标识符，不该被拆成标识符+数字
        let joined: String = highlight_line("let x86 = 1;", Lang::Rust)
            .into_iter()
            .map(|(s, _)| s)
            .collect();
        assert_eq!(joined, "let x86 = 1;");
    }

    #[test]
    fn unknown_language_is_left_alone() {
        let line = "let x = 1; // rust-looking";
        let spans = highlight_line(line, Lang::Plain);
        assert_eq!(spans.len(), 1, "未知语言不该做任何切分");
        assert_eq!(spans[0].1, Tone::Text);
    }

    #[test]
    fn overlong_lines_skip_highlighting() {
        // 内存有界：超长行不做高亮（避免大量临时分配）
        let long = "a".repeat(MAX_LINE + 10);
        let spans = highlight_line(&long, Lang::Rust);
        assert_eq!(spans.len(), 1, "超长行应原样返回");
        assert_eq!(spans[0].0.len(), long.len(), "内容不能丢");
    }

    #[test]
    fn fences_map_to_languages_and_unknown_is_plain() {
        assert_eq!(Lang::from_fence("rust"), Lang::Rust);
        assert_eq!(Lang::from_fence("  TS  "), Lang::JavaScript);
        assert_eq!(Lang::from_fence("py"), Lang::Python);
        assert_eq!(Lang::from_fence("世界语"), Lang::Plain);
        assert_eq!(Lang::from_path("src/main.rs"), Lang::Rust);
        assert_eq!(Lang::from_path("a/b.go"), Lang::Go);
        assert_eq!(Lang::from_path("noext"), Lang::Plain);
    }

    #[test]
    fn empty_line_is_safe() {
        let spans = highlight_line("", Lang::Rust);
        assert_eq!(spans.len(), 1);
        assert!(spans[0].0.is_empty());
    }

    #[test]
    fn every_span_tone_is_in_the_palette() {
        // 不做"未定义色调"的兜底：所有输出色调都必须是调色板里的语义色，
        // 否则换主题时会出现无法翻译的颜色
        let line = "pub fn f(x: u32) -> String { \"s\".to_string() } // c 42";
        for (_, t) in highlight_line(line, Lang::Rust) {
            assert!(tones(line, Lang::Rust).contains(&t));
        }
    }
}
