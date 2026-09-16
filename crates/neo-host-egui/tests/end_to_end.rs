//! 端到端：真内核 → 真事件流 → 界面模型。
//!
//! # 为什么要有这一层（它替代了什么）
//!
//! `ui.rs` 里的单测喂的是**手工构造的 EventMsg**，验的是"给我这条事件我会怎么放"。
//! 但界面真正需要回答的问题是另一个：**内核跑一轮之后，用户的屏幕上到底有没有
//! 东西**？这两件事不等价 —— 事件可能根本没被发出（比如"内核审批前已生成 diff"
//! 这句承诺，若内核没发 `PatchProposed`，界面写得再对也是空的）。
//!
//! 所以这里起一个**真内核**（脚本化 provider + 真工具注册表），把驱动线程与
//! `Transcript` 接起来跑完整一轮，然后断言 D2–D7 各自的内容确实出现在模型里。
//!
//! # 为什么不用真机点击验证
//!
//! 试过：CUA 无法向 egui 的自绘画布注入文本（`set_value`/`type` 都报
//! `target_verification_status: mismatched`，egui 不通过 accessibility 暴露
//! 可写的文本值）。**手工点击不是可重复的验证手段**，而这一层是：
//! 它跑在 CI 里，改坏任何一个数据通路都会红。
//!
//! 观感（排版、配色、字体）仍需真人看窗口 —— 那部分机器测不了，不在这里假装。

use neo_core::models::ModelRegistry;
use neo_core::{Kernel, ModelDelta};
use neo_exec::{build_kernel, ExecOptions};
use neo_host_egui::driver;
use neo_host_egui::ui::{Block, Transcript};
use neo_mock::InMemoryPersistence;
use neo_protocol::{EventMsg, ExecMode, Op};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 用**生产装配函数**建内核，而不是自己拼 `Kernel::new`。
///
/// 这不是省事：`build_kernel` 负责装上 compactor 与 goal orchestrator（策略在
/// L4），自己拼会漏 —— 本文件第一版就漏了编排器，于是 `GoalSet` 直接报
/// `GoalUnavailable`。**用生产装配能让测试验到真实的接线**，而不是验一个
/// 只在测试里存在的内核。
fn kernel_with(script: Vec<Vec<ModelDelta>>) -> Kernel {
    let models = ModelRegistry::single(Arc::new(
        neo_llm_deepseek::ScriptedProvider::scripted(script, "tail"),
    ));
    let sandbox = Arc::new(neo_sandbox_local::LocalSandbox::new(std::path::Path::new("/tmp")));
    let persistence = Box::new(InMemoryPersistence::new());
    let opts = ExecOptions { mode: ExecMode::Default, max_steps: 16, ..Default::default() };
    // build_kernel 会注册生产默认工具集（bash / apply_patch / …）+
    // compactor + goal orchestrator —— 测试因此验的是真实装配
    build_kernel("s-gui-e2e", std::path::Path::new("/tmp"), &opts, models, sandbox, persistence)
}

/// 跑一轮真内核，把事件喂进 `Transcript`，返回 (转录, 观察到的全部事件)。
///
/// 时间上限是**有界的**（不是固定 sleep 等）：轮询直到本轮结束或超时。
fn run_turn(script: Vec<Vec<ModelDelta>>) -> (Transcript, Vec<EventMsg>) {
    let kernel = kernel_with(script);

    let (handle, cmd_rx, batch_tx) = driver::channel();
    let _thread = // egui 是即时模式：自己每帧轮询，不需要唤醒钩子
driver::spawn(kernel, cmd_rx, batch_tx, None);

    let mut t = Transcript::new();
    let mut all_events = Vec::new();

    handle.send(Op::BeginTurn { text: "你好".into(), refs: vec![] });

    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        // **每轮都发 Pump**：`Op::Pump` 只推进一步（一次模型往返 + 它的工具
        // 执行），多步轮次需要多次。真人界面是每帧发一个 —— 这里照同样的节奏
        // （本文件第一版只发了一次，多步轮次因此空等到超时）。
        if t.pending.is_none() {
            handle.send(Op::Pump);
        }
        let batch = handle.drain();
        if !batch.is_empty() {
            all_events.extend(batch.iter().cloned());
            t.push_batch(&batch);
        }
        // 结束判据用事件（不是时间）：TurnComplete 到了就停
        if all_events
            .iter()
            .any(|e| matches!(e, EventMsg::TurnComplete { .. }))
        {
            break;
        }
        if t.pending.is_some() {
            break; // 挂起等审批，本轮到此为止
        }
        std::thread::sleep(Duration::from_millis(10)); // 有界等待
    }
    (t, all_events)
}

fn text_of(blocks: &[Block]) -> String {
    blocks
        .iter()
        .map(|b| match b {
            Block::Assistant(s) | Block::Reasoning(s) | Block::User(s) => s.clone(),
            Block::Notice { text, .. } => text.clone(),
            Block::Diff { path, diff } => format!("{path}\n{diff}"),
            Block::Tool(c) => format!("{}\n{}\n{}", c.name, c.args, c.stdout),
            Block::Files(f) => f.iter().map(|(p, a, d)| format!("{p} +{a} -{d}")).collect(),
            Block::TurnSummary { .. } => String::new(),
            Block::Todos(items) => items.iter().map(|i| i.content.clone()).collect(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// **D2**：助手正文必须出现在转录里，且流式增量合并成一块。
#[test]
fn a_real_turn_produces_visible_assistant_text() {
    let (t, events) = run_turn(
        vec![vec![
            ModelDelta::Text("第一句。".into()),
            ModelDelta::Text("第二句。".into()),
        ]],
    );

    assert!(
        events.iter().any(|e| matches!(e, EventMsg::AgentMessageDone { .. })),
        "内核应发出正文事件：{events:?}"
    );
    let assistants: Vec<&Block> = t
        .blocks
        .iter()
        .filter(|b| matches!(b, Block::Assistant(_)))
        .collect();
    assert_eq!(assistants.len(), 1, "增量应合并为一块：{:?}", t.blocks);
    assert!(
        text_of(&t.blocks).contains("第一句") && text_of(&t.blocks).contains("第二句"),
        "正文必须可见：{}",
        text_of(&t.blocks)
    );
}

/// **D3**：思考轨迹必须出现在转录里（旧的内置页面把它整段丢弃）。
#[test]
fn reasoning_reaches_the_transcript() {
    let (t, events) = run_turn(
        vec![vec![
            ModelDelta::Reasoning("我先看看".into()),
            ModelDelta::Text("答案".into()),
        ]],
    );

    assert!(
        events.iter().any(|e| matches!(e, EventMsg::ReasoningDelta { .. })),
        "内核应发出思考事件：{events:?}"
    );
    assert!(
        t.blocks.iter().any(|b| matches!(b, Block::Reasoning(s) if s.contains("我先看看"))),
        "思考轨迹必须保留（这是与旧页面的关键差别）：{:?}",
        t.blocks
    );
}

/// **D4**：工具卡片必须含名字、参数摘要与输出。
#[test]
fn tool_cards_carry_name_args_and_output() {
    // 用**真实工具**（生产默认集里的 bash）+ 只读命令：Default 档放行、无需审批，
    // 于是能跑到工具结束、验完整张卡片（含真实 stdout）。
    let (t, events) = run_turn(
        vec![
            vec![neo_mock::tool_call("c1", "bash", serde_json::json!({"cmd": "echo hello-neo"}))],
            vec![ModelDelta::Text("跑完了".into())],
        ],
    );

    assert!(
        events.iter().any(|e| matches!(e, EventMsg::ToolCallBegin { .. })),
        "应有工具调用：{events:?}"
    );
    let cards: Vec<_> = t
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Tool(c) => Some(c),
            _ => None,
        })
        .collect();
    assert!(!cards.is_empty(), "应出现工具卡片：{:?}", t.blocks);
    let card = cards[0];
    // 诊断：若 bash echo 在 Default 档需审批，本轮会挂在 pending 上（卡片未完成）
    assert!(
        t.pending.is_none(),
        "本用例假设 bash echo 在 Default 档无需审批；实际挂起了审批：{:?}\n事件：{events:?}",
        t.pending
    );
    assert_eq!(card.name, "bash");
    assert!(card.done, "工具结束事件应把卡片标记完成");
    assert_eq!(card.exit_code, Some(0));
    assert!(
        card.args.contains("echo hello-neo"),
        "参数摘要要含命令（'执行了什么'不用展开详情）：{}",
        card.args
    );
    assert!(
        card.stdout.contains("hello-neo"),
        "工具输出必须进卡片（否则只有 exit 0，看不出做了什么）：{:?}",
        card.stdout
    );
}

/// **D5**：审批前的 diff 必须能被界面拿到。
///
/// 这条验的是计划里那句承诺 —— "富界面所需数据早就在事件流里，内核审批前
/// 已生成 diff"。若内核没发 `PatchProposed`，界面再对也是空的。
#[test]
fn the_diff_shown_before_approval_actually_arrives() {
    // 用**真实工具** apply_patch：它是唯一会生成 `PatchProposed` 的工具
    // （内核在审批前调 `tool.preview()` 算出 diff）。用不存在的工具名
    // 也能触发审批，但**不会产生 diff** —— 那样下面的 diff 断言就成了
    // 永远不执行的摆设（本项目明确反对"看起来有保护"的测试）。
    let (t, events) = run_turn(
        vec![vec![neo_mock::tool_call(
            "c1",
            "apply_patch",
            serde_json::json!({"path": "a.rs", "new": "fn main() {}\n"}),
        )]],
    );

    // 先确认内核确实发了（不是界面的问题）
    assert!(
        events.iter().any(|e| matches!(e, EventMsg::ApprovalRequest { .. })),
        "Default 档下写工具应挂起审批：{events:?}"
    );
    assert!(
        t.pending.is_some(),
        "界面必须进入待审批状态（据此阻塞输入）：{:?}",
        t.blocks
    );
    // **硬断言**（不是"若发了才查"）：apply_patch 必须生成 diff 预览。
    // 这条同时验证了计划里那句承诺 —— "富界面所需数据早就在事件流里，
    // 内核审批前已生成 diff" —— 是真的，而不是文档里的一句话。
    assert!(
        events.iter().any(|e| matches!(e, EventMsg::PatchProposed { .. })),
        "apply_patch 必须产出 PatchProposed（审批前把改动画给用户看）：{events:?}"
    );
    let diff_block = t.blocks.iter().find_map(|b| match b {
        Block::Diff { path, diff } => Some((path, diff)),
        _ => None,
    });
    let (path, diff) = diff_block.expect("界面必须显示审批前的 diff");
    assert_eq!(path, "a.rs", "应显示被改的文件");
    assert!(
        diff.contains("+fn main()"),
        "diff 应含新增内容（否则预览等于没给信息）：{diff}"
    );
}

/// **D6**：轮摘要（token 数）必须出现，且状态从"运行中"回到结束。
#[test]
fn turn_summary_appears_and_running_flag_clears() {
    let (t, _events) = run_turn(vec![vec![ModelDelta::Text("完".into())]]);
    assert!(
        t.blocks.iter().any(|b| matches!(b, Block::TurnSummary { .. })),
        "应有轮摘要：{:?}",
        t.blocks
    );
    assert!(!t.running, "轮结束后 running 必须复位（状态行据此显示'就绪'）");
}

/// **D7**：Goal 快照必须能到达界面模型。
#[test]
fn goal_snapshot_reaches_the_ui_model() {
    // 目标设置不经过模型，直接发 Op 即可
    // Goal 需要编排器 —— 走生产装配才有（这正是改用 build_kernel 的原因）
    let kernel = kernel_with(vec![vec![ModelDelta::Text("做完了".into())]]);
    let (handle, cmd_rx, batch_tx) = driver::channel();
    let _thread = // egui 是即时模式：自己每帧轮询，不需要唤醒钩子
driver::spawn(kernel, cmd_rx, batch_tx, None);

    handle.send(Op::GoalSet { goal: "把 A 做完\n把 B 做完".into() });

    let mut t = Transcript::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        let batch = handle.drain();
        if !batch.is_empty() {
            t.push_batch(&batch);
        }
        if t.goal.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10)); // 有界等待
    }

    let g = t.goal.as_ref().expect("Goal 快照应到达界面模型");
    assert_eq!(g.goal_id, "goal-1", "确定性编号");
    assert_eq!(g.subtasks.len(), 2, "两行 = 两个子任务：{:?}", g.subtasks);
    // 摘要必须可用（界面直接读它，不自己拼 —— 各宿主自拼会漂移）
    assert!(g.summary().contains("goal-1"), "{}", g.summary());
}
