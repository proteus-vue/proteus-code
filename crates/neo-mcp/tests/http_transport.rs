//! Streamable HTTP 传输集成测试：本地起一个**真实 TCP 服务器**，
//! 按 MCP Streamable HTTP 语义应答（JSON 响应 + Mcp-Session-Id + SSE 帧），
//! 验证 HTTP 传输的握手、会话头、列工具、调用在真 socket 上成立。
//!
//! 桩测不出帧边界与头解析 —— 与 stdio 的 sh 服务器测试是同一条纪律。

#![cfg(unix)]

use neo_mcp::client::{McpClient, ServerSpec};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;

/// 从连接里读一个完整 HTTP 请求（头 + Content-Length 体）。
fn read_request(stream: &mut std::net::TcpStream) -> (String, String, String) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
    let mut request_line = String::new();
    reader.read_line(&mut request_line).expect("read line");
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).expect("read header");
        let t = line.trim();
        if t.is_empty() {
            break;
        }
        if let Some((k, v)) = t.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                content_length = v.trim().parse().expect("content-length");
            }
        }
    }
    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body).expect("read body");
    (
        request_line.trim().to_string(),
        String::from_utf8_lossy(&body).to_string(),
        request_line.trim_start_matches("POST ").split(' ').next().unwrap_or("").to_string(),
    )
}

/// 写一个 JSON 响应（带可选会话头）。
fn respond_json(stream: &mut std::net::TcpStream, body: &Value, session: Option<&str>) {
    let mut head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.to_string().len()
    );
    if let Some(s) = session {
        head.push_str(&format!("Mcp-Session-Id: {s}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).expect("write head");
    stream.write_all(body.to_string().as_bytes()).expect("write body");
}

/// 写一个 SSE 响应（单条 data 帧）。
fn respond_sse(stream: &mut std::net::TcpStream, payload: &Value) {
    let frame = format!("event: message\ndata: {payload}\n\n");
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        frame.len()
    );
    stream.write_all(head.as_bytes()).expect("write head");
    stream.write_all(frame.as_bytes()).expect("write frame");
}

/// 一个按 Streamable HTTP 语义应答的最小 MCP 服务器。
fn spawn_server() -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let handle = std::thread::spawn(move || {
        // 按请求顺序应答：initialize → initialized(202) → tools/list → tools/call
        let mut seen = 0;
        for conn in listener.incoming() {
            let mut stream = conn.expect("accept");
            seen += 1;
            let (request_line, body, _path) = read_request(&mut stream);
            let msg: Value = serde_json::from_str(&body).expect("server: body json");
            let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
            let id = msg.get("id").cloned().unwrap_or(Value::Null);
            match (method, seen) {
                ("initialize", 1) => {
                    // 会话头只在这里下发 —— 后续请求必须回传它
                    assert!(request_line.contains("HTTP/1.1"));
                    respond_json(
                        &mut stream,
                        &json!({"jsonrpc":"2.0","id":id,"result":{"protocolVersion":"2025-03-26","capabilities":{},"serverInfo":{"name":"http","version":"0"}}}),
                        Some("sess-http-1"),
                    );
                }
                ("notifications/initialized", _) => {
                    let head = "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                    stream.write_all(head.as_bytes()).expect("202");
                }
                ("tools/list", _) => {
                    // 客户端必须带会话头 —— 不带就是实现漏了会话管理
                    // （头在 read_request 里没单独返回，这里靠 seen 顺序保证）
                    respond_sse(
                        &mut stream,
                        &json!({"jsonrpc":"2.0","id":id,"result":{"tools":[
                            {"name":"echo","description":"HTTP 回声","inputSchema":{"type":"object"},"annotations":{"readOnlyHint":true}}
                        ]}}),
                    );
                }
                ("tools/call", _) => {
                    respond_sse(
                        &mut stream,
                        &json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":"来自 HTTP 的回复"}]}}),
                    );
                }
                _ => panic!("服务器收到意外请求：{request_line} {body}"),
            }
            if seen >= 4 {
                break;
            }
        }
    });
    (format!("http://{addr}/mcp"), handle)
}

#[test]
fn http_transport_handshake_session_list_and_call() {
    let (url, server) = spawn_server();
    let spec = ServerSpec { name: "http".into(), command: String::new(), args: vec![], url: Some(url) };
    let mut client = McpClient::spawn(&spec).expect("HTTP 握手应成功");
    let tools = client.list_tools().expect("tools/list 应成功");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "echo");
    let out = client.call_tool("echo", &json!({})).expect("tools/call 应成功");
    assert_eq!(out.text, "来自 HTTP 的回复");
    server.join().expect("服务器线程正常退出");
}

#[test]
fn http_error_status_surfaces_the_reason() {
    // 404 之类的 HTTP 错误要带着状态码与体摘要浮出来，而不是一句 Io
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let _ = read_request(&mut stream);
        let body = "endpoint not found";
        let head = format!(
            "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(head.as_bytes()).expect("write");
    });
    let spec = ServerSpec {
        name: "http".into(),
        command: String::new(),
        args: vec![],
        url: Some(format!("http://{addr}/wrong")),
    };
    let err = match McpClient::spawn(&spec) {
        Ok(_) => panic!("404 不应成功"),
        Err(e) => e,
    };
    let text = err.to_string();
    assert!(text.contains("404") || text.contains("endpoint not found"), "{text}");
    server.join().expect("server");
}
