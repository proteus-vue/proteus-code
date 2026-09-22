//! `neo app-server` 真实二进制端到端：真内核 + 真沙箱 + 确定性桩 provider。
//!
//! 与 `neo-host-appserver` 的进程内测试互补 —— 那些用脚本化的假内核把**协议**
//! 逐条钉死，这条证明**装配是真的**：CLI 子命令 → `build_kernel` → stdio 传输
//! → 内核真的跑完一轮并落盘。
//!
//! 不用固定等待：每一行输出由读线程推进通道，主线程按条件 `recv_timeout`
//! 等目标行（超时即失败）。固定 `sleep` 在这里既慢又不稳（机器忙时假失败）。

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// 唯一临时工作区。**不能写固定路径**：同机并行跑测试时，两个用例会互相删目录
/// （这个 flaky 在本仓真实发生过）。
fn unique_workspace() -> PathBuf {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).expect("系统时钟").as_nanos();
    std::env::temp_dir().join(format!("neo-appserver-e2e-{}-{nanos}", std::process::id()))
}

/// 会话收尾：无论断言是否失败都杀掉子进程、删掉临时目录。
struct Session {
    child: Child,
    ws: PathBuf,
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.ws);
    }
}

/// 等到某一行满足条件为止（其余行丢弃）。超时或进程提前退出都算失败。
fn wait_for(rx: &Receiver<String>, timeout: Duration, what: &str, pred: impl Fn(&str) -> bool) -> String {
    let deadline = std::time::Instant::now() + timeout;
    let mut seen: Vec<String> = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            panic!("等 {what} 超时；已收到的行：{seen:#?}");
        }
        match rx.recv_timeout(left) {
            Ok(line) => {
                if pred(&line) {
                    return line;
                }
                seen.push(line);
            }
            Err(RecvTimeoutError::Timeout) => continue, // 下一轮 left 归零 → 报超时
            Err(RecvTimeoutError::Disconnected) => {
                panic!("等 {what} 时子进程已关闭 stdout；已收到的行：{seen:#?}");
            }
        }
    }
}

fn send(stdin: &mut impl Write, line: &str) {
    stdin.write_all(line.as_bytes()).expect("写 stdin");
    stdin.write_all(b"\n").expect("写换行");
    stdin.flush().expect("flush");
}

#[test]
fn real_binary_handshakes_runs_a_turn_and_shuts_down() {
    let ws = unique_workspace();
    std::fs::create_dir_all(&ws).expect("建临时工作区");

    let child = Command::new(env!("CARGO_BIN_EXE_neo"))
        .args(["app-server", "--provider", "mock", "--workspace"])
        .arg(&ws)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("启动 neo app-server");
    let mut session = Session { child, ws: ws.clone() };

    let mut stdin = session.child.stdin.take().expect("取 stdin");
    let stdout = session.child.stdout.take().expect("取 stdout");
    let (tx, rx) = channel::<String>();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            match line {
                Ok(l) => {
                    if tx.send(l).is_err() {
                        return;
                    }
                }
                Err(_) => return,
            }
        }
    });

    // ① 握手：不握手就发请求会被拒（这里直接走正确路径）
    send(&mut stdin, r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);
    let init = wait_for(&rx, Duration::from_secs(30), "initialize 响应", |l| l.contains(r#""id":1"#));
    assert!(init.contains("\"protocol_version\""), "握手响应应带协议版本：{init}");
    assert!(init.contains("turn/start"), "握手响应应带方法表：{init}");

    // ② 提交一轮（桩 provider：一句固定文本，不调真实模型）
    send(&mut stdin, r#"{"jsonrpc":"2.0","id":2,"method":"turn/start","params":{"text":"你好"}}"#);
    let accepted = wait_for(&rx, Duration::from_secs(30), "turn/start 响应", |l| l.contains(r#""id":2"#));
    assert!(accepted.contains("accepted"), "响应语义是「已受理」：{accepted}");

    // ③ 内核回显用户消息 → 模型答复 → 轮结束：证明真跑完了一轮
    let echoed = wait_for(&rx, Duration::from_secs(60), "user_submitted 通知", |l| l.contains("user_submitted"));
    assert!(echoed.contains("你好"), "回显应带用户原话：{echoed}");
    let answered = wait_for(&rx, Duration::from_secs(60), "agent_message_done 通知", |l| l.contains("agent_message_done"));
    assert!(answered.contains("mock provider"), "桩 provider 的文本应送达：{answered}");
    wait_for(&rx, Duration::from_secs(60), "turn_complete 通知", |l| l.contains("turn_complete"));

    // ④ 会话真的落盘了（AGENTS.md「模型可见即已落日志」：这一轮必须能回放）。
    // 文件名跟着会话库走（`SessionStore::path_for`，起动 id 是 `appserver`）——
    // thread/* 那轮把固定名改成了会话库命名，这里的断言要跟上，否则红的是路径不是功能
    let log = ws.join(".neo/sessions/appserver.jsonl");
    let text = std::fs::read_to_string(&log).unwrap_or_else(|e| panic!("会话日志 {} 读不到：{e}", log.display()));
    assert!(text.contains("user_submitted"), "日志应含用户消息：{text}");

    // ⑤ 关停：内核确认后进程自己退出
    send(&mut stdin, r#"{"jsonrpc":"2.0","id":3,"method":"shutdown","params":{}}"#);
    wait_for(&rx, Duration::from_secs(30), "shutdown_complete 通知", |l| l.contains("shutdown_complete"));
    drop(stdin);

    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        match session.child.try_wait().expect("try_wait") {
            Some(status) => {
                assert!(status.success(), "关停应正常退出，实际 {status}");
                break;
            }
            None if std::time::Instant::now() >= deadline => panic!("关停后 30s 进程仍未退出"),
            None => std::thread::sleep(Duration::from_millis(50)), // 有界轮询（上限见上）
        }
    }
}
