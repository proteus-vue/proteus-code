//! 本地插件市场（Codex `marketplace/*` + `plugin/*` 的无账号子集）。
//!
//! # 与账号体系的边界
//!
//! 插件市场是**本地磁盘能力**：注册市场源、安装/卸载插件、读清单与技能 ——
//! 与账号登录无关。`plugin/share/*` 远程分享绑身份，**刻意不做**（见下）。
//!
//! # 目录契约（NEO 本地，不是 Codex 私有格式的复刻）
//!
//! ```text
//! $NEO_HOME/marketplaces.json
//!   { "marketplaces": [ { "name": "local", "source": "/abs/path" } ] }
//!
//! $NEO_HOME/plugins/installed.json
//!   { "plugins": [ { "pluginId": "foo", "marketplace": "local", "path": "..." } ] }
//!
//! <source>/plugins/<id>/plugin.json
//!   { "id": "foo", "name": "Foo", "version": "0.1.0", "skills": ["foo"] }
//! <source>/plugins/<id>/skills/**/SKILL.md   （可选）
//! <source>/plugins/<id>/agents/*.md          （可选）
//! ```
//!
//! 安装 = 把 `<source>/plugins/<id>/` **复制**到 `$NEO_HOME/plugins/<id>/`。
//! 不做远程 git 拉取（本轮）：`source` 必须是已存在的本地路径，否则报错说明原因。
//!
//! # 有界与安全
//!
//! - 配置与清单 JSON 解析失败 = 错误，不静默吞
//! - 插件 id 走与 session 相同的文件名消毒思路（字母数字 `-` `_`）
//! - 不执行插件内的任何可执行文件 —— 只复制资源（skills/agents 文本）

use std::path::{Path, PathBuf};

/// `NEO_HOME`（与 skill-loader / instructions 同一规则）。
pub fn neo_home() -> Option<PathBuf> {
    std::env::var_os("NEO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .map(|h| if h.ends_with(".neo") { h } else { h.join(".neo") })
}

fn marketplaces_path() -> Option<PathBuf> {
    neo_home().map(|h| h.join("marketplaces.json"))
}

fn installed_path() -> Option<PathBuf> {
    neo_home().map(|h| h.join("plugins").join("installed.json"))
}

fn plugins_root() -> Option<PathBuf> {
    neo_home().map(|h| h.join("plugins"))
}

fn sanitize_id(id: &str) -> String {
    let mut s: String = id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    while s.starts_with('.') {
        s.remove(0);
    }
    if s.is_empty() {
        s.push_str("unnamed");
    }
    s.truncate(64);
    s
}

fn read_json(path: &Path) -> Result<serde_json::Value, String> {
    if !path.exists() {
        return Ok(serde_json::json!({}));
    }
    let raw = std::fs::read_to_string(path).map_err(|e| format!("读取 {}: {e}", path.display()))?;
    serde_json::from_str(&raw).map_err(|e| format!("解析 {}: {e}", path.display()))
}

fn write_json(path: &Path, value: &serde_json::Value) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("创建 {}: {e}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    std::fs::write(path, text + "\n").map_err(|e| format!("写入 {}: {e}", path.display()))
}

fn load_marketplaces() -> Result<Vec<serde_json::Value>, String> {
    let path = marketplaces_path().ok_or("NEO_HOME/HOME 不可用，无法读市场配置")?;
    let v = read_json(&path)?;
    Ok(v.get("marketplaces")
        .and_then(|a| a.as_array())
        .cloned()
        .unwrap_or_default())
}

fn save_marketplaces(list: &[serde_json::Value]) -> Result<(), String> {
    let path = marketplaces_path().ok_or("NEO_HOME/HOME 不可用，无法写市场配置")?;
    write_json(&path, &serde_json::json!({ "marketplaces": list }))
}

fn load_installed() -> Result<Vec<serde_json::Value>, String> {
    let path = installed_path().ok_or("NEO_HOME/HOME 不可用，无法读已安装插件")?;
    let v = read_json(&path)?;
    Ok(v.get("plugins")
        .and_then(|a| a.as_array())
        .cloned()
        .unwrap_or_default())
}

fn save_installed(list: &[serde_json::Value]) -> Result<(), String> {
    let path = installed_path().ok_or("NEO_HOME/HOME 不可用，无法写已安装插件")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    write_json(&path, &serde_json::json!({ "plugins": list }))
}

/// 注册本地市场源。`source` 必须是已存在的目录（不拉 git）。
pub fn marketplace_add(name: &str, source: &str) -> Result<serde_json::Value, String> {
    let name = name.trim();
    let source = source.trim();
    if name.is_empty() || source.is_empty() {
        return Err("marketplace/add 需要非空 name 与 source".into());
    }
    let src = PathBuf::from(source);
    if !src.is_dir() {
        return Err(format!(
            "source 必须是已存在的本地目录（本轮不拉远程 git）：{}",
            src.display()
        ));
    }
    let mut list = load_marketplaces()?;
    let entry = serde_json::json!({ "name": name, "source": src.display().to_string() });
    // 同名覆盖
    list.retain(|m| m.get("name").and_then(|v| v.as_str()) != Some(name));
    list.push(entry.clone());
    save_marketplaces(&list)?;
    Ok(entry)
}

pub fn marketplace_remove(name: &str) -> Result<bool, String> {
    let mut list = load_marketplaces()?;
    let before = list.len();
    list.retain(|m| m.get("name").and_then(|v| v.as_str()) != Some(name));
    if list.len() == before {
        return Ok(false);
    }
    save_marketplaces(&list)?;
    Ok(true)
}

/// 刷新本地市场目录里的插件清单（不复制内容）。
pub fn marketplace_upgrade(name: Option<&str>) -> Result<serde_json::Value, String> {
    let list = load_marketplaces()?;
    let mut scanned = Vec::new();
    for m in &list {
        let mname = m.get("name").and_then(|v| v.as_str()).unwrap_or_default();
        if let Some(want) = name {
            if mname != want {
                continue;
            }
        }
        let source = m.get("source").and_then(|v| v.as_str()).unwrap_or("");
        let plugins = scan_marketplace_plugins(Path::new(source));
        scanned.push(serde_json::json!({
            "name": mname,
            "source": source,
            "plugins": plugins,
        }));
    }
    Ok(serde_json::json!({ "marketplaces": scanned }))
}

fn scan_marketplace_plugins(source: &Path) -> Vec<serde_json::Value> {
    let root = source.join("plugins");
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in entries.flatten() {
        let dir = e.path();
        if !dir.is_dir() {
            continue;
        }
        if let Some(p) = read_plugin_json(&dir) {
            out.push(p);
        }
    }
    out.sort_by_key(|p| {
        p.get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    });
    out
}

fn read_plugin_json(dir: &Path) -> Option<serde_json::Value> {
    let raw = std::fs::read_to_string(dir.join("plugin.json")).ok()?;
    let mut v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let fallback = dir
        .file_name()
        .map(|s| sanitize_id(&s.to_string_lossy()))
        .unwrap_or_else(|| "unnamed".into());
    let id = v
        .get("id")
        .and_then(|x| x.as_str())
        .map(sanitize_id)
        .unwrap_or(fallback);
    if let Some(obj) = v.as_object_mut() {
        obj.insert("id".into(), serde_json::Value::String(id));
        obj.insert(
            "path".into(),
            serde_json::Value::String(dir.display().to_string()),
        );
    }
    Some(v)
}

/// 列出可安装插件（扫所有市场）+ 安装状态。
pub fn plugin_list() -> Result<serde_json::Value, String> {
    let markets = load_marketplaces()?;
    let installed = load_installed()?;
    let mut installed_ids: std::collections::BTreeSet<String> = installed
        .iter()
        .filter_map(|p| p.get("pluginId").and_then(|v| v.as_str()).map(sanitize_id))
        .collect();
    let mut available = Vec::new();
    for m in &markets {
        let source = m.get("source").and_then(|v| v.as_str()).unwrap_or("");
        for mut p in scan_marketplace_plugins(Path::new(source)) {
            let id = p
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if let Some(obj) = p.as_object_mut() {
                obj.insert(
                    "installed".into(),
                    serde_json::Value::Bool(installed_ids.contains(&id)),
                );
                obj.insert(
                    "marketplace".into(),
                    m.get("name").cloned().unwrap_or(serde_json::Value::Null),
                );
            }
            available.push(p);
            installed_ids.insert(id);
        }
    }
    Ok(serde_json::json!({
        "plugins": available,
        "marketplaces": markets,
    }))
}

pub fn plugin_installed() -> Result<serde_json::Value, String> {
    let installed = load_installed()?;
    // 仍存在的目录才算
    let alive: Vec<serde_json::Value> = installed
        .into_iter()
        .filter(|p| {
            p.get("path")
                .and_then(|v| v.as_str())
                .map(|s| Path::new(s).is_dir())
                .unwrap_or(false)
        })
        .collect();
    Ok(serde_json::json!({ "plugins": alive }))
}

fn find_in_marketplaces(plugin_name: &str) -> Result<(serde_json::Value, PathBuf), String> {
    let markets = load_marketplaces()?;
    for m in &markets {
        let source = m.get("source").and_then(|v| v.as_str()).unwrap_or("");
        for p in scan_marketplace_plugins(Path::new(source)) {
            let id = p.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
            if id == plugin_name || name == plugin_name {
                let path = p
                    .get("path")
                    .and_then(|v| v.as_str())
                    .map(PathBuf::from)
                    .ok_or_else(|| format!("插件 {plugin_name} 缺少 path"))?;
                let mname = m.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let mut out = p;
                if let Some(obj) = out.as_object_mut() {
                    obj.insert(
                        "marketplace".into(),
                        serde_json::Value::String(mname.to_string()),
                    );
                }
                return Ok((out, path));
            }
        }
    }
    Err(format!(
        "在已注册市场中找不到插件「{plugin_name}」（先 marketplace/add，再 plugin/list）"
    ))
}

fn copy_dir_recursive(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("创建 {}: {e}", to.display()))?;
    for e in std::fs::read_dir(from)
        .map_err(|e| format!("读取 {}: {e}", from.display()))?
        .flatten()
    {
        let src = e.path();
        let dst = to.join(e.file_name());
        if src.is_dir() {
            copy_dir_recursive(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst)
                .map_err(|e| format!("复制 {} → {}: {e}", src.display(), dst.display()))?;
        }
    }
    Ok(())
}

/// 安装：从市场目录复制到 `$NEO_HOME/plugins/<id>/`，并登记 installed。
pub fn plugin_install(plugin_name: &str) -> Result<serde_json::Value, String> {
    let (manifest, src) = find_in_marketplaces(plugin_name)?;
    let id = manifest
        .get("id")
        .and_then(|v| v.as_str())
        .map(sanitize_id)
        .unwrap_or_else(|| sanitize_id(plugin_name));
    let root = plugins_root().ok_or("NEO_HOME/HOME 不可用")?;
    let dest = root.join(&id);
    // 覆盖安装 = 先删再拷（幂等）
    if dest.exists() {
        std::fs::remove_dir_all(&dest).map_err(|e| format!("清理旧插件失败：{e}"))?;
    }
    copy_dir_recursive(&src, &dest)?;
    let mut installed = load_installed()?;
    installed.retain(|p| p.get("pluginId").and_then(|v| v.as_str()) != Some(id.as_str()));
    installed.push(serde_json::json!({
        "pluginId": id,
        "marketplace": manifest.get("marketplace").cloned().unwrap_or(serde_json::Value::Null),
        "path": dest.display().to_string(),
        "name": manifest.get("name").cloned().unwrap_or(serde_json::Value::Null),
        "version": manifest.get("version").cloned().unwrap_or(serde_json::Value::Null),
        "skills": manifest.get("skills").cloned().unwrap_or(serde_json::json!([])),
    }));
    save_installed(&installed)?;
    Ok(serde_json::json!({
        "pluginId": id,
        "path": dest.display().to_string(),
        "installed": true,
    }))
}

pub fn plugin_uninstall(plugin_id: &str) -> Result<bool, String> {
    let id = sanitize_id(plugin_id);
    let mut installed = load_installed()?;
    let before = installed.len();
    let path = installed
        .iter()
        .find(|p| p.get("pluginId").and_then(|v| v.as_str()) == Some(id.as_str()))
        .and_then(|p| p.get("path").and_then(|v| v.as_str()).map(|s| PathBuf::from(s)));
    installed.retain(|p| p.get("pluginId").and_then(|v| v.as_str()) != Some(id.as_str()));
    if installed.len() == before {
        return Ok(false);
    }
    save_installed(&installed)?;
    if let Some(p) = path {
        if p.exists() {
            let _ = std::fs::remove_dir_all(&p);
        }
    }
    Ok(true)
}

pub fn plugin_read(plugin_name: &str) -> Result<serde_json::Value, String> {
    // 先查已安装
    let installed = load_installed()?;
    for p in &installed {
        let id = p.get("pluginId").and_then(|v| v.as_str()).unwrap_or("");
        if id == plugin_name {
            if let Some(path) = p.get("path").and_then(|v| v.as_str()) {
                if let Some(man) = read_plugin_json(Path::new(path)) {
                    return Ok(man);
                }
            }
            return Ok(p.clone());
        }
    }
    let (manifest, _) = find_in_marketplaces(plugin_name)?;
    Ok(manifest)
}

pub fn plugin_skill_read(plugin_name: &str, skill_name: &str) -> Result<serde_json::Value, String> {
    let installed = load_installed()?;
    let path = installed
        .iter()
        .find(|p| {
            p.get("pluginId").and_then(|v| v.as_str()) == Some(sanitize_id(plugin_name).as_str())
        })
        .and_then(|p| p.get("path").and_then(|v| v.as_str()).map(PathBuf::from))
        .ok_or_else(|| format!("插件 {plugin_name} 未安装"))?;
    // 布局1: skills/SKILL.md 或 skills/<skill>/SKILL.md 或 skills/<skill>.md
    let candidates = [
        path.join("skills").join(skill_name).join("SKILL.md"),
        path.join("skills").join(format!("{skill_name}.md")),
        path.join("skills").join("SKILL.md"),
        path.join("SKILL.md"),
    ];
    for c in candidates {
        if c.is_file() {
            let body = std::fs::read_to_string(&c)
                .map_err(|e| format!("读取 {}: {e}", c.display()))?;
            return Ok(serde_json::json!({
                "pluginId": sanitize_id(plugin_name),
                "skillName": skill_name,
                "path": c.display().to_string(),
                "content": body,
            }));
        }
    }
    Err(format!(
        "插件 {plugin_name} 中找不到技能 {skill_name}（期望 skills/{skill_name}/SKILL.md）"
    ))
}

/// 校验已安装路径与 installed.json 一致；丢失目录则剔除登记。
pub fn plugin_reconcile() -> Result<serde_json::Value, String> {
    let installed = load_installed()?;
    let mut alive = Vec::new();
    let mut removed = Vec::new();
    for p in installed {
        let path = p
            .get("path")
            .and_then(|v| v.as_str())
            .map(PathBuf::from);
        let id = p.get("pluginId").and_then(|v| v.as_str()).unwrap_or("").to_string();
        match path {
            Some(dir) if dir.is_dir() => alive.push(p),
            _ => removed.push(id),
        }
    }
    save_installed(&alive)?;
    Ok(serde_json::json!({
        "alive": alive.len(),
        "removed": removed,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_home(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("neo-mkt-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn write(path: &Path, s: &str) {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(path, s).unwrap();
    }

    #[test]
    fn marketplace_add_list_install_uninstall_roundtrip() {
        // 与 skill_config 测试共享 NEO_HOME：进程内串行，避免并行 env 互踩
        let _guard = crate::test_env::neo_home_lock();
        let home = tmp_home("rt");
        // 构造本地市场
        let src = home.join("src-market");
        write(
            &src.join("plugins/demo/plugin.json"),
            r#"{"id":"demo","name":"Demo","version":"0.1.0","skills":["demo"]}"#,
        );
        write(
            &src.join("plugins/demo/skills/demo/SKILL.md"),
            "---\nname: demo\ndescription: t\n---\n# demo\n",
        );

        unsafe {
            std::env::set_var("NEO_HOME", &home);
        }

        let m = marketplace_add("local", &src.display().to_string()).unwrap();
        assert_eq!(m["name"], "local");

        let list = plugin_list().unwrap();
        let plugins = list["plugins"].as_array().unwrap();
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0]["id"], "demo");
        assert_eq!(plugins[0]["installed"], false);

        let inst = plugin_install("demo").unwrap();
        assert_eq!(inst["pluginId"], "demo");
        assert!(inst["path"].as_str().unwrap().contains("demo"));

        let installed = plugin_installed().unwrap();
        assert_eq!(installed["plugins"].as_array().unwrap().len(), 1);

        let list2 = plugin_list().unwrap();
        assert_eq!(list2["plugins"][0]["installed"], true);

        let skill = plugin_skill_read("demo", "demo").unwrap();
        assert!(skill["content"].as_str().unwrap().contains("# demo"));

        assert!(plugin_uninstall("demo").unwrap());
        assert!(!plugin_uninstall("demo").unwrap());

        assert!(marketplace_remove("local").unwrap());
        assert!(!marketplace_remove("local").unwrap());

        unsafe {
            std::env::remove_var("NEO_HOME");
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn marketplace_add_rejects_missing_source() {
        let _guard = crate::test_env::neo_home_lock();
        let home = tmp_home("badsrc");
        unsafe {
            std::env::set_var("NEO_HOME", &home);
        }
        let err = marketplace_add("x", "/definitely/not/here").unwrap_err();
        assert!(err.contains("本地目录"), "{err}");
        unsafe {
            std::env::remove_var("NEO_HOME");
        }
        let _ = std::fs::remove_dir_all(&home);
    }
}
