//! 工作区**文件索引**：给文件树提供"有哪些文件"这一层。
//!
//! # 它为什么是一个需要认真设计的东西（不是一次 `read_dir`）
//!
//! 界面上的文件树看着简单，但"扫什么、不扫什么"是一串**必须做对**的决定 ——
//! 出错的方式都是"看起来在工作，实际很糟"：
//!
//! | 决定 | 做错的后果 |
//! |---|---|
//! | 遵守 `.gitignore` | 列出 `node_modules/`、`target/`：几万条，用户找不到自己的文件 |
//! | 跳过隐藏文件 | 把 `.git/` 内部结构也列出来（对象库是散列文件，对人类无意义） |
//! | 不跟随符号链接 | 跟随就会**成环**（`a -> ../..`），扫描永远不结束 |
//! | 只扫到有限深度 | 深目录让扫描变成"打开面板卡十秒" |
//! | 条目数有上限 | 巨型仓库（10 万文件）撑爆内存与界面 |
//!
//! 前四条由 `ignore` crate 负责（它正是 ripgrep 用的那个，语义经过大量实践），
//! 第五条由本模块负责：**任何可能产生大输出的路径都必须受上限约束并如实上报
//! `truncated`**（AGENTS.md 的"内存有界性是内核义务"）。
//!
//! # 为什么它住在 `neo-platform`（L0）
//!
//! 它与渲染、与内核都无关，只依赖文件系统 —— 和剪贴板/提醒同一个层次。
//! 放在 L0 的另一个好处：**TUI 与两个 GUI 宿主将来都能用它**，
//! 不必各自实现一遍"什么该忽略"（那正是"改一处忘一处"的来源）。
//!
//! # 诚实边界
//!
//! - **不做增量/监听**：每次调用是一次完整扫描。界面在打开面板时扫一次；
//!   文件在会话中途变化**不会**自动反映（需要重启面板或后续接 `notify`）。
//! - **不读文件内容**：本模块只给路径。内容由 `@文件` 引用链路（内核）负责。
//!   这条边界让索引不可能把大文件读进内存。

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// 一次扫描的**上限**（必须全部有界，否则巨型仓库会拖垮界面）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanLimits {
    /// 最多返回多少个条目。超出即停止并标记 `truncated`。
    ///
    /// 取 5000：文件树一次真正能看的量远小于此（面板高度就几百像素），
    /// 而 5000 条路径的内存占用可忽略。它挡的是"10 万文件仓库"。
    pub max_entries: usize,
    /// 最大递归深度（相对工作区根）。
    ///
    /// 取 12：正常项目很少超过（`src/a/b/c/...` 十几层已很深），
    /// 而它挡住的是病态深目录（生成器或软链造成的近似无限层级）。
    pub max_depth: usize,
    /// 扫描总时间上限。
    ///
    /// 即使条目数与深度都有限，在极慢的磁盘（网络挂载）上仍可能很慢 ——
    /// 时间是最后一道闸。取 2 秒：超过它，用户已经在怀疑界面卡死了。
    pub max_time: Duration,
}

impl Default for ScanLimits {
    fn default() -> Self {
        Self {
            max_entries: 5000,
            max_depth: 12,
            max_time: Duration::from_secs(2),
        }
    }
}

/// 扫描结果。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileIndex {
    /// 工作区内的**文件**路径（相对根，已排序）。
    ///
    /// 只含文件、不含目录：文件树的展开需要目录，但"哪些文件可选作 `@引用`"
    /// 与"目录怎么分组"是两件事 —— 目录由渲染层从路径前缀派生
    /// （见 `FileIndex::dirs`），不必让扫描多带一份数据。
    pub files: Vec<PathBuf>,
    /// 是否因为触及上限而**没有列全**。
    ///
    /// ⚠️ **必须如实上报**：用户看到一棵树时会默认"这就是全部"，
    /// 而上限意味着不是。不报的话"文件找不到"会被当成"文件不存在"。
    pub truncated: bool,
    /// 被上限拦下的原因（诊断用；未截断时为 `None`）。
    pub truncated_reason: Option<TruncatedReason>,
}

/// 截断的原因（让"没列全"可解释，而不是一个笼统的布尔）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TruncatedReason {
    /// 条目数达到上限
    TooManyEntries,
    /// 达到最大深度
    MaxDepth,
    /// 扫描超时
    TimedOut,
}

impl TruncatedReason {
    /// 给用户看的一句话（界面直接显示它，不要自己编）。
    pub fn explain(self, limits: &ScanLimits) -> String {
        match self {
            Self::TooManyEntries => {
                format!("文件过多，只显示前 {} 个", limits.max_entries)
            }
            Self::MaxDepth => format!("目录过深，只展开到第 {} 层", limits.max_depth),
            Self::TimedOut => format!("扫描超时（>{:?}），可能实际包含更多文件", limits.max_time),
        }
    }
}

impl FileIndex {
    /// 从已排序的文件列表**派生目录集合**（供文件树分组）。
    ///
    /// 为什么不放在扫描里一起收：目录集合完全由文件路径决定（每个文件的各级
    /// 祖先），多带一份就等于"同一件事有两个来源"，而它们可能不一致。
    pub fn dirs(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        for f in &self.files {
            let mut cur = f.parent();
            while let Some(d) = cur {
                if d.as_os_str().is_empty() {
                    break;
                }
                if out.last().map(|l| l.as_path()) != Some(d) {
                    // 路径已排序 → 同一目录的祖先连续出现；只在变化时查重，
                    // 避免每个文件都做一次 O(n) 的 contains。
                    if !out.contains(&d.to_path_buf()) {
                        out.push(d.to_path_buf());
                    }
                }
                cur = d.parent();
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// 相对路径 → 可直接插入输入框的 `@引用` 文本。
    ///
    /// 用**协议层的**格式化函数（`format_file_ref`），不在这里拼字符串 ——
    /// 路径含空格时必须加引号，那是协议层的约定，只有一份实现。
    pub fn as_file_ref(&self, rel: &Path) -> Option<String> {
        let s = rel.to_str()?;
        // 统一用 `/` 分隔：`Path::to_str` 在 Windows 上给 `\`，而引用语法里
        // `\` 是转义字符（见协议层），不统一会让路径在跨平台时解析错。
        let normalized = s.replace('\\', "/");
        neo_protocol::format_file_ref(&normalized, None)
    }
}

/// 扫描工作区，返回**有界**的文件索引。
///
/// 只看文件（不看目录），并遵守 `.gitignore` / `.ignore` / 隐藏文件规则
/// （与 ripgrep 同源，见模块头部说明）。
///
/// `root` 不存在或不是目录 → 返回空索引（不 panic：界面上"工作区没文件"
/// 是正常状态，不该让面板崩掉）。
pub fn scan_workspace(root: &Path) -> FileIndex {
    scan_workspace_with(root, ScanLimits::default())
}

/// 同上，但可指定上限（测试用它构造"必然截断"的小上限）。
pub fn scan_workspace_with(root: &Path, limits: ScanLimits) -> FileIndex {
    if !root.is_dir() {
        return FileIndex::default();
    }
    let started = Instant::now();
    let mut files: Vec<PathBuf> = Vec::new();
    let mut truncated: Option<TruncatedReason> = None;

    let walker = ignore::WalkBuilder::new(root)
        // 与 ripgrep 同一套默认：遵守 .gitignore/.ignore、跳过隐藏文件。
        // `standard_filters(true)` 是这几条的开关（低于它各自设会更难读）。
        .standard_filters(true)
        // ⚠️ **必须显式关掉 `require_git`**（实测踩到）。
        //
        // 它默认为 `true`，语义是"只有身处 git 仓库内才应用 .gitignore"——
        // 那是 git 自身的语义。但对**文件树**来说这是错的：
        //   - 非 git 目录（用户的工作区可能只是普通文件夹）会**完全不忽略**，
        //     于是 `node_modules/` 之类照样列出来 —— 正是本模块要防的事；
        //   - 而 .gitignore 表达的是"作者认为无需给人看的文件"，那个意图与
        //     是不是 git 仓库无关。
        // 实测症状：临时目录（无 .git）里 .gitignore 完全失效、条目一个不漏。
        .require_git(false)
        // **绝不跟随符号链接**：跟随就会成环（`a -> ../..`），扫描永不结束。
        // 这不是效率优化，是终止性要求。
        .follow_links(false)
        .max_depth(Some(limits.max_depth))
        // 路径按稳定顺序返回：文件树的顺序不应每次打开都变（否则找文件靠运气）。
        .sort_by_file_path(|a, b| a.cmp(b))
        .build();

    for entry in walker {
        // 超时检查放在每个条目上：慢磁盘上"条目不多但每个都很慢"也能被拦住。
        if started.elapsed() > limits.max_time {
            truncated = Some(TruncatedReason::TimedOut);
            break;
        }
        let Ok(entry) = entry else {
            // 单个条目读失败（权限/竞态删除）跳过即可 —— 一棵树里有一个
            // 读不到的文件，不该让整棵树扫不出来。
            continue;
        };
        if entry.file_type().is_some_and(|t| t.is_dir()) {
            continue; // 只收文件（目录由 `dirs()` 从路径派生）
        }
        let Ok(rel) = entry.path().strip_prefix(root) else {
            continue;
        };
        if rel.as_os_str().is_empty() {
            continue; // root 自身
        }
        files.push(rel.to_path_buf());
        if files.len() >= limits.max_entries {
            truncated = Some(TruncatedReason::TooManyEntries);
            break;
        }
    }

    // 深度被截断时 `ignore` 只是不往下走，不会告诉我们"下面还有东西" ——
    // 所以这里无法区分"恰好 12 层扫完"与"还有更深"。**如实处理**：
    // 达到 `max_depth` 的目录由 walker 静默停止，故只在明确触限时才报截断。
    // 这一点写进文档而不是猜一个布尔。
    files.sort();

    FileIndex {
        files,
        truncated: truncated.is_some(),
        truncated_reason: truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一棵临时目录树。返回根（调用方负责清理）。
    fn fixture(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("neo-idx-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn write(root: &Path, rel: &str, content: &str) {
        let p = root.join(rel);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(p, content).unwrap();
    }

    /// **遵守 `.gitignore`**：被忽略的目录不该出现在索引里。
    ///
    /// 这是本模块最要紧的一条：不遵守的话 `node_modules/`、`target/` 会淹没
    /// 界面（几万条），而"列出来"本身看起来是在正常工作 —— 典型的静默变糟。
    #[test]
    fn gitignored_paths_are_excluded() {
        let root = fixture("ignore");
        write(&root, ".gitignore", "target/\nnode_modules/\n*.log\n");
        write(&root, "src/main.rs", "fn main(){}");
        write(&root, "target/debug/big.bin", "x");
        write(&root, "node_modules/pkg/index.js", "x");
        write(&root, "debug.log", "x");
        write(&root, "keep.txt", "x");

        let idx = scan_workspace(&root);
        let names: Vec<String> = idx
            .files
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();

        assert!(names.iter().any(|n| n == "src/main.rs" || n == "src\\main.rs"), "正常文件应在：{names:?}");
        assert!(names.iter().any(|n| n == "keep.txt"), "未忽略的文件应在：{names:?}");
        assert!(
            !names.iter().any(|n| n.contains("target")),
            "被 gitignore 的 target/ 不该出现：{names:?}"
        );
        assert!(
            !names.iter().any(|n| n.contains("node_modules")),
            "被 gitignore 的 node_modules/ 不该出现：{names:?}"
        );
        assert!(!names.iter().any(|n| n.ends_with(".log")), "被忽略的 *.log 不该出现");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// **跳过隐藏文件**（尤其 `.git/` 内部那些散列对象对人类无意义）。
    #[test]
    fn hidden_paths_are_excluded() {
        let root = fixture("hidden");
        write(&root, "visible.rs", "x");
        write(&root, ".hidden.rs", "x");
        write(&root, ".git/objects/ab/cdef", "x");
        write(&root, ".config/settings", "x");

        let idx = scan_workspace(&root);
        let names: Vec<String> = idx.files.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        assert!(names.iter().any(|n| n == "visible.rs"), "{names:?}");
        assert!(!names.iter().any(|n| n.starts_with('.')), "隐藏项不该出现：{names:?}");
        assert!(!names.iter().any(|n| n.contains(".git")), "{names:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// **绝不跟随符号链接**（终止性要求，不是效率优化）。
    ///
    /// 造一个自指的软链环：跟随的话扫描永不结束，本用例会挂住 ——
    /// 那正是要防的后果。
    #[test]
    #[cfg(unix)]
    fn symlink_cycles_do_not_hang_the_scan() {
        let root = fixture("symlink");
        write(&root, "real/a.rs", "x");
        // 指向祖先的环：跟随就会无限递归
        std::os::unix::fs::symlink(&root, root.join("loop")).unwrap();

        let idx = scan_workspace(&root);
        assert!(
            idx.files.iter().any(|p| p.ends_with("a.rs")),
            "真实文件应被扫到：{:?}",
            idx.files
        );
        // 没有断言"loop 不在"——符号链接本身可能被列出（它不是目录），
        // 关键是**扫描返回了**（没有成环挂死）。
        let _ = std::fs::remove_dir_all(&root);
    }

    /// **条目数上限 + 如实上报**。上限是内存有界性要求；上报是诚实要求。
    #[test]
    fn entry_limit_is_enforced_and_reported() {
        let root = fixture("limit");
        for i in 0..50 {
            write(&root, &format!("f{i:03}.txt"), "x");
        }
        let idx = scan_workspace_with(
            &root,
            ScanLimits {
                max_entries: 10,
                ..ScanLimits::default()
            },
        );
        assert_eq!(idx.files.len(), 10, "应恰好停在上限");
        assert!(idx.truncated, "触及上限必须上报 truncated");
        assert_eq!(idx.truncated_reason, Some(TruncatedReason::TooManyEntries));
        // 上报的原因要能变成一句给用户看的话（否则"截断"只是个静默事实）
        let msg = idx.truncated_reason.unwrap().explain(&ScanLimits { max_entries: 10, ..Default::default() });
        assert!(msg.contains("10"), "说明里应给出具体数字：{msg}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 深度上限被遵守。
    #[test]
    fn depth_limit_is_respected() {
        let root = fixture("depth");
        write(&root, "a/b/c/d/deep.txt", "x");
        write(&root, "top.txt", "x");
        let idx = scan_workspace_with(
            &root,
            ScanLimits {
                max_depth: 2,
                ..ScanLimits::default()
            },
        );
        let names: Vec<String> = idx.files.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        assert!(names.iter().any(|n| n == "top.txt"), "{names:?}");
        assert!(
            !names.iter().any(|n| n.contains("deep.txt")),
            "第 4 层的文件不该越界出现：{names:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// **目录从文件路径派生**（不额外存一份，避免两个来源）。
    #[test]
    fn dirs_are_derived_from_file_paths() {
        let root = fixture("dirs");
        write(&root, "src/a.rs", "x");
        write(&root, "src/nested/b.rs", "x");
        write(&root, "top.txt", "x");

        let idx = scan_workspace(&root);
        let dirs: Vec<String> = idx.dirs().iter().map(|p| p.to_string_lossy().into_owned()).collect();
        assert!(dirs.iter().any(|d| d == "src"), "{dirs:?}");
        assert!(dirs.iter().any(|d| d == "src/nested" || d == "src\\nested"), "{dirs:?}");
        // 目录集合与文件集合必须自洽：每个目录都是某个文件的祖先
        for d in idx.dirs() {
            assert!(
                idx.files.iter().any(|f| f.starts_with(&d)),
                "目录 {d:?} 不是任何文件的祖先 —— 它不该存在"
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 不存在的根 → 空索引（不 panic：界面上"工作区没文件"是正常状态）。
    #[test]
    fn a_missing_root_yields_an_empty_index() {
        let idx = scan_workspace(Path::new("/definitely/not/here/neo-idx"));
        assert!(idx.files.is_empty());
        assert!(!idx.truncated, "空结果不是截断");
        assert!(idx.dirs().is_empty());
    }

    /// 根是**文件**（不是目录）时也返回空索引，不 panic。
    ///
    /// 面板文案里有"路径不存在或不是目录"这一句 —— 这条用例保证那句话是真的，
    /// 而不是一个没被验证的猜测。
    #[test]
    fn a_root_that_is_a_file_yields_an_empty_index() {
        let root = fixture("file-root");
        let f = root.join("notadir.txt");
        std::fs::write(&f, "x").unwrap();
        let idx = scan_workspace(&f);
        assert!(idx.files.is_empty(), "文件当根 → 空索引：{:?}", idx.files);
        assert!(!idx.truncated);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// **`as_file_ref` 产出能被协议层解析回来的引用**（含空格路径）。
    ///
    /// 这是索引与"点一下加进上下文"之间的接线契约 —— 拼错的话引用会静默丢掉。
    #[test]
    fn file_refs_from_the_index_round_trip() {
        let root = fixture("refs");
        write(&root, "src/main.rs", "x");
        write(&root, "my file.txt", "x");
        write(&root, "中文 路径.rs", "x");

        let idx = scan_workspace(&root);
        for f in &idx.files {
            let r = idx
                .as_file_ref(f)
                .unwrap_or_else(|| panic!("应能生成引用：{f:?}"));
            let parsed = neo_protocol::parse_refs(&r);
            assert_eq!(parsed.len(), 1, "{r:?} 应解析出恰好一条引用");
            assert_eq!(parsed[0].kind, neo_protocol::RefKind::File);
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 排序稳定：同一目录两次扫描结果顺序一致（否则找文件靠运气）。
    #[test]
    fn the_order_is_stable_across_scans() {
        let root = fixture("order");
        for n in ["z.rs", "a.rs", "m.rs", "b/x.rs"] {
            write(&root, n, "x");
        }
        let a = scan_workspace(&root).files;
        let b = scan_workspace(&root).files;
        assert_eq!(a, b, "两次扫描顺序应一致");
        let mut sorted = a.clone();
        sorted.sort();
        assert_eq!(a, sorted, "结果应已排序");
        let _ = std::fs::remove_dir_all(&root);
    }
}
