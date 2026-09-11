//! L3 PROVIDER —— `ModelProvider` 的真实实现（DeepSeek chat-completions）
//!
//! # 为什么手写 HTTP 而不引 reqwest
//!
//! 1. **调试链浅**：依赖越少，出错时能查的地方越少。生产级 HTTP 栈（tower /
//!    hyper / rustls）会带来十几层间接，一个连接失败要翻很久。
//! 2. **零 unsafe**：标准库 `TcpStream` 全程 safe。
//! 3. 本场景只需 POST 一个 JSON、读回一个响应 —— 不需要连接池/重定向/HTTP2。
//!
//! **仅支持 HTTPS**：明文 HTTP 会让 API key 在链路上裸奔，直接拒绝。
//! TLS 由系统 `openssl` 命令承担（进程级，不改链接），这样既不引 crate
//! 也不自己做密码学 —— 自己实现 TLS 是明确的反模式。
//!
//! # 诚实边界
//!
//! - **非流式**：等待完整响应后一次性产出增量。真实流式（SSE 增量解析）**未实现**。
//!   内核的 `ModelStream` 契约本就是流式形状，所以接入真流式时内核无需改动。
//! - **每次请求一个进程**：`openssl s_client` 走进程，有进程启动开销。
//!   生产实现应改用常驻连接 —— 当前优先"能跑通且可调试"。
//! - **未处理**：重试、超时细分、代理、SSE 多事件、tool_choice 强制。

use neo_core::{Message, ModelDelta, ModelProvider, ModelRequest, ModelStream, ToolInvocation};
use std::io::{Read, Write};
use std::process::{Command, Stdio};

/// DeepSeek 官方 endpoint（OpenAI 兼容形态）。
pub const DEFAULT_ENDPOINT: &str = "api.deepseek.com";
pub const DEFAULT_PATH: &str = "/chat/completions";
pub const DEFAULT_MODEL: &str = "deepseek-chat";

pub struct DeepSeekProvider {
    pub api_key: String,
    pub endpoint: String,
    pub path: String,
    pub model: String,
    /// 传给模型的温度。默认 0（确定性优先，便于回放与测试）。
    pub temperature: f32,
}

impl DeepSeekProvider {
    /// 从环境变量 `DEEPSEEK_API_KEY` 构造。
    pub fn from_env() -> Result<Self, String> {
        let api_key = std::env::var("DEEPSEEK_API_KEY")
            .map_err(|_| "未设置环境变量 DEEPSEEK_API_KEY".to_string())?;
        if api_key.trim().is_empty() {
            return Err("DEEPSEEK_API_KEY 为空".into());
        }
        Ok(Self {
            api_key,
            endpoint: std::env::var("DEEPSEEK_BASE_URL")
                .unwrap_or_else(|_| DEFAULT_ENDPOINT.to_string()),
            path: DEFAULT_PATH.to_string(),
            model: std::env::var("DEEPSEEK_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string()),
            temperature: 0.0,
        })
    }

    pub fn with_model(mut self, m: impl Into<String>) -> Self { self.model = m.into(); self }

    /// 把内核的 Message 序列表成 OpenAI 兼容的 messages 数组。
    fn encode_messages(&self, req: &ModelRequest<'_>) -> serde_json::Value {
        let mut out = Vec::new();
        out.push(serde_json::json!({ "role": "system", "content": req.system }));
        for m in req.messages {
            match m {
                Message::System(s) => out.push(serde_json::json!({ "role": "system", "content": s })),
                Message::User(s) => out.push(serde_json::json!({ "role": "user", "content": s })),
                Message::Assistant { text, tool_calls } => {
                    let calls: Vec<_> = tool_calls
                        .iter()
                        .map(|c| {
                            serde_json::json!({
                                "id": c.id,
                                "type": "function",
                                "function": {
                                    "name": c.name,
                                    "arguments": c.arguments.to_string(),
                                }
                            })
                        })
                        .collect();
                    let mut obj = serde_json::json!({
                        "role": "assistant",
                        "content": if text.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(text.clone()) },
                    });
                    if !calls.is_empty() {
                        obj["tool_calls"] = serde_json::Value::Array(calls);
                    }
                    out.push(obj);
                }
                Message::ToolResult { id, name, output } => {
                    out.push(serde_json::json!({
                        "role": "tool",
                        "tool_call_id": id,
                        "name": name,
                        "content": output.stdout,
                    }));
                }
            }
        }
        serde_json::Value::Array(out)
    }

    /// 工具 schema → OpenAI function 形态。
    fn encode_tools(&self, req: &ModelRequest<'_>) -> Option<serde_json::Value> {
        if req.tools.is_empty() {
            return None;
        }
        Some(serde_json::Value::Array(
            req.tools
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            // 本原型的工具不声明细粒度参数 schema（内核的 ToolSchema
                            // 只有 name/description）。补参数 schema 属后续工作。
                            "parameters": { "type": "object", "properties": {}, "additionalProperties": true }
                        }
                    })
                })
                .collect(),
        ))
    }

    fn build_body(&self, req: &ModelRequest<'_>) -> serde_json::Value {
        let mut body = serde_json::json!({
            "model": self.model,
            "messages": self.encode_messages(req),
            "temperature": self.temperature,
        });
        if let Some(tools) = self.encode_tools(req) {
            body["tools"] = tools;
        }
        body
    }

    /// 发一次请求，返回响应体字符串。
    ///
    /// TLS 走系统 `openssl s_client` 进程：不引 crate、不自写密码学。
    fn post(&self, body: &str) -> Result<String, String> {
        let (status, body) = self.post_raw(body)?;
        if !(200..300).contains(&status) {
            return Err(format!("HTTP {status}：{}", body.trim()));
        }
        Ok(body)
    }

    /// 发一次请求，返回（状态码, 响应体）。**不做状态码判断** ——
    /// 让调用方（含集成测试）能看到真实状态。
    pub fn post_raw(&self, body: &str) -> Result<(u16, String), String> {
        let request = format!(
            "POST {path} HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {key}\r\nContent-Type: application/json\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{body}",
            path = self.path,
            host = self.endpoint,
            key = self.api_key,
            len = body.as_bytes().len(),
            body = body,
        );

        // HTTPS：交给 openssl s_client（-quiet 抑制握手噪声）
        let mut child = Command::new("openssl")
            .args(["s_client", "-quiet", "-connect", &format!("{}:443", self.endpoint), "-servername", &self.endpoint])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("无法启动 openssl（HTTPS 必需）：{e}"))?;

        child
            .stdin
            .as_mut()
            .ok_or("openssl stdin 不可用")?
            .write_all(request.as_bytes())
            .map_err(|e| format!("写入请求失败：{e}"))?;
        // 关闭 stdin 让 s_client 发完即读
        drop(child.stdin.take());

        let mut raw = String::new();
        {
            let mut out = child.stdout.take().ok_or("openssl stdout 不可用")?;
            out.read_to_string(&mut raw).map_err(|e| format!("读取响应失败：{e}"))?;
        }
        let _ = child.wait();

        // 拆 HTTP 头 / 体
        let body_start = raw.find("\r\n\r\n").map(|i| i + 4).unwrap_or(0);
        let head = &raw[..body_start.min(raw.len())];
        let resp_body = &raw[body_start.min(raw.len())..];

        // 状态码
        let status = head
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(0);

        if status == 0 {
            return Err(format!("未收到 HTTP 响应（原始前 200 字符）：{}", &raw[..raw.len().min(200)]));
        }
        Ok((status, resp_body.to_string()))
    }
}

/// 把一次响应 JSON 解析成内核增量序列。
///
/// 抽成自由函数是为了**可单测**：不联网也能验证解析正确（含工具调用与错误形态）。
pub fn parse_completion(resp_body: &str) -> Result<Vec<ModelDelta>, String> {
    let v: serde_json::Value =
        serde_json::from_str(resp_body).map_err(|e| format!("响应不是合法 JSON：{e}"))?;

    if let Some(err) = v.get("error") {
        let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("未知错误");
        return Err(format!("模型返回错误：{msg}"));
    }

    let choice = v
        .get("choices")
        .and_then(|c| c.get(0))
        .ok_or("响应缺少 choices[0]")?;
    let msg = choice.get("message").ok_or("choices[0] 缺少 message")?;

    let mut deltas = Vec::new();

    if let Some(text) = msg.get("content").and_then(|c| c.as_str()) {
        if !text.is_empty() {
            deltas.push(ModelDelta::Text(text.to_string()));
        }
    }

    if let Some(calls) = msg.get("tool_calls").and_then(|c| c.as_array()) {
        for call in calls {
            let f = call.get("function").ok_or("tool_call 缺少 function")?;
            let name = f.get("name").and_then(|n| n.as_str()).ok_or("function 缺少 name")?;
            // arguments 是**字符串形式的 JSON**（OpenAI 规范），需二次解析
            let args_raw = f.get("arguments").and_then(|a| a.as_str()).unwrap_or("{}");
            let arguments: serde_json::Value =
                serde_json::from_str(args_raw).unwrap_or_else(|_| serde_json::json!({}));
            deltas.push(ModelDelta::ToolCall(ToolInvocation {
                id: call
                    .get("id")
                    .and_then(|i| i.as_str())
                    .unwrap_or("call-unknown")
                    .to_string(),
                name: name.to_string(),
                arguments,
            }));
        }
    }

    if let Some(usage) = v.get("usage") {
        deltas.push(ModelDelta::Usage {
            input_tokens: usage.get("prompt_tokens").and_then(|t| t.as_u64()).unwrap_or(0),
            output_tokens: usage.get("completion_tokens").and_then(|t| t.as_u64()).unwrap_or(0),
        });
    }

    Ok(deltas)
}

impl ModelProvider for DeepSeekProvider {
    fn name(&self) -> &str { "deepseek" }

    fn stream(&self, request: &ModelRequest<'_>) -> ModelStream {
        let body = self.build_body(request).to_string();
        let deltas = match self.post(&body).and_then(|r| parse_completion(&r)) {
            Ok(d) => d,
            Err(e) => vec![ModelDelta::Text(format!("[provider 错误] {e}"))],
        };
        Box::new(deltas.into_iter())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_plain_text_completion() {
        let body = r#"{"choices":[{"message":{"content":"hello"}}],"usage":{"prompt_tokens":3,"completion_tokens":1}}"#;
        let d = parse_completion(body).unwrap();
        assert!(matches!(&d[0], ModelDelta::Text(t) if t == "hello"));
        assert!(matches!(d.last(), Some(ModelDelta::Usage { input_tokens: 3, output_tokens: 1 })));
    }

    #[test]
    fn parses_a_tool_call_with_stringified_arguments() {
        // OpenAI 规范里 arguments 是**字符串**形式的 JSON —— 常见实现错误是当对象读
        let body = r#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"c1","type":"function","function":{"name":"bash","arguments":"{\"cmd\":\"ls\"}"}}]}}]}"#;
        let d = parse_completion(body).unwrap();
        match &d[0] {
            ModelDelta::ToolCall(c) => {
                assert_eq!(c.name, "bash");
                assert_eq!(c.arguments["cmd"], "ls");
                assert_eq!(c.id, "c1");
            }
            other => panic!("应解析出工具调用，实际 {other:?}"),
        }
    }

    #[test]
    fn surfaces_a_model_side_error() {
        let body = r#"{"error":{"message":"invalid api key"}}"#;
        let e = parse_completion(body).unwrap_err();
        assert!(e.contains("invalid api key"), "应透出模型侧错误：{e}");
    }

    #[test]
    fn rejects_a_response_without_choices() {
        assert!(parse_completion(r#"{"id":"x"}"#).is_err());
    }

    #[test]
    fn encodes_tool_results_as_tool_role_messages() {
        let p = DeepSeekProvider {
            api_key: "k".into(), endpoint: "e".into(), path: "/p".into(),
            model: "m".into(), temperature: 0.0,
        };
        let msgs = vec![
            Message::User("hi".into()),
            Message::Assistant {
                text: String::new(),
                tool_calls: vec![ToolInvocation {
                    id: "c1".into(), name: "bash".into(),
                    arguments: serde_json::json!({"cmd":"ls"}),
                }],
            },
            Message::ToolResult {
                id: "c1".into(), name: "bash".into(),
                output: neo_protocol::ToolOutput {
                    exit_code: 0, stdout: "a.txt".into(), stderr: String::new(), truncated: false,
                },
            },
        ];
        let tools: Vec<neo_core::ToolSchema> = Vec::new();
        let req = ModelRequest { system: "sys", messages: &msgs, tools: &tools };
        let encoded = p.encode_messages(&req);

        assert_eq!(encoded[0]["role"], "system");
        assert_eq!(encoded[1]["role"], "user");
        assert_eq!(encoded[2]["role"], "assistant");
        assert_eq!(encoded[2]["tool_calls"][0]["function"]["name"], "bash");
        assert_eq!(encoded[3]["role"], "tool");
        assert_eq!(encoded[3]["tool_call_id"], "c1");
    }
}

// ─────────────── 零网络的确定性 provider（离线模式 / 链路自检）───────────

/// 脚本化 provider：按**步**给出预定响应。
///
/// 两种用途，语义不同：
/// - `text_only(text)`：离线模式。只回一句话，用于不联网时验证
///   「装配 → 内核 → 沙箱 → 落盘」这条链路本身。
/// - `scripted(steps)`：**链路自检**。按预定脚本调用工具（如 apply_patch），
///   用于在无 API key 的前提下验证「模型 → 工具 → 真实落盘」的完整循环。
///
/// 步序推断是确定性的：按请求里 assistant 消息的条数定位脚本下标。
/// 因此同一请求序列必得同一输出（T2 可回放）。
pub struct ScriptedProvider {
    pub script: Vec<Vec<ModelDelta>>,
    pub tail: String,
}

impl ScriptedProvider {
    pub fn text_only(text: &str) -> Self {
        Self { script: Vec::new(), tail: text.to_string() }
    }

    /// 按步给出响应；每一步用完后回落到 `tail`。
    pub fn scripted(steps: Vec<Vec<ModelDelta>>, tail: &str) -> Self {
        Self { script: steps, tail: tail.to_string() }
    }
}

impl ModelProvider for ScriptedProvider {
    fn name(&self) -> &str { "mock" }

    fn stream(&self, req: &ModelRequest<'_>) -> ModelStream {
        let step = req
            .messages
            .iter()
            .filter(|m| matches!(m, Message::Assistant { .. }))
            .count();
        let deltas = self
            .script
            .get(step)
            .cloned()
            .unwrap_or_else(|| vec![ModelDelta::Text(self.tail.clone())]);
        Box::new(deltas.into_iter())
    }
}

/// 便利构造：一次工具调用增量（供 `--provider selftest` 的脚本使用）。
pub fn tool_call(name: &str, args: serde_json::Value) -> ModelDelta {
    ModelDelta::ToolCall(ToolInvocation {
        // 确定性 id：自检模式必须可回放，不能用随机数
        id: format!("selftest-{name}"),
        name: name.to_string(),
        arguments: args,
    })
}

/// 用生产同一套帧构造，对任意 host/path 发一个 POST —— 供集成测试验证帧正确性。
///
/// 抽出来是为了让"HTTP 帧是否正确"可被**独立验证**：不必有 API key，
/// 打公共回显端点即可确认 Content-Length、Host、Connection 等头都正确。
pub fn probe_post(host: &str, path: &str, body: &str) -> Result<(u16, String), String> {
    let probe = DeepSeekProvider {
        api_key: "probe-not-a-real-key".to_string(),
        endpoint: host.to_string(),
        path: path.to_string(),
        model: "n/a".to_string(),
        temperature: 0.0,
    };
    probe.post_raw(body)
}
