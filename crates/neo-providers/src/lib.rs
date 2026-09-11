//! L3 PROVIDER —— 服务商注册表：从**用户级** JSON 读出多个 OpenAI 兼容服务商。
//!
//! # 为什么必须有这一层（而不只是环境变量）
//!
//! 之前一个 provider 完全由环境变量决定（`DEEPSEEK_API_KEY` + `DEEPSEEK_BASE_URL`）。
//! 这有两个问题：
//! 1. **多服务商不可配**：想同时挂 DeepSeek 官方的和某个自建网关，环境变量做不到。
//! 2. **密钥无处安放**：让用户把 key 写进项目里的配置文件是危险的 ——
//!    一次 `git add .` 就泄露了。所以注册表**只从用户级路径读**，
//!    项目级路径即使存在也被显式拒绝（见 [`resolve_path`] 的返回值）。
//!
//! # 与 `neo-config` 的四级契约一致
//!
//! 契约（`docs/neo-plan/05-验证/checks/check_config_layers.py` C2）说
//! 「安全敏感键只允许在用户级设置，项目级忽略」。provider 密钥是最典型的
//! 安全敏感键，所以这里把该契约落到实现上：**只读用户级**，
//! 项目级文件若存在则报错说明（而不是静默忽略 —— 用户会以为它生效了）。
//!
//! # 文件格式（`$NEO_HOME/providers.json`）
//!
//! ```json
//! {
//!   "providers": [
//!     {
//!       "name": "deepseek",
//!       "base_url": "https://api.deepseek.com",
//!       "model": "deepseek-chat",
//!       "api_key_env": "DEEPSEEK_API_KEY",
//!       "description": "DeepSeek 官方",
//!       "context_limit": 64000
//!     }
//!   ]
//! }
//! ```
//!
//! `api_key_env` 存的是**环境变量名**而不是密钥本身：注册表进版本库也安全，
//! 密钥仍在环境里（或另一个 0600 的文件里）。这是刻意的取舍 ——
//! 把密钥写进 JSON 文件会更"方便"，也更危险。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 一个服务商条目（注册表里的一条）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderEntry {
    /// 唯一名字（用于 `--provider` 与 `/model` 列表展示）。
    pub name: String,
    /// OpenAI 兼容 base URL，如 `https://api.deepseek.com`。
    #[serde(default)]
    pub base_url: Option<String>,
    /// 默认模型名。
    #[serde(default)]
    pub model: Option<String>,
    /// **环境变量名**（不是密钥本身）。
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// 上下文窗口（0 = 未知，UI 不显示百分比 —— 宁可不显示也不给假数字）。
    #[serde(default)]
    pub context_limit: u64,
    /// 是否标记为可供真实任务使用。
    #[serde(default = "yes")]
    pub production: bool,
}

fn yes() -> bool { true }

/// 注册表（用户级 providers.json 的内容）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRegistry {
    #[serde(default)]
    pub providers: Vec<ProviderEntry>,
}

impl ProviderRegistry {
    /// 解析 JSON。**未知字段不报错**（前向兼容：新版本加的键不该让旧版本拒载），
    /// 但结构错误（不是对象、providers 不是数组）必须报错。
    pub fn parse(raw: &str) -> Result<Self, String> {
        serde_json::from_str(raw).map_err(|e| format!("providers.json 解析失败：{e}"))
    }

    pub fn get(&self, name: &str) -> Option<&ProviderEntry> {
        // 名字**大小写敏感**：`DeepSeek` 与 `deepseek` 视为不同条目，
        // 与技能注册表同一取舍（猜错会连到别的服务商，比找不到更糟）。
        self.providers.iter().find(|p| p.name == name)
    }

    pub fn names(&self) -> Vec<&str> {
        self.providers.iter().map(|p| p.name.as_str()).collect()
    }

    pub fn is_empty(&self) -> bool { self.providers.is_empty() }

    /// 校验条目：名字非空、无重名。**重名必须报错而不是"后者覆盖"** ——
    /// 静默覆盖会让用户以为配了两个服务商，实际只有一个能连上。
    pub fn validate(&self) -> Result<(), String> {
        let mut seen = std::collections::BTreeSet::new();
        for p in &self.providers {
            if p.name.trim().is_empty() {
                return Err("providers.json 里有条目 name 为空".into());
            }
            if !seen.insert(p.name.clone()) {
                return Err(format!("providers.json 里服务商重名：{}", p.name));
            }
        }
        Ok(())
    }
}

/// `$NEO_HOME`（默认 `~/.neo`），与 trust / skills / instructions 同一约定。
pub fn neo_home() -> Option<PathBuf> {
    std::env::var_os("NEO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .map(|h| if h.ends_with(".neo") { h } else { h.join(".neo") })
}

/// 注册表的**唯一**读取路径：用户级。
pub fn resolve_path() -> Option<PathBuf> {
    neo_home().map(|h| h.join("providers.json"))
}

/// 读取结果：为什么读不出来，要能说清（否则用户面对空列表无从下手）。
#[derive(Debug)]
pub enum LoadOutcome {
    /// 读到了并校验通过
    Loaded(ProviderRegistry),
    /// 没有配置文件 —— 不是错误，用环境变量那条老路即可
    Absent,
    /// 有文件但读不动/不合法（含项目级被拒）
    Failed(String),
}

/// 从用户级路径加载。**只在用户级**：项目级 `./providers.json` 若存在，
/// 明确报错而不是静默忽略 —— 用户会很自然地建一个项目级文件，
/// 静默忽略会让他对着"配置了却不生效"百思不得其解。
pub fn load_from(path: &Path, cwd: &Path) -> LoadOutcome {
    let risky = cwd.join("providers.json");
    if risky.is_file() {
        return LoadOutcome::Failed(format!(
            "拒绝读取项目级 {}：服务商密钥属安全敏感配置，只允许放在用户级 {}",
            risky.display(),
            path.display()
        ));
    }
    let raw = match std::fs::read_to_string(path) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return LoadOutcome::Absent,
        Err(e) => return LoadOutcome::Failed(format!("读取 {} 失败：{e}", path.display())),
    };
    match ProviderRegistry::parse(&raw).and_then(|r| r.validate().map(|_| r)) {
        Ok(r) => LoadOutcome::Loaded(r),
        Err(e) => LoadOutcome::Failed(e),
    }
}

/// 便捷入口：按约定路径 + 当前目录加载。
pub fn load(cwd: &Path) -> LoadOutcome {
    match resolve_path() {
        Some(p) => load_from(&p, cwd),
        None => LoadOutcome::Absent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("neo-prov-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn parses_a_minimal_registry() {
        let raw = r#"{"providers":[
            {"name":"deepseek","base_url":"https://api.deepseek.com","model":"deepseek-chat",
             "api_key_env":"DEEPSEEK_API_KEY","context_limit":64000}
        ]}"#;
        let r = ProviderRegistry::parse(raw).unwrap();
        assert_eq!(r.names(), vec!["deepseek"]);
        let p = r.get("deepseek").unwrap();
        assert_eq!(p.api_key_env.as_deref(), Some("DEEPSEEK_API_KEY"));
        assert!(p.production, "production 默认 true");
    }

    #[test]
    fn unknown_fields_are_tolerated_but_bad_shape_is_not() {
        // 前向兼容：多出来的键不报错
        let raw = r#"{"providers":[{"name":"x","future_field":1}],"future_top":true}"#;
        assert!(ProviderRegistry::parse(raw).is_ok(), "未知字段应容忍");
        // 结构错误必须报错，不能静默得到空注册表
        assert!(ProviderRegistry::parse(r#"{"providers":{}}"#).is_err());
        assert!(ProviderRegistry::parse("not json").is_err());
    }

    #[test]
    fn duplicate_names_are_rejected_not_silently_overwritten() {
        let raw = r#"{"providers":[{"name":"dup"},{"name":"dup"}]}"#;
        let r = ProviderRegistry::parse(raw).unwrap();
        let err = r.validate().unwrap_err();
        assert!(err.contains("重名"), "重名必须报错：{err}");
    }

    #[test]
    fn empty_name_is_rejected() {
        let raw = r#"{"providers":[{"name":"  "}]}"#;
        let r = ProviderRegistry::parse(raw).unwrap();
        assert!(r.validate().is_err());
    }

    #[test]
    fn project_level_file_is_refused_loudly() {
        // 项目级的 providers.json 必须**明确拒绝**，不能静默忽略：
        // 用户会很自然地建一个，静默忽略会让他对着"配了不生效"想不通。
        let user = tmp("user");
        let proj = tmp("proj");
        std::fs::write(proj.join("providers.json"), r#"{"providers":[]}"#).unwrap();
        match load_from(&user.join("providers.json"), &proj) {
            LoadOutcome::Failed(msg) => {
                assert!(msg.contains("只允许放在用户级"), "应说清拒绝原因：{msg}");
            }
            other => panic!("项目级应被拒，实得 {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&user);
        let _ = std::fs::remove_dir_all(&proj);
    }

    #[test]
    fn absent_file_is_not_an_error() {
        let d = tmp("absent");
        match load_from(&d.join("nope.json"), &d) {
            LoadOutcome::Absent => {}
            other => panic!("缺文件应是 Absent，实得 {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn name_lookup_is_case_sensitive() {
        let r = ProviderRegistry::parse(r#"{"providers":[{"name":"DeepSeek"},{"name":"deepseek"}]}"#)
            .unwrap();
        r.validate().unwrap();
        assert_eq!(r.get("deepseek").unwrap().name, "deepseek");
        assert!(r.get("DEEPSEEK").is_none(), "不做大小写模糊匹配");
    }
}
