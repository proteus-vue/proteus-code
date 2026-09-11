//! L5 HOST · Web —— 零依赖 HTTP + SSE 宿主
//!
//! # 架构：内核独占一个线程，HTTP 线程只通过 channel 通信
//!
//! 内核（`Kernel`）不是 `Sync`，若让 HTTP 处理线程直接拿它就要加锁，
//! 而"持锁跑一轮模型调用"会把所有请求串行化 —— 那不是隔离，是假并发。
//!
//! 正确的做法：**内核独占一个工作线程**，HTTP 侧只发 Op、收事件：
//!
//! ```text
//!  浏览器 ──POST /api/turn──▶ HTTP 线程 ──mpsc(Op)──▶ 内核线程
//!  浏览器 ◀──SSE /api/events── HTTP 线程 ◀─broadcast(Event)─┘
//! ```
//!
//! 好处：内核保持单线程语义（不需要 `Sync`、没有锁竞争），
//! 而多个浏览器标签可以同时订阅事件流。
//!
//! # 铁律：不含业务逻辑
//!
//! 它只做三件事：解析 HTTP、把 Op 转给内核、把事件写成 SSE。
//! 事件到"用户可见事实"的映射仍在协议层（`facts_of`），与其它宿主一致。

pub mod broadcast;
pub mod http;
pub mod page;

use neo_core::{DiffSupport, HostBackend, HostCapabilities, ImageSupport};
use neo_protocol::{EventMsg, Fact, Op};
use std::io::Write;
use std::net::TcpListener;
use std::sync::mpsc::{channel, Receiver, Sender};

/// 输入解析：委派给 L0 协议层的共享实现（宿主不自造解析语义）。
pub use neo_protocol::parse_refs;

/// Web 宿主的 HostBackend 视图（T6 宿主等价性的比较对象）。
///
/// 它只累积事件、按协议层的 `facts_of` 抽取事实 —— 与 exec / TUI / desktop
/// 用的是同一个抽取器，因此"三宿主等价"是构造上成立的，不是巧合。
pub struct WebFacts {
    events: Vec<EventMsg>,
}

impl WebFacts {
    pub fn new() -> Self { Self { events: Vec::new() } }
}

impl Default for WebFacts {
    fn default() -> Self { Self::new() }
}

impl HostBackend for WebFacts {
    fn id(&self) -> &'static str { "web" }

    fn capabilities(&self) -> HostCapabilities {
        HostCapabilities {
            images: ImageSupport::External, // 浏览器经 URL 显示图片
            rich_text: true,                // 完整 HTML/CSS
            interactive_prompt: true,       // 能弹审批
            diffs: DiffSupport::Hunk,       // 可做 hunk 级交互
        }
    }

    fn consume(&mut self, event: &EventMsg) -> Result<(), String> {
        self.events.push(event.clone());
        Ok(())
    }

    fn facts(&self) -> Vec<Fact> { neo_protocol::facts_of(&self.events) }
}

/// 事件 → Web 线格式。
///
/// serde 默认是**外部标签**（`{"agent_message_delta":{"delta":"x"}}`），
/// 前端要写 `m[k].delta` 这种别扭的取值。宿主负责把它规范成扁平信封
/// （`{"kind":"agent_message_delta","delta":"x"}`）—— 这是**渲染适配**，
/// 不是业务逻辑：事实语义仍由协议层的 `facts_of` 定义。
pub fn wire_event(event: &EventMsg) -> String {
    let v = serde_json::to_value(event).unwrap_or(serde_json::Value::Null);
    let (kind, payload) = match v {
        serde_json::Value::Object(mut map) => {
            let k = map.keys().next().cloned().unwrap_or_default();
            let inner = map.remove(&k).unwrap_or(serde_json::Value::Null);
            (k, inner)
        }
        other => (String::new(), other),
    };
    let mut obj = match payload {
        serde_json::Value::Object(m) => m,
        other => {
            let mut m = serde_json::Map::new();
            if !other.is_null() {
                m.insert("value".to_string(), other);
            }
            m
        }
    };
    obj.insert("kind".to_string(), serde_json::Value::String(kind));
    serde_json::Value::Object(obj).to_string()
}

/// 已启动的 Web 宿主。
pub struct WebServer {
    pub addr: std::net::SocketAddr,
    ops: Sender<Op>,
}

impl WebServer {
    /// 提交一个 Op（供内部或测试使用）。
    pub fn submit(&self, op: Op) -> bool { self.ops.send(op).is_ok() }
}

/// 启动 Web 宿主。
///
/// `handle_op` 由调用方注入（通常是"把 Op 交给内核线程"），
/// 因此本 crate **不依赖内核的具体类型**，只依赖 `Op`/`EventMsg` 契据。
pub fn start<F>(
    bind: &str,
    mut handle_op: F,
) -> std::io::Result<(WebServer, std::thread::JoinHandle<()>)>
where
    F: FnMut(Op) -> Vec<EventMsg> + Send + 'static,
{
    let listener = TcpListener::bind(bind)?;
    let addr = listener.local_addr()?;
    let bus = broadcast::Broadcast::new();
    let (op_tx, op_rx): (Sender<Op>, Receiver<Op>) = channel();

    // 内核侧工作线程：独占处理 Op，把事件广播出去。
    let bus_for_worker = bus.clone();
    let kernel_thread = std::thread::spawn(move || {
        while let Ok(op) = op_rx.recv() {
            let events = handle_op(op);
            for e in &events {
                {
                    let json = wire_event(e);
                    let dropped = bus_for_worker.publish(&json);
                    if dropped > 0 {
                        eprintln!("[web] 因订阅者过慢，断开 {dropped} 个连接");
                    }
                }
            }
        }
    });

    let bus_for_http = bus.clone();
    let ops = op_tx.clone();
    let http_thread = std::thread::spawn(move || {
        let _ = http::serve(listener, move |stream, req| {
            route(stream, req, &bus_for_http, &ops);
        });
    });

    Ok((WebServer { addr, ops: op_tx }, {
        // 返回内核线程句柄（HTTP 线程随 listener 关闭而结束）
        let _ = http_thread;
        kernel_thread
    }))
}

/// 路由：只有三个固定路径，用精确匹配即可。
fn route(
    stream: &mut std::net::TcpStream,
    req: http::Request,
    bus: &broadcast::Broadcast,
    ops: &Sender<Op>,
) {
    // 路由只看路径部分（query 由具体 handler 自行解析）
    let path_only = req.path.split('?').next().unwrap_or("/").to_string();
    match (req.method.as_str(), path_only.as_str()) {
        ("GET", "/") | ("GET", "/index.html") => {
            let _ = http::write_response(stream, 200, "text/html; charset=utf-8", page::INDEX_HTML);
        }
        ("POST", "/api/turn") => {
            // 请求体就是任务文本（不要求 JSON 包装：更简单的客户端）
            let task = req.body.trim();
            if task.is_empty() {
                let _ = http::write_response(stream, 400, "text/plain; charset=utf-8", "任务为空");
                return;
            }
            // 解析 @file / #session / /command / $skill 引用，与其它宿主同一套输入语义
            let op = Op::UserTurn { text: task.to_string(), refs: parse_refs(task) };
            if ops.send(op).is_err() {
                let _ = http::write_response(stream, 503, "text/plain; charset=utf-8", "内核线程已退出");
                return;
            }
            let _ = http::write_response(stream, 200, "text/plain; charset=utf-8", "已提交");
        }
        ("GET", "/api/events") => {
            // SSE：订阅后持续写事件，直到客户端断开
            if http::write_sse_headers(stream).is_err() {
                return;
            }
            let sub = bus.subscribe();
            // 订阅确认，让客户端知道流已建立
            if http::write_sse_event(stream, "{\"kind\":\"subscribed\"}").is_err() {
                bus.unsubscribe(sub.id());
                return;
            }
            while let Some(msg) = sub.recv() {
                if http::write_sse_event(stream, &msg).is_err() {
                    break; // 客户端断开
                }
            }
            bus.unsubscribe(sub.id());
            let _ = stream.flush();
        }
        ("GET", "/api/approve") => {
            // 审批应答：query 形如 ?id=...&allow=true
            let mut id = String::new();
            let mut allow = false;
            if let Some(q) = req.path.split_once('?').map(|(_, q)| q) {
                for pair in q.split('&') {
                    let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
                    let v = v.replace("%3D", "=");
                    match k {
                        "id" => id = v,
                        "allow" => allow = v == "true",
                        _ => {}
                    }
                }
            }
            if id.is_empty() {
                let _ = http::write_response(stream, 400, "text/plain; charset=utf-8", "缺少 id");
                return;
            }
            let decision = if allow {
                neo_protocol::Decision::Allow
            } else {
                neo_protocol::Decision::Deny
            };
            if ops.send(Op::Approve { id, decision }).is_err() {
                let _ = http::write_response(stream, 503, "text/plain; charset=utf-8", "内核线程已退出");
                return;
            }
            let _ = http::write_response(stream, 200, "text/plain; charset=utf-8", "ok");
        }
        ("GET", "/api/facts") => {
            // 只读诊断端点：当前订阅者数（便于确认有界性行为）
            let body = format!("{{\"subscribers\":{}}}", bus.subscriber_count());
            let _ = http::write_response(stream, 200, "application/json", &body);
        }
        _ => {
            let _ = http::write_response(stream, 404, "text/plain; charset=utf-8", "未找到");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;



    #[test]
    fn wire_format_is_flat_and_tagged() {
        // 前端契约：kind 在顶层，字段也展开在顶层
        let line = wire_event(&EventMsg::AgentMessageDelta { delta: "hi".into() });
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["kind"], "agent_message_delta");
        assert_eq!(v["delta"], "hi");
    }

    #[test]
    fn web_host_uses_the_shared_fact_extractor() {
        // 与 exec/TUI 用同一套事实语义（T6 前提）
        let mut h = WebFacts::new();
        h.consume(&EventMsg::AgentMessageDone { text: "hi".into() }).unwrap();
        h.consume(&EventMsg::ToolCallBegin { id: "c".into(), name: "bash".into(), arguments: serde_json::Value::Null }).unwrap();
        h.consume(&EventMsg::ToolCallEnd { id: "c".into(), exit_code: 0, stdout: String::new(), stderr: String::new(), truncated: false }).unwrap();
        assert_eq!(
            h.facts(),
            vec![
                Fact::AssistantSaid("hi".into()),
                Fact::ToolFinished { name: "bash".into(), exit_code: 0, stdout: String::new(), stderr: String::new(), truncated: false },
            ]
        );
    }
}
