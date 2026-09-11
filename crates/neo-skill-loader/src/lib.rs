//! L3 PROVIDER —— 技能加载器：把磁盘上的技能目录变成 `SkillRegistry`。
//!
//! # 目录契约（与 docs/ 下现有的技能布局一致）
//!
//! 两种布局都支持，因为两种都真实存在：
//! 1. 一个目录下直接放 `SKILL.md`（单技能目录，如 `docs/ai-efficiency-rules/`）。
//! 2. 一个目录下放**子目录**，每个子目录里一个 `SKILL.md`
//!    （技能集合，如 zcode 的 `skills/<name>/SKILL.md`）。
//!
//! 递归深度**固定为 2**：`root/*/SKILL.md`。不做无限递归 ——
//! 技能目录里一旦有 `node_modules`，无限递归会把整棵依赖树扫一遍，
//! 既慢又会把依赖包里的文档当成技能加载进来（真实踩过的形态）。
//!
//! # 失败策略：坏目录不阻断好技能
//!
//! 单个 `SKILL.md` 读不出（编码/权限）不影响其余技能；
//! 名字冲突时**后加载的覆盖先加载的**，并如实记进 [`LoadReport::replacements`] ——
//! 静默覆盖会让用户以为加载了两个技能，实际只有一个生效。

use neo_core::skills::{Skill, SkillRegistry};
use std::path::{Path, PathBuf};

/// 加载结果：注册表 + 发生了什么（供用户/日志查看）。
#[derive(Debug, Default, Clone)]
pub struct LoadReport {
    /// 成功加载的技能名（按加载顺序）。
    pub loaded: Vec<String>,
    /// 名字冲突被后来者覆盖的（`(名字, 被覆盖的文件)`）。
    pub replacements: Vec<(String, PathBuf)>,
    /// 读取失败的文件与原因（**不中断加载**）。
    pub failures: Vec<(PathBuf, String)>,
}

impl LoadReport {
    /// 一行摘要（TUI/CLI 展示用）。空加载时说明"一个都没找到"，
    /// 而不是打一行空字符串让用户以为加载器没跑。
    pub fn summary(&self) -> String {
        if self.loaded.is_empty() && self.failures.is_empty() {
            return "未找到任何技能（SKILL.md）".to_string();
        }
        let mut s = format!("已加载 {} 个技能", self.loaded.len());
        if !self.replacements.is_empty() {
            s.push_str(&format!("；{} 个被覆盖", self.replacements.len()));
        }
        if !self.failures.is_empty() {
            s.push_str(&format!("；{} 个读取失败", self.failures.len()));
        }
        s
    }
}

/// 从若干根目录加载技能。后出现的根目录覆盖先出现的同名技能。
pub fn load_roots(roots: &[PathBuf]) -> (SkillRegistry, LoadReport) {
    let mut skills: Vec<Skill> = Vec::new();
    let mut report = LoadReport::default();

    for root in roots {
        load_one_root(root, &mut skills, &mut report);
    }

    (SkillRegistry::from_vec(skills), report)
}

/// 默认技能根目录：`$NEO_HOME/skills`（默认 `~/.neo/skills`）
/// 加上**仓库内**的 `docs/*/SKILL.md`。
///
/// 为什么把仓库内的 docs/ 也算上：`docs/ai-efficiency-rules/SKILL.md` 是这个项目
/// 自己挂的效率规范 —— 用户 `$ai-efficiency-rules` 引用它时应当能命中，
/// 而不需要先手动拷贝到 home 目录。`NEO_SKILL_DIR` 可覆盖（测试用）。
pub fn default_roots(cwd: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(d) = std::env::var_os("NEO_SKILL_DIR") {
        roots.push(PathBuf::from(d));
        return roots; // 显式指定即独占，避免测试被 home 目录污染
    }
    let home = std::env::var_os("NEO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from));
    if let Some(h) = home {
        roots.push(h.join(".neo").join("skills"));
    }
    roots.push(cwd.join("docs"));
    roots
}

fn load_one_root(root: &Path, out: &mut Vec<Skill>, report: &mut LoadReport) {
    if !root.is_dir() {
        return;
    }
    // 布局 1：root/SKILL.md
    let direct = root.join("SKILL.md");
    if direct.is_file() {
        push_file(&direct, root, out, report);
    }
    // 布局 2：root/<child>/SKILL.md（深度 2，不递归更深）
    let Ok(entries) = std::fs::read_dir(root) else { return };
    let mut children: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    // 排序保证加载顺序确定（否则目录遍历顺序随文件系统变化，
    // 同名覆盖的"谁赢"就不可复现）。
    children.sort();
    for child in children {
        let f = child.join("SKILL.md");
        if f.is_file() {
            push_file(&f, &child, out, report);
        }
    }
}

fn push_file(path: &Path, stem_src: &Path, out: &mut Vec<Skill>, report: &mut LoadReport) {
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            report.failures.push((path.to_path_buf(), e.to_string()));
            return;
        }
    };
    // 名字兜底用**文件夹名**而不是文件名：所有文件都叫 SKILL.md，
    // 用它当兜底名字会让"没有 frontmatter 的技能"全部同名而互相覆盖。
    let fallback = stem_src
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "skill".to_string());
    let skill = Skill::parse(&raw, &fallback);

    if let Some(prev) = out.iter_mut().find(|s| s.name == skill.name) {
        report.replacements.push((skill.name.clone(), path.to_path_buf()));
        *prev = skill;
    } else {
        report.loaded.push(skill.name.clone());
        out.push(skill);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("neo-skill-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write(path: &Path, content: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn loads_single_and_collection_layouts() {
        let root = tmp("layouts");
        write(&root.join("SKILL.md"), "---\nname: direct\n---\n正文A\n");
        write(&root.join("sub/SKILL.md"), "---\nname: sub-skill\n---\n正文B\n");

        let (reg, report) = load_roots(&[root.clone()]);
        assert!(report.failures.is_empty(), "不该有读取失败: {:?}", report.failures);
        assert_eq!(reg.get("direct").unwrap().body, "正文A\n");
        assert_eq!(reg.get("sub-skill").unwrap().body, "正文B\n");
        assert_eq!(report.loaded.len(), 2);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn no_frontmatter_uses_folder_name_not_skill_md() {
        // 兜底名必须用文件夹名：两个无 frontmatter 的技能若都用 "SKILL.md"
        // 当名字，会互相覆盖 —— 用户看到"加载 2 个"实际只有 1 个。
        let root = tmp("fallback");
        write(&root.join("alpha/SKILL.md"), "无 frontmatter 的正文\n");
        write(&root.join("beta/SKILL.md"), "另一篇\n");

        let (reg, report) = load_roots(&[root.clone()]);
        assert_eq!(report.loaded.len(), 2, "两篇都应独立存在");
        assert!(reg.get("alpha").is_some(), "兜底名应为文件夹名 alpha");
        assert!(reg.get("beta").is_some());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn duplicate_name_reports_replacement() {
        let root = tmp("dup");
        write(&root.join("a/SKILL.md"), "---\nname: same\n---\n先\n");
        write(&root.join("b/SKILL.md"), "---\nname: same\n---\n后\n");

        let (reg, report) = load_roots(&[root.clone()]);
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.get("same").unwrap().body, "后\n", "后加载者生效");
        assert_eq!(report.replacements.len(), 1, "覆盖必须如实记录");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_root_is_not_an_error() {
        let (reg, report) = load_roots(&[PathBuf::from("/nonexistent/neo/skills")]);
        assert!(reg.is_empty());
        assert!(report.failures.is_empty(), "目录不存在不是失败，只是没有");
        assert_eq!(report.summary(), "未找到任何技能（SKILL.md）");
    }

    #[test]
    fn does_not_recurse_into_node_modules() {
        // 深度固定 2：root/node_modules/<deep>/SKILL.md 不该被加载。
        let root = tmp("depth");
        write(&root.join("node_modules/pkg/SKILL.md"), "---\nname: dep-doc\n---\nx\n");
        write(&root.join("real/SKILL.md"), "---\nname: real\n---\ny\n");

        let (reg, _) = load_roots(&[root.clone()]);
        assert!(reg.get("real").is_some());
        assert!(reg.get("dep-doc").is_none(), "不得把依赖包里的文档当技能");
        let _ = std::fs::remove_dir_all(&root);
    }
}
