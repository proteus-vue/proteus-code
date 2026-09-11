//! 输入辅助：命令历史、模糊匹配、文件候选。
//!
//! 三块都抽成**纯函数**，因为它们各有易错的边界，而 TUI 交互一旦错了
//! 很难靠肉眼定位（历史串位、模糊匹配返回无关结果、候选越界）。
//!
//! # 设计立场
//!
//! 这些是**宿主的输入便利**，不是业务：它们不碰内核，只影响"下一个 Op 长什么样"。
//! 因此全部放 L5，不进 L2/L3。

/// 命令历史。去重相邻项、有上限，避免长会话无限增长。
pub struct History {
    entries: Vec<String>,
    /// 浏览游标：None 表示"在最新之后"（正在输入新内容）
    cursor: Option<usize>,
    max: usize,
}

impl History {
    pub fn new(max: usize) -> Self {
        Self { entries: Vec::new(), cursor: None, max }
    }

    /// 记录一次提交。空串不记；与上一条相同不重复记（连续重复输入很常见）。
    pub fn push(&mut self, line: &str) {
        let line = line.trim();
        if line.is_empty() {
            return;
        }
        if self.entries.last().map(|e| e.as_str()) == Some(line) {
            self.cursor = None;
            return;
        }
        self.entries.push(line.to_string());
        if self.entries.len() > self.max {
            self.entries.remove(0);
        }
        self.cursor = None;
    }

    pub fn entries(&self) -> &[String] { &self.entries }
    pub fn is_empty(&self) -> bool { self.entries.is_empty() }

    /// 向上浏览（更早的一条）。到顶后停住（不循环 —— 循环会让人分不清首尾）。
    pub fn prev(&mut self) -> Option<&str> {
        if self.entries.is_empty() {
            return None;
        }
        let next = match self.cursor {
            None => self.entries.len() - 1,
            Some(0) => 0,
            Some(i) => i - 1,
        };
        self.cursor = Some(next);
        self.entries.get(next).map(String::as_str)
    }

    /// 向下浏览（更近的一条）。越过最新则回到 None（空输入行）。
    pub fn next_entry(&mut self) -> Option<&str> {
        let Some(i) = self.cursor else { return None };
        if i + 1 >= self.entries.len() {
            self.cursor = None;
            return None;
        }
        self.cursor = Some(i + 1);
        self.entries.get(i + 1).map(String::as_str)
    }

    /// 重置浏览游标（用户开始输入新内容时调用）。
    pub fn reset_cursor(&mut self) { self.cursor = None; }

    /// 反向搜索（Ctrl+R 的核心）：从最新往回找**包含** `needle` 的条目。
    /// 大小写不敏感 —— 命令行历史里大小写常常记不清。
    pub fn search(&self, needle: &str) -> Option<&str> {
        if needle.is_empty() {
            return self.entries.last().map(String::as_str);
        }
        let n = needle.to_lowercase();
        self.entries.iter().rev().find(|e| e.to_lowercase().contains(&n)).map(String::as_str)
    }
}

impl Default for History {
    fn default() -> Self { Self::new(200) }
}

/// 模糊匹配：`query` 是否是 `candidate` 的**子序列**（大小写不敏感）。
///
/// 返回评分（越小越好），评分规则刻意简单且可解释：
/// - 优先**连续匹配**（`ab` 匹配 `abc` 优于 `a_b_c`）
/// - 优先**靠前匹配**（同分时更短的候选/更早的位置胜出）
/// - 优先**词边界起始**（`ho` 匹配 `home.ts` 优于 `shopping.rs`）
pub fn fuzzy_score(query: &str, candidate: &str) -> Option<u32> {
    if query.is_empty() {
        return Some(0);
    }
    let q: Vec<char> = query.to_lowercase().chars().collect();
    let c: Vec<char> = candidate.to_lowercase().chars().collect();
    // 原字符串用于判定词边界（大小写不影响边界）
    let orig: Vec<char> = candidate.chars().collect();

    let mut qi = 0;
    let mut score = 0u32;
    let mut last_match: Option<usize> = None;

    for (ci, &ch) in c.iter().enumerate() {
        if qi >= q.len() {
            break;
        }
        if ch == q[qi] {
            // 连续匹配给低惩罚
            if let Some(lm) = last_match {
                if ci == lm + 1 {
                    score += 1;
                } else {
                    score += 3 + (ci - lm) as u32;
                }
            } else {
                // 首次匹配：越靠前越好
                score += ci as u32;
                // 词边界起始（开头、/ _ - . 之后）额外优惠
                let boundary = ci == 0
                    || matches!(orig.get(ci.wrapping_sub(1)), Some('/') | Some('_') | Some('-') | Some('.'));
                if boundary {
                    score = score.saturating_sub(2);
                }
            }
            last_match = Some(ci);
            qi += 1;
        }
    }

    if qi == q.len() {
        // 完全匹配的子序列才有效
        Some(score)
    } else {
        None
    }
}

/// 在一组候选中按模糊分数排序，取前 `limit` 个。
pub fn fuzzy_rank<'a>(query: &str, candidates: &'a [String], limit: usize) -> Vec<&'a String> {
    let mut scored: Vec<(u32, &String)> = candidates
        .iter()
        .filter_map(|c| fuzzy_score(query, c).map(|s| (s, c)))
        .collect();
    // 同分时按字典序，保证结果**稳定可复现**（否则每次刷新候选顺序都在跳）
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
    scored.into_iter().take(limit).map(|(_, c)| c).collect()
}

/// 列出工作区内的文件候选（供 `@` 搜索）。
///
/// 两个上限都是必须的：工作区可能有几十万文件，无界遍历会让 TUI 卡死。
/// 达到上限即停止并**如实标记**（`truncated`），上层据此提示用户。
pub fn list_files(root: &std::path::Path, max_files: usize, max_depth: usize) -> (Vec<String>, bool) {
    let mut out = Vec::new();
    let mut truncated = false;
    // 目录名黑名单：这些目录体量大且几乎不会是用户想引用的
    const SKIP: &[&str] = &[
        ".git", "node_modules", "target", "dist", "build", ".venv", "__pycache__", ".next", ".cache",
    ];

    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > max_depth {
            truncated = true;
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            if out.len() >= max_files {
                truncated = true;
                break;
            }
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                if SKIP.contains(&name.as_str()) || name.starts_with('.') {
                    continue;
                }
                stack.push((path, depth + 1));
            } else if ft.is_file() {
                // 存相对路径：候选列表要短且可读
                if let Ok(rel) = path.strip_prefix(root) {
                    out.push(rel.to_string_lossy().to_string());
                }
            }
        }
    }
    // 排序保证候选顺序稳定（read_dir 顺序依赖文件系统，不可复现）
    out.sort();
    (out, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_records_and_dedups_adjacent() {
        let mut h = History::new(10);
        h.push("one");
        h.push("one"); // 相邻重复不记
        h.push("two");
        assert_eq!(h.entries(), &["one", "two"]);
        h.push(""); // 空串不记
        assert_eq!(h.entries().len(), 2);
    }

    #[test]
    fn history_bounds_its_length() {
        let mut h = History::new(3);
        for i in 0..5 {
            h.push(&format!("cmd{i}"));
        }
        assert_eq!(h.entries(), &["cmd2", "cmd3", "cmd4"], "应丢弃最旧的");
    }

    #[test]
    fn history_navigates_up_and_down_without_wrapping() {
        let mut h = History::new(10);
        h.push("first");
        h.push("second");

        assert_eq!(h.prev(), Some("second"));
        assert_eq!(h.prev(), Some("first"));
        assert_eq!(h.prev(), Some("first"), "到顶后停住，不循环");
        assert_eq!(h.next_entry(), Some("second"));
        assert_eq!(h.next_entry(), None, "越过最新回到空输入");
    }

    #[test]
    fn history_reverse_search_is_case_insensitive_and_newest_first() {
        let mut h = History::new(10);
        h.push("cargo test --lib");
        h.push("git status");
        h.push("cargo build");

        assert_eq!(h.search("cargo"), Some("cargo build"), "应从最新往回找");
        assert_eq!(h.search("CARGO"), Some("cargo build"), "大小写不敏感");
        assert_eq!(h.search("status"), Some("git status"));
        assert_eq!(h.search("nope"), None);
        assert_eq!(h.search(""), Some("cargo build"), "空查询给最新一条");
    }

    #[test]
    fn fuzzy_requires_a_subsequence() {
        assert!(fuzzy_score("ho", "home.ts").is_some());
        assert!(fuzzy_score("hmt", "home.ts").is_some(), "子序列即可");
        assert!(fuzzy_score("xyz", "home.ts").is_none(), "非子序列应拒绝");
        assert!(fuzzy_score("", "anything").is_some(), "空查询匹配一切");
    }

    #[test]
    fn fuzzy_prefers_contiguous_and_early_matches() {
        let contiguous = fuzzy_score("ab", "abc").unwrap();
        let scattered = fuzzy_score("ab", "axxxxxb").unwrap();
        assert!(contiguous < scattered, "连续匹配应得分更优");

        let early = fuzzy_score("a", "abc").unwrap();
        let late = fuzzy_score("a", "xxxxa").unwrap();
        assert!(early < late, "靠前匹配应更优");
    }

    #[test]
    fn fuzzy_prefers_word_boundary_starts() {
        let boundary = fuzzy_score("ho", "home.ts").unwrap();
        let middle = fuzzy_score("ho", "shopping.rs").unwrap();
        assert!(boundary < middle, "词边界起始应更优");
    }

    #[test]
    fn fuzzy_rank_is_stable_and_limited() {
        let cands = vec![
            "src/home.ts".to_string(),
            "src/shopping.rs".to_string(),
            "README.md".to_string(),
            "src/host.rs".to_string(),
        ];
        let r = fuzzy_rank("ho", &cands, 2);
        assert_eq!(r.len(), 2, "应受 limit 约束");
        // 同分时按字典序 → 结果稳定
        let again = fuzzy_rank("ho", &cands, 2);
        assert_eq!(r, again, "排序必须可复现，否则候选列表会跳动");
        assert!(r.iter().any(|c| c.contains("home") || c.contains("host")), "相关项应入选：{r:?}");
    }

    #[test]
    fn list_files_skips_noise_dirs_and_reports_truncation() {
        let root = std::env::temp_dir().join(format!("neo-listfiles-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join("src/a.rs"), "// a").unwrap();
        std::fs::write(root.join("src/b.rs"), "// b").unwrap();
        std::fs::write(root.join("node_modules/pkg/index.js"), "// noise").unwrap();
        std::fs::write(root.join(".git/config"), "noise").unwrap();
        std::fs::write(root.join("README.md"), "# r").unwrap();

        let (files, truncated) = list_files(&root, 100, 5);
        assert!(!truncated, "文件很少，不应标记截断");
        assert!(files.contains(&"src/a.rs".to_string()));
        assert!(files.contains(&"README.md".to_string()));
        assert!(
            !files.iter().any(|f| f.contains("node_modules") || f.contains(".git")),
            "噪声目录应被跳过：{files:?}"
        );
        // 顺序稳定（已排序）
        let mut sorted = files.clone();
        sorted.sort();
        assert_eq!(files, sorted, "候选必须有序，否则每次刷新顺序都在变");

        // 上限生效时如实标记
        let (few, cut) = list_files(&root, 1, 5);
        assert_eq!(few.len(), 1);
        assert!(cut, "达到上限必须标记 truncated");

        let _ = std::fs::remove_dir_all(&root);
    }
}
