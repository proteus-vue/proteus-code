//! stdio 协议的全流程测试：**真传输**（请求行进 `Cursor`，输出行进内存缓冲）、
//! 脚本化的假内核。
//!
//! 两条纪律（与 `neo-host-web/tests/http_endpoints.rs` 一致）：
//! 1. **不用固定等待**：输入一次性给全（含 EOF 或 `shutdown`），`serve` 返回即
//!    "内核线程与写线程都已收尾"，断言的都是确定性的最终结果。
//! 2. **不假定响应与通知的先后**：两者由两个生产者写进同一条通道，
//!    谁先谁后取决于调度（内核慢时响应先到，快时通知先到）。所以断言按集合过滤，
//!    不按行号取 —— 按行号取在真机上必然偶发失败。

use std::io::{Cursor, Write};
use std::sync::{Arc, Mutex};

use neo_host_appserver::{jsonrpc, serve, PROTOCOL_VERSION};
use neo_protocol::{Decision, EventMsg, ExecMode, Op, SessionPatch};
use serde_json::{json, Value};

/// 写端：只累积到内存，供测试逐行解析。
#[derive(Clone, Default)]
struct SharedBuf(Arc<Mutex<Vec<u8>>>);

impl Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("锁中毒").extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// 一次会话的结果：全部输出行 + 内核实际收到的 Op。
struct Session {
    lines: Vec<Value>,
    ops: Vec<Op>,
}

impl Session {
    /// 跑一次会话：`reply_for` 决定"某个 Op 会产生哪些事件"（假内核脚本）。
    fn run<F>(input: &[Value], reply_for: F) -> Session
    where
        F: Fn(&Op) -> Vec<EventMsg> + Send + 'static,
    {
        let text = input.iter().map(Value::to_string).collect::<Vec<_>>().join("\n") + "\n";
        let ops = Arc::new(Mutex::new(Vec::new()));
        let sink = SharedBuf::default();
        let kernel_side = ops.clone();
        serve(Cursor::new(text.into_bytes()), sink.clone(), move |op: Op| {
            let events = reply_for(&op);
            kernel_side.lock().expect("锁中毒").push(op);
            events
        })
        .expect("serve 不该失败");

        let raw = String::from_utf8(sink.0.lock().expect("锁中毒").clone()).expect("输出必须是 UTF-8");
        let lines = raw
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).expect("每一行都必须是合法 JSON"))
            .collect();
        // 先取出 Vec 再构造（不能让 MutexGuard 活到块尾：那样 Arc 会比守卫先析构）
        let recorded = ops.lock().expect("锁中毒").clone();
        Session { lines, ops: recorded }
    }

    /// 响应（带 id 的行）。
    fn responses(&self) -> Vec<&Value> {
        self.lines.iter().filter(|l| l.get("id").is_some()).collect()
    }

    /// 事件通知（`method == "event"` 的行）。
    fn events(&self) -> Vec<&Value> {
        self.lines
            .iter()
            .filter(|l| l.get("method").and_then(Value::as_str) == Some("event"))
            .collect()
    }

    fn response_to(&self, id: i64) -> &Value {
        self.responses()
            .into_iter()
            .find(|l| l["id"] == json!(id))
            .unwrap_or_else(|| panic!("没有 id={id} 的响应：{:?}", self.lines))
    }
}

/// 事件批：每次收到 Op 都回这同一批（除了 `Shutdown` —— 内核要回 `ShutdownComplete`）。
fn stream_for(op: &Op) -> Vec<EventMsg> {
    if matches!(op, Op::Shutdown) {
        return vec![EventMsg::ShutdownComplete];
    }
    vec![
        EventMsg::TurnStarted { turn_id: "t1".into() },
        EventMsg::AgentMessageDelta { delta: "你".into() },
    ]
}

fn init_line(id: i64) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": "initialize", "params": {} })
}

fn op_line(id: i64, method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

#[test]
fn handshake_reports_version_methods_and_host_capabilities() {
    let s = Session::run(&[init_line(1)], |_| Vec::new());
    let r = s.response_to(1);
    assert_eq!(r["jsonrpc"], "2.0");
    assert_eq!(r["result"]["protocol_version"], PROTOCOL_VERSION);
    assert_eq!(r["result"]["server"]["name"], "neo-app-server");
    // 方法表是契约的一部分：客户端据此知道内核能干什么
    let methods = r["result"]["methods"].as_array().expect("methods 必须是数组");
    assert_eq!(methods.len(), 18, "initialize + 17 个 Op 方法");
    assert!(methods.iter().any(|m| m == "turn/start"));
    assert!(methods.iter().any(|m| m == "turn/interrupt"), "中断必须在线上可达");
    // 宿主能力（SPI 的既有数据，不是这条协议新造的）
    assert_eq!(r["result"]["host"]["id"], "app-server");
    assert_eq!(r["result"]["host"]["capabilities"]["interactive_prompt"], true);
}

#[test]
fn requests_before_handshake_are_refused_with_a_machine_readable_code() {
    let s = Session::run(&[op_line(1, "turn/start", json!({ "text": "hi" }))], stream_for);
    let e = &s.response_to(1)["error"];
    assert_eq!(e["code"], jsonrpc::NOT_INITIALIZED);
    assert_eq!(e["data"]["expected"], "initialize");
    assert!(s.ops.is_empty(), "未握手时不得把 Op 下发内核");
    assert!(s.events().is_empty(), "更不该产生事件");
}

#[test]
fn version_mismatch_is_rejected_and_the_connection_stays_uninitialized() {
    let bad = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
                      "params": { "protocol_version": PROTOCOL_VERSION + 1 } });
    // 失败后仍可用正确版本重来：错误不是终态
    let s = Session::run(
        &[bad, init_line(2), op_line(3, "turn/pump", json!({}))],
        stream_for,
    );
    let e = &s.response_to(1)["error"];
    assert_eq!(e["code"], jsonrpc::VERSION_MISMATCH);
    assert_eq!(e["data"]["server"], PROTOCOL_VERSION);
    assert!(s.response_to(2)["result"].is_object(), "改正版本后应能握手成功");
    assert_eq!(s.ops, vec![Op::Pump], "握手成功后 Op 才下发");
}

#[test]
fn every_op_method_reaches_the_kernel_as_its_own_op() {
    // 方法表 → Op 的**全量**对照：17 个方法一个不漏。
    // 这条测试是"新增 Op 却忘了映射"的哨兵（jsonrpc::OP_METHODS 的长度另有单测）。
    let cases: Vec<(&str, Value, Op)> = vec![
        ("turn/start", json!({ "text": "hi" }), Op::UserTurn { text: "hi".into(), refs: vec![] }),
        ("turn/begin", json!({ "text": "hi" }), Op::BeginTurn { text: "hi".into(), refs: vec![] }),
        ("turn/pump", json!({}), Op::Pump),
        ("turn/interrupt", json!({}), Op::Interrupt),
        ("command/exec", json!({ "command": "ls -la" }), Op::Shell { command: "ls -la".into() }),
        (
            "approval/respond",
            json!({ "id": "ap-1", "decision": "allow" }),
            Op::Approve { id: "ap-1".into(), decision: Decision::Allow, reason: None },
        ),
        (
            "approval/respondStep",
            json!({ "id": "ap-2", "decision": "allow_always" }),
            Op::ApproveStep { id: "ap-2".into(), decision: Decision::AllowAlways, reason: None },
        ),
        (
            "session/configure",
            json!({ "exec_mode": "plan", "model": "deepseek" }),
            Op::ConfigureSession {
                patch: SessionPatch {
                    exec_mode: Some(ExecMode::Plan),
                    model: Some("deepseek".into()),
                    ..Default::default()
                },
            },
        ),
        ("session/compact", json!({}), Op::Compact),
        ("session/fork", json!({}), Op::Fork),
        ("session/rewind", json!({ "turns": 2 }), Op::Rewind { turns: 2 }),
        ("goal/set", json!({ "goal": "把测试补上" }), Op::GoalSet { goal: "把测试补上".into() }),
        ("goal/pause", json!({ "goal_id": "goal-1" }), Op::GoalPause { goal_id: "goal-1".into() }),
        ("goal/resume", json!({ "goal_id": "goal-1" }), Op::GoalResume { goal_id: "goal-1".into() }),
        ("goal/advance", json!({}), Op::GoalAdvance),
        ("goal/clear", json!({}), Op::GoalClear),
        ("shutdown", json!({}), Op::Shutdown),
    ];
    assert_eq!(cases.len(), jsonrpc::OP_METHODS.len(), "用例必须覆盖全部方法");

    let mut input = vec![init_line(1)];
    for (i, (method, params, _)) in cases.iter().enumerate() {
        input.push(op_line(i as i64 + 2, method, params.clone()));
    }
    let s = Session::run(&input, stream_for);

    let expected: Vec<Op> = cases.iter().map(|(_, _, op)| op.clone()).collect();
    assert_eq!(s.ops, expected, "每个方法都必须映射成它自己的那个 Op");

    // 每个方法都要有响应（受理语义），且都是 accepted
    for i in 0..cases.len() {
        let r = s.response_to(i as i64 + 2);
        assert_eq!(r["result"]["accepted"], true, "方法 {} 应被受理", cases[i].0);
    }
}

#[test]
fn events_arrive_as_notifications_with_seq_kind_and_nested_payload() {
    // 用 ApprovalRequest 当样本是**故意的**：它自带 `kind` 字段（内核判定的
    // 调用类别），若把载荷摊平到顶层就会与事件名撞键 —— 这条测试钉住"不摊平"。
    let s = Session::run(&[init_line(1), op_line(2, "turn/pump", json!({}))], |_| {
        vec![
            EventMsg::TurnStarted { turn_id: "t1".into() },
            EventMsg::ApprovalRequest {
                id: "ap-1".into(),
                detail: "bash rm -rf build".into(),
                kind: "write".into(),
            },
        ]
    });
    let events = s.events();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["params"]["seq"], 1);
    assert_eq!(events[0]["params"]["kind"], "turn_started");
    assert_eq!(events[0]["params"]["payload"]["turn_id"], "t1");
    assert_eq!(events[1]["params"]["seq"], 2, "seq 在同一连接内严格递增");
    assert_eq!(events[1]["params"]["kind"], "approval_request");
    assert_eq!(events[1]["params"]["payload"]["kind"], "write", "调用类别必须保留");
    assert_eq!(events[1]["params"]["payload"]["id"], "ap-1");
}

#[test]
fn approval_round_trip_carries_the_denial_reason_back_to_the_kernel() {
    let s = Session::run(
        &[
            init_line(1),
            op_line(2, "turn/start", json!({ "text": "改一下配置" })),
            // 客户端看到 ApprovalRequest 通知后，用同一个 id 应答
            op_line(3, "approval/respond", json!({ "id": "ap-9", "decision": "deny", "reason": "改错文件了" })),
        ],
        stream_for,
    );
    let ops = &s.ops;
    assert_eq!(ops.len(), 2);
    match &ops[1] {
        Op::Approve { id, decision, reason } => {
            assert_eq!(id, "ap-9");
            assert_eq!(*decision, Decision::Deny);
            assert_eq!(reason.as_deref(), Some("改错文件了"), "拒绝理由要进模型可见的工具结果");
        }
        other => panic!("approval/respond 应产出 Approve：{other:?}"),
    }
}

#[test]
fn unknown_method_bad_params_and_malformed_json_do_not_kill_the_server() {
    let s = Session::run(
        &[
            init_line(1),
            op_line(2, "turn/nope", json!({})),
            op_line(3, "turn/start", json!({})),            // 缺 text
            op_line(4, "session/configure", json!({ "execMode": "plan" })), // camelCase 拼错
            json!({ "jsonrpc": "2.0", "id": 5, "method": "turn/pump" }),    // 正常
        ],
        stream_for,
    );
    assert_eq!(s.response_to(2)["error"]["code"], jsonrpc::METHOD_NOT_FOUND);
    assert_eq!(s.response_to(3)["error"]["code"], jsonrpc::INVALID_PARAMS);
    assert_eq!(s.response_to(4)["error"]["code"], jsonrpc::INVALID_PARAMS);
    assert!(
        s.response_to(4)["error"]["message"].as_str().unwrap_or_default().contains("execMode"),
        "错误消息要点名是哪个键"
    );
    assert_eq!(s.response_to(5)["result"]["accepted"], true, "前面的错误不能影响后续请求");
    assert_eq!(s.ops, vec![Op::Pump], "只有合法请求下发内核");
}

#[test]
fn malformed_json_yields_a_parse_error_with_a_null_id() {
    // 直接插一行非 JSON：它没有可信的 id，按规范用 null
    let text = format!("{}\n这不是 JSON\n{}\n", init_line(1), op_line(2, "turn/pump", json!({})));
    let sink = SharedBuf::default();
    serve(Cursor::new(text.into_bytes()), sink.clone(), |op: Op| stream_for(&op)).expect("serve 不该失败");
    let raw = String::from_utf8(sink.0.lock().expect("锁中毒").clone()).expect("UTF-8");
    let lines: Vec<Value> = raw.lines().map(|l| serde_json::from_str(l).expect("每行合法 JSON")).collect();
    let parse_err = lines
        .iter()
        .find(|l| l.get("error").is_some())
        .expect("应有一行 parse error");
    assert_eq!(parse_err["id"], Value::Null);
    assert_eq!(parse_err["error"]["code"], jsonrpc::PARSE_ERROR);
    // 坏行之后服务照常
    assert!(lines.iter().any(|l| l["result"]["accepted"] == true));
}

#[test]
fn invalid_envelopes_are_rejected_but_the_session_survives() {
    let s = Session::run(
        &[
            json!({ "id": 1, "method": "initialize", "params": {} }),        // 缺 jsonrpc
            json!({ "jsonrpc": "1.0", "id": 2, "method": "initialize" }),     // 版本不对
            json!({ "jsonrpc": "2.0", "id": 3 }),                            // 缺 method
            init_line(4),                                                    // 正常
            op_line(5, "turn/pump", json!({})),
        ],
        stream_for,
    );
    for id in [1, 2, 3] {
        assert_eq!(s.response_to(id)["error"]["code"], jsonrpc::INVALID_REQUEST, "id={id}");
    }
    assert!(s.response_to(4)["result"].is_object(), "信封错误之后仍能握手");
    assert_eq!(s.ops, vec![Op::Pump]);
}

#[test]
fn oversized_request_lines_are_dropped_with_an_honest_byte_count() {
    // 一条远超上限、且**不含换行**的请求行。没有输入上限时，这一行就能把宿主
    // 的内存吃光；有上限时它必须被丢弃，而不是被解析或让连接崩掉。
    let limit = neo_host_appserver::serve::MAX_REQUEST_BYTES;
    let blast = format!(
        r#"{{"jsonrpc":"2.0","id":9,"method":"turn/start","params":{{"text":"{}"}}}}"#,
        "x".repeat(limit + 64)
    );
    let text = format!(
        "{}\n{}\n{}\n",
        init_line(1),
        blast,
        op_line(2, "turn/pump", json!({})),
    );

    let ops = Arc::new(Mutex::new(Vec::new()));
    let sink = SharedBuf::default();
    let kernel_side = ops.clone();
    serve(Cursor::new(text.into_bytes()), sink.clone(), move |op: Op| {
        kernel_side.lock().expect("锁中毒").push(op);
        Vec::new()
    })
    .expect("serve 不该失败");

    let raw = String::from_utf8(sink.0.lock().expect("锁中毒").clone()).expect("UTF-8");
    let lines: Vec<Value> = raw.lines().map(|l| serde_json::from_str(l).expect("每行合法 JSON")).collect();

    let over = lines
        .iter()
        .find(|l| l["error"]["message"].as_str().unwrap_or_default().contains("超过上限"))
        .unwrap_or_else(|| panic!("应有一条「超过上限」的错误：{lines:#?}"));
    assert_eq!(over["error"]["code"], jsonrpc::INVALID_REQUEST);
    assert_eq!(over["error"]["data"]["limit"], limit);
    // 口径：丢弃字节数 = **整行的总字节数**（含换行符）。客户端据此知道
    // "这条请求有多少字节没被处理"，而不是只报越界的那部分。
    assert_eq!(
        over["error"]["data"]["discarded_bytes"],
        blast.len() + 1,
        "丢弃字节数要如实上报整行长度：{over:#?}"
    );

    // 关键：残片**没有**被当成下一条请求解析（否则这里会出现一串 parse error），
    // 而且超限行之后的服务照常
    assert_eq!(lines.iter().filter(|l| l["error"].is_object()).count(), 1, "只应有一条错误：{lines:#?}");
    let recorded = ops.lock().expect("锁中毒").clone();
    assert_eq!(recorded, vec![Op::Pump], "超限的行不得下发内核，后续请求照常");
}

#[test]
fn client_notifications_get_no_response() {
    // 无 id = 客户端通知：本协议不定义任何客户端通知，按规范静默忽略而不是回错
    let s = Session::run(
        &[init_line(1), json!({ "jsonrpc": "2.0", "method": "turn/pump", "params": {} })],
        stream_for,
    );
    assert_eq!(s.responses().len(), 1, "只应有 initialize 那一条响应：{:?}", s.lines);
    assert!(s.ops.is_empty(), "通知不得被当成请求下发内核");
}

#[test]
fn shutdown_ends_the_session_after_the_kernel_confirms() {
    let s = Session::run(
        &[init_line(1), op_line(2, "shutdown", json!({})), op_line(3, "turn/pump", json!({}))],
        stream_for,
    );
    assert_eq!(s.response_to(2)["result"]["accepted"], true);
    assert_eq!(s.ops, vec![Op::Shutdown], "关停之后的行不再被读取（pump 不该下发）");
    let kinds: Vec<&str> = s.events().iter().filter_map(|e| e["params"]["kind"].as_str()).collect();
    assert!(kinds.contains(&"shutdown_complete"), "内核的收尾事件必须送达：{kinds:?}");
}

#[test]
fn eof_closes_the_session_without_a_shutdown_method() {
    // 客户端直接关掉 stdin（崩溃/被杀）也要干净收尾，而不是挂住线程
    let s = Session::run(&[init_line(1), op_line(2, "turn/pump", json!({}))], stream_for);
    assert_eq!(s.ops, vec![Op::Pump]);
    assert_eq!(s.events().len(), 2, "已下发 Op 的事件仍要写完");
}
