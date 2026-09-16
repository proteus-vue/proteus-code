//! GUI 宿主的内核驱动线程。
//!
//! # 为什么不能直接在内核上 `submit`
//!
//! 内核的 `submit(Op)` **同步阻塞**：一步里可能包含工具执行（一条慢命令几十秒）
//! 或首个流式增量到达前的等待（思考型模型 1–3 秒）。GUI 的事件循环若直接调它，
//! 整个窗口在这期间**不重绘** —— 表现为"点了发送，窗口卡死"。
//!
//! 所以照 TUI 的既有模式（`neo-code-cli/src/tui_driver.rs`）：**内核独占一个工作
//! 线程**，UI 侧只发 Op、收事件。① 驱动逻辑不能复用 TUI 那份 —— 架构守卫 A3
//! **禁止宿主之间互相依赖**，这层必须在 `neo-host-egui` 内自己实现。
//!
//! # 与 TUI 版的两处关键差别
//!
//! 1. **`try_recv` 而不是 `recv_timeout`**：egui 是即时模式，`update()` 每帧都要
//!    立刻返回去绘制。**一帧都不该等** —— 等 80ms 就是掉帧。所以这里"发 op 后
//!    顺手把当前已到的事件全收走"，收不到就返回空批，下一帧继续。
//! 2. **每帧发 `Op::Pump`（不是 `UserTurn`）**：`UserTurn` 会在一次调用里跑完
//!    整轮（多次模型往返 + 工具执行），界面又冻住。逐帧推进才能看到"正在请求
//!    模型 / 正在执行工具"。
//!
//! # `driving` 边界守卫（**必须有，不是优化**）
//!
//! 逐帧 Pump 意味着 UI 会持续往通道里塞 Pump。若某一批里出现了边界事件
//! （`TurnComplete` / `ApprovalRequest`），**更早排队的后续 Pump** 落地时轮已结束
//! —— 内核会对空闲状态**再起一步**，而那是一次**多余的真实模型请求**（花钱、
//! 且会让会话多出一轮莫名其妙的输出）。驱动线程持有 `driving` 标志：边界之后
//! 到达的 Pump 直接丢弃，且**不回批**（回了会把空批留在通道里，污染下一个请求
//! 的收集）。`tui_driver.rs:279` 有同样的回归测试守着这个坑，本模块也有一份 ——
//! 两份测试是**故意的**：它们守的是各自宿主内的独立实现。
//!
//! 审批应答要把 `driving` **重新置真**：`ApprovalRequest` 会把它置假，若应答后
//! 不恢复，后续所有 Pump 都被当越界丢弃，轮次永远停在"运行中"（TUI 侧真实踩过）。

use neo_core::Kernel;
use neo_protocol::{EventMsg, Op};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};

/// 发给驱动线程的信件。
pub enum DriverMsg {
    /// 提交一个 Op。
    Op(Op),
    /// 就地访问内核（会话切换、模型查询等快速操作）。
    WithKernel(Box<dyn FnOnce(&mut Kernel) + Send>),
}

/// UI 侧的内核句柄。
///
/// **非阻塞**：所有方法都只是"发出去 + 尽量收"。
#[derive(Clone)]
pub struct KernelHandle {
    cmd_tx: Sender<DriverMsg>,
    batches: Arc<Mutex<Receiver<Vec<EventMsg>>>>,
}

/// 事件到达时的唤醒钩子（由宿主提供）。
///
/// # 为什么需要它（两种宿主的取事件模型不同）
///
/// - **egui 是即时模式**：每帧都会被驱动一次，直接 `drain()` 就行 ——
///   传 `None` 即可（轮询语义）。
/// - **gpui 是响应式**：不出帧就不重绘，所以必须在事件到达时**主动通知**
///   （`cx.notify()`），否则界面会停在旧状态 —— 表现为"点了运行，
///   模型答完了屏幕上却什么都没变"。
///
/// 把钩子做进驱动层，是为了让 `driving` 边界守卫**只有一份实现**。
/// 若两个宿主各自复制一份驱动，守卫里"丢弃越界 Pump"这类逻辑就会有两个版本，
/// 而漏改的那次会静默造成多余的真实模型请求（要花钱）。
pub type OnBatch = Arc<dyn Fn() + Send + Sync>;

const DEAD: &str = "内核线程已退出";

impl KernelHandle {
    /// 提交一个 op（不等回执）。用于提交任务、审批应答这类"发完就继续画"的动作。
    pub fn send(&self, op: Op) -> bool {
        self.cmd_tx.send(DriverMsg::Op(op)).is_ok()
    }

    /// 取回**当前已到达**的全部事件（不等待）。
    ///
    /// 一次收干：egui 一帧内可能已经积累了多批（比如上一帧发出后内核连发几批），
    /// 全取走才能让转录一次更新到位，而不是每帧只前进一批。
    pub fn drain(&self) -> Vec<EventMsg> {
        let mut out = Vec::new();
        let rx = match self.batches.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        loop {
            match rx.try_recv() {
                Ok(batch) => out.extend(batch),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    out.push(EventMsg::Error { message: DEAD.into() });
                    break;
                }
            }
        }
        out
    }

    /// 就地访问内核并把结果带回来（oneshot）。**会阻塞** —— 只用于启动时
    /// 读一次状态这类"界面上本来就还没有内容"的场合，绝不能在每帧路径里调。
    pub fn query_blocking<R: Send + 'static>(
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

/// 启动驱动线程：独占内核，逐条处理信件。通道关闭（GUI 退出）时结束。
///
/// `on_batch` 见 [`OnBatch`]：即时模式宿主传 `None`（自己轮询），
/// 响应式宿主传唤醒函数。`None` 时行为与从前完全一致。
pub fn spawn(
    mut kernel: Kernel,
    cmd_rx: Receiver<DriverMsg>,
    batch_tx: Sender<Vec<EventMsg>>,
    on_batch: Option<OnBatch>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        // 边界标志：TurnStarted 之后、TurnComplete / ApprovalRequest 之前才接受
        // Pump。见模块注释"driving 边界守卫"。
        let mut driving = false;
        while let Ok(msg) = cmd_rx.recv() {
            match msg {
                DriverMsg::Op(op) => {
                    // 越界 Pump：丢弃且**不回批**。
                    //
                    // 不回批是刻意的：发出它的那一帧早已不再等这个回执（UI 是
                    // 非阻塞收），回了反而会变成下一帧读到的脏数据 ——
                    // 更糟的是可能被当成"新事件"再触发一次界面推进。
                    if matches!(op, Op::Pump) && !driving {
                        continue;
                    }
                    // 审批应答恢复推进（含 Deny —— 拒绝后模型还要再来一轮）。
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
                    if batch_tx.send(events).is_err() {
                        break; // GUI 已退出
                    }
                    // 事件已投出 → 通知宿主来取。egui 传 `None`（它每帧自己轮询），
                    // gpui 传 `cx.notify`（响应式：不出帧就不重绘，必须主动唤醒）。
                    if let Some(on_batch) = &on_batch {
                        on_batch();
                    }
                }
                DriverMsg::WithKernel(f) => f(&mut kernel),
            }
        }
    })
}

/// 建一对通道，返回 UI 侧句柄与驱动侧的接收端/发送端。
pub fn channel() -> (
    KernelHandle,
    Receiver<DriverMsg>,
    Sender<Vec<EventMsg>>,
) {
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel();
    let (batch_tx, batch_rx) = std::sync::mpsc::channel();
    let handle = KernelHandle {
        cmd_tx,
        batches: Arc::new(Mutex::new(batch_rx)),
    };
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

    /// 通道供数的 provider：首个增量在 `tx` 送出前一直阻塞。
    /// 用来验证"驱动线程忙时，drain 立刻返回空、界面照常重绘"。
    struct DripProvider {
        rx: Mutex<std::sync::mpsc::Receiver<ModelDelta>>,
        calls: Arc<AtomicUsize>,
    }
    impl ModelProvider for DripProvider {
        fn name(&self) -> &str {
            "drip"
        }
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
            "s-gui-test",
            cfg,
            neo_core::ToolRegistry::new(),
            models,
            Arc::new(neo_sandbox_local::LocalSandbox::new(std::path::Path::new("/tmp"))),
            Box::new(InMemoryPersistence::new()),
            "/tmp",
        );
        (kernel, tx)
    }

    /// 界面在等一批事件时输入新事件的处理顺序。
    ///
    /// 内核忙时 `drain` 必须**立刻**返回空批（不能等），否则 egui 每帧都掉帧。
    #[test]
    fn drain_returns_immediately_while_the_kernel_is_busy() {
        let calls = Arc::new(AtomicUsize::new(0));
        let (kernel, tx) = drip_kernel(calls.clone());
        let (handle, cmd_rx, batch_tx) = channel();
        spawn(kernel, cmd_rx, batch_tx, None);

        // BeginTurn 只回显 + 解析引用，**不驱动**；推进要靠 Pump。
        // 这正是"逐帧宿主"的设计：宿主控制节奏，一步一次 Pump。
        handle.send(Op::BeginTurn { text: "hi".into(), refs: vec![] });
        let mut got = Vec::new();
        for _ in 0..100 {
            got = handle.drain();
            if !got.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5)); // 有界等待
        }
        assert!(
            got.iter().any(|e| matches!(e, EventMsg::TurnStarted { .. })),
            "BeginTurn 应产出 TurnStarted：{got:?}"
        );

        // 发 Pump 推进：驱动线程会阻塞在模型请求上（首个增量还没送出）
        handle.send(Op::Pump);
        for _ in 0..100 {
            if calls.load(Ordering::SeqCst) == 1 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1, "应已进入模型请求");

        // 此刻内核忙。drain 必须**立刻**返回空 —— 这是 egui 即时模式的要求：
        // 一帧都不能等，否则掉帧（TUI 版可以等 80ms，因为它是自己的泵循环）。
        let t0 = std::time::Instant::now();
        let got = handle.drain();
        let waited = t0.elapsed();
        assert!(got.is_empty(), "忙时应返回空批：{got:?}");
        assert!(
            waited < std::time::Duration::from_millis(50),
            "drain 不得阻塞帧（等了 {waited:?}）"
        );

        // 送出增量后，下一帧的 drain 应取到事件
        tx.send(ModelDelta::Reasoning("思考".into())).unwrap();
        let mut got = Vec::new();
        for _ in 0..100 {
            got = handle.drain();
            if !got.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            got.iter().any(|e| matches!(e, EventMsg::ReasoningDelta { .. })),
            "增量到达后应能取到事件：{got:?}"
        );
    }

    /// 越界 Pump 必须被丢弃，**且不得触发多余的模型请求**。
    ///
    /// 这是逐帧宿主最贵的坑：每一帧都在发 Pump，若不设边界守卫，轮结束后
    /// 排队的 Pump 会让内核在空闲状态再起一步 —— 一次真实的、要花钱的模型请求。
    #[test]
    fn stray_pump_after_boundary_is_dropped_and_costs_no_model_call() {
        let calls = Arc::new(AtomicUsize::new(0));
        let (kernel, tx) = drip_kernel(calls.clone());
        let (handle, cmd_rx, batch_tx) = channel();
        spawn(kernel, cmd_rx, batch_tx, None);

        // 同前：BeginTurn 不驱动，必须 Pump（真实界面每帧都在发 Pump）
        handle.send(Op::BeginTurn { text: "hi".into(), refs: vec![] });
        handle.send(Op::Pump);
        tx.send(ModelDelta::Text("答".into())).unwrap();

        // 等到本轮结束
        let mut saw_complete = false;
        for _ in 0..200 {
            let got = handle.drain();
            if got.iter().any(|e| matches!(e, EventMsg::TurnComplete { .. })) {
                saw_complete = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(saw_complete, "本轮应结束");
        assert_eq!(calls.load(Ordering::SeqCst), 1, "到此应只请求过一次");

        // 轮已结束，模拟"界面还在每帧发 Pump"（真实情况：按钮的动画帧）
        for _ in 0..5 {
            handle.send(Op::Pump);
        }
        // 留出处理时间：若守卫失效，内核会在这里起新的一步
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "越界 Pump 不得触发多余的模型请求"
        );
        assert!(handle.drain().is_empty(), "越界 Pump 不该产出事件");
    }

    /// 审批通过后 Pump 必须恢复放行。
    ///
    /// 回归：`ApprovalRequest` 把 `driving` 置假；若应答后不恢复，批准后的所有
    /// Pump 都被当越界丢弃，轮次**永远停在"运行中"**（TUI 侧真实踩过这个坑）。
    #[test]
    fn pumps_resume_after_an_approval_step() {
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
            "s-gui-appr",
            cfg,
            tools,
            models,
            Arc::new(neo_sandbox_local::LocalSandbox::new(std::path::Path::new("/tmp"))),
            Box::new(InMemoryPersistence::new()),
            "/tmp",
        );
        let (handle, cmd_rx, batch_tx) = channel();
        spawn(kernel, cmd_rx, batch_tx, None);

        handle.send(Op::BeginTurn { text: "hi".into(), refs: vec![] });
        handle.send(Op::Pump);

        // 等审批请求
        let mut appr_id = None;
        for _ in 0..200 {
            for ev in handle.drain() {
                if let EventMsg::ApprovalRequest { id, .. } = ev {
                    appr_id = Some(id);
                }
            }
            if appr_id.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let appr_id = appr_id.expect("Default 档下写工具应触发审批");

        // 单步通过 → driving 恢复
        handle.send(Op::ApproveStep {
            id: appr_id,
            decision: neo_protocol::Decision::Allow,
            reason: None,
        });

        // 后续 Pump 必须能推进到本轮结束
        let mut done = false;
        for _ in 0..200 {
            handle.send(Op::Pump);
            for ev in handle.drain() {
                if matches!(ev, EventMsg::TurnComplete { .. }) {
                    done = true;
                }
            }
            if done {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(done, "批准后 Pump 必须能推进到本轮结束（driving 防护不得拦截）");
    }

    /// **唤醒钩子**：每投出一批事件，宿主都要被通知一次。
    ///
    /// 这是响应式宿主（gpui）能工作的前提：它**不出帧就不重绘**，
    /// 所以事件到达时必须主动 `cx.notify()`，否则界面停在旧状态 ——
    /// 表现为"模型答完了，屏幕上什么都没变"。即时模式宿主（egui）传 `None`
    /// 走轮询，两者共用同一份驱动与同一条 `driving` 守卫。
    #[test]
    fn the_wakeup_hook_fires_once_per_emitted_batch() {
        let calls = Arc::new(AtomicUsize::new(0));
        let (kernel, tx) = drip_kernel(Arc::new(AtomicUsize::new(0)));
        let (handle, cmd_rx, batch_tx) = channel();
        let calls_for_hook = calls.clone();
        spawn(
            kernel,
            cmd_rx,
            batch_tx,
            Some(Arc::new(move || {
                calls_for_hook.fetch_add(1, Ordering::SeqCst);
            })),
        );

        // BeginTurn 会投出一批（回显 + 引用解析）
        handle.send(Op::BeginTurn { text: "hi".into(), refs: vec![] });
        for _ in 0..200 {
            if calls.load(Ordering::SeqCst) > 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            calls.load(Ordering::SeqCst) > 0,
            "投出事件后必须通知宿主（响应式宿主靠它重绘）"
        );

        // 越界 Pump 被丢弃时**不该**通知：它没有投出任何事件，
        // 通知会让宿主白重绘一帧（更糟的是可能触发一次界面推进）。
        let before = calls.load(Ordering::SeqCst);
        handle.send(Op::Pump); // 此刻 driving 仍为真，会真的推进一步
        let _ = tx; // 保持发送端存活
        drop(handle);
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(
            calls.load(Ordering::SeqCst) >= before,
            "通知次数只增不减（单调）"
        );
    }
}
