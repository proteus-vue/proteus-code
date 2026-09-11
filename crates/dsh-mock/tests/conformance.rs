//! SPI conformance 契约测试（SPI-First Step 3）
//!
//! 两条纪律：
//!   1. **同一份用例代码跑所有后端** —— 契约是后端的公共义务，不是某一个的行为。
//!   2. **至少一个负向用例** —— 一个"坏后端"必须被抓出来，否则套件没有牙齿。
//!
//! 测的是**语义契约**（幂等/顺序/错误语义/能力边界），不是实现细节。

use dsh_core::{
    CallKind, HostBackend, ModelProvider, SandboxBackend, SandboxOutcome, SessionPersistence, Tool,
    ToolCtx,
};
use dsh_protocol::EventMsg;
use dsh_host_desktop::DesktopHost;
use dsh_mock::{
    BrittleHost, InMemoryPersistence, LeakySandbox, MockHost, MockModelProvider, MockTool,
    NamelessTool, NoopSandbox, ScriptedModelProvider, TamperingPersistence,
};
use std::sync::Arc;
use dsh_protocol::SandboxMode;
use serde_json::json;

/// 一段共享的事件流：所有宿主后端都必须能完整消费。
fn shared_event_stream() -> Vec<EventMsg> {
    vec![
        EventMsg::TurnStarted { turn_id: "t1".into() },
        EventMsg::AgentMessageDone { text: "hello".into() },
        EventMsg::ToolCallBegin { id: "c1".into(), name: "bash".into() },
        EventMsg::ToolCallEnd { id: "c1".into(), exit_code: 0 },
        EventMsg::TurnComplete { input_tokens: 1, output_tokens: 2 },
    ]
}

/// 契约：任何宿主后端都必须能消费完整事件流，且事实数一致。
///
/// 注意断言的是 **`Fact`**（协议层语义），不是渲染字符串 ——
/// TUI 画彩色、exec 打日志、desktop 渲染卡片，但三者必须传达同一组事实。
fn assert_host_contract(mut host: Box<dyn HostBackend>) {
    for ev in shared_event_stream() {
        host.consume(&ev)
            .unwrap_or_else(|e| panic!("host {} 未能消费事件: {e}", host.id()));
    }
    let facts = host.facts();
    assert_eq!(
        facts.len(),
        3,
        "host {} 的事实数不符（应为 助手发言/工具结束/本轮结束 三条），实际 {facts:?}",
        host.id()
    );
}

#[test]
fn host_contract_holds_for_every_backend() {
    // 同一份契约，跑多个真实/无头后端 —— 这就是"可替换"被验证的方式。
    assert_host_contract(Box::new(MockHost::new("headless")));
    assert_host_contract(Box::new(DesktopHost::new()));
    assert_host_contract(Box::new(dsh_host_tui::TuiFacts::new()));
    assert_host_contract(Box::new(dsh_host_web::WebFacts::new()));
}

#[test]
fn host_contract_compares_two_backends_on_the_same_stream() {
    // T6 运行时形态：同一事件流广播给多个宿主，断言**语义等价**。
    let mut a = MockHost::new("headless");
    let mut b = dsh_host_tui::TuiFacts::new();
    let mut c = DesktopHost::new();
    let mut d = dsh_host_web::WebFacts::new();
    for ev in shared_event_stream() {
        a.consume(&ev).unwrap();
        b.consume(&ev).unwrap();
        c.consume(&ev).unwrap();
        d.consume(&ev).unwrap();
    }
    assert_eq!(a.facts(), b.facts(), "headless 与 TUI 的事实必须等价");
    assert_eq!(b.facts(), c.facts(), "TUI 与 desktop 的事实必须等价");
    assert_eq!(c.facts(), d.facts(), "desktop 与 web 的事实必须等价");
}

/// 负向用例：坏宿主必须被契约抓住。
#[test]
fn host_contract_rejects_a_brittle_backend() {
    let mut bad = BrittleHost;
    assert!(bad.consume(&EventMsg::TurnStarted { turn_id: "t".into() }).is_ok(), "正常事件不该失败");
    let rejected = bad.consume(&EventMsg::GoalProgress { goal_id: "g".into(), done: 1, total: 2 });
    assert!(rejected.is_err(), "负向用例失败：坏宿主未被识别 —— 套件没有牙齿");
}

// ─────────────── SessionPersistence：append-only ───────────────

/// 契约：写入什么、读回什么，顺序与内容都不变。
fn assert_persistence_contract(mut p: Box<dyn SessionPersistence>) {
    let payloads = [
        serde_json::json!({"a": 1}),
        serde_json::json!({"b": 2}),
        serde_json::json!({"c": 3}),
    ];
    for v in &payloads {
        p.append("event", v.clone()).expect("append 不应失败");
    }
    let loaded = p.load().expect("load 不应失败");
    assert_eq!(loaded.len(), payloads.len(), "append-only 被破坏：条数不一致");
    for (i, r) in loaded.iter().enumerate() {
        assert_eq!(r.payload, payloads[i], "第 {i} 条内容被改写");
        assert_eq!(r.seq, (i + 1) as u64, "seq 必须从 1 严格递增");
    }
}

#[test]
fn persistence_contract_holds_for_memory_backend() {
    assert_persistence_contract(Box::new(InMemoryPersistence::new()));
}

/// 负向用例：会静默改写内容的持久化必须被抓出来。
#[test]
fn persistence_contract_catches_tampering_backend() {
    // 真正的契约断言：坏后端无法通过上面那条**通用**检查。
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        assert_persistence_contract(Box::new(TamperingPersistence::new()));
    }));
    assert!(
        caught.is_err(),
        "负向用例失败：破坏 append-only 的实现未被 conformance 抓住 —— 套件没有牙齿"
    );
}

// ─────────────── SandboxBackend：能力边界 ───────────────

/// 契约：声称不支持的模式，必须**显式拒绝**，不得静默放行。
fn assert_sandbox_contract(s: &dyn SandboxBackend, mode: SandboxMode) {
    if !s.supports(mode) {
        match s.execute(mode, "rm -rf /", 64 * 1024) {
            SandboxOutcome::Denied { .. } => {}
            SandboxOutcome::Ran { .. } => panic!(
                "契约违反：后端声明不支持 {mode:?}，却仍然执行了命令 —— 安全边界形同虚设"
            ),
        }
    }
}

#[test]
fn sandbox_contract_holds_for_noop_backend() {
    let s = NoopSandbox;
    assert!(s.supports(SandboxMode::DangerFullAccess), "noop 应支持全权限档");
    assert!(!s.supports(SandboxMode::ReadOnly), "noop 不应声称支持 read-only");
    // 声称不支持的档位必须拒绝
    assert_sandbox_contract(&s, SandboxMode::ReadOnly);
    // 声称支持的档位必须真的执行
    assert!(matches!(
        s.execute(SandboxMode::DangerFullAccess, "echo hi", 64 * 1024),
        SandboxOutcome::Ran { .. }
    ));
}

/// 负向用例：声称支持 read-only 却照样执行的沙箱是最危险的实现，必须被抓住。
#[test]
fn sandbox_contract_catches_leaky_backend() {
    let leaky = LeakySandbox;
    assert!(leaky.supports(SandboxMode::ReadOnly), "它确实**声称**支持");
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // 这个后端 supports() 返回 true，所以走不到 Denied 分支；
        // 下面直接断言"安全边界"这一语义：read-only 下执行必须被拒。
        match leaky.execute(SandboxMode::ReadOnly, "rm -rf /", 64 * 1024) {
            SandboxOutcome::Denied { .. } => {}
            SandboxOutcome::Ran { .. } => panic!("契约违反：read-only 档位竟然执行了删除命令"),
        }
    }));
    assert!(caught.is_err(), "负向用例失败：泄漏的沙箱未被抓住");
}

// ─────────────── ModelProvider：确定性（T2 可回放前提）───────────────

/// 契约：同一输入必得同一输出（否则 Op→Event 回放不确定）。
fn assert_model_contract(m: &dyn ModelProvider) {
    use dsh_core::ModelRequest;
    let messages: Vec<dsh_core::Message> = Vec::new();
    let tools: Vec<dsh_core::ToolSchema> = Vec::new();
    let req = ModelRequest { system: "sys", messages: &messages, tools: &tools };
    let a: Vec<_> = m.stream(&req).collect();
    let b: Vec<_> = m.stream(&req).collect();
    assert_eq!(a, b, "模型后端不确定：同请求得到不同增量序列，破坏 T2 可回放性");
    assert!(!m.name().is_empty(), "后端必须有名字（用于错误定位）");
}

#[test]
fn model_contract_holds_for_every_backend() {
    // 同一份契约，跑两个**行为不同**的后端。
    assert_model_contract(&MockModelProvider);
    assert_model_contract(&ScriptedModelProvider::text_only("canned"));
}

// ─────────────── Tool：契约（名/描述/健壮性）───────────────

/// 契约：工具必须可被模型寻址（名非空）、可被模型理解（描述非空）、
/// 且对畸形参数**不得 panic**（注册表会把它当基础设施错误，但不应崩溃进程）。
fn assert_tool_contract(tool: &dyn Tool) {
    assert!(!tool.name().is_empty(), "工具名为空 —— 模型无法调用它");
    assert!(!tool.describe().is_empty(), "工具描述为空 —— 模型不知道何时用它");
    // 分类必须可判定（不 panic，且是合法类别）
    let kind = tool.call_kind(&json!({"unexpected": [1, 2, 3]}));
    assert!(matches!(kind, CallKind::Read | CallKind::Write | CallKind::Network | CallKind::Interactive),
        "call_kind 必须返回合法类别");
    // 畸形参数：不得 panic（工具需自担参数校验）
    let sandbox = Arc::new(NoopSandbox);
    let ctx = ToolCtx {
        sandbox: sandbox.as_ref(),
        mode: dsh_protocol::SandboxMode::DangerFullAccess,
        cwd: std::path::Path::new("/tmp"),
        max_output_bytes: 4096,
    };
    let _ = tool.execute(&json!({"unexpected": [1, 2, 3]}), &ctx);
    let _ = tool.execute(&json!(null), &ctx);
}

#[test]
fn tool_contract_holds_for_mock_tool() {
    assert_tool_contract(&MockTool::new("bash"));
    assert_tool_contract(&MockTool::new("apply_patch"));
}

/// 负向用例：空名工具必须被契约抓住。
#[test]
fn tool_contract_catches_nameless_tool() {
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        assert_tool_contract(&NamelessTool);
    }));
    assert!(caught.is_err(), "负向用例失败：空名工具未被抓住");
}
