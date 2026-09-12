//! 真进程集成测试（cfg(unix)）：用 `/bin/sh` 起一个真实 MCP 服务器，
//! 验证换行分帧、握手、列工具、调用在**真管道**上成立。
//!
//! 桩测不出缓冲与字节边界 —— 管道是行缓冲还是块缓冲、写后是否 flush、
//! EOF 的时机，这些只有真进程能证明（与沙箱必须真机验证同一条纪律）。

#![cfg(unix)]

use neo_mcp::client::{McpClient, ServerSpec};
use serde_json::json;

/// 一个最小 MCP 服务器：读行、回握手、回一页工具、回应一次调用。
/// printf 后必须 flush —— `sh` 的 printf 直接写 fd 1（无用户态缓冲），
/// 每条以 `\n` 结尾，符合换行分帧。
const SERVER: &str = r#"
read line
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"t","version":"0"}}}'
read line
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"echo","description":"原样返回","inputSchema":{"type":"object"},"annotations":{"readOnlyHint":true}}]}}'
read line
printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"来自真进程的回复"}]}}'
read line
"#;

#[test]
fn real_process_handshake_list_and_call() {
    let spec = ServerSpec {
        name: "test".into(),
        command: "/bin/sh".into(),
        args: vec!["-c".into(), SERVER.into()],
        url: None,
    };
    let mut client = McpClient::spawn(&spec).expect("真进程握手应成功");
    let tools = client.list_tools().expect("tools/list 应成功");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "echo");
    assert_eq!(tools[0].read_only_hint, Some(true));

    let out = client.call_tool("echo", &json!({"x": 1})).expect("tools/call 应成功");
    assert!(!out.is_error);
    assert_eq!(out.text, "来自真进程的回复");
}

#[test]
fn nonexistent_command_reports_spawn_failure() {
    let spec = ServerSpec {
        name: "nope".into(),
        command: "/nonexistent/neo-mcp-missing-cmd".into(),
        args: vec![],
        url: None,
    };
    let err = match McpClient::spawn(&spec) {
        Ok(_) => panic!("不存在的命令不应成功"),
        Err(e) => e,
    };
    assert!(matches!(err, neo_mcp::McpError::Spawn(_)), "{err:?}");
    assert!(err.to_string().contains("无法启动"), "错误要说清是启动失败：{err}");
}
