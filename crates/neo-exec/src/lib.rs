//! L5 HOST · 无头 / CI —— 零交互把一轮任务跑完
//!
//! # 它同时是**内核的第一个真实宿主**
//!
//! 前四个宿主（TUI/Desktop/Web）都还没写，但内核已经不依赖界面即可运行：
//! `neo-exec` 用最少的代码把「真实模型 → 真实沙箱 → 真实工具」串起来，
//! 让内核能跑通一轮真任务。这也是 T6（宿主语义等价）的第一个被试点。
//!
//! # 审批策略：无头环境下如何决定
//!
//! 无头宿主无法弹交互式审批（`HostCapabilities::interactive_prompt = false`）。
//! 两种正确做法：
//!   - 用 `FullAccess` 档（自动放行，风险由调用方承担）
//!   - 用受限档 + 预置的自动应答（本实现：**默认拒绝**未预授权的写）
//!
//! 本实现选择**默认拒绝**：安全边界在无人值守时更该收紧，而不是放开。
//! 想放行就显式换档（`--mode auto-edit` 或 `--mode full`），不隐式放水。

use neo_config::{resolve, Config};
use neo_core::{Kernel, Message, ToolRegistry};
use neo_protocol::{Decision, EventMsg, ExecMode, Op};

/// 一轮运行的配置。
pub struct ExecOptions {
    pub task: String,
    pub mode: ExecMode,
    pub max_steps: usize,
    /// 无人值守时对审批请求的默认动作。
    pub on_approval: Decision,
    pub json: bool,
}

impl Default for ExecOptions {
    fn default() -> Self {
        Self {
            task: String::new(),
            mode: ExecMode::Default,
            max_steps: 16,
            // 默认拒绝未预授权的写：无人值守时安全边界该收紧
            on_approval: Decision::Deny,
            json: false,
        }
    }
}

/// 跑一轮，返回（是否成功, 输出文本）。
pub fn run_task(
    mut kernel: Kernel,
    opts: &ExecOptions,
) -> (bool, String) {
    let mut log: Vec<String> = Vec::new();
    let mut ok = true;

    // 与 TUI / Web 走同一份 `parse_refs`：`neo exec "@src/main.rs 解释下"`
    // 与在 TUI 里敲同一句话必须等价，否则同一输入在不同宿主产生不同请求。
    let events = match kernel.submit(Op::UserTurn {
        refs: neo_protocol::parse_refs(&opts.task),
        text: opts.task.clone(),
    }) {
        Ok(e) => e,
        Err(e) => return (false, format!("提交失败：{e}")),
    };
    render(&events, opts, &mut log);

    // 审批循环：内核每挂起一次就应答一次，直到本轮结束。
    // 有最大轮次上限，避免应答逻辑出错时无限转。
    let mut guard = 0;
    while let neo_core::KernelState::AwaitingApproval { id } = kernel.state().clone() {
        guard += 1;
        if guard > 64 {
            ok = false;
            log.push("[exec] 审批轮次过多，中止（可能是应答逻辑或内核状态机异常）".into());
            break;
        }
        if !opts.json {
            log.push(format!("[exec] 审批 {id} → {:?}（无人值守默认策略）", opts.on_approval));
        }
        // Decision 是 Copy，取引用后的副本即可
        let decision = opts.on_approval;
        match kernel.submit(Op::Approve { id, decision }) {
            Ok(events) => render(&events, opts, &mut log),
            Err(e) => {
                ok = false;
                log.push(format!("[exec] 审批失败：{e}"));
                break;
            }
        }
    }

    // 失败原因可见性：工具非零退出时，把对应结果的原因打出来。
    // 没有这一步，用户只看到 `exit -1` 却不知为什么 —— 无头宿主尤其需要。
    if !opts.json {
        for m in kernel.messages() {
            if let Message::ToolResult { name, output, .. } = m {
                if output.exit_code != 0 && !output.stderr.is_empty() {
                    log.push(format!("[工具 {name} 失败] {}", output.stderr.trim()));
                }
            }
        }
    }

    // 汇总最终答复
    let final_text: String = kernel
        .messages()
        .iter()
        .rev()
        .find_map(|m| match m {
            Message::Assistant { text, .. } if !text.is_empty() => Some(text.clone()),
            _ => None,
        })
        .unwrap_or_default();

    if opts.json {
        let out = serde_json::json!({ "ok": ok, "final": final_text });
        (ok, out.to_string())
    } else {
        if !final_text.is_empty() {
            log.push(String::new());
            log.push(format!("── 最终答复 ──\n{final_text}"));
        }
        (ok, log.join("\n"))
    }
}

/// 事件 → 终端文本。**宿主只做渲染，不含业务逻辑。**
fn render(events: &[EventMsg], opts: &ExecOptions, log: &mut Vec<String>) {
    for e in events {
        let line = match e {
            // 用户消息也进转录：无头输出应能看出"当时问的是什么"
            EventMsg::UserSubmitted { text } => Some(format!("> {text}")),
            // 引用解析结果用户必须知道（"引用的文件到底读到了没有"），
            // 但注入的正文不进无头转录 —— 那是模型上下文，不是对话。
            EventMsg::RefsResolved { summary, .. } => {
                if summary.is_empty() {
                    None
                } else {
                    Some(format!("[refs] {}", summary.join("；")))
                }
            }
            EventMsg::FilesChanged { files } => {
                let adds: usize = files.iter().map(|f| f.additions).sum();
                let dels: usize = files.iter().map(|f| f.deletions).sum();
                Some(format!("[files] {} 个文件已改（+{adds} -{dels}）", files.len()))
            }
            // 未聚合前的单文件改动不单独输出（否则同一文件会刷屏）；由 FilesChanged 汇总
            EventMsg::FileChanged { .. } => None,
            EventMsg::ContextCompacted { removed_messages, .. } => {
                Some(format!("[compact] 上下文已压缩（{removed_messages} 条消息 → 1 条摘要）"))
            }
            EventMsg::Rewound { turns, removed_messages, files_kept } => Some(format!(
                "[rewind] 回退 {turns} 轮（删除 {removed_messages} 条消息）；\
                 磁盘上 {files_kept} 个文件改动**未**撤销"
            )),
            EventMsg::TodoUpdated { items } => {
                let done = items.iter().filter(|i| matches!(i.status, neo_protocol::TodoStatus::Completed)).count();
                Some(format!("[todo] {done}/{} 完成", items.len()))
            }
            EventMsg::TurnStarted { .. } => Some("[turn] 开始".to_string()),
            EventMsg::AgentMessageDelta { delta } => Some(delta.clone()),
            EventMsg::AgentMessageDone { .. } => None, // 增量已输出，避免重复
            EventMsg::ToolCallBegin { name, id, .. } => Some(format!("[tool] {name} ({id})")),
            EventMsg::ToolCallEnd { id, exit_code, stdout, stderr, truncated } => {
                // 无头宿主也把输出打出来：否则日志里只有退出码，
                // 出问题时无法从日志复盘"命令到底打印了什么"。
                let mut s = format!("[tool] {id} → exit {exit_code}");
                let out = if stdout.is_empty() { stderr } else { stdout };
                if !out.trim().is_empty() {
                    s.push('\n');
                    s.push_str(out.trim_end());
                }
                if *truncated {
                    s.push_str("\n… 输出已截断");
                }
                Some(s)
            }
            EventMsg::ApprovalRequest { detail, .. } => Some(format!("[审批] {detail}")),
            EventMsg::Error { message } => Some(format!("[错误] {message}")),
            EventMsg::TurnComplete { input_tokens, output_tokens } => Some(format!(
                "[turn] 完成（in {input_tokens} / out {output_tokens} tokens）"
            )),
            EventMsg::ShutdownComplete => Some("[shutdown]".to_string()),
            EventMsg::SessionConfigured { session_id } => {
                Some(format!("[session] {session_id} 配置已更新"))
            }
            EventMsg::ModelSwitched { model, context_limit } => Some(if *context_limit == 0 {
                format!("[model] 已切换到 {model}（上下文窗口未知）")
            } else {
                format!("[model] 已切换到 {model}（上下文 {context_limit} tokens）")
            }),
            // 推理默认不单独打（噪声大）；`--json` 时它在事件流里，
            // 需要时按需取。这里只给一个极简标记，避免刷屏。
            EventMsg::ReasoningDelta { .. } => None,
            EventMsg::PatchProposed { path, .. } => Some(format!("[patch] {path}")),
            EventMsg::CheckpointSaved { checkpoint_id } => {
                Some(format!("[checkpoint] {checkpoint_id}"))
            }
            EventMsg::GoalProgress { done, total, .. } => Some(format!("[goal] {done}/{total}")),
        };
        if let Some(l) = line {
            if !opts.json {
                log.push(l);
            }
        }
    }
}

/// 由 `neo exec` 解析出的参数构造一个可运行的 kernel。
///
/// 之所以把装配放在这里而不是 main：让 `main` 只做参数解析与输出，
/// 装配逻辑可被测试与其它宿主复用。
pub fn build_kernel(
    session_id: &str,
    workspace: &std::path::Path,
    opts: &ExecOptions,
    models: neo_core::models::ModelRegistry,
    sandbox: std::sync::Arc<dyn neo_core::SandboxBackend>,
    persistence: Box<dyn neo_core::SessionPersistence>,
) -> Kernel {
    let mut tools = ToolRegistry::new();
    neo_capability::register_defaults(&mut tools);
    let cfg = Config { exec_mode: opts.mode, ..Config::default() };
    // 技能目录在**装配点**加载一次（而不是每次 `$skill` 引用都扫盘）：
    // 引用是热路径，扫盘是冷路径。代价是会话中途新增技能需要重启才可见 ——
    // 这个取舍写在 README 的诚实边界里。
    let roots = neo_skill_loader::default_roots(workspace);
    let (skills, _report) = neo_skill_loader::load_roots(&roots);
    Kernel::new(session_id, cfg, tools, models, sandbox, persistence, workspace)
        .with_max_steps(opts.max_steps)
        // 压缩策略由 L4 提供（内核只认契据）—— 这样 `/compact` 不是空操作
        .with_compactor(Box::new(
            neo_orchestration::PolicyCompactor::default(),
        ))
        .with_skills(skills)
}

/// 档位短名（footer / 状态栏用）。
///
/// 与 `describe_mode` 分开是刻意的：长描述含"沙箱/审批/文件编辑"三段，
/// 放状态栏会把右侧信息挤掉；短名保证窄终端也放得下。
pub fn mode_short(mode: ExecMode) -> &'static str {
    match mode {
        ExecMode::Plan => "plan",
        ExecMode::ConfirmBefore => "confirm",
        ExecMode::Default => "default",
        ExecMode::AutoEdit => "auto-edit",
        ExecMode::FullAccess => "full",
    }
}

/// 当前档位的可读描述（用于启动提示）。
pub fn describe_mode(mode: ExecMode) -> String {
    let r = resolve(mode);
    format!("{:?}（沙箱 {:?} / 审批 {:?} / 文件编辑 {:?}）", mode, r.sandbox, r.approval, r.file_edit)
}
