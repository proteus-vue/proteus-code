//! unix socket 多客户端测试：两个客户端共用一个内核。
//!
//! 与 `stdio_protocol.rs` 同两条纪律：
//! 1. **不用固定等待** —— 输入一次给全、断言确定性终态；`shutdown` 走完整收尾。
//! 2. **不假定响应与通知的先后** —— 响应与事件由不同线程投递到同一连接，
//!    谁先谁后取决于调度。所以 `Client::wait_for` 按谓词过滤、未命中的行
//!    **存进 pending 不丢弃**（丢弃过一次：B 自己那个 Op 的广播事件排在响应
//!    前面，把待断言的响应顶掉了）。
//!
//! 测试判据是模块头那四条语义：事件广播、响应回发起连接、审批同看同控、
//! `shutdown` 全局收尾（断开一个连接不影响其它）。

use std::io::{BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use neo_host_appserver::{Job, JobOut};
use neo_protocol::EventMsg;
use serde_json::{json, Value};

/// 唯一临时 socket 路径（同机并行跑测试会互相删目录 —— 本仓真实踩过）。
fn unique_socket() -> PathBuf {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).expect("系统时钟").as_nanos();
    std::env::temp_dir().join(format!("neo-appserver-unix-{}-{nanos}.sock", std::process::id()))
}

/// 一个测试客户端：写一行请求、按谓词取它的输出行。
struct Client {
    stream: UnixStream,
    rx: Receiver<String>,
    /// 已收但尚未被断言消费的行（响应与通知无先后保证，不能按行号取）
    pending: Vec<Value>,
}

impl Client {
    fn connect(socket: &Path) -> Client {
        // 条件等待（有界重试，不是固定 sleep）：serve_unix 先 bind 再进 accept
        let deadline = Instant::now() + Duration::from_secs(10);
        let stream = loop {
            match UnixStream::connect(socket) {
                Ok(s) => break s,
                Err(e) if Instant::now() < deadline => {
                    let _ = e;
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => panic!("连不上 {}：{e}", socket.display()),
            }
        };
        let read_half = stream.try_clone().expect("克隆读端");
        let (tx, rx) = channel::<String>();
        std::thread::spawn(move || {
            for line in BufReader::new(read_half).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    return;
                }
            }
        });
        Client { stream, rx, pending: Vec::new() }
    }

    fn send(&mut self, line: &Value) {
        self.stream
            .write_all(line.to_string().as_bytes())
            .and_then(|()| self.stream.write_all(b"\n"))
            .and_then(|()| self.stream.flush())
            .expect("写请求行");
    }

    /// 取下一条满足谓词的行；未命中的行存进 `pending`（后续断言还能取到）。
    fn wait_for(&mut self, what: &str, pred: impl Fn(&Value) -> bool) -> Value {
        if let Some(i) = self.pending.iter().position(&pred) {
            return self.pending.remove(i);
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                panic!("等 {what} 超时；pending: {:?}", self.pending);
            }
            match self.rx.recv_timeout(left) {
                Ok(line) => {
                    let v: Value = serde_json::from_str(&line).expect("输出行必须是 JSON");
                    if pred(&v) {
                        return v;
                    }
                    self.pending.push(v);
                }
                Err(RecvTimeoutError::Timeout) => panic!("等 {what} 超时；pending: {:?}", self.pending),
                Err(RecvTimeoutError::Disconnected) => {
                    panic!("等 {what} 时连接已关闭；pending: {:?}", self.pending)
                }
            }
        }
    }

    /// 响应行（按 id 取 —— id 只在本连接内有意义）。
    fn response(&mut self, id: i64) -> Value {
        self.wait_for(&format!("id={id} 的响应"), |v| v["id"] == json!(id))
    }

    /// 事件通知（按 kind 取）。
    fn event(&mut self, kind: &str) -> Value {
        self.wait_for(&format!("{kind} 事件"), |v| {
            v["method"] == "event" && v["params"]["kind"] == json!(kind)
        })
    }

    /// 客户端走了：关掉整条连接（服务端读侧看到 EOF）。
    ///
    /// 只 drop 本结构体**不够** —— 读线程还持有套接字克隆，连接不会断。
    fn disconnect(self) {
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

fn init_line(id: i64) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": "initialize", "params": {} })
}

/// 假内核：每个 Op 回一条可辨认的事件（delta 带 Op 名），shutdown 回收尾。
fn events_for(op: &neo_protocol::Op) -> Vec<EventMsg> {
    if matches!(op, neo_protocol::Op::Shutdown) {
        return vec![EventMsg::ShutdownComplete];
    }
    vec![EventMsg::AgentMessageDelta { delta: format!("{op:?}") }]
}

/// 装配：脚本化假内核（这里测**传输语义**，thread/* 的装配在 stdio_protocol.rs 已覆盖）。
fn serve_fake(socket: &Path) -> std::io::Result<()> {
    neo_host_appserver::serve_unix(socket, |job| match job {
        Job::Op(op) => JobOut::Events(events_for(&op)),
        Job::Thread { .. } => JobOut::Thread(neo_host_appserver::ThreadResult::Error(
            "本装配未接会话库".into(),
        )),
    })
}

/// 多客户端：广播 + 响应路由 + 一条先走不影响另一条 + shutdown 全局收尾。
#[test]
fn two_clients_share_one_kernel_with_broadcast_and_own_responses() {
    let socket = unique_socket();
    let sock = socket.clone();
    let server = std::thread::spawn(move || serve_fake(&sock));

    let mut a = Client::connect(&socket);
    let mut b = Client::connect(&socket);

    // ① 各自握手（握手是**每连接**的事）
    a.send(&init_line(1));
    let ra = a.response(1);
    assert_eq!(ra["result"]["protocol_version"], json!(neo_host_appserver::PROTOCOL_VERSION));
    b.send(&init_line(10));
    let rb = b.response(10);
    assert_eq!(rb["result"]["protocol_version"], json!(neo_host_appserver::PROTOCOL_VERSION));

    // ② A 提交 → 事件广播给两条连接（B 没提交也能看到），响应回 A
    a.send(&json!({ "jsonrpc": "2.0", "id": 2, "method": "turn/pump", "params": {} }));
    let resp = a.response(2);
    assert_eq!(resp["result"]["accepted"], true, "响应回发起连接：{resp}");
    let ev_a = a.event("agent_message_delta");
    let ev_b = b.event("agent_message_delta");
    assert_eq!(ev_a["params"]["seq"], ev_b["params"]["seq"], "seq 是内核全局的，两条连接同一条事件");
    assert!(
        ev_b["params"]["payload"]["delta"].as_str().unwrap_or_default().contains("Pump"),
        "B 收到的应是 A 提交那个 Op 的事件：{ev_b}"
    );

    // ③ B 提交 → 响应回 B（id=20 只在 B 连接内有意义），A 同样看到广播
    b.send(&json!({ "jsonrpc": "2.0", "id": 20, "method": "turn/interrupt", "params": {} }));
    let resp_b = b.response(20);
    assert_eq!(resp_b["result"]["accepted"], true);
    let ev_a2 = a.event("agent_message_delta");
    assert!(
        ev_a2["params"]["payload"]["delta"].as_str().unwrap_or_default().contains("Interrupt"),
        "A 也该看到 B 那个 Op 的事件：{ev_a2}"
    );

    // ④ A 先走（EOF）：B 照常 —— 断开一个连接不影响另一个
    a.disconnect();
    b.send(&json!({ "jsonrpc": "2.0", "id": 30, "method": "turn/pump", "params": {} }));
    let resp_b2 = b.response(30);
    assert_eq!(resp_b2["result"]["accepted"], true, "A 走后 B 应照常：{resp_b2}");

    // ⑤ shutdown 全局收尾：B 的 shutdown 让服务整体收尾（ShutdownComplete 广播回来）
    b.send(&json!({ "jsonrpc": "2.0", "id": 40, "method": "shutdown", "params": {} }));
    let final_resp = b.response(40);
    assert_eq!(final_resp["result"]["accepted"], true);
    let ev = b.event("shutdown_complete");
    assert!(ev["params"]["seq"].as_u64().unwrap_or(0) >= 1, "收尾事件也带全局 seq：{ev}");

    server.join().expect("服务线程收尾").expect("serve_unix 不该失败");
    assert!(!socket.exists(), "收尾应清掉 socket 文件");
}

/// 一条连接 shutdown 时，另一条也要收到收尾事件（多看同控的收尾）。
#[test]
fn shutdown_reaches_both_clients() {
    let socket = unique_socket();
    let sock = socket.clone();
    let server = std::thread::spawn(move || serve_fake(&sock));

    let mut a = Client::connect(&socket);
    let mut b = Client::connect(&socket);
    a.send(&init_line(1));
    a.response(1);
    b.send(&init_line(10));
    b.response(10);

    // A 发 shutdown：**两条**连接都要看到 shutdown_complete
    a.send(&json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown", "params": {} }));
    a.response(2);
    let ev_a = a.event("shutdown_complete");
    let ev_b = b.event("shutdown_complete");
    assert_eq!(ev_a["params"]["seq"], ev_b["params"]["seq"], "收尾事件同一条（全局 seq）");

    server.join().expect("服务线程收尾").expect("serve_unix 不该失败");
}
