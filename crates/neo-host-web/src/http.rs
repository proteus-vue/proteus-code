//! 最简 HTTP/1.1 服务端（零依赖）
//!
//! # 为什么手写而不引 axum/hyper
//!
//! 与本项目一贯立场一致：**调试链要浅**。这里需要的 HTTP 能力只有三样：
//! 解析请求行与头、按 Content-Length 读体、写响应。引入生产级 HTTP 栈
//! 会带来十几层中间件抽象，一个"请求没到"的问题要翻很久。
//!
//! # 有界性（Rust 不保证的部分）
//!
//! 三处必须有界，否则一个恶意或异常的请求就能吃光内存：
//! - 请求头总字节上限
//! - 请求体上限（按 Content-Length 预检**并**在读时累计校验）
//! - 并发连接上限
//!
//! # 诚实边界
//!
//! - 只支持 HTTP/1.1 的最小子集：不处理 chunked 编码、不支持长连接复用、
//!   不支持 TLS（放在本机回环 + 反向代理之后使用）。
//! - 不做路由匹配（只有几个固定路径的精确匹配）。

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;

/// 请求头总字节上限（防超大头撑爆内存）。
pub const MAX_HEADER_BYTES: usize = 16 * 1024;
/// 请求体上限。
pub const MAX_BODY_BYTES: usize = 1024 * 1024;
/// 并发连接上限。
pub const MAX_CONNECTIONS: usize = 64;
/// 单个请求的读超时（避免慢速攻击占住线程）。
pub const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// 一个已解析的请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub body: String,
}

/// 解析出错的种类（用于给出合适的 HTTP 状态码）。
#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    /// 请求头过大
    HeaderTooLarge,
    /// 请求体过大
    BodyTooLarge,
    /// 格式非法
    Malformed,
}

impl ParseError {
    pub fn status(&self) -> u16 {
        match self {
            Self::HeaderTooLarge | Self::BodyTooLarge => 413,
            Self::Malformed => 400,
        }
    }
    pub fn message(&self) -> &'static str {
        match self {
            Self::HeaderTooLarge => "请求头过大",
            Self::BodyTooLarge => "请求体过大",
            Self::Malformed => "请求格式非法",
        }
    }
}

/// 从流中解析一个请求。
///
/// 所有读取都受上限约束：**先按上限读，再判断是否超限**。
/// 若先无条件读到 EOF 再检查，上限就形同虚设。
pub fn parse_request<R: Read>(reader: &mut BufReader<R>) -> Result<Request, ParseError> {
    // ── 请求行 ──
    let mut line = String::new();
    let n = reader.read_line(&mut line).map_err(|_| ParseError::Malformed)?;
    if n == 0 {
        return Err(ParseError::Malformed); // 连接空转
    }
    let mut parts = line.trim_end().split_whitespace();
    let method = parts.next().ok_or(ParseError::Malformed)?.to_string();
    let path = parts.next().ok_or(ParseError::Malformed)?.to_string();

    // ── 头（带上限累计）──
    let mut header_bytes = line.len();
    let mut content_length: Option<usize> = None;
    loop {
        let mut h = String::new();
        let n = reader.read_line(&mut h).map_err(|_| ParseError::Malformed)?;
        if n == 0 {
            break;
        }
        header_bytes += n;
        if header_bytes > MAX_HEADER_BYTES {
            return Err(ParseError::HeaderTooLarge);
        }
        let t = h.trim_end();
        if t.is_empty() {
            break; // 头结束
        }
        if let Some((k, v)) = t.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                content_length = v.trim().parse::<usize>().ok();
            }
        }
    }

    // ── 体 ──
    let body = match content_length {
        None | Some(0) => String::new(),
        Some(len) => {
            if len > MAX_BODY_BYTES {
                return Err(ParseError::BodyTooLarge);
            }
            // take(len) 保证不会读到超过声明长度的字节
            let mut buf = vec![0u8; len];
            reader.read_exact(&mut buf).map_err(|_| ParseError::Malformed)?;
            String::from_utf8_lossy(&buf).into_owned()
        }
    };

    Ok(Request { method, path, body })
}

/// 写一个普通响应。
pub fn write_response(out: &mut impl Write, status: u16, content_type: &str, body: &str) -> std::io::Result<()> {
    let status_text = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        503 => "Service Unavailable",
        _ => "OK",
    };
    write!(
        out,
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{body}",
        body.as_bytes().len()
    )?;
    out.flush()
}

/// 写 SSE 流的响应头。之后由调用方逐事件写 `data:` 行。
pub fn write_sse_headers(out: &mut impl Write) -> std::io::Result<()> {
    write!(
        out,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nConnection: keep-alive\r\nX-Accel-Buffering: no\r\n\r\n"
    )?;
    out.flush()
}

/// 写一条 SSE 事件并立即 flush（不 flush 浏览器收不到）。
pub fn write_sse_event(out: &mut impl Write, data: &str) -> std::io::Result<()> {
    // SSE 规定：data 中出现换行要拆成多个 data: 行
    for line in data.split('\n') {
        write!(out, "data: {line}\r\n")?;
    }
    write!(out, "\r\n")?;
    out.flush()
}

/// 一个极简的单线程连接处理器：每连接一线程，带并发上限。
///
/// 返回实际监听的地址（端口传 0 时由 OS 分配，便于测试）。
pub fn serve<F>(listener: TcpListener, handler: F) -> std::io::Result<()>
where
    F: Fn(&mut TcpStream, Request) + Send + Sync + 'static,
{
    let handler = Arc::new(handler);
    let active = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };

        // 并发上限：超出即拒（503），不排队 —— 排队会把内存与延迟都变成攻击面
        let current = active.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if current >= MAX_CONNECTIONS {
            active.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            let _ = write_response(&mut stream, 503, "text/plain; charset=utf-8", "连接数已达上限");
            continue;
        }

        let handler = Arc::clone(&handler);
        let active = Arc::clone(&active);
        std::thread::spawn(move || {
            let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
            let _ = stream.set_write_timeout(Some(READ_TIMEOUT));
            let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
            match parse_request(&mut reader) {
                Ok(req) => handler(&mut stream, req),
                Err(e) => {
                    let _ = write_response(
                        &mut stream,
                        e.status(),
                        "text/plain; charset=utf-8",
                        e.message(),
                    );
                }
            }
            active.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn parse(raw: &str) -> Result<Request, ParseError> {
        let mut r = BufReader::new(Cursor::new(raw.as_bytes().to_vec()));
        parse_request(&mut r)
    }

    #[test]
    fn parses_a_simple_get() {
        let req = parse("GET / HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
        assert_eq!(req.method, "GET");
        assert_eq!(req.path, "/");
        assert!(req.body.is_empty());
    }

    #[test]
    fn parses_a_post_with_body() {
        let req = parse("POST /api/turn HTTP/1.1\r\nContent-Length: 5\r\n\r\nhello").unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.path, "/api/turn");
        assert_eq!(req.body, "hello");
    }

    #[test]
    fn content_length_is_case_insensitive() {
        let req = parse("POST / HTTP/1.1\r\ncontent-LENGTH: 2\r\n\r\nok").unwrap();
        assert_eq!(req.body, "ok");
    }

    #[test]
    fn rejects_an_oversized_body_by_declared_length() {
        // 关键：必须先看 Content-Length 就拒，而不是真去读 2 GB
        let raw = format!(
            "POST / HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            MAX_BODY_BYTES + 1
        );
        assert_eq!(parse(&raw), Err(ParseError::BodyTooLarge));
        assert_eq!(ParseError::BodyTooLarge.status(), 413);
    }

    #[test]
    fn rejects_oversized_headers() {
        let big = "X-Pad: ".to_string() + &"a".repeat(MAX_HEADER_BYTES + 10);
        let raw = format!("GET / HTTP/1.1\r\n{big}\r\n\r\n");
        assert_eq!(parse(&raw), Err(ParseError::HeaderTooLarge));
    }

    #[test]
    fn rejects_a_truncated_request_line() {
        assert_eq!(parse("GET\r\n\r\n"), Err(ParseError::Malformed));
        assert_eq!(parse(""), Err(ParseError::Malformed));
    }

    #[test]
    fn only_reads_the_declared_body_length() {
        // 声明 3 字节却给了更多 —— 只应读到 3 字节，多余的留给下一个请求
        let req = parse("POST / HTTP/1.1\r\nContent-Length: 3\r\n\r\nabcdef").unwrap();
        assert_eq!(req.body, "abc", "必须严格按 Content-Length 读");
    }

    #[test]
    fn sse_event_wraps_multiline_data() {
        let mut out = Vec::new();
        write_sse_event(&mut out, "line1\nline2").unwrap();
        let s = String::from_utf8(out).unwrap();
        assert_eq!(s, "data: line1\r\ndata: line2\r\n\r\n", "换行必须拆成多条 data:");
    }

    #[test]
    fn error_status_codes_are_sane() {
        assert_eq!(ParseError::HeaderTooLarge.status(), 413);
        assert_eq!(ParseError::BodyTooLarge.status(), 413);
        assert_eq!(ParseError::Malformed.status(), 400);
    }
}
