//! MCP 服务器的配置来源：**只允许用户级**，项目级显式拒绝。
//!
//! # 为什么项目级必须显式拒绝（而不是静默忽略）
//!
//! 一个仓库可以指定的可执行文件就是**供应链注入**：恶意仓库带一份
//! `mcp.json`，`git clone` 后跑一次 Neo 就执行了仓库作者选定的程序。
//! 这与 4.13 信任门挡住的是同一类威胁 —— 信任门回答"这个目录能不能动"，
//! 这里回答"要不要替这个目录启动一个常驻进程"。静默忽略在这里是错的
//! 体验（用户配了却不生效，对着现象想不通），所以像项目级
//! `providers.json` 一样：**报错，说清原因与正确位置**。

use std::path::{Path, PathBuf};

use crate::client::ServerSpec;

/// `NEO_HOME`（默认 `$HOME/.neo`）。与 neo-instructions 同一解析规则。
fn neo_home() -> Option<PathBuf> {
    std::env::var_os("NEO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .map(|h| if h.ends_with(".neo") { h } else { h.join(".neo") })
}

/// 用户级配置路径：`$NEO_HOME/mcp.json`。
pub fn user_config_path() -> Option<PathBuf> {
    neo_home().map(|h| h.join("mcp.json"))
}

/// 读用户级配置。文件不存在 = 没配 MCP 服务器（`Ok(vec![])`，不是错误 ——
/// MCP 是可选能力）。JSON 坏了才是错误（静默吞掉配置错误 = 用户以为
/// 配好了、实际一个服务器都没起）。
pub fn load_user_config() -> Result<Vec<ServerSpec>, String> {
    let Some(path) = user_config_path() else {
        return Ok(Vec::new());
    };
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("读取 {}: {e}", path.display())),
    };
    parse_user_config(&raw)
        .map_err(|e| format!("配置 {} 无法解析：{e}", path.display()))
}

/// 解析配置内容（独立成函数：装配点直接拿路径走这里，测试不必碰
/// 进程级 `NEO_HOME`）。
pub fn parse_user_config(raw: &str) -> Result<Vec<ServerSpec>, String> {
    let v: serde_json::Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    let servers = v.get("servers").and_then(|s| s.as_array()).ok_or("缺少 servers 数组")?;
    let mut out = Vec::new();
    for s in servers {
        let name = s.get("name").and_then(|x| x.as_str()).unwrap_or_default().to_string();
        let has_command = s
            .get("command")
            .and_then(|x| x.as_str())
            .map(|c| !c.is_empty())
            .unwrap_or(false);
        let has_url = s.get("url").and_then(|x| x.as_str()).is_some();
        if name.is_empty() || !(has_command || has_url) {
            // 半条配置直接拒：带病配置最危险的表现是"起了一半还看起来正常"
            return Err(format!(
                "servers 里的条目缺少 name 或 transport（command / url）：{s}"
            ));
        }
        let spec: ServerSpec = serde_json::from_value(s.clone()).map_err(|e| e.to_string())?;
        // command（stdio）与 url（Streamable HTTP）恰填其一：
        // 都填 = 到底连哪个说不清；都不填 = 一个起不来的服务器
        if spec.command.is_empty() == spec.url.is_none() {
            return Err(format!(
                "服务器 {} 必须且只能填 command（stdio）或 url（HTTP）之一",
                spec.name
            ));
        }
        out.push(spec);
    }
    Ok(out)
}

/// 检查工作区里是否有项目级 MCP 配置；有则拒绝并说明原因。
/// 由装配点调用（与"项目级 providers.json 拒读"同一模式）。
pub fn reject_project_level(workspace: &Path) -> Result<(), String> {
    for candidate in ["mcp.json", ".neo/mcp.json"] {
        let p = workspace.join(candidate);
        if p.is_file() {
            return Err(format!(
                "拒绝读取项目级 {}：MCP 服务器是可执行程序，属于供应链敏感配置，\
                 只允许放在用户级 ~/.neo/mcp.json（仓库指定的程序会让 git clone 变成任意代码执行）",
                p.display()
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_config() {
        let raw = r#"{"servers":[
            {"name":"fs","command":"npx","args":["-y","mcp-fs"]},
            {"name":"git","command":"/usr/local/bin/mcp-git"}
        ]}"#;
        let servers = parse_user_config(raw).unwrap();
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0].name, "fs");
        assert_eq!(servers[0].args, vec!["-y", "mcp-fs"]);
        assert!(servers[1].args.is_empty(), "args 缺省为空");
    }

    #[test]
    fn rejects_incomplete_entries() {
        // 缺 command：起一半的配置比没配置更危险
        assert!(parse_user_config(r#"{"servers":[{"name":"x"}]}"#).is_err());
        assert!(parse_user_config(r#"{"servers":[{"command":"x"}]}"#).is_err());
        assert!(parse_user_config(r#"{}"#).is_err());
        assert!(parse_user_config("not json").is_err());
    }

    #[test]
    fn command_and_url_are_mutually_exclusive_but_one_is_required() {
        // url = Streamable HTTP 传输；与 command（stdio）恰填其一
        let ok_http = parse_user_config(r#"{"servers":[{"name":"remote","url":"http://127.0.0.1:9999/mcp"}]}"#).unwrap();
        assert_eq!(ok_http[0].url.as_deref(), Some("http://127.0.0.1:9999/mcp"));
        // 都填 = 说不清连哪个；都不填 = 起不来的服务器
        assert!(parse_user_config(
            r#"{"servers":[{"name":"x","command":"a","url":"http://b"}]}"#
        )
        .is_err());
        assert!(parse_user_config(r#"{"servers":[{"name":"x"}]}"#).is_err());
    }

    #[test]
    fn project_level_files_are_rejected_with_reason() {
        let dir = std::env::temp_dir().join(format!("neo-mcp-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("mcp.json"), r#"{"servers":[]}"#).unwrap();
        let err = reject_project_level(&dir).unwrap_err();
        assert!(err.contains("供应链"), "错误要说清为什么拒绝：{err}");
        assert!(err.contains("用户级"), "错误要给出正确位置：{err}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn workspace_without_project_config_is_accepted() {
        let dir = std::env::temp_dir().join(format!("neo-mcp-clean-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(reject_project_level(&dir).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
