//! stdio 传输：请求行进，响应行 + 事件通知行出。
//!
//! # 线程模型（与 Web 宿主同一条：内核独占线程）
//!
//! ```text
//!  读取（本线程）──mpsc(Op)──▶ 内核线程 ──mpsc(行)──▶ 写线程 ──▶ stdout
//!        └──────────── 响应行 ────────────────┘
//! ```
//!
//! 三条不变量：
//! 1. **stdout 上只有协议行**：诊断一律走 stderr。任何一句 `println!` 都会
//!    让客户端解析失败 —— 这是 stdio 协议最容易踩的坑。
//! 2. **响应与通知共用一条通道**：两种行都经同一个 mpsc 进写线程，因此
//!    "一行"始终完整，不会出现两个线程交叉写坏半行。
//! 3. **响应表示已受理，不表示已完成**：`turn/start` 立即返回 `accepted`，
//!    整轮进度以事件通知的形式陆续到达（与 Codex app-server 同款语义）。
//!    这样客户端不必在"等到整轮跑完"和"界面冻结"之间二选一。
//!
//! # 为什么不需要"事件重放/断线续订"
//!
//! stdio 连接的生命期就是进程生命期：客户端没有"断线重连到同一会话"这回事
//! （重连 = 新进程 = 新会话）。Web 宿主需要游标，是因为浏览器标签会重连 ——
//! 那是 SSE 的问题，不是 stdio 的。真要做跨进程续聊，正道是读会话日志
//! （JSONL）重建，而不是在这里加缓冲区。
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
//! 一句不带换行的巨型输入就能把宿主的内存吃光 —— stdio 客户端虽然是本地进程，
//! 但"新增可能产生大输出的路径必须受上限约束"这条义务与对端是谁无关。

use std::io::{BufRead, Write};
use std::sync::mpsc::{channel, Receiver, Sender};

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
    let (out_tx, out_rx) = channel::<String>();
    // (请求 id, 工作项)：thread/* 要用 id 回响应行；Op 的 id 由读线程自己回 accepted
    let (job_tx, job_rx) = channel::<Job>();

    let writer = std::thread::spawn(move || write_lines(output, out_rx));

    let events_tx = out_tx.clone();
    let kernel = std::thread::spawn(move || {
        let mut seq: u64 = 0;
        while let Ok(job) = job_rx.recv() {
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
            if let Some(line) = reply {
                if events_tx.send(line).is_err() {
                    return;
                }
            }
            for event in events {
                seq += 1;
                let line = notification(seq, &event).to_string();
                if events_tx.send(line).is_err() {
                    return; // 写端已走
                }
            }
        }
    });

let result = read_loop(input, &out_tx, &job_tx);

    // 收尾顺序：先关 Op 通道（内核跑完手头这一批），再关行通道（写线程写完）
    drop(job_tx);
    let _ = kernel.join();
    drop(out_tx);
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

/// 读取循环：逐行读、逐行分派。返回即"客户端 EOF 或请求了关停"。
fn read_loop<R: BufRead>(
    mut input: R,
    out_tx: &Sender<String>,
    job_tx: &Sender<Job>,
) -> std::io::Result<()> {
    let mut initialized = false;
    let mut buf: Vec<u8> = Vec::new();
    loop {
        match read_line_bounded(&mut input, &mut buf)? {
            LineRead::Eof => return Ok(()),
            LineRead::TooLong { discarded } => {
                // 该行连解析都没做，没有可信的 id，按规范用 null
                let _ = out_tx.send(error_line(
                    Value::Null,
                    &RpcError::with_data(
                        INVALID_REQUEST,
                        format!(
                            "请求行超过上限（{MAX_REQUEST_BYTES} 字节），已整行丢弃（{} 字节）",
                            discarded
                        ),
                        json!({ "limit": MAX_REQUEST_BYTES, "discarded_bytes": discarded }),
                    ),
                ));
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
                let _ = out_tx.send(error_line(
                    Value::Null,
                    &RpcError::new(PARSE_ERROR, format!("不是合法 JSON：{e}")),
                ));
                continue;
            }
        };
        let envelope = match envelope(&value) {
            Ok(e) => e,
            Err(e) => {
                let id = value.get("id").cloned().unwrap_or(Value::Null);
                let _ = out_tx.send(error_line(id, &e));
                continue;
            }
        };
        // 客户端通知（无 id）：本协议不定义客户端通知（唯一"发完就不管"的方向
        // 是服务端→客户端的事件流），按规范静默忽略而不是回一个错。
        let Some(id) = envelope.id else { continue };

        match jsonrpc::dispatch(&envelope.method, &envelope.params) {
            Err(e) => {
                let _ = out_tx.send(error_line(id, &e));
            }
            Ok(Action::Initialize(params)) => {
                if initialized {
                    let _ = out_tx.send(error_line(
                        id,
                        &RpcError::new(ALREADY_INITIALIZED, "本连接已握手过：一个连接只握手一次"),
                    ));
                    continue;
                }
                match jsonrpc::initialize_result(&params) {
                    Ok(mut result) => {
                        // 宿主能力声明：客户端据此决定怎么渲染（Hunk diff 还是
                        // 纯文本、能否弹交互审批）。这是 SPI 的既有数据，
                        // 不是为这条协议新造的字段。
                        result["host"] = crate::host_capabilities_json();
                        initialized = true;
                        let _ = out_tx.send(result_line(id, result));
                    }
                    // 版本不匹配：**不置 initialized**（客户端改正后可重来）
                    Err(e) => {
                        let _ = out_tx.send(error_line(id, &e));
                    }
                }
            }
            Ok(Action::Submit(op)) => {
                if !initialized {
                    let _ = out_tx.send(error_line(id, &not_initialized(&envelope.method)));
                    continue;
                }
                if job_tx.send(Job::Op(op)).is_err() {
                    let _ = out_tx.send(error_line(
                        id,
                        &RpcError::new(KERNEL_ERROR, "内核通道已断（宿主正在退出）"),
                    ));
                    continue;
                }
                // 受理 ≠ 完成：整轮进度以事件通知到达
                let _ = out_tx.send(result_line(id, json!({ "accepted": true })));
            }
            Ok(Action::Thread(cmd)) => {
                if !initialized {
                    let _ = out_tx.send(error_line(id, &not_initialized(&envelope.method)));
                    continue;
                }
                // thread/* 是**同步**的：响应由内核线程在处理完后写回，
                // 这里不提前发 accepted（list/get 没有后续事件可推）。
                if job_tx.send(Job::Thread { id: id.clone(), cmd }).is_err() {
                    let _ = out_tx.send(error_line(
                        id,
                        &RpcError::new(KERNEL_ERROR, "内核通道已断（宿主正在退出）"),
                    ));
                }
            }
            Ok(Action::Shutdown) => {
                if !initialized {
                    let _ = out_tx.send(error_line(id, &not_initialized(&envelope.method)));
                    continue;
                }
                let _ = job_tx.send(Job::Op(Op::Shutdown));
                let _ = out_tx.send(result_line(id, json!({ "accepted": true })));
                // 停止读取：内核会把 ShutdownComplete 发出来，写线程写完即收尾
                return Ok(());
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
