//! MCP stdio 客户端：启动服务器进程、握手、列工具、调用工具。
//!
//! # 进程边界是可替换的
//!
//! [`ChildProcess`] 抽象了"一个会说换行 JSON 的对端"。真实实现
//! [`StdioChild`] 起真进程；测试用 `MockChild` 在进程内演协议 ——
//! 客户端逻辑（握手、分页、超时、错误映射）不依赖真进程就能测，
//! 而**线格式与真进程的兼容性**由带 `cfg(unix)` 的真进程集成测试另测
//! （桩测不出缓冲与字节边界，这与沙箱必须真机验证是同一条纪律）。
//!
//! # 所有等待都有界
//!
//! 每个阶段独立超时；超时后连接标记为**已污染**（迟到的响应无法与
//! 后续请求配对，继续使用只会张冠李戴）—— 之后所有调用立即失败。
//! stderr 由固定容量缓冲收尾，服务器刷屏不会把内存吃穿。

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use crate::wire::{self, Notification, Request, Response, PROTOCOL_VERSION};

/// 一个服务器连接可能失败的所有方式。每个变体都说清**该查哪里**。
#[derive(Debug)]
pub enum McpError {
    /// 进程没起来（命令不存在 / 无执行权限）
    Spawn(String),
    /// 管道 IO 失败
    Io(String),
    /// 对端说的不是我们期待的协议
    Protocol(String),
    /// 等待超时（连接已污染，不可再用）
    Timeout { method: &'static str, secs: u64 },
    /// 服务器返回了 JSON-RPC 错误
    Server { code: i64, message: String },
    /// 服务器进程退出（带 stderr 尾部）
    Exited { stderr_tail: String },
}

impl std::fmt::Display for McpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            McpError::Spawn(e) => write!(f, "无法启动 MCP 服务器：{e}"),
            McpError::Io(e) => write!(f, "MCP 服务器管道错误：{e}"),
            McpError::Protocol(e) => write!(f, "MCP 协议错误：{e}"),
            McpError::Timeout { method, secs } => {
                write!(f, "MCP 请求「{method}」等待 {secs}s 超时（连接已不可用，需重启 Neo）")
            }
            McpError::Server { code, message } => write!(f, "MCP 服务器错误（{code}）：{message}"),
            McpError::Exited { stderr_tail } => {
                write!(f, "MCP 服务器进程已退出。stderr 尾部：{stderr_tail}")
            }
        }
    }
}
impl std::error::Error for McpError {}

/// `recv_timeout` 的失败方式：超时 / 对端关闭。
#[derive(Debug, PartialEq, Eq)]
pub enum RecvError {
    Timeout,
    Closed,
}

/// 服务器进程的抽象（真进程 / 测试桩共用这条边界）。
pub trait ChildProcess: Send {
    fn send(&mut self, line: &str) -> std::io::Result<()>;
    /// 等一行 stdout；`Err(Timeout)`=超时未到，`Ok(None)`=对端关闭（EOF）。
    fn recv_timeout(&mut self, timeout: Duration) -> Result<Option<String>, RecvError>;
    fn exited(&mut self) -> bool;
    fn stderr_tail(&mut self) -> String;
    fn kill(&mut self);
}

/// stderr 环形缓冲的容量。只留尾部做诊断 —— 错误信息几乎总在最后。
const STDERR_TAIL_CAP: usize = 4096;

/// 真实子进程实现。stdout 由读线程搬运进有界通道（容量 64 行），
/// stderr 由守护线程持续排空进环形缓冲 —— 不排空的话管道缓冲区写满后
/// 服务器会阻塞在 stderr 上（沙箱测试抓过的同类死锁，见 PROJECT_MEMORY 4.7）。
pub struct StdioChild {
    child: Child,
    stdin: std::process::ChildStdin,
    stdout_rx: mpsc::Receiver<String>,
    stderr_tail: Arc<Mutex<VecDeque<u8>>>,
}

impl StdioChild {
    pub fn spawn(command: &str, args: &[String]) -> Result<Self, McpError> {
        let mut child = Command::new(command)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| McpError::Spawn(format!("{command}: {e}")))?;
        let stdin = child.stdin.take().ok_or_else(|| McpError::Spawn("stdin 不可用".into()))?;
        let stdout = child.stdout.take().ok_or_else(|| McpError::Spawn("stdout 不可用".into()))?;
        let stderr = child.stderr.take().ok_or_else(|| McpError::Spawn("stderr 不可用".into()))?;

        // 有界通道：满 = 服务器刷屏快于消费，丢最老的行。
        // 保住"读线程永远在排空"这个不死锁的性质，比不丢行重要。
        let (tx, rx) = mpsc::sync_channel::<String>(64);
        std::thread::Builder::new()
            .name("mcp-stdout".into())
            .spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let _ = tx.send(line.unwrap_or_default());
                }
            })
            .map_err(|e| McpError::Spawn(format!("读线程启动失败：{e}")))?;

        let tail = Arc::new(Mutex::new(VecDeque::with_capacity(STDERR_TAIL_CAP)));
        let tail2 = tail.clone();
        std::thread::Builder::new()
            .name("mcp-stderr".into())
            .spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    let mut buf = tail2.lock().unwrap_or_else(|e| e.into_inner());
                    for b in line.bytes().chain(std::iter::once(b'\n')) {
                        if buf.len() == STDERR_TAIL_CAP {
                            buf.pop_front();
                        }
                        buf.push_back(b);
                    }
                }
            })
            .map_err(|e| McpError::Spawn(format!("stderr 线程启动失败：{e}")))?;

        Ok(Self { child, stdin, stdout_rx: rx, stderr_tail: tail })
    }
}

impl ChildProcess for StdioChild {
    fn send(&mut self, line: &str) -> std::io::Result<()> {
        self.stdin.write_all(line.as_bytes())?;
        self.stdin.write_all(b"\n")?;
        self.stdin.flush()
    }
    fn recv_timeout(&mut self, timeout: Duration) -> Result<Option<String>, RecvError> {
        match self.stdout_rx.recv_timeout(timeout) {
            Ok(line) => Ok(Some(line)),
            Err(RecvTimeoutError::Timeout) => Err(RecvError::Timeout),
            Err(RecvTimeoutError::Disconnected) => Ok(None),
        }
    }
    fn exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }
    fn stderr_tail(&mut self) -> String {
        let buf = self.stderr_tail.lock().unwrap_or_else(|e| e.into_inner());
        String::from_utf8_lossy(&buf.iter().copied().collect::<Vec<u8>>()).into_owned()
    }
    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// 与一个 MCP 服务器的会话。`Drop` 时杀掉服务器进程。
pub struct McpClient {
    child: Box<dyn ChildProcess>,
    next_id: AtomicU64,
    /// 超时后置位：迟到的响应无法与后续请求配对，连接不可再用。
    poisoned: bool,
}

/// 列工具的硬上限（分页循环的退出条件 —— 无界循环是缺陷）。
const MAX_TOOL_PAGES: usize = 100;
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
pub const LIST_TIMEOUT: Duration = Duration::from_secs(15);
pub const CALL_TIMEOUT: Duration = Duration::from_secs(120);

/// `tools/list` 里的一条工具描述（只取本客户端关心的字段）。
#[derive(Debug, Clone)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    /// JSON Schema（原样给模型看 —— 它是服务器声明的契约）
    pub input_schema: Value,
    /// `annotations.readOnlyHint`：服务器**声明**的只读提示。
    /// 没有 hint 就按写处理（把写误判成读会绕过审批，代价不对称）。
    pub read_only_hint: Option<bool>,
}

/// 一次工具调用的结果。
#[derive(Debug, Clone)]
pub struct CallOutcome {
    /// 全部 text 内容按序拼接；非 text 内容用诚实占位符标出
    pub text: String,
    /// MCP 的 `isError`：工具执行出错（协议层成功，语义层失败）
    pub is_error: bool,
}

impl McpClient {
    /// 启动服务器（stdio 进程或 HTTP 端点）并完成握手。
    pub fn spawn(spec: &ServerSpec) -> Result<Self, McpError> {
        // 传输决定协议版本：stdio 锁 2024-11-05；Streamable HTTP 自
        // 2025-03-26 引入，接受其后继。版本协商只接受已知集合 ——
        // 静默接受未知版本会让两端对消息语义各有各的理解。
        let (child, accepted): (Box<dyn ChildProcess>, &[&str]) = match &spec.url {
            Some(url) => (Box::new(HttpChild::new(url)), &HTTP_PROTOCOL_VERSIONS),
            None => (Box::new(StdioChild::spawn(&spec.command, &spec.args)?), &[PROTOCOL_VERSION]),
        };
        let mut client = Self { child, next_id: AtomicU64::new(1), poisoned: false };
        let params = json!({
            "protocolVersion": accepted[0],
            "capabilities": {},
            "clientInfo": {"name": "neo", "version": env!("CARGO_PKG_VERSION")},
        });
        let result = client.request("initialize", Some(params), HANDSHAKE_TIMEOUT)?;
        Self::check_version(&result, accepted)?;
        client.notify("notifications/initialized", None)?;
        Ok(client)
    }

    /// 版本协商检查（`spawn` 用；独立成函数以便直测）。
    /// 只接受声明的版本集合。
    fn check_version(result: &Value, accepted: &[&str]) -> Result<(), McpError> {
        let got = result.get("protocolVersion").and_then(Value::as_str).unwrap_or("");
        if !accepted.contains(&got) {
            return Err(McpError::Protocol(format!(
                "协议版本不一致：客户端 {}，服务器 {got:?}",
                accepted.join(" / ")
            )));
        }
        Ok(())
    }

    /// 从测试桩构造（不走真握手）。仅 crate 内测试可用。
    #[cfg(test)]
    pub(crate) fn with_child(child: Box<dyn ChildProcess>) -> Self {
        Self { child, next_id: AtomicU64::new(1), poisoned: false }
    }

    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }

    /// 发请求并等响应。id 不匹配的消息继续等（超时兜底）。
    fn request(
        &mut self,
        method: &'static str,
        params: Option<Value>,
        timeout: Duration,
    ) -> Result<Value, McpError> {
        if self.poisoned {
            return Err(McpError::Protocol("连接已因先前的超时被污染".into()));
        }
        let id = self.next_id();
        let req = Request { jsonrpc: "2.0", id, method, params: params.as_ref() };
        let line = serde_json::to_string(&req).map_err(|e| McpError::Io(e.to_string()))?;
        self.child.send(&line).map_err(|e| McpError::Io(e.to_string()))?;

        loop {
            match self.child.recv_timeout(timeout) {
                Ok(Some(text)) => match wire::parse_line(&text).map_err(McpError::Protocol)? {
                    Some(Response::Ok { id: rid, result }) if rid == id => return Ok(result),
                    Some(Response::Err { id: rid, code, message }) if rid == Some(id) => {
                        return Err(McpError::Server { code, message })
                    }
                    // id 不匹配 / 通知：丢弃并继续等（超时兜底）
                    _ => continue,
                },
                Ok(None) => {
                    return Err(McpError::Exited { stderr_tail: self.child.stderr_tail() });
                }
                Err(RecvError::Timeout) => {
                    self.poisoned = true;
                    return Err(McpError::Timeout { method, secs: timeout.as_secs() });
                }
                Err(RecvError::Closed) => {
                    return Err(McpError::Exited { stderr_tail: self.child.stderr_tail() });
                }
            }
        }
    }

    fn notify(&mut self, method: &str, params: Option<Value>) -> Result<(), McpError> {
        let n = Notification { jsonrpc: "2.0", method, params: params.as_ref() };
        let line = serde_json::to_string(&n).map_err(|e| McpError::Io(e.to_string()))?;
        self.child.send(&line).map_err(|e| McpError::Io(e.to_string()))
    }

    /// 列出服务器的全部工具（自动跟随分页游标）。
    pub fn list_tools(&mut self) -> Result<Vec<ToolInfo>, McpError> {
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_TOOL_PAGES {
            let params = cursor.as_ref().map(|c| json!({"cursor": c}));
            let result = self.request("tools/list", params, LIST_TIMEOUT)?;
            let tools = result.get("tools").and_then(Value::as_array).cloned().unwrap_or_default();
            for t in tools {
                let name = t.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
                if name.is_empty() {
                    continue; // 没名字的工具无法被调用，跳过而不是让整个列表失败
                }
                let read_only_hint = t
                    .get("annotations")
                    .and_then(|a| a.get("readOnlyHint"))
                    .and_then(Value::as_bool);
                out.push(ToolInfo {
                    name,
                    description: t.get("description").and_then(Value::as_str).unwrap_or("").to_string(),
                    input_schema: t.get("inputSchema").cloned().unwrap_or(json!({"type": "object"})),
                    read_only_hint,
                });
            }
            cursor = result
                .get("nextCursor")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(String::from);
            if cursor.is_none() {
                return Ok(out);
            }
        }
        Err(McpError::Protocol(format!("tools/list 分页超过 {MAX_TOOL_PAGES} 页仍未结束")))
    }

    /// 调用一个工具。
    pub fn call_tool(&mut self, name: &str, args: &Value) -> Result<CallOutcome, McpError> {
        let params = json!({"name": name, "arguments": args});
        let result = self.request("tools/call", Some(params), CALL_TIMEOUT)?;
        let is_error = result.get("isError").and_then(Value::as_bool).unwrap_or(false);
        let mut text = String::new();
        if let Some(items) = result.get("content").and_then(Value::as_array) {
            for item in items {
                let ty = item.get("type").and_then(Value::as_str).unwrap_or("unknown");
                if !text.is_empty() {
                    text.push('\n');
                }
                match ty {
                    "text" => text.push_str(item.get("text").and_then(Value::as_str).unwrap_or("")),
                    // 诚实占位：图片/资源等非文本内容不能装作不存在
                    other => text.push_str(&format!("[非文本内容：{other}]")),
                }
            }
        }
        Ok(CallOutcome { text, is_error })
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        self.child.kill();
    }
}

/// 服务器传输声明（来自用户级配置）。
///
/// 两种形态二选一：`command`（stdio，本机进程）或 `url`（Streamable
/// HTTP，远程/内网服务器）。配置校验强制恰填其一。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ServerSpec {
    pub name: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub url: Option<String>,
}

/// HTTP 传输接受的协议版本（Streamable HTTP 自 2025-03-26 引入；
/// 2025-06-18 是其后继）。stdio 保持 2024-11-05 严格单一版本。
pub const HTTP_PROTOCOL_VERSIONS: [&str; 2] = ["2025-03-26", "2025-06-18"];

/// Streamable HTTP 传输：把"POST JSON → 读回 JSON/短 SSE"映射到
/// `ChildProcess` 的行协议上。
///
/// 映射关系（刻意的）：`send(line)` = POST 一条 JSON-RPC 请求，把响应
/// 解出的消息排进队列；`recv` = 逐条吐出。对 [`McpClient`] 而言对端
/// 仍是"会说换行 JSON 的东西"，握手/分页/超时/错误映射全部复用。
/// 会话由 `Mcp-Session-Id` 响应头建立并在后续请求回传。
pub struct HttpChild {
    url: String,
    session_id: Option<String>,
    pending: std::collections::VecDeque<String>,
    last_error: Option<String>,
}

impl HttpChild {
    pub fn new(url: &str) -> Self {
        Self { url: url.to_string(), session_id: None, pending: Default::default(), last_error: None }
    }
}

impl ChildProcess for HttpChild {
    fn send(&mut self, line: &str) -> std::io::Result<()> {
        self.pending.clear(); // HTTP 是请求/响应式：新请求作废旧残影
        let mut headers: Vec<(&str, String)> = Vec::new();
        if let Some(sid) = &self.session_id {
            headers.push(("Mcp-Session-Id", sid.clone()));
        }
        let resp = crate::http::post_json(&self.url, line, &headers, std::time::Duration::from_secs(120))
            .map_err(std::io::Error::other)?;
        if let Some(sid) = resp.header("mcp-session-id") {
            self.session_id = Some(sid.to_string());
        }
        match resp.status {
            200 => {
                let is_sse = resp
                    .header("content-type")
                    .map(|c| c.contains("text/event-stream"))
                    .unwrap_or(false);
                if is_sse {
                    self.pending.extend(crate::http::sse_data_lines(&resp.body));
                } else if !resp.body.trim().is_empty() {
                    self.pending.push_back(resp.body.trim().to_string());
                }
                Ok(())
            }
            // 202 = 通知已收（无响应体）—— 队列留空即可
            202 => Ok(()),
            status => {
                let excerpt: String = resp.body.chars().take(200).collect();
                let msg = format!("HTTP {status}: {excerpt}");
                self.last_error = Some(msg.clone());
                Err(std::io::Error::other(msg))
            }
        }
    }
    fn recv_timeout(&mut self, _timeout: Duration) -> Result<Option<String>, RecvError> {
        // 响应在 send 时已完整到达：队列空 = 本轮消息取完
        Ok(self.pending.pop_front())
    }
    fn exited(&mut self) -> bool {
        false // 远端服务器没有"进程退出"的概念
    }
    fn stderr_tail(&mut self) -> String {
        self.last_error.clone().unwrap_or_default()
    }
    fn kill(&mut self) {
        self.session_id = None; // 丢弃会话（无连接可杀）
    }
}

#[cfg(test)]
pub(crate) mod mock {
    use super::*;

    /// 脚本桩：预先排好"服务器要发的话"，按 `recv` 顺序吐出；
    /// 记录收到的每一行供断言。不做任何校验 —— 校验是断言的事。
    pub(crate) struct MockChild {
        pub script: Mutex<std::collections::VecDeque<Option<String>>>,
        pub received: Mutex<Vec<String>>,
    }

    impl MockChild {
        pub(crate) fn new(lines: Vec<Option<String>>) -> Self {
            Self { script: Mutex::new(lines.into()), received: Mutex::new(Vec::new()) }
        }
        pub(crate) fn received(&self) -> Vec<String> {
            self.received.lock().unwrap_or_else(|e| e.into_inner()).clone()
        }
    }

    impl ChildProcess for MockChild {
        fn send(&mut self, line: &str) -> std::io::Result<()> {
            self.received.lock().unwrap_or_else(|e| e.into_inner()).push(line.to_string());
            Ok(())
        }
        fn recv_timeout(&mut self, _timeout: Duration) -> Result<Option<String>, RecvError> {
            let mut script = self.script.lock().unwrap_or_else(|e| e.into_inner());
            // 立即出队（测试不等真实时间）；空了 = EOF
            Ok(script.pop_front().flatten())
        }
        fn exited(&mut self) -> bool {
            false
        }
        fn stderr_tail(&mut self) -> String {
            String::new()
        }
        fn kill(&mut self) {}
    }
}

#[cfg(test)]
mod tests {
    use super::mock::MockChild;
    use super::*;

    /// 服务器的握手响应（id 必须与请求匹配；脚本里放占位 1，
    /// 因为客户端从 1 开始编号）。
    fn handshake_ok(id: u64) -> String {
        format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{{"protocolVersion":"{PROTOCOL_VERSION}","capabilities":{{}},"serverInfo":{{"name":"t","version":"0"}}}}}}"#)
    }

    /// 走完握手的桩客户端，返回 (client, child)。
    fn client_with(script: Vec<Option<String>>) -> (McpClient, Arc<Mutex<MockChild>>) {
        let child = Arc::new(Mutex::new(MockChild::new(script)));
        let bound = MockBinding(child.clone());
        let c = McpClient::with_child(Box::new(bound));
        (c, child)
    }

    // 把响应脚本包上请求 id（客户端从 1 开始编号，跳过握手的测试
    // 脚本必须从 id=1 开始）。避免每个测试手写 id 出错。
    fn resp(id: u64, result: &serde_json::Value) -> String {
        format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{result}}}"#)
    }

    /// 把 `Arc<Mutex<MockChild>>` 适配成 ChildProcess（测试专用）。
    struct MockBinding(Arc<Mutex<MockChild>>);
    impl ChildProcess for MockBinding {
        fn send(&mut self, line: &str) -> std::io::Result<()> {
            self.0.lock().unwrap_or_else(|e| e.into_inner()).send(line)
        }
        fn recv_timeout(&mut self, t: Duration) -> Result<Option<String>, RecvError> {
            self.0.lock().unwrap_or_else(|e| e.into_inner()).recv_timeout(t)
        }
        fn exited(&mut self) -> bool {
            false
        }
        fn stderr_tail(&mut self) -> String {
            String::new()
        }
        fn kill(&mut self) {}
    }

    #[test]
    fn handshake_sends_initialize_then_initialized_notification() {
        let (mut c, child) = client_with(vec![Some(handshake_ok(1)), None]);
        // 手动驱动一次握手路径（spawn 需要真进程，这里直接调 request）
        let params = json!({"protocolVersion": PROTOCOL_VERSION});
        let r = c.request("initialize", Some(params), LIST_TIMEOUT).unwrap();
        assert_eq!(r["protocolVersion"], PROTOCOL_VERSION);
        let sent = child.lock().unwrap_or_else(|e| e.into_inner()).received();
        assert!(sent[0].contains(r#""method":"initialize""#), "{sent:?}");
        assert!(sent[0].contains(r#""id":1"#), "首个请求 id 应为 1（确定性编号）");
    }

    #[test]
    fn list_tools_follows_pagination_cursor() {
        // 第一页给 1 个工具 + nextCursor，第二页给 1 个工具（无游标）。
        // 跳过握手：脚本响应从 id=1 开始对齐客户端计数器。
        let page1 = json!({"tools":[{"name":"a","description":"A","inputSchema":{"type":"object"}}],"nextCursor":"p2"});
        let page2 = json!({"tools":[{"name":"b","description":"B","inputSchema":{"type":"object"}}]});
        let (mut c, child) = client_with(vec![Some(resp(1, &page1)), Some(resp(2, &page2))]);
        let tools = c.list_tools().unwrap();
        assert_eq!(tools.len(), 2, "两页应合并");
        assert_eq!(tools[0].name, "a");
        assert_eq!(tools[1].name, "b");
        let sent = child.lock().unwrap_or_else(|e| e.into_inner()).received();
        assert!(sent[1].contains(r#""cursor":"p2""#), "第二页请求必须带游标：{sent:?}");
    }

    #[test]
    fn read_only_hint_is_parsed_and_absent_hint_is_none() {
        let tools_json = json!({"tools":[
            {"name":"ro","description":"","inputSchema":{"type":"object"},"annotations":{"readOnlyHint":true}},
            {"name":"rw","description":"","inputSchema":{"type":"object"}}
        ]});
        let (mut c, _) = client_with(vec![Some(resp(1, &tools_json))]);
        let tools = c.list_tools().unwrap();
        assert_eq!(tools[0].read_only_hint, Some(true));
        assert_eq!(tools[1].read_only_hint, None, "无 hint 不得臆造");
    }

    #[test]
    fn call_tool_joins_text_content_and_marks_non_text() {
        let result = json!({"content":[
            {"type":"text","text":"第一段"},
            {"type":"image","data":"...","mimeType":"image/png"},
            {"type":"text","text":"第二段"}
        ]});
        let (mut c, _) = client_with(vec![Some(resp(1, &result))]);
        let out = c.call_tool("x", &json!({})).unwrap();
        assert_eq!(out.text, "第一段\n[非文本内容：image]\n第二段");
        assert!(!out.is_error);
    }

    #[test]
    fn server_side_tool_error_maps_to_is_error() {
        let result = json!({"content":[{"type":"text","text":"boom"}],"isError":true});
        let (mut c, _) = client_with(vec![Some(resp(1, &result))]);
        let out = c.call_tool("x", &json!({})).unwrap();
        assert!(out.is_error, "isError 必须如实上抛");
        assert_eq!(out.text, "boom");
    }

    #[test]
    fn json_rpc_error_response_surfaces_code_and_message() {
        let (mut c, _) = client_with(vec![
            Some(r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"no such tool"}}"#.into()),
        ]);
        let err = c.call_tool("x", &json!({})).unwrap_err();
        assert!(matches!(err, McpError::Server { code: -32601, .. }), "{err:?}");
        assert!(err.to_string().contains("no such tool"));
    }

    #[test]
    fn timeout_poisons_the_connection() {
        // 连接被污染后必须立即拒绝后续请求，而不是继续用
        // （迟到的响应无法与后续请求配对，继续用只会张冠李戴）。
        let (mut c, _) = client_with(vec![]);
        c.poisoned = true; // 直接置位（等价于发生过一次超时）
        let err = c.request("tools/list", None, Duration::from_millis(1)).unwrap_err();
        assert!(matches!(err, McpError::Protocol(_)), "{err:?}");
        assert!(err.to_string().contains("污染"), "错误要说明连接不可再用的原因");
    }

    #[test]
    fn eof_maps_to_exited_error() {
        let (mut c, _) = client_with(vec![None]);
        let err = c.request("tools/list", None, LIST_TIMEOUT).unwrap_err();
        assert!(matches!(err, McpError::Exited { .. }), "{err:?}");
    }

    #[test]
    fn version_mismatch_is_an_explicit_error() {
        // 协商只接受声明的版本：不一致必须显式失败，绝不静默继续
        let bad = json!({"protocolVersion": "1999-01-01"});
        let err = McpClient::check_version(&bad, &[PROTOCOL_VERSION]).unwrap_err();
        assert!(err.to_string().contains("1999-01-01"), "错误要说清两边的版本：{err}");
        let good = json!({"protocolVersion": PROTOCOL_VERSION});
        assert!(McpClient::check_version(&good, &[PROTOCOL_VERSION]).is_ok());
        // HTTP 传输接受两个版本
        assert!(McpClient::check_version(&json!({"protocolVersion": "2025-06-18"}), &HTTP_PROTOCOL_VERSIONS).is_ok());
    }
}
