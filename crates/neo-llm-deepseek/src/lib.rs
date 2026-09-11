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
pub const DEFAULT_ENDPOINT: &str = "api.deepseek.com";pub const DEFAULT_PATH: &str = "/chat/completions";
pub const DEFAULT_MODEL: &str = "deepseek-chat";
pub const DEFAULT_LABEL: &str = "deepseek";

/// 把（可能有问题的）响应文本截成一段**可安全打印**的预览：
/// 控制字符替换成 `.`（直接打印二进制会把终端弄乱），长度上限 200 字符。
/// 换行保留 —— 多行 JSON 的可读性依赖它。
fn preview_text(s: &str) -> String {
    s.chars()
        .take(200)
        .map(|c| if c.is_control() && c != '\n' && c != '\r' { '.' } else { c })
        .collect()
}

pub struct DeepSeekProvider {
    pub api_key: String,
    pub endpoint: String,
    pub path: String,
    pub model: String,
    /// 传给模型的温度。默认 0（确定性优先，便于回放与测试）。
    pub temperature: f32,
    /// 自报名。默认 `"deepseek"`；由服务商注册表构造时设为注册名。
    ///
    /// # 为什么需要它
    ///
    /// `ModelRegistry::new` 校验"注册名 == provider 自报名"，以免两处各说一套。
    /// 但同一个 OpenAI 兼容客户端可指向**任意**网关 —— 自建代理也会自报
    /// `deepseek`，于是以别名注册时校验失败（"注册名 my-gateway 与自报名
    /// deepseek 不一致"）。名字必须跟着**用途**走，不能硬编码。
    pub label: String,
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
            label: DEFAULT_LABEL.to_string(),
        })
    }

    /// 覆盖自报名（用于别名/网关注册）。
    pub fn with_label(mut self, l: impl Into<String>) -> Self { self.label = l.into(); self }

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
                    // **stderr 必须带上**：拒绝、沙箱拦截、命令失败的原因都在
                    // stderr 里。只发 stdout 会让模型看到"成功但无输出" ——
                    // 实测时它真的据此推断"工具返回空、无报错"，然后请求重试，
                    // 完全不知道是自己被审批拒绝了。
                    let mut content = output.stdout.clone();
                    if !output.stderr.trim().is_empty() {
                        if !content.is_empty() && !content.ends_with('\n') {
                            content.push('\n');
                        }
                        content.push_str("[stderr] ");
                        content.push_str(&output.stderr);
                    }
                    // 非零退出码也明确告知：模型需要区分"命令成功"与"命令失败"
                    if output.exit_code != 0 {
                        content.push_str(&format!("\n[exit_code] {}", output.exit_code));
                    }
                    if output.truncated {
                        content.push_str("\n[truncated] 输出已被截断");
                    }
                    out.push(serde_json::json!({
                        "role": "tool",
                        "tool_call_id": id,
                        "name": name,
                        "content": content,
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
    /// 组装一次请求的原始文本。
    ///
    /// 独立成方法是为了**可单测**：有些头不发就会出问题，但它们的效果
    /// 只在真机才显现（例如压缩）。抽出来才能在单测里断言"这个头确实在"。
    fn build_request(&self, body: &str) -> String {
        // `Accept-Encoding: identity` 是**显式要求不压缩**。
        //
        // 不发它时，有些网关仍会 gzip 响应体 —— 而响应是按文本解析的，
        // 收到 gzip 字节就变成"响应不是合法 UTF-8"（用户真机报过这个错）。
        // 在请求侧声明 identity 比在客户端解压简单，也少一个失败点。
        format!(
            "POST {path} HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {key}\r\nContent-Type: application/json\r\nAccept: application/json\r\nAccept-Encoding: identity\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{body}",
            path = self.path,
            host = self.endpoint,
            key = self.api_key,
            len = body.as_bytes().len(),
            body = body,
        )
    }

    pub fn post_raw(&self, body: &str) -> Result<(u16, String), String> {
        let request = self.build_request(body);

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

        // 按**字节**读，再自行解码。
        //
        // 之前用 `read_to_string`：响应里只要有一个非 UTF-8 字节就整体失败，
        // 报出"stream did not contain valid UTF-8" —— 用户既看不到状态码、
        // 也看不到服务商到底回了什么，完全无从下手。真实场景里非 UTF-8 很常见：
        // 网关把错误页返回成二进制，或响应里夹了非 UTF-8 字段。
        let mut bytes: Vec<u8> = Vec::new();
        {
            let mut out = child.stdout.take().ok_or("openssl stdout 不可用")?;
            out.read_to_end(&mut bytes)
                .map_err(|e| format!("读取响应失败：{e}"))?;
        }
        let _ = child.wait();
        let raw = match String::from_utf8(bytes) {
            Ok(s) => s,
            Err(e) => {
                let lossy = String::from_utf8_lossy(e.as_bytes()).into_owned();
                // 连 HTTP 状态行都认不出：多半是压缩体/二进制错误页。
                // 如实说明并给一段可打印预览 —— 比一句 "invalid UTF-8" 有用得多。
                if !lossy.starts_with("HTTP/") {
                    return Err(format!(
                        "响应不是合法 UTF-8（多半是压缩体或二进制错误页）；预览：{}",
                        preview_text(&lossy)
                    ));
                }
                lossy
            }
        };

        // 拆 HTTP 头 / 体
        let body_start = raw.find("\r\n\r\n").map(|i| i + 4).unwrap_or(0);
        let head = &raw[..body_start.min(raw.len())];
        let raw_body = &raw[body_start.min(raw.len())..];

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

        // **必须处理 chunked**：真实 API 走 HTTP/1.1 分块传输，
        // 响应体形如 `1d1\r\n{...}\r\n0\r\n\r\n` —— 直接把这段交给
        // serde_json 会得到 "trailing characters at line 1 column 2"
        // （它把长度前缀 `1d1` 当成数字字面量 `1`，后面 `d` 就成了多余字符）。
        // 之前的帧格式测试用 httpbin，那个服务用 Content-Length，
        // 所以这条路径一直没被覆盖 —— 只有真实 API 才暴露。
        let chunked = head.lines().any(|l| {
            let l = l.to_ascii_lowercase();
            l.starts_with("transfer-encoding") && l.contains("chunked")
        });
        let resp_body = if chunked {
            decode_chunked(raw_body)?
        } else {
            raw_body.to_string()
        };
        Ok((status, resp_body))
    }
}

/// 解码 HTTP/1.1 chunked 传输编码。
///
/// 格式：`<十六进制长度>[;扩展]CRLF <数据> CRLF`，重复；以长度 0 结束，
/// 之后可能有 trailer（可忽略）。
///
/// 为什么必须自己解：我们走 `openssl s_client` 裸谈 HTTP，没有 HTTP 库
/// 帮忙。真实 API 用 chunked，不解就会把长度前缀送进 JSON 解析器。
///
/// 内存有界：按声明长度 `with_capacity`，并对**异常大的声明长度**设上限 ——
/// 避免一个坏响应让我们预先分配巨量内存。
pub fn decode_chunked(body: &str) -> Result<String, String> {
    /// 单块上限（16 MiB）。真实响应的单块通常几十 KB 到几 MB。
    const MAX_CHUNK: usize = 16 * 1024 * 1024;

    let mut out = String::new();
    let mut rest = body;
    loop {
        // 块头：十六进制长度，可能带 `;ext`
        let Some(nl) = rest.find("\r\n") else {
            // 没有更多块头：若剩余全是空白就正常结束，否则格式不对
            return if rest.trim().is_empty() {
                Ok(out)
            } else {
                Err("chunked 响应格式错误：块头缺少 CRLF".into())
            };
        };
        let size_line = &rest[..nl];
        let size_hex = size_line.split(';').next().unwrap_or("").trim();
        // 空长度行 = 端点（有些实现用裸 CRLF 结尾）
        if size_hex.is_empty() {
            return Ok(out);
        }
        let size = usize::from_str_radix(size_hex, 16)
            .map_err(|_| format!("chunked 块长度不是合法十六进制：{size_hex:?}"))?;
        if size > MAX_CHUNK {
            return Err(format!("chunked 单块长度 {size} 超出上限 {MAX_CHUNK}"));
        }
        if size == 0 {
            return Ok(out); // 结束块；trailer 可忽略
        }
        let data_start = nl + 2;
        let data_end = data_start
            .checked_add(size)
            .filter(|e| *e <= rest.len())
            .ok_or_else(|| format!("chunked 数据不完整：声明 {size} 字节但剩余不足"))?;
        out.push_str(&rest[data_start..data_end]);
        // 块数据后应跟 CRLF
        rest = rest[data_end..].strip_prefix("\r\n").unwrap_or(&rest[data_end..]);
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

    // 推理过程（DeepSeek 的 `reasoning_content`）。**先于正文**入列，
    // 与模型产出顺序一致（先想后说）。缺失/为空就跳过 —— 非推理模型没有这段。
    if let Some(r) = msg.get("reasoning_content").and_then(|c| c.as_str()) {
        if !r.is_empty() {
            deltas.push(ModelDelta::Reasoning(r.to_string()));
        }
    }

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
    fn name(&self) -> &str { &self.label }

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

    // ── chunked 传输解码 ──────────────────────────────────────────────
    //
    // 这些用例是照着**真实响应的字节**写的：DeepSeek API 走 chunked，
    // 手动拼出分块格式，确保不联网也能覆盖这条路径。

    #[test]
    fn decodes_a_single_chunk() {
        let body = "1d1\r\n{\"ok\":true}\r\n0\r\n\r\n";
        // 长度按其真实字节数算，避免手写错误
        let payload = "{\"ok\":true}";
        let manual = format!("{:x}\r\n{payload}\r\n0\r\n\r\n", payload.len());
        assert_eq!(decode_chunked(&manual).unwrap(), payload);
        let _ = body;
    }

    #[test]
    fn decodes_multiple_chunks_and_concatenates() {
        let a = "{\"a\":";
        let b = "1}";
        let raw = format!(
            "{:x}\r\n{a}\r\n{:x}\r\n{b}\r\n0\r\n\r\n",
            a.len(),
            b.len()
        );
        assert_eq!(decode_chunked(&raw).unwrap(), "{\"a\":1}");
    }

    #[test]
    fn tolerates_chunk_extensions_and_trailers() {
        // 有些实现会在长度后带 `;ext`，结束块后带 trailer —— 都要能跳过
        let p = "{\"x\":1}";
        let raw = format!("{:x};ext=1\r\n{p}\r\n0\r\nX-Trace: abc\r\n\r\n", p.len());
        assert_eq!(decode_chunked(&raw).unwrap(), p);
    }

    #[test]
    fn handles_uppercase_hex_lengths() {
        let p = "A".repeat(0x1A);
        let raw = format!("1A\r\n{p}\r\n0\r\n\r\n");
        assert_eq!(decode_chunked(&raw).unwrap(), p);
    }

    #[test]
    fn decodes_a_realistic_api_response_prefix() {
        // 真实响应实测以 `1d1\r\n{...` 开头；这里用缩小的等价结构
        let json = r#"{"id":"x","object":"chat.completion","choices":[{"message":{"content":"你好"}}]}"#;
        let raw = format!("{:x}\r\n{json}\r\n0\r\n\r\n", json.len());
        let decoded = decode_chunked(&raw).unwrap();
        assert_eq!(decoded, json);
        // 解出来的必须是合法 JSON（这才是最初的目的）
        assert!(serde_json::from_str::<serde_json::Value>(&decoded).is_ok());
    }

    #[test]
    fn chunked_decoding_is_what_makes_json_parse_work() {
        // 正向证明：不解码则 JSON 解析失败（复现真实报错），解码后成功。
        let json = r#"{"ok":true}"#;
        let raw = format!("{:x}\r\n{json}\r\n0\r\n\r\n", json.len());
        assert!(
            serde_json::from_str::<serde_json::Value>(&raw).is_err(),
            "原始 chunked 文本不该能当 JSON 解析（这正是修复前的症状）"
        );
        let decoded = decode_chunked(&raw).unwrap();
        assert!(serde_json::from_str::<serde_json::Value>(&decoded).is_ok());
    }

    #[test]
    fn malformed_chunked_is_an_error_not_a_silent_truncation() {
        // 声明长度超过实际数据：必须报错，不能悄悄给一段截断的 JSON
        let raw = "100\r\nshort\r\n0\r\n\r\n";
        assert!(decode_chunked(raw).is_err(), "声明 256 字节但只有 5 字节，应报错");
        // 长度不是十六进制
        assert!(decode_chunked("zz\r\ndata\r\n").is_err());
    }

    #[test]
    fn oversized_chunk_is_rejected_before_allocating() {
        // 内存有界：异常大的单块声明要在分配前拒绝
        let raw = "7fffffff\r\nx\r\n0\r\n\r\n";
        let err = decode_chunked(raw).unwrap_err();
        assert!(err.contains("上限"), "应提示超过上限：{err}");
    }

    #[test]
    fn empty_body_is_an_empty_chunk_stream() {
        assert_eq!(decode_chunked("0\r\n\r\n").unwrap(), "");
    }

    #[test]
    fn tool_result_carries_stderr_and_exit_code_to_the_model() {
        // 真实教训：只发 stdout 会让"被审批拒绝"看起来像"成功但无输出"。
        // 实测时模型据此说"工具返回空、无报错"，然后要求重试 —— 它完全
        // 不知道是自己被拒了。
        let msgs = vec![Message::ToolResult {
            id: "c1".into(),
            name: "apply_patch".into(),
            output: neo_protocol::ToolOutput {
                exit_code: -1,
                stdout: String::new(),
                stderr: "用户拒绝了该调用".into(),
                truncated: false,
            },
        }];
        let tools: Vec<neo_core::ToolSchema> = Vec::new();
        let req = ModelRequest { system: "sys", messages: &msgs, tools: &tools };
        let p = DeepSeekProvider {
            endpoint: "example.invalid".into(),
            path: "/x".into(),
            api_key: "test".into(),
            model: "m".into(),
            temperature: 0.0,
            label: DEFAULT_LABEL.into(),
        };
        let encoded = p.encode_messages(&req);
        // 系统消息在最前，工具结果在后面 —— 按 role 找，别硬取下标
        let content = encoded
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["role"] == "tool")
            .expect("应有 tool 消息")["content"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(content.contains("用户拒绝了该调用"), "拒绝原因必须到达模型：{content}");
        assert!(content.contains("-1"), "退出码必须到达模型：{content}");
    }

    #[test]
    fn successful_tool_result_without_stderr_is_not_polluted() {
        // 正常成功且无 stderr 时不该插入多余标记（否则提示词里全是噪声）
        let msgs = vec![Message::ToolResult {
            id: "c1".into(),
            name: "bash".into(),
            output: neo_protocol::ToolOutput {
                exit_code: 0,
                stdout: "file.txt".into(),
                stderr: String::new(),
                truncated: false,
            },
        }];
        let tools: Vec<neo_core::ToolSchema> = Vec::new();
        let req = ModelRequest { system: "sys", messages: &msgs, tools: &tools };
        let p = DeepSeekProvider {
            endpoint: "example.invalid".into(),
            path: "/x".into(),
            api_key: "test".into(),
            model: "m".into(),
            temperature: 0.0,
            label: DEFAULT_LABEL.into(),
        };
        let content = p
            .encode_messages(&req)
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["role"] == "tool")
            .expect("应有 tool 消息")["content"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(content, "file.txt", "干净的成功结果不该加标记：{content}");
    }

    #[test]
    fn truncated_tool_result_is_labelled_for_the_model() {
        // 模型必须知道输出被截断，否则会把不完整内容当成全部
        let msgs = vec![Message::ToolResult {
            id: "c1".into(),
            name: "bash".into(),
            output: neo_protocol::ToolOutput {
                exit_code: 0,
                stdout: "head".into(),
                stderr: String::new(),
                truncated: true,
            },
        }];
        let tools: Vec<neo_core::ToolSchema> = Vec::new();
        let req = ModelRequest { system: "sys", messages: &msgs, tools: &tools };
        let p = DeepSeekProvider {
            endpoint: "example.invalid".into(),
            path: "/x".into(),
            api_key: "test".into(),
            model: "m".into(),
            temperature: 0.0,
            label: DEFAULT_LABEL.into(),
        };
        let content = p
            .encode_messages(&req)
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["role"] == "tool")
            .expect("应有 tool 消息")["content"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(content.contains("truncated"), "应标注截断：{content}");
    }

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
    fn parses_reasoning_content_into_a_reasoning_delta() {
        // 少了这条，推理链路的**第一环**就是断的：协议与 TUI 都支持思考过程，
        // 但 provider 从不报上来，用户永远看不到。
        let body = r#"{"choices":[{"message":{
            "reasoning_content":"先看依赖方向。","content":"结论：合法。"}}]}"#;
        let deltas = parse_completion(body).unwrap();
        assert!(
            deltas.iter().any(|d| matches!(d, ModelDelta::Reasoning(t) if t.contains("依赖方向"))),
            "必须把 reasoning_content 解析成 Reasoning 增量：{deltas:?}"
        );
        // 顺序：先推理后正文（与模型产出顺序一致）
        let ri = deltas.iter().position(|d| matches!(d, ModelDelta::Reasoning(_))).unwrap();
        let ti = deltas.iter().position(|d| matches!(d, ModelDelta::Text(_))).unwrap();
        assert!(ri < ti, "推理应排在正文之前");
        // 非推理模型（无该字段）不该产生空增量
        let plain = r#"{"choices":[{"message":{"content":"hi"}}]}"#;
        assert!(!parse_completion(plain).unwrap().iter().any(|d| matches!(d, ModelDelta::Reasoning(_))));
    }

    #[test]
    fn preview_text_is_printable_and_bounded() {
        let got = preview_text("HTTP/1.1 200 OK\u{0007}\u{0001}body");
        assert!(!got.contains('\u{0007}'), "控制字符必须替换：{got:?}");
        assert!(got.contains("HTTP/1.1 200 OK"));
        assert!(preview_text("a\nb").contains('\n'), "换行要保留");
        assert!(preview_text(&"x".repeat(5000)).chars().count() <= 200, "预览要有上限");
    }

    #[test]
    fn request_declares_identity_encoding() {
        // 显式拒绝压缩：否则网关可能回 gzip，被当成"非法 UTF-8"。
        let p = DeepSeekProvider {
            api_key: "k".into(), endpoint: "e.invalid".into(), path: "/p".into(),
            model: "m".into(), temperature: 0.0, label: DEFAULT_LABEL.into(),
        };
        let req = p.build_request("{}");
        assert!(req.contains("Accept-Encoding: identity"), "缺 identity 声明：{req}");
        assert!(req.starts_with("POST /p HTTP/1.1"), "请求行应含 path：{req}");
    }

    #[test]
    fn encodes_tool_results_as_tool_role_messages() {
        let p = DeepSeekProvider {
            api_key: "k".into(), endpoint: "e".into(), path: "/p".into(),
            model: "m".into(), temperature: 0.0, label: DEFAULT_LABEL.into(),
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
    /// 自报名字。默认 `"mock"`（保持既有行为），可用 `with_name` 改成
    /// `demo` / `selftest` 等 —— 注册表要求"注册名 == 自报名"，
    /// 一个 provider 想以多个名字注册就必须能自报不同名字。
    pub label: String,
}

impl ScriptedProvider {
    pub fn text_only(text: &str) -> Self {
        Self { script: Vec::new(), tail: text.to_string(), label: "mock".into() }
    }

    /// 改自报名字（供注册表以不同名字注册同一类桩）。
    pub fn with_name(mut self, name: &str) -> Self {
        self.label = name.to_string();
        self
    }

    /// 按步给出响应；每一步用完后回落到 `tail`。
    /// 演示用：产出 markdown 与任务清单，便于人眼验证渲染效果。
    /// 它**不参与任何生产路径**，只让"高亮/清单到底长什么样"能被直接看到。
    pub fn demo() -> Self {
        Self::scripted(
            vec![vec![
                // 带一段推理：让"思考过程"这条链路**离线可验**。
                // 之前 demo 只发工具调用与正文，于是推理显示对不对
                // 根本无从检查（真实模型才有 reasoning_content）。
                ModelDelta::Reasoning(
                    "先看依赖方向是否合法，再确认缩进与行号，最后跑门禁验证。".to_string(),
                ),
                Self::tool_call_delta(
                    "todo1",
                    "todowrite",
                    serde_json::json!({
                        "items": [
                            {"content": "读取配置", "status": "completed"},
                            {"content": "实现高亮", "status": "in_progress"},
                            {"content": "补测试", "status": "pending"},
                        ]
                    }),
                ),
            ]],
            "# 完成情况\n\n这是一段**说明**，含 `cargo test` 行内代码。\n\n- 第一项\n- 第二项\n\n```rust\nfn main() {\n    let s = \"hi\"; // 注释\n    println!(\"{}\", s);\n}\n```\n\n> 引用：注意代码块已高亮。",
        )
    }

    fn tool_call_delta(id: &str, name: &str, args: serde_json::Value) -> ModelDelta {
        ModelDelta::ToolCall(neo_core::ToolInvocation {
            id: id.to_string(),
            name: name.to_string(),
            arguments: args,
        })
    }

    pub fn scripted(steps: Vec<Vec<ModelDelta>>, tail: &str) -> Self {
        Self { script: steps, tail: tail.to_string(), label: "mock".into() }
    }
}

impl ModelProvider for ScriptedProvider {
    fn name(&self) -> &str { &self.label }

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
        label: DEFAULT_LABEL.to_string(),
    };
    probe.post_raw(body)
}
