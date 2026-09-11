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

    /// 插入或更新一条（按 name 匹配）。返回 `true` 表示新增。
    pub fn upsert(&mut self, entry: ProviderEntry) -> bool {
        match self.providers.iter_mut().find(|p| p.name == entry.name) {
            Some(slot) => {
                *slot = entry;
                false
            }
            None => {
                self.providers.push(entry);
                true
            }
        }
    }

    /// 按 name 删除。返回是否真的删掉了。
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.providers.len();
        self.providers.retain(|p| p.name != name);
        self.providers.len() != before
    }

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

/// 把注册表写回用户级路径。写前先校验（重名/空名不得落盘）。
///
/// # 为什么写入要**替换整个文件**而不是追加
///
/// 注册表是"当前配置的完整事实"，不是事件流。追加会留下已删除的条目，
/// 让文件与实际生效的配置不一致。`providers.json` 不含密钥（只存变量名），
/// 所以整文件重写不涉及"把密钥写进可分享文件"的风险。
pub fn save(reg: &ProviderRegistry, path: &Path) -> Result<(), String> {
    reg.validate()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("创建目录失败：{e}"))?;
    }
    let json = serde_json::to_string_pretty(reg).map_err(|e| format!("序列化失败：{e}"))?;
    std::fs::write(path, format!("{json}\n")).map_err(|e| format!("写入失败：{e}"))
}

// ══════════════════════════════════════════════════════════════════════
// 密钥存储：与注册表**分开的文件**，权限 0600
// ══════════════════════════════════════════════════════════════════════
//
// 为什么不把密钥写进 providers.json：那个文件是"配置"，用户会分享、会进版本库
// （虽然我们拒绝项目级，但用户仍可能手动复制）。密钥必须在一个**语义上就叫密钥**
// 的文件里，权限收紧到 0600，用户看一眼就知道"这个文件不能外传"。
//
// 格式（`$NEO_HOME/provider_keys.json`）：`{"providers":{"name":"sk-..."}}`
// 用 map 而不是数组：密钥文件不需要顺序，按名字取更直接。

/// 密钥库路径。
pub fn keys_path() -> Option<PathBuf> {
    neo_home().map(|h| h.join("provider_keys.json"))
}

/// 读取密钥库。解析失败**返回空而不报错**：密钥文件损坏时，
/// 正确行为是"让用户重填"，而不是让整个应用起不来。
pub fn load_keys() -> std::collections::BTreeMap<String, String> {
    let Some(p) = keys_path() else { return Default::default() };
    let Ok(raw) = std::fs::read_to_string(&p) else { return Default::default() };
    #[derive(Deserialize)]
    struct KeysFile {
        #[serde(default)]
        providers: std::collections::BTreeMap<String, String>,
    }
    serde_json::from_str::<KeysFile>(&raw).map(|f| f.providers).unwrap_or_default()
}

/// 写入密钥库，并**把权限收紧到 0600**。
///
/// 权限是这条路径上唯一真正的安全控制：文件内容本身是明文密钥，
/// 若权限是 0644，同机其它用户就能读到。
/// Unix 上用 `OpenOptions::mode`（创建即 0600，不留"先 0644 再 chmod"的窗口）。
pub fn save_keys(keys: &std::collections::BTreeMap<String, String>) -> Result<(), String> {
    let Some(path) = keys_path() else { return Err("无法确定 NEO_HOME".into()) };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("创建目录失败：{e}"))?;
    }
    #[derive(Serialize)]
    struct KeysFile<'a> {
        providers: &'a std::collections::BTreeMap<String, String>,
    }
    let json = serde_json::to_string_pretty(&KeysFile { providers: keys })
        .map_err(|e| format!("序列化失败：{e}"))?;

    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| format!("打开密钥文件失败：{e}"))?;
        f.write_all(format!("{json}\n").as_bytes())
            .map_err(|e| format!("写入密钥失败：{e}"))?;
        // 已存在的旧文件权限可能仍是 0644，显式再收一次
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        // 非 Unix（Windows）没有 POSIX 权限位：写入但仍**如实说明**限制，
        // 而不是假装已经保护好了。
        std::fs::write(&path, format!("{json}\n")).map_err(|e| format!("写入密钥失败：{e}"))?;
    }
    Ok(())
}

/// 把用户写的 `base_url` 规范成**裸主机名 + 路径**。
///
/// # 为什么需要它
///
/// 用户会自然地写 `https://api.deepseek.com/v1`（这是 OpenAI 兼容生态的写法），
/// 而底层 HTTP 客户端需要的是**裸主机**（它用 `openssl s_client -connect host:443`
/// 建连，带 `https://` 会直接失败）。若不规范化，用户在设置页里填的 URL
/// 看着完全正确、实际却连不上 —— 这种"看起来对但用不了"最难排查。
///
/// 返回 `(host, path)`：`host` 不含 scheme、不含路径；`path` 以 `/` 开头。
/// 只处理 `http(s)://` 前缀与尾部 `/`；**不猜**别的（不做"补全 https"之类），
/// 猜错比不猜更糟。
pub fn normalize_base_url(raw: &str) -> (String, String) {
    let t = raw.trim();
    let rest = t
        .strip_prefix("https://")
        .or_else(|| t.strip_prefix("http://"))
        .unwrap_or(t);
    match rest.split_once('/') {
        Some((host, path)) => {
            let path = path.trim_end_matches('/');
            (
                host.trim_end_matches('/').to_string(),
                format!("/{path}"),
            )
        }
        None => (rest.trim_end_matches('/').to_string(), String::new()),
    }
}

/// 取某个服务商的可用密钥：**先看密钥库，再回落到环境变量**。
///
/// 顺序是刻意的：密钥库是用户在设置页里显式填的，意图更明确；
/// 环境变量是部署/CI 场景的老路径。两者都没有则返回 `None`。
pub fn key_for(entry: &ProviderEntry, store: &std::collections::BTreeMap<String, String>) -> Option<String> {
    if let Some(k) = store.get(&entry.name) {
        if !k.trim().is_empty() {
            return Some(k.clone());
        }
    }
    entry
        .api_key_env
        .as_deref()
        .and_then(|k| std::env::var(k).ok())
        .filter(|v| !v.trim().is_empty())
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
    fn upsert_and_remove_mutate_by_name() {
        let mut r = ProviderRegistry::parse(r#"{"providers":[{"name":"a"}]}"#).unwrap();
        assert!(!r.upsert(ProviderEntry {
            name: "a".into(), base_url: Some("u".into()), model: None,
            api_key_env: None, description: None, context_limit: 0, production: true,
        }), "同名应为更新而非新增");
        assert_eq!(r.get("a").unwrap().base_url.as_deref(), Some("u"));
        assert!(r.upsert(ProviderEntry {
            name: "b".into(), base_url: None, model: None,
            api_key_env: None, description: None, context_limit: 0, production: true,
        }), "新名应为新增");
        assert_eq!(r.providers.len(), 2);
        assert!(r.remove("a"));
        assert!(!r.remove("a"), "重复删除应返回 false");
        assert_eq!(r.names(), vec!["b"]);
    }

    #[test]
    fn save_refuses_to_write_an_invalid_registry() {
        // 重名不得落盘 —— 否则下次启动才报错，用户不知道是自己刚写坏的。
        let d = tmp("save-invalid");
        let mut r = ProviderRegistry::default();
        r.providers.push(ProviderEntry {
            name: "dup".into(), base_url: None, model: None, api_key_env: None,
            description: None, context_limit: 0, production: true,
        });
        r.providers.push(ProviderEntry {
            name: "dup".into(), base_url: None, model: None, api_key_env: None,
            description: None, context_limit: 0, production: true,
        });
        let p = d.join("providers.json");
        assert!(save(&r, &p).is_err(), "非法注册表不该写盘");
        assert!(!p.exists(), "校验失败时不该创建文件");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn save_then_load_roundtrip() {
        // 用户级目录与 cwd **必须分开**：同一目录会触发"项目级被拒"的守卫
        // （第一版就是这么写的，恰好说明那条守卫真的在起作用）。
        let d = tmp("roundtrip");
        let cwd = tmp("roundtrip-cwd");
        let mut r = ProviderRegistry::default();
        r.upsert(ProviderEntry {
            name: "gw".into(), base_url: Some("https://gw/v1".into()),
            model: Some("m".into()), api_key_env: Some("GW_KEY".into()),
            description: Some("网关".into()), context_limit: 128000, production: true,
        });
        let p = d.join("providers.json");
        save(&r, &p).unwrap();
        match load_from(&p, &cwd) {
            LoadOutcome::Loaded(got) => assert_eq!(got, r),
            other => panic!("回读失败：{other:?}"),
        }
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(&cwd);
    }

    #[test]
    fn base_url_is_normalized_to_bare_host() {
        // 用户会写 https://host/v1；底层要的是裸 host（openssl -connect host:443）。
        assert_eq!(
            normalize_base_url("https://api.deepseek.com/v1"),
            ("api.deepseek.com".to_string(), "/v1".to_string())
        );
        assert_eq!(
            normalize_base_url("api.deepseek.com"),
            ("api.deepseek.com".to_string(), String::new())
        );
        // 尾部斜杠不该留下空路径段
        assert_eq!(
            normalize_base_url("https://gw.example.com/v1/"),
            ("gw.example.com".to_string(), "/v1".to_string())
        );
        assert_eq!(
            normalize_base_url("http://localhost:8080"),
            ("localhost:8080".to_string(), String::new())
        );
    }

    #[test]
    fn key_store_prefers_stored_key_over_env() {
        use std::collections::BTreeMap;
        let e = ProviderEntry {
            name: "gw".into(), base_url: None, model: None,
            api_key_env: Some("NEO_TEST_KEY_XYZ".into()), description: None,
            context_limit: 0, production: true,
        };
        let mut store = BTreeMap::new();
        // 只配环境变量
        std::env::set_var("NEO_TEST_KEY_XYZ", "from-env");
        assert_eq!(key_for(&e, &store).as_deref(), Some("from-env"));
        // 密钥库里有值时应优先于环境变量（用户在设置页显式填的意图更明确）
        store.insert("gw".into(), "from-store".into());
        assert_eq!(key_for(&e, &store).as_deref(), Some("from-store"));
        // 空串不算"配了"
        store.insert("gw".into(), "   ".into());
        assert_eq!(key_for(&e, &store).as_deref(), Some("from-env"));
        std::env::remove_var("NEO_TEST_KEY_XYZ");
    }

    #[cfg(unix)]
    #[test]
    fn key_file_is_written_with_0600() {
        // 明文密钥文件的权限是这条路径上**唯一**的安全控制，
        // 必须可验证：64 4 意味着同机其它用户能读到。
        use std::os::unix::fs::PermissionsExt;
        let d = std::env::temp_dir().join(format!("neo-keys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::env::set_var("NEO_HOME", &d);
        let mut m = std::collections::BTreeMap::new();
        m.insert("gw".to_string(), "sk-secret".to_string());
        save_keys(&m).unwrap();
        let mode = std::fs::metadata(keys_path().unwrap()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "密钥文件权限必须是 0600，实得 {mode:o}");
        assert_eq!(load_keys().get("gw").map(String::as_str), Some("sk-secret"));
        std::env::remove_var("NEO_HOME");
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
