//! L3 PROVIDER —— 项目指令（`AGENTS.md`）的级联发现与加载。
//!
//! # 契约（来自 `docs/neo-plan/02-架构设计/上下文策略-fresh-by-default.md`）
//!
//! ```text
//! ~/.neo/AGENTS.override.md   ← 始终最高优先级
//! ~/.neo/AGENTS.md            ← 个人基线
//! <repo>/AGENTS.md            ← 团队共享
//! <repo>/sub/dir/AGENTS.md    ← 目录级联，越具体越靠后
//! ```
//!
//! - **每目录最多一个** `AGENTS.md`。
//! - 合并上限 **32 KiB**（`neo_config::AGENTS_MAX_BYTES`）——防滥用。
//! - 加载顺序即上面这个顺序；注入文本里会**显式标注**优先级，
//!   不靠位置暗示（位置在提示词里的"显著性"是经验性的，标注是确定的）。
//!
//! # 为什么按 git 根界定"仓库范围"
//!
//! 若不设界，向上遍历会一路捡到 `$HOME/AGENTS.md` 之类与项目无关的文件。
//! 契约把"个人基线"明确放在 `~/.neo/AGENTS.md`，所以仓库级级联应从
//! **仓库根**（最近的含 `.git` 的祖先）开始，而不是从文件系统根开始。
//! 找不到 git 根时只取 `cwd` 自身 —— 宁可不级联，也不越界捡无关文件。
//!
//! # 超限行为
//!
//! 文档规划的是"超限让模型生成握手摘要"。那是更大的一步，**当前未实现**；
//! 这里做的是**诚实截断**：截到上限、`truncated = true`、并在注入文本里
//! 明说"内容已被截断"。悄悄丢掉一半指令比截断更危险 —— 模型会以为
//! 它看到了全部约定。

use std::path::{Path, PathBuf};

/// 合并上限，直接复用配置层的常量（那里原本是个无人使用的死常量）。
pub const MAX_BYTES: usize = neo_config::AGENTS_MAX_BYTES;

/// 最多级联几个仓库内目录的 `AGENTS.md`（防止深层目录栈把上限吃光）。
const MAX_REPO_FILES: usize = 16;

// 返回值用 L2 的 `neo_core::Instructions` —— 数据表示属于内核，
// 本 crate 只负责"从磁盘算出它"（与 skill-loader 返回 SkillRegistry 同一分工）。
pub use neo_core::instructions::Instructions;

/// `NEO_HOME`（默认 `$HOME/.neo`）。测试用 `NEO_HOME` 隔离。
pub fn neo_home() -> Option<PathBuf> {
    std::env::var_os("NEO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .map(|h| if h.ends_with(".neo") { h } else { h.join(".neo") })
}

/// 按契约顺序算出要读哪些文件及各自的优先级标注。
fn discover(home: Option<&Path>, cwd: &Path) -> Vec<(PathBuf, &'static str)> {
    let mut out: Vec<(PathBuf, &'static str)> = Vec::new();

    if let Some(h) = home {
        out.push((h.join("AGENTS.override.md"), "最高优先级（override）"));
        out.push((h.join("AGENTS.md"), "个人基线"));
    }

    // 仓库根 = 最近的含 .git 的祖先；找不到则只取 cwd。
    let root = git_root(cwd);
    let dirs: Vec<PathBuf> = match &root {
        Some(r) => dirs_between(r, cwd),
        None => vec![cwd.to_path_buf()],
    };
    for d in dirs.iter().take(MAX_REPO_FILES) {
        let label = if Some(d) == root.as_ref() { "仓库约定" } else { "目录级联（越具体越靠后）" };
        out.push((d.join("AGENTS.md"), label));
    }
    out
}

/// 最近的含 `.git` 的祖先（`.git` 可能是目录，也可能是 worktree 的文件）。
fn git_root(cwd: &Path) -> Option<PathBuf> {
    let mut cur = Some(cwd);
    while let Some(d) = cur {
        if d.join(".git").exists() {
            return Some(d.to_path_buf());
        }
        cur = d.parent();
    }
    None
}

/// `root` 到 `cwd`（含两端）的目录链，**从根到具体**。
fn dirs_between(root: &Path, cwd: &Path) -> Vec<PathBuf> {
    let mut chain = Vec::new();
    let mut cur = Some(cwd);
    while let Some(d) = cur {
        chain.push(d.to_path_buf());
        if d == root {
            break;
        }
        cur = d.parent();
    }
    chain.reverse();
    chain
}

/// 加载并按上限合并。文件不存在**不是错误**（大多数目录没有 AGENTS.md）。
pub fn load(home: Option<&Path>, cwd: &Path) -> Instructions {
    let mut picked: Vec<(PathBuf, &'static str)> = Vec::new();
    for (p, label) in discover(home, cwd) {
        if p.is_file() {
            picked.push((p, label));
        }
    }
    if picked.is_empty() {
        return Instructions::default();
    }

    let mut block = String::new();
    let mut sources = Vec::new();
    let mut truncated = false;

    for (path, label) in &picked {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            // 单个文件读不出不影响其余：记进 sources 但没有内容会误导，
            // 所以跳过并在注释里说明（这里的选择是"不假装加载成功"）。
            Err(_) => continue,
        };
        let section = format!(
            "\n<!-- 来源：{} · {label} -->\n{}\n",
            path.display(),
            content.trim_end()
        );
        // 逐段判上限：宁可少注入后面的，也不整段超发。
        let room = MAX_BYTES.saturating_sub(block.len());
        if section.len() > room {
            let (cut, _) = neo_core::truncate_utf8(&section, room);
            if !cut.is_empty() {
                block.push_str(cut);
                sources.push(path.display().to_string());
            }
            truncated = true;
            break;
        }
        block.push_str(&section);
        sources.push(path.display().to_string());
    }

    if block.is_empty() {
        return Instructions::default();
    }

    let header = format!(
        "以下内容来自工作区与用户目录的 AGENTS.md，是**必须遵守**的项目约定；\
         出现冲突时以标注「最高优先级」的为准。{}",
        if truncated { "（注意：内容已按 32 KiB 上限截断，未展示的部分不生效）" } else { "" }
    );
    let full = format!("{header}\n{block}");

    Instructions { sources, block: full, truncated }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("neo-instr-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn put(dir: &Path, name: &str, content: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(name), content).unwrap();
    }

    #[test]
    fn cascade_order_is_override_personal_repo_then_nested() {
        let home = tmp("home");
        let repo = tmp("repo");
        put(&repo, ".git", "gitdir: /nowhere\n"); // .git 可为文件（worktree）
        put(&home, "AGENTS.override.md", "OVERRIDE");
        put(&home, "AGENTS.md", "PERSONAL");
        put(&repo, "AGENTS.md", "REPO");
        let sub = repo.join("crates/app");
        put(&sub, "AGENTS.md", "NESTED");

        let got = load(Some(&home), &sub);
        let b = &got.block;
        let i_ov = b.find("OVERRIDE").expect("override 必须在内");
        let i_pe = b.find("PERSONAL").expect("personal 必须在内");
        let i_re = b.find("REPO").expect("repo 必须在内");
        let i_ne = b.find("NESTED").expect("nested 必须在内");
        assert!(i_ov < i_pe && i_pe < i_re && i_re < i_ne, "加载顺序必须符合契约");
        assert_eq!(got.sources.len(), 4);
        assert!(!got.truncated);
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn repo_scope_does_not_pick_up_unrelated_ancestors() {
        // 没有 .git 时只取 cwd：向上遍历不得捡到 $HOME/AGENTS.md 之类。
        let repo = tmp("norepo");
        let sub = repo.join("deep/nested");
        put(&repo, "AGENTS.md", "SHOULD_NOT_LOAD");
        put(&sub, "AGENTS.md", "LOCAL_ONLY");

        let got = load(None, &sub);
        assert!(got.block.contains("LOCAL_ONLY"));
        assert!(!got.block.contains("SHOULD_NOT_LOAD"), "无 git 根时不得向上级联");
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn missing_files_yield_empty_not_error() {
        let d = tmp("empty");
        let got = load(Some(&d), &d);
        assert!(got.is_empty());
        assert!(got.sources.is_empty());
        assert_eq!(got.summary(), "未找到项目指令（AGENTS.md）");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn honors_byte_cap_and_reports_truncation() {
        let repo = tmp("cap");
        put(&repo, ".git", "x");
        let big = "A".repeat(MAX_BYTES + 5000);
        put(&repo, "AGENTS.md", &big);

        let got = load(None, &repo);
        assert!(got.truncated, "超限必须如实标记");
        // 上限只管正文；header 是固定的一小段（远小于 512 字节）。
        assert!(got.block.len() <= MAX_BYTES + 512, "正文不得越过上限：{}", got.bytes());
        assert!(got.block.contains("已按 32 KiB 上限截断"), "必须告知模型内容不全");
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn truncation_never_splits_a_utf8_codepoint() {
        // 截断点若落在多字节字符中间会 panic 或产生乱码 —— 中文 AGENTS.md 是常态。
        let repo = tmp("utf8");
        put(&repo, ".git", "x");
        put(&repo, "AGENTS.md", &"约".repeat(MAX_BYTES)); // 每字 3 字节，必超限

        let got = load(None, &repo); // 不得 panic
        assert!(got.truncated);
        assert!(std::str::from_utf8(got.block.as_bytes()).is_ok());
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn header_declares_precedence_explicitly() {
        let repo = tmp("hdr");
        put(&repo, ".git", "x");
        put(&repo, "AGENTS.md", "R");
        let got = load(None, &repo);
        assert!(got.block.contains("最高优先级"), "优先级要显式标注，不靠位置暗示");
        let _ = std::fs::remove_dir_all(&repo);
    }
}
