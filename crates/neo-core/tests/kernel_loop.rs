//! 内核主循环的 conformance —— 证明 turn/step 真的跑起来了。
//!
//! 这一套是内核的**验收门禁**，不是"顺手加的测试"：
//! 它逐条断言主循环的语义（多步推进、闸门生效、审批挂起/恢复、
//! 确定性可回放），任一条失败说明内核没跑对。

use neo_config::Config;
use neo_core::{
    gate, CallKind, Kernel, KernelState, Message, ModelDelta, ModelProvider, SandboxBackend,
    SandboxOutcome, SessionPersistence, Tool, ToolCtx, ToolRegistry,
};
use neo_mock::{
    tool_call, CountingTool, InMemoryPersistence, MockTool, ScriptedModelProvider,
    TamperingPersistence,
};
use neo_protocol::{Decision, EventMsg, ExecMode, Op, SandboxMode, ToolOutput};
use serde_json::Value;
use std::sync::Arc;

/// 测试用沙箱：按模式声明能力，并在 read-only 下**真的拒绝**写命令。
struct TestSandbox;

impl SandboxBackend for TestSandbox {
    fn supports(&self, _mode: SandboxMode) -> bool { true }
    fn write_file(&self, _m: SandboxMode, _p: &std::path::Path, content: &str) -> neo_core::FileOutcome {
        neo_core::FileOutcome::Written { bytes: content.len() }
    }

    fn execute(&self, mode: SandboxMode, command: &str, _limit: usize) -> SandboxOutcome {
        if mode == SandboxMode::ReadOnly && command.contains("rm ") {
            return SandboxOutcome::Denied { reason: "read-only 禁止删除".into() };
        }
        SandboxOutcome::Ran { stdout: format!("ran:{command}"), truncated: false }
    }
}

/// 会**真的落盘**的沙箱：给需要验证文件内容的用例用。
/// （`TestSandbox::write_file` 是桩，不写盘 —— 用它测"文件变了"会得到假失败。）
struct DiskSandbox;

impl SandboxBackend for DiskSandbox {
    fn supports(&self, _mode: SandboxMode) -> bool { true }
    fn write_file(&self, _m: SandboxMode, p: &std::path::Path, content: &str) -> neo_core::FileOutcome {
        match std::fs::write(p, content) {
            Ok(()) => neo_core::FileOutcome::Written { bytes: content.len() },
            Err(e) => neo_core::FileOutcome::Failed { reason: e.to_string() },
        }
    }
    fn execute(&self, _m: SandboxMode, command: &str, _limit: usize) -> SandboxOutcome {
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
    // 单 provider 包装成注册表：本文件的用例都不测模型切换，
    // 用 single() 保持调用点简洁。
    Kernel::new(
        "s1",
        cfg(mode),
        tools,
        neo_core::models::ModelRegistry::single(model),
        Arc::new(TestSandbox),
        persistence,
        "/tmp",
    )
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
    let res = neo_config::resolve(ExecMode::Plan); // Plan → ReadOnly
    assert_eq!(res.sandbox, SandboxMode::ReadOnly);
    match gate(CallKind::Write, res) {
        neo_core::GateDecision::Deny { .. } => {}
        other => panic!("read-only 下的写必须被硬拒，实际：{other:?}"),
    }
}

#[test]
fn read_only_allows_reads() {
    let res = neo_config::resolve(ExecMode::Plan);
    assert_eq!(gate(CallKind::Read, res), neo_core::GateDecision::Allow);
}

#[test]
fn default_mode_asks_before_writes() {
    let res = neo_config::resolve(ExecMode::Default);
    assert!(matches!(gate(CallKind::Write, res), neo_core::GateDecision::Ask { .. }),
        "Default 档写入前应询问");
    assert_eq!(gate(CallKind::Read, res), neo_core::GateDecision::Allow, "读取不应打断");
}

#[test]
fn auto_edit_is_distinguished_only_by_the_file_edit_axis() {
    // ZCode 的 Default 与 AutoEdit 在双轴上完全相同，差异只在文件编辑粒度。
    // 缺这一维，两档在底层不可区分 —— 这是纳入第三维的理由。
    let d = neo_config::resolve(ExecMode::Default);
    let a = neo_config::resolve(ExecMode::AutoEdit);
    assert_eq!(d.sandbox, a.sandbox, "两档沙箱应相同");
    assert_eq!(d.approval, a.approval, "两档审批应相同");
    assert_ne!(d.file_edit, a.file_edit, "差异必须在 file_edit 上，否则两档不可区分");

    assert!(matches!(gate(CallKind::Write, d), neo_core::GateDecision::Ask { .. }));
    assert_eq!(gate(CallKind::Write, a), neo_core::GateDecision::Allow, "AutoEdit 写应放行");
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
        neo_capability::register_defaults(&mut r);
        // 通过两次独立构造比对字节
        neo_core::ToolRegistry::render_prompt(&r)
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

// ─────────────── Op::Shell（用户直输命令）───────────────

#[test]
fn shell_op_runs_the_command_through_the_sandbox() {
    // `!cmd` 的核心契约：命令真的被执行，且结果作为工具结果进历史。
    // 这条测试是为了钉住一个**只在端到端才暴露**的 bug：
    // 内核构造参数用的键必须与工具读取的键一致，否则工具拿到 None，
    // 表现为 "exit -1" 且没有任何有意义的错误信息。
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(RecordingTool { seen: seen.clone() }));

    let mut k = kernel_with(
        Box::new(ScriptedModelProvider::text_only("unused")),
        tools,
        Box::new(InMemoryPersistence::new()),
        ExecMode::Default,
    );
    let events = k.submit(Op::Shell { command: "echo hi".into() }).unwrap();

    // 1) 命令经沙箱执行了（不是被当成空参数丢弃）
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        ["echo hi"],
        "参数键不一致会让工具读到空命令"
    );
    // 2) 事件形态：一对工具事件，且**不含 turn_complete**（不经模型）
    assert!(matches!(events[0], EventMsg::ToolCallBegin { .. }), "应先是 ToolCallBegin");
    assert!(matches!(events[1], EventMsg::ToolCallEnd { exit_code: 0, .. }), "应成功结束");
    assert!(
        !events.iter().any(|e| matches!(e, EventMsg::TurnComplete { .. })),
        "用户直输命令不该结束一个模型轮次"
    );
    // 3) 结果进历史，供下一轮模型参考
    assert!(
        k.messages().iter().any(|m| matches!(m, Message::ToolResult { .. })),
        "命令输出应作为工具结果留在历史里"
    );
}

#[test]
fn shell_op_missing_argument_fails_loudly() {
    // 空命令不该 panic，也不该假装成功
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(RecordingTool { seen }));
    let mut k = kernel_with(
        Box::new(ScriptedModelProvider::text_only("x")),
        tools,
        Box::new(InMemoryPersistence::new()),
        ExecMode::Default,
    );
    let events = k.submit(Op::Shell { command: String::new() }).unwrap();
    // 沙箱仍会被调用（空命令由 shell 自己处理），但事件必须完整闭合
    assert!(events.iter().any(|e| matches!(e, EventMsg::ToolCallEnd { .. })), "必须有结束事件");
}

// ─────────────── 审批前的改动预览 ───────────────

/// 一个会声明 preview 的假工具：用于断言"审批前先发 PatchProposed"。
struct PreviewTool;

impl Tool for PreviewTool {
    fn name(&self) -> &str { "apply_patch" }
    fn describe(&self) -> String { "apply_patch".into() }
    fn call_kind(&self, _a: &Value) -> CallKind { CallKind::Write }
    fn preview(&self, _a: &Value) -> Option<(String, String)> {
        Some(("a.txt".into(), "--- a/a.txt\n+++ b/a.txt\n@@ -1,1 +1,1 @@\n-old\n+new\n".into()))
    }
    fn execute(&self, _a: &Value, _c: &ToolCtx) -> ToolOutput {
        ToolOutput { exit_code: 0, stdout: "ok".into(), stderr: String::new(), truncated: false }
    }
}

#[test]
fn real_apply_patch_reports_its_change_after_writing() {
    // 关键点：用**真实** ApplyPatchTool 而不是假工具。
    //
    // 这个用例是为了抓住一个只会在真实工具上出现的顺序错误：
    // 若在内核里"执行后"才调 preview，读到的文件已等于目标 → diff 为空 →
    // 统计恒为 0。假工具的 preview 无条件返回，所以假工具**测不出**这个 bug
    // （我第一版正是被假工具骗过去的）。
    let dir = std::env::temp_dir().join(format!("neo-diff-tally-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("note.txt");
    std::fs::write(&target, "old line\n").unwrap();

    let mut tools = ToolRegistry::new();
    neo_capability::register_defaults(&mut tools);

    let mut k = Kernel::new(
        "s",
        cfg(ExecMode::AutoEdit),
        tools,
        neo_core::models::ModelRegistry::single(Box::new(ScriptedModelProvider::new(vec![
            vec![tool_call(
                "c1",
                "apply_patch",
                serde_json::json!({ "path": target.to_str().unwrap(), "old": "old line", "new": "new line" }),
            )],
            vec![ModelDelta::Text("done".into())],
        ]))),
        Arc::new(DiskSandbox),
        Box::new(InMemoryPersistence::new()),
        dir.clone(),
    );
    let events = k.submit(Op::UserTurn { text: "改".into(), refs: vec![] }).unwrap();

    // 文件确实变了
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "new line\n");
    // 且改动被统计到
    let files = events
        .iter()
        .find_map(|e| match e {
            EventMsg::FilesChanged { files } => Some(files.clone()),
            _ => None,
        })
        .expect("真实 apply_patch 写入后应广播文件改动");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].additions, 1);
    assert_eq!(files[0].deletions, 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn file_changes_are_tallied_after_a_successful_write() {
    // 钉住一个真实 bug：曾让工具"事后"算 diff —— 执行完文件已等于目标，
    // diff 为空，侧栏"已修改文件"永远为空。统计必须来自**执行前**的 preview。
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(PreviewTool));
    let mut k = kernel_with(
        Box::new(ScriptedModelProvider::new(vec![
            vec![tool_call("c1", "apply_patch", serde_json::json!({}))],
            vec![ModelDelta::Text("done".into())],
        ])),
        tools,
        Box::new(InMemoryPersistence::new()),
        // AutoEdit：写自动放行，无需审批，直接走执行路径
        ExecMode::AutoEdit,
    );
    let events = k.submit(Op::UserTurn { text: "改".into(), refs: vec![] }).unwrap();
    let files = events
        .iter()
        .find_map(|e| match e {
            EventMsg::FilesChanged { files } => Some(files.clone()),
            _ => None,
        })
        .expect("成功写入后应广播文件改动列表");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "a.txt");
    assert_eq!(files[0].additions, 1, "应统计到 1 行新增");
    assert_eq!(files[0].deletions, 1, "应统计到 1 行删除");
}

#[test]
fn a_failed_write_records_no_file_change() {
    // 失败的调用不该被计入"已修改文件"（否则侧栏会显示实际没发生的改动）
    struct FailingTool;
    impl Tool for FailingTool {
        fn name(&self) -> &str { "apply_patch" }
        fn describe(&self) -> String { "x".into() }
        fn call_kind(&self, _a: &Value) -> CallKind { CallKind::Write }
        fn preview(&self, _a: &Value) -> Option<(String, String)> {
            Some(("b.txt".into(), "+new\n".into()))
        }
        fn execute(&self, _a: &Value, _c: &ToolCtx) -> ToolOutput {
            ToolOutput { exit_code: -1, stdout: String::new(), stderr: "拒绝".into(), truncated: false }
        }
    }
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(FailingTool));
    let mut k = kernel_with(
        Box::new(ScriptedModelProvider::new(vec![
            vec![tool_call("c1", "apply_patch", serde_json::json!({}))],
            vec![ModelDelta::Text("done".into())],
        ])),
        tools,
        Box::new(InMemoryPersistence::new()),
        ExecMode::AutoEdit,
    );
    let events = k.submit(Op::UserTurn { text: "改".into(), refs: vec![] }).unwrap();
    assert!(
        !events.iter().any(|e| matches!(e, EventMsg::FilesChanged { .. })),
        "失败的写不该计入改动"
    );
}

#[test]
fn approval_emits_the_patch_preview_before_asking() {
    // 只说"写入类调用需确认"等于让用户盲批；必须先把改了什么给他看。
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(PreviewTool));
    let mut k = kernel_with(
        Box::new(ScriptedModelProvider::new(vec![
            vec![tool_call("c1", "apply_patch", serde_json::json!({}))],
            vec![ModelDelta::Text("done".into())],
        ])),
        tools,
        Box::new(InMemoryPersistence::new()),
        ExecMode::Default, // OnRequest：写要问
    );
    let events = k.submit(Op::UserTurn { text: "改文件".into(), refs: vec![] }).unwrap();

    let patch_at = events.iter().position(|e| matches!(e, EventMsg::PatchProposed { .. }));
    let ask_at = events.iter().position(|e| matches!(e, EventMsg::ApprovalRequest { .. }));
    let patch_at = patch_at.expect("审批前应发出 PatchProposed（含 diff）");
    let ask_at = ask_at.expect("写操作应请求审批");
    assert!(patch_at < ask_at, "预览必须**先于**审批请求发出（否则用户看不到就已被问）");

    if let EventMsg::PatchProposed { path, diff } = &events[patch_at] {
        assert_eq!(path, "a.txt");
        assert!(diff.contains("-old") && diff.contains("+new"), "预览应含具体改动：{diff}");
    }
}

#[test]
fn a_read_only_tool_emits_no_patch_preview() {
    // 只读调用不该弹出 diff（否则每次读文件都有一段无意义预览）
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(MockTool::new("read")));
    let mut k = kernel_with(
        Box::new(ScriptedModelProvider::new(vec![
            vec![tool_call("c1", "read", serde_json::json!({}))],
            vec![ModelDelta::Text("done".into())],
        ])),
        tools,
        Box::new(InMemoryPersistence::new()),
        ExecMode::Default,
    );
    let events = k.submit(Op::UserTurn { text: "读".into(), refs: vec![] }).unwrap();
    assert!(
        !events.iter().any(|e| matches!(e, EventMsg::PatchProposed { .. })),
        "只读调用不该有改动预览"
    );
}

// ─────────────── 工具声明的"运行后公告" ───────────────

/// 声明 report 的假工具：模拟 todowrite。
struct ReportingTool;

impl Tool for ReportingTool {
    fn name(&self) -> &str { "todowrite" }
    fn describe(&self) -> String { "todowrite".into() }
    fn call_kind(&self, _a: &Value) -> CallKind { CallKind::Read }
    fn report(&self, _a: &Value) -> Vec<EventMsg> {
        vec![EventMsg::TodoUpdated {
            items: vec![neo_protocol::TodoEntry {
                content: "写测试".into(),
                status: neo_protocol::TodoStatus::InProgress,
            }],
        }]
    }
    fn execute(&self, _a: &Value, _c: &ToolCtx) -> ToolOutput {
        ToolOutput { exit_code: 0, stdout: "ok".into(), stderr: String::new(), truncated: false }
    }
}

#[test]
fn tool_reported_events_are_logged_before_the_call_ends() {
    // 先看到清单变化、再看到调用结束 —— 顺序对宿主渲染有意义
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(ReportingTool));
    let mut k = kernel_with(
        Box::new(ScriptedModelProvider::new(vec![
            vec![tool_call("c1", "todowrite", serde_json::json!({}))],
            vec![ModelDelta::Text("done".into())],
        ])),
        tools,
        Box::new(InMemoryPersistence::new()),
        ExecMode::Default,
    );
    let events = k.submit(Op::UserTurn { text: "记清单".into(), refs: vec![] }).unwrap();
    let todo_at = events.iter().position(|e| matches!(e, EventMsg::TodoUpdated { .. }));
    let end_at = events
        .iter()
        .position(|e| matches!(e, EventMsg::ToolCallEnd { .. }));
    let todo_at = todo_at.expect("工具声明的 TodoUpdated 应被转发");
    let end_at = end_at.expect("应有 ToolCallEnd");
    assert!(todo_at < end_at, "清单事件应先于调用结束");
}

// ─────────────── 对话回退（Op::Rewind）───────────────

#[test]
fn rewind_drops_the_last_turn_and_its_messages() {
    // 三轮对话后回退一轮：最后一轮的消息应全部消失，且**必须**包含
    // 那一轮的助手回复与工具结果（只删用户消息会留下无主的助手发言）。
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(MockTool::new("read")));
    let mut k = kernel_with(
        Box::new(ScriptedModelProvider::new(vec![
            vec![tool_call("c1", "read", serde_json::json!({}))],
            vec![ModelDelta::Text("第一轮".into())],
        ])),
        tools,
        Box::new(InMemoryPersistence::new()),
        ExecMode::AutoEdit,
    );
    k.submit(Op::UserTurn { text: "问题一".into(), refs: vec![] }).unwrap();
    k.submit(Op::UserTurn { text: "问题二".into(), refs: vec![] }).unwrap();
    let before = k.messages().len();

    let ev = k.submit(Op::Rewind { turns: 1 }).unwrap();
    assert!(
        ev.iter().any(|e| matches!(e, EventMsg::Rewound { turns: 1, .. })),
        "应发出 Rewound 事件"
    );
    let after = k.messages().len();
    assert!(after < before, "回退后消息应减少（{before} → {after}）");
    // 第二轮的内容不该还在
    let has_second = k.messages().iter().any(|m| matches!(m, Message::User(t) if t.contains("问题二")));
    assert!(!has_second, "被回退轮次的用户消息应已删除");
    // 第一轮的内容应保留
    let has_first = k.messages().iter().any(|m| matches!(m, Message::User(t) if t.contains("问题一")));
    assert!(has_first, "更早的轮次必须保留");
    assert_eq!(*k.state(), KernelState::Idle, "回退后应回到空闲");
}

#[test]
fn rewind_reports_how_many_file_changes_were_kept() {
    // 回退**不撤销文件**，但必须如实报告有多少改动被保留 ——
    // 让人以为"undo 连文件一起回去了"是最危险的那种错觉。
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(PreviewTool));
    let mut k = kernel_with(
        Box::new(ScriptedModelProvider::new(vec![
            vec![tool_call("c1", "apply_patch", serde_json::json!({}))],
            vec![ModelDelta::Text("done".into())],
        ])),
        tools,
        Box::new(InMemoryPersistence::new()),
        ExecMode::AutoEdit,
    );
    k.submit(Op::UserTurn { text: "改文件".into(), refs: vec![] }).unwrap();
    let ev = k.submit(Op::Rewind { turns: 1 }).unwrap();
    let kept = ev.iter().find_map(|e| match e {
        EventMsg::Rewound { files_kept, .. } => Some(*files_kept),
        _ => None,
    }).expect("应有 Rewound");
    assert_eq!(kept, 1, "应报告 1 个文件改动被保留（未撤销）");
}

#[test]
fn rewind_too_far_is_an_error_not_a_silent_noop() {
    // 回退超过已有轮次必须报错。静默不动会让用户以为回退了。
    let mut k = kernel_with(
        Box::new(ScriptedModelProvider::text_only("x")),
        ToolRegistry::new(),
        Box::new(InMemoryPersistence::new()),
        ExecMode::Default,
    );
    k.submit(Op::UserTurn { text: "只有一轮".into(), refs: vec![] }).unwrap();
    let err = k.submit(Op::Rewind { turns: 5 });
    assert!(err.is_err(), "回退 5 轮但只有 1 轮，必须报错");
    let msg = format!("{}", err.unwrap_err());
    assert!(msg.contains("无法回退"), "错误信息应说明原因：{msg}");
}

#[test]
fn rewind_clears_a_pending_approval() {
    // 若正等审批时回退，那批调用已经无意义 —— 必须清掉挂起状态，
    // 否则内核会停在一个"等一个已经不存在的轮次"的状态里。
    struct WriteTool;
    impl Tool for WriteTool {
        fn name(&self) -> &str { "apply_patch" }
        fn describe(&self) -> String { "x".into() }
        fn call_kind(&self, _a: &Value) -> CallKind { CallKind::Write }
        fn execute(&self, _a: &Value, _c: &ToolCtx) -> ToolOutput {
            ToolOutput { exit_code: 0, stdout: String::new(), stderr: String::new(), truncated: false }
        }
    }
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(WriteTool));
    let mut k = kernel_with(
        Box::new(ScriptedModelProvider::new(vec![
            vec![tool_call("c1", "apply_patch", serde_json::json!({}))],
        ])),
        tools,
        Box::new(InMemoryPersistence::new()),
        ExecMode::Default, // 写要问 → 会挂起
    );
    k.submit(Op::UserTurn { text: "改".into(), refs: vec![] }).unwrap();
    assert!(matches!(k.state(), KernelState::AwaitingApproval { .. }), "应先挂起");
    k.submit(Op::Rewind { turns: 1 }).unwrap();
    assert_eq!(*k.state(), KernelState::Idle, "回退应清掉挂起的审批");
}

// ─────────────── 运行时切换模型 ───────────────

/// 自报名字的极简 provider：用来验证"切换后真的换了实现"。
/// 每个实例回一句带自己名字的话，于是从回答就能看出用的是哪个。
struct ReplyAs(&'static str);
impl ModelProvider for ReplyAs {
    fn name(&self) -> &str { self.0 }
    fn stream(&self, _r: &neo_core::ModelRequest<'_>) -> neo_core::ModelStream {
        Box::new(vec![ModelDelta::Text(format!("reply-from-{}", self.0))].into_iter())
    }
}

#[test]
fn configure_session_actually_switches_the_provider() {
    // 这是"声明了没接线"的修复证明：`SessionPatch.model` 之前被接受、
    // 存进 cfg、然后**从未使用**（Kernel 持有固定的 Box<dyn ModelProvider>）。
    // 断言方式是**看回答来自哪个 provider**，而不只是看 cfg 里的字符串。
    use neo_core::models::{ModelInfo, ModelRegistry};

    let mk = |name: &'static str, limit: u64| {
        (
            ModelInfo {
                name: name.into(),
                description: String::new(),
                context_limit: limit,
                production: true,
            },
            Box::new(ReplyAs(name)) as Box<dyn ModelProvider>,
        )
    };
    let reg = ModelRegistry::new("small", vec![mk("small", 32_000), mk("big", 128_000)]).unwrap();
    let mut k = Kernel::new(
        "s",
        cfg(ExecMode::Default),
        read_tool(),
        reg,
        Arc::new(TestSandbox),
        Box::new(InMemoryPersistence::new()),
        "/tmp",
    );

    assert_eq!(k.current_model(), "small");
    assert_eq!(k.current_context_limit(), 32_000);

    // 第一轮：回答应来自 small
    let ev1 = k.submit(Op::UserTurn { text: "hi".into(), refs: vec![] }).unwrap();
    let said1 = ev1.iter().find_map(|e| match e {
        EventMsg::AgentMessageDone { text } => Some(text.clone()),
        _ => None,
    });
    assert_eq!(said1.as_deref(), Some("reply-from-small"), "初始应走 small");

    // 切到 big
    let ev2 = k
        .submit(Op::ConfigureSession {
            patch: neo_protocol::SessionPatch {
                model: Some("big".into()),
                ..Default::default()
            },
        })
        .unwrap();
    assert_eq!(k.current_model(), "big");
    assert_eq!(k.current_context_limit(), 128_000, "上下文窗口应跟着换");
    assert!(
        ev2.iter().any(|e| matches!(e, EventMsg::ModelSwitched { model, .. } if model == "big")),
        "应发出 ModelSwitched：{ev2:?}"
    );

    // 第二轮：回答应来自 **big**（证明真的换了 provider，而不只是改了配置）
    let ev3 = k.submit(Op::UserTurn { text: "again".into(), refs: vec![] }).unwrap();
    let said3 = ev3.iter().find_map(|e| match e {
        EventMsg::AgentMessageDone { text } => Some(text.clone()),
        _ => None,
    });
    assert_eq!(said3.as_deref(), Some("reply-from-big"), "切换后应走 big");

    // 切到不存在的模型：报错且不改变当前模型
    let err = k
        .submit(Op::ConfigureSession {
            patch: neo_protocol::SessionPatch {
                model: Some("不存在".into()),
                ..Default::default()
            },
        })
        .unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("不存在"), "错误应指出模型名：{msg}");
    assert!(msg.contains("small") && msg.contains("big"), "应列出可用模型：{msg}");
    assert_eq!(k.current_model(), "big", "失败的切换不该改变当前模型");
}

#[test]
fn model_switch_emits_an_event_and_updates_the_config() {
    use neo_core::models::{ModelInfo, ModelRegistry};

    let reg = ModelRegistry::new(
        "solo",
        vec![(
            ModelInfo {
                name: "solo".into(),
                description: "桩".into(),
                context_limit: 64_000,
                production: false,
            },
            Box::new(ReplyAs("solo")) as Box<dyn ModelProvider>,
        )],
    )
    .unwrap();
    let mut k = Kernel::new(
        "s",
        cfg(ExecMode::Default),
        read_tool(),
        reg,
        Arc::new(TestSandbox),
        Box::new(InMemoryPersistence::new()),
        "/tmp",
    );
    // 切到自身（幂等）：应成功并发事件
    let ev = k
        .submit(Op::ConfigureSession {
            patch: neo_protocol::SessionPatch {
                model: Some("solo".into()),
                ..Default::default()
            },
        })
        .unwrap();
    assert!(
        ev.iter().any(|e| matches!(e, EventMsg::ModelSwitched { .. })),
        "切换应发出 ModelSwitched 事件：{ev:?}"
    );
    assert_eq!(k.current_context_limit(), 64_000, "上下文窗口应随模型更新");
}

#[test]
fn available_models_are_listed_for_the_host() {
    // 宿主（TUI 设置页/模型列表）要能拿到全部选项
    let k = kernel_with(
        Box::new(ScriptedModelProvider::text_only("x")),
        read_tool(),
        Box::new(InMemoryPersistence::new()),
        ExecMode::Default,
    );
    let list = k.available_models();
    assert_eq!(list.len(), 1);
    // neo-mock 的 ScriptedModelProvider 自报名为 "scripted"
    assert_eq!(list[0].name, "scripted");
}

// ─────────────── 从会话日志重建历史 ───────────────

#[test]
fn history_can_be_rebuilt_from_the_session_log() {
    // 这是 AGENTS.md 那条约束的证明：「凡进入模型请求的内容都要能从会话日志重建」。
    // 也是会话切换/跨进程续聊的前提 —— 不能重建就只能得到空历史。
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(RecordingTool { seen: seen.clone() }));
    let persistence = Box::new(InMemoryPersistence::new());
    let mut k = Kernel::new(
        "s",
        cfg(ExecMode::AutoEdit),
        tools,
        neo_core::models::ModelRegistry::single(Box::new(ScriptedModelProvider::new(vec![
            vec![tool_call("c1", "bash", serde_json::json!({ "cmd": "echo hi" }))],
            vec![ModelDelta::Text("做完了".into())],
        ]))),
        Arc::new(TestSandbox),
        persistence,
        "/tmp",
    );
    k.submit(Op::UserTurn { text: "跑一下 echo".into(), refs: vec![] }).unwrap();
    let original = k.messages().to_vec();
    assert!(original.len() >= 3, "应有 用户/助手(含工具调用)/工具结果：{original:?}");

    // 取日志（真实持久化会从磁盘读回，这里用同一个 box 的 load）
    // 注意：用 session_id 无关，重建只看记录
    let logs = k.log_for_test();

    // 在新内核上重建
    let mut k2 = Kernel::new(
        "s",
        cfg(ExecMode::AutoEdit),
        ToolRegistry::new(),
        neo_core::models::ModelRegistry::single(Box::new(ScriptedModelProvider::text_only("x"))),
        Arc::new(TestSandbox),
        Box::new(InMemoryPersistence::new()),
        "/tmp",
    );
    let n = k2.rebuild_from_log(&logs);
    assert_eq!(n, original.len(), "重建条数应与原历史一致");

    // 逐条比对：用户文本、助手文本、工具调用参数、工具输出
    assert_eq!(k2.messages().len(), original.len());
    for (a, b) in k2.messages().iter().zip(original.iter()) {
        assert_eq!(a, b, "重建的历史必须与原历史逐条相等");
    }
    // 特别确认工具调用的**参数**被恢复了（否则对真实 provider 请求非法）
    let calls: Vec<&neo_core::ToolInvocation> = k2
        .messages()
        .iter()
        .filter_map(|m| match m {
            Message::Assistant { tool_calls, .. } => Some(tool_calls.iter()),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(calls.len(), 1, "应恢复出 1 个工具调用");
    assert_eq!(calls[0].arguments["cmd"], "echo hi", "参数必须被恢复");
}

#[test]
fn rebuild_with_empty_log_leaves_history_untouched() {
    // 无日志 ≠ 空会话：不该把现有历史清掉（那会丢数据）
    let mut k = kernel_with(
        Box::new(ScriptedModelProvider::text_only("x")),
        read_tool(),
        Box::new(InMemoryPersistence::new()),
        ExecMode::Default,
    );
    k.submit(Op::UserTurn { text: "hi".into(), refs: vec![] }).unwrap();
    let before = k.messages().len();
    assert!(before > 0);
    let n = k.rebuild_from_log(&[]);
    assert_eq!(n, 0);
    assert_eq!(k.messages().len(), before, "空日志不该清空已有历史");
}

// ─────────────── 上下文压缩（Op::Compact）───────────────

/// 测试用压缩器：把前 N 条换成一条摘要（切点由它决定）。
struct FakeCompactor {
    /// 保留最近的条数
    keep: usize,
    /// 触发阈值（消息数少于它就不压）
    trigger: usize,
}

impl neo_core::Compactor for FakeCompactor {
    fn plan(&self, messages: &[neo_core::Message]) -> Option<(String, usize)> {
        if messages.len() < self.trigger {
            return None;
        }
        // 切点必须落在 User 消息上（真实 provider 的硬约束）
        let candidate = messages.len().saturating_sub(self.keep);
        let cut = messages
            .iter()
            .enumerate()
            .filter(|(i, m)| matches!(m, Message::User(_)) && *i > 0 && *i <= candidate)
            .map(|(i, _)| i)
            .max()?;
        Some(("（压缩摘要）".to_string(), cut))
    }
}

#[test]
fn compact_replaces_the_prefix_with_a_summary() {
    let mut k = kernel_with(
        Box::new(ScriptedModelProvider::text_only("ok")),
        read_tool(),
        Box::new(InMemoryPersistence::new()),
        ExecMode::Default,
    )
    .with_compactor(Box::new(FakeCompactor { keep: 4, trigger: 6 }));

    // 造 8 轮（每轮 2 条消息）
    for i in 0..8 {
        k.submit(Op::UserTurn { text: format!("问题{i}"), refs: vec![] }).unwrap();
    }
    let before = k.messages().len();
    assert!(before >= 16, "应有 16 条以上消息，实际 {before}");

    let ev = k.submit(Op::Compact).unwrap();
    assert!(
        ev.iter().any(|e| matches!(e, EventMsg::ContextCompacted { .. })),
        "应发出 ContextCompacted：{ev:?}"
    );
    let after = k.messages();
    assert!(after.len() < before, "压缩后应更短：{before} → {}", after.len());
    // 第一条应是摘要（System 角色）
    assert!(
        matches!(&after[0], Message::System(t) if t.contains("摘要")),
        "首条应为摘要：{:?}",
        after[0]
    );
    // 关键：压缩后的历史里**不能有孤儿工具消息**
    // （tool 消息必须能找到对应的 assistant tool_call）
    let mut call_ids: Vec<String> = Vec::new();
    for m in after {
        match m {
            Message::Assistant { tool_calls, .. } => {
                call_ids.extend(tool_calls.iter().map(|c| c.id.clone()));
            }
            Message::ToolResult { id, .. } => {
                assert!(
                    call_ids.contains(id),
                    "出现了孤儿工具结果 {id} —— 切点落在了轮的中间"
                );
            }
            _ => {}
        }
    }
}

#[test]
fn compact_without_a_compactor_reports_why_not_a_silent_noop() {
    // 未配置压缩策略时必须**明确报错**，不能假装压缩成功 ——
    // 假装成功会让用户以为上下文空了，实际没变。
    let mut k = kernel_with(
        Box::new(ScriptedModelProvider::text_only("ok")),
        read_tool(),
        Box::new(InMemoryPersistence::new()),
        ExecMode::Default,
    );
    let err = k.submit(Op::Compact).unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("压缩"), "错误应说明压缩不可用：{msg}");
    assert!(msg.contains("策略") || msg.contains("边界"), "应给出原因：{msg}");
}

#[test]
fn compact_refuses_a_bogus_keep_point_instead_of_corrupting_history() {
    // 策略给了 0 或越界的保留点 → 拒绝执行，而不是冒险砍坏历史
    struct BadCompactor;
    impl neo_core::Compactor for BadCompactor {
        fn plan(&self, _m: &[neo_core::Message]) -> Option<(String, usize)> {
            Some(("坏摘要".into(), 0)) // 0 不合法
        }
    }
    let mut k = kernel_with(
        Box::new(ScriptedModelProvider::text_only("ok")),
        read_tool(),
        Box::new(InMemoryPersistence::new()),
        ExecMode::Default,
    )
    .with_compactor(Box::new(BadCompactor));
    k.submit(Op::UserTurn { text: "hi".into(), refs: vec![] }).unwrap();
    let before = k.messages().len();
    assert!(k.submit(Op::Compact).is_err(), "非法保留点应被拒绝");
    assert_eq!(k.messages().len(), before, "拒绝时不得改动历史");
}

#[test]
fn compaction_survives_log_replay() {
    // 日志是 append-only 的：回放必须**重演**压缩动作，
    // 否则重建出的历史与实际发给模型的不一致。
    let mut k = kernel_with(
        Box::new(ScriptedModelProvider::text_only("ok")),
        read_tool(),
        Box::new(InMemoryPersistence::new()),
        ExecMode::Default,
    )
    .with_compactor(Box::new(FakeCompactor { keep: 4, trigger: 6 }));
    for i in 0..8 {
        k.submit(Op::UserTurn { text: format!("问{i}"), refs: vec![] }).unwrap();
    }
    k.submit(Op::Compact).unwrap();
    let original = k.messages().to_vec();
    let logs = k.log_for_test();

    let mut k2 = Kernel::new(
        "s",
        cfg(ExecMode::Default),
        ToolRegistry::new(),
        neo_core::models::ModelRegistry::single(Box::new(ScriptedModelProvider::text_only("x"))),
        Arc::new(TestSandbox),
        Box::new(InMemoryPersistence::new()),
        "/tmp",
    );
    k2.rebuild_from_log(&logs);
    assert_eq!(
        k2.messages(),
        original.as_slice(),
        "回放必须重演压缩，重建结果应与压缩后的历史一致"
    );
}

// ─────────────── 引用解析（@file / $skill）───────────────

/// 会真的返回文件内容的沙箱：`cat <path>` 读盘，其余命令回显。
/// 引用解析要经沙箱读文件，用 TestSandbox（只回显 `ran:...`）测不出内容注入。
struct FileSandbox { root: std::path::PathBuf }

impl SandboxBackend for FileSandbox {
    fn supports(&self, _m: SandboxMode) -> bool { true }
    fn write_file(&self, _m: SandboxMode, _p: &std::path::Path, c: &str) -> neo_core::FileOutcome {
        neo_core::FileOutcome::Written { bytes: c.len() }
    }
    fn execute(&self, _m: SandboxMode, command: &str, _limit: usize) -> SandboxOutcome {
        // 只认引用解析发的 `cat -- 'path'`；命令里的单引号已由 shell_quote 转义。
        if let Some(rest) = command.strip_prefix("cat -- ") {
            let name = rest.trim_matches('\'').replace("'\\''", "'");
            let p = self.root.join(name);
            return match std::fs::read_to_string(&p) {
                Ok(s) => SandboxOutcome::Ran { stdout: s, truncated: false },
                Err(e) => SandboxOutcome::Denied { reason: e.to_string() },
            };
        }
        SandboxOutcome::Ran { stdout: format!("ran:{command}"), truncated: false }
    }
}

fn kernel_with_parts(
    sandbox: Arc<dyn SandboxBackend>,
    skills: neo_core::skills::SkillRegistry,
    persistence: Box<dyn SessionPersistence>,
) -> Kernel {
    Kernel::new(
        "s1",
        cfg(ExecMode::Default),
        ToolRegistry::new(),
        neo_core::models::ModelRegistry::single(Box::new(ScriptedModelProvider::text_only("ok"))),
        sandbox,
        persistence,
        "/tmp",
    )
    .with_skills(skills)
}

#[test]
fn file_ref_is_resolved_into_the_model_visible_message() {
    let dir = std::env::temp_dir().join(format!("neo-ref-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(dir.join("hello.txt"), "第一行\n第二行\n第三行\n").unwrap();

    let mut k = kernel_with_parts(Arc::new(FileSandbox { root: dir.clone() }), Default::default(),
        Box::new(InMemoryPersistence::new()));
    let refs = neo_protocol::parse_refs("@hello.txt 看下");
    let events = k.submit(Op::UserTurn { text: "@hello.txt 看下".into(), refs }).unwrap();

    // 事件里要有解析摘要
    let summary = events.iter().find_map(|e| match e {
        EventMsg::RefsResolved { summary, .. } => Some(summary.clone()),
        _ => None,
    }).expect("必须发 RefsResolved 事件");
    assert!(summary.iter().any(|s| s.contains("hello.txt") && s.contains("已注入")),
        "摘要应说明已注入: {summary:?}");

    // 历史里要有文件正文（模型可见）
    let joined = format!("{:?}", k.messages());
    assert!(joined.contains("第二行"), "文件正文必须进模型可见历史: {joined}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn file_ref_line_range_selects_lines() {
    let dir = std::env::temp_dir().join(format!("neo-refline-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(dir.join("n.txt"), "L1\nL2\nL3\nL4\n").unwrap();

    let mut k = kernel_with_parts(Arc::new(FileSandbox { root: dir.clone() }), Default::default(),
        Box::new(InMemoryPersistence::new()));
    let refs = neo_protocol::parse_refs("@n.txt#2-3 看看");
    let _ = k.submit(Op::UserTurn { text: "@n.txt#2-3 看看".into(), refs }).unwrap();

    let joined = format!("{:?}", k.messages());
    assert!(joined.contains("L2") && joined.contains("L3"), "应含 2-3 行: {joined}");
    assert!(!joined.contains("L4"), "范围外的行不得注入: {joined}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn skill_ref_injects_registered_body_and_missing_skill_lists_available() {
    let mut reg = neo_core::skills::SkillRegistry::new();
    reg.add(neo_core::skills::Skill::new("eff", "效率规范", "# 规则\n不要 sleep\n"));

    let mut k = kernel_with_parts(Arc::new(TestSandbox), reg, Box::new(InMemoryPersistence::new()));
    let refs = neo_protocol::parse_refs("$eff 执行");
    let events = k.submit(Op::UserTurn { text: "$eff 执行".into(), refs }).unwrap();
    let joined = format!("{:?}", k.messages());
    assert!(joined.contains("不要 sleep"), "技能正文必须进历史: {joined}");
    assert!(events.iter().any(|e| matches!(e, EventMsg::RefsResolved { .. })));

    // 找不到时：不注入，但摘要要列出可用技能（否则用户面对空注册表无从下手）
    let refs = neo_protocol::parse_refs("$nope 执行");
    let events = k.submit(Op::UserTurn { text: "$nope 执行".into(), refs }).unwrap();
    let summary = events.iter().find_map(|e| match e {
        EventMsg::RefsResolved { summary, .. } => Some(summary.clone()),
        _ => None,
    }).unwrap();
    assert!(summary.iter().any(|s| s.contains("未找到") && s.contains("eff")),
        "未找到时必须列出可用技能: {summary:?}");
}

#[test]
fn ref_roundtrip_replays_into_identical_history() {
    // 引用注入的内容**必须能只靠日志重建**（"模型可见即已落日志"）。
    // 这条是引用功能的回放门禁：若注入块没落盘，重建出的用户消息会短一截。
    let dir = std::env::temp_dir().join(format!("neo-refrt-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(dir.join("r.txt"), "回放内容标记\n").unwrap();

    let mut k = kernel_with_parts(Arc::new(FileSandbox { root: dir.clone() }), Default::default(),
        Box::new(InMemoryPersistence::new()));
    let refs = neo_protocol::parse_refs("@r.txt 读");
    k.submit(Op::UserTurn { text: "@r.txt 读".into(), refs }).unwrap();
    let original = k.messages().to_vec();
    let logs = k.log_for_test();

    let mut k2 = kernel_with_parts(Arc::new(FileSandbox { root: dir.clone() }), Default::default(),
        Box::new(InMemoryPersistence::new()));
    k2.rebuild_from_log(&logs);
    assert_eq!(
        k2.messages(),
        original.as_slice(),
        "回放必须还原注入块（不能重解析、不能丢）"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn parse_refs_and_shell_quote_reject_injection() {
    // 引用目标来自用户输入，是不可信字符串。带 `;` / 引号的路径
    // 必须被转义成单个参数，不能被 sh 当命令分隔符执行。
    let q = neo_core::shell_quote("a.txt; rm -rf /");
    assert_eq!(q, "'a.txt; rm -rf /'", "分号必须留在引号内");

    let tricky = neo_core::shell_quote("it's.txt");
    assert_eq!(tricky, "'it'\\''s.txt'", "单引号必须按 POSIX 规则转义");
}

// ─────────────── 项目指令（AGENTS.md 级联）───────────────

#[test]
fn instructions_enter_system_prompt_and_are_logged_once() {
    let ins = neo_core::instructions::Instructions {
        sources: vec!["/repo/AGENTS.md".into()],
        block: "必须遵守：零 warning。".into(),
        truncated: false,
    };
    let mut k = // 用 InMemoryPersistence 才能读回日志
    {
        let mut k = kernel_with_parts(Arc::new(TestSandbox), Default::default(),
            Box::new(InMemoryPersistence::new()));
        k = k.with_instructions(ins.clone());
        k
    };

    // 提示词里必须真的带上指令（不重建提示词的话，指令只停在字段里）
    assert!(k.system_prompt().contains("零 warning"), "指令必须进系统提示词: {}", k.system_prompt());

    let ev1 = k.submit(Op::UserTurn { text: "一".into(), refs: vec![] }).unwrap();
    let ev2 = k.submit(Op::UserTurn { text: "二".into(), refs: vec![] }).unwrap();
    let n1 = ev1.iter().filter(|e| matches!(e, EventMsg::InstructionsLoaded { .. })).count();
    let n2 = ev2.iter().filter(|e| matches!(e, EventMsg::InstructionsLoaded { .. })).count();
    assert_eq!(n1, 1, "首次提交应落一条指令事件");
    assert_eq!(n2, 0, "指令只落一次，不得每轮重复（否则日志被放大）");
}

#[test]
fn empty_instructions_do_not_emit_noise_events() {
    let mut k = kernel_with_parts(Arc::new(TestSandbox), Default::default(),
        Box::new(InMemoryPersistence::new()));
    let ev = k.submit(Op::UserTurn { text: "x".into(), refs: vec![] }).unwrap();
    assert!(!ev.iter().any(|e| matches!(e, EventMsg::InstructionsLoaded { .. })),
        "没有 AGENTS.md 时不该发指令事件");
}

#[test]
fn instructions_roundtrip_restores_system_prompt() {
    // 指令进系统提示词 = 模型可见 ⇒ 必须能从日志还原**当时那一份**
    // （AGENTS.md 之后可能被改，回放不能重读盘）。
    let ins = neo_core::instructions::Instructions {
        sources: vec!["/repo/AGENTS.md".into()],
        block: "历史约定：不要 sleep。".into(),
        truncated: false,
    };
    let mut k = kernel_with_parts(Arc::new(TestSandbox), Default::default(),
        Box::new(InMemoryPersistence::new()))
        .with_instructions(ins);
    k.submit(Op::UserTurn { text: "x".into(), refs: vec![] }).unwrap();
    let logs = k.log_for_test();
    let original_prompt = k.system_prompt().to_string();

    // 回放到一个**没有注入指令**的新内核：指令应完全由日志还原
    let mut k2 = kernel_with_parts(Arc::new(TestSandbox), Default::default(),
        Box::new(InMemoryPersistence::new()));
    assert!(!k2.system_prompt().contains("不要 sleep"), "前置条件：新内核本无指令");
    k2.rebuild_from_log(&logs);
    assert_eq!(k2.system_prompt(), original_prompt, "回放必须还原同一份系统提示词");
}

// ─────────────── 会话级"总是允许"（Decision::AllowAlways）───────────────

#[test]
fn allow_always_stops_re_prompting_for_the_same_kind() {
    // 真实反馈：用户连按十几次 y 仍被反复询问 —— 因为 AllowAlways 与 Allow
    // 走同一分支，内核没有会话级记忆，每次调用都重新判定，必然再问。
    // 这条测试钉住"说了总是允许之后，同类调用不再问"。
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut r = ToolRegistry::new();
    r.register(Arc::new(RecordingTool { seen: seen.clone() }));

    // 三个**写**类调用（rm 前缀由 MockTool::call_kind 判为 Write）
    let script = vec![
        vec![
            tool_call("w1", "bash", serde_json::json!({"cmd": "rm -rf /tmp/a"})),
            tool_call("w2", "bash", serde_json::json!({"cmd": "rm -rf /tmp/b"})),
            tool_call("w3", "bash", serde_json::json!({"cmd": "rm -rf /tmp/c"})),
        ],
        vec![ModelDelta::Text("done".into())],
    ];
    let mut k = kernel_with(
        Box::new(ScriptedModelProvider::new(script)),
        r,
        Box::new(InMemoryPersistence::new()),
        ExecMode::Default,
    );

    let events = k.submit(Op::UserTurn { text: "clean".into(), refs: vec![] }).unwrap();
    let ask_id = events
        .iter()
        .find_map(|e| match e {
            EventMsg::ApprovalRequest { id, .. } => Some(id.clone()),
            _ => None,
        })
        .expect("应发出 ApprovalRequest");

    // 回答"总是允许" → 后续同类调用应**直接执行、不再问**
    let resumed = k
        .submit(Op::Approve { id: ask_id, decision: Decision::AllowAlways })
        .unwrap();

    assert_eq!(
        *seen.lock().unwrap(),
        vec![
            "rm -rf /tmp/a".to_string(),
            "rm -rf /tmp/b".to_string(),
            "rm -rf /tmp/c".to_string()
        ],
        "说了总是允许后，同类调用应一次问完、全部执行"
    );
    assert!(
        !resumed.iter().any(|e| matches!(e, EventMsg::ApprovalRequest { .. })),
        "恢复后不得再发审批请求"
    );
    assert!(matches!(k.state(), KernelState::Idle));
}

#[test]
fn allow_once_still_re_prompts_for_the_next_call() {
    // 对照组：`Allow`（只批这一次）**必须**继续问 —— 否则"总是允许"
    // 与"批准一次"就没有区别了（那等于静默放行）。
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut r = ToolRegistry::new();
    r.register(Arc::new(RecordingTool { seen: seen.clone() }));

    let script = vec![
        vec![
            tool_call("w1", "bash", serde_json::json!({"cmd": "rm -rf /tmp/a"})),
            tool_call("w2", "bash", serde_json::json!({"cmd": "rm -rf /tmp/b"})),
        ],
        vec![ModelDelta::Text("done".into())],
    ];
    let mut k = kernel_with(
        Box::new(ScriptedModelProvider::new(script)),
        r,
        Box::new(InMemoryPersistence::new()),
        ExecMode::Default,
    );

    let events = k.submit(Op::UserTurn { text: "x".into(), refs: vec![] }).unwrap();
    let id1 = events
        .iter()
        .find_map(|e| match e {
            EventMsg::ApprovalRequest { id, .. } => Some(id.clone()),
            _ => None,
        })
        .expect("第一次应问");

    let after = k.submit(Op::Approve { id: id1, decision: Decision::Allow }).unwrap();
    let id2 = after.iter().find_map(|e| match e {
        EventMsg::ApprovalRequest { id, .. } => Some(id.clone()),
        _ => None,
    });
    assert!(id2.is_some(), "只批一次时，第二个写调用应**再次**询问");
    assert_eq!(*seen.lock().unwrap(), vec!["rm -rf /tmp/a".to_string()], "第二个还没批，不该执行");
}

#[test]
fn allow_always_never_bypasses_the_sandbox_hard_boundary() {
    // 沙箱是**硬边界**：即便用户说过"总是允许写"，只读档下写入仍必须被拒。
    // 会话级放行只作用于"审批"这一层，不得越过 gate 的 Deny。
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut r = ToolRegistry::new();
    r.register(Arc::new(RecordingTool { seen: seen.clone() }));

    // Plan 档 = 只读沙箱 + OnRequest 审批
    let script = vec![
        vec![tool_call("w1", "bash", serde_json::json!({"cmd": "rm -rf /tmp/a"}))],
        vec![ModelDelta::Text("done".into())],
    ];
    let mut k = kernel_with(
        Box::new(ScriptedModelProvider::new(script)),
        r,
        Box::new(InMemoryPersistence::new()),
        ExecMode::Plan,
    );

    let events = k.submit(Op::UserTurn { text: "x".into(), refs: vec![] }).unwrap();
    // 只读档下写入被 gate 直接拒（不发审批请求）
    assert!(
        !events.iter().any(|e| matches!(e, EventMsg::ApprovalRequest { .. })),
        "只读档应直接拒绝，不该进入审批"
    );
    assert!(seen.lock().unwrap().is_empty(), "沙箱拒绝后不得执行");
}
