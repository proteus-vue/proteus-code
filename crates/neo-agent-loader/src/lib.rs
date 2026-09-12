//! L3 PROVIDER —— 子代理(Agent)定义的发现与加载。
//!
//! # 格式(`.neo/agents/*.md`,工作区级;用户级 `~/.neo/agents/` 同构)
//!
//! ```markdown
//! ---
//! name: reviewer
//! tools: bash, apply_patch
//! model: deepseek        ← 可选;省略 = 继承主内核当前模型
//! ---
//! 你是一个只负责审查的助手……(系统提示词正文)
//! ```
//!
//! 与技能加载器(SKILL.md)同一套纪律:
//! - frontmatter **未闭合不予采信**(残缺文件按残缺处理,防止以错误
//!   的身份注册);
//! - 文件名作兜底名(所有 agent 文件名各不相同,与 SKILL.md 全同名
//!   的情形不同,但兜底规则保持一致);
//! - 递归深度固定 2(防 node_modules 之类的依赖树被整棵扫掉);
//! - 重名 = 后加载的跳过 + 记入报告(加载报告如实呈现,不静默)。

use neo_core::agents::AgentSpec;
use std::path::{Path, PathBuf};

/// `NEO_HOME`(与 neo-instructions / neo-mcp 同一解析规则)。
pub fn neo_home() -> Option<PathBuf> {
    std::env::var_os("NEO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .map(|h| if h.ends_with(".neo") { h } else { h.join(".neo") })
}

/// 一个加载出来的子代理。
pub struct LoadedAgent {
    pub spec: AgentSpec,
    /// 来源文件(诊断用)
    pub source: PathBuf,
}

/// 加载报告:代理清单 + 非致命问题(重名跳过、frontmatter 残缺等)。
pub struct LoadReport {
    pub agents: Vec<LoadedAgent>,
    pub problems: Vec<String>,
}

/// 递归扫描目录(深度 ≤ 2),取 *.md 解析为 AgentSpec。
pub fn load_dirs(dirs: &[PathBuf]) -> LoadReport {
    let mut agents: Vec<LoadedAgent> = Vec::new();
    let mut problems = Vec::new();
    let mut seen: std::collections::BTreeMap<String, ()> = Default::default();

    for dir in dirs {
        for entry in walk(dir, 0) {
            if entry.extension().map(|e| e != "md").unwrap_or(true) {
                continue;
            }
            let raw = match std::fs::read_to_string(&entry) {
                Ok(s) => s,
                Err(e) => {
                    problems.push(format!("读取 {} 失败:{e}", entry.display()));
                    continue;
                }
            };
            let (spec, warning) = parse_agent_markdown(&raw, &entry);
            if let Some(w) = warning {
                problems.push(w);
            }
            if seen.contains_key(&spec.name) {
                problems.push(format!(
                    "子代理重名跳过:{}(来自 {})",
                    spec.name,
                    entry.display()
                ));
                continue;
            }
            seen.insert(spec.name.clone(), ());
            agents.push(LoadedAgent { spec, source: entry.clone() });
        }
    }
    LoadReport { agents, problems }
}

/// 深度受控递归(≤ 2 层):agent 目录里若出现子目录(乃至误装的
/// node_modules),无限递归会把依赖树整棵扫掉 —— 固定深度是防线。
fn walk(dir: &Path, depth: usize) -> Vec<PathBuf> {
    if depth > 2 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p, depth + 1));
        } else {
            out.push(p);
        }
    }
    out.sort(); // 稳定顺序(同名跳过与报告的确定性依赖它)
    out
}

/// 解析一个 agent Markdown 文件。
/// 返回 (spec, 警告)。警告 = frontmatter 残缺等"仍加载但有话说"的情形。
fn parse_agent_markdown(raw: &str, source: &Path) -> (AgentSpec, Option<String>) {
    let fallback = source
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    // frontmatter:必须以 --- 开头,且存在**闭合**的 --- 才采信
    if !raw.starts_with("---") {
        return (spec_from(fallback, None, Vec::new(), raw), None);
    }
    let Some(rest) = raw.strip_prefix("---") else {
        return (spec_from(fallback, None, Vec::new(), raw), None);
    };
    let Some((fm, body)) = rest.split_once("\n---") else {
        // 未闭合:整份按正文处理,frontmatter 不采信
        return (
            spec_from(fallback, None, Vec::new(), raw),
            Some(format!("frontmatter 未闭合,不采信:{}", source.display())),
        );
    };
    // 跳过闭合行的剩余部分(可能带 \r)
    let body = body.trim_start_matches(['-', '\n', '\r']);
    let body = body.strip_prefix('\n').unwrap_or(body);

    let mut name = None;
    let mut model = None;
    let mut tools = Vec::new();
    for line in fm.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("name:") {
            let v = v.trim();
            if !v.is_empty() {
                name = Some(v.to_string());
            }
        } else if let Some(v) = line.strip_prefix("model:") {
            let v = v.trim();
            if !v.is_empty() {
                model = Some(v.to_string());
            }
        } else if let Some(v) = line.strip_prefix("tools:") {
            tools = v
                .split(&[',', ' '][..])
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect();
        }
    }

    (
        AgentSpec {
            name: name.unwrap_or(fallback),
            model,
            tools,
            body: body.trim().to_string(),
        },
        None,
    )
}

fn spec_from(name: String, model: Option<String>, tools: Vec<String>, body: &str) -> AgentSpec {
    AgentSpec { name, model, tools, body: body.trim().to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_agent(dir: &Path, file: &str, content: &str) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let p = dir.join(file);
        std::fs::write(&p, content).unwrap();
        p
    }

    #[test]
    fn parses_frontmatter_name_tools_and_body() {
        let raw = "---\nname: reviewer\ntools: bash, apply_patch\n---\n你负责审查。\n";
        let (spec, warn) = parse_agent_markdown(raw, Path::new("reviewer.md"));
        assert!(warn.is_none());
        assert_eq!(spec.name, "reviewer");
        assert_eq!(spec.tools, vec!["bash", "apply_patch"]);
        assert_eq!(spec.body, "你负责审查。");
        assert_eq!(spec.model, None, "model 省略 = 继承主内核");
    }

    #[test]
    fn unclosed_frontmatter_is_not_trusted() {
        // 半截 frontmatter 是残缺文件:不采信其中的 name,整份按正文
        let raw = "---\nname: bad\n正文(没有结束 ---)";
        let (spec, warn) = parse_agent_markdown(raw, Path::new("fallback-name.md"));
        assert_eq!(spec.name, "fallback-name", "兜底名来自文件名");
        assert!(spec.body.contains("name: bad"), "原样进正文");
        assert!(warn.is_some(), "要有警告");
    }

    #[test]
    fn duplicates_are_skipped_with_a_report_entry() {
        let dir = std::env::temp_dir().join(format!("neo-agent-dup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write_agent(&dir, "a.md", "---\nname: same\n---\n第一个");
        write_agent(&dir, "b.md", "---\nname: same\n---\n第二个");
        let report = load_dirs(&[dir.clone()]);
        assert_eq!(report.agents.len(), 1, "重名跳过");
        assert_eq!(report.agents[0].spec.body, "第一个", "先到先得");
        assert!(report.problems.iter().any(|p| p.contains("重名")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn recursion_is_capped_at_two_levels() {
        let dir = std::env::temp_dir().join(format!("neo-agent-deep-{}", std::process::id()));
        let deep = dir.join("a/b/c/d");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("deep.md"), "---\nname: deep\n---\n深层").unwrap();
        write_agent(&dir, "top.md", "---\nname: top\n---\n顶层");
        let report = load_dirs(&[dir.clone()]);
        assert!(
            !report.agents.iter().any(|a| a.spec.name == "deep"),
            "第 3 层以下不得被扫到"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
