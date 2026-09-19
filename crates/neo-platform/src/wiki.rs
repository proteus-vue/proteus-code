//! Repo Wiki：把仓库里的文档**聚合成一处**，并在聚合前过一道敏感信息闸。
//!
//! # 它解决什么
//!
//! 仓库里的说明散落在 `README.md` / `docs/**` / `AGENTS.md` 各处。要"了解这个
//! 仓库"就得自己找、自己拼，而找文档的成本往往高于读文档。Repo Wiki 把它们
//! 收进一个面板，带目录与正文。
//!
//! # 底线：绝不打包私密信息
//!
//! 聚合本身就是**风险放大器**：一份 `docs/setup.md` 里贴过一段 `.env`、
//! 某个示例里带了真 key —— 内容原本分散着没人注意，聚合之后被**打包**成一个
//! 入口，一次全露。所以本模块把这件事做成**结构性保证**，而不是"写文档提醒
//! 自己注意"：
//!
//! **任何一段内容在被收进 Wiki 之前，必须先过 [`crate::secrets`] 的检测；
//! 命中即整篇排除，并如实把"哪一篇、哪一行、什么类型"报给用户。**
//!
//! 三条由此推出的性质（都不是顺手写的，是设计）：
//!
//! 1. **闸在聚合路径上，不在调用方**。检测发生在 [`build_wiki`] 内部 ——
//!    宿主、未来的导出功能、给模型的上下文包，只要内容从这条路径过，
//!    就必然被查过。放在调用方意味着"将来多一个消费者就可能漏一处"。
//! 2. **排除是整篇，不是删行**。把命中的行抠掉、留下残缺的文档更糟：
//!    读者会以为看到的是完整内容（而它已经不自洽了），而作者也永远不会知道
//!    自己泄了东西。整篇排除 + 明确告知，才是"让人去修"的信号。
//! 3. **报告里不含密钥原文**（沿用 [`crate::secrets`] 的纪律）。
//!
//! # 有界
//!
//! 与文件树同一套口径：篇数、单篇字节、总字节三重上限，超限如实上报
//! （`truncated` + 原因）。"聚合"是最容易吃内存的形状 —— 一次性把整个
//! `docs/` 装进内存，遇到大仓库就是灾难。

use std::path::{Path, PathBuf};

use crate::file_index::{preview_file_with, Preview, PreviewLimits};
use crate::secrets::{find_secrets, SecretHit};

/// Wiki 的三重上限。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WikiLimits {
    /// 最多收多少篇。超出按排序截断并上报。
    ///
    /// 取 200：正常仓库的文档量远小于此（本仓 `docs/` 也就几十篇），
    /// 它挡的是"文档目录里混进了生成物"或"巨型 monorepo"。
    pub max_docs: usize,
    /// 单篇最多多少字节（超出截断并如实标注）。
    ///
    /// 取 256 KiB，与文件预览一致 —— 同一类"给人看"的内容没理由两套口径。
    pub max_bytes_per_doc: usize,
    /// 全部文档加起来最多多少字节。
    ///
    /// 取 8 MiB：单篇有界不代表总量有界（200 篇 × 256 KiB = 50 MiB 是可能的）。
    /// 这是**内存**闸 —— 到顶就停止收录并上报。
    pub max_total_bytes: usize,
}

impl Default for WikiLimits {
    fn default() -> Self {
        Self { max_docs: 200, max_bytes_per_doc: 256 * 1024, max_total_bytes: 8 * 1024 * 1024 }
    }
}

/// 一篇被收录的文档。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiPage {
    /// 相对工作区根的路径（与文件树同形，可直接用于展示与 `@引用`）。
    pub path: PathBuf,
    /// 标题：取正文第一个 `# ` 标题；没有则用文件名。
    pub title: String,
    /// 正文（Markdown 原文，渲染交给宿主的共享 Markdown 解析器）。
    pub body: String,
    /// 本篇是否被截断（字节上限）。
    pub truncated: bool,
    /// 截断前的总行数（未截断时等于实际行数）。
    pub total_lines: usize,
}

/// 一篇**没有**被收录的文档，以及原因。
///
/// ⚠️ 它必须被呈现给用户。默默少一篇文档，读者会把"没有这一篇"当成
/// "这个仓库没有这份文档" —— 与文件树 `truncated` 是同一个道理。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WikiSkip {
    /// 命中敏感信息（**整篇排除**）。
    ///
    /// `hits` 的每条都**不含密钥原文**（见 [`crate::secrets`]）。
    Sensitive { hits: Vec<SecretHit> },
    /// 单篇超出字节上限 —— 截断后收录，或（当前策略）截断收录。
    /// 保留这个分支是为了让"为什么这篇不全"有明确出处。
    Unreadable { reason: String },
}

/// 一篇被跳过的文档。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedDoc {
    pub path: PathBuf,
    pub reason: WikiSkip,
}

/// 聚合结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Wiki {
    pub pages: Vec<WikiPage>,
    /// 被跳过的文档（**要显示给用户**，见 [`WikiSkip`]）。
    pub skipped: Vec<SkippedDoc>,
    /// 是否因为上限而**没有收全**。
    pub truncated: bool,
    /// 未收全的原因（未截断时为 `None`）。
    pub truncated_reason: Option<String>,
}

/// 从工作区聚合 Wiki。
///
/// `files` 是文件索引给出的相对路径列表（复用同一套 `.gitignore` 语义，
/// 不在这里再实现一份遍历 —— 两份遍历必然在某个边界上不一致）。
pub fn build_wiki(root: &Path, files: &[PathBuf]) -> Wiki {
    build_wiki_with(root, files, WikiLimits::default())
}

/// 同上，可指定上限（测试用它构造"必然截断"的小上限）。
pub fn build_wiki_with(root: &Path, files: &[PathBuf], limits: WikiLimits) -> Wiki {
    let mut docs: Vec<&PathBuf> = files.iter().filter(|p| is_wiki_doc(p)).collect();
    sort_docs(&mut docs);

    let mut wiki = Wiki::default();
    let mut total_bytes = 0usize;

    for rel in docs {
        if wiki.pages.len() >= limits.max_docs {
            wiki.truncated = true;
            wiki.truncated_reason =
                Some(format!("文档过多，只收录前 {} 篇", limits.max_docs));
            break;
        }
        if total_bytes >= limits.max_total_bytes {
            wiki.truncated = true;
            wiki.truncated_reason = Some(format!(
                "文档总大小超过 {} KiB，已停止收录",
                limits.max_total_bytes / 1024
            ));
            break;
        }

        // 读取复用文件预览那条路径：同一套工作区限定（两道闸）与同一套截断。
        //
        // ⚠️ **扫的就是收的**：下面检测的是 `preview` 给出的这段字节，
        // 而收录进 `WikiPage.body` 的是**同一段**。若截断发生，两者一起被截 ——
        // 于是"没被扫到的内容"与"没被收进去的内容"永远是同一段，
        // 不会出现"收进去的没查过"这个洞。这是本模块最关键的一处对齐。
        let preview = preview_file_with(
            root,
            rel,
            PreviewLimits {
                max_bytes: limits.max_bytes_per_doc,
                max_line_chars: usize::MAX, // 文档不该被按行截断（会破坏 Markdown 结构）
            },
        );

        let (body, truncated, total_lines) = match preview {
            Preview::Text { lines, truncated, total_lines } => {
                (lines.join("\n"), truncated, total_lines)
            }
            Preview::Binary { .. } => {
                wiki.skipped.push(SkippedDoc {
                    path: rel.clone(),
                    reason: WikiSkip::Unreadable { reason: "是二进制文件".into() },
                });
                continue;
            }
            Preview::Unreadable { reason } => {
                wiki.skipped
                    .push(SkippedDoc { path: rel.clone(), reason: WikiSkip::Unreadable { reason } });
                continue;
            }
        };

        // ── 底线：聚合前过闸 ────────────────────────────────────────────
        let hits = find_secrets(&body);
        if !hits.is_empty() {
            wiki.skipped
                .push(SkippedDoc { path: rel.clone(), reason: WikiSkip::Sensitive { hits } });
            continue;
        }

        total_bytes += body.len();
        let title = extract_title(&body).unwrap_or_else(|| fallback_title(rel));
        wiki.pages.push(WikiPage { path: rel.clone(), title, body, truncated, total_lines });
    }

    wiki
}

/// 哪些文件算"文档"。
///
/// 只认 Markdown，且**只认有意义的层**：
/// - 根目录的说明文件（`README` / `AGENTS` / `CONTRIBUTING` …）—— 它们是
///   "了解这个仓库"的入口；
/// - `docs/` 下的任意 `.md`（含子目录）。
///
/// 为什么不收全仓所有 `.md`：`node_modules`（已被 gitignore 挡掉，但
/// vendored 目录不一定）、测试夹具、`CHANGELOG` 片段会让 Wiki 从"文档入口"
/// 变成"Markdown 堆积场"。**收得准比收得多重要**。
fn is_wiki_doc(rel: &Path) -> bool {
    if rel.extension().and_then(|e| e.to_str()) != Some("md") {
        return false;
    }
    // 大小写不敏感（`README.MD` 也存在）
    let name = rel.file_name().and_then(|n| n.to_str()).unwrap_or("").to_ascii_lowercase();
    if rel.parent().map(|p| p.as_os_str().is_empty()).unwrap_or(true) {
        // 根目录：只认入口类文件
        const ROOT_DOCS: &[&str] =
            &["readme.md", "agents.md", "contributing.md", "architecture.md", "security.md"];
        return ROOT_DOCS.contains(&name.as_str());
    }
    // 其余：`docs/` 之下（含多级）
    let mut comps = rel.components();
    matches!(
        comps.next().and_then(|c| c.as_os_str().to_str()),
        Some("docs") | Some("documentation") | Some("doc")
    )
}

/// 排序：根目录的入口文件在前，其余按路径。
///
/// 顺序即阅读顺序 —— `README` 应该是第一眼看到的那一篇。
fn sort_docs(docs: &mut [&PathBuf]) {
    docs.sort_by_key(|p| {
        let depth = p.components().count();
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_ascii_lowercase();
        // 根级 README 排最前，其次其它根级、然后是深层文档
        let rank = if depth == 1 && name == "readme.md" {
            0
        } else if depth == 1 {
            1
        } else {
            2
        };
        (rank, p.to_path_buf())
    });
}

/// 取正文第一个一级标题作为标题。
fn extract_title(body: &str) -> Option<String> {
    for line in body.lines().take(60) {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("# ") {
            let title = rest.trim();
            if !title.is_empty() {
                return Some(title.to_string());
            }
        }
        // 遇到二级标题就停：说明这篇没有一级标题（不要拿小节名当标题）
        if t.starts_with("## ") {
            break;
        }
    }
    None
}

/// 没有一级标题时用文件名（去掉扩展名、`-`/`_` 换成空格）。
fn fallback_title(rel: &Path) -> String {
    rel.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("（无标题）")
        .replace(['-', '_'], " ")
}

/// 一句话说明被跳过的原因（界面直接显示，不要自己编）。
pub fn skip_explain(skip: &WikiSkip) -> String {
    match skip {
        WikiSkip::Sensitive { hits } => {
            let mut uniq: Vec<&str> = Vec::new();
            for k in hits.iter().map(|h| h.kind.explain()) {
                if !uniq.contains(&k) {
                    uniq.push(k);
                }
            }
            // 行号去重：同一行可能被两条规则各命中一次（例如 `KEY=sk-…`
            // 同时是"赋值"与"服务商前缀"），在文案里报两遍只会让人困惑。
            let mut line_nos: Vec<usize> = hits.iter().map(|h| h.line).collect();
            line_nos.sort_unstable();
            line_nos.dedup();
            let lines: Vec<String> = line_nos.iter().map(|n| n.to_string()).collect();
            format!(
                "含疑似{}（第 {} 行）—— 整篇未收录",
                uniq.join(" / "),
                lines.join("、")
            )
        }
        WikiSkip::Unreadable { reason } => reason.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, rel: &str, content: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("neo-wiki-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn collects_root_readme_and_docs() {
        let d = tmpdir("basic");
        write(&d, "README.md", "# 我的项目\n\n说明。\n");
        write(&d, "docs/guide/setup.md", "# 安装\n\n步骤。\n");
        let files = vec![
            PathBuf::from("README.md"),
            PathBuf::from("docs/guide/setup.md"),
            PathBuf::from("src/main.rs"),
            PathBuf::from("docs/notes.txt"),
        ];
        let w = build_wiki(&d, &files);
        assert_eq!(w.pages.len(), 2, "只该收 Markdown 文档：{:?}", w.pages);
        assert_eq!(w.pages[0].path, PathBuf::from("README.md"), "README 必须排第一");
        assert_eq!(w.pages[0].title, "我的项目");
        assert_eq!(w.pages[1].title, "安装");
    }

    /// 只收"入口类"根文件与 `docs/` —— 不是全仓 Markdown 堆积场。
    #[test]
    fn ignores_markdown_outside_docs() {
        let d = tmpdir("scope");
        write(&d, "README.md", "# r\n");
        write(&d, "src/notes.md", "# 随手记\n");
        write(&d, "vendor/pkg/README.md", "# 第三方\n");
        let files = vec![
            PathBuf::from("README.md"),
            PathBuf::from("src/notes.md"),
            PathBuf::from("vendor/pkg/README.md"),
        ];
        let w = build_wiki(&d, &files);
        assert_eq!(w.pages.len(), 1, "只该收根级 README：{:?}", w.pages);
    }

    // ── 底线：敏感信息必须整篇排除 ───────────────────────────────────────

    #[test]
    fn documents_with_secrets_are_excluded_entirely() {
        let d = tmpdir("secrets");
        write(&d, "README.md", "# 项目\n\n正常内容。\n");
        write(
            &d,
            "docs/setup.md",
            "# 安装\n\n先设置环境：\n\nOPENAI_API_KEY=sk-9fK2mQ7dLp3Xr8Tn5Vw1Za6\n\n然后跑。\n",
        );
        let files = vec![PathBuf::from("README.md"), PathBuf::from("docs/setup.md")];
        let w = build_wiki(&d, &files);

        assert_eq!(w.pages.len(), 1, "含密钥的文档不该被收录：{:?}", w.pages);
        assert_eq!(w.pages[0].path, PathBuf::from("README.md"));
        assert_eq!(w.skipped.len(), 1);
        assert_eq!(w.skipped[0].path, PathBuf::from("docs/setup.md"));
        assert!(matches!(w.skipped[0].reason, WikiSkip::Sensitive { .. }));
    }

    /// **整篇排除**而不是删行：留下的残缺文档会让人以为看到的是全部。
    #[test]
    fn exclusion_is_whole_document_not_redacted_lines() {
        let d = tmpdir("whole");
        write(&d, "docs/a.md", "# A\n\nGH_TOKEN=ghp_9fK2mQ7dLp3Xr8Tn5Vw1Za6Bc4\n\n结尾段落。\n");
        let w = build_wiki(&d, &[PathBuf::from("docs/a.md")]);
        assert!(w.pages.is_empty());
        // 不能出现"删掉那一行、保留其余"的结果
        assert!(!w.pages.iter().any(|p| p.body.contains("结尾段落")));
    }

    /// 跳过的报告**不得回显密钥**，且要给出可定位的信息（行号、类型）。
    #[test]
    fn skip_report_never_echoes_the_secret() {
        let d = tmpdir("report");
        let secret = "sk-9fK2mQ7dLp3Xr8Tn5Vw1Za6Bc4Ye0Hg";
        write(&d, "docs/setup.md", &format!("# 安装\n\nKEY={secret}\n"));
        let w = build_wiki(&d, &[PathBuf::from("docs/setup.md")]);
        let text = skip_explain(&w.skipped[0].reason);
        for win in secret.as_bytes().windows(6) {
            let frag = std::str::from_utf8(win).unwrap();
            assert!(!text.contains(frag), "报告里泄露了密钥片段：{text}");
        }
        assert!(text.contains('3'), "该带上行号便于定位：{text}");
    }

    #[test]
    fn placeholder_docs_are_still_collected() {
        let d = tmpdir("placeholder");
        write(
            &d,
            "docs/setup.md",
            "# 安装\n\n```\nOPENAI_API_KEY=sk-YOUR_KEY_HERE\n```\n",
        );
        let w = build_wiki(&d, &[PathBuf::from("docs/setup.md")]);
        assert_eq!(w.pages.len(), 1, "教学用占位不该被拦：{:?}", w.skipped);
    }

    // ── 有界 ─────────────────────────────────────────────────────────────

    #[test]
    fn doc_count_limit_is_reported() {
        let d = tmpdir("count");
        let mut files = Vec::new();
        for i in 0..5 {
            let rel = format!("docs/{i}.md");
            write(&d, &rel, &format!("# 第 {i} 篇\n"));
            files.push(PathBuf::from(rel));
        }
        let w = build_wiki_with(
            &d,
            &files,
            WikiLimits { max_docs: 3, ..Default::default() },
        );
        assert_eq!(w.pages.len(), 3);
        assert!(w.truncated);
        assert!(w.truncated_reason.unwrap().contains("3"));
    }

    #[test]
    fn per_doc_byte_limit_truncates_and_says_so() {
        let d = tmpdir("bytes");
        write(&d, "docs/big.md", &format!("# 大文件\n\n{}", "x".repeat(5000)));
        let w = build_wiki_with(
            &d,
            &[PathBuf::from("docs/big.md")],
            WikiLimits { max_bytes_per_doc: 500, ..Default::default() },
        );
        assert_eq!(w.pages.len(), 1);
        assert!(w.pages[0].truncated, "截断必须如实标注");
        assert!(w.pages[0].body.len() <= 500);
    }

    /// 单篇有界 ≠ 总量有界：多篇小文件叠起来仍可能很大。
    #[test]
    fn total_byte_limit_stops_collection() {
        let d = tmpdir("total");
        let mut files = Vec::new();
        for i in 0..10 {
            let rel = format!("docs/{i}.md");
            write(&d, &rel, &format!("# 第 {i} 篇\n\n{}", "y".repeat(800)));
            files.push(PathBuf::from(rel));
        }
        let w = build_wiki_with(
            &d,
            &files,
            WikiLimits { max_total_bytes: 2500, ..Default::default() },
        );
        assert!(w.pages.len() < 10, "总量闸没生效：收了 {} 篇", w.pages.len());
        assert!(w.truncated);
    }

    /// 截断点之外的内容不会进 Wiki —— 所以"扫的就是收的"这条对齐成立。
    #[test]
    fn nothing_beyond_the_truncation_point_is_included() {
        let d = tmpdir("align");
        // 密钥放在第 300 字节之后，而单篇上限是 200 字节
        let body = format!("# 文档\n\n{}\nOPENAI_API_KEY=sk-9fK2mQ7dLp3Xr8Tn5Vw1Za6\n", "a".repeat(300));
        write(&d, "docs/a.md", &body);
        let w = build_wiki_with(
            &d,
            &[PathBuf::from("docs/a.md")],
            WikiLimits { max_bytes_per_doc: 200, ..Default::default() },
        );
        // 无论它被判定为"截断收录"还是"整篇排除"，都**不能**把密钥带进来
        for p in &w.pages {
            assert!(!p.body.contains("sk-9fK2mQ7dLp3Xr8Tn5Vw1Za6"));
        }
    }

    #[test]
    fn title_falls_back_to_filename() {
        let d = tmpdir("title");
        write(&d, "docs/getting-started.md", "正文没有一级标题。\n");
        let w = build_wiki(&d, &[PathBuf::from("docs/getting-started.md")]);
        assert_eq!(w.pages[0].title, "getting started");
    }

    #[test]
    fn empty_workspace_yields_empty_wiki() {
        let d = tmpdir("empty");
        let w = build_wiki(&d, &[]);
        assert!(w.pages.is_empty());
        assert!(w.skipped.is_empty());
        assert!(!w.truncated);
    }

    /// 读不出来的文档要**报出来**，不能静默少一篇。
    #[test]
    fn unreadable_docs_are_reported_not_silently_dropped() {
        let d = tmpdir("unreadable");
        // 索引里有、磁盘上没有（索引过期是真实情形）
        let w = build_wiki(&d, &[PathBuf::from("docs/missing.md")]);
        assert!(w.pages.is_empty());
        assert_eq!(w.skipped.len(), 1, "缺失的文档必须被报告");
        assert!(matches!(w.skipped[0].reason, WikiSkip::Unreadable { .. }));
    }
}
