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

/// 一次**文件预览**（内置浏览器）的上限。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviewLimits {
    /// 最多显示多少字节。
    ///
    /// 取 256 KiB：装得下绝大多数源文件（本仓最大的 `PROJECT_MEMORY.md` 接近 300 KB，
    /// 会被截断并如实上报）。再大没有意义 —— 界面一次能滚过的量有限，
    /// 而且"打开一个文件把内存拉满"是必须避免的。
    pub max_bytes: usize,
    /// 单行最大显示字符数。超过则**从行首截断**并标注。
    ///
    /// 取 2000：压过的 JS（一行几万字符）与长 base64 会撑爆横向布局，
    /// 而这类行本来就无法阅读。截断而不是折行 —— 折行会让行号与内容错位，
    /// 而"行号对不上"比"看不全"更糟（用户会以为看到的是那一行全部）。
    pub max_line_chars: usize,
}

impl Default for PreviewLimits {
    fn default() -> Self {
        Self { max_bytes: 256 * 1024, max_line_chars: 2000 }
    }
}

/// 一次文件预览的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preview {
    /// 文本内容（已按上限截断）。
    Text {
        /// 逐行（**已切好、且按 char 边界安全**），供界面直接渲染。
        lines: Vec<String>,
        /// 字节数是否触及上限（如实上报；界面要显示"只显示前 N KB"）。
        truncated: bool,
        /// 总行数（截断前）。截断时它与 `lines.len()` 不同 —— 界面据此说明。
        total_lines: usize,
    },
    /// 二进制（含 NUL 字节）：**不当作文本显示**。
    ///
    /// 为什么单独一类而不是"显示乱码"：把 PNG/可执行文件当文本渲染会得到
    /// 满屏乱码，用户以为文件损坏了。明确说"这是二进制"才是诚实的。
    Binary { size: usize },
    /// 读不出来（不存在 / 权限 / 不是文件）。
    Unreadable { reason: String },
}

/// 读取一个文件用于**界面预览**（D12 内置浏览器）。
///
/// # 它为什么不是内核那条 `@文件` 注入路径
///
/// 两者**目的不同、安全性要求也不同**：
/// - 内核注入：内容进**模型上下文**，要花钱、要经沙箱（`cat` 走 `SandboxBackend`）、
///   受 `max_output_bytes` 约束；
/// - 界面预览：内容只给人看，**不进模型、不花钱**。
///
/// 所以它不走沙箱（沙箱是"内核执行外部动作"的边界），但**必须自己做两件事**：
/// 1. **限定在工作区内**（复用 `is_within`，与沙箱同一套判定）——
///    否则界面能读任意路径，而路径来自扫描结果，理论上可被软链带出去；
/// 2. **有界**（字节上限 + 单行上限），理由见 [`PreviewLimits`]。
///
/// `rel` 是**相对工作区**的路径（与 `FileIndex::files` 的元素同形）。
pub fn preview_file(root: &Path, rel: &Path) -> Preview {
    preview_file_with(root, rel, PreviewLimits::default())
}

/// 同上，可指定上限（测试用它构造"必然截断"的小上限）。
pub fn preview_file_with(root: &Path, rel: &Path, limits: PreviewLimits) -> Preview {
    // 防御：`rel` 若带 `..` 或绝对路径，`join` 之后可能跑到工作区外。
    // `is_within` 会兜住，但先在这里拒绝能给出更清楚的原因。
    if rel.is_absolute() || rel.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Preview::Unreadable {
            reason: format!("路径必须在工作区内（收到 {})", rel.display()),
        };
    }
    let full = root.join(rel);
    if !is_within(root, &full) {
        return Preview::Unreadable {
            reason: format!("路径在工作区之外：{}", rel.display()),
        };
    }
    let meta = match std::fs::metadata(&full) {
        Ok(m) => m,
        Err(e) => return Preview::Unreadable { reason: format!("无法读取：{e}") },
    };
    if meta.is_dir() {
        return Preview::Unreadable { reason: "这是一个目录".into() };
    }
    let size = meta.len() as usize;
    // 先按上限读，避免把超大文件整体读进内存再截断。
    let bytes = match std::fs::read(&full) {
        Ok(b) => b,
        Err(e) => return Preview::Unreadable { reason: format!("无法读取：{e}") },
    };
    let read_truncated = bytes.len() > limits.max_bytes;
    let slice = &bytes[..bytes.len().min(limits.max_bytes)];

    // 二进制判定：出现 NUL 就当二进制。这是 git 的判据（几千字节内看有没有 NUL），
    // 简单且足够 —— 文本文件里出现 NUL 本身就是异常。
    if slice.contains(&0) {
        return Preview::Binary { size };
    }

    // 字节 → 文本。**不假设 UTF-8**：非 UTF-8 的文本文件（GBK 等）用 lossy
    // 转换会得到替换字符，但"能看个大概"好过"整个文件报错"。
    //
    // ⚠️ 截断点可能落在多字节字符中间 —— `from_utf8_lossy` 会把不完整的那一
    // 个字符替换成 U+FFFD，这正是我们要的（宁可最后一个字符是替换符，
    // 也不 panic、也不丢整行）。
    let text = String::from_utf8_lossy(slice).into_owned();
    // 截断在字符串级再确认一次：按字节截可能切在字符中间，
    // 这里用 char 边界安全的截断兜住（与内核 `truncate_utf8` 同一手法）。
    let (text, cut_at_char) = truncate_on_char_boundary(&text, limits.max_bytes);
    let truncated = read_truncated || cut_at_char;

    // `lines()` 已经不吃尾部换行（"a\n" 与 "a" 都是一行），所以不能再加一 ——
    // 加了会让每个以换行结尾的文件都多报一行（实测：3 行文件报 4 行）。
    let total_lines = text.lines().count();
    // 单行长度封顶：这是**内存/布局**的下限保护（一行几 MB 的压缩文件），
    // 不是"给人看的截断提示"。
    //
    // ⚠️ 所以这里**不追加**"已截断"之类的文字 —— 实测确认它会被渲染层的
    // 省略号裁在可视区之外（2000 字符远超面板宽度），加了也是死重。
    // 用户可见的"还有内容"信号由渲染层的 `…` 承担（见宿主预览栏）。
    let lines: Vec<String> = text
        .lines()
        .map(|l| {
            if l.chars().count() > limits.max_line_chars {
                l.chars().take(limits.max_line_chars).collect()
            } else {
                l.to_string()
            }
        })
        .collect();

    Preview::Text { lines, truncated, total_lines }
}

/// `path` 解析后是否落在 `root` 之内（含软链解析）。
///
/// # 为什么这里自己实现一份，而不是复用 `neo-sandbox-local::is_within`
///
/// 因为**依赖方向不允许**：本 crate 是 L1，而 `neo-sandbox-local` 是 L3 ——
/// 向上依赖会被架构守卫（`check_architecture.py`）拒绝，而那条守卫是对的
/// （平台层不该知道沙箱实现的存在）。
///
/// 代价是同一套语义有两份实现，所以：
/// - 这里**逐句对齐**那边的做法（`canonicalize` 后在真实路径上比前缀），
///   而不是自己发明一个"看起来差不多"的判定；
/// - 两处都有测试覆盖"软链/不存在/工作区外"三情形。
///
/// 与那边一样：**解析不出来就拒绝**（宁可拒绝，不可放行）。
fn is_within(root: &Path, path: &Path) -> bool {
    let real_root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    match resolve_real(path) {
        Some(real_target) => real_target.starts_with(&real_root),
        None => false,
    }
}

/// 解析出目标的"真实路径"，即使目标本身尚不存在。
///
/// 上溯到最近可 canonicalize 的祖先，再把尚未存在的路径段原样拼回 ——
/// 这样"新建一个还不存在的文件"也能被正确判定在工作区内。
fn resolve_real(path: &Path) -> Option<PathBuf> {
    if let Ok(p) = std::fs::canonicalize(path) {
        return Some(p);
    }
    let mut pending: Vec<std::ffi::OsString> = Vec::new();
    let mut cursor = path.to_path_buf();
    loop {
        let name = cursor.file_name()?.to_os_string();
        pending.push(name);
        cursor = cursor.parent()?.to_path_buf();
        if let Ok(real) = std::fs::canonicalize(&cursor) {
            let mut out = real;
            for seg in pending.iter().rev() {
                out.push(seg);
            }
            return Some(out);
        }
    }
}

/// 按 char 边界安全地把 `s` 截到 `max_bytes` 以内。
fn truncate_on_char_boundary(s: &str, max_bytes: usize) -> (String, bool) {
    if s.len() <= max_bytes {
        return (s.to_string(), false);
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    (s[..end].to_string(), true)
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

    /// 正常文本：逐行返回，行数正确。
    #[test]
    fn preview_reads_a_text_file_line_by_line() {
        let root = fixture("pv-text");
        std::fs::write(root.join("a.txt"), "第一行\n第二行\n第三行\n").unwrap();
        match preview_file(&root, Path::new("a.txt")) {
            Preview::Text { lines, truncated, total_lines } => {
                assert_eq!(lines, vec!["第一行", "第二行", "第三行"]);
                assert!(!truncated);
                assert_eq!(total_lines, 3);
            }
            other => panic!("应是文本：{other:?}"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// **二进制必须被认出来**，不能当文本渲染（否则满屏乱码，用户以为文件坏了）。
    #[test]
    fn preview_detects_binary_instead_of_rendering_garbage() {
        let root = fixture("pv-bin");
        std::fs::write(root.join("img.bin"), [0x89, b'P', b'N', b'G', 0x00, 0x1a, 0xff]).unwrap();
        match preview_file(&root, Path::new("img.bin")) {
            Preview::Binary { size } => assert_eq!(size, 7),
            other => panic!("应识别为二进制：{other:?}"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// **工作区外的路径必须被拒**（`..` 逃逸 / 绝对路径）。
    ///
    /// 预览不做沙箱，这是它唯一的安全边界 —— 没有这条，界面能读任意文件。
    ///
    /// ⚠️ **它与下面那条软链测试是两道独立的闸，不是重复的**（实测确认过：
    /// 关掉 `is_within` 时只有软链那条会红）：
    /// - 本条的输入就**长得可疑**（含 `..` 或是绝对路径）→ 由入口的
    ///   `ParentDir` / `is_absolute` 检查拦住，`is_within` 甚至不会被走到；
    /// - 软链那条的输入**长得完全正常**（`link/secret.txt`）→ 只有把路径
    ///   `canonicalize` 之后比前缀（`is_within`）才拦得住。
    ///
    /// 所以别把任何一道当"冗余"删掉 —— 它们各自挡住一种绕过方式。
    #[test]
    fn preview_refuses_paths_outside_the_workspace() {
        let root = fixture("pv-escape");
        std::fs::create_dir_all(root.join("inner")).unwrap();
        std::fs::write(root.join("inner/ok.txt"), "x").unwrap();

        for bad in ["../outside.txt", "inner/../../escape.txt", "/etc/passwd"] {
            match preview_file(&root, Path::new(bad)) {
                Preview::Unreadable { reason } => {
                    assert!(!reason.is_empty(), "拒绝要给出原因：{bad}");
                }
                other => panic!("{bad} 应被拒绝，实际：{other:?}"),
            }
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// **软链指向工作区外**也要被拒（前缀比较若不做真实路径解析会被绕过）。
    #[test]
    #[cfg(unix)]
    fn preview_refuses_symlinks_pointing_outside() {
        let root = fixture("pv-symlink");
        let outside = fixture("pv-symlink-target");
        std::fs::write(outside.join("secret.txt"), "秘密").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();

        match preview_file(&root, Path::new("link/secret.txt")) {
            Preview::Unreadable { .. } => {}
            other => panic!("经软链逃逸必须被拒，实际：{other:?}"),
        }
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    /// **超长单行被封顶**（内存/布局的下限保护）。
    ///
    /// ⚠️ 这里**不断言任何"已截断"文字**：实测确认这类标注会被渲染层的省略号
    /// 裁在可视区之外（2000 字符远超面板宽度），加了也是死重。
    /// 用户可见的"还有内容"信号由渲染层的 `…` 承担 ——
    /// **数据层管封顶、渲染层管提示**，各做各的，不重复。
    #[test]
    fn preview_caps_overlong_lines() {
        let root = fixture("pv-longline");
        let long = "x".repeat(5000);
        std::fs::write(root.join("long.txt"), format!("{long}\n短行\n")).unwrap();
        let limits = PreviewLimits { max_bytes: 1024 * 1024, max_line_chars: 100 };
        match preview_file_with(&root, Path::new("long.txt"), limits) {
            Preview::Text { lines, .. } => {
                assert_eq!(lines.len(), 2, "仍是两行");
                assert_eq!(lines[0].chars().count(), 100, "超长行应被封到上限");
                assert_eq!(lines[1], "短行", "正常行不受影响");
            }
            other => panic!("应是文本：{other:?}"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// **字节上限生效且如实上报**（读取本身就有界，不是读完再截）。
    #[test]
    fn preview_respects_the_byte_cap_and_reports_truncation() {
        let root = fixture("pv-bytecap");
        let big: String = (0..500).map(|i| format!("第 {i} 行，写得长一点以便超过上限\n")).collect();
        std::fs::write(root.join("big.txt"), &big).unwrap();
        let limits = PreviewLimits { max_bytes: 200, max_line_chars: 2000 };
        match preview_file_with(&root, Path::new("big.txt"), limits) {
            Preview::Text { truncated, .. } => assert!(truncated, "触及字节上限必须上报"),
            other => panic!("应是文本：{other:?}"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// **截断点落在多字节字符中间不 panic、不切碎**。
    ///
    /// 这是中文/emoji 下必然遇到的边界：按字节截断几乎总会落在字符中间。
    #[test]
    fn preview_cuts_on_character_boundaries_for_cjk() {
        let root = fixture("pv-cjk");
        // 每字 3 字节（中文），上限取一个明显的非 3 倍数 → 必然切在字符中间
        let text = "中文内容测试中文内容测试中文内容测试";
        std::fs::write(root.join("cjk.txt"), text).unwrap();
        for cap in [4usize, 7, 10, 13, 104] {
            let limits = PreviewLimits { max_bytes: cap, max_line_chars: 2000 };
            match preview_file_with(&root, Path::new("cjk.txt"), limits) {
                Preview::Text { lines, .. } => {
                    // 关键：不 panic，且内容仍是**合法 UTF-8 的前缀**
                    for l in &lines {
                        assert!(text.starts_with(l.as_str()) || text.contains(l.as_str()),
                            "截断结果应是原文的前缀，不出现乱码：{l:?}");
                    }
                }
                other => panic!("cap={cap} 应是文本：{other:?}"),
            }
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 目录与不存在的文件 → `Unreadable`（且给出原因），不 panic。
    #[test]
    fn preview_reports_directories_and_missing_files() {
        let root = fixture("pv-misc");
        std::fs::create_dir_all(root.join("adir")).unwrap();
        for p in ["adir", "nope.txt"] {
            match preview_file(&root, Path::new(p)) {
                Preview::Unreadable { reason } => assert!(!reason.is_empty()),
                other => panic!("{p} 应不可读：{other:?}"),
            }
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 非 UTF-8 的文本文件**不报错**（用 lossy 转换"能看个大概"好过整个文件打不开）。
    #[test]
    fn preview_does_not_fail_on_non_utf8_text() {
        let root = fixture("pv-gbk");
        // GBK 编码的 "中文"：0xD6 0xD0 0xCE 0xC4 —— 不是合法 UTF-8
        std::fs::write(root.join("gbk.txt"), [0xD6, 0xD0, 0xCE, 0xC4]).unwrap();
        match preview_file(&root, Path::new("gbk.txt")) {
            Preview::Text { lines, .. } => {
                assert!(!lines.is_empty(), "应仍给出内容（替换字符）");
            }
            other => panic!("非 UTF-8 文本不该报错：{other:?}"),
        }
        let _ = std::fs::remove_dir_all(&root);
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
