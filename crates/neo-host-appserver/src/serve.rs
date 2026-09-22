//! 传输层：stdio（`neo app-server`，一行一条）与 unix socket（`--listen`，多客户端）。
//!
//! # 线程模型（与 Web 宿主同一条：内核独占线程）
//!
//! ```text
//!  读取（每连接一个）──(id,Job)──▶ 内核线程（全局唯一）─┬─响应行─▶ 该连接的写线程 ─▶ 客户端
//!        └──────────── 响应行 ─────────────────────────┘ └─事件行─▶ **全部**连接的写线程（广播）
//! ```
//!
//! 三条不变量：
//! 1. **输出流上只有协议行**：诊断一律走 stderr。任何一句 `println!` 都会让
//!    客户端解析失败 —— 这是 stdio 协议最容易踩的坑（unix socket 同理）。
//! 2. **响应与通知共用一条通道**：同一连接的两种行都经同一个 mpsc 进写线程，
//!    因此"一行"始终完整，不会出现两个线程交叉写坏半行。
//! 3. **响应表示已受理，不表示已完成**：`turn/start` 立即返回 `accepted`，
//!    整轮进度以事件通知的形式陆续到达（与 Codex app-server 同款语义）。
//!    这样客户端不必在"等到整轮跑完"和"界面冻结"之间二选一。
//!
//! # 多客户端（`--listen unix://`）：共用一个内核
//!
//! 多个客户端连同一个服务进程、共用**一个**内核 —— `Kernel` 不是 `Sync`，
//! 串行是它的真实模型，宿主层不伪造并行。在这个前提下四条语义：
//!
//! - **事件广播**：内核事件推给**所有**连接；`seq` 是内核全局严格递增的
//!   （它是事件流位置，与会话日志同序），不是每连接一套。
//!   `thread/resume` 的历史同样广播 —— 切的是共用会话，所有客户端都要重画。
//! - **响应回发起连接**：请求 id 只在自己的连接内有意义，不串台。
//! - **审批同看同控**：`ApprovalRequest` 的 id 由内核单一事实源给出，哪个
//!   连接答都算（与 Web 宿主 SSE 扇出同一语义：多看同控）。
//! - **`shutdown` 关的是内核**（共用的那个）：收尾事件广播给所有连接，
//!   然后全部收到 EOF、进程退出。**断开一个连接（EOF）只是这个客户端走了**，
//!   其它连接照常。
//!
//! # 为什么不需要"事件重放/断线续订"
//!
//! stdio 连接的生命期就是进程生命期；unix 连接断开 = 这个客户端走了，重连
//! 是一个新连接 —— 而事件流是内核全局的（`seq` 不因连接进出重置）。客户端
//! 要"接着看"，正道是 `thread/history`（`facts_of` 投影）或读会话日志（JSONL）
//! 重建，而不是在这条传输上加缓冲。
//!
//! 客户端还应当知道一条性质：**响应与通知之间没有先后保证**。两者由不同线程
//! 写进同一条通道（内核慢时响应先到，快时通知先到），所以按 id 关联响应、
//! 按 `seq` 关联事件，不要假定交错顺序。
//!
//! # 输入有界（"内存有界性"的另一半）
//!
//! 内核侧对**输出**有上限（截断 + `truncated` 如实标注）；这条传输对**输入**
//! 同样设上限：单条请求行不得超过 [`MAX_REQUEST_BYTES`]，超限那一行**不缓冲、
//! 不解析**地丢弃，并回一条 `invalid_request` 说明丢了多少字节。没有这条约束时，
//! 一句不带换行的巨型输入就能把宿主的内存吃光 —— 本地客户端也不例外，
//! "新增可能产生大输出的路径必须受上限约束"这条义务与对端是谁无关。

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use neo_protocol::{EventMsg, Op};
use serde_json::{json, Value};

use crate::jsonrpc::{
    self, Action, RpcError, ThreadCmd, ALREADY_INITIALIZED, INVALID_REQUEST, JSONRPC_VERSION,
    KERNEL_ERROR, NOT_INITIALIZED, PARSE_ERROR,
};

/// 单条请求行的字节上限（含换行符）。
///
/// 8 MiB 是"远大于任何真实请求、又远小于会把机器吃光"的量级：一轮 user turn
/// 的文本、一次 `session/configure` 的参数都在几 KiB 量级，三个数量级的余量足够，
/// 同时把失控或恶意客户端挡在有界内存内。
pub const MAX_REQUEST_BYTES: usize = 8 * 1024 * 1024;

/// 全局关停信号（unix 模式）：所有客户端断开或收到 `shutdown` 后置位。
///
/// 为什么不是"关掉 listener"来唤醒阻塞的 `accept`：那在语义上说不通
///（关停是协议动作，不该动传输层的句柄），实现上还要跨线程共享句柄。
/// 用**一次自连**唤醒：本地 unix socket 上自己连自己，`accept` 立刻返回，
/// 读到的是空流即知"不是真客户端" —— 等价于 self-pipe 写唤醒，但连额外的
/// 管道都不用开，且拒绝一切轮询（见 ai-efficiency-rules）。
static SHUTDOWN: AtomicBool = AtomicBool::new(false);

/// 连接 id：只用于把响应路由回发起它的那条连接。0 = 唯一连接（stdio）。
type ConnId = u64;

const SOLE_CONN: ConnId = 0;

/// 连接 id 分配器（unix 模式下每条连接一个）。
static NEXT_CONN: AtomicU64 = AtomicU64::new(1);

/// 每连接一条"待写行"通道；内核线程按 id 路由响应、按全员广播事件。
type Outputs = Arc<Mutex<HashMap<ConnId, Sender<String>>>>;

/// 把一行回给发起请求的那条连接（响应路径）。
fn send_to(outputs: &Outputs, conn: ConnId, line: String) {
    if let Some(tx) = outputs.lock().expect("输出表锁中毒").get(&conn) {
        let _ = tx.send(line);
    }
}

/// 把一行广播给**全部**连接（事件路径：内核事件、resume 历史、收尾）。
fn send_all(outputs: &Outputs, line: String) {
    for tx in outputs.lock().expect("输出表锁中毒").values() {
        let _ = tx.send(line.clone());
    }
}

/// 控制命令：读取线程 → 内核线程。
enum Control {
    /// 一个工作项（带发起它的连接 id，响应回那儿）
    Job { conn: ConnId, job: Job },
    /// 所有客户端断开：停内核线程（不再注入任何 Op —— EOF 不是协议动作）
    Quit,
}

/// 处理一个工作项：返回（响应行，待广播的事件行）。
///
/// 单/多客户端共用这一份 —— 差别只在"响应行投给谁"，由调用方的 `conn` 决定。
/// 抽成函数而不是在两处各写一遍：两份 `match handle(...)` 必然漂移。
fn run_job(
    handle: &mut impl FnMut(Job) -> JobOut,
    seq: &mut u64,
    job: Job,
) -> (Option<String>, Vec<String>) {
    let (events, reply) = match job {
        Job::Op(op) => match handle(Job::Op(op)) {
            JobOut::Events(ev) => (ev, None),
            // Op 处理器不该回 ThreadResult；回了就如实报在 null id 上
            JobOut::Thread(ThreadResult::Error(msg)) => {
                (Vec::new(), Some(error_line(Value::Null, &RpcError::new(KERNEL_ERROR, msg))))
            }
            JobOut::Thread(ThreadResult::Value(v)) => {
                (Vec::new(), Some(result_line(Value::Null, v)))
            }
            JobOut::Thread(ThreadResult::Resumed { history, .. }) => (history, None),
        },
        Job::Thread { id, cmd } => match handle(Job::Thread { id: id.clone(), cmd }) {
            JobOut::Thread(ThreadResult::Value(v)) => (Vec::new(), Some(result_line(id, v))),
            JobOut::Thread(ThreadResult::Resumed { result, history }) => {
                (history, Some(result_line(id, result)))
            }
            JobOut::Thread(ThreadResult::Error(msg)) => (
                Vec::new(),
                Some(error_line(id, &RpcError::new(KERNEL_ERROR, msg))),
            ),
            JobOut::Events(ev) => (ev, None),
        },
    };
    // seq 是**内核全局**的事件流位置（与会话日志同序），不是每连接一套
    let broadcast = events
        .iter()
        .map(|e| {
            *seq += 1;
            notification(*seq, e).to_string()
        })
        .collect();
    (reply, broadcast)
}

/// `thread/*` 的处理结果（由装配层给出，宿主只负责变成响应行）。
#[derive(Debug)]
pub enum ThreadResult {
    /// 普通 RPC result
    Value(Value),
    /// 切换成功：result + 要以事件通知推送的历史（宿主重画转录用）
    Resumed { result: Value, history: Vec<EventMsg> },
    /// 失败（不存在 / 正在使用 / 内核线程已退出…）
    Error(String),
}

/// 内核线程上的工作项。
///
/// 为什么 `Op` 与 `thread/*` 走**同一个**处理器：切换会话要调
/// `Kernel::switch_session`，而 `Kernel` 只能被一个闭包持有
/// （它不是 `Sync`）。两个 `FnMut` 各拿一半是编不过的。
#[derive(Debug)]
pub enum Job {
    /// 推进内核状态机
    Op(Op),
    /// 会话库控制（响应是同步的，见 [`ThreadResult`]）。
    /// `id` 是 JSON-RPC 请求 id，处理完用它写响应行。
    Thread { id: Value, cmd: ThreadCmd },
}

/// [`Job`] 的处理结果。
#[derive(Debug)]
pub enum JobOut {
    /// Op 产生的事件（以通知推送）
    Events(Vec<EventMsg>),
    /// thread/* 的 RPC 结果
    Thread(ThreadResult),
}

/// 启动 stdio 传输（`neo app-server` 的入口）。
///
/// 返回即"客户端已关闭连接或请求了关停"——内核线程与写线程都已收尾。
/// `handle` 同时处理 `Op` 与 `thread/*`（同一闭包持有内核）。
pub fn serve_stdio<F>(handle: F) -> std::io::Result<()>
where
    F: FnMut(Job) -> JobOut + Send + 'static,
{
    let stdin = std::io::stdin();
    serve(stdin.lock(), std::io::stdout(), handle)
}

/// [`serve_ops`] 的 stdio 版。
pub fn serve_stdio_ops<F>(handle_op: F) -> std::io::Result<()>
where
    F: FnMut(Op) -> Vec<EventMsg> + Send + 'static,
{
    let stdin = std::io::stdin();
    serve_ops(stdin.lock(), std::io::stdout(), handle_op)
}

/// 在任意读写端上跑协议（测试用 `Cursor` + 内存缓冲即可覆盖全流程）。
///
/// `handle_op` 由调用方注入（通常是"把 Op 交给内核线程"），因此本 crate
/// **不依赖内核的具体类型**，只依赖 `Op`/`EventMsg` 契据 —— 与 Web 宿主同款，
/// 也让测试能注入一个脚本化的假内核。
/// 只处理 `Op` 的便捷入口（测试假内核 / 未装配会话库的装配）。
/// `thread/*` 一律回"未接会话库"，**不静默空列表**。
pub fn serve_ops<R, W, F>(input: R, output: W, mut handle_op: F) -> std::io::Result<()>
where
    R: BufRead,
    W: Write + Send + 'static,
    F: FnMut(Op) -> Vec<EventMsg> + Send + 'static,
{
    serve(input, output, move |job| match job {
        Job::Op(op) => JobOut::Events(handle_op(op)),
        Job::Thread { .. } => JobOut::Thread(ThreadResult::Error(
            "本装配未接会话库，无法使用 thread/*".into(),
        )),
    })
}

pub fn serve<R, W, F>(input: R, output: W, mut handle: F) -> std::io::Result<()>
where
    R: BufRead,
    W: Write + Send + 'static,
    F: FnMut(Job) -> JobOut + Send + 'static,
{
    // 唯一连接：多客户端模型的退化情形（conn = SOLE_CONN，广播 = 单发）
    let (out_tx, out_rx) = channel::<String>();
    let outputs: Outputs = Arc::new(Mutex::new(HashMap::from([(SOLE_CONN, out_tx)])));
    let (job_tx, job_rx) = channel::<Control>();

    let writer = std::thread::spawn(move || write_lines(output, out_rx));

    let kernel_outputs = outputs.clone();
    let kernel = std::thread::spawn(move || {
        let mut seq: u64 = 0;
        while let Ok(ctrl) = job_rx.recv() {
            match ctrl {
                Control::Job { conn, job } => {
                    let (reply, broadcast) = run_job(&mut handle, &mut seq, job);
                    if let Some(line) = reply {
                        send_to(&kernel_outputs, conn, line);
                    }
                    for line in broadcast {
                        send_all(&kernel_outputs, line);
                    }
                }
                Control::Quit => return,
            }
        }
    });

    let result = read_loop(SOLE_CONN, input, &outputs, &job_tx).map(|_| ());
    // stdio 的生命期 = 进程生命期：读到 EOF 或 shutdown 即整个会话结束，
    // 但写线程要把手头的行写完（含内核的收尾事件），故最后才 drop
    let _ = job_tx.send(Control::Quit);
    let _ = kernel.join();
    outputs.lock().expect("输出表锁中毒").remove(&SOLE_CONN);
    let _ = writer.join();
    result
}

/// 事件 → 通知行。
///
/// 载荷嵌在 `payload` 下**不摊平**：`ApprovalRequest` 自带 `kind`（内核判定的
/// 调用类别），摊平会与通知的 `kind`（事件名）撞键，必然丢一个。
pub fn notification(seq: u64, event: &EventMsg) -> Value {
    let value = serde_json::to_value(event).unwrap_or(Value::Null);
    let (kind, payload) = match value {
        // 带字段的变体：外部标签 → `{"<kind>": {…}}`
        Value::Object(mut map) if map.len() == 1 => {
            let k = map.keys().next().cloned().unwrap_or_default();
            let inner = map.remove(&k).unwrap_or(Value::Null);
            (k, inner)
        }
        // 单元变体（`ShutdownComplete`）：serde 产出裸字符串 `"shutdown_complete"`
        Value::String(k) => (k, Value::Null),
        other => (String::new(), other),
    };
    json!({
        "jsonrpc": JSONRPC_VERSION,
        "method": "event",
        "params": { "seq": seq, "kind": kind, "payload": payload },
    })
}

fn error_line(id: Value, error: &RpcError) -> String {
    json!({ "jsonrpc": JSONRPC_VERSION, "id": id, "error": error.to_json() }).to_string()
}

fn result_line(id: Value, result: Value) -> String {
    json!({ "jsonrpc": JSONRPC_VERSION, "id": id, "result": result }).to_string()
}

fn write_lines<W: Write>(mut out: W, rx: Receiver<String>) {
    for line in rx {
        let ok = out
            .write_all(line.as_bytes())
            .and_then(|()| out.write_all(b"\n"))
            .and_then(|()| out.flush())
            .is_ok();
        if !ok {
            return; // 客户端已断开：不再尝试（读取侧会因 EOF/关停收尾）
        }
    }
}

/// unix socket 上的多客户端服务（`neo app-server --listen unix://…`）。
///
/// 多个客户端共用**一个**内核：事件广播、响应回发起连接、审批同看同控、
/// `shutdown` 全局收尾（模块头有四条语义的完整说明）。
///
/// 返回即"所有客户端都已断开或请求了关停"：内核线程与各写线程都已收尾。
pub fn serve_unix<F>(socket_path: &Path, handle: F) -> std::io::Result<()>
where
    F: FnMut(Job) -> JobOut + Send + 'static,
{
    // 上次崩溃留下的旧 socket 文件会让 bind 失败；只有确认不是活的才清掉
    if socket_path.exists() {
        match UnixStream::connect(socket_path) {
            Ok(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AddrInUse,
                    format!(
                        "{} 已有实例在监听（能连通）—— 先退出它，或换一个 socket 路径",
                        socket_path.display()
                    ),
                ));
            }
            Err(_) => {
                let _ = std::fs::remove_file(socket_path);
            }
        }
    }
    let listener = UnixListener::bind(socket_path)?;
    SHUTDOWN.store(false, Ordering::SeqCst);
    let outputs: Outputs = Arc::new(Mutex::new(HashMap::new()));
    let (job_tx, job_rx) = channel::<Control>();

    let kernel_outputs = outputs.clone();
    let kernel = std::thread::spawn(move || {
        let mut handle = handle;
        let mut seq: u64 = 0;
        while let Ok(ctrl) = job_rx.recv() {
            match ctrl {
                Control::Job { conn, job } => {
                    let (reply, broadcast) = run_job(&mut handle, &mut seq, job);
                    if let Some(line) = reply {
                        send_to(&kernel_outputs, conn, line);
                    }
                    for line in broadcast {
                        send_all(&kernel_outputs, line);
                    }
                }
                // 停机本身不是内核动作：`shutdown` 方法已在它自己的 Job 里
                // 让内核发过 ShutdownComplete；EOF 则什么协议事件都没有
                Control::Quit => return,
            }
        }
    });

    let mut writers: Vec<std::thread::JoinHandle<()>> = Vec::new();
    let result = accept_loop(&listener, socket_path, &outputs, &job_tx, &mut writers);

    // 收尾顺序：先停内核（它可能还在广播收尾事件），再让写线程写完
    let _ = job_tx.send(Control::Quit);
    let _ = kernel.join();
    drop(job_tx);
    outputs.lock().expect("输出表锁中毒").clear();
    for w in writers {
        let _ = w.join();
    }
    let _ = std::fs::remove_file(socket_path);
    result
}

/// accept 循环：给每条新连接派读线程；`shutdown` 或全部断开后返回。
///
/// 关停唤醒用**一次自连**（[`SHUTDOWN`] 的注释）：不是轮询。唤醒自连到达时
/// `SHUTDOWN` 必已置位（先置位、后自连），故 accept 返回后先查它 —— 是则
/// 直接收尾，不需要 peek 区分真假客户端。
fn accept_loop(
    listener: &UnixListener,
    socket_path: &Path,
    outputs: &Outputs,
    job_tx: &Sender<Control>,
    writers: &mut Vec<std::thread::JoinHandle<()>>,
) -> std::io::Result<()> {
    let live = Arc::new(Mutex::new(0usize));
    loop {
        let (stream, _) = listener.accept()?;
        if SHUTDOWN.load(Ordering::SeqCst) {
            return Ok(()); // 唤醒自连（或关停期间的迟到连接）：服务在收尾
        }
        *live.lock().expect("连接计数锁中毒") += 1;
        let conn = NEXT_CONN.fetch_add(1, Ordering::SeqCst);
        let (out_tx, out_rx) = channel::<String>();
        outputs.lock().expect("输出表锁中毒").insert(conn, out_tx);

        let write_half = stream.try_clone()?;
        writers.push(std::thread::spawn(move || {
            write_lines(write_half, out_rx)
        }));

        let conn_outputs = outputs.clone();
        let conn_jobs = job_tx.clone();
        let conn_live = live.clone();
        let path = socket_path.to_path_buf();
        std::thread::spawn(move || {
            serve_connection(conn, stream, &conn_outputs, &conn_jobs, &conn_live, &path);
        });
    }
}

/// 一条连接的读线程：跑协议读取循环，走完负责注销自己与唤醒收尾。
fn serve_connection(
    conn: ConnId,
    stream: UnixStream,
    outputs: &Outputs,
    job_tx: &Sender<Control>,
    live: &Arc<Mutex<usize>>,
    socket_path: &Path,
) {
    let end = read_loop(conn, BufReader::new(stream), outputs, job_tx).unwrap_or(ConnEnd::Eof);
    let mut n = live.lock().expect("连接计数锁中毒");
    *n -= 1;
    let all_gone = *n == 0;
    drop(n);

    match end {
        // EOF：这个客户端走了，通道即可注销（写线程随之收尾）
        ConnEnd::Eof => {
            outputs.lock().expect("输出表锁中毒").remove(&conn);
        }
        // shutdown：通道**留着** —— 内核的 ShutdownComplete 还要广播到它，
        // 由 serve_unix 收尾时统一清
        ConnEnd::Shutdown => {}
    }

    if all_gone || matches!(end, ConnEnd::Shutdown) {
        SHUTDOWN.store(true, Ordering::SeqCst);
        // 一次自连唤醒阻塞的 accept（见 SHUTDOWN 的注释）；连接失败就让它
        // 继续阻塞到有真客户端 —— 反正服务已经在收尾，不值得为此轮询
        let _ = UnixStream::connect(socket_path);
    }
    if matches!(end, ConnEnd::Shutdown) {
        let _ = job_tx.send(Control::Quit);
    }
}

/// 请求信封（JSON-RPC 2.0 的成员）。
struct Envelope {
    /// `None` = 客户端发来的**通知**（无 id）：按规范不回响应。
    id: Option<Value>,
    method: String,
    params: Value,
}

fn envelope(value: &Value) -> Result<Envelope, RpcError> {
    let obj = value
        .as_object()
        .ok_or_else(|| RpcError::new(INVALID_REQUEST, "请求必须是 JSON 对象"))?;
    match obj.get("jsonrpc") {
        Some(Value::String(v)) if v == JSONRPC_VERSION => {}
        Some(other) => {
            return Err(RpcError::new(
                INVALID_REQUEST,
                format!("jsonrpc 必须是 \"{JSONRPC_VERSION}\"，收到 {other}"),
            ))
        }
        None => {
            return Err(RpcError::new(
                INVALID_REQUEST,
                format!("缺 jsonrpc 字段（本协议只接受 JSON-RPC {JSONRPC_VERSION}）"),
            ))
        }
    }
    let method = obj
        .get("method")
        .and_then(Value::as_str)
        .ok_or_else(|| RpcError::new(INVALID_REQUEST, "缺 method 字段（必须是字符串）"))?
        .to_string();
    let id = match obj.get("id") {
        None | Some(Value::Null) => None,
        Some(v) if v.is_string() || v.is_number() => Some(v.clone()),
        Some(other) => {
            return Err(RpcError::new(
                INVALID_REQUEST,
                format!("id 必须是字符串或数字：{other}"),
            ))
        }
    };
    // 参数缺省 = 空对象（与 JSON-RPC 的"省略 params"一致）
    let params = obj.get("params").cloned().unwrap_or_else(|| json!({}));
    Ok(Envelope { id, method, params })
}

/// 一条连接的结束方式（读线程的退出原因）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnEnd {
    /// 客户端关掉了流（EOF / 进程退出）：只是这个客户端走了
    Eof,
    /// 客户端请求了 `shutdown`：内核正在收尾，广播还会来
    Shutdown,
}

/// 读取循环：逐行读、逐行分派。
///
/// 返回即这条连接读完了：[`ConnEnd::Eof`]（对端关流）或 [`ConnEnd::Shutdown`]。
fn read_loop<R: BufRead>(
    conn: ConnId,
    mut input: R,
    outputs: &Outputs,
    job_tx: &Sender<Control>,
) -> std::io::Result<ConnEnd> {
    let mut initialized = false;
    let mut buf: Vec<u8> = Vec::new();
    loop {
        match read_line_bounded(&mut input, &mut buf)? {
            LineRead::Eof => return Ok(ConnEnd::Eof),
            LineRead::TooLong { discarded } => {
                // 该行连解析都没做，没有可信的 id，按规范用 null
                send_to(
                    outputs,
                    conn,
                    error_line(
                        Value::Null,
                        &RpcError::with_data(
                            INVALID_REQUEST,
                            format!(
                                "请求行超过上限（{MAX_REQUEST_BYTES} 字节），已整行丢弃（{} 字节）",
                                discarded
                            ),
                            json!({ "limit": MAX_REQUEST_BYTES, "discarded_bytes": discarded }),
                        ),
                    ),
                );
                continue;
            }
            LineRead::Line => {}
        }
        // 非 UTF-8 不做特殊处理：替换字符会让 JSON 解析失败，从而走到 PARSE_ERROR
        // —— 那正是我们想给客户端的答复（比 panic 好在有回执、连接仍在）。
        let text = String::from_utf8_lossy(&buf);
        let line = text.trim();
        if line.is_empty() {
            continue; // 空行忽略（便于人工用 echo 调试）
        }
        let value: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                // 整行不是 JSON：没有可信的 id，按规范用 null
                send_to(outputs, conn, error_line(Value::Null, &RpcError::new(PARSE_ERROR, format!("不是合法 JSON：{e}"))));
                continue;
            }
        };
        let envelope = match envelope(&value) {
            Ok(e) => e,
            Err(e) => {
                let id = value.get("id").cloned().unwrap_or(Value::Null);
                send_to(outputs, conn, error_line(id, &e));
                continue;
            }
        };
        // 客户端通知（无 id）：本协议不定义客户端通知（唯一"发完就不管"的方向
        // 是服务端→客户端的事件流），按规范静默忽略而不是回一个错。
        let Some(id) = envelope.id else { continue };

        match jsonrpc::dispatch(&envelope.method, &envelope.params) {
            Err(e) => {
                send_to(outputs, conn, error_line(id, &e));
            }
            Ok(Action::Initialize(params)) => {
                // 握手是**每连接**的事：多客户端模式下每条连接各自握一次手
                if initialized {
                    send_to(
                        outputs,
                        conn,
                        error_line(
                            id,
                            &RpcError::new(ALREADY_INITIALIZED, "本连接已握手过：一个连接只握手一次"),
                        ),
                    );
                    continue;
                }
                match jsonrpc::initialize_result(&params) {
                    Ok(mut result) => {
                        // 宿主能力声明：客户端据此决定怎么渲染（Hunk diff 还是
                        // 纯文本、能否弹交互审批）。这是 SPI 的既有数据，
                        // 不是为这条协议新造的字段。
                        result["host"] = crate::host_capabilities_json();
                        initialized = true;
                        send_to(outputs, conn, result_line(id, result));
                    }
                    // 版本不匹配：**不置 initialized**（客户端改正后可重来）
                    Err(e) => {
                        send_to(outputs, conn, error_line(id, &e));
                    }
                }
            }
            Ok(Action::Submit(op)) => {
                if !initialized {
                    send_to(outputs, conn, error_line(id, &not_initialized(&envelope.method)));
                    continue;
                }
                if job_tx.send(Control::Job { conn, job: Job::Op(op) }).is_err() {
                    send_to(
                        outputs,
                        conn,
                        error_line(id, &RpcError::new(KERNEL_ERROR, "内核通道已断（宿主正在退出）")),
                    );
                    continue;
                }
                // 受理 ≠ 完成：整轮进度以事件通知到达
                send_to(outputs, conn, result_line(id, json!({ "accepted": true })));
            }
            Ok(Action::Thread(cmd)) => {
                if !initialized {
                    send_to(outputs, conn, error_line(id, &not_initialized(&envelope.method)));
                    continue;
                }
                // thread/* 是**同步**的：响应由内核线程在处理完后写回，
                // 这里不提前发 accepted（list/get 没有后续事件可推）。
                if job_tx.send(Control::Job { conn, job: Job::Thread { id: id.clone(), cmd } }).is_err() {
                    send_to(
                        outputs,
                        conn,
                        error_line(id, &RpcError::new(KERNEL_ERROR, "内核通道已断（宿主正在退出）")),
                    );
                }
            }
            Ok(Action::Shutdown) => {
                if !initialized {
                    send_to(outputs, conn, error_line(id, &not_initialized(&envelope.method)));
                    continue;
                }
                // 关停的是**共用的那个内核**：Op::Shutdown 让内核发 ShutdownComplete
                //（广播给所有连接，含本条），随后整个服务收尾
                let _ = job_tx.send(Control::Job { conn, job: Job::Op(Op::Shutdown) });
                send_to(outputs, conn, result_line(id, json!({ "accepted": true })));
                return Ok(ConnEnd::Shutdown);
            }
        }
    }
}

fn not_initialized(method: &str) -> RpcError {
    RpcError::with_data(
        NOT_INITIALIZED,
        format!("{method} 需要在 initialize 之后调用"),
        json!({ "expected": "initialize" }),
    )
}

/// 一次有界读取的结果。
enum LineRead {
    /// 读到一行（若以换行结束，换行符保留在缓冲里）
    Line,
    /// 输入到末尾
    Eof,
    /// 超过 [`MAX_REQUEST_BYTES`]：该行已被**整行**丢弃（不解析、不缓冲）
    TooLong {
        /// 该行的总字节数（含换行符）—— 客户端据此知道"被丢掉的是这一条的多少字节"
        discarded: usize,
    },
}

/// 有界读一行：最多读 `MAX_REQUEST_BYTES + 1` 字节。
///
/// 超限时**必须把该行剩余部分读掉丢弃**，而不是就此返回：否则残片会被当成
/// 下一条请求去解析，客户端会收到一串与真实原因无关的 parse error ——
/// 那种错误比"超限"难查得多。
///
/// 只用 `fill_buf`/`consume` 逐块走，不用 `read_until`/`take`：后两者会按值
/// 拿走读取端（`&mut R` 上的借用解析会变成"移出可变引用"，编译不过），
/// 而逐块走还能让"超限"在**读到一半时**就决定丢弃，不必先把整行读进内存。
fn read_line_bounded<R: BufRead>(input: &mut R, buf: &mut Vec<u8>) -> std::io::Result<LineRead> {
    buf.clear();
    loop {
        // 先只取三个标量（有没有换行 / 换行在哪 / 本段多长），拿到后立刻结束
        // 对 input 的不可变借用 —— 后面还要 consume 与丢弃，不能同时持有借用。
        let probe = {
            let available = input.fill_buf()?;
            if available.is_empty() {
                None
            } else {
                Some((available.iter().position(|&b| b == b'\n'), available.len()))
            }
        };
        let Some((newline_at, chunk_len)) = probe else {
            // EOF：已经读到内容就算一行（最后一行可能没有换行符）
            return Ok(if buf.is_empty() { LineRead::Eof } else { LineRead::Line });
        };
        let taken = newline_at.map_or(chunk_len, |i| i + 1); // 含换行符
        if buf.len() + taken > MAX_REQUEST_BYTES {
            input.consume(taken);
            // 换行符若已随这一段消费掉，这行到此为止，**不能再 drain** ——
            // 继续丢就会把下一条合法请求一起吞掉（集成测试抓到的真 bug）。
            let extra = if newline_at.is_some() { 0 } else { drain_until_newline(input)? };
            return Ok(LineRead::TooLong { discarded: taken + extra });
        }
        // 重新借一次只为取字节：`available` 的最后一次使用在 extend 那一行，
        // 因此紧接着的 `consume` 不会与它冲突（NLL）。
        let available = input.fill_buf()?;
        buf.extend_from_slice(&available[..taken]);
        input.consume(taken);
        if newline_at.is_some() {
            return Ok(LineRead::Line);
        }
    }
}

/// 丢弃直到行尾（含换行），返回丢弃字节数。
///
/// 用 `fill_buf`/`consume` 逐块扫，**不累积**任何数据 —— 一旦累积，
/// 内存就又交回给输入长度了（这正是本函数要防的事）。
fn drain_until_newline<R: BufRead>(input: &mut R) -> std::io::Result<usize> {
    let mut discarded = 0usize;
    loop {
        let available = input.fill_buf()?;
        if available.is_empty() {
            return Ok(discarded); // 对端没写换行就关了
        }
        match available.iter().position(|&b| b == b'\n') {
            Some(i) => {
                input.consume(i + 1);
                return Ok(discarded + i + 1);
            }
            None => {
                let len = available.len();
                input.consume(len);
                discarded += len;
            }
        }
    }
}
