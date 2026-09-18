//! 文件树的**层级推导**：把扁平路径列表变成可折叠的树行。
//!
//! # 它解决什么
//!
//! 索引给的是一串扁平相对路径（`crates/neo-platform/src/file_index.rs`）。
//! 直接列表显示的问题在真实仓库上很明显：本仓 342 个文件 → 一堵**路径墙**，
//! 而且每一行都重复了长前缀（`crates/neo-platform/src/`），横向空间几乎全花在
//! 重复信息上。Zed / VS Code / ZCode 都用**缩进树 + 可折叠**表达层级。
//!
//! # 三条必须做对的语义
//!
//! 1. **默认全部折叠**（只显示根级条目）。这是本模块存在的直接理由 ——
//!    若默认全展开，342 个文件仍然是一堵墙（只是换了缩进），等于没做。
//!    折叠后根级通常只有十来个条目，一眼能扫。
//! 2. **只显示"确实含文件"的目录**。索引里若出现空目录（理论上不会，但扫描
//!    与展示之间可能有竞态），不该在树上多出一个点不开的节点 ——
//!    目录集合**从文件路径派生**，不额外收一份数据（避免两个来源不一致）。
//! 3. **目录排在文件前**（同级内），各自按名字排序。这是文件管理器的通行约定，
//!    也是"先看有什么模块、再看根目录散落的文件"的阅读顺序。
//!
//! # 为什么是纯逻辑
//!
//! "展开哪些目录 → 显示哪些行"是**纯计算**，与渲染无关，所以放行为层：
//! - 两个宿主（将来 TUI 也用）共用同一套层级推导，否则同一份文件列表在
//!   两处长得不一样；
//! - 它可以逐条断言（空输入、深嵌套、折叠态、排序、重复路径）。
//!
//! # ⚠️ 展开状态按**路径**记，不按下标
//!
//! 与 `expanded_diff_folds` / `collapsed_reasoning` 同一教训，但更严重：
//! 文件列表**会被重扫**（用户点"刷新"、将来接文件监听）—— 下标会整批错位，
//! 于是"我展开的目录突然变成别的目录"。路径是稳定的，所以用它做键。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// 树里一行的种类。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowKind {
    /// 目录（可展开/折叠）。
    Dir {
        /// 直接子项数（展开后立即能看到几行）。
        ///
        /// 给**直接子项数**而不是"递归文件数"：前者与"点开一下会多出几行"
        /// 严格对应，用户能据此预期；后者（含深层）在折叠时容易与视觉不符。
        child_count: usize,
        /// 当前是否展开（渲染层据此画 ▸/▾）。
        expanded: bool,
    },
    /// 文件。
    File,
}

/// 树里的一行（**显示行**，不是索引里的一行）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    /// 相对路径（文件或目录）。
    pub path: PathBuf,
    /// 显示名（**只是名字**，不含父路径 —— 层级由缩进表达）。
    pub name: String,
    /// 缩进层级（根级条目为 0）。
    pub depth: usize,
    pub kind: RowKind,
}

/// 取显示名（最后一段）。路径为空时返回空串。
fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// 把扁平文件列表推导成**树行**。
///
/// - `files`：相对路径（与 `FileIndex::files` 同形，可含子目录）。
/// - `expanded`：已展开的**目录路径**集合。
///
/// 返回的行已按显示顺序排好（同级内目录在前、各自按名字升序）。
pub fn tree_rows(files: &[PathBuf], expanded: &BTreeSet<PathBuf>) -> Vec<TreeRow> {
    // 0) **去重**：索引理论上不会给重复路径（目录遍历每个文件只出现一次），
    //    但展示层不该因此把同一个文件画两遍 —— 那是"看起来像有两个文件"
    //    的假象，而假象的代价比一次去重高得多。
    let uniq: BTreeSet<&PathBuf> = files.iter().collect();

    // 1) 从文件路径**派生**目录集合（不额外收一份，避免两个来源不一致）。
    //
    // 上溯时命中已插入的目录就停：插入一个目录时它的祖先一定也已插入
    //（下面的循环是逐级上溯的），所以这个提前退出是**正确**的，不是启发式。
    let mut dirs: BTreeSet<PathBuf> = BTreeSet::new();
    for f in &uniq {
        let mut cur = f.parent();
        while let Some(d) = cur {
            if d.as_os_str().is_empty() {
                break; // 根（空路径不代表一个目录条目）
            }
            if !dirs.insert(d.to_path_buf()) {
                break;
            }
            cur = d.parent();
        }
    }

    // 2) 建父子索引：父路径 → 子项（目录与文件混在一起，稍后排序区分）。
    let mut children: BTreeMap<PathBuf, Vec<(PathBuf, bool)>> = BTreeMap::new();
    for d in &dirs {
        let parent = d.parent().unwrap_or(Path::new("")).to_path_buf();
        children.entry(parent).or_default().push((d.clone(), true));
    }
    for f in &uniq {
        let parent = f.parent().unwrap_or(Path::new("")).to_path_buf();
        children
            .entry(parent)
            .or_default()
            .push(((*f).clone(), false));
    }

    // 3) 同级排序：**目录在前**，再按名字。
    //
    // 用名字（而非完整路径）比较：同级的父路径相同，比完整路径等价 ——
    // 但用名字更直白地表达"这一层里谁在前"。
    for v in children.values_mut() {
        v.sort_by(|a, b| {
            b.1.cmp(&a.1) // true（目录）排前面
                .then_with(|| name_of(&a.0).cmp(&name_of(&b.0)))
        });
    }

    // 4) 深度优先展开：只在目录**已展开**时才下探。
    let mut out = Vec::new();
    push_children(Path::new(""), 0, &children, expanded, &mut out);
    out
}

/// 递归地把 `parent` 的子项推进 `out`（目录已展开时继续下探）。
fn push_children(
    parent: &Path,
    depth: usize,
    children: &BTreeMap<PathBuf, Vec<(PathBuf, bool)>>,
    expanded: &BTreeSet<PathBuf>,
    out: &mut Vec<TreeRow>,
) {
    let Some(kids) = children.get(parent) else {
        return;
    };
    for (path, is_dir) in kids {
        if *is_dir {
            let child_count = children.get(path).map(|v| v.len()).unwrap_or(0);
            let is_expanded = expanded.contains(path);
            out.push(TreeRow {
                path: path.clone(),
                name: name_of(path),
                depth,
                kind: RowKind::Dir { child_count, expanded: is_expanded },
            });
            if is_expanded {
                push_children(path, depth + 1, children, expanded, out);
            }
        } else {
            out.push(TreeRow {
                path: path.clone(),
                name: name_of(path),
                depth,
                kind: RowKind::File,
            });
        }
    }
}

/// 全部目录路径（供"展开全部"用）。
pub fn all_dirs(files: &[PathBuf]) -> BTreeSet<PathBuf> {
    let mut dirs: BTreeSet<PathBuf> = BTreeSet::new();
    for f in files {
        let mut cur = f.parent();
        while let Some(d) = cur {
            if d.as_os_str().is_empty() {
                break;
            }
            if !dirs.insert(d.to_path_buf()) {
                break;
            }
            cur = d.parent();
        }
    }
    dirs
}

/// 一个文件/目录的**所有祖先目录**（供"展开到它"用）。
///
/// 返回从根到直接父的顺序；根级条目返回空集。
pub fn ancestors_of(path: &Path) -> BTreeSet<PathBuf> {
    let mut out = BTreeSet::new();
    let mut cur = path.parent();
    while let Some(d) = cur {
        if d.as_os_str().is_empty() {
            break;
        }
        out.insert(d.to_path_buf());
        cur = d.parent();
    }
    out
}

/// 缩进上限（视觉层级）。
///
/// # 为什么需要上限
///
/// 极深的路径（生成物、嵌套很深的工程）会让文本被推到面板右边界外 ——
/// 那时名字反而看不见了（**缩进挤掉了内容**）。超过上限后不再增加缩进，
/// 与 Zed 的做法一致。
///
/// 取 8：按每级 12px 算，最深一档是 96px，在 280px 宽的列表栏里仍留有
/// 看清名字的空间。
pub const MAX_INDENT_DEPTH: usize = 8;

/// 缩进的实际层级（受 [`MAX_INDENT_DEPTH`] 限制）。
pub fn indent_level(depth: usize) -> usize {
    depth.min(MAX_INDENT_DEPTH)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(list: &[&str]) -> Vec<PathBuf> {
        list.iter().map(PathBuf::from).collect()
    }

    fn names(rows: &[TreeRow]) -> Vec<String> {
        rows.iter().map(|r| r.name.clone()).collect()
    }

    fn expanded(list: &[&str]) -> BTreeSet<PathBuf> {
        list.iter().map(PathBuf::from).collect()
    }

    /// **默认全折叠**：空 expanded → 只显示根级条目。
    ///
    /// 这是本模块存在的直接理由：默认全展开的话，342 个文件仍然是一堵墙
    /// （只是换了缩进），等于没做。
    #[test]
    fn nothing_is_expanded_by_default() {
        let files = paths(&["a.rs", "src/b.rs", "src/deep/c.rs", "docs/d.md"]);
        let rows = tree_rows(&files, &BTreeSet::new());

        // 根级：a.rs + docs/ + src/
        assert_eq!(names(&rows), vec!["docs", "src", "a.rs"], "只有根级条目");
        assert!(rows.iter().all(|r| r.depth == 0), "全是第 0 层");
        // 深层文件不该出现
        assert!(!names(&rows).iter().any(|n| n == "c.rs" || n == "b.rs"));
    }

    /// 展开一个目录 → 它的**直接**子项出现，孙项仍不出现。
    #[test]
    fn expanding_shows_only_the_next_level() {
        let files = paths(&["src/b.rs", "src/deep/c.rs"]);
        let rows = tree_rows(&files, &expanded(&["src"]));

        assert_eq!(names(&rows), vec!["src", "deep", "b.rs"]);
        assert_eq!(rows[0].depth, 0, "src 是根级");
        assert_eq!(rows[1].depth, 1, "deep 在 src 下一层");
        assert_eq!(rows[2].depth, 1, "b.rs 在 src 下一层");
        // deep 未展开 → 它的子项不出现
        assert!(!names(&rows).iter().any(|n| n == "c.rs"));
    }

    /// 逐层展开到底。
    #[test]
    fn expanding_each_level_reveals_the_whole_chain() {
        let files = paths(&["a/b/c/d.rs"]);
        let rows = tree_rows(&files, &expanded(&["a", "a/b", "a/b/c"]));
        assert_eq!(names(&rows), vec!["a", "b", "c", "d.rs"]);
        assert_eq!(
            rows.iter().map(|r| r.depth).collect::<Vec<_>>(),
            vec![0, 1, 2, 3],
            "深度应逐层递增"
        );
    }

    /// **目录在文件之前**（同级内），各自按名字升序。
    #[test]
    fn directories_sort_before_files_within_a_level() {
        let files = paths(&["zeta.rs", "alpha.rs", "mid/x.rs", "beta/y.rs"]);
        let rows = tree_rows(&files, &BTreeSet::new());
        assert_eq!(
            names(&rows),
            vec!["beta", "mid", "alpha.rs", "zeta.rs"],
            "目录（beta/mid）先，然后文件（alpha/zeta）按名字"
        );
    }

    /// **只显示确实含文件的目录** —— 不凭空多出一个点不开的节点。
    ///
    /// 目录集合是从文件路径派生的，所以"空目录"根本不会进树。
    #[test]
    fn only_directories_that_contain_files_appear() {
        let files = paths(&["src/a.rs", "top.rs"]);
        let rows = tree_rows(&files, &expanded(&["src", "src/empty", "nowhere"]));
        let ns = names(&rows);
        assert!(ns.contains(&"src".to_string()));
        assert!(
            !ns.iter().any(|n| n == "empty" || n == "nowhere"),
            "不存在的目录不该出现：{ns:?}"
        );
    }

    /// 折叠的目录要报**直接子项数**（渲染层显示"N 项"）。
    #[test]
    fn a_collapsed_directory_reports_its_direct_child_count() {
        let files = paths(&["src/a.rs", "src/b.rs", "src/c.rs"]);
        let rows = tree_rows(&files, &BTreeSet::new());
        match &rows[0].kind {
            RowKind::Dir { child_count, expanded } => {
                assert_eq!(*child_count, 3, "src 有 3 个直接子项");
                assert!(!expanded, "默认折叠");
            }
            other => panic!("src 应是目录：{other:?}"),
        }
    }

    /// 展开态在行上如实反映（渲染层据此画 ▸ 还是 ▾）。
    #[test]
    fn the_expanded_flag_reflects_the_input_set() {
        let files = paths(&["src/a.rs"]);
        let collapsed = tree_rows(&files, &BTreeSet::new());
        let opened = tree_rows(&files, &expanded(&["src"]));
        let dir_state = |rows: &[TreeRow]| match &rows[0].kind {
            RowKind::Dir { expanded, .. } => *expanded,
            _ => panic!("应是目录"),
        };
        assert!(!dir_state(&collapsed), "未在 expanded 里 → 折叠");
        assert!(dir_state(&opened), "在 expanded 里 → 展开");
    }

    /// **展开态按路径记，重扫后仍然有效**（不因下标错位而错乱）。
    ///
    /// 这条对应真实的"点刷新"场景：文件列表变了（新增了排在前面的目录），
    /// 但用户展开的那个目录**还是原来那个**。
    #[test]
    fn expansion_survives_a_rescan_that_reorders_entries() {
        let before = paths(&["src/a.rs", "zzz/b.rs"]);
        let after = paths(&["aaa/new.rs", "src/a.rs", "zzz/b.rs"]); // 前面多了一个目录
        let exp = expanded(&["src"]);

        let rows_before = tree_rows(&before, &exp);
        let rows_after = tree_rows(&after, &exp);

        let find = |rows: &[TreeRow]| {
            rows.iter()
                .find(|r| r.name == "src")
                .map(|r| matches!(r.kind, RowKind::Dir { expanded: true, .. }))
        };
        assert_eq!(find(&rows_before), Some(true));
        assert_eq!(
            find(&rows_after),
            Some(true),
            "重扫后 src 仍应展开（按路径记，不按位置）"
        );
        // 而新目录默认折叠
        let new_dir = rows_after.iter().find(|r| r.name == "aaa").unwrap();
        assert!(matches!(new_dir.kind, RowKind::Dir { expanded: false, .. }));
    }

    /// 深度逐层递增；且**缩进有上限**（避免把名字挤出可视区）。
    #[test]
    fn deep_paths_keep_increasing_depth_but_indent_is_capped() {
        let files = paths(&["a/b/c/d/e/f/g/h/i/j/k/deep.rs"]);
        let mut exp = BTreeSet::new();
        // 逐层展开到 deep.rs 可见
        exp.insert(PathBuf::from("a"));
        exp.insert(PathBuf::from("a/b"));
        exp.insert(PathBuf::from("a/b/c"));
        exp.insert(PathBuf::from("a/b/c/d"));
        exp.insert(PathBuf::from("a/b/c/d/e"));
        exp.insert(PathBuf::from("a/b/c/d/e/f"));
        exp.insert(PathBuf::from("a/b/c/d/e/f/g"));
        exp.insert(PathBuf::from("a/b/c/d/e/f/g/h"));
        exp.insert(PathBuf::from("a/b/c/d/e/f/g/h/i"));
        exp.insert(PathBuf::from("a/b/c/d/e/f/g/h/i/j"));
        exp.insert(PathBuf::from("a/b/c/d/e/f/g/h/i/j/k"));
        let rows = tree_rows(&files, &exp);
        let last = rows.last().expect("应有 deep.rs");
        assert_eq!(last.name, "deep.rs");
        assert!(last.depth >= 10, "真实深度应保留：{}", last.depth);
        assert_eq!(
            indent_level(last.depth),
            MAX_INDENT_DEPTH,
            "但缩进要封顶，否则名字被挤出面板"
        );
        assert_eq!(indent_level(0), 0);
        assert_eq!(indent_level(3), 3);
    }

    /// 退化输入不 panic、不产出无意义行。
    #[test]
    fn degenerate_inputs_produce_sane_output() {
        assert!(tree_rows(&[], &BTreeSet::new()).is_empty(), "空列表");
        // 只有根级文件
        let flat = paths(&["a.rs", "b.rs"]);
        let rows = tree_rows(&flat, &BTreeSet::new());
        assert_eq!(names(&rows), vec!["a.rs", "b.rs"]);
        assert!(rows.iter().all(|r| matches!(r.kind, RowKind::File)));
        // 重复路径不该产生重复行（索引理论上不去重，展示层要稳）
        let dup = paths(&["a.rs", "a.rs"]);
        let rows = tree_rows(&dup, &BTreeSet::new());
        assert_eq!(rows.len(), 1, "重复路径应只显示一行");
    }

    /// **顺序稳定**：同一输入两次推导结果一致（否则每次重绘树都在跳）。
    #[test]
    fn the_order_is_stable() {
        let files = paths(&["z/x.rs", "a/y.rs", "m.rs"]);
        let a = tree_rows(&files, &expanded(&["z", "a"]));
        let b = tree_rows(&files, &expanded(&["z", "a"]));
        assert_eq!(a, b);
    }

    /// `all_dirs` 与 `tree_rows` 派生出的目录集合**一致**（同一份规则）。
    #[test]
    fn all_dirs_matches_what_the_tree_shows() {
        let files = paths(&["a/b/c.rs", "a/d.rs", "top.rs"]);
        let dirs = all_dirs(&files);
        let rows = tree_rows(&files, &dirs);
        let dir_rows: BTreeSet<PathBuf> = rows
            .iter()
            .filter(|r| matches!(r.kind, RowKind::Dir { .. }))
            .map(|r| r.path.clone())
            .collect();
        assert_eq!(dir_rows, dirs, "两者应给出同一组目录");
        assert!(dirs.contains(Path::new("a")));
        assert!(dirs.contains(Path::new("a/b")));
    }

    /// `ancestors_of` 给出到根的完整链（供"展开到"用）。
    #[test]
    fn ancestors_of_returns_the_whole_chain() {
        let a = ancestors_of(Path::new("a/b/c/d.rs"));
        assert_eq!(
            a.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>(),
            vec!["a", "a/b", "a/b/c"]
        );
        assert!(ancestors_of(Path::new("top.rs")).is_empty(), "根级文件无祖先");
    }
}
