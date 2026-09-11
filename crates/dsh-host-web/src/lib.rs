//! L5 HOST · Web（axum + WebSocket/SSE）
//!
//! 铁律：不含业务逻辑。不依赖 Electron —— 内核是 Rust 单二进制，
//! Web 宿主只是本地服务 + 浏览器渲染。

/// 输入解析器：吸收 ZCode 的 @ / # / / / $ 上下文引用体系
pub fn parse_refs(input: &str) -> Vec<(char, String)> {
    let mut out = Vec::new();
    for token in input.split_whitespace() {
        let Some(first) = token.chars().next() else { continue };
        if matches!(first, '@' | '#' | '/' | '$') && token.len() > 1 {
            out.push((first, token[1..].to_string()));
        }
    }
    out
}

pub fn render(event_json: &str) -> String { format!("[web] {event_json}") }
