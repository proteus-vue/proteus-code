//! JSON-RPC 2.0 线格式（MCP stdio 传输：**换行分隔**的 JSON 对象，
//! 消息内不得含裸换行 —— 这是 MCP 规范的界定方式，不是 LSP 的 Content-Length）。
//!
//! 只实现 MCP 用到的子集：request / response / error / notification。
//! 不做泛化 JSON-RPC 库 —— 多余的表达力就是多余的出错面。

use serde::Serialize;
use serde_json::Value;

/// 一条发往服务器的请求（带 id，期待响应）。
#[derive(Debug, Serialize)]
pub struct Request<'a> {
    pub jsonrpc: &'static str,
    pub id: u64,
    pub method: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<&'a Value>,
}

/// 一条发往服务器的通知（无 id，不期待响应）。
#[derive(Debug, Serialize)]
pub struct Notification<'a> {
    pub jsonrpc: &'static str,
    pub method: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<&'a Value>,
}

/// 服务器的响应（成功 result 或 error，二者必居其一）。
#[derive(Debug)]
pub enum Response {
    Ok { id: u64, result: Value },
    Err { id: Option<u64>, code: i64, message: String },
}

/// 解析服务器发来的一行 JSON。
///
/// 响应之外还会有通知（如日志、进度）—— 目前一律丢弃并返回 `Ok(None)`。
/// 静默丢弃通知是有意的：它们不携带本客户端依赖的语义，接住它们
/// 只会引入需要背锅的状态。
pub fn parse_line(line: &str) -> Result<Option<Response>, String> {
    let v: Value = serde_json::from_str(line).map_err(|e| format!("不是合法 JSON：{e}"))?;
    let obj = v.as_object().ok_or("消息必须是 JSON 对象")?;
    // 有 method = 请求或通知（服务器 → 客户端方向，本客户端不处理）
    if obj.contains_key("method") {
        return Ok(None);
    }
    let id = obj.get("id").and_then(Value::as_u64);
    match obj.get("error") {
        Some(err) => {
            let code = err.get("code").and_then(Value::as_i64).unwrap_or(-32603);
            let message = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("（服务器未给错误说明）")
                .to_string();
            Ok(Some(Response::Err { id, code, message }))
        }
        None => {
            let id = id.ok_or("成功响应缺少 id")?;
            Ok(Some(Response::Ok { id, result: obj.get("result").cloned().unwrap_or(Value::Null) }))
        }
    }
}

/// MCP 初始化握手的协议版本。
/// 锁死一个版本而不是跟随服务器：版本协商失败要**显式失败**，
/// 静默接受未知版本会让两端对消息语义各有各的理解。
pub const PROTOCOL_VERSION: &str = "2024-11-05";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_success_response() {
        let r = parse_line(r#"{"jsonrpc":"2.0","id":7,"result":{"tools":[]}}"#).unwrap();
        match r {
            Some(Response::Ok { id, result }) => {
                assert_eq!(id, 7);
                assert_eq!(result["tools"], serde_json::json!([]));
            }
            other => panic!("应为成功响应：{other:?}"),
        }
    }

    #[test]
    fn parses_error_response() {
        let r = parse_line(r#"{"jsonrpc":"2.0","id":3,"error":{"code":-32601,"message":"no such method"}}"#).unwrap();
        match r {
            Some(Response::Err { id, code, message }) => {
                assert_eq!(id, Some(3));
                assert_eq!(code, -32601);
                assert_eq!(message, "no such method");
            }
            other => panic!("应为错误响应：{other:?}"),
        }
    }

    #[test]
    fn drops_notifications_instead_of_failing() {
        let r = parse_line(r#"{"jsonrpc":"2.0","method":"notifications/message","params":{"level":"info"}}"#).unwrap();
        assert!(r.is_none(), "通知应被丢弃而不是当错误");
    }

    #[test]
    fn rejects_non_object_and_bad_json() {
        assert!(parse_line("[1,2]").is_err());
        assert!(parse_line("not json").is_err());
    }

    #[test]
    fn request_serializes_without_null_params() {
        let params = serde_json::json!({"name": "x"});
        let r = Request { jsonrpc: "2.0", id: 1, method: "tools/call", params: Some(&params) };
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains(r#""params":{"name":"x"}"#), "{s}");
        let n = Notification { jsonrpc: "2.0", method: "notifications/initialized", params: None };
        let s = serde_json::to_string(&n).unwrap();
        assert!(!s.contains("params"), "无参数时不得序列化出 params：{s}");
    }
}
