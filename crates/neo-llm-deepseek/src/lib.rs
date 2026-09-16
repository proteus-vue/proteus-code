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
//! # 流式（SSE）
//!
//! 请求带 `stream: true`，响应按 **SSE 逐事件**解析：`data:` 行 →
//! `choices[0].delta` 的 `reasoning_content` / `content` / `tool_calls` 分片，
//! `data: [DONE]` 结束。迭代器是**惰性**的：每个 `ModelDelta` 在网络字节到达时
//! 才产出 —— 内核据此把思考过程实时交给宿主显示。
//!
//! 任何 OpenAI 兼容网关都走这一条实现（智谱 glm 系列与 DeepSeek 共用），
//! 字段按两家都用的 OpenAI 形态读取（`reasoning_content`、`stream_options.include_usage`）。
//!
//! 兼容兜底：响应体以 `{` 开头（网关忽略 `stream` 标志回了整段 JSON，或
//! 本地桩服务器）时退回 `parse_completion` 一次性解析 —— 流式实现**对
//! 不配合的网关保持可用**，只是失去实时性。
//!
//! # 诚实边界
//!
//! - **每次请求一个进程**：`openssl s_client` 走进程，有进程启动开销。
//!   生产实现应改用常驻连接 —— 当前优先"能跑通且可调试"。
//! - **未处理**：重试、超时细分、代理、tool_choice 强制。

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
                            // 结构化参数 schema:模型靠它得到参数的形状契约。
                            // 真机回归(glm-4.6)发现没有它时模型会把数组
                            // 参数字符串化(双重编码),反复重试浪费预算。
                            "parameters": t.parameters,
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
            // **真流式**：缺了它整条响应是一次性 JSON，思考过程就只能在
            // 全部生成完后才可见（真实反馈："运行中转圈到最后一次性出结果"）。
            "stream": true,
            // 用量随流返回（OpenAI 规范字段；DeepSeek 与智谱的兼容端点都支持）。
            // 开了它最终会有一个 `choices` 为空数组、只带 usage 的 chunk。
            "stream_options": { "include_usage": true },
        });
        if let Some(tools) = self.encode_tools(req) {
            body["tools"] = tools;
        }
        body
    }

    /// 发一次请求，返回（状态码, 响应体）。**不做状态码判断** ——
    /// 让调用方（含集成测试）能看到真实状态。
    /// 组装一次请求的原始文本。
    ///
    /// 独立成方法是为了**可单测**：有些头不发就会出问题，但它们的效果
    /// 只在真机才显现（例如压缩）。抽出来才能在单测里断言"这个头确实在"。
    fn build_request_with(&self, body: &str, accept: &str) -> String {
        // `Accept-Encoding: identity` 是**显式要求不压缩**。
        //
        // 不发它时，有些网关仍会 gzip 响应体 —— 而响应是按文本解析的，
        // 收到 gzip 字节就变成"响应不是合法 UTF-8"（用户真机报过这个错）。
        // 在请求侧声明 identity 比在客户端解压简单，也少一个失败点。
        format!(
            "POST {path} HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {key}\r\nContent-Type: application/json\r\nAccept: {accept}\r\nAccept-Encoding: identity\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{body}",
            path = self.path,
            host = self.endpoint,
            key = self.api_key,
            accept = accept,
            len = body.as_bytes().len(),
            body = body,
        )
    }

    /// 非流式请求帧（`post_raw` / probe 集成测试用）。
    pub fn build_request(&self, body: &str) -> String {
        self.build_request_with(body, "application/json")
    }

    /// 流式请求帧：`Accept: text/event-stream` 声明我们按 SSE 读响应。
    fn build_stream_request(&self, body: &str) -> String {
        self.build_request_with(body, "text/event-stream")
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

// ─────────────── 流式响应（SSE 增量解析）───────────────
//
// 与上面的整段路径（post_raw + decode_chunked + parse_completion）并存：
// 流式路径**不等完整响应**，字节到达即产出增量 —— "实时显示思考过程"
// 依赖这一点。帧解析逻辑与整段版一致，只是从"一次吃完"改成"逐字节状态机"。

/// SSE 单行上限。正常事件只有几百字节；超限说明对端不是 SSE，
/// 继续缓冲只会吃内存（内存有界性是内核义务）。
const MAX_SSE_LINE: usize = 1024 * 1024;
/// JSON 兜底模式的整段缓冲上限（与 chunked 单块上限同级）。
const MAX_FALLBACK_BODY: usize = 16 * 1024 * 1024;
/// HTTP 响应头读取上限。
const MAX_HEAD: usize = 64 * 1024;
/// 错误响应体的读取上限（只用于拼错误消息）。
const MAX_ERROR_BODY: usize = 256 * 1024;

/// 从流里读出 HTTP 响应头（到 `\r\n\r\n`），返回 (状态码, 是否 chunked)。
///
/// 逐字节读经 `BufReader` 摊薄系统调用；多读进缓冲区的体字节不会丢 ——
/// 后续从同一个 `BufReader` 继续读。
fn read_response_head<R: Read>(r: &mut R) -> Result<(u16, bool), String> {
    let mut head: Vec<u8> = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let n = r.read(&mut byte).map_err(|e| format!("读取响应失败：{e}"))?;
        if n == 0 {
            return Err("未收到 HTTP 响应（连接提前关闭）".into());
        }
        head.push(byte[0]);
        if head.len() > MAX_HEAD {
            return Err("HTTP 响应头超过上限".into());
        }
        if head.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    let status = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| format!("无法解析 HTTP 状态行：{}", preview_text(&head)))?;
    let chunked = head.to_ascii_lowercase().lines().any(|l| {
        l.starts_with("transfer-encoding") && l.contains("chunked")
    });
    Ok((status, chunked))
}

/// 增量负载读取器：从任意 `Read` 拉原始字节，产出**已剥掉 chunked 帧**的负载。
///
/// 与 `decode_chunked`（整段、一次性）同一套帧规则，改成逐字节状态机：
/// 流式路径不能等全部数据到齐。分块模式下每读到一个负载字节就返回，
/// 保证低延迟（思考文本到一笔显一笔）。
struct PayloadReader<R: Read> {
    inner: R,
    chunked: bool,
    /// 块长行累积（块与块之间的字节）
    size_line: String,
    /// 当前块剩余字节数（0 = 不在块数据中）
    remaining: usize,
    /// 终止块（长度 0）已收到
    finished: bool,
}

impl<R: Read> PayloadReader<R> {
    fn new(inner: R, chunked: bool) -> Self {
        Self { inner, chunked, size_line: String::new(), remaining: 0, finished: false }
    }

    /// 读出至少一个负载字节（阻塞直到有数据或流结束）。`Ok(false)` = 流到头。
    fn read_payload(&mut self, out: &mut Vec<u8>) -> Result<bool, String> {
        if self.finished {
            return Ok(false);
        }
        if !self.chunked {
            // 非分块（Content-Length / 连接关闭定界）：整段透传
            let mut buf = [0u8; 8192];
            let n = self.inner.read(&mut buf).map_err(|e| format!("读取响应失败：{e}"))?;
            if n == 0 {
                self.finished = true;
                return Ok(false);
            }
            out.extend_from_slice(&buf[..n]);
            return Ok(true);
        }
        let mut byte = [0u8; 1];
        loop {
            let n = self.inner.read(&mut byte).map_err(|e| format!("读取响应失败：{e}"))?;
            if n == 0 {
                // 连接关闭：在块长行等待中（无半截数据）算正常结束；
                // 其余位置断开 = 数据不完整，必须报错而不是静默截断
                return if self.remaining > 0 || !self.size_line.is_empty() {
                    Err("chunked 数据不完整：连接提前关闭".into())
                } else {
                    self.finished = true;
                    Ok(false)
                };
            }
            let b = byte[0];
            if self.remaining > 0 {
                out.push(b);
                self.remaining -= 1;
                return Ok(true); // 有负载即返回，别为凑批牺牲延迟
            }
            // 块外字节：累积成块长行（空行 = 数据块后的 CRLF，跳过）
            if b == b'\n' {
                let line = std::mem::take(&mut self.size_line);
                let line = line.trim_end_matches('\r');
                if line.is_empty() {
                    continue;
                }
                let hex = line.split(';').next().unwrap_or("").trim();
                let size = usize::from_str_radix(hex, 16)
                    .map_err(|_| format!("chunked 块长度不是合法十六进制：{hex:?}"))?;
                const MAX_CHUNK: usize = 16 * 1024 * 1024; // 与 decode_chunked 同一上限
                if size > MAX_CHUNK {
                    return Err(format!("chunked 单块长度 {size} 超出上限 {MAX_CHUNK}"));
                }
                if size == 0 {
                    self.finished = true; // 终止块；trailer 随连接关闭一起丢弃
                    return Ok(false);
                }
                self.remaining = size;
            } else {
                if self.size_line.len() >= 64 {
                    return Err("chunked 块长行异常过长".into());
                }
                self.size_line.push(b as char);
            }
        }
    }
}

/// 流式 tool_calls 的分片累积。参数按 OpenAI 规范**分片字符串**到达，
/// 按 `index` 拼接；`finish_reason` 出现时才算收齐。
struct ToolAcc {
    id: String,
    name: String,
    args: String,
}

/// 响应体形态。首个非空白字节判定：`{` = 整段 JSON（兜底），否则按 SSE。
#[derive(PartialEq)]
enum BodyMode {
    Undecided,
    Sse,
    Json,
}

/// SSE 流 → [`ModelDelta`] 迭代器（惰性：字节到达才产出）。
///
/// 生产路径 `R = BufReader<ChildStdout>`（openssl s_client 管道），
/// 测试路径 `R = Cursor<Vec<u8>>` —— 同一套解析代码，不联网可测。
/// 构造**不读网络**：响应头在首个 `next()` 时解析，配置错误（非 2xx、
/// 非 HTTP）以 `[provider 错误] …` 文本增量透出，与旧路径同形。
struct SseDeltaStream<R: Read + Send + 'static> {
    /// 生产路径持有子进程；Drop（或终结）时 kill，中断后不留 openssl 残留。
    child: Option<std::process::Child>,
    state: StreamState<R>,
    /// SSE 行缓冲（≤ MAX_SSE_LINE）
    line: Vec<u8>,
    mode: BodyMode,
    /// JSON 兜底的整段缓冲
    json_buf: Vec<u8>,
    tools: std::collections::BTreeMap<usize, ToolAcc>,
    tools_flushed: bool,
    /// 是否收到过任何 data 事件（区分"空响应"与"半途断开"）
    received_any: bool,
    /// 是否见过 finish_reason（没有 [DONE] 但见到它也算完整收尾）
    saw_finish: bool,
    pending: std::collections::VecDeque<ModelDelta>,
    done: bool,
    err: Option<String>,
}

enum StreamState<R: Read> {
    /// 响应头未读（首个 next() 时解析）
    Head(R),
    /// 头已解析，按 chunked/identity 出负载
    Body(PayloadReader<R>),
    /// 已终结，不再碰网络
    Closed,
}

impl<R: Read + Send + 'static> SseDeltaStream<R> {
    fn new(child: Option<std::process::Child>, reader: R) -> Self {
        Self {
            child,
            state: StreamState::Head(reader),
            line: Vec::new(),
            mode: BodyMode::Undecided,
            json_buf: Vec::new(),
            tools: Default::default(),
            tools_flushed: false,
            received_any: false,
            saw_finish: false,
            pending: Default::default(),
            done: false,
            err: None,
        }
    }

    /// 终结：关闭网络端并 kill 子进程。
    fn close(&mut self) {
        self.state = StreamState::Closed;
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    /// 解析响应头；非 2xx 时读错误体并返回可读错误。
    fn open_body(r: &mut R) -> Result<bool, String> {
        let (status, chunked) = read_response_head(r)?;
        if !(200..300).contains(&status) {
            let mut rest = Vec::new();
            let _ = r.by_ref().take(MAX_ERROR_BODY as u64).read_to_end(&mut rest);
            let text = String::from_utf8_lossy(&rest).into_owned();
            let body = if chunked { decode_chunked(&text).unwrap_or(text) } else { text };
            return Err(format!("HTTP {status}：{}", body.trim()));
        }
        Ok(chunked)
    }

    /// 从网络拉一批字节并推进解析。`Ok(false)` = 流到头。
    fn pull(&mut self, p: &mut PayloadReader<R>) -> Result<bool, String> {
        let mut raw = Vec::new();
        if !p.read_payload(&mut raw)? {
            return Ok(false);
        }
        self.ingest(&raw)?;
        Ok(true)
    }

    fn ingest(&mut self, raw: &[u8]) -> Result<(), String> {
        if self.mode == BodyMode::Undecided {
            let first = raw.iter().copied().find(|b| !b.is_ascii_whitespace());
            match first {
                Some(b'{') => self.mode = BodyMode::Json,
                Some(_) => self.mode = BodyMode::Sse,
                // 还全是空白，形态未定，等下一批
                None => return Ok(()),
            }
        }
        match self.mode {
            BodyMode::Json => {
                self.json_buf.extend_from_slice(raw);
                if self.json_buf.len() > MAX_FALLBACK_BODY {
                    return Err("响应体超出上限（16 MiB）".into());
                }
            }
            BodyMode::Sse => {
                for &b in raw {
                    if self.done || self.err.is_some() {
                        break; // 已终结：剩余字节没有意义了
                    }
                    if b == b'\n' {
                        let line = std::mem::take(&mut self.line);
                        self.handle_line(&line);
                    } else {
                        if self.line.len() >= MAX_SSE_LINE {
                            return Err("SSE 单行超过上限".into());
                        }
                        self.line.push(b);
                    }
                }
            }
            BodyMode::Undecided => {}
        }
        Ok(())
    }

    fn handle_line(&mut self, raw_line: &[u8]) {
        let line = if raw_line.ends_with(b"\r") { &raw_line[..raw_line.len() - 1] } else { raw_line };
        let Some(rest) = line.strip_prefix(b"data:".as_slice()) else {
            return; // event:/id:/retry:/注释/空行都与增量无关
        };
        let text = String::from_utf8_lossy(rest);
        let payload = text.strip_prefix(' ').unwrap_or(&text);
        self.received_any = true;
        if payload.trim() == "[DONE]" {
            self.flush_tools();
            self.done = true;
            return;
        }
        self.handle_event(payload);
    }

    fn handle_event(&mut self, payload: &str) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(payload) else {
            self.err = Some(format!("SSE 事件不是合法 JSON：{}", preview_text(payload)));
            return;
        };
        if let Some(err) = v.get("error") {
            let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("未知错误");
            self.err = Some(format!("模型返回错误：{msg}"));
            return;
        }
        if let Some(choice) = v.get("choices").and_then(|c| c.get(0)) {
            if let Some(d) = choice.get("delta") {
                // 推理过程先于正文（模型先想后说），这里按到达顺序产出，
                // 与整段版"先 Reasoning 后 Text"的顺序约定一致。
                if let Some(r) = d.get("reasoning_content").and_then(|c| c.as_str()) {
                    if !r.is_empty() {
                        self.pending.push_back(ModelDelta::Reasoning(r.to_string()));
                    }
                }
                if let Some(t) = d.get("content").and_then(|c| c.as_str()) {
                    if !t.is_empty() {
                        self.pending.push_back(ModelDelta::Text(t.to_string()));
                    }
                }
                if let Some(fragments) = d.get("tool_calls").and_then(|c| c.as_array()) {
                    for f in fragments {
                        let idx = f.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                        let acc = self
                            .tools
                            .entry(idx)
                            .or_insert_with(|| ToolAcc { id: String::new(), name: String::new(), args: String::new() });
                        if let Some(id) = f.get("id").and_then(|i| i.as_str()) {
                            if !id.is_empty() {
                                acc.id = id.to_string();
                            }
                        }
                        if let Some(func) = f.get("function") {
                            if let Some(name) = func.get("name").and_then(|n| n.as_str()) {
                                if !name.is_empty() {
                                    acc.name = name.to_string();
                                }
                            }
                            if let Some(args) = func.get("arguments").and_then(|a| a.as_str()) {
                                acc.args.push_str(args);
                                if acc.args.len() > MAX_FALLBACK_BODY {
                                    self.err = Some("工具调用参数超出上限（16 MiB）".into());
                                    return;
                                }
                            }
                        }
                    }
                }
            }
            // finish_reason 出现：参数分片已收齐，冲刷成完整工具调用
            if choice.get("finish_reason").and_then(|f| f.as_str()).is_some() {
                self.saw_finish = true;
                self.flush_tools();
            }
        }
        if let Some(usage) = v.get("usage").filter(|u| !u.is_null()) {
            self.pending.push_back(ModelDelta::Usage {
                input_tokens: usage.get("prompt_tokens").and_then(|t| t.as_u64()).unwrap_or(0),
                output_tokens: usage.get("completion_tokens").and_then(|t| t.as_u64()).unwrap_or(0),
            });
        }
    }

    /// 把分片累积的工具调用落成完整 `ToolCall` 增量（按 index 序）。
    fn flush_tools(&mut self) {
        if self.tools_flushed {
            return;
        }
        self.tools_flushed = true;
        for (_, acc) in std::mem::take(&mut self.tools) {
            // arguments 是字符串形式的 JSON（OpenAI 规范），需二次解析；
            // 解析失败按空参数处理，与整段版 parse_completion 同一策略
            let arguments: serde_json::Value =
                serde_json::from_str(&acc.args).unwrap_or_else(|_| serde_json::json!({}));
            self.pending.push_back(ModelDelta::ToolCall(ToolInvocation {
                id: if acc.id.is_empty() { "call-unknown".to_string() } else { acc.id },
                name: acc.name,
                arguments,
            }));
        }
    }

    /// 流到头（EOF）时的收尾。
    fn finish_stream(&mut self) {
        match self.mode {
            BodyMode::Json => {
                let text = String::from_utf8_lossy(&self.json_buf).into_owned();
                match parse_completion(&text) {
                    Ok(deltas) => self.pending.extend(deltas),
                    Err(e) => self.err = Some(e),
                }
            }
            _ => {
                self.flush_tools();
                if !self.received_any {
                    self.err = Some("响应为空（未收到任何数据）".into());
                } else if !self.saw_finish {
                    // 既没 [DONE] 也没 finish_reason 就断了：内容不完整，
                    // 如实报错而不是把半截回答当完整答案
                    self.err = Some("连接在响应完成前关闭".into());
                }
            }
        }
        self.done = true;
    }
}

impl<R: Read + Send + 'static> Iterator for SseDeltaStream<R> {
    type Item = ModelDelta;

    fn next(&mut self) -> Option<ModelDelta> {
        loop {
            if let Some(d) = self.pending.pop_front() {
                return Some(d);
            }
            // err 先于 done：收尾路径（finish_stream）可能同时置两者，
            // 错误增量必须先透出，然后才允许流终结
            if let Some(e) = self.err.take() {
                self.done = true;
                return Some(ModelDelta::Text(format!("[provider 错误] {e}")));
            }
            if self.done {
                self.close();
                return None;
            }
            match std::mem::replace(&mut self.state, StreamState::Closed) {
                StreamState::Head(mut r) => match Self::open_body(&mut r) {
                    Ok(chunked) => {
                        self.state = StreamState::Body(PayloadReader::new(r, chunked));
                    }
                    Err(e) => {
                        // r 在此丢弃：管道关闭，openssl 随 Drop 清理
                        self.err = Some(e);
                    }
                },
                StreamState::Body(mut p) => match self.pull(&mut p) {
                    Ok(true) => self.state = StreamState::Body(p),
                    Ok(false) => {
                        self.state = StreamState::Body(p);
                        self.finish_stream();
                    }
                    Err(e) => {
                        self.state = StreamState::Body(p);
                        self.err = Some(e);
                    }
                },
                StreamState::Closed => self.done = true,
            }
        }
    }
}

impl<R: Read + Send + 'static> Drop for SseDeltaStream<R> {
    fn drop(&mut self) {
        self.close();
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
        // 组装即发请求（写 openssl 管道）；响应头与首个增量在迭代器
        // 首次被消费时才读 —— 组装失败（起不了进程）在此就地转错误增量，
        // 其余错误（非 2xx、断流）由迭代器以同一形态透出。
        let body = self.build_body(request).to_string();
        let req = self.build_stream_request(&body);
        let opened = (|| -> Result<SseDeltaStream<std::io::BufReader<std::process::ChildStdout>>, String> {
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
                .write_all(req.as_bytes())
                .map_err(|e| format!("写入请求失败：{e}"))?;
            // 关闭 stdin 让 s_client 发完即读
            drop(child.stdin.take());
            let stdout = child.stdout.take().ok_or("openssl stdout 不可用")?;
            Ok(SseDeltaStream::new(Some(child), std::io::BufReader::with_capacity(64 * 1024, stdout)))
        })();
        match opened {
            Ok(s) => Box::new(s),
            Err(e) => Box::new(std::iter::once(ModelDelta::Text(format!("[provider 错误] {e}")))),
        }
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

    // ── SSE 流式解析 ──────────────────────────────────────────────────
    //
    // 用例照着 DeepSeek / 智谱两家真实流式响应的字节形态写：
    // chunked 帧边界与 SSE 事件边界**不对齐**（真实网关如此），
    // 解析器必须按字节状态机工作，而不是按行整读。

    /// 把 body 按给定切点切成 chunked 帧（帧边界故意与事件边界错开）。
    fn chunked_response(body: &str, sizes: &[usize]) -> Vec<u8> {
        let mut raw = String::from(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n",
        );
        let mut rest = body;
        let mut i = 0;
        while !rest.is_empty() {
            let mut n = sizes[i % sizes.len()].min(rest.len());
            // 帧边界不能落在多字节字符中间（借来的字节切不动）
            while n < rest.len() && !rest.is_char_boundary(n) {
                n += 1;
            }
            i += 1;
            raw.push_str(&format!("{:x}\r\n{}\r\n", n, &rest[..n]));
            rest = &rest[n..];
        }
        raw.push_str("0\r\n\r\n");
        raw.into_bytes()
    }

    fn stream_deltas(raw: Vec<u8>) -> Vec<ModelDelta> {
        SseDeltaStream::new(None, std::io::Cursor::new(raw)).collect()
    }

    fn sse_body(events: &[&str]) -> String {
        events
            .iter()
            .map(|e| format!("data: {e}\n\n"))
            .collect()
    }

    #[test]
    fn sse_stream_yields_reasoning_then_text_deltas() {
        let body = &sse_body(&[
            r#"{"id":"1","choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":"先看"}}]}"#,
            r#"{"id":"1","choices":[{"index":0,"delta":{"reasoning_content":"依赖方向。"}}]}"#,
            r#"{"id":"1","choices":[{"index":0,"delta":{"content":"结论："}}]}"#,
            r#"{"id":"1","choices":[{"index":0,"delta":{"content":"合法。"},"finish_reason":"stop"}]}"#,
            // include_usage 的收尾 chunk：choices 为空数组、只带 usage
            r#"{"id":"1","choices":[],"usage":{"prompt_tokens":3,"completion_tokens":2}}"#,
            "[DONE]",
        ]);
        let d = stream_deltas(chunked_response(body, &[13]));
        assert_eq!(
            d,
            vec![
                ModelDelta::Reasoning("先看".into()),
                ModelDelta::Reasoning("依赖方向。".into()),
                ModelDelta::Text("结论：".into()),
                ModelDelta::Text("合法。".into()),
                ModelDelta::Usage { input_tokens: 3, output_tokens: 2 },
            ],
            "增量必须按到达顺序实时产出：{d:?}"
        );
    }

    #[test]
    fn sse_tool_call_fragments_are_accumulated_by_index() {
        // 流式 tool_calls：arguments 按分片字符串到达，finish_reason 时收齐
        let body = &sse_body(&[
            r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"c1","type":"function","function":{"name":"bash","arguments":""}}]}}]}"#,
            r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"cmd\":"}}]}}]}"#,
            r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"ls\"}"}}]}}]}"#,
            r#"{"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
            r#"{"choices":[],"usage":{"prompt_tokens":1,"completion_tokens":1}}"#,
            "[DONE]",
        ]);
        let d = stream_deltas(chunked_response(body, &[29]));
        assert_eq!(d.len(), 2, "分片应聚合成一次完整工具调用：{d:?}");
        match &d[0] {
            ModelDelta::ToolCall(c) => {
                assert_eq!(c.id, "c1");
                assert_eq!(c.name, "bash");
                assert_eq!(c.arguments["cmd"], "ls");
            }
            other => panic!("应解析出工具调用，实际 {other:?}"),
        }
        assert!(matches!(&d[1], ModelDelta::Usage { input_tokens: 1, output_tokens: 1 }));
    }

    #[test]
    fn sse_ignores_comment_and_event_lines() {
        let body = ": keep-alive\nevent: ping\ndata: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"好\"}}]}\n\ndata: [DONE]\n\n";
        let d = stream_deltas(chunked_response(body, &[11]));
        assert_eq!(d, vec![ModelDelta::Text("好".into())], "{d:?}");
    }

    #[test]
    fn json_fallback_when_gateway_ignores_stream_flag() {
        // 网关/桩服务器无视 stream:true 回整段 JSON：必须仍可用（失去实时性但不出错）
        let raw = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"choices\":[{\"message\":{\"reasoning_content\":\"想\",\"content\":\"答\"}}]}".as_bytes().to_vec();
        let d = stream_deltas(raw);
        assert_eq!(d, vec![ModelDelta::Reasoning("想".into()), ModelDelta::Text("答".into())], "{d:?}");
    }

    #[test]
    fn http_error_surfaces_as_provider_error_text() {
        let raw = "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\n\r\n{\"error\":{\"message\":\"invalid api key\"}}".as_bytes().to_vec();
        let d = stream_deltas(raw);
        assert_eq!(d.len(), 1);
        match &d[0] {
            ModelDelta::Text(t) => {
                assert!(t.contains("[provider 错误] HTTP 401"), "{t}");
                assert!(t.contains("invalid api key"), "错误原因必须可见：{t}");
            }
            other => panic!("应透出错误增量，实际 {other:?}"),
        }
    }

    #[test]
    fn truncated_stream_is_an_error_not_silent_partial() {
        // 既没 [DONE] 也没 finish_reason 就断流：已到的增量照常产出，
        // 但必须跟着一条错误 —— 把半截回答当完整答案是撒谎
        let body = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"写了一半\"}}]}\n\n";
        let d = stream_deltas(chunked_response(body, &[9]));
        assert!(matches!(&d[0], ModelDelta::Text(t) if t == "写了一半"), "{d:?}");
        let last = d.last().expect("应有错误增量");
        assert!(
            matches!(last, ModelDelta::Text(t) if t.contains("连接在响应完成前关闭")),
            "断流必须显式报错：{d:?}"
        );
    }

    #[test]
    fn empty_response_is_an_error_like_the_batched_path() {
        let d = stream_deltas(chunked_response("", &[1]));
        assert!(
            matches!(&d[0], ModelDelta::Text(t) if t.contains("响应为空")),
            "{d:?}"
        );
    }

    #[test]
    fn body_declares_stream_and_usage_options() {
        let p = DeepSeekProvider {
            api_key: "k".into(), endpoint: "e".into(), path: "/p".into(),
            model: "m".into(), temperature: 0.0, label: DEFAULT_LABEL.into(),
        };
        let msgs = vec![Message::User("hi".into())];
        let tools: Vec<neo_core::ToolSchema> = Vec::new();
        let req = ModelRequest { system: "sys", messages: &msgs, tools: &tools };
        let body = p.build_body(&req);
        assert_eq!(body["stream"], true, "必须请求流式");
        assert_eq!(body["stream_options"]["include_usage"], true, "用量必须随流返回");
    }

    #[test]
    fn stream_request_declares_event_stream_accept() {
        let p = DeepSeekProvider {
            api_key: "k".into(), endpoint: "e.invalid".into(), path: "/p".into(),
            model: "m".into(), temperature: 0.0, label: DEFAULT_LABEL.into(),
        };
        let req = p.build_stream_request("{}");
        assert!(req.contains("Accept: text/event-stream"), "流式请求要声明 SSE：{req}");
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
    tool_call_n(0, name, args)
}

/// 同上，但带序号 —— **一轮内多次调用必须用不同的 `seq`**。
///
/// 工具调用是靠 `id` 配对的：同名工具调用两次而 id 相同，第二遍的
/// `ToolCallEnd` 会认领错卡片（界面上表现为第一次永远停在"执行中"）。
/// 真实模型每次调用给的都是新的 id，桩必须同样如此 —— 这是脚本自己的
/// 契约，不是可以省掉的细节。
pub fn tool_call_n(seq: usize, name: &str, args: serde_json::Value) -> ModelDelta {
    ModelDelta::ToolCall(ToolInvocation {
        // 确定性 id：自检模式必须可回放，不能用随机数
        id: format!("selftest-{seq}-{name}"),
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
