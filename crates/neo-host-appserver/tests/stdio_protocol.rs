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

use neo_host_appserver::{jsonrpc, serve_ops, PROTOCOL_VERSION};
use neo_host_appserver::serve as serve_jobs;
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
        serve_ops(Cursor::new(text.into_bytes()), sink.clone(), move |op: Op| {
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
    assert_eq!(methods.len(), 109, "initialize + 19 Op + 85 control + aliases");
    assert!(methods.iter().any(|m| m == "turn/start"));
    assert!(methods.iter().any(|m| m == "turn/interrupt"), "中断必须在线上可达");
    assert!(methods.iter().any(|m| m == "turn/steer"), "Codex steer 必须可达");
    assert!(methods.iter().any(|m| m == "thread/rename"), "改名必须在线上可达");
    assert!(methods.iter().any(|m| m == "thread/goal/get"), "Codex goal 只读查询必须可达");
    assert!(methods.iter().any(|m| m == "user_input/respond"), "问用户反向通道必须可达");
    assert!(methods.iter().any(|m| m == "command/exec/write"), "终端 write 必须可达");
    assert!(methods.iter().any(|m| m == "skills/list"), "skills/list 必须可达");
    assert!(methods.iter().any(|m| m == "fs/readFile"), "fs/readFile 必须可达");
    assert!(methods.iter().any(|m| m == "thread/items/list"), "items/list 必须可达");
    assert!(methods.iter().any(|m| m == "model/list"), "Codex model/list 别名必须可达");
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
        ("turn/steer", json!({ "text": "改用 Rust" }), Op::Steer { text: "改用 Rust".into() }),
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
            "user_input/respond",
            json!({ "id": "ui-1", "response": "选 A" }),
            Op::RespondUserInput { id: "ui-1".into(), response: "选 A".into() },
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
    serve_ops(Cursor::new(text.into_bytes()), sink.clone(), |op: Op| stream_for(&op)).expect("serve 不该失败");
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
    serve_ops(Cursor::new(text.into_bytes()), sink.clone(), move |op: Op| {
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


/// `thread/*`：会话库控制走同步响应，历史以事件通知推送。
#[test]
fn thread_list_get_resume_create_delete_round_trip() {
    use neo_host_appserver::{Job, JobOut, ThreadResult};
    use neo_host_appserver::ThreadCmd;

    let input = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
        json!({"jsonrpc":"2.0","id":2,"method":"thread/list","params":{}}),
        json!({"jsonrpc":"2.0","id":3,"method":"thread/get","params":{"id":"t-1"}}),
        json!({"jsonrpc":"2.0","id":4,"method":"thread/resume","params":{"id":"t-1"}}),
        json!({"jsonrpc":"2.0","id":5,"method":"thread/create","params":{}}),
        json!({"jsonrpc":"2.0","id":6,"method":"thread/delete","params":{"id":"t-1"}}),
        json!({"jsonrpc":"2.0","id":7,"method":"thread/get","params":{"id":"nope"}}),
        json!({"jsonrpc":"2.0","id":8,"method":"shutdown","params":{}}),
    ];
    let text = input.iter().map(Value::to_string).collect::<Vec<_>>().join("\n") + "\n";
    let sink = SharedBuf::default();
    serve_jobs(
        Cursor::new(text.into_bytes()),
        sink.clone(),
        |job| match job {
            Job::Op(_) => JobOut::Events(vec![EventMsg::ShutdownComplete]),
            Job::Thread { cmd, .. } => JobOut::Thread(match cmd {
                ThreadCmd::List => ThreadResult::Value(json!({
                    "threads": [{"id":"t-1","title":"修 bug","has_title":true,"state":"idle"}]
                })),
                ThreadCmd::Get { id } if id == "t-1" => ThreadResult::Value(json!({
                    "thread": {"id":"t-1","title":"修 bug","has_title":true,"state":"idle"}
                })),
                ThreadCmd::Get { id } => ThreadResult::Error(format!("会话 {id} 不存在")),
                ThreadCmd::Resume { id } => ThreadResult::Resumed {
                    result: json!({"id": id, "events_replayed": 1}),
                    history: vec![EventMsg::UserSubmitted { text: "hi".into() }],
                },
                ThreadCmd::Create => ThreadResult::Value(json!({"id": "t-new"})),
                ThreadCmd::Delete { id } => ThreadResult::Value(json!({"removed": id == "t-1"})),
                ThreadCmd::Rename { id, title } => {
                    ThreadResult::Value(json!({"id": id, "title": title}))
                }
                ThreadCmd::History { .. } => ThreadResult::Value(json!({"items": []})),
                ThreadCmd::Export { .. } => ThreadResult::Value(json!({"content": "# ok"})),
                ThreadCmd::Tools => ThreadResult::Value(json!({"tools": [{"name":"bash"}]})),
                ThreadCmd::Models => ThreadResult::Value(json!({"models": [{"name":"mock"}], "current": "mock"})),
                ThreadCmd::GitInfo { .. } => ThreadResult::Value(json!({"in_repo": false})),
                ThreadCmd::GoalGet => ThreadResult::Value(json!({"goal": null})),
                ThreadCmd::InjectItems { text } => ThreadResult::Value(json!({"injected": text.chars().count()})),
                ThreadCmd::Revert { before_turn_id } => ThreadResult::Value(json!({"before": before_turn_id, "turns": 0})),
                ThreadCmd::MetadataUpdate { id, .. } => ThreadResult::Value(json!({"id": id, "updated": true})),
                ThreadCmd::AttachmentAdd { id, attachment_type, identity_key, .. } => ThreadResult::Value(json!({
                    "id": id, "attachmentType": attachment_type, "identityKey": identity_key
                })),
                ThreadCmd::AttachmentList { id } => ThreadResult::Value(json!({"id": id, "attachments": []})),
                ThreadCmd::AttachmentRemove { id, attachment_type, identity_key } => ThreadResult::Value(json!({
                    "id": id, "attachmentType": attachment_type, "identityKey": identity_key, "removed": true
                })),
                ThreadCmd::McpToolCall { server, tool, .. } => ThreadResult::Value(json!({
                    "server": server, "tool": tool, "output": ""
                })),
                ThreadCmd::McpResourceRead { server, uri } => ThreadResult::Value(json!({
                    "server": server, "uri": uri, "text": ""
                })),
                ThreadCmd::Archive { id, archived } => ThreadResult::Value(json!({"id": id, "archived": archived})),
                ThreadCmd::HooksList => ThreadResult::Value(json!({"hooks": []})),
                ThreadCmd::McpServerStatusList => ThreadResult::Value(json!({"servers": []})),
                ThreadCmd::PermissionProfileList => ThreadResult::Value(json!({"profiles": []})),
                ThreadCmd::ModelProviderCapabilities => ThreadResult::Value(json!({"models": [], "current": "mock"})),
                ThreadCmd::FsCopy { from, to } => ThreadResult::Value(json!({"from": from, "to": to, "bytes": 0})),
                ThreadCmd::FsCreateDirectory { path } => ThreadResult::Value(json!({"path": path, "created": true})),
                ThreadCmd::FsRemove { path } => ThreadResult::Value(json!({"path": path, "removed": true})),
                ThreadCmd::NameSet { id, title } => ThreadResult::Value(json!({"id": id, "title": title})),
                ThreadCmd::ItemsList { .. } => ThreadResult::Value(json!({"items": []})),
                ThreadCmd::TurnsList { .. } => ThreadResult::Value(json!({"turns": []})),
                ThreadCmd::SkillsList => ThreadResult::Value(json!({"skills": []})),
                ThreadCmd::ConfigRead => ThreadResult::Value(json!({"model": "mock"})),
                ThreadCmd::FsReadFile { path } => ThreadResult::Value(json!({"path": path, "content": "", "bytes": 0, "truncated": false})),
                ThreadCmd::FsWriteFile { path, .. } => ThreadResult::Value(json!({"path": path, "bytes": 0})),
                ThreadCmd::FsGetMetadata { path } => ThreadResult::Value(json!({"path": path, "is_file": true, "is_dir": false, "bytes": 0})),
                ThreadCmd::FsReadDirectory { path } => ThreadResult::Value(json!({"path": path, "entries": []})),
                ThreadCmd::ExecStart { command, cols, rows } => ThreadResult::Resumed {
                    result: json!({
                        "session_id": "exec-1",
                        "running": true,
                        "output": "",
                        "exit_code": null,
                        "truncated": false,
                        "mode": "pty",
                        "command": command,
                        "cols": cols,
                        "rows": rows,
                    }),
                    history: vec![],
                },
                ThreadCmd::ExecWrite { session_id, data } => ThreadResult::Value(json!({
                    "session_id": session_id,
                    "output": data,
                    "running": true,
                    "exit_code": null,
                    "truncated": false,
                })),
                ThreadCmd::ExecResize { session_id, cols, rows } => ThreadResult::Value(
                    json!({"session_id": session_id, "cols": cols, "rows": rows, "resized": true}),
                ),
                ThreadCmd::ExecTerminate { session_id } => ThreadResult::Value(
                    json!({"session_id": session_id, "terminated": true, "running": false}),
                ),
                ThreadCmd::MarketplaceAdd { name, source } => ThreadResult::Value(
                    json!({"name": name, "source": source}),
                ),
                ThreadCmd::MarketplaceRemove { name } => ThreadResult::Value(
                    json!({"name": name, "removed": true}),
                ),
                ThreadCmd::MarketplaceUpgrade { name } => ThreadResult::Value(
                    json!({"marketplaces": [], "name": name}),
                ),
                ThreadCmd::PluginList => ThreadResult::Value(json!({"plugins": [], "marketplaces": []})),
                ThreadCmd::PluginInstalled => ThreadResult::Value(json!({"plugins": []})),
                ThreadCmd::PluginReconcile => ThreadResult::Value(json!({"alive": 0, "removed": []})),
                ThreadCmd::PluginRead { name } => ThreadResult::Value(json!({"id": name})),
                ThreadCmd::PluginInstall { name } => ThreadResult::Value(
                    json!({"pluginId": name, "installed": true}),
                ),
                ThreadCmd::PluginUninstall { id } => ThreadResult::Value(
                    json!({"pluginId": id, "removed": true}),
                ),
                ThreadCmd::PluginSkillRead { plugin, skill } => ThreadResult::Value(
                    json!({"pluginId": plugin, "skillName": skill, "content": ""}),
                ),
                ThreadCmd::SectionList { .. } => ThreadResult::Value(
                    json!({"sections": [], "nextCursor": null}),
                ),
                ThreadCmd::SectionCreate { name, .. } => ThreadResult::Value(
                    json!({"sectionId": "sec-1", "name": name}),
                ),
                ThreadCmd::SectionUpdate { section_id, name, .. } => ThreadResult::Value(
                    json!({"sectionId": section_id, "name": name}),
                ),
                ThreadCmd::SectionDelete { section_id } => ThreadResult::Value(
                    json!({"sectionId": section_id, "removed": true}),
                ),
                ThreadCmd::SectionMoveThread { thread_id, section_id, .. } => ThreadResult::Value(
                    json!({"threadId": thread_id, "sectionId": section_id}),
                ),
                ThreadCmd::ThreadUnsubscribe { id } => ThreadResult::Value(
                    json!({"threadId": id, "unsubscribed": true}),
                ),
                ThreadCmd::ConfigValueWrite { key_path, .. } => ThreadResult::Value(
                    json!({"keyPath": key_path, "applied": true}),
                ),
                ThreadCmd::ConfigBatchWrite { edits, .. } => ThreadResult::Value(
                    json!({"applied": edits.len()}),
                ),
                ThreadCmd::ConfigMcpReload => ThreadResult::Value(
                    json!({"reloaded": true, "servers": []}),
                ),
                ThreadCmd::ConfigRequirementsRead => ThreadResult::Value(
                    json!({"requirements": []}),
                ),
                ThreadCmd::SkillsConfigWrite { enabled, name, path } => ThreadResult::Value(
                    json!({"enabled": enabled, "name": name, "path": path}),
                ),
                ThreadCmd::SkillsExtraRootsSet { extra_roots } => ThreadResult::Value(
                    json!({"extraRoots": extra_roots}),
                ),
                ThreadCmd::ExperimentalList { .. } => ThreadResult::Value(
                    json!({"features": [], "nextCursor": null}),
                ),
                ThreadCmd::ExperimentalSet { enablement } => ThreadResult::Value(
                    json!({"enablement": enablement}),
                ),
                ThreadCmd::AppList { .. } => ThreadResult::Value(json!({"apps": []})),
                ThreadCmd::AppRead { app_ids, .. } => ThreadResult::Value(
                    json!({"apps": [], "requested": app_ids}),
                ),
                ThreadCmd::AppInstalled { .. } => ThreadResult::Value(json!({"apps": []})),
                ThreadCmd::FuzzyFileSearch { query, .. } => ThreadResult::Value(
                    json!({"matches": [], "truncated": false, "query": query}),
                ),
                ThreadCmd::WindowsSandboxReadiness => ThreadResult::Value(
                    json!({"supported": false, "ready": false}),
                ),
                ThreadCmd::WindowsSandboxSetupStart { mode, .. } => ThreadResult::Error(
                    format!("仅 Windows：{mode}"),
                ),
                ThreadCmd::GuardianDenied { id, .. } => ThreadResult::Error(
                    format!("无 Guardian：{id}"),
                ),
                ThreadCmd::ReviewStart { thread_id, .. } => ThreadResult::Value(
                    json!({"started": true, "threadId": thread_id, "drives": false}),
                ),
                ThreadCmd::FeedbackUpload { classification, .. } => ThreadResult::Value(
                    json!({"uploaded": false, "classification": classification}),
                ),
                ThreadCmd::FsWatch { path, watch_id } => ThreadResult::Value(
                    json!({"watchId": watch_id, "path": path, "watching": true}),
                ),
                ThreadCmd::FsUnwatch { watch_id } => ThreadResult::Value(
                    json!({"watchId": watch_id, "stopped": true}),
                ),
                ThreadCmd::ExtAgentDetect { .. } => ThreadResult::Value(
                    json!({"migrationItems": [], "migrationSource": "local-fs"}),
                ),
                ThreadCmd::ExtAgentImport { migration_items, .. } => ThreadResult::Value(
                    json!({"imported": 0, "skipped": migration_items.len(), "failed": 0}),
                ),
                ThreadCmd::ExtAgentImportReadHistories => ThreadResult::Value(
                    json!({"histories": []}),
                ),
                ThreadCmd::ExtAgentImportRecordHistory { provider_id, .. } => ThreadResult::Value(
                    json!({"recorded": true, "providerId": provider_id}),
                ),
                ThreadCmd::McpOauthLogin { name, .. } => ThreadResult::Error(
                    format!("无 OAuth：{name}"),
                ),
            }),
        },
    )
    .expect("serve 不该失败");

    let raw = String::from_utf8(sink.0.lock().expect("锁中毒").clone()).expect("UTF-8");
    let lines: Vec<Value> = raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("行应是 JSON"))
        .collect();

    let by_id = |id: u64| {
        lines
            .iter()
            .find(|v| v["id"] == json!(id) && v.get("result").is_some())
            .unwrap_or_else(|| panic!("id={id} 应有 result"))
            .clone()
    };
    assert_eq!(by_id(2)["result"]["threads"][0]["id"], "t-1");
    assert_eq!(by_id(3)["result"]["thread"]["title"], "修 bug");
    assert_eq!(by_id(4)["result"]["id"], "t-1");
    assert_eq!(by_id(4)["result"]["events_replayed"], 1);
    assert_eq!(by_id(5)["result"]["id"], "t-new");
    assert_eq!(by_id(6)["result"]["removed"], true);
    // 不存在的会话：error 而不是空 result
    let err = lines
        .iter()
        .find(|v| v["id"] == json!(7) && v.get("error").is_some())
        .expect("id=7 应有 error");
    assert!(err["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .contains("不存在"));

    // resume 的历史必须以事件通知到达（带 seq），且载荷在 payload 下
    let ev = lines
        .iter()
        .find(|v| v.get("method") == Some(&json!("event")))
        .expect("应有事件通知");
    assert!(ev["params"]["seq"].as_u64().unwrap_or(0) >= 1);
    assert_eq!(ev["params"]["kind"], "user_submitted");
    assert_eq!(ev["params"]["payload"]["text"], "hi");
}

/// 未装配会话库时，thread/* 必须如实报错，不能静默空列表。
#[test]
fn thread_methods_without_session_store_fail_honestly() {
    let input = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
        json!({"jsonrpc":"2.0","id":2,"method":"thread/list","params":{}}),
    ];
    let text = input.iter().map(Value::to_string).collect::<Vec<_>>().join("\n") + "\n";
    let sink = SharedBuf::default();
    serve_ops(
        Cursor::new(text.into_bytes()),
        sink.clone(),
        |_: Op| Vec::new(),
    )
    .expect("serve 不该失败");
    let raw = String::from_utf8(sink.0.lock().expect("锁中毒").clone()).expect("UTF-8");
    let lines: Vec<Value> = raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("行应是 JSON"))
        .collect();
    let err = lines
        .iter()
        .find(|v| v["id"] == json!(2) && v.get("error").is_some())
        .expect("thread/list 在无会话库时必须报错");
    assert!(err["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .contains("会话库"));
}


/// `thread/history`：投影 = `facts_of`（T6 同源），不是第二套判定。
#[test]
fn thread_history_returns_facts_of_events_not_a_second_projection() {
    use neo_host_appserver::{Job, JobOut, ThreadCmd, ThreadResult};
    use neo_protocol::facts_of;

    let events = vec![
        EventMsg::UserSubmitted { text: "看下日志".into() },
        EventMsg::AgentMessageDone { text: "好的".into() },
        EventMsg::ToolCallBegin {
            id: "t1".into(),
            name: "bash".into(),
            arguments: json!({"command": "ls"}),
        },
        EventMsg::ToolCallEnd {
            id: "t1".into(),
            exit_code: 0,
            stdout: "a.rs\n".into(),
            stderr: String::new(),
            truncated: false,
        },
    ];
    let expected = facts_of(&events);

    let input = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
        json!({"jsonrpc":"2.0","id":2,"method":"thread/history","params":{}}),
        json!({"jsonrpc":"2.0","id":3,"method":"thread/history","params":{"id":"other"}}),
    ];
    let text = input.iter().map(Value::to_string).collect::<Vec<_>>().join("\n") + "\n";
    let sink = SharedBuf::default();
    serve_jobs(
        Cursor::new(text.into_bytes()),
        sink.clone(),
        move |job| match job {
            Job::Op(_) => JobOut::Events(vec![]),
            Job::Thread { cmd, .. } => JobOut::Thread(match cmd {
                ThreadCmd::History { id } => {
                    let id = id.unwrap_or_else(|| "current".into());
                    if id == "other" {
                        ThreadResult::Error(format!("会话 {id} 不存在"))
                    } else {
                        ThreadResult::Value(json!({
                            "id": id,
                            "events_replayed": events.len(),
                            "items": facts_of(&events),
                        }))
                    }
                }
                _ => ThreadResult::Value(json!({})),
            }),
        },
    )
    .expect("serve 不该失败");

    let raw = String::from_utf8(sink.0.lock().expect("锁中毒").clone()).expect("UTF-8");
    let lines: Vec<Value> = raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("行应是 JSON"))
        .collect();

    let hist = lines
        .iter()
        .find(|v| v["id"] == json!(2) && v.get("result").is_some())
        .expect("id=2 应有 result");
    let items = hist["result"]["items"].as_array().expect("items 应是数组");
    assert_eq!(items.len(), expected.len(), "投影条数必须等于 facts_of");

    // 关键断言：线格式反序列化后与 facts_of 逐条相等（同源，不是长得像）
    let wire: Vec<neo_protocol::Fact> =
        serde_json::from_value(hist["result"]["items"].clone()).expect("items 应能反序列化为 Fact");
    assert_eq!(wire, expected, "thread/history 的 items 必须就是 facts_of 的结果");

    // 不存在的 id：error
    let err = lines
        .iter()
        .find(|v| v["id"] == json!(3) && v.get("error").is_some())
        .expect("id=3 应有 error");
    assert!(err["error"]["message"].as_str().unwrap_or_default().contains("不存在"));
}


/// `tools/list` / `git/info` / `thread/export`：控制面三件套。
#[test]
fn tools_list_git_info_and_thread_export() {
    use neo_host_appserver::{Job, JobOut, ThreadCmd, ThreadResult};

    let input = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
        json!({"jsonrpc":"2.0","id":3,"method":"git/info","params":{}}),
        json!({"jsonrpc":"2.0","id":4,"method":"thread/export","params":{"format":"markdown"}}),
        json!({"jsonrpc":"2.0","id":5,"method":"thread/export","params":{"format":"yaml"}}),
        json!({"jsonrpc":"2.0","id":6,"method":"models/list","params":{}}),
    ];
    let text = input.iter().map(Value::to_string).collect::<Vec<_>>().join("\n") + "\n";
    let sink = SharedBuf::default();
    serve_jobs(
        Cursor::new(text.into_bytes()),
        sink.clone(),
        |job| match job {
            Job::Op(_) => JobOut::Events(vec![]),
            Job::Thread { cmd, .. } => JobOut::Thread(match cmd {
                ThreadCmd::Tools => ThreadResult::Value(json!({
                    "tools": [
                        {"name":"bash","description":"执行命令","parameters":{"type":"object"}},
                        {"name":"apply_patch","description":"落盘","parameters":{"type":"object"}},
                    ]
                })),
                ThreadCmd::GitInfo { .. } => ThreadResult::Value(json!({
                    "in_repo": true, "branch": "main", "sha": "abc1234",
                    "origin_url": "https://example.com/r.git", "root": "/ws"
                })),
                ThreadCmd::Export { format, .. } => {
                    let fmt = format.as_deref().unwrap_or("markdown");
                    if fmt == "yaml" {
                        ThreadResult::Error("不支持的导出格式：yaml（可选 markdown / json）".into())
                    } else {
                        ThreadResult::Value(json!({"format": fmt, "content": "# 会话\n"}))
                    }
                }
                ThreadCmd::Models => ThreadResult::Value(json!({
                    "models": [
                        {"name":"glm-4.6","description":"智谱","context_limit":128000,"production":true,"current":true},
                        {"name":"mock","description":"桩","context_limit":0,"production":false,"current":false},
                    ],
                    "current": "glm-4.6"
                })),
                _ => ThreadResult::Value(json!({})),
            }),
        },
    )
    .expect("serve 不该失败");

    let raw = String::from_utf8(sink.0.lock().expect("锁中毒").clone()).expect("UTF-8");
    let lines: Vec<Value> = raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("行应是 JSON"))
        .collect();

    let res = |id: u64| {
        lines
            .iter()
            .find(|v| v["id"] == json!(id) && v.get("result").is_some())
            .unwrap_or_else(|| panic!("id={id} 应有 result"))
            .clone()
    };
    assert_eq!(res(2)["result"]["tools"][0]["name"], "bash");
    assert_eq!(res(2)["result"]["tools"].as_array().unwrap().len(), 2);
    assert_eq!(res(3)["result"]["branch"], "main");
    assert_eq!(res(3)["result"]["origin_url"], "https://example.com/r.git");
    assert_eq!(res(4)["result"]["format"], "markdown");
    assert!(res(4)["result"]["content"].as_str().unwrap().contains("会话"));
    assert_eq!(res(6)["result"]["current"], "glm-4.6");
    assert_eq!(res(6)["result"]["models"].as_array().unwrap().len(), 2);

    let err = lines
        .iter()
        .find(|v| v["id"] == json!(5) && v.get("error").is_some())
        .expect("未知格式应报错");
    assert!(err["error"]["message"].as_str().unwrap_or_default().contains("yaml"));
}
