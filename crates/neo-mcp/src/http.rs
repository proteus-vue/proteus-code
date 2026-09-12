//! 最小 HTTP/1.1 客户端 —— 只为 MCP 的 Streamable HTTP 传输服务。
//!
//! # 为什么不与 neo-llm-deepseek 共享 HTTP 代码
//!
//! 那边的栈是 provider 专属的（Bearer 鉴权、chat-completions 的**长流式**
//! SSE 消费、超时档位都不同）。这里只需要"POST 一段 JSON → 读回一段
//! 响应（JSON 或短 SSE）"。提取一个共享 crate 会动到真实模型路径，
//! 换来的复用不足百行 —— 两者的需求本就不同，这不是"同一语义写两遍"
//! （语义抽取必须收敛，如 facts_of / goal_awaiting_advance），而是
//! 两套帧格式的差异。若将来出现第三个使用者，再提取不迟。
//!
//! # 有界性
//!
//! 响应体按字节上限读取（超限即错，不静默截断 —— 截断的 JSON/RPC 帧
//! 无法被安全解析）；chunked 解码同样受上限约束。每个请求一条
//! `Connection: close` 连接 —— MCP 调用频率低，连接复用的收益配不上
//! 它带来的状态管理复杂度。

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// 响应体上限（1 MiB）。MCP 的 JSON-RPC 响应与短 SSE 都远小于此；
/// 超限更可能是服务器出错或被攻击，如实报错比默默吃下更对。
pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

/// 一次 HTTP 响应的解析结果。
pub struct HttpResponse {
    pub status: u16,
    /// 响应头（名字小写）。只保留需要的几个，其余丢弃。
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers.iter().find(|(k, _)| *k == name).map(|(_, v)| v.as_str())
    }
}

/// 发送一个 POST 请求（JSON 请求体）并读回完整响应。
///
/// `extra_headers` 供会话头（`Mcp-Session-Id` 等）使用。
pub fn post_json(
    url: &str,
    body: &str,
    extra_headers: &[(&str, String)],
    timeout: Duration,
) -> Result<HttpResponse, String> {
    // 只支持 http://（MCP 服务器通常在本机或内网；https 需要 TLS 栈，
    // 是一个刻意不拉入的依赖 —— 与"原型阶段不引入平台图形栈"同一取舍）
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| format!("仅支持 http:// 的 MCP 端点，收到：{url}"))?;
    let (host, path) = rest.split_once('/').map_or((rest, ""), |(h, p)| (h, p));
    let path = format!("/{path}");

    let mut stream = TcpStream::connect(host).map_err(|e| format!("连接 {host} 失败：{e}"))?;
    stream.set_read_timeout(Some(timeout)).map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(timeout)).map_err(|e| e.to_string())?;

    let mut req = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\n\
         Accept: application/json, text/event-stream\r\nAccept-Encoding: identity\r\n\
         Content-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (k, v) in extra_headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(body);
    stream
        .write_all(req.as_bytes())
        .map_err(|e| format!("发送请求失败：{e}"))?;

    // 读到 EOF（Connection: close）。按字节有界：超限即错。
    let mut raw = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = stream.read(&mut buf).map_err(|e| format!("读取响应失败：{e}"))?;
        if n == 0 {
            break;
        }
        if raw.len() + n > MAX_RESPONSE_BYTES {
            return Err(format!("响应超过 {} 字节上限", MAX_RESPONSE_BYTES));
        }
        raw.extend_from_slice(&buf[..n]);
    }
    parse_response(&String::from_utf8_lossy(&raw))
}

/// 解析 HTTP 响应：状态行 + 头 + 体（Content-Length / chunked）。
pub fn parse_response(raw: &str) -> Result<HttpResponse, String> {
    let (head, body) = raw
        .split_once("\r\n\r\n")
        .ok_or_else(|| "响应缺少头/体分隔".to_string())?;
    let mut lines = head.lines();
    let status_line = lines.next().ok_or("响应为空")?;
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("状态行无法解析：{status_line}"))?;
    let mut headers = Vec::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    let chunked = headers
        .iter()
        .any(|(k, v)| k == "transfer-encoding" && v.to_ascii_lowercase().contains("chunked"));
    let body = if chunked { decode_chunked(body)? } else { body.to_string() };
    if body.len() > MAX_RESPONSE_BYTES {
        // 读取循环已经挡过一次（防内存膨胀）；这里再兜一次，保证
        // 无论响应怎么到达，超限都如实报错而不是被默默解析
        return Err(format!("响应超过 {} 字节上限", MAX_RESPONSE_BYTES));
    }
    Ok(HttpResponse { status, headers, body })
}

/// chunked 解码（与 neo-llm-deepseek 同一套规则：块长十六进制、
/// trailer、上限保护）。独立实现：跨 crate 共享会引入依赖方向的难题，
/// 而规则本身有测试钉住。
pub fn decode_chunked(body: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = body;
    loop {
        let Some((size_line, after)) = rest.split_once("\r\n") else {
            return Err("chunked 格式错误：缺少块长行".into());
        };
        let size_str = size_line.split(';').next().unwrap_or(size_line).trim();
        let size = usize::from_str_radix(size_str, 16)
            .map_err(|_| format!("chunked 块长无法解析：{size_str:?}"))?;
        if size == 0 {
            return Ok(out); // 终止块（trailer 连同后续内容忽略）
        }
        if out.len() + size > MAX_RESPONSE_BYTES {
            return Err("chunked 内容超过响应上限".into());
        }
        let bytes = after.as_bytes();
        if bytes.len() < size {
            return Err("chunked 格式错误：块不完整".into());
        }
        // 块数据可能切在 UTF-8 边界内 —— 按字节取再整体无损转字符串
        let chunk = &after[..size];
        out.push_str(chunk);
        rest = &after[size..];
        rest = rest.strip_prefix("\r\n").unwrap_or(rest);
    }
}

/// 从 SSE 响应体里抽出 `data:` 行（MCP Streamable HTTP 用它承载
/// JSON-RPC 消息）。每条 `data:` 一条消息；`event:`/注释行忽略。
pub fn sse_data_lines(body: &str) -> Vec<String> {
    body.lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .map(|d| d.strip_prefix(' ').unwrap_or(d).trim_end().to_string())
        .filter(|d| !d.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_json_response() {
        let raw = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}";
        let r = parse_response(raw).unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(r.header("content-type"), Some("application/json"));
        assert_eq!(r.body, "{}");
    }

    #[test]
    fn decodes_chunked_bodies() {
        // 5 字节 "hello" + 2 字节 "hi" + 终止块
        let raw = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
                   5\r\nhello\r\n2\r\nhi\r\n0\r\n\r\n";
        let r = parse_response(raw).unwrap();
        assert_eq!(r.body, "hellohi");
    }

    #[test]
    fn chunked_with_extensions_and_trailer() {
        let raw = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
                   3;ext=1\r\nabc\r\n0\r\nX-Trailer: y\r\n\r\n";
        let r = parse_response(raw).unwrap();
        assert_eq!(r.body, "abc");
    }

    #[test]
    fn response_size_is_bounded() {
        let big = "x".repeat(MAX_RESPONSE_BYTES + 1);
        let raw = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{big}", big.len());
        assert!(parse_response(&raw).is_err(), "超限响应必须报错");
    }

    #[test]
    fn extracts_data_lines_from_sse() {
        let body = ": keepalive\n\
                    event: message\n\
                    data: {\"jsonrpc\":\"2.0\",\"id\":1}\n\
                    \n\
                    data: {\"jsonrpc\":\"2.0\",\"id\":2}\n\n";
        let lines = sse_data_lines(body);
        assert_eq!(lines, vec![
            r#"{"jsonrpc":"2.0","id":1}"#,
            r#"{"jsonrpc":"2.0","id":2}"#,
        ]);
    }
}
