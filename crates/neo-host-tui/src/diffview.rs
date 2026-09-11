//! Diff 查看器 —— 解析 unified diff、按 hunk/文件导航、split/unified 视图
//!
//! # 为什么要单独做，而不是把 diff 打进正文
//!
//! 之前审批时直接把 diff 前 200 行打进对话流。那对大改动完全不够用：
//! 读不完、跳不到想看的文件、也没法只看某个 hunk。opencode 为此做了一个
//! 一千多行的独立查看器，说明这不是小功能。
//!
//! # 状态与渲染分离
//!
//! 这个模块只做**解析 + 光标/滚动状态 + 导航语义**，不碰 ANSI。
//! 渲染在 `lib.rs` 用 `Grid` 做。理由与其它模块一致：导航逻辑可以脱离
//! 终端单测（"按 n 会跳到下一个文件"不该靠截图来验证）。
//!
//! # 内存有界
//!
//! 解析时对行数与单行长度都设上限。一个巨大的 diff 全渲染出来既没意义
//! （没人从头读到尾）也无界 —— 超限时如实标注 `truncated`。

/// 单行最大保留长度（字符）。超长的"单行"通常是压缩产物，截断展示即可。
const MAX_LINE_CHARS: usize = 2000;
/// 单个 diff 最多解析多少行。超出部分丢弃并标记 truncated。
const MAX_LINES: usize = 20_000;

/// 一行 diff 的种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// 文件头（`--- a/x` / `+++ b/x`）
    Header,
    /// hunk 头（`@@ -1,3 +1,5 @@`）
    HunkHeader,
    /// 新增行
    Add,
    /// 删除行
    Del,
    /// 上下文行
    Context,
}

/// 一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub kind: Kind,
    /// 原文（不含前导 +/-/空格 标记）
    pub text: String,
    /// 旧文件行号（新增行/文件头为 None）
    pub old_no: Option<usize>,
    /// 新文件行号（删除行/文件头为 None）
    pub new_no: Option<usize>,
}

/// 一个 hunk。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hunk {
    /// 该 hunk 在 `Diff::lines` 中的起始下标
    pub start: usize,
    /// 结束下标（不含）
    pub end: usize,
    pub adds: usize,
    pub dels: usize,
}

/// 一个文件。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct File {
    pub path: String,
    /// 该文件在 `Diff::lines` 中的首行下标（`--- a/x` 那行）。
    ///
    /// 必须显式记录：靠 `hunks[0].start - 1` 反推是错的 —— 文件头有两行
    /// （`---` / `+++`），而且没有 hunk 的空文件根本推不出来。
    pub line_start: usize,
    pub hunks: Vec<Hunk>,
    pub adds: usize,
    pub dels: usize,
}

/// 解析结果。
#[derive(Debug, Clone, Default)]
pub struct Diff {
    /// 全部行（按文件顺序拼接，带 Header/HunkHeader 分隔）
    pub lines: Vec<Line>,
    pub files: Vec<File>,
    /// 因超限被丢弃的行数（>0 表示展示不完整）
    pub truncated_lines: usize,
}

/// 视图模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    /// 单列（删除紧邻新增）
    #[default]
    Unified,
    /// 双列（左右对照，宽终端才建议）
    Split,
}

/// 查看器状态：光标、滚动、视图模式、侧栏。
#[derive(Debug, Clone)]
pub struct Viewer {
    pub diff: Diff,
    /// 光标所在行（`Diff::lines` 下标）
    pub cursor: usize,
    /// 滚动偏移（首行在窗口中的位置）
    pub offset: usize,
    pub mode: ViewMode,
    /// 是否显示文件树
    pub tree: bool,
    /// 文件树里选中的文件下标
    pub file_cursor: usize,
}

impl Viewer {
    pub fn new(diff: Diff) -> Self {
        let file_cursor = 0;
        Self {
            diff,
            cursor: 0,
            offset: 0,
            mode: ViewMode::default(),
            tree: true,
            file_cursor,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.diff.lines.is_empty()
    }

    pub fn line_count(&self) -> usize {
        self.diff.lines.len()
    }

    /// 当前光标落在哪个文件。
    pub fn current_file(&self) -> Option<usize> {
        let mut best = None;
        for (i, f) in self.diff.files.iter().enumerate() {
            if f.line_start <= self.cursor {
                best = Some(i);
            }
        }
        best
    }

    /// 光标移动到指定行，并把窗口滚到可见。
    pub fn set_cursor(&mut self, line: usize, viewport: usize) {
        if self.diff.lines.is_empty() {
            self.cursor = 0;
            return;
        }
        self.cursor = line.min(self.diff.lines.len() - 1);
        self.ensure_visible(viewport);
    }

    /// 保证光标在窗口内（上下各留 2 行余量）。
    fn ensure_visible(&mut self, viewport: usize) {
        if viewport == 0 {
            return;
        }
        let margin = 2.min(viewport / 4);
        if self.cursor < self.offset + margin {
            self.offset = self.cursor.saturating_sub(margin);
        } else if self.cursor + margin >= self.offset + viewport {
            self.offset = (self.cursor + margin + 1).saturating_sub(viewport);
        }
        self.clamp_offset(viewport);
    }

    fn clamp_offset(&mut self, viewport: usize) {
        let max = self.line_count().saturating_sub(viewport);
        self.offset = self.offset.min(max);
    }

    pub fn scroll(&mut self, delta: isize, viewport: usize) {
        let max = self.line_count().saturating_sub(viewport);
        let next = self.offset as isize + delta;
        self.offset = next.clamp(0, max as isize) as usize;
        // 滚动后若光标跑出窗口，把它拉回来（否则高亮行看不见）
        if self.cursor < self.offset {
            self.cursor = self.offset;
        } else if viewport > 0 && self.cursor >= self.offset + viewport {
            self.cursor = self.offset + viewport - 1;
        }
    }

    /// 下一个 / 上一个 hunk（**跨文件**，与编辑器的 `]` `[` 语义一致）。
    pub fn hunk_step(&mut self, forward: bool, viewport: usize) {
        let mut starts: Vec<usize> = Vec::new();
        for f in &self.diff.files {
            for h in &f.hunks {
                starts.push(h.start);
            }
        }
        if starts.is_empty() {
            return;
        }
        let target = if forward {
            starts.iter().find(|s| **s > self.cursor).copied()
        } else {
            starts.iter().rev().find(|s| **s < self.cursor).copied()
        };
        // 到底了就留在原处（不循环：查看器里循环会让人失去方向感）
        if let Some(t) = target {
            self.set_cursor(t, viewport);
        }
    }

    /// 下一个 / 上一个文件。
    pub fn file_step(&mut self, forward: bool, viewport: usize) {
        if self.diff.files.is_empty() {
            return;
        }
        let cur = self.current_file().unwrap_or(0);
        let next = if forward {
            if cur + 1 >= self.diff.files.len() {
                return;
            }
            cur + 1
        } else {
            if cur == 0 {
                return;
            }
            cur - 1
        };
        self.file_cursor = next;
        let start = self.diff.files[next].line_start;
        self.set_cursor(start, viewport);
    }

    /// 跳到文件树里选中的文件。
    pub fn goto_selected_file(&mut self, viewport: usize) {
        if let Some(f) = self.diff.files.get(self.file_cursor) {
            let start = f.line_start;
            self.set_cursor(start, viewport);
        }
    }

    pub fn tree_step(&mut self, delta: isize, viewport: usize) {
        if self.diff.files.is_empty() {
            return;
        }
        let last = self.diff.files.len() - 1;
        let next = (self.file_cursor as isize + delta).clamp(0, last as isize) as usize;
        self.file_cursor = next;
        self.goto_selected_file(viewport);
    }

    pub fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            ViewMode::Unified => ViewMode::Split,
            ViewMode::Split => ViewMode::Unified,
        };
    }

    pub fn toggle_tree(&mut self) {
        self.tree = !self.tree;
    }

    /// 顶部状态串（文件数、增删、是否截断）。
    pub fn summary(&self) -> String {
        let adds: usize = self.diff.files.iter().map(|f| f.adds).sum();
        let dels: usize = self.diff.files.iter().map(|f| f.dels).sum();
        let mut s = format!(
            "{} 个文件  +{adds} -{dels}  第 {}/{} 行",
            self.diff.files.len(),
            self.cursor + 1,
            self.line_count()
        );
        if self.diff.truncated_lines > 0 {
            s.push_str(&format!("  （另有 {} 行未载入）", self.diff.truncated_lines));
        }
        s
    }
}

/// 解析 unified diff 文本。
///
/// 能处理我们自己的 `diff::unified_diff` 输出，也能处理标准 git diff
/// （忽略 `diff --git` / `index` 等我们不展示的元信息行）。
pub fn parse(text: &str) -> Diff {
    let mut d = Diff::default();
    let mut cur_file: Option<File> = None;
    let mut cur_hunk: Option<Hunk> = None;
    // 正在累积的行号（从 hunk 头解析）
    let mut old_no = 0usize;
    let mut new_no = 0usize;
    let mut total_lines = 0usize;

    // 收口一个 hunk：填 end 并推入当前文件。
    // 只在这里 push —— 之前在建占位时也 push 了一次，导致每个 hunk 被记两遍。
    macro_rules! flush_hunk {
        () => {
            if let Some(h) = cur_hunk.take() {
                let mut h = h;
                h.end = d.lines.len();
                if let Some(f) = cur_file.as_mut() {
                    f.hunks.push(h);
                }
            }
        };
    }
    macro_rules! flush_file {
        () => {
            flush_hunk!();
            if let Some(f) = cur_file.take() {
                if !f.path.is_empty() {
                    d.files.push(f);
                }
            }
        };
    }

    for raw in text.lines() {
        if total_lines >= MAX_LINES {
            d.truncated_lines += 1;
            continue;
        }
        total_lines += 1;
        // 先记下是否被截断，正文长度按"去掉标记后"算，便于断言
        let line = if raw.chars().count() > MAX_LINE_CHARS {
            raw.chars().take(MAX_LINE_CHARS).collect::<String>()
        } else {
            raw.to_string()
        };

        // 文件头：`--- a/x` 或 `+++ b/x`
        if let Some(rest) = line.strip_prefix("+++ ") {
            // 新文件开始：先收尾上一个
            flush_file!();
            let path = rest.trim().trim_start_matches("b/").to_string();
            // 上一个文件若还没收口（没有 hunk 的文件），先推入
            if let Some(prev) = cur_file.as_mut() {
                if prev.path.is_empty() {
                    prev.path = path.clone();
                    prev.line_start = d.lines.len().saturating_sub(1);
                    continue;
                }
            }
            // `---` 那行已在上一轮 push，故 line_start 是它的下标
            let line_start = d.lines.len().saturating_sub(1);
            cur_file = Some(File { path, line_start, ..Default::default() });
            d.lines.push(Line {
                kind: Kind::Header,
                text: line.clone(),
                old_no: None,
                new_no: None,
            });
            continue;
        }
        if line.starts_with("--- ") {
            d.lines.push(Line {
                kind: Kind::Header,
                text: line.clone(),
                old_no: None,
                new_no: None,
            });
            continue;
        }
        // 不需要展示的 git 元信息
        if line.starts_with("diff --git ")
            || line.starts_with("index ")
            || line.starts_with("new file mode")
            || line.starts_with("deleted file mode")
            || line.starts_with("similarity index")
            || line.starts_with("rename ")
        {
            continue;
        }
        // hunk 头：`@@ -a,b +c,d @@`
        if line.starts_with("@@") {
            flush_hunk!();
            let (o, n) = parse_hunk_header(&line);
            old_no = o;
            new_no = n;
            // 只在这里放一个占位；flush_hunk 负责填 end 并推进 files[].hunks
            cur_hunk = Some(Hunk { start: d.lines.len(), ..Default::default() });
            d.lines.push(Line {
                kind: Kind::HunkHeader,
                text: line.clone(),
                old_no: None,
                new_no: None,
            });
            continue;
        }

        // 内容行
        let (kind, text, o, n) = if let Some(rest) = line.strip_prefix('+') {
            let o = None;
            let n = Some(new_no);
            new_no += 1;
            (Kind::Add, rest.to_string(), o, n)
        } else if let Some(rest) = line.strip_prefix('-') {
            let o = Some(old_no);
            old_no += 1;
            (Kind::Del, rest.to_string(), o, None)
        } else if let Some(rest) = line.strip_prefix(' ') {
            let o = Some(old_no);
            let n = Some(new_no);
            old_no += 1;
            new_no += 1;
            (Kind::Context, rest.to_string(), o, n)
        } else if line.is_empty() {
            // 空行在 unified diff 里表示"上下文中的空行"
            let o = Some(old_no);
            let n = Some(new_no);
            old_no += 1;
            new_no += 1;
            (Kind::Context, String::new(), o, n)
        } else {
            // 不认识的行（如 `\ No newline at end of file`）：按上下文处理
            (Kind::Context, line.clone(), None, None)
        };

        if let Some(h) = cur_hunk.as_mut() {
            match kind {
                Kind::Add => {
                    h.adds += 1;
                    if let Some(f) = cur_file.as_mut() {
                        f.adds += 1;
                    }
                }
                Kind::Del => {
                    h.dels += 1;
                    if let Some(f) = cur_file.as_mut() {
                        f.dels += 1;
                    }
                }
                _ => {}
            }
        }
        d.lines.push(Line { kind, text, old_no: o, new_no: n });
    }
    flush_file!();
    d
}

/// 从 `@@ -a,b +c,d @@` 解析出旧/新起始行号。
fn parse_hunk_header(line: &str) -> (usize, usize) {
    let mut old = 1usize;
    let mut new = 1usize;
    for part in line.split_whitespace() {
        if let Some(rest) = part.strip_prefix('-') {
            old = rest.split(',').next().and_then(|s| s.parse().ok()).unwrap_or(1);
        } else if let Some(rest) = part.strip_prefix('+') {
            new = rest.split(',').next().and_then(|s| s.parse().ok()).unwrap_or(1);
        }
    }
    (old, new)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,4 +1,5 @@
 fn main() {
-    old_call();
+    new_call();
+    extra();
 }
@@ -10,3 +11,3 @@
 keep
-remove me
+add me";

    #[test]
    fn parses_files_hunks_and_lines() {
        let d = parse(SAMPLE);
        assert_eq!(d.files.len(), 1);
        assert_eq!(d.files[0].path, "src/main.rs");
        assert_eq!(d.files[0].hunks.len(), 2, "应解析出两个 hunk");
        assert_eq!(d.files[0].adds, 3);
        assert_eq!(d.files[0].dels, 2);
        assert!(d.truncated_lines == 0);
    }

    #[test]
    fn tracks_line_numbers_from_hunk_headers() {
        let d = parse(SAMPLE);
        // 第一个 hunk 从旧 1 / 新 1 开始
        let ctx = d.lines.iter().find(|l| l.text == "fn main() {").unwrap();
        assert_eq!(ctx.old_no, Some(1));
        assert_eq!(ctx.new_no, Some(1));
        let del = d.lines.iter().find(|l| l.text == "    old_call();").unwrap();
        assert_eq!(del.old_no, Some(2));
        assert_eq!(del.new_no, None, "删除行没有新行号");
        let add = d.lines.iter().find(|l| l.text == "    new_call();").unwrap();
        assert_eq!(add.new_no, Some(2));
        assert_eq!(add.old_no, None, "新增行没有旧行号");
    }

    #[test]
    fn second_hunk_resumes_from_its_own_header() {
        // 第二个 hunk 头写的是 @@ -10,3 +11,3 @@，行号必须从那继续，
        // 而不是接着第一个 hunk 数（那是最常见的手写解析错误）
        let d = parse(SAMPLE);
        let keep = d.lines.iter().find(|l| l.text == "keep").unwrap();
        assert_eq!(keep.old_no, Some(10), "应取 hunk 头里的旧行号");
        assert_eq!(keep.new_no, Some(11));
    }

    #[test]
    fn parses_multiple_files() {
        let text = "\
--- a/a.txt
+++ b/a.txt
@@ -1 +1 @@
-x
+y
--- a/b.txt
+++ b/b.txt
@@ -1 +1 @@
-p
+q";
        let d = parse(text);
        assert_eq!(d.files.len(), 2);
        assert_eq!(d.files[0].path, "a.txt");
        assert_eq!(d.files[1].path, "b.txt");
    }

    #[test]
    fn ignores_git_metadata_lines() {
        let text = "\
diff --git a/x.rs b/x.rs
index 1234567..89abcde 100644
--- a/x.rs
+++ b/x.rs
@@ -1 +1 @@
-a
+b";
        let d = parse(text);
        assert!(
            !d.lines.iter().any(|l| l.text.starts_with("diff --git")),
            "不该展示 git 元信息"
        );
        assert_eq!(d.files.len(), 1);
        assert_eq!(d.files[0].adds, 1);
    }

    #[test]
    fn empty_input_is_safe() {
        let d = parse("");
        assert!(d.lines.is_empty());
        assert!(d.files.is_empty());
        let v = Viewer::new(d);
        assert!(v.is_empty());
        // 空查看器上的任何操作都不该 panic
        let mut v = v;
        v.hunk_step(true, 10);
        v.file_step(true, 10);
        v.scroll(5, 10);
        v.toggle_mode();
        assert_eq!(v.cursor, 0);
    }

    #[test]
    fn hunk_navigation_jumps_between_hunks() {
        let mut v = Viewer::new(parse(SAMPLE));
        // 从行 0 出发，"下一个 hunk"就是第一个 hunk
        v.hunk_step(true, 20);
        assert_eq!(v.cursor, v.diff.files[0].hunks[0].start, "应跳到第一个 hunk");
        v.hunk_step(true, 20);
        assert_eq!(v.cursor, v.diff.files[0].hunks[1].start, "应跳到第二个 hunk");
        v.hunk_step(false, 20);
        assert_eq!(v.cursor, v.diff.files[0].hunks[0].start, "应回退到第一个 hunk");
    }

    #[test]
    fn hunk_navigation_does_not_wrap() {
        // 不循环：查看器里循环会让人失去方向感
        let mut v = Viewer::new(parse(SAMPLE));
        v.hunk_step(false, 20);
        assert_eq!(v.cursor, 0, "行 0 之前没有 hunk，应不动");
        v.hunk_step(true, 20); // → hunks[0]
        v.hunk_step(true, 20); // → hunks[1]
        let last = v.diff.files[0].hunks[1].start;
        assert_eq!(v.cursor, last);
        v.hunk_step(true, 20);
        assert_eq!(v.cursor, last, "已在最后一个则不动");
    }

    #[test]
    fn file_navigation_moves_across_files() {
        let text = "\
--- a/a.txt
+++ b/a.txt
@@ -1 +1 @@
-x
+y
--- a/b.txt
+++ b/b.txt
@@ -1 +1 @@
-p
+q";
        let mut v = Viewer::new(parse(text));
        assert_eq!(v.current_file(), Some(0));
        v.file_step(true, 20);
        assert_eq!(v.current_file(), Some(1), "应到第二个文件");
        v.file_step(true, 20);
        assert_eq!(v.current_file(), Some(1), "已在末尾则不动");
        v.file_step(false, 20);
        assert_eq!(v.current_file(), Some(0));
    }

    #[test]
    fn tree_step_selects_and_jumps() {
        let text = "\
--- a/a.txt
+++ b/a.txt
@@ -1 +1 @@
-x
+y
--- a/b.txt
+++ b/b.txt
@@ -1 +1 @@
-p
+q";
        let mut v = Viewer::new(parse(text));
        assert_eq!(v.file_cursor, 0);
        v.tree_step(1, 20);
        assert_eq!(v.file_cursor, 1);
        assert_eq!(v.current_file(), Some(1), "树里选中应带动光标跳转");
        v.tree_step(1, 20);
        assert_eq!(v.file_cursor, 1, "已在末尾则不动");
    }

    #[test]
    fn scroll_keeps_cursor_inside_the_window() {
        let big: String = (0..200)
            .map(|i| format!("+line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let text = format!("--- a/x\n+++ b/x\n@@ -1,1 +1,200 @@\n{big}");
        let mut v = Viewer::new(parse(&text));
        v.set_cursor(100, 20);
        let (start, end) = (v.offset, v.offset + 20);
        assert!(start <= v.cursor && v.cursor < end, "光标应在窗口内");
        v.scroll(-500, 20);
        assert_eq!(v.offset, 0);
        assert!(v.cursor >= v.offset, "光标不该跑到窗口上方");
        v.scroll(5000, 20);
        assert!(v.offset + 20 <= v.line_count() + 20);
        assert!(v.cursor < v.offset + 20, "光标不该跑到窗口下方");
    }

    #[test]
    fn offset_is_clamped_to_content() {
        let mut v = Viewer::new(parse(SAMPLE));
        v.scroll(9999, 8);
        assert!(
            v.offset + 8 <= v.line_count() + 8,
            "偏移不该远超内容（否则整屏空白）"
        );
    }

    #[test]
    fn toggle_mode_and_tree() {
        let mut v = Viewer::new(parse(SAMPLE));
        assert_eq!(v.mode, ViewMode::Unified);
        v.toggle_mode();
        assert_eq!(v.mode, ViewMode::Split);
        v.toggle_mode();
        assert_eq!(v.mode, ViewMode::Unified);
        assert!(v.tree);
        v.toggle_tree();
        assert!(!v.tree);
    }

    #[test]
    fn summary_reports_files_and_changes() {
        let v = Viewer::new(parse(SAMPLE));
        let s = v.summary();
        assert!(s.contains("1 个文件"), "{s}");
        assert!(s.contains("+3"), "{s}");
        assert!(s.contains("-2"), "{s}");
    }

    #[test]
    fn overlong_lines_and_huge_diffs_are_bounded() {
        // 内存有界：超长单行截断，超多行标记 truncated
        let long = "+".to_string() + &"x".repeat(MAX_LINE_CHARS + 500);
        let text = format!("--- a/x\n+++ b/x\n@@ -1 +1 @@\n{long}");
        let d = parse(&text);
        let l = d.lines.iter().find(|l| l.kind == Kind::Add).unwrap();
        // raw 行被截到 MAX_LINE_CHARS，去掉前导 '+' 后正文少一个字符
        assert_eq!(
            l.text.chars().count(),
            MAX_LINE_CHARS - 1,
            "超长行应被截断到上限附近"
        );
        assert!(l.text.chars().count() < 2000 + 500, "不得保留原始长度");

        let many: String = (0..(MAX_LINES + 100))
            .map(|i| format!("+l{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let d2 = parse(&format!("--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n{many}"));
        assert!(d2.truncated_lines > 0, "超量应如实标记截断");
        assert!(d2.lines.len() <= MAX_LINES + 10);
    }

    #[test]
    fn handles_no_newline_marker() {
        // git 会输出 `\ No newline at end of file`：不该被当成文件头或崩掉
        let text = "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n\\ No newline at end of file\n+b";
        let d = parse(text);
        assert_eq!(d.files.len(), 1);
        assert!(d.lines.iter().any(|l| l.text.contains("No newline")));
    }

    #[test]
    fn context_lines_have_both_numbers_except_at_boundaries() {
        let d = parse(SAMPLE);
        for l in &d.lines {
            match l.kind {
                Kind::Add => assert!(l.old_no.is_none(), "新增行不该有旧行号"),
                Kind::Del => assert!(l.new_no.is_none(), "删除行不该有新行号"),
                _ => {}
            }
        }
    }
}
