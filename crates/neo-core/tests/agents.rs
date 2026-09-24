//! 子代理工厂/工具的单元测试。
//!
//! 放在集成测试而不是 lib 内嵌 `#[cfg(test)]`:neo-mock 是 neo-core 的
//! dev-dependency,又依赖 neo-core —— 内嵌单元测试会让 neo_core 编译
//! 两份(cfg(test) 与非 test),两边的类型互不相同(编译器报
//! "compiled multiple times")。集成测试只链接一份,没有这个问题。

use neo_core::{
    agents::{AgentFactory, AgentSpec, AgentTool, MultiAgentKit},
    CallKind, ModelDelta, ModelProvider, SandboxBackend, SandboxOutcome, Tool, ToolCtx,
    ToolRegistry,
};
use neo_protocol::SandboxMode;
use neo_mock::{MockTool, ScriptedModelProvider};
use std::sync::Arc;

fn factory_with(script: Vec<Vec<ModelDelta>>, tools: ToolRegistry) -> AgentFactory {
    AgentFactory::new(
        Arc::new(ScriptedModelProvider::new(script)) as Arc<dyn ModelProvider>,
        "test-model".into(),
        tools,
        Arc::new(neo_mock::NoopSandbox) as Arc<dyn SandboxBackend>,
        SandboxMode::WorkspaceWrite,
        std::path::Path::new("/tmp"),
        16,
    )
}

fn spec(name: &str, tools: &[&str]) -> AgentSpec {
    AgentSpec {
        name: name.into(),
        model: None,
        tools: tools.iter().map(|s| s.to_string()).collect(),
        body: "你是测试子代理。".into(),
    }
}

fn test_ctx() -> ToolCtx<'static> {
    struct Null;
    impl SandboxBackend for Null {
        fn supports(&self, _m: SandboxMode) -> bool { true }
        fn write_file(&self, _m: SandboxMode, _p: &std::path::Path, c: &str) -> neo_core::FileOutcome {
            neo_core::FileOutcome::Written { bytes: c.len() }
        }
        fn execute(&self, _m: SandboxMode, _c: &str, _l: usize) -> SandboxOutcome {
            SandboxOutcome::Ran { stdout: String::new(), truncated: false }
        }
    }
    let sandbox: &'static Null = Box::leak(Box::new(Null));
    let cwd: &'static std::path::Path = Box::leak(std::path::PathBuf::from("/tmp").into_boxed_path());
    ToolCtx { sandbox, mode: SandboxMode::WorkspaceWrite, cwd, max_output_bytes: 64 * 1024 }
}

#[test]
fn subagent_runs_whitelisted_tool_and_returns_final_reply() {
    // 两步:第一步调白名单内的只读工具,第二步给最终答复
    let script = vec![
        vec![neo_mock::tool_call("r1", "read", serde_json::json!({}))],
        vec![ModelDelta::Text("子代理的最终答复".into())],
    ];
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(MockTool::new("read")));
    let f = factory_with(script, tools);
    let out = f.run(&spec("tester", &["read"]), "帮我查点东西");
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    assert_eq!(out.stdout, "子代理的最终答复", "最终答复应作为工具输出");
}

#[test]
fn whitelisted_unknown_tool_fails_the_run_with_a_clear_error() {
    // 白名单点名了不存在的工具:运行时如实报错,而不是静默启动一个
    // 缺工具的子代理
    let f = factory_with(vec![], ToolRegistry::new());
    let out = f.run(&spec("broken", &["nope"]), "任务");
    assert_eq!(out.exit_code, -1);
    assert!(out.stderr.contains("nope"), "{}", out.stderr);
}

#[test]
fn empty_whitelist_is_a_valid_text_only_agent() {
    // 无工具子代理是合法形态:纯文本助手
    let script = vec![vec![ModelDelta::Text("纯文本答复".into())]];
    let f = factory_with(script, ToolRegistry::new());
    let out = f.run(&spec("plain", &[]), "任务");
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    assert_eq!(out.stdout, "纯文本答复");
}

#[test]
fn agent_tool_name_sanitizes_and_requires_task() {
    let f = factory_with(vec![], ToolRegistry::new());
    let t = AgentTool::new(Arc::new(f), spec("my-agent.v2", &[]));
    assert_eq!(t.name(), "agent_my_agent_v2", "非法字符收敛为下划线");
    let out = t.execute(&serde_json::Value::Null, &test_ctx());
    assert_eq!(out.exit_code, -1);
    assert!(out.stderr.contains("缺少参数 task"));
}

#[test]
fn agent_tool_is_conservatively_write_classified() {
    // 白名单里可能有写工具:判定交给审批闸门,保守判写
    let f = factory_with(vec![], ToolRegistry::new());
    let t = AgentTool::new(Arc::new(f), spec("s", &[]));
    assert_eq!(t.call_kind(&serde_json::Value::Null), CallKind::Write);
}

#[test]
fn multi_agent_v1_list_and_spawn() {
    let script = vec![vec![ModelDelta::Text("审完了".into())]];
    let f = Arc::new(factory_with(script, ToolRegistry::new()));
    let kit = Arc::new(MultiAgentKit::new(f, vec![spec("reviewer", &[])]));
    let mut reg = ToolRegistry::new();
    kit.register(&mut reg);

    let list = reg.get("list_agents").expect("应注册 list_agents");
    let out = list.execute(&serde_json::Value::Null, &test_ctx());
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    assert!(out.stdout.contains("multi_agent_v1"));
    assert!(out.stdout.contains("reviewer"));

    let spawn = reg.get("spawn_agent").expect("应注册 spawn_agent");
    let ok = spawn.execute(
        &serde_json::json!({"agent_type": "reviewer", "task": "看一眼"}),
        &test_ctx(),
    );
    assert_eq!(ok.exit_code, 0, "{}", ok.stderr);
    assert_eq!(ok.stdout, "审完了");

    let bad = spawn.execute(
        &serde_json::json!({"agent_type": "nope", "task": "x"}),
        &test_ctx(),
    );
    assert_eq!(bad.exit_code, -1);
    assert!(bad.stderr.contains("nope"), "{}", bad.stderr);
    assert!(bad.stderr.contains("reviewer"), "应列出可用类型：{}", bad.stderr);
}

#[test]
fn multi_agent_kit_with_no_specs_registers_nothing() {
    let f = Arc::new(factory_with(vec![], ToolRegistry::new()));
    let kit = Arc::new(MultiAgentKit::new(f, vec![]));
    let mut reg = ToolRegistry::new();
    kit.register(&mut reg);
    assert!(reg.get("list_agents").is_none());
    assert!(reg.get("spawn_agent").is_none());
}
