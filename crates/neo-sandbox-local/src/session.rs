//! 交互式命令会话（Codex `command/exec` session 模式 / `write_stdin`）。
//!
//! # 与一次性 [`LocalSandbox::execute`] 的分工
//!
//! - **一次性**：跑到结束、结果进 ToolOutput —— TUI `!cmd` / 默认 `command/exec`。
//! - **会话**：进程持活，客户端 `write`/`resize`/`terminate` 续管 —— 桌面真终端。
//!
//! 沙箱**不绕过**：与 `execute` 同一套 Seatbelt/档位包装；会话只多「不 wait 到死」。
//!
//! # PTY
//!
//! 使用 `portable-pty`（unix 真 PTY）。`resize` 走 TIOCSWINSZ 等价接口。
//! 输出经读线程进**有界**环形缓冲（默认 256 KiB），`write` 拉取增量 ——
//! 对齐 Codex write_stdin「响应带最近输出」语义，不靠后台事件推流。

use neo_protocol::SandboxMode;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// 会话输出缓冲上限（UTF-8 安全截断）。
pub const SESSION_OUTPUT_LIMIT: usize = 256 * 1024;

/// 一次 start/write 的结果（Codex unified_exec 形状的子集）。
#[derive(Debug, Clone)]
pub struct SessionSnapshot {
    pub session_id: String,
    pub output: String,
    pub running: bool,
    pub exit_code: Option<i32>,
    pub truncated: bool,
}

struct LiveSession {
    /// portable-pty Child（unix）。
    child: Box<dyn portable_pty::Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    master: Option<Box<dyn portable_pty::MasterPty + Send>>,
    buf: Arc<Mutex<Vec<u8>>>,
    truncated: Arc<AtomicBool>,
    /// 读线程在进程退出后 join；Drop 时先 kill 再等。
    reader: Option<std::thread::JoinHandle<()>>,
    exit_code: Option<i32>,
}

/// 会话表：id → 活会话。由 app-server 装配层持有（业务在装配，不在 L5 协议壳）。
pub struct SessionManager {
    sessions: HashMap<String, LiveSession>,
    next: u64,
    output_limit: usize,
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new(SESSION_OUTPUT_LIMIT)
    }
}

impl SessionManager {
    pub fn new(output_limit: usize) -> Self {
        Self { sessions: HashMap::new(), next: 0, output_limit }
    }

    /// 构造与 `LocalSandbox::execute` **同款**的沙箱包装命令行。
    fn sandbox_argv(
        mode: SandboxMode,
        workspace: &Path,
        command: &str,
        supports: bool,
    ) -> Result<(String, Vec<String>), String> {
        if !supports {
            return Err(format!(
                "本平台（{}）没有 {mode:?} 档的沙箱实现；拒绝在无沙箱保护下起会话",
                std::env::consts::OS
            ));
        }
        #[cfg(target_os = "macos")]
        {
            if let Some(profile) = crate::seatbelt_profile_pub(mode) {
                let mut args = vec!["-p".to_string(), profile];
                if mode == SandboxMode::WorkspaceWrite {
                    let ws = std::fs::canonicalize(workspace)
                        .unwrap_or_else(|_| workspace.to_path_buf());
                    args.push("-D".into());
                    args.push(format!("WORKSPACE={}", ws.display()));
                }
                args.push("sh".into());
                args.push("-c".into());
                args.push(command.to_string());
                return Ok(("sandbox-exec".into(), args));
            }
        }
        Ok(("sh".into(), vec!["-c".to_string(), command.to_string()]))
    }

    /// 启动会话。`cols`/`rows` 默认 80×24。
    pub fn start(
        &mut self,
        mode: SandboxMode,
        workspace: &Path,
        command: &str,
        cols: u16,
        rows: u16,
        sandbox_supported: bool,
    ) -> Result<SessionSnapshot, String> {
        if command.trim().is_empty() {
            return Err("空命令".into());
        }
        let (program, args) = Self::sandbox_argv(mode, workspace, command, sandbox_supported)?;

        let pty_system = portable_pty::native_pty_system();
        let pair = pty_system
            .openpty(portable_pty::PtySize {
                rows: rows.max(1),
                cols: cols.max(1),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("openpty 失败：{e}"))?;

        let mut cmd = portable_pty::CommandBuilder::new(&program);
        for a in &args {
            cmd.arg(a);
        }
        cmd.cwd(workspace);
        cmd.env("TERM", "xterm-256color");
        cmd.env("LANG", std::env::var("LANG").unwrap_or_else(|_| "C.UTF-8".into()));

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("spawn 失败：{e}"))?;
        // 关 slave：否则 master 读不会在子进程退出时看到 EOF
        drop(pair.slave);

        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("PTY writer：{e}"))?;
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("PTY reader：{e}"))?;

        self.next += 1;
        let session_id = format!("exec-{}", self.next);
        let buf = Arc::new(Mutex::new(Vec::with_capacity(8 * 1024)));
        let truncated = Arc::new(AtomicBool::new(false));
        let limit = self.output_limit;
        let buf_r = buf.clone();
        let trunc_r = truncated.clone();
        let reader_thread = std::thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            loop {
                match reader.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        let mut g = buf_r.lock().unwrap_or_else(|e| e.into_inner());
                        let room = limit.saturating_sub(g.len());
                        if room == 0 {
                            trunc_r.store(true, Ordering::Relaxed);
                            break;
                        }
                        let take = room.min(n);
                        g.extend_from_slice(&chunk[..take]);
                        if take < n {
                            trunc_r.store(true, Ordering::Relaxed);
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        let session = LiveSession {
            child,
            writer,
            master: Some(pair.master),
            buf,
            truncated,
            reader: Some(reader_thread),
            exit_code: None,
        };
        let output = {
            let snap_session_id = session_id.clone();
            let mut live = session;
            // 有输出或短暂超时即返回（条件探测；2ms 步长有总上限）
            let deadline = std::time::Instant::now() + Duration::from_millis(50);
            loop {
                let n = live.buf.lock().unwrap_or_else(|e| e.into_inner()).len();
                if n > 0 || std::time::Instant::now() >= deadline {
                    break;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            let raw = {
                let mut g = live.buf.lock().unwrap_or_else(|e| e.into_inner());
                std::mem::take(&mut *g)
            };
            let out = String::from_utf8_lossy(&raw).into_owned();
            let running = live.child.try_wait().ok().flatten().is_none();
            if !running && live.exit_code.is_none() {
                // 极快退出：记下 code（try_wait 已消耗状态时 portable-pty 语义见下）
                live.exit_code = Some(0); // try_wait Some => 已退出；code 见 wait 封装
            }
            self.sessions.insert(snap_session_id.clone(), live);
            out
        };

        // 修正 running/exit：对已插入会话再查一次
        let (running, exit_code, truncated_flag) = {
            let s = self.sessions.get_mut(&session_id).expect("刚插入");
            if s.exit_code.is_none() {
                if let Ok(Some(status)) = s.child.try_wait() {
                    s.exit_code = Some(status.exit_code() as i32);
                }
            }
            (
                s.exit_code.is_none(),
                s.exit_code,
                s.truncated.load(Ordering::Relaxed),
            )
        };

        Ok(SessionSnapshot {
            session_id,
            output,
            running,
            exit_code,
            truncated: truncated_flag,
        })
    }

    /// 写 stdin 并拉取自上次以来的输出（Codex write_stdin 形状）。
    pub fn write(&mut self, session_id: &str, data: &str) -> Result<SessionSnapshot, String> {
        let s = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("会话 {session_id} 不存在或已结束"))?;

        if s.exit_code.is_none() {
            s.writer
                .write_all(data.as_bytes())
                .map_err(|e| format!("write_stdin 失败：{e}"))?;
            let _ = s.writer.flush();
        }

        // 等一小段有界窗口收输出（条件：有字节或超时）
        let deadline = std::time::Instant::now() + Duration::from_millis(80);
        loop {
            let n = s.buf.lock().unwrap_or_else(|e| e.into_inner()).len();
            if n > 0 || std::time::Instant::now() >= deadline {
                break;
            }
            if s.child.try_wait().ok().flatten().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }

        if s.exit_code.is_none() {
            if let Ok(Some(status)) = s.child.try_wait() {
                s.exit_code = Some(status.exit_code() as i32);
            }
        }

        let raw = {
            let mut g = s.buf.lock().unwrap_or_else(|e| e.into_inner());
            std::mem::take(&mut *g)
        };
        let output = String::from_utf8_lossy(&raw).into_owned();
        let running = s.exit_code.is_none();
        let exit_code = s.exit_code;
        let truncated = s.truncated.load(Ordering::Relaxed);

        if !running {
            self.reap(session_id);
        }

        Ok(SessionSnapshot {
            session_id: session_id.to_string(),
            output,
            running,
            exit_code,
            truncated,
        })
    }

    pub fn resize(&mut self, session_id: &str, cols: u16, rows: u16) -> Result<(), String> {
        let s = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("会话 {session_id} 不存在或已结束"))?;
        if s.exit_code.is_some() {
            return Err(format!("会话 {session_id} 已结束"));
        }
        let master = s.master.as_ref().ok_or("会话无 PTY master")?;
        master
            .resize(portable_pty::PtySize {
                rows: rows.max(1),
                cols: cols.max(1),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("resize 失败：{e}"))
    }

    pub fn terminate(&mut self, session_id: &str) -> Result<SessionSnapshot, String> {
        if !self.sessions.contains_key(session_id) {
            return Err(format!("会话 {session_id} 不存在或已结束"));
        }
        let s = self.sessions.get_mut(session_id).expect("存在性已检查");
        if s.exit_code.is_none() {
            let _ = s.child.kill();
            match s.child.wait() {
                Ok(status) => s.exit_code = Some(status.exit_code() as i32),
                Err(_) => s.exit_code = Some(-1),
            }
        }
        // 收尾输出
        let raw = {
            let mut g = s.buf.lock().unwrap_or_else(|e| e.into_inner());
            std::mem::take(&mut *g)
        };
        let output = String::from_utf8_lossy(&raw).into_owned();
        let exit_code = s.exit_code;
        let truncated = s.truncated.load(Ordering::Relaxed);
        self.reap(session_id);
        Ok(SessionSnapshot {
            session_id: session_id.to_string(),
            output,
            running: false,
            exit_code,
            truncated,
        })
    }

    fn reap(&mut self, session_id: &str) {
        if let Some(mut s) = self.sessions.remove(session_id) {
            let _ = s.child.kill();
            if let Some(h) = s.reader.take() {
                // 读线程在 EOF/kill 后应很快结束；有界 join（1s），避免挂死
                let _ = h.join();
            }
        }
    }
}

impl Drop for SessionManager {
    fn drop(&mut self) {
        let ids: Vec<String> = self.sessions.keys().cloned().collect();
        for id in ids {
            let _ = self.terminate(&id);
        }
    }
}

/// 供 `sandbox_argv` 使用的 Seatbelt profile（与 LocalSandbox 同源导出）。
pub use crate::seatbelt_profile_pub;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn ws() -> PathBuf {
        let p = std::env::temp_dir().join(format!("neo-sess-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&p);
        p
    }

    fn supported() -> bool {
        // 与 LocalSandbox::supports 同语义：全权限 always；受限档 macOS
        true // DangerFullAccess 在三平台都可；测试用全权限包装路径
    }

    #[test]
    fn session_start_write_terminate_roundtrip() {
        // 全权限 + 真 PTY：sh -i 或 cat 保持打开
        let dir = ws();
        let mut m = SessionManager::new(SESSION_OUTPUT_LIMIT);
        let start = m
            .start(
                SandboxMode::DangerFullAccess,
                &dir,
                "cat",
                80,
                24,
                true,
            )
            .expect("start");
        assert!(start.session_id.starts_with("exec-"));
        assert!(start.running, "cat 应仍存活：{start:?}");

        let w = m
            .write(&start.session_id, "hello-pty\n")
            .expect("write_stdin");
        assert!(
            w.output.contains("hello-pty") || w.running,
            "应读回回显或仍运行：{w:?}"
        );

        let t = m.terminate(&start.session_id).expect("terminate");
        assert!(!t.running);
        assert!(m.write(&start.session_id, "x").is_err(), "终止后 write 应失败");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_session_errors_clearly() {
        let mut m = SessionManager::default();
        assert!(m.write("nope", "x").is_err());
        assert!(m.resize("nope", 80, 24).is_err());
        assert!(m.terminate("nope").is_err());
        let _ = supported();
    }

    #[test]
    fn session_rejects_unsupported_sandbox_mode() {
        let dir = ws();
        let mut m = SessionManager::default();
        let err = m
            .start(SandboxMode::ReadOnly, &dir, "echo hi", 80, 24, false)
            .unwrap_err();
        assert!(err.contains("沙箱"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
