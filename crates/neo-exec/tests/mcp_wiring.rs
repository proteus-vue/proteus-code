//! MCP 装配接线的测试：用户级配置生效、项目级拒绝、坏服务器不拖垮装配。
//!
//! 用真 `/bin/sh` 进程当 MCP 服务器（桩测不出"起进程"这条路径）；
//! 配置文件放在临时目录、以**参数**传入 —— 不碰进程级 `NEO_HOME`，
//! 并行测试不会串环境（PROJECT_MEMORY 4.13d 的教训）。

#![cfg(unix)]

use neo_exec::register_mcp_tools;
use neo_core::ToolRegistry;
use serde_json::json;

/// 最小 MCP 服务器（握手 → 一页工具 → 一次调用）。
const SERVER: &str = r#"
read line
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"t","version":"0"}}}'
read line
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"echo","description":"原样返回","inputSchema":{"type":"object"},"annotations":{"readOnlyHint":true}}]}}'
read line
printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"ok"}]}}'
read line
"#;

fn tmp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("neo-exec-mcp-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn user_level_config_registers_external_tools() {
    let dir = tmp_dir("ok");
    let ws = dir.join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    let cfg = dir.join("config-mcp.json");
    std::fs::write(
        &cfg,
        json!({"servers":[{"name":"test","command":"/bin/sh","args":["-c", SERVER]}]}).to_string(),
    )
    .unwrap();
    let mut tools = ToolRegistry::new();
    let warnings = register_mcp_tools(&mut tools, Some(&cfg), &ws);
    assert!(warnings.is_empty(), "不应有警告：{warnings:?}");
    let registered = tools.get("mcp__test__echo").expect("外部工具应入表");

    // 外部工具对内核就是普通工具：描述进提示词、语义按声明分类
    let prompt = tools.render_prompt();
    assert!(prompt.contains("mcp__test__echo"), "提示词必须包含外部工具：{prompt}");
    assert!(prompt.contains("原样返回"), "服务器的描述要进提示词");
    assert!(prompt.contains(r#""type":"object""#), "参数 schema 要给模型看");
    let _ = registered;
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn broken_server_warns_but_does_not_block_assembly() {
    let dir = tmp_dir("broken");
    let ws = dir.join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    let cfg = dir.join("config-mcp.json");
    std::fs::write(
        &cfg,
        json!({"servers":[
            {"name":"dead","command":"/nonexistent/neo-mcp-missing","args":[]},
            {"name":"garbage","command":"/bin/sh","args":["-c","read line; printf 'not json\\n'"]}
        ]}).to_string(),
    )
    .unwrap();
    let mut tools = ToolRegistry::new();
    let warnings = register_mcp_tools(&mut tools, Some(&cfg), &ws);
    assert_eq!(warnings.len(), 2, "两台坏服务器各一条告警：{warnings:?}");
    assert!(warnings[0].contains("dead"), "{warnings:?}");
    assert!(warnings[1].contains("garbage"), "{warnings:?}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn project_level_config_is_rejected_not_silently_ignored() {
    let dir = tmp_dir("proj");
    // 工作区里放项目级 mcp.json（仓库作者指定的程序 = 供应链注入）
    std::fs::write(dir.join("mcp.json"), r#"{"servers":[]}"#).unwrap();
    let mut tools = ToolRegistry::new();
    let warnings = register_mcp_tools(&mut tools, None, &dir);
    assert_eq!(warnings.len(), 1, "项目级配置必须产生告警：{warnings:?}");
    assert!(warnings[0].contains("供应链"), "告警要说清威胁：{warnings:?}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn missing_config_is_not_a_warning() {
    let dir = tmp_dir("none");
    let mut tools = ToolRegistry::new();
    let warnings = register_mcp_tools(&mut tools, Some(&dir.join("nope.json")), &dir);
    assert!(warnings.is_empty(), "没配 MCP 是常态不是故障：{warnings:?}");
    std::fs::remove_dir_all(&dir).unwrap();
}
