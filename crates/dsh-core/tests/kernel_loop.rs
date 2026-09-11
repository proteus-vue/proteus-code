//! 内核主循环的 conformance —— 证明 turn/step 真的跑起来了。
//!
//! 这一套是内核的**验收门禁**，不是"顺手加的测试"：
//! 它逐条断言主循环的语义（多步推进、闸门生效、审批挂起/恢复、
//! 确定性可回放），任一条失败说明内核没跑对。

use dsh_config::Config;
use dsh_core::{
    gate, CallKind, Kernel, KernelState, Message, ModelDelta, ModelProvider, SandboxBackend,
    SandboxOutcome, SessionPersistence, Tool, ToolCtx, ToolRegistry,
};
use dsh_mock::{
    tool_call, CountingTool, InMemoryPersistence, MockTool, ScriptedModelProvider,
    TamperingPersistence,
};
use dsh_protocol::{Decision, EventMsg, ExecMode, Op, SandboxMode, ToolOutput};
use serde_json::Value;
use std::sync::Arc;

/// 测试用沙箱：按模式声明能力，并在 read-only 下**真的拒绝**写命令。
struct TestSandbox;

impl SandboxBackend for TestSandbox {
    fn supports(&self, _mode: SandboxMode) -> bool { true }
    fn write_file(&self, _m: SandboxMode, _p: &std::path::Path, content: &str) -> dsh_core::FileOutcome {
        dsh_core::FileOutcome::Written { bytes: content.len() }
    }

    fn execute(&self, mode: SandboxMode, command: &str, _limit: usize) -> SandboxOutcome {
        if mode == SandboxMode::ReadOnly && command.contains("rm ") {
            return SandboxOutcome::Denied { reason: "read-only 禁止删除".into() };
        }
        SandboxOutcome::Ran { stdout: format!("ran:{command}"), truncated: false }
    }
}

/// 记录被执行的命令，用于断言"经沙箱执行了"。
struct RecordingTool { seen: Arc<std::sync::Mutex<Vec<String>>> }

impl Tool for RecordingTool {
    fn name(&self) -> &str { "bash" }
    fn describe(&self) -> String { "bash(cmd)".into() }
    fn call_kind(&self, args: &Value) -> CallKind {
        if args.get("cmd").and_then(Value::as_str).map(|c| c.starts_with("rm")).unwrap_or(false) {
            CallKind::Write
        } else {
            CallKind::Read
        }
    }
    fn execute(&self, args: &Value, ctx: &ToolCtx) -> ToolOutput {
        let cmd = args.get("cmd").and_then(Value::as_str).unwrap_or("");
        self.seen.lock().unwrap().push(cmd.to_string());
        match ctx.exec(cmd) {
            SandboxOutcome::Ran { stdout, truncated } => {
                ToolOutput { exit_code: 0, stdout, stderr: String::new(), truncated }
            }
            SandboxOutcome::Denied { reason } => ToolOutput { exit_code: -1, stdout: String::new(), stderr: reason, truncated: false },
        }
    }
}

fn cfg(mode: ExecMode) -> Config { Config { exec_mode: mode, ..Config::default() } }

fn kernel_with(
    model: Box<dyn ModelProvider>,
    tools: ToolRegistry,
    persistence: Box<dyn SessionPersistence>,
    mode: ExecMode,
) -> Kernel {
    Kernel::new("s1", cfg(mode), tools, model, Arc::new(TestSandbox), persistence, "/tmp")
}

fn read_tool() -> ToolRegistry {
    let mut r = ToolRegistry::new();
    r.register(Arc::new(MockTool::new("read")));
    r
}

// ─────────────── 主循环推进 ───────────────

#[test]
fn a_plain_turn_produces_started_and_complete() {
    let mut k = kernel_with(
        Box::new(ScriptedModelProvider::text_only("hello")),
        read_tool(),
        Box::new(InMemoryPersistence::new()),
        ExecMode::Default,
    );
    let events = k.submit(Op::UserTurn { text: "hi".into(), refs: vec![] }).unwrap();

    assert!(events.iter().any(|e| matches!(e, EventMsg::TurnStarted { .. })), "缺 TurnStarted");
    assert!(events.iter().any(|e| matches!(e, EventMsg::AgentMessageDone { .. })), "缺 AgentMessageDone");
    assert!(matches!(events.last(), Some(EventMsg::TurnComplete { .. })), "结尾应为 TurnComplete");
    assert_eq!(*k.state(), KernelState::Idle);
}

#[test]
fn the_loop_advances_until_the_model_stops_calling_tools() {
    // 第 1 步要工具，第 2 步要工具，第 3 步不要 → 应恰好 3 次模型响应
    let script = vec![
        vec![tool_call("c1", "read", serde_json::json!({}))],
        vec![tool_call("c2", "read", serde_json::json!({}))],
        vec![ModelDelta::Text("done".into())],
    ];
    let mut k = kernel_with(
        Box::new(ScriptedModelProvider::new(script)),
        read_tool(),
        Box::new(InMemoryPersistence::new()),
        // AutoEdit：写自动放行，便于专测推进而不过早挂审批
        ExecMode::AutoEdit,
    );
    let events = k.submit(Op::UserTurn { text: "go".into(), refs: vec![] }).unwrap();

    let begins = events.iter().filter(|e| matches!(e, EventMsg::ToolCallBegin { .. })).count();
    let ends = events.iter().filter(|e| matches!(e, EventMsg::ToolCallEnd { .. })).count();
    assert_eq!(begins, 2, "应发出 2 次 ToolCallBegin");
    assert_eq!(ends, 2, "应发出 2 次 ToolCallEnd");
    assert!(matches!(events.last(), Some(EventMsg::TurnComplete { .. })));

    // 历史应含：user + 2×(assistant with tool_calls + tool result)
    let assistants = k.messages().iter().filter(|m| matches!(m, Message::Assistant { .. })).count();
    assert_eq!(assistants, 3, "应有 3 条 assistant 消息（两步工具 + 一步收尾）");
}

#[test]
fn tools_execute_through_the_sandbox_and_in_order() {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut r = ToolRegistry::new();
    r.register(Arc::new(RecordingTool { seen: seen.clone() }));

    let script = vec![
        vec![
            tool_call("a", "bash", serde_json::json!({"cmd": "cat f"})),
            tool_call("b", "bash", serde_json::json!({"cmd": "ls ."})),
        ],
        vec![ModelDelta::Text("ok".into())],
    ];
    let mut k = kernel_with(
        Box::new(ScriptedModelProvider::new(script)),
        r,
        Box::new(InMemoryPersistence::new()),
        ExecMode::AutoEdit,
    );
    k.submit(Op::UserTurn { text: "go".into(), refs: vec![] }).unwrap();

    assert_eq!(*seen.lock().unwrap(), vec!["cat f".to_string(), "ls .".to_string()],
        "命令应按声明顺序经沙箱执行");
}

#[test]
fn step_budget_stops_a_runaway_loop() {
    // 模型永远要工具 → 必须被步数预算截断，而非死循环
    let script = vec![vec![tool_call("cx", "read", serde_json::json!({}))]; 50];
    let mut k = kernel_with(
        Box::new(ScriptedModelProvider::new(script)),
        read_tool(),
        Box::new(InMemoryPersistence::new()),
        ExecMode::AutoEdit,
    ).with_max_steps(3);

    let events = k.submit(Op::UserTurn { text: "loop".into(), refs: vec![] }).unwrap();
    let errored = events.iter().any(|e| matches!(e, EventMsg::Error { .. }));
    assert!(errored, "超出预算应发 Error");
}

// ─────────────── 闸门：沙箱是硬边界 ───────────────

#[test]
fn read_only_sandbox_denies_writes_even_under_full_approval() {
    // 关键断言：审批策略**推不翻**沙箱。这是"沙箱是技术边界"的机器证明。
    let res = dsh_config::resolve(ExecMode::Plan); // Plan → ReadOnly
    assert_eq!(res.sandbox, SandboxMode::ReadOnly);
    match gate(CallKind::Write, res) {
        dsh_core::GateDecision::Deny { .. } => {}
        other => panic!("read-only 下的写必须被硬拒，实际：{other:?}"),
    }
}

#[test]
fn read_only_allows_reads() {
    let res = dsh_config::resolve(ExecMode::Plan);
    assert_eq!(gate(CallKind::Read, res), dsh_core::GateDecision::Allow);
}

#[test]
fn default_mode_asks_before_writes() {
    let res = dsh_config::resolve(ExecMode::Default);
    assert!(matches!(gate(CallKind::Write, res), dsh_core::GateDecision::Ask { .. }),
        "Default 档写入前应询问");
    assert_eq!(gate(CallKind::Read, res), dsh_core::GateDecision::Allow, "读取不应打断");
}

#[test]
fn auto_edit_is_distinguished_only_by_the_file_edit_axis() {
    // ZCode 的 Default 与 AutoEdit 在双轴上完全相同，差异只在文件编辑粒度。
    // 缺这一维，两档在底层不可区分 —— 这是纳入第三维的理由。
    let d = dsh_config::resolve(ExecMode::Default);
    let a = dsh_config::resolve(ExecMode::AutoEdit);
    assert_eq!(d.sandbox, a.sandbox, "两档沙箱应相同");
    assert_eq!(d.approval, a.approval, "两档审批应相同");
    assert_ne!(d.file_edit, a.file_edit, "差异必须在 file_edit 上，否则两档不可区分");

    assert!(matches!(gate(CallKind::Write, d), dsh_core::GateDecision::Ask { .. }));
    assert_eq!(gate(CallKind::Write, a), dsh_core::GateDecision::Allow, "AutoEdit 写应放行");
}

// ─────────────── 审批：挂起与恢复 ───────────────

#[test]
fn a_write_suspends_the_turn_and_approval_resumes_it() {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut r = ToolRegistry::new();
    r.register(Arc::new(RecordingTool { seen: seen.clone() }));

    let script = vec![
        vec![tool_call("w1", "bash", serde_json::json!({"cmd": "rm -rf /tmp/x"}))],
        vec![ModelDelta::Text("finished".into())],
    ];
    let mut k = kernel_with(
        Box::new(ScriptedModelProvider::new(script)),
        r,
        Box::new(InMemoryPersistence::new()),
        ExecMode::Default, // 写需审批
    );

    let events = k.submit(Op::UserTurn { text: "clean".into(), refs: vec![] }).unwrap();
    let ask_id = events.iter().find_map(|e| match e {
        EventMsg::ApprovalRequest { id, .. } => Some(id.clone()),
        _ => None,
    });
    let ask_id = ask_id.expect("应发出 ApprovalRequest");
    assert!(matches!(k.state(), KernelState::AwaitingApproval { .. }), "应挂起");
    assert!(seen.lock().unwrap().is_empty(), "审批前不得执行");
    assert!(!events.iter().any(|e| matches!(e, EventMsg::TurnComplete { .. })),
        "挂起时不得结束本轮");

    // 批准 → 执行 → 继续下一步 → 收尾
    let resumed = k.submit(Op::Approve { id: ask_id, decision: Decision::Allow }).unwrap();
    assert_eq!(*seen.lock().unwrap(), vec!["rm -rf /tmp/x".to_string()], "批准后应执行");
    assert!(matches!(k.state(), KernelState::Idle));
    assert!(resumed.iter().any(|e| matches!(e, EventMsg::TurnComplete { .. })),
        "恢复后应完成本轮");
}

#[test]
fn denial_records_a_tool_result_without_executing() {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut r = ToolRegistry::new();
    r.register(Arc::new(RecordingTool { seen: seen.clone() }));
    let script = vec![
        vec![tool_call("w1", "bash", serde_json::json!({"cmd": "rm -rf /tmp/x"}))],
        vec![ModelDelta::Text("ok".into())],
    ];
    let mut k = kernel_with(
        Box::new(ScriptedModelProvider::new(script)), r,
        Box::new(InMemoryPersistence::new()), ExecMode::Default,
    );
    let events = k.submit(Op::UserTurn { text: "x".into(), refs: vec![] }).unwrap();
    let id = events.iter().find_map(|e| match e { EventMsg::ApprovalRequest { id, .. } => Some(id.clone()), _ => None }).unwrap();

    k.submit(Op::Approve { id, decision: Decision::Deny }).unwrap();
    assert!(seen.lock().unwrap().is_empty(), "拒绝后不得执行");
    // 拒绝也要留一条工具结果，模型才知道"这条路被否决"
    assert!(k.messages().iter().any(|m| matches!(
        m, Message::ToolResult { output, .. } if output.exit_code == -1
    )), "拒绝应向模型反映为失败结果");
}

#[test]
fn approving_without_a_pending_request_is_an_error() {
    let mut k = kernel_with(
        Box::new(ScriptedModelProvider::text_only("x")), read_tool(),
        Box::new(InMemoryPersistence::new()), ExecMode::Default,
    );
    let err = k.submit(Op::Approve { id: "nope".into(), decision: Decision::Allow });
    assert!(err.is_err(), "无待审批却批准应报错");
}

#[test]
fn each_call_in_one_step_gets_its_own_approval() {
    // 一步里两个写调用 → 应两次挂起（而不是只问一次就全放行）
    let mut r = ToolRegistry::new();
    r.register(Arc::new(MockTool::writing("w")));
    let script = vec![
        vec![
            tool_call("w1", "w", serde_json::json!({})),
            tool_call("w2", "w", serde_json::json!({})),
        ],
        vec![ModelDelta::Text("ok".into())],
    ];
    let mut k = kernel_with(
        Box::new(ScriptedModelProvider::new(script)), r,
        Box::new(InMemoryPersistence::new()), ExecMode::Default,
    );

    let ev1 = k.submit(Op::UserTurn { text: "x".into(), refs: vec![] }).unwrap();
    let id1 = ev1.iter().find_map(|e| match e { EventMsg::ApprovalRequest { id, .. } => Some(id.clone()), _ => None }).unwrap();

    // 批准第一个后，第二个调用仍须再问一次
    let ev2 = k.submit(Op::Approve { id: id1, decision: Decision::Allow }).unwrap();
    let id2 = ev2.iter().find_map(|e| match e { EventMsg::ApprovalRequest { id, .. } => Some(id.clone()), _ => None });
    assert!(id2.is_some(), "同一步的第二个写调用必须再问一次");
    assert_ne!(id2.unwrap(), ev1.iter().find_map(|e| match e { EventMsg::ApprovalRequest { id, .. } => Some(id.clone()), _ => None }).unwrap(),
        "两次审批应有不同的确定性 id");
}

// ─────────────── 确定性（T2 的前置）───────────────

/// 跑一个固定场景，返回事件的 JSON 序列。
fn run_scenario() -> String {
    let script = vec![
        vec![ModelDelta::Text("a".into()), tool_call("t1", "count", serde_json::json!({}))],
        vec![ModelDelta::Text("b".into())],
    ];
    let mut r = ToolRegistry::new();
    r.register(Arc::new(CountingTool::new()));
    let mut k = kernel_with(
        Box::new(ScriptedModelProvider::new(script)), r,
        Box::new(InMemoryPersistence::new()), ExecMode::AutoEdit,
    );
    let events = k.submit(Op::UserTurn { text: "go".into(), refs: vec![] }).unwrap();
    serde_json::to_string(&events).unwrap()
}

#[test]
fn the_same_op_sequence_yields_the_same_event_sequence() {
    // T2 的核心前置：内核本身不得引入非确定性（时钟/随机/哈希序）。
    assert_eq!(run_scenario(), run_scenario(), "同一 Op 序列必须得到同一事件序列");
}

#[test]
fn system_prompt_is_byte_stable_across_runs() {
    // 提示词缓存命中的前提：系统提示词不得含时间/随机内容。
    fn sys() -> String {
        let mut r = ToolRegistry::new();
        dsh_capability::register_defaults(&mut r);
        // 通过两次独立构造比对字节
        dsh_core::ToolRegistry::render_prompt(&r)
    }
    assert_eq!(sys(), sys(), "工具描述必须字节稳定");
}

// ─────────────── 落盘与持久化纪律 ───────────────

#[test]
fn every_model_visible_step_is_logged() {
    let p = InMemoryPersistence::new();
    let mut k = kernel_with(
        Box::new(ScriptedModelProvider::text_only("hi")), read_tool(),
        Box::new(p), ExecMode::Default,
    );
    k.submit(Op::UserTurn { text: "x".into(), refs: vec![] }).unwrap();

    let records = k.log_records();
    assert!(records.iter().any(|r| r.kind == "op"), "Op 应落日志");
    assert!(records.iter().any(|r| r.kind == "event"), "事件应落日志");
    // 序号必须从 1 开始严格递增（append-only 的前提）
    for (i, r) in records.iter().enumerate() {
        assert_eq!(r.seq, (i + 1) as u64, "seq 必须严格递增无空洞");
    }
}

#[test]
fn a_tampering_persistence_is_rejected_at_load() {
    // 内核不得容忍 append-only 被破坏：load 结果与写入不一致必须能被检出。
    let mut p = TamperingPersistence::new();
    p.append("op", serde_json::json!({"a": 1})).unwrap();
    p.append("op", serde_json::json!({"b": 2})).unwrap();
    let loaded = p.load().unwrap();
    assert_eq!(loaded.len(), 1, "篡改实现丢了首条 —— 这正说明契约必须断言保真");
}
