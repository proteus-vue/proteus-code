//! 转录视图状态：滚动位置与搜索
//!
//! # 为什么"自动滚到底"不够
//!
//! 之前只显示末尾 N 行。长会话里想回看前面的内容毫无办法 —— 这是阅读
//! 长对话的硬需求（opencode 有 PageUp/PageDown、逐行、半页、跳首尾等一整套键）。
//!
//! # 关键语义：跟随底部（follow）
//!
//! 新内容到达时该不该自动滚到底？两种做法都错：
//!   - 永远滚到底 → 用户往上翻看时被新内容**拽回底部**，没法读；
//!   - 永不滚到底 → 用户得手动追新内容。
//! 正确做法是引入 `follow` 标志：在底部时跟随新内容；一旦用户主动上翻就
//! 停止跟随，直到底部再次可见（或按"跳到最新"）。

/// 转录视图状态。
#[derive(Debug, Clone, Default)]
pub struct View {
    /// 从**底部**往上偏移多少行。0 = 贴底（显示最新）。
    offset: usize,
    /// 是否跟随新内容（在底部时为真）。
    follow: bool,
    /// 搜索状态：`Some` 表示正在搜索（含当前查询与命中位置）。
    search: Option<Search>,
}

/// 搜索状态。
#[derive(Debug, Clone)]
pub struct Search {
    /// 查询串（子串匹配，大小写不敏感）
    pub query: String,
    /// 命中的行号（升序）
    pub hits: Vec<usize>,
    /// 当前在第几个命中（用于"下一个/上一个"循环）
    pub current: usize,
}

impl Default for Search {
    fn default() -> Self {
        Self { query: String::new(), hits: Vec::new(), current: 0 }
    }
}

impl View {
    pub fn new() -> Self {
        Self { offset: 0, follow: true, search: None }
    }

    /// 是否跟随底部。渲染层据此决定"内容不足时贴底还是贴顶"。
    pub fn follow(&self) -> bool {
        self.follow
    }

    pub fn offset(&self) -> usize {
        self.offset
    }

    pub fn search(&self) -> Option<&Search> {
        self.search.as_ref()
    }

    /// 滚到最新（偏移归零并恢复跟随）。
    pub fn to_bottom(&mut self) {
        self.offset = 0;
        self.follow = true;
    }

    /// 滚到最顶。
    pub fn to_top(&mut self, total: usize, viewport: usize) {
        self.offset = total.saturating_sub(viewport);
        self.follow = false;
    }

    /// 向上滚 N 行（看更早的内容）。
    pub fn scroll_up(&mut self, n: usize, total: usize, viewport: usize) {
        let max = total.saturating_sub(viewport);
        self.offset = (self.offset + n).min(max);
        // 只有真的离开底部才停止跟随
        self.follow = self.offset == 0;
    }

    /// 向下滚 N 行（看更新的内容）。到底则恢复跟随。
    pub fn scroll_down(&mut self, n: usize, _total: usize, _viewport: usize) {
        self.offset = self.offset.saturating_sub(n);
        self.follow = self.offset == 0;
    }

    // ── 搜索 ─────────────────────────────────────────────────────────

    /// 更新查询并重算命中。`current` 重置到**最近一条命中**（往回看更常见）。
    pub fn set_search(&mut self, lines: &[String], query: &str) {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            self.search = None;
            return;
        }
        let hits: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect();
        // 默认定位到最后一个命中（最靠近当前的对话）
        let current = hits.len().saturating_sub(1);
        self.search = Some(Search { query: q, hits, current });
    }

    /// 跳到下一个 / 上一个命中（**循环**，与多数编辑器的查找一致）。
    pub fn search_step(&mut self, forward: bool, total_lines: usize, viewport: usize) {
        let Some(s) = self.search.as_mut() else {
            return;
        };
        if s.hits.is_empty() {
            return;
        }
        let n = s.hits.len();
        s.current = if forward { (s.current + 1) % n } else { (s.current + n - 1) % n };
        let line = s.hits[s.current];
        // 把命中行滚进视野（略偏上，留上下文）
        let bottom_start = total_lines.saturating_sub(viewport);
        if line < bottom_start {
            self.offset = (total_lines - viewport_safe(line, viewport)).min(total_lines);
        } else {
            self.offset = total_lines.saturating_sub(viewport).saturating_sub(0);
        }
        // 直接据命中行算出偏移：让命中行位于视口 1/3 处
        let target_top = line.saturating_sub(viewport / 3);
        self.offset = (total_lines.saturating_sub(viewport)).saturating_sub(target_top);
        self.follow = self.offset == 0;
    }

    /// 关闭搜索。
    pub fn clear_search(&mut self) {
        self.search = None;
    }

    /// 把偏移夹到当前内容允许的范围内。
    ///
    /// **必须夹**：滚动时的 total 来自"估算行数"，渲染时的 total 来自
    /// "实际渲染行数"，两者可能不一致（Markdown 换行数难以精确预估）。
    /// 不夹的话偏移可能指到内容之外，视口算出 `start == end` → **整屏空白**，
    /// 用户会觉得程序挂了。这里以渲染时的 total 为准做最后一道夹取。
    pub fn clamped_offset(&self, total: usize, viewport: usize) -> usize {
        self.offset.min(total.saturating_sub(viewport))
    }

    /// 视口起始行号（供渲染）。
    ///
    /// `total` = 总行数，`viewport` = 可视行数。返回 `(start, end)`。
    pub fn window(&self, total: usize, viewport: usize) -> (usize, usize) {
        let offset = self.clamped_offset(total, viewport);
        let end = total.saturating_sub(offset);
        let start = end.saturating_sub(viewport);
        (start, end)
    }
}

fn viewport_safe(line: usize, viewport: usize) -> usize {
    viewport.max(1).min(line.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("line{i}")).collect()
    }

    #[test]
    fn starts_following_the_bottom() {
        let v = View::new();
        assert!(v.follow());
        assert_eq!(v.offset(), 0);
        // 视口应贴底
        let (s, e) = v.window(100, 10);
        assert_eq!((s, e), (90, 100));
    }

    #[test]
    fn scrolling_up_stops_following() {
        // 关键语义：用户上翻后**不再**被新内容拽回底部
        let mut v = View::new();
        v.scroll_up(5, 100, 10);
        assert_eq!(v.offset(), 5);
        assert!(!v.follow(), "上翻后应停止跟随");
    }

    #[test]
    fn scrolling_back_to_bottom_resumes_following() {
        let mut v = View::new();
        v.scroll_up(5, 100, 10);
        v.scroll_down(5, 100, 10);
        assert_eq!(v.offset(), 0);
        assert!(v.follow(), "回到底部应恢复跟随");
    }

    #[test]
    fn scroll_up_cannot_go_past_the_first_line() {
        let mut v = View::new();
        v.scroll_up(9999, 100, 10);
        assert_eq!(v.offset(), 90, "偏移上限 = 总行数 - 视口");
        let (s, _) = v.window(100, 10);
        assert_eq!(s, 0);
    }

    #[test]
    fn scrolling_when_content_fits_does_nothing() {
        // 内容比视口短：没有可滚的，偏移必须保持 0（否则会出现空白）
        let mut v = View::new();
        v.scroll_up(10, 3, 10);
        assert_eq!(v.offset(), 0);
        assert!(v.follow());
    }

    #[test]
    fn to_top_shows_the_first_line() {
        let mut v = View::new();
        v.to_top(100, 10);
        let (s, e) = v.window(100, 10);
        assert_eq!(s, 0);
        assert_eq!(e, 10);
        assert!(!v.follow());
    }

    #[test]
    fn search_finds_all_case_insensitive_matches() {
        let mut v = View::new();
        let ls: Vec<String> =
            vec!["Hello".into(), "world".into(), "HELLO again".into(), "other".into()];
        v.set_search(&ls, "hello");
        let s = v.search().expect("应进入搜索状态");
        assert_eq!(s.hits, vec![0, 2]);
        assert_eq!(s.current, 1, "默认定位到最后一个命中（最靠近当前对话）");
    }

    #[test]
    fn empty_query_exits_search_mode() {
        let mut v = View::new();
        v.set_search(&lines(5), "line");
        assert!(v.search().is_some());
        v.set_search(&lines(5), "   ");
        assert!(v.search().is_none(), "空白查询应退出搜索");
    }

    #[test]
    fn search_with_no_match_is_reported_not_silently_empty() {
        // 无命中时仍应处于搜索态（界面要能显示"无匹配"），而不是当作没搜索
        let mut v = View::new();
        v.set_search(&lines(5), "找不到的词");
        let s = v.search().expect("应仍是搜索态");
        assert!(s.hits.is_empty(), "应如实报告 0 个命中");
    }

    #[test]
    fn search_step_cycles_forward_and_backward() {
        let ls = lines(20);
        let mut v = View::new();
        v.set_search(&ls, "line1"); // 命中 line1, line10..line19 → 多个
        let n = v.search().unwrap().hits.len();
        assert!(n > 1);
        let first = v.search().unwrap().current;
        v.search_step(true, ls.len(), 10);
        assert_eq!(v.search().unwrap().current, (first + 1) % n, "前进应循环");
        v.search_step(false, ls.len(), 10);
        assert_eq!(v.search().unwrap().current, first, "后退应回到原处");
    }

    #[test]
    fn search_step_with_no_hits_is_safe() {
        let ls = lines(5);
        let mut v = View::new();
        v.set_search(&ls, "zzz");
        v.search_step(true, ls.len(), 3);
        v.search_step(false, ls.len(), 3);
        assert!(v.search().unwrap().hits.is_empty());
    }

    #[test]
    fn search_brings_the_hit_into_view() {
        // 命中行若不在视口内，必须滚过去 —— 否则"找到了"却看不见
        let ls = lines(200);
        let mut v = View::new();
        v.set_search(&ls, "line5");
        v.search_step(false, ls.len(), 10); // 往回找更早的命中
        let hit = v.search().unwrap().hits[v.search().unwrap().current];
        let (s, e) = v.window(ls.len(), 10);
        assert!(s <= hit && hit < e, "命中行 {hit} 应在视口 [{s},{e}) 内");
    }

    #[test]
    fn window_never_overflows_bounds() {
        let mut v = View::new();
        for offset_press in [0usize, 1, 5, 50, 500] {
            v.scroll_up(offset_press, 100, 10);
            let (s, e) = v.window(100, 10);
            assert!(e <= 100, "end {e} 越界");
            assert!(s <= e, "start {s} > end {e}");
        }
    }

    #[test]
    fn clear_search_leaves_scroll_position_alone() {
        // 退出搜索不该打乱用户当前的阅读位置
        let ls = lines(100);
        let mut v = View::new();
        v.scroll_up(7, ls.len(), 10);
        v.set_search(&ls, "line1");
        v.clear_search();
        assert!(v.search().is_none());
        assert_eq!(v.offset(), 7, "退出搜索后滚动位置应保持");
    }
}
