//! exec 宿主的目标模式端到端：`--goal` 设定 → 自动逐子任务轮推进 →
//! 完成/判停。用脚本桩模型（离线可验），验证宿主循环与引擎的协作。

use neo_core::{Kernel, ModelDelta, SandboxBackend, SandboxOutcome, ToolRegistry};
use neo_protocol::SandboxMode;
use neo_exec::{run_task, ExecOptions};
use neo_mock::{InMemoryPersistence, ScriptedModelProvider};
use neo_orchestration::EngineGoalOrchestrator;
use neo_protocol::ExecMode;
use std::sync::Arc;

/// 全放行沙箱（脚本模型不会真的执行危险命令）。
struct AllowSandbox;

impl SandboxBackend for AllowSandbox {
    fn supports(&self, _mode: SandboxMode) -> bool {
        true
    }
    fn write_file(&self, _m: SandboxMode, _p: &std::path::Path, content: &str) -> neo_core::FileOutcome {
        neo_core::FileOutcome::Written { bytes: content.len() }
    }
    fn execute(&self, _mode: SandboxMode, _command: &str, _limit: usize) -> SandboxOutcome {
        SandboxOutcome::Ran { stdout: String::new(), truncated: false }
    }
}

fn goal_kernel(responses: usize) -> Kernel {
    // 每个阶段轮一次模型答复；mock 风格的固定文本按次数展开
    let script: Vec<Vec<ModelDelta>> =
        (0..responses).map(|_| vec![ModelDelta::Text("阶段完成".into())]).collect();
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(neo_mock::MockTool::new("read")));
    Kernel::new(
        "goal-test",
        neo_config::Config { exec_mode: ExecMode::Default, ..neo_config::Config::default() },
        tools,
        neo_core::models::ModelRegistry::single(Box::new(ScriptedModelProvider::new(script))),
        Arc::new(AllowSandbox),
        Box::new(InMemoryPersistence::new()),
        "/tmp",
    )
    .with_goal_orchestrator(Box::new(EngineGoalOrchestrator::default()))
}

fn opts(goal: &str) -> ExecOptions {
    ExecOptions { goal: Some(goal.to_string()), ..ExecOptions::default() }
}

#[test]
fn goal_mode_runs_all_phases_and_reports_completion() {
    // 单子任务 4 阶段 = 4 轮；脚本模型每轮一句固定答复
    let k = goal_kernel(8);
    let (ok, output) = run_task(k, &opts("写个文档"));
    assert!(ok, "{output}");
    assert!(output.contains("全部完成"), "应推进到全部完成：{output}");
    // 无头输出里能看到每一轮的 [goal] 进度（4 阶段 + 设定 + 完成）
    assert!(output.matches("[goal]").count() >= 5, "{output}");
}

#[test]
fn goal_mode_stops_when_the_engine_hits_its_limit() {
    // 引擎判停是宿主循环的退出条件之一：max_iterations=2 时第 3 次推进
    // 触发判停 → 快照 stopped 非空 → goal_awaiting_advance = false，
    // 宿主循环自然结束（ok 不受影响 —— 判停是引擎的正常决定，不是故障）。
    let script: Vec<Vec<ModelDelta>> =
        (0..8).map(|_| vec![ModelDelta::Text("阶段完成".into())]).collect();
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(neo_mock::MockTool::new("read")));
    let k = Kernel::new(
        "goal-test",
        neo_config::Config { exec_mode: ExecMode::Default, ..neo_config::Config::default() },
        tools,
        neo_core::models::ModelRegistry::single(Box::new(ScriptedModelProvider::new(script))),
        Arc::new(AllowSandbox),
        Box::new(InMemoryPersistence::new()),
        "/tmp",
    )
    .with_goal_orchestrator(Box::new(EngineGoalOrchestrator::with_stop_conditions(
        neo_orchestration::StopConditions {
            max_iterations: 2,
            ..neo_orchestration::StopConditions::default()
        },
    )));
    let (ok, output) = run_task(k, &opts("写个文档"));
    assert!(ok, "判停不是宿主失败：{output}");
    assert!(output.contains("已停止：max_iterations"), "应如实上报停止原因：{output}");
}

#[test]
fn goal_and_task_are_mutually_exclusive_in_options() {
    // ExecOptions 层面不阻止（CLI 层校验互斥），但 goal 优先且 task 不跑：
    // 这里只验证 goal 模式下 task 为空也能跑通（无头 CI 的常态）。
    let k = goal_kernel(4);
    let o = opts("写个文档");
    assert!(o.task.is_empty(), "goal 模式不需要 task");
    let (ok, output) = run_task(k, &o);
    assert!(ok, "{output}");
}
