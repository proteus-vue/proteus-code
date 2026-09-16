//! Web 宿主 HTTP 层集成测试 —— 真实 TCP、真实路由、注入的假内核。
//!
//! # 为什么补这一层
//!
//! 此前 `neo-host-web` 只有单元测试，覆盖的是 HTTP **解析**与广播两件事；
//! `route()` 与五个端点（`/`、`/api/turn`、`/api/events`、`/api/approve`、
//! `/api/goal`、`/api/facts`）的实际行为**零覆盖**。而桌面宿主的 `--webview`
//! 路径复用同一条 HTTP 链路 —— 动界面之前必须先有这层网。
//!
//! # 测法：只替换模型那一侧
//!
//! `start()` 由调用方注入 `handle_op`，所以这里能起一个**真实监听**的服务，
//! 用一个记录 Op 的假内核替换真内核。于是 HTTP 解析、路由、状态码、SSE 时序、
//! `current_goal` 跟踪全部按真实路径走，只有"模型怎么回"是假的。
//!
//! 断言的是**协议契约**（提交了哪个 Op、回播了哪个事件），不是渲染字符串。

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use neo_host_web::{start, WebServer};
use neo_protocol::{ContextRef, Decision, EventMsg, GoalSnapshot, Op, RefKind};

// ───────────────────────────── 测试台 ─────────────────────────────

/// 一个真实监听的 Web 宿主 + 记录收到的 Op 的假内核。
struct Harness {
    addr: SocketAddr,
    ops: Arc<Mutex<Vec<Op>>>,
    /// 假内核要"回播"的事件（模拟内核跑完一个 Op 产生的事件流）。
    reply: Arc<Mutex<Vec<EventMsg>>>,
    /// 持有句柄，保证 Op 通道不因两侧 Sender 都被丢弃而关闭。
    _server: WebServer,
}

impl Harness {
    fn spawn() -> Self {
        let ops: Arc<Mutex<Vec<Op>>> = Arc::new(Mutex::new(Vec::new()));
        let reply: Arc<Mutex<Vec<EventMsg>>> = Arc::new(Mutex::new(Vec::new()));
        let ops_for_kernel = Arc::clone(&ops);
        let reply_for_kernel = Arc::clone(&reply);
        let (server, _kernel_thread) = start("127.0.0.1:0", move |op| {
            ops_for_kernel.lock().expect("锁中毒").push(op);
            reply_for_kernel.lock().expect("锁中毒").clone()
        })
        .expect("绑定回环端口失败");
        Self { addr: server.addr, ops, reply, _server: server }
    }

    /// 设定假内核下一次的回复。
    ///
    /// 必须在提交对应 Op **之前**调用：Op 经 channel 送达内核线程，
    /// 而 send 发生在路由返回响应之前，所以"先设后发"是有序的。
    fn set_reply(&self, events: Vec<EventMsg>) {
        *self.reply.lock().expect("锁中毒") = events;
    }

    /// 目前收到的全部 Op（按提交顺序）。
    fn received(&self) -> Vec<Op> {
        self.ops.lock().expect("锁中毒").clone()
    }
}

fn goal_snapshot(goal_id: &str) -> GoalSnapshot {
    GoalSnapshot {
        goal_id: goal_id.to_string(),
        goal: "把 X 做完".to_string(),
        paused: false,
        stopped: None,
        subtasks: Vec::new(),
        iterations: 0,
        consecutive_failures: 0,
        turns_remaining: 1,
        budget_used: 0,
    }
}

// ───────────────────────────── HTTP 客户端 ─────────────────────────────

/// 一个已解析的响应。
struct Resp {
    status: u16,
    head: String,
    body: String,
}

impl Resp {
    fn header(&self, name: &str) -> Option<String> {
        for line in self.head.lines() {
            if let Some((k, v)) = line.split_once(':') {
                if k.eq_ignore_ascii_case(name) {
                    return Some(v.trim().to_string());
                }
            }
        }
        None
    }
}

fn connect(addr: SocketAddr) -> TcpStream {
    let s = TcpStream::connect(addr).expect("连接失败");
    // 设了超时，测试卡住时是失败而不是挂起
    s.set_read_timeout(Some(Duration::from_secs(5))).expect("设读超时失败");
    s.set_write_timeout(Some(Duration::from_secs(5))).expect("设写超时失败");
    s
}

/// 发一个请求并读到 EOF（普通端点都回 `Connection: close`）。
fn request(addr: SocketAddr, raw: &str) -> Resp {
    let mut s = connect(addr);
    s.write_all(raw.as_bytes()).expect("写请求失败");
    let mut buf = String::new();
    s.read_to_string(&mut buf).expect("读响应失败");
    parse_response(&buf)
}

fn parse_response(raw: &str) -> Resp {
    let (head, body) = raw.split_once("\r\n\r\n").expect("响应缺少头/体分隔");
    let status = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse().ok())
        .expect("响应行缺少状态码");
    Resp { status, head: head.to_string(), body: body.to_string() }
}

fn get(path: &str) -> String {
    format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
}

fn post(path: &str, body: &str) -> String {
    // Content-Length 是**字节数**：正文含中文时 len() 与 chars().count() 不同
    format!(
        "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// 打开 SSE 连接，返回 (状态行, 读取器)。
///
/// 服务端在 `subscribe()` **之后**才写 `subscribed` 事件，所以读到它就
/// 说明订阅已建立 —— 后续提交的 Op 产生的事件一条都不会漏。
fn open_sse(addr: SocketAddr) -> (String, BufReader<TcpStream>) {
    let s = connect(addr);
    let mut w = s.try_clone().expect("克隆流失败");
    w.write_all(get("/api/events").as_bytes()).expect("写 SSE 请求失败");
    let mut reader = BufReader::new(s);
    let mut status = String::new();
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).expect("读 SSE 响应头失败");
        assert!(n > 0, "SSE 响应头意外结束");
        if status.is_empty() {
            status = line.trim_end().to_string();
        }
        if line == "\r\n" {
            return (status, reader);
        }
    }
}

/// 读一条 SSE 事件（多个 `data:` 行按 SSE 规范用换行拼回）。
fn read_sse_event(reader: &mut BufReader<TcpStream>) -> String {
    let mut data: Vec<String> = Vec::new();
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).expect("读 SSE 事件失败");
        assert!(n > 0, "SSE 流在事件中途结束");
        let t = line.trim_end_matches(['\r', '\n']);
        if t.is_empty() {
            return data.join("\n");
        }
        if let Some(p) = t.strip_prefix("data: ") {
            data.push(p.to_string());
        }
    }
}

/// 订阅者数（走真实端点，验证 `/api/facts` 本身）。
fn subscriber_count(addr: SocketAddr) -> usize {
    let r = request(addr, &get("/api/facts"));
    assert_eq!(r.status, 200, "/api/facts 应可用");
    let v: serde_json::Value = serde_json::from_str(&r.body).expect("/api/facts 应返回 JSON");
    v["subscribers"].as_u64().expect("subscribers 应是数字") as usize
}

// ───────────────────────────── 静态页面 ─────────────────────────────

#[test]
fn serves_the_builtin_page_at_root_and_index() {
    let h = Harness::spawn();
    for path in ["/", "/index.html"] {
        let r = request(h.addr, &get(path));
        assert_eq!(r.status, 200, "{path} 应返回内置页面");
        assert_eq!(
            r.header("content-type").as_deref(),
            Some("text/html; charset=utf-8"),
            "{path} 的 Content-Type 不符"
        );
        assert!(r.body.starts_with("<!doctype html>"), "{path} 的正文不是内置 HTML");
    }
}

#[test]
fn unknown_path_and_wrong_method_are_404() {
    let h = Harness::spawn();
    assert_eq!(request(h.addr, &get("/nope")).status, 404);
    // 路由按 (方法, 路径) 精确匹配：错误方法不会落到同一个处理函数
    assert_eq!(request(h.addr, &get("/api/turn")).status, 404);
    assert_eq!(request(h.addr, &post("/api/events", "x")).status, 404);
}

// ───────────────────────────── /api/turn ─────────────────────────────

#[test]
fn turn_submits_userturn_with_refs_parsed_by_the_protocol_layer() {
    let h = Harness::spawn();
    let text = "看下 @src/main.rs 和 $skill-x";
    assert_eq!(request(h.addr, &post("/api/turn", text)).status, 200);

    let ops = h.received();
    assert_eq!(ops.len(), 1, "应恰好提交一个 Op");
    match &ops[0] {
        Op::UserTurn { text: got, refs } => {
            assert_eq!(got, text);
            assert_eq!(
                refs,
                &vec![
                    ContextRef { kind: RefKind::File, target: "src/main.rs".into(), lines: None },
                    ContextRef { kind: RefKind::Skill, target: "skill-x".into(), lines: None },
                ],
                "宿主必须复用协议层的引用解析，而不是自造一套语义"
            );
        }
        other => panic!("期望 UserTurn，实际 {other:?}"),
    }
}

#[test]
fn turn_rejects_a_blank_task() {
    let h = Harness::spawn();
    assert_eq!(request(h.addr, &post("/api/turn", "   \n  ")).status, 400);
    assert!(h.received().is_empty(), "空任务不应提交任何 Op");
}

// ───────────────────────────── /api/events（SSE）─────────────────────────────

#[test]
fn sse_streams_subscribed_then_kernel_events_in_wire_format() {
    let h = Harness::spawn();
    let (status, mut reader) = open_sse(h.addr);
    assert!(status.starts_with("HTTP/1.1 200"), "SSE 状态行异常：{status}");

    // 先收到订阅确认，客户端据此知道流已建立
    assert_eq!(read_sse_event(&mut reader), r#"{"kind":"subscribed"}"#);

    h.set_reply(vec![
        EventMsg::TurnStarted { turn_id: "t1".into() },
        EventMsg::AgentMessageDelta { delta: "你".into() },
    ]);
    assert_eq!(request(h.addr, &post("/api/turn", "hi")).status, 200);

    let first: serde_json::Value =
        serde_json::from_str(&read_sse_event(&mut reader)).expect("事件应是 JSON");
    assert_eq!(first["kind"], "turn_started");

    let second: serde_json::Value =
        serde_json::from_str(&read_sse_event(&mut reader)).expect("事件应是 JSON");
    assert_eq!(second["kind"], "agent_message_delta");
    assert_eq!(
        second["delta"], "你",
        "字段必须展开在顶层，前端才不用写 m[k].delta 这种别扭取值"
    );
    // 多字节正文在 SSE 里必须按字节原样过，不能截断
    assert!(second.to_string().contains('你'));
}

#[test]
fn sse_fans_out_to_every_subscriber() {
    let h = Harness::spawn();
    let (_, mut a) = open_sse(h.addr);
    let (_, mut b) = open_sse(h.addr);
    assert_eq!(read_sse_event(&mut a), r#"{"kind":"subscribed"}"#);
    assert_eq!(read_sse_event(&mut b), r#"{"kind":"subscribed"}"#);
    assert_eq!(subscriber_count(h.addr), 2, "两个标签页 = 两个订阅者");

    h.set_reply(vec![EventMsg::AgentMessageDone { text: "hi".into() }]);
    assert_eq!(request(h.addr, &post("/api/turn", "hi")).status, 200);

    // 同一事件必须抵达两个客户端（多标签页场景）
    for (who, reader) in [("a", &mut a), ("b", &mut b)] {
        let ev: serde_json::Value =
            serde_json::from_str(&read_sse_event(reader)).expect("事件应是 JSON");
        assert_eq!(ev["kind"], "agent_message_done", "订阅者 {who} 未收到事件");
        assert_eq!(ev["text"], "hi");
    }
}

/// 客户端断开后，订阅槽位与连接线程必须**立即**归还 —— 不需要等下一条事件。
///
/// 这是回归测试。曾经的实现只在"写事件失败"时才发现断开，于是空闲期间
/// 断开的连接会一直占着槽位与线程；并发上限 64，攒满 64 个这样的连接，
/// 新连接只会收到 503。修复是事件驱动的断开探测（服务端读侧返回 0），
/// 不发心跳、不改线格式。这个用例刻意**不发任何事件**：事件驱动的那种
/// 实现会在这里失败。
#[test]
fn a_disconnected_client_releases_its_slot_without_needing_an_event() {
    let h = Harness::spawn();
    let (_, reader) = open_sse(h.addr);
    assert_eq!(subscriber_count(h.addr), 1, "连接后应有 1 个订阅者");

    drop(reader); // 客户端断开，且此后没有任何事件流过

    // 有界轮询（每轮一次真实请求往返，不盲等）：断开探测是异步的，
    // 给它若干轮机会；探测本身是事件驱动的，通常第一轮就已回收。
    let mut reclaimed = false;
    for _ in 0..50 {
        if subscriber_count(h.addr) == 0 {
            reclaimed = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(5)); // 有界等待，不是盲等
    }
    assert!(
        reclaimed,
        "客户端断开后槽位应立即回收，而不是等到下一条事件（否则空闲断开连接会耗尽并发预算）"
    );
}

/// 探测不得误伤**健康**的空闲连接：断开必须被准确区分于"暂时没话说"。
///
/// 服务端读侧被清掉了读超时，所以健康连接在长时间无事件时不应被踢。
/// 这里换个角度验证：断开一个订阅者，另一个必须原样健在并能继续收事件。
#[test]
fn probing_does_not_disturb_the_other_live_subscribers() {
    let h = Harness::spawn();
    let (_, mut gone) = open_sse(h.addr);
    let (_, mut alive) = open_sse(h.addr);
    assert_eq!(read_sse_event(&mut gone), r#"{"kind":"subscribed"}"#);
    assert_eq!(read_sse_event(&mut alive), r#"{"kind":"subscribed"}"#);
    assert_eq!(subscriber_count(h.addr), 2);

    drop(gone);
    let mut narrowed = false;
    for _ in 0..50 {
        if subscriber_count(h.addr) == 1 {
            narrowed = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(narrowed, "断开者应被摘除，且只摘除它自己");

    // 存活者照常收到事件 —— 探测没有把它一起摘掉
    h.set_reply(vec![EventMsg::AgentMessageDone { text: "还活着".into() }]);
    assert_eq!(request(h.addr, &post("/api/turn", "hi")).status, 200);
    let ev: serde_json::Value =
        serde_json::from_str(&read_sse_event(&mut alive)).expect("事件应是 JSON");
    assert_eq!(ev["kind"], "agent_message_done");
    assert_eq!(ev["text"], "还活着");
}

// ───────────────────────────── /api/approve ─────────────────────────────

#[test]
fn approve_maps_the_query_to_a_decision() {
    let h = Harness::spawn();
    assert_eq!(request(h.addr, &get("/api/approve?id=a1&allow=true")).status, 200);
    assert_eq!(request(h.addr, &get("/api/approve?id=a2&allow=false")).status, 200);

    assert_eq!(
        h.received(),
        vec![
            Op::Approve { id: "a1".into(), decision: Decision::Allow, reason: None },
            Op::Approve { id: "a2".into(), decision: Decision::Deny, reason: None },
        ],
        "allow 只认字面量 true；其余一律按拒绝处理"
    );
}

#[test]
fn approve_unescapes_percent_encoded_ids() {
    // 审批 id 可能含 `=`，前端按 query 规则编码成 %3D
    let h = Harness::spawn();
    assert_eq!(request(h.addr, &get("/api/approve?id=a%3Db&allow=true")).status, 200);
    assert_eq!(
        h.received(),
        vec![Op::Approve { id: "a=b".into(), decision: Decision::Allow, reason: None }],
        "id 必须先解码再提交，否则回不到那个挂起的审批"
    );
}

#[test]
fn approve_without_an_id_is_rejected() {
    let h = Harness::spawn();
    assert_eq!(request(h.addr, &get("/api/approve?allow=true")).status, 400);
    assert!(h.received().is_empty(), "缺 id 不应提交审批 Op");
}

// ───────────────────────────── /api/goal ─────────────────────────────

#[test]
fn goal_set_submits_the_goal_text() {
    let h = Harness::spawn();
    let body = "重构 A\n重构 B";
    assert_eq!(request(h.addr, &post("/api/goal", body)).status, 200);
    assert_eq!(
        h.received(),
        vec![Op::GoalSet { goal: body.into() }],
        "多行正文 = 多子任务，必须原样送达"
    );
}

#[test]
fn goal_set_rejects_a_blank_goal() {
    let h = Harness::spawn();
    assert_eq!(request(h.addr, &post("/api/goal", "  ")).status, 400);
    assert!(h.received().is_empty());
}

#[test]
fn goal_actions_map_to_ops_and_unknown_actions_are_rejected() {
    let h = Harness::spawn();
    assert_eq!(request(h.addr, &get("/api/goal?action=advance")).status, 200);
    assert_eq!(request(h.addr, &get("/api/goal?action=clear")).status, 200);
    assert_eq!(request(h.addr, &get("/api/goal?action=bogus")).status, 400);
    assert_eq!(request(h.addr, &get("/api/goal")).status, 400, "缺 action 也是 400");

    assert_eq!(
        h.received(),
        vec![Op::GoalAdvance, Op::GoalClear],
        "非法 action 必须被拒，不能落到内核"
    );
}

#[test]
fn goal_pause_without_an_active_goal_is_409() {
    let h = Harness::spawn();
    let r = request(h.addr, &get("/api/goal?action=pause"));
    assert_eq!(r.status, 409, "没有活动目标时应如实告知，而不是提交一个必然报错的 Op");
    assert!(h.received().is_empty());
}

/// pause / resume 要作用于**当前**目标，而目标 id 只存在于事件流里。
///
/// 这条覆盖的是路由层唯一的跨请求状态：`current_goal` 由内核线程在广播
/// **之前**更新（`lib.rs` 顺序：先更新共享状态，再 publish）。因此
/// "读到 `goal_updated` 事件"就是"跟踪状态已就位"的同步点 —— 不靠 sleep。
#[test]
fn pause_and_resume_target_the_goal_from_the_latest_snapshot() {
    let h = Harness::spawn();
    let (_, mut reader) = open_sse(h.addr);
    assert_eq!(read_sse_event(&mut reader), r#"{"kind":"subscribed"}"#);

    h.set_reply(vec![EventMsg::GoalUpdated { snapshot: goal_snapshot("goal-7") }]);
    assert_eq!(request(h.addr, &post("/api/turn", "起个目标")).status, 200);
    let ev: serde_json::Value =
        serde_json::from_str(&read_sse_event(&mut reader)).expect("事件应是 JSON");
    assert_eq!(ev["kind"], "goal_updated", "同步点：跟踪状态此刻已就位");

    // 后续 Op 不产生事件，事件流保持可预测
    h.set_reply(vec![]);
    assert_eq!(request(h.addr, &get("/api/goal?action=pause")).status, 200);
    assert_eq!(request(h.addr, &get("/api/goal?action=resume")).status, 200);
    assert_eq!(
        h.received(),
        vec![
            Op::UserTurn { text: "起个目标".into(), refs: vec![] },
            Op::GoalPause { goal_id: "goal-7".into() },
            Op::GoalResume { goal_id: "goal-7".into() },
        ],
        "pause/resume 必须落在最新快照的目标 id 上，而不是猜一个"
    );

    // 清除目标后不再有活动目标，pause 应退回 409
    h.set_reply(vec![EventMsg::GoalCleared { goal_id: "goal-7".into() }]);
    assert_eq!(request(h.addr, &post("/api/turn", "清掉")).status, 200);
    let ev: serde_json::Value =
        serde_json::from_str(&read_sse_event(&mut reader)).expect("事件应是 JSON");
    assert_eq!(ev["kind"], "goal_cleared");
    assert_eq!(request(h.addr, &get("/api/goal?action=pause")).status, 409);
}
