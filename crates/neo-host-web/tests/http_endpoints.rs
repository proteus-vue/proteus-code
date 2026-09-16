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
    /// 访问令牌。带令牌的请求走 `get`/`post`/`open_sse`；
    /// 需要故意不带令牌的负向用例走 `raw`。
    token: String,
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
        let token = server.token().to_string();
        Self { addr: server.addr, token, ops, reply, _server: server }
    }

    /// 带令牌的 GET（令牌走 query —— 浏览器侧的通用方式）。
    fn get(&self, path: &str) -> Resp {
        request(self.addr, &get_raw(&self.with_token(path)))
    }

    /// 带令牌的 GET，令牌走请求头（程序化客户端的方式）。
    fn get_with_token_header(&self, path: &str) -> Resp {
        request(
            self.addr,
            &format!(
                "GET {path} HTTP/1.1\r\nHost: localhost\r\nX-Neo-Token: {}\r\nConnection: close\r\n\r\n",
                self.token
            ),
        )
    }

    /// 带令牌的 POST。
    fn post(&self, path: &str, body: &str) -> Resp {
        request(self.addr, &post_raw(&self.with_token(path), body))
    }

    /// 打开带令牌的 SSE 连接。
    fn open_sse(&self) -> (String, BufReader<TcpStream>) {
        open_sse(self.addr, &self.with_token("/api/events"))
    }

    /// 把令牌接到路径上（无 query 用 `?`，有则用 `&`）。
    fn with_token(&self, path: &str) -> String {
        let sep = if path.contains('?') { '&' } else { '?' };
        format!("{path}{sep}token={}", self.token)
    }

    /// 发一个**原样**请求（用于故意不带/带错令牌的负向用例）。
    fn raw(&self, raw: &str) -> Resp {
        request(self.addr, raw)
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

/// 构造一个 GET 请求行（`path` 已含 query）。
fn get_raw(path: &str) -> String {
    format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
}

/// 构造一个 POST 请求行（`path` 已含 query）。
fn post_raw(path: &str, body: &str) -> String {
    // Content-Length 是**字节数**：正文含中文时 len() 与 chars().count() 不同
    format!(
        "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
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

/// 打开 SSE 连接，返回 (状态行, 读取器)。`path` 已含令牌。
///
/// 服务端在 `subscribe()` **之后**才写 `subscribed` 事件，所以读到它就
/// 说明订阅已建立 —— 后续提交的 Op 产生的事件一条都不会漏。
fn open_sse(addr: SocketAddr, path: &str) -> (String, BufReader<TcpStream>) {
    let s = connect(addr);
    let mut w = s.try_clone().expect("克隆流失败");
    w.write_all(get_raw(path).as_bytes()).expect("写 SSE 请求失败");
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
fn subscriber_count(h: &Harness) -> usize {
    let r = h.get("/api/facts");
    assert_eq!(r.status, 200, "/api/facts 应可用");
    let v: serde_json::Value = serde_json::from_str(&r.body).expect("/api/facts 应返回 JSON");
    v["subscribers"].as_u64().expect("subscribers 应是数字") as usize
}

// ───────────────────────────── 鉴权 ─────────────────────────────
//
// 这一节是**安全契约**，不是"顺手测一下"。只绑回环不构成访问控制：
// 本机任意进程都能枚举端口，浏览器里的任意网页也能跨源 POST（端点是
// 简单请求、不触发预检，且**请求会生效**）。令牌是唯一的门槛。

/// 宿主必须把 `page_url()`（带令牌）交给用户，而不是裸地址。
///
/// 桌面宿主把这个 URL 直接喂给 webview —— 它错了，窗口打开就是一张 401 页。
#[test]
fn page_url_carries_the_token_in_the_fragment() {
    let h = Harness::spawn();
    let url = h._server.page_url();
    assert!(url.starts_with("http://"), "应是可直接打开的 URL：{url}");
    assert!(
        url.ends_with(&format!("#token={}", h.token)),
        "令牌必须在 fragment 里（不发给服务器、不进 Referer）：{url}"
    );
    assert!(!url.contains("?token="), "令牌不能在 query 里：{url}");
    // 显式路径：页面用相对路径发请求，空路径 base 的相对解析依赖 merge 规则
    assert!(url.contains(&format!(":{}/", h.addr.port())), "路径应显式为 /：{url}");
}

#[test]
fn the_page_itself_needs_no_token_so_the_browser_can_load_it() {
    let h = Harness::spawn();
    // 页面是静态的、无秘密，且必须能被浏览器直接加载（令牌在 fragment 里，
    // 页面自己去取）。所以裸地址取页面必须成功。
    let r = h.raw(&get_raw("/"));
    assert_eq!(r.status, 200, "内置页面不应要求令牌");
    assert!(r.body.contains("<!doctype html>"));
}

#[test]
fn every_api_endpoint_rejects_a_request_without_a_token() {
    let h = Harness::spawn();
    // 逐个端点验证 —— 漏掉任何一个，那个端点就是敞开的写入口
    for (what, r) in [
        ("POST /api/turn", h.raw(&post_raw("/api/turn", "rm -rf /"))),
        ("GET /api/events", h.raw(&get_raw("/api/events"))),
        ("GET /api/approve", h.raw(&get_raw("/api/approve?id=a1&allow=true"))),
        ("GET /api/facts", h.raw(&get_raw("/api/facts"))),
        ("POST /api/goal", h.raw(&post_raw("/api/goal", "目标"))),
        ("GET /api/goal", h.raw(&get_raw("/api/goal?action=advance"))),
    ] {
        assert_eq!(r.status, 401, "{what} 未带令牌时必须 401");
    }
    // 关键：拒绝必须发生在"送达内核"之前，而不是响应层的事后拦截
    assert!(h.received().is_empty(), "未鉴权请求绝不能产生任何 Op");
}

#[test]
fn an_unknown_path_is_indistinguishable_when_unauthenticated() {
    let h = Harness::spawn();
    // 不存在的路径也返回 401 而不是 404：未鉴权调用者连"哪些路由存在"
    // 都问不出来（否则 404/401 的差别就是一个路由枚举器）。
    assert_eq!(h.raw(&get_raw("/definitely-not-a-route")).status, 401);
    assert_eq!(h.raw(&post_raw("/definitely-not-a-route", "x")).status, 401);
}

#[test]
fn a_wrong_token_is_rejected_like_no_token_at_all() {
    let h = Harness::spawn();

    // 「只差最后一位」的构造必须真的差一位：
    //   format!("{}0", &token[..63]) 这种写法在 token 末位本就是 '0' 时
    //   **恰好等于真令牌**（1/16 概率），于是这条负向用例会随机变绿或变红 ——
    //   一个 6% 概率的 flaky 门禁。末位改成与真实末位不同的字符才成立。
    let last = h.token.chars().next_back().expect("令牌非空");
    let flipped = if last == '0' { '1' } else { '0' };
    let off_by_one_last = format!("{}{}", &h.token[..63], flipped);

    for bogus in [
        "0".repeat(64),      // 长度对但值错
        h.token[..63].to_string(), // 前缀正确、长度差一
        off_by_one_last,     // 长度对、只差最后一位（且确定不同）
        String::new(),       // 空
        "short".to_string(), // 长度不对
    ] {
        // 自检：构造出的"错令牌"绝不能等于真令牌，否则下面的断言毫无意义
        assert_ne!(bogus, h.token, "负向用例构造出了真令牌，测试本身失效");
        let r = h.raw(&get_raw(&format!("/api/facts?token={bogus}")));
        assert_eq!(r.status, 401, "令牌 {bogus:?} 不应被接受");
    }
    assert!(h.received().is_empty());
}

#[test]
fn the_token_is_accepted_via_query_and_via_header() {
    let h = Harness::spawn();
    assert_eq!(h.get("/api/facts").status, 200, "query 形式（浏览器侧通用）");
    assert_eq!(
        h.get_with_token_header("/api/facts").status,
        200,
        "X-Neo-Token 头形式（程序化客户端）"
    );
}

/// 令牌必须**自动**接在页面的每个请求上，且放在 fragment 而非 query。
///
/// 这是给前端接线上的锁：若有人把 `apiPath()` 从某个 fetch 上摘掉，或者
/// 把令牌从 fragment 挪到 query，页面会在真实点击时静默 401 —— 那属于
/// "只在浏览器里才暴露"的一类 bug。这里用源码断言把它钉在 CI 上。
#[test]
fn the_builtin_page_plumbs_the_token_onto_every_request() {
    let html = neo_host_web::page::INDEX_HTML;
    assert!(
        html.contains("new URLSearchParams(location.hash.slice(1))"),
        "令牌必须来自 URL fragment（不发给服务器、不进 Referer）"
    );
    assert!(
        !html.contains("location.search"),
        "不能从 query 取令牌 —— 那会把令牌发到服务端与 Referer"
    );
    for call in [
        "fetch(apiPath('./api/goal?action=advance'))",
        "fetch(apiPath('./api/goal?action=' + action))",
        "fetch(apiPath('./api/goal'), { method: 'POST', body: text })",
        "fetch(apiPath('./api/turn'), { method: 'POST', body: text })",
        "new EventSource(apiPath('./api/events'))",
    ] {
        assert!(html.contains(call), "页面请求未接令牌：{call}");
    }
    // 审批那一条是拼接的，单独查
    assert!(
        html.contains("fetch(apiPath('./api/approve?id='"),
        "审批请求未接令牌"
    );
    // EventSource 不能设自定义头 —— 这正是选 query 而非头的原因
    assert!(
        !html.contains("new EventSource('./api/events')"),
        "SSE 必须走 apiPath（EventSource 设不了请求头）"
    );
}

// ───────────────────────────── 静态页面 ─────────────────────────────

#[test]
fn serves_the_builtin_page_at_root_and_index() {
    let h = Harness::spawn();
    for path in ["/", "/index.html"] {
        let r = h.get(path);
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
    assert_eq!(h.get("/nope").status, 404);
    // 路由按 (方法, 路径) 精确匹配：错误方法不会落到同一个处理函数
    assert_eq!(h.get("/api/turn").status, 404);
    assert_eq!(h.post("/api/events", "x").status, 404);
}

// ───────────────────────────── /api/turn ─────────────────────────────

#[test]
fn turn_submits_userturn_with_refs_parsed_by_the_protocol_layer() {
    let h = Harness::spawn();
    let text = "看下 @src/main.rs 和 $skill-x";
    assert_eq!(h.post("/api/turn", text).status, 200);

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
    assert_eq!(h.post("/api/turn", "   \n  ").status, 400);
    assert!(h.received().is_empty(), "空任务不应提交任何 Op");
}

// ───────────────────────────── /api/events（SSE）─────────────────────────────

#[test]
fn sse_streams_subscribed_then_kernel_events_in_wire_format() {
    let h = Harness::spawn();
    let (status, mut reader) = h.open_sse();
    assert!(status.starts_with("HTTP/1.1 200"), "SSE 状态行异常：{status}");

    // 先收到订阅确认，客户端据此知道流已建立
    assert_eq!(read_sse_event(&mut reader), r#"{"kind":"subscribed"}"#);

    h.set_reply(vec![
        EventMsg::TurnStarted { turn_id: "t1".into() },
        EventMsg::AgentMessageDelta { delta: "你".into() },
    ]);
    assert_eq!(h.post("/api/turn", "hi").status, 200);

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
    let (_, mut a) = h.open_sse();
    let (_, mut b) = h.open_sse();
    assert_eq!(read_sse_event(&mut a), r#"{"kind":"subscribed"}"#);
    assert_eq!(read_sse_event(&mut b), r#"{"kind":"subscribed"}"#);
    assert_eq!(subscriber_count(&h), 2, "两个标签页 = 两个订阅者");

    h.set_reply(vec![EventMsg::AgentMessageDone { text: "hi".into() }]);
    assert_eq!(h.post("/api/turn", "hi").status, 200);

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
    let (_, reader) = h.open_sse();
    assert_eq!(subscriber_count(&h), 1, "连接后应有 1 个订阅者");

    drop(reader); // 客户端断开，且此后没有任何事件流过

    // 有界轮询（每轮一次真实请求往返，不盲等）：断开探测是异步的，
    // 给它若干轮机会；探测本身是事件驱动的，通常第一轮就已回收。
    let mut reclaimed = false;
    for _ in 0..50 {
        if subscriber_count(&h) == 0 {
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
    let (_, mut gone) = h.open_sse();
    let (_, mut alive) = h.open_sse();
    assert_eq!(read_sse_event(&mut gone), r#"{"kind":"subscribed"}"#);
    assert_eq!(read_sse_event(&mut alive), r#"{"kind":"subscribed"}"#);
    assert_eq!(subscriber_count(&h), 2);

    drop(gone);
    let mut narrowed = false;
    for _ in 0..50 {
        if subscriber_count(&h) == 1 {
            narrowed = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(narrowed, "断开者应被摘除，且只摘除它自己");

    // 存活者照常收到事件 —— 探测没有把它一起摘掉
    h.set_reply(vec![EventMsg::AgentMessageDone { text: "还活着".into() }]);
    assert_eq!(h.post("/api/turn", "hi").status, 200);
    let ev: serde_json::Value =
        serde_json::from_str(&read_sse_event(&mut alive)).expect("事件应是 JSON");
    assert_eq!(ev["kind"], "agent_message_done");
    assert_eq!(ev["text"], "还活着");
}

// ───────────────────────────── /api/approve ─────────────────────────────

#[test]
fn approve_maps_the_query_to_a_decision() {
    let h = Harness::spawn();
    assert_eq!(h.get("/api/approve?id=a1&allow=true").status, 200);
    assert_eq!(h.get("/api/approve?id=a2&allow=false").status, 200);

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
    assert_eq!(h.get("/api/approve?id=a%3Db&allow=true").status, 200);
    assert_eq!(
        h.received(),
        vec![Op::Approve { id: "a=b".into(), decision: Decision::Allow, reason: None }],
        "id 必须先解码再提交，否则回不到那个挂起的审批"
    );
}

#[test]
fn approve_without_an_id_is_rejected() {
    let h = Harness::spawn();
    assert_eq!(h.get("/api/approve?allow=true").status, 400);
    assert!(h.received().is_empty(), "缺 id 不应提交审批 Op");
}

// ───────────────────────────── /api/goal ─────────────────────────────

#[test]
fn goal_set_submits_the_goal_text() {
    let h = Harness::spawn();
    let body = "重构 A\n重构 B";
    assert_eq!(h.post("/api/goal", body).status, 200);
    assert_eq!(
        h.received(),
        vec![Op::GoalSet { goal: body.into() }],
        "多行正文 = 多子任务，必须原样送达"
    );
}

#[test]
fn goal_set_rejects_a_blank_goal() {
    let h = Harness::spawn();
    assert_eq!(h.post("/api/goal", "  ").status, 400);
    assert!(h.received().is_empty());
}

#[test]
fn goal_actions_map_to_ops_and_unknown_actions_are_rejected() {
    let h = Harness::spawn();
    assert_eq!(h.get("/api/goal?action=advance").status, 200);
    assert_eq!(h.get("/api/goal?action=clear").status, 200);
    assert_eq!(h.get("/api/goal?action=bogus").status, 400);
    assert_eq!(h.get("/api/goal").status, 400, "缺 action 也是 400");

    assert_eq!(
        h.received(),
        vec![Op::GoalAdvance, Op::GoalClear],
        "非法 action 必须被拒，不能落到内核"
    );
}

#[test]
fn goal_pause_without_an_active_goal_is_409() {
    let h = Harness::spawn();
    let r = h.get("/api/goal?action=pause");
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
    let (_, mut reader) = h.open_sse();
    assert_eq!(read_sse_event(&mut reader), r#"{"kind":"subscribed"}"#);

    h.set_reply(vec![EventMsg::GoalUpdated { snapshot: goal_snapshot("goal-7") }]);
    assert_eq!(h.post("/api/turn", "起个目标").status, 200);
    let ev: serde_json::Value =
        serde_json::from_str(&read_sse_event(&mut reader)).expect("事件应是 JSON");
    assert_eq!(ev["kind"], "goal_updated", "同步点：跟踪状态此刻已就位");

    // 后续 Op 不产生事件，事件流保持可预测
    h.set_reply(vec![]);
    assert_eq!(h.get("/api/goal?action=pause").status, 200);
    assert_eq!(h.get("/api/goal?action=resume").status, 200);
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
    assert_eq!(h.post("/api/turn", "清掉").status, 200);
    let ev: serde_json::Value =
        serde_json::from_str(&read_sse_event(&mut reader)).expect("事件应是 JSON");
    assert_eq!(ev["kind"], "goal_cleared");
    assert_eq!(h.get("/api/goal?action=pause").status, 409);
}
