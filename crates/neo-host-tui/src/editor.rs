//! 多行输入编辑器 —— 光标、行编辑、撤销
//!
//! # 为什么不能继续用 `String`
//!
//! 之前输入是 `String`，所有编辑操作都是 `push` / `pop` —— 只能改**末尾**。
//! 贴一段代码进去就没法改中间、没法换行、没法删整行。opencode 用的是多行
//! textarea，这是每天都会碰到的差别。
//!
//! # 全部按**字符**索引，不按字节
//!
//! `col` 是字符下标而不是字节偏移。中文一行 10 个字、字节长度 30，
//! 混用这两个口径会让光标跳到半个汉字上，或在 `String::insert` 时 panic
//! （不是字符边界）。这个坑在本项目里已经踩过一次（Markdown 强调解析），
//! 所以这里从类型上就固定口径：`lines: Vec<String>` + `col` 为字符数。
//!
//! # 撤销有界
//!
//! 撤销栈存快照，因此**必须有上限** —— 否则一个长会话下来内存无界增长。
//! 超过上限就丢最旧的（`VecDeque`），代价是极早的操作不可撤销，
//! 这比"内存随输入无限增长"可接受得多。

use std::collections::VecDeque;

/// 撤销栈上限（份数）。每份是一整份文本，故不能太大。
const MAX_UNDO: usize = 100;

/// 多行输入缓冲。
#[derive(Debug, Clone, Default)]
pub struct Editor {
    /// 至少一行（空编辑器是 `vec![""]`）。
    lines: Vec<String>,
    /// 光标行（0 基）。
    row: usize,
    /// 光标列（0 基，**字符**索引）。
    col: usize,
    /// 撤销栈（旧的文本快照）与重做栈。
    undo: VecDeque<String>,
    redo: Vec<String>,
    /// 上一次插入的字符（用于合并连续输入的撤销粒度）。
    last_was_typing: bool,
}

impl Editor {
    pub fn new() -> Self {
        Self { lines: vec![String::new()], row: 0, col: 0, ..Default::default() }
    }

    pub fn from_text(text: &str) -> Self {
        let mut e = Self::new();
        e.set(text);
        e
    }

    /// 完整文本（行间以 `\n` 连接）。
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// 供渲染用的行切片。
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn cursor(&self) -> (usize, usize) {
        (self.row, self.col)
    }

    /// 整体是否为空（所有行都是空串）。
    pub fn is_empty(&self) -> bool {
        self.lines.len() == 1 && self.lines[0].is_empty()
    }

    /// 替换全部内容，光标移到末尾。
    pub fn set(&mut self, text: &str) {
        self.push_undo();
        self.lines = text.split('\n').map(|s| s.to_string()).collect();
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.row = self.lines.len() - 1;
        self.col = self.lines[self.row].chars().count();
        self.last_was_typing = false;
    }

    pub fn clear(&mut self) {
        if self.is_empty() {
            return;
        }
        self.push_undo();
        self.lines = vec![String::new()];
        self.row = 0;
        self.col = 0;
        self.last_was_typing = false;
    }

    // ── 插入 ─────────────────────────────────────────────────────────

    pub fn insert_char(&mut self, c: char) {
        // 连续打字合并为一次撤销（否则逐字符撤销很烦）
        if !self.last_was_typing {
            self.push_undo();
        }
        self.last_was_typing = true;
        let idx = self.byte_index(self.row, self.col);
        self.lines[self.row].insert(idx, c);
        self.col += 1;
    }

    /// 插入一段文本（粘贴）。内嵌换行会拆成多行。
    pub fn insert_str(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        self.push_undo();
        self.last_was_typing = false;
        let parts: Vec<&str> = s.split('\n').collect();
        if parts.len() == 1 {
            let idx = self.byte_index(self.row, self.col);
            self.lines[self.row].insert_str(idx, parts[0]);
            self.col += parts[0].chars().count();
            return;
        }
        // 多行：当前行拆开，中间整段插入
        let idx = self.byte_index(self.row, self.col);
        let tail = self.lines[self.row][idx..].to_string();
        self.lines[self.row].truncate(idx);
        self.lines[self.row].push_str(parts[0]);
        let mut insert_at = self.row + 1;
        for mid in &parts[1..parts.len() - 1] {
            self.lines.insert(insert_at, mid.to_string());
            insert_at += 1;
        }
        let last = parts[parts.len() - 1];
        self.lines.insert(insert_at, format!("{last}{tail}"));
        self.row = insert_at;
        self.col = last.chars().count();
    }

    /// 在光标处换行。
    pub fn newline(&mut self) {
        self.push_undo();
        self.last_was_typing = false;
        let idx = self.byte_index(self.row, self.col);
        let tail = self.lines[self.row][idx..].to_string();
        self.lines[self.row].truncate(idx);
        self.lines.insert(self.row + 1, tail);
        self.row += 1;
        self.col = 0;
    }

    // ── 删除 ─────────────────────────────────────────────────────────

    /// 退格。返回是否有内容被删（无内容时调用方可做别的处理）。
    pub fn backspace(&mut self) -> bool {
        self.last_was_typing = false;
        if self.col > 0 {
            self.push_undo();
            let start = self.byte_index(self.row, self.col - 1);
            let end = self.byte_index(self.row, self.col);
            self.lines[self.row].replace_range(start..end, "");
            self.col -= 1;
            return true;
        }
        if self.row > 0 {
            self.push_undo();
            let cur = self.lines.remove(self.row);
            self.row -= 1;
            self.col = self.lines[self.row].chars().count();
            self.lines[self.row].push_str(&cur);
            return true;
        }
        false
    }

    /// 删除光标处字符（Delete 键）。
    pub fn delete_forward(&mut self) -> bool {
        self.last_was_typing = false;
        let len = self.lines[self.row].chars().count();
        if self.col < len {
            self.push_undo();
            let start = self.byte_index(self.row, self.col);
            let end = self.byte_index(self.row, self.col + 1);
            self.lines[self.row].replace_range(start..end, "");
            return true;
        }
        if self.row + 1 < self.lines.len() {
            self.push_undo();
            let next = self.lines.remove(self.row + 1);
            self.lines[self.row].push_str(&next);
            return true;
        }
        false
    }

    /// `ctrl+k`：删到行尾；行尾时并入下一行（与多数编辑器一致）。
    pub fn delete_to_line_end(&mut self) {
        let len = self.lines[self.row].chars().count();
        if self.col < len {
            self.push_undo();
            self.last_was_typing = false;
            let idx = self.byte_index(self.row, self.col);
            self.lines[self.row].truncate(idx);
            return;
        }
        if self.row + 1 < self.lines.len() {
            self.delete_forward();
        }
    }

    /// `ctrl+u`：删到行首。
    pub fn delete_to_line_start(&mut self) {
        if self.col > 0 {
            self.push_undo();
            self.last_was_typing = false;
            let idx = self.byte_index(self.row, self.col);
            self.lines[self.row].replace_range(..idx, "");
            self.col = 0;
        }
    }

    /// `ctrl+w`：删前一个词。
    ///
    /// **词边界按空白划分**（与 shell / readline 一致），中文不按单字拆。
    /// 取舍理由：按空白划分行为可预测、与用户在 shell 里的肌肉记忆一致；
    /// 按 Unicode 词边界拆中文会让"中文测试"被逐字删除，反而意外。
    /// 代价是 `中文测试ok`（无空白）会被整段删掉 —— 已知边界。
    pub fn delete_word_backward(&mut self) {
        if self.col == 0 && self.row > 0 {
            self.backspace();
            return;
        }
        if self.col == 0 {
            return;
        }
        self.push_undo();
        self.last_was_typing = false;
        let chars: Vec<char> = self.lines[self.row].chars().collect();
        let mut i = self.col;
        // 先吃空白，再吃非空白（与 shell 的 ctrl+w 行为一致）
        while i > 0 && chars[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !chars[i - 1].is_whitespace() {
            i -= 1;
        }
        let start = self.byte_index(self.row, i);
        let end = self.byte_index(self.row, self.col);
        self.lines[self.row].replace_range(start..end, "");
        self.col = i;
    }

    // ── 移动 ─────────────────────────────────────────────────────────

    pub fn move_left(&mut self) {
        if self.col > 0 {
            self.col -= 1;
        } else if self.row > 0 {
            self.row -= 1;
            self.col = self.lines[self.row].chars().count();
        }
    }

    pub fn move_right(&mut self) {
        let len = self.lines[self.row].chars().count();
        if self.col < len {
            self.col += 1;
        } else if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.col = 0;
        }
    }

    /// 上移；已在首行则移到行首（与多数编辑器一致）。
    pub fn move_up(&mut self) {
        if self.row > 0 {
            self.row -= 1;
            self.col = self.col.min(self.lines[self.row].chars().count());
        } else {
            self.col = 0;
        }
    }

    pub fn move_down(&mut self) {
        if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.col = self.col.min(self.lines[self.row].chars().count());
        } else {
            self.col = self.lines[self.row].chars().count();
        }
    }

    pub fn move_home(&mut self) {
        self.col = 0;
    }

    pub fn move_end(&mut self) {
        self.col = self.lines[self.row].chars().count();
    }

    /// `alt+b`：光标移到前一个词首。
    pub fn word_backward(&mut self) {
        if self.col == 0 {
            return;
        }
        let chars: Vec<char> = self.lines[self.row].chars().collect();
        let mut i = self.col;
        while i > 0 && chars[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !chars[i - 1].is_whitespace() {
            i -= 1;
        }
        self.col = i;
    }

    /// `alt+f`：光标移到下一个词首。
    pub fn word_forward(&mut self) {
        let chars: Vec<char> = self.lines[self.row].chars().collect();
        let mut i = self.col;
        while i < chars.len() && !chars[i].is_whitespace() {
            i += 1;
        }
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        self.col = i;
    }

    // ── 撤销 / 重做 ───────────────────────────────────────────────────

    pub fn undo(&mut self) {
        let Some(prev) = self.undo.pop_back() else {
            return;
        };
        self.redo.push(self.text());
        self.restore(&prev);
        self.last_was_typing = false;
    }

    pub fn redo(&mut self) {
        let Some(next) = self.redo.pop() else {
            return;
        };
        self.undo.push_back(self.text());
        self.restore(&next);
        self.last_was_typing = false;
    }

    fn push_undo(&mut self) {
        self.undo.push_back(self.text());
        if self.undo.len() > MAX_UNDO {
            self.undo.pop_front();
        }
        // 任何新编辑都作废重做栈（标准语义）
        self.redo.clear();
    }

    /// 恢复文本（撤销/重做共用）。光标夹到合法位置。
    fn restore(&mut self, text: &str) {
        self.lines = text.split('\n').map(|s| s.to_string()).collect();
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.row = self.row.min(self.lines.len() - 1);
        self.col = self.col.min(self.lines[self.row].chars().count());
    }

    /// 字符列 → 该行的字节偏移。**这是唯一做这个换算的地方** ——
    /// 散落各处就会有人忘了换，导致中文上 panic 或切错字符。
    fn byte_index(&self, row: usize, col: usize) -> usize {
        let line = &self.lines[row];
        line.char_indices().nth(col).map(|(i, _)| i).unwrap_or(line.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_empty_with_one_line() {
        let e = Editor::new();
        assert!(e.is_empty());
        assert_eq!(e.line_count(), 1);
        assert_eq!(e.cursor(), (0, 0));
        assert_eq!(e.text(), "");
    }

    #[test]
    fn typing_and_backspace_round_trip() {
        let mut e = Editor::new();
        for c in "hello".chars() {
            e.insert_char(c);
        }
        assert_eq!(e.text(), "hello");
        assert_eq!(e.cursor(), (0, 5));
        assert!(e.backspace());
        assert_eq!(e.text(), "hell");
    }

    #[test]
    fn backspace_at_start_of_line_joins_lines() {
        let mut e = Editor::from_text("ab\ncd");
        // 光标在 (1,2) → 移到第 1 行行首 (1,0)
        e.move_home();
        assert_eq!(e.cursor(), (1, 0));
        assert!(e.backspace(), "非首行行首退格应把两行合回");
        assert_eq!(e.text(), "abcd");
        assert_eq!(e.cursor(), (0, 2), "光标应落到接合处");
    }

    #[test]
    fn backspace_at_very_start_reports_nothing_deleted() {
        // 首行行首没有可删内容 → 必须返回 false，让调用方知道"退格无效果"
        // （UI 据此决定是否要转而关闭弹窗等）
        let mut e = Editor::new();
        assert!(!e.backspace());
        let mut e = Editor::from_text("ab");
        e.move_home();
        assert!(!e.backspace());
        assert_eq!(e.text(), "ab");
    }

    #[test]
    fn newline_splits_and_enter_joins() {
        let mut e = Editor::from_text("abcd");
        e.move_home();
        e.move_right();
        e.move_right();
        e.newline();
        assert_eq!(e.text(), "ab\ncd");
        assert_eq!(e.cursor(), (1, 0));
        assert!(e.backspace());
        assert_eq!(e.text(), "abcd", "行首退格应把两行合回");
    }

    #[test]
    fn ctrl_k_deletes_to_end_and_then_joins_next_line() {
        // from_text 后光标在末行末尾 → 先移到首行再定位到 col 2
        let mut e = Editor::from_text("abcdef\nghi");
        e.move_up();
        e.move_home();
        e.move_right();
        e.move_right();
        assert_eq!(e.cursor(), (0, 2));
        e.delete_to_line_end();
        assert_eq!(e.text(), "ab\nghi", "应删掉本行 col 2 之后的内容");
        // 已在行尾，再按应并入下一行
        e.delete_to_line_end();
        assert_eq!(e.text(), "abghi");
    }

    #[test]
    fn ctrl_u_deletes_to_line_start() {
        let mut e = Editor::from_text("hello world");
        e.move_home();
        for _ in 0..6 {
            e.move_right();
        }
        e.delete_to_line_start();
        assert_eq!(e.text(), "world");
        assert_eq!(e.cursor(), (0, 0));
    }

    #[test]
    fn ctrl_w_deletes_a_word_including_trailing_space() {
        let mut e = Editor::from_text("foo bar baz");
        e.delete_word_backward();
        assert_eq!(e.text(), "foo bar ");
        e.delete_word_backward();
        assert_eq!(e.text(), "foo ");
    }

    #[test]
    fn word_movement_skips_whitespace() {
        let mut e = Editor::from_text("foo bar");
        e.word_backward();
        assert_eq!(e.cursor(), (0, 4), "应停在 bar 的词首");
        e.word_forward();
        assert_eq!(e.cursor(), (0, 7));
    }

    #[test]
    fn arrows_clamp_and_cross_lines() {
        let mut e = Editor::from_text("ab\ncdef");
        // 从末尾 (1,4) 上移到 (0,2)（列夹到较短行长度）
        e.move_up();
        assert_eq!(e.cursor(), (0, 2));
        e.move_down();
        assert_eq!(e.cursor(), (1, 2));
        e.move_home();
        e.move_left();
        assert_eq!(e.cursor(), (0, 2), "跨行左移应到上一行末尾");
        e.move_end();
        e.move_right();
        assert_eq!(e.cursor(), (1, 0), "跨行右移应到下一行行首");
    }

    #[test]
    fn delete_forward_joins_lines_at_line_end() {
        let mut e = Editor::from_text("ab\ncd");
        e.move_up();
        e.move_end();
        assert!(e.delete_forward());
        assert_eq!(e.text(), "abcd");
    }

    #[test]
    fn multibyte_editing_never_panics_and_keeps_content() {
        // 关键契约：中文/emoji 下所有编辑都不得 panic，且不丢字符。
        // 若把字节偏移当字符下标用，这里会 panic 或切出半个字符。
        let mut e = Editor::from_text("中文测试🚀ok");
        e.move_home();
        e.move_right();
        e.move_right();
        // col=2 表示"中文"之后（字符下标，不是字节）
        assert_eq!(e.cursor(), (0, 2));
        e.insert_char('新');
        assert_eq!(e.text(), "中文新测试🚀ok");
        assert!(e.backspace());
        assert_eq!(e.text(), "中文测试🚀ok");
        // 词边界按**空白**划分（与 shell/readline 一致）：
        // `中文测试🚀ok` 中间没有空白 → 整段算一个词，ctrl+w 会全删。
        // 中文不按单字拆分：这是刻意的取舍（见模块文档的边界说明）。
        e.move_end();
        e.delete_word_backward();
        assert!(e.is_empty(), "无空白分隔时整段是一个词");

        // 有空白时按词删
        e.set("中文测试 ok");
        e.delete_word_backward();
        assert_eq!(e.text(), "中文测试 ");
    }

    #[test]
    fn cursor_column_is_character_count_not_bytes() {
        let mut e = Editor::from_text("中文");
        assert_eq!(e.cursor(), (0, 2), "两个汉字 → 列号应为 2 而不是 6");
        e.move_home();
        e.move_right();
        assert_eq!(e.cursor(), (0, 1));
        e.insert_char('X');
        assert_eq!(e.text(), "中X文");
    }

    #[test]
    fn paste_with_newlines_splits_into_lines() {
        let mut e = Editor::new();
        e.insert_str("line1\nline2\nline3");
        assert_eq!(e.line_count(), 3);
        assert_eq!(e.text(), "line1\nline2\nline3");
        assert_eq!(e.cursor(), (2, 5));
    }

    #[test]
    fn paste_in_the_middle_keeps_both_sides() {
        let mut e = Editor::from_text("HEADTAIL");
        e.move_home();
        for _ in 0..4 {
            e.move_right();
        }
        e.insert_str("A\nB");
        assert_eq!(e.text(), "HEADA\nBTAIL");
    }

    #[test]
    fn undo_and_redo_restore_text() {
        let mut e = Editor::new();
        e.insert_char('a');
        e.insert_char('b');
        let after = e.text();
        // 连续打字算一个撤销单元 → 一次撤销应回到空
        e.undo();
        assert!(e.text() != after);
        e.redo();
        assert_eq!(e.text(), after);
    }

    #[test]
    fn undo_stack_is_bounded() {
        // 内存有界：撤销栈不能随输入无限增长
        let mut e = Editor::new();
        for i in 0..(MAX_UNDO * 3) {
            // 每次插入都断开"连续打字"合并，确保产生独立的撤销点
            e.insert_str(&format!("{i} "));
        }
        assert!(e.undo.len() <= MAX_UNDO, "撤销栈 {} 超过上限", e.undo.len());
    }

    #[test]
    fn new_edit_clears_redo_stack() {
        let mut e = Editor::new();
        e.insert_str("one");
        e.undo();
        e.insert_str("two");
        e.redo(); // 重做栈已作废，不该有变化
        assert_eq!(e.text(), "two");
    }

    #[test]
    fn set_replaces_content_and_puts_cursor_at_end() {
        let mut e = Editor::new();
        e.set("/theme");
        assert_eq!(e.text(), "/theme");
        assert_eq!(e.cursor(), (0, 6));
        // 多行
        e.set("a\nb\nc");
        assert_eq!(e.line_count(), 3);
        assert_eq!(e.cursor(), (2, 1));
    }

    #[test]
    fn clear_only_records_undo_when_there_was_content() {
        let mut e = Editor::new();
        e.clear(); // 本来就空 → 不该产生撤销点
        assert!(e.undo.is_empty());
        e.insert_str("x");
        e.clear();
        assert!(e.is_empty());
        e.undo();
        assert_eq!(e.text(), "x", "清空应可撤销");
    }

    #[test]
    fn text_and_lines_agree() {
        let e = Editor::from_text("a\nbb\nccc");
        assert_eq!(e.lines().len(), 3);
        assert_eq!(e.text(), e.lines().join("\n"));
        for (i, l) in e.lines().iter().enumerate() {
            assert_eq!(l, e.text().lines().nth(i).unwrap());
        }
    }

    #[test]
    fn empty_editor_operations_are_safe() {
        // 空编辑器上任何操作都不该 panic
        let mut e = Editor::new();
        e.backspace();
        e.delete_forward();
        e.delete_to_line_end();
        e.delete_to_line_start();
        e.delete_word_backward();
        e.move_left();
        e.move_right();
        e.move_up();
        e.move_down();
        e.word_backward();
        e.word_forward();
        e.undo();
        e.redo();
        assert!(e.is_empty());
    }

    #[test]
    fn cursor_stays_valid_after_undo_shrinks_text() {
        // 撤销会让文本变短；光标若越界，后续插入会 panic
        let mut e = Editor::from_text("aaaaaaaaaa\nbbbbbbbbbb");
        e.insert_str("很长的一段追加");
        assert!(e.cursor().0 < e.line_count());
        e.undo();
        assert!(e.cursor().0 < e.line_count());
        assert!(e.cursor().1 <= e.lines()[e.cursor().0].chars().count());
        e.insert_char('x'); // 不得 panic
    }
}
