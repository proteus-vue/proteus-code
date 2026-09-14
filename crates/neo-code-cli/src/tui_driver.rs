//! TUI 的内核驱动线程。
//!
//! # 为什么需要它
//!
//! 内核的 `submit(Op::Pump)` 是同步的：一步里可能包含**工具执行**（一条
//! 慢命令几十秒）或**首个流式增量到达前的等待**（思考型模型 1-3 秒）。
//! 期间 TUI 主线程被阻塞在 submit 里，无法重绘 —— 界面静止，观感等同
//! 卡死（真实反馈）。把内核挪到驱动线程后，TUI 的 Pump 变成
//! 「发出 op + 最多等一个时间片收事件批」：批没到就拿到**空批**照常
//! 重绘（spinner 转），内核在后台继续干活。
//!
//! # 为什么 `Kernel` 能跨线程
//!
//! Web 宿主早已把 `Kernel` move 进 `std::thread::spawn`（neo-host-web::start），
//! `Kernel: Send` 由既有代码证明。这里不引入任何锁：驱动线程独占内核，
//! 主线程与它只通过 mpsc 通道通信（TUI 单线程，任一时刻至多一个在途
//! 请求；批次内容自解释 —— 边界判定看 TurnComplete/ApprovalRequest，
//! 不看请求配对）。
//!
//! # 越界 Pump 的防护
//!
//! 长步执行期间 TUI 会按时间片发多个 Pump。若某一批里出现了
//! `TurnComplete` / `ApprovalRequest`（边界），**更早排队的后续 Pump**
//! 落地时轮已结束 —— 内核会对空闲状态再起一步（多余的真实模型请求）。
//! 驱动线程维护 `driving` 标志：边界之后到达的 Pump 直接丢弃
//! （**不回批**，避免把空批留在通道里污染下一个请求的收集）。

use neo_core::Kernel;
use neo_protocol::{EventMsg, Op};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

/// TUI 收集事件批的等待上限。与内核的 Pump 时间片同级：批通常刚好
/// 在片末到达；超时拿到空批就重绘一帧（动画节拍）。
pub const PUMP_COLLECT: Duration = Duration::from_millis(80);

/// 发给驱动线程的信件。
pub enum DriverMsg {
    /// 提交一个 Op（Pump 的分帧语义由内核保证）。
    Op(Op),
    /// 就地访问内核：会话切换、服务商热装载、状态查询等快速操作。
    /// 闭包无返回值 —— 需要结果的用 [`KernelHandle::query`]（oneshot 回传）。
    WithKernel(Box<dyn FnOnce(&mut Kernel) + Send>),
}

/// 驱动线程的回信：每条 **Op 信件** 恰好一个批次；
/// `WithKernel` 的结果走各自的 oneshot，不过这条通道。
pub struct DriverBatch {
    pub events: Vec<EventMsg>,
}

/// TUI 侧的内核句柄：克隆给 submit 闭包、会话管理、服务商管理共用。
/// 所有接收端串行使用（单线程 UI），`Mutex` 只是满足共享所有权的形。
#[derive(Clone)]
pub struct KernelHandle {
    cmd_tx: Sender<DriverMsg>,
    batches: std::sync::Arc<std::sync::Mutex<Receiver<DriverBatch>>>,
}

const DEAD: &str = "内核线程已退出";

impl KernelHandle {
    /// 提交 op 并**阻塞**等它的批次。用于快速命令类操作（命令、审批
    /// 应答、`!shell` 等）—— 它们要么快，要么本来就该让界面等待。
    pub fn call(&self, op: Op) -> Vec<EventMsg> {
        if self.cmd_tx.send(DriverMsg::Op(op)).is_err() {
            return vec![EventMsg::Error { message: DEAD.into() }];
        }
        match self.batches.lock().unwrap().recv() {
            Ok(b) => b.events,
            Err(_) => vec![EventMsg::Error { message: DEAD.into() }],
        }
    }

    /// 提交 `Op::Pump`：发出后最多等 [`PUMP_COLLECT`]。
    ///
    /// 超时返回**空批** —— 调用方（泵循环）拿到空批也会重绘一帧，
    /// 然后发下一个 Pump 继续收。这正是"工具执行期间 spinner 会转"的机制。
    pub fn pump_collect(&self) -> Result<Vec<EventMsg>, String> {
        self.cmd_tx
            .send(DriverMsg::Op(Op::Pump))
            .map_err(|_| DEAD.to_string())?;
        match self.batches.lock().unwrap().recv_timeout(PUMP_COLLECT) {
            Ok(b) => Ok(b.events),
            Err(RecvTimeoutError::Timeout) => Ok(vec![]),
            Err(RecvTimeoutError::Disconnected) => Err(DEAD.into()),
        }
    }

    /// 就地访问内核并把任意结果带回来（oneshot；驱动线程不回批次）。
    pub fn query<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Kernel) -> R + Send + 'static,
    ) -> Option<R> {
        let (rtx, rrx) = std::sync::mpsc::channel();
        self.cmd_tx
            .send(DriverMsg::WithKernel(Box::new(move |k| {
                let _ = rtx.send(f(k));
            })))
            .ok()?;
        rrx.recv().ok()
    }
}

/// 启动驱动线程：独占内核，逐条处理信件。通道关闭（TUI 退出）时结束。
pub fn spawn(
    mut kernel: Kernel,
    cmd_rx: Receiver<DriverMsg>,
    batch_tx: Sender<DriverBatch>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        // 边界标志：TurnStarted 之后、TurnComplete / ApprovalRequest 之前
        // 才接受 Pump。见模块注释"越界 Pump 的防护"。
        let mut driving = false;
        while let Ok(msg) = cmd_rx.recv() {
            match msg {
                DriverMsg::Op(op) => {
                    // 越界 Pump：丢弃且**不回批**。发出它的泵循环早已
                    // 在边界批上退出，不会有人等这个回执；回了反而会
                    // 变成下一个请求读到的脏数据。
                    if matches!(op, Op::Pump) && !driving {
                        continue;
                    }
                    // 审批应答恢复推进：ApprovalRequest 会把 driving 置假，
                    // 应答（含 Deny —— 拒绝后模型还要再来一轮）之后必须
                    // 重新放行 Pump，否则批准后轮次卡死在"运行中"。
                    if matches!(op, Op::Approve { .. } | Op::ApproveStep { .. }) {
                        driving = true;
                    }
                    let events = kernel
                        .submit(op)
                        .unwrap_or_else(|e| vec![EventMsg::Error { message: e.to_string() }]);
                    for ev in &events {
                        match ev {
                            EventMsg::TurnStarted { .. } => driving = true,
                            EventMsg::TurnComplete { .. } => driving = false,
                            EventMsg::ApprovalRequest { .. } => driving = false,
                            _ => {}
                        }
                    }
                    if batch_tx.send(DriverBatch { events }).is_err() {
                        break; // TUI 已退出
                    }
                }
                DriverMsg::WithKernel(f) => f(&mut kernel),
            }
        }
    })
}

/// 建一对通道，返回 TUI 侧句柄与驱动侧的接收端/发送端。
pub fn channel() -> (KernelHandle, Receiver<DriverMsg>, Sender<DriverBatch>) {
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel();
    let (batch_tx, batch_rx) = std::sync::mpsc::channel();
    let handle = KernelHandle { cmd_tx, batches: std::sync::Arc::new(std::sync::Mutex::new(batch_rx)) };
    (handle, cmd_rx, batch_tx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use neo_config::Config;
    use neo_core::models::ModelRegistry;
    use neo_core::{ModelDelta, ModelProvider, ModelRequest, ModelStream};
    use neo_mock::InMemoryPersistence;
    use neo_protocol::ExecMode;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    /// 通道供数的 provider：首个增量在 `tx` 送出前一直阻塞。
    /// 用于验证"驱动线程忙时，pump_collect 超时返回空批"。
    struct DripProvider {
        rx: Mutex<std::sync::mpsc::Receiver<ModelDelta>>,
        calls: Arc<AtomicUsize>,
    }
    impl ModelProvider for DripProvider {
        fn name(&self) -> &str { "drip" }
        fn stream(&self, _req: &ModelRequest<'_>) -> ModelStream {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let first = self.rx.lock().unwrap().recv().ok();
            Box::new(
                first
                    .into_iter()
                    .chain(std::iter::once(ModelDelta::Text("答".into())))
                    .collect::<Vec<_>>()
                    .into_iter(),
            )
        }
    }

    fn drip_kernel(calls: Arc<AtomicUsize>) -> (Kernel, Sender<ModelDelta>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let cfg = Config { exec_mode: ExecMode::Default, ..Config::default() };
        let models = ModelRegistry::single(Arc::new(DripProvider { rx: Mutex::new(rx), calls }));
        let kernel = Kernel::new(
            "s-test",
            cfg,
            neo_core::ToolRegistry::new(),
            models,
            Arc::new(neo_sandbox_local::LocalSandbox::new(std::path::Path::new("/tmp"))),
            Box::new(InMemoryPersistence::new()),
            "/tmp",
        );
        (kernel, tx)
    }

    #[test]
    fn pump_collect_times_out_while_kernel_is_busy_then_delivers() {
        let calls = Arc::new(AtomicUsize::new(0));
        let (kernel, tx) = drip_kernel(calls.clone());
        let (handle, cmd_rx, batch_tx) = channel();
        spawn(kernel, cmd_rx, batch_tx);

        handle.call(Op::BeginTurn { text: "hi".into(), refs: vec![] });
        // 首个增量未送出：驱动线程阻塞在模型请求上，pump_collect 应超时返回空批
        let got = handle.pump_collect().unwrap();
        assert!(got.is_empty(), "忙时应返回空批让界面继续重绘：{got:?}");
        // 送出增量后，批应带着事件到达
        tx.send(ModelDelta::Reasoning("思考".into())).unwrap();
        let got = handle.pump_collect().unwrap();
        assert!(
            got.iter().any(|e| matches!(e, EventMsg::ReasoningDelta { .. })),
            "增量到达后应产出事件批：{got:?}"
        );
    }

    #[test]
    fn pumps_resume_after_an_approval_step() {
        // 回归：ApprovalRequest 把 driving 置假；应答（ApproveStep）后必须
        // 重新放行 Pump —— 否则批准后所有 Pump 被越界防护丢弃，
        // 轮次永远停在"运行中"。
        let cfg = Config { exec_mode: ExecMode::Default, ..Config::default() };
        let mut tools = neo_core::ToolRegistry::new();
        // Write 类别在 Default 档必 Ask —— 稳定触发审批挂起
        tools.register(Arc::new(neo_mock::MockTool::writing("write")));
        let script = vec![
            vec![neo_mock::tool_call("c1", "write", serde_json::json!({}))],
            vec![ModelDelta::Text("done".into())],
        ];
        let models = ModelRegistry::single(Arc::new(
            neo_llm_deepseek::ScriptedProvider::scripted(script, "tail"),
        ));
        let kernel = Kernel::new(
            "s-appr",
            cfg,
            tools,
            models,
            Arc::new(neo_sandbox_local::LocalSandbox::new(std::path::Path::new("/tmp"))),
            Box::new(InMemoryPersistence::new()),
            "/tmp",
        );
        let (handle, cmd_rx, batch_tx) = channel();
        spawn(kernel, cmd_rx, batch_tx);

        handle.call(Op::BeginTurn { text: "hi".into(), refs: vec![] });
        // 第一步要 read 工具 → Default 档 Ask → 挂起审批
        let got = handle.pump_collect().unwrap();
        let appr_id = got.iter().find_map(|e| match e {
            EventMsg::ApprovalRequest { id, .. } => Some(id.clone()),
            _ => None,
        });
        let Some(appr_id) = appr_id else {
            panic!("应有审批请求：{got:?}");
        };
        // 应答（单步通过）→ driving 恢复 → 后续 Pump 推进到本轮结束
        handle.call(Op::ApproveStep {
            id: appr_id,
            decision: neo_protocol::Decision::Allow,
            reason: None,
        });
        let got = handle.pump_collect().unwrap();
        assert!(
            got.iter().any(|e| matches!(e, EventMsg::TurnComplete { .. })),
            "批准后 Pump 必须能推进到本轮结束（driving 防护不得拦截）：{got:?}"
        );
    }

    #[test]
    fn stray_pump_after_boundary_is_dropped_without_polluting_the_channel() {
        let calls = Arc::new(AtomicUsize::new(0));
        let (kernel, tx) = drip_kernel(calls.clone());
        let (handle, cmd_rx, batch_tx) = channel();
        spawn(kernel, cmd_rx, batch_tx);

        handle.call(Op::BeginTurn { text: "hi".into(), refs: vec![] });
        // 一步收尾（Text 增量 → Done → TurnComplete）
        tx.send(ModelDelta::Text("答".into())).unwrap();
        let got = handle.pump_collect().unwrap();
        assert!(
            got.iter().any(|e| matches!(e, EventMsg::TurnComplete { .. })),
            "该批应完成本轮：{got:?}"
        );
        // 越界 Pump：驱动线程必须丢弃且不回批 —— 否则空批会污染下一个请求
        let got = handle.pump_collect().unwrap();
        assert!(got.is_empty(), "越界 Pump 不该产出事件：{got:?}");
        // 通道必须仍是干净的：下一个请求立刻拿到自己的回执，而不是陈旧批
        let models = handle.query(|k| {
            k.available_models().into_iter().map(|m| m.name).collect::<Vec<_>>()
        });
        assert_eq!(models.as_deref(), Some(&["drip".to_string()][..]), "query 应正常拿到回执");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "越界 Pump 不得触发多余的模型请求"
        );
    }
}
