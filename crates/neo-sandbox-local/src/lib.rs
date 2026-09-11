//! L3 PROVIDER —— `SandboxBackend` 的本机实现（OS 级强制）
//!
//! # 三个必须做对的地方（都是踩出来的）
//!
//! 1. **fail-closed**：请求受限档位但平台无可用沙箱 → 返回 `Denied`，
//!    **绝不裸奔执行**。宁可跑不起来，不可悄悄不设防。
//! 2. **边读边限**：输出上限是 SPI 契约的一部分，实现必须在读取循环里就停止
//!    累积。注意达到上限后要**立即返回**，不能继续 drain —— 否则遇到
//!    `yes` 这类永不结束的输出源，读线程永不返回。
//! 3. **必须并发读取 stdout/stderr**：管道缓冲区（macOS 约 64 KB）写满后，
//!    子进程会**阻塞在写**上、永不退出。若先 `wait` 再读，就是经典管道死锁
//!    （本实现第一版踩过：测试挂到 120s 超时才失败）。
//!    两个读线程是 OS 语义的必需品，不是"加深调试链"。
//!
//! # 诚实边界（三平台不同构，不得声称等价）
//!
//! | 平台 | 机制 | 限制 |
//! |---|---|---|
//! | macOS | `sandbox-exec`（Seatbelt） | Apple 已标记 deprecated 但仍可用 |
//! | Linux | **未实现** | 需 Landlock + bwrap；当前 fail-closed |
//! | Windows | **未实现** | 需 restricted token + ACL |
//!
//! 受限档位在未实现平台会**拒绝执行**，而不是降级放行 —— 静默降级会让
//! "沙箱"变成一句空话。

use neo_core::{FileOutcome, SandboxBackend, SandboxOutcome};
use neo_protocol::SandboxMode;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

pub struct LocalSandbox {
    /// 可写根：`WorkspaceWrite` 档下**唯一**允许写入的范围。
    pub workspace: PathBuf,
    pub timeout: Duration,
}

impl LocalSandbox {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self { workspace: workspace.into(), timeout: DEFAULT_TIMEOUT }
    }
    pub fn with_timeout(mut self, t: Duration) -> Self { self.timeout = t; self }
}

/// Seatbelt profile。`None` 表示该档不需要沙箱（全权限档）。
///
/// 路径通过 `-D` 参数注入而非拼进字符串：路径里若含引号或反斜杠，
/// 直接拼接会破坏 profile 语法。`sandbox-exec -D k=v` 是官方途径。
///
/// **不放开 `/tmp`**（与 Codex 一致）：只放开显式声明的可写根。
/// 放开全局临时目录等于把 "workspace-write" 退化成 "几乎任意写" ——
/// 本实现第一版就这么写过，被真机测试当场抓出越权。
#[cfg(target_os = "macos")]
fn seatbelt_profile(mode: SandboxMode) -> Option<String> {
    const BASE: &str = r#"
(version 1)
(deny default)
(allow process-exec)
(allow process-fork)
(allow signal (target same-sandbox))
(allow process-info* (target same-sandbox))
(allow sysctl-read)
(allow mach-lookup)
(allow file-read*)
; 允许丢弃输出：`2>/dev/null` 极常见，不放开会让大量正常命令失败
(allow file-write-data
  (require-all (path "/dev/null") (vnode-type CHARACTER-DEVICE)))
(allow file-ioctl (literal "/dev/null"))
"#;

    match mode {
        SandboxMode::DangerFullAccess => None,
        SandboxMode::ReadOnly => Some(BASE.to_string()),
        SandboxMode::WorkspaceWrite => Some(format!(
            "{BASE}\n; 唯一的可写根：由内核注入的 workspace 参数\n(allow file-write* (subpath (param \"WORKSPACE\")))\n"
        )),
    }
}

/// 判断 `path` 是否落在 `root` 之内（安全判定的核心，故单测覆盖）。
///
/// 三个细节，少一个就会漏：
/// 1. **必须 canonicalize**：`/tmp` 在 macOS 上是 `/private/tmp` 的符号链接，
///    纯字符串前缀比较会把真实路径判成"在根之外"（反之也能用符号链接逃逸）。
/// 2. **目标可能还不存在**（新建文件/新建多级目录）：需**上溯到最近已存在的
///    祖先**再判定。只上溯一层不够 —— `root/a/b/c.txt` 里 `a/b` 都可能待建。
/// 3. **判定用的是祖先的真实路径**：若某个已存在的中间段是指向根外的符号链接，
///    canonicalize 会把它解析到根外，从而正确拒绝。
pub fn is_within(root: &Path, path: &Path) -> bool {
    let real_root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    match resolve_real(path) {
        Some(real_target) => real_target.starts_with(&real_root),
        None => false, // 解析不出来 → 宁可拒绝
    }
}

/// 解析出目标的"真实路径"，即使目标本身尚不存在。
///
/// 上溯到最近可 canonicalize 的祖先，再把尚未存在的路径段原样拼回。
fn resolve_real(path: &Path) -> Option<PathBuf> {
    if let Ok(p) = std::fs::canonicalize(path) {
        return Some(p);
    }
    let mut pending: Vec<std::ffi::OsString> = Vec::new();
    let mut cursor = path.to_path_buf();
    loop {
        let name = cursor.file_name()?.to_os_string();
        pending.push(name);
        cursor = cursor.parent()?.to_path_buf();
        if let Ok(real) = std::fs::canonicalize(&cursor) {
            let mut out = real;
            for seg in pending.iter().rev() {
                out.push(seg);
            }
            return Some(out);
        }
    }
}

/// 读一个管道到上限为止，**达到上限立即返回**。
///
/// 为什么立即返回而不是继续 drain：`yes` 这类输出源永不结束，继续 drain
/// 会让读线程永不返回，把超时机制架死。返回后读取端被 drop，
/// 子进程收到 SIGPIPE 而终止 —— 这既保住了内存有界，也保住了可终止性。
fn read_capped<R: Read>(mut r: R, limit: usize) -> (Vec<u8>, bool) {
    let mut kept: Vec<u8> = Vec::with_capacity(limit.min(64 * 1024));
    let mut buf = [0u8; 8192];
    loop {
        match r.read(&mut buf) {
            Ok(0) => return (kept, false),
            Ok(n) => {
                let room = limit.saturating_sub(kept.len());
                if room == 0 {
                    return (kept, true); // 已达上限 → 立即返回（drop 读取端）
                }
                let take = room.min(n);
                kept.extend_from_slice(&buf[..take]);
                if take < n {
                    return (kept, true);
                }
            }
            Err(_) => return (kept, false),
        }
    }
}

/// 字节还原成字符串，**不切开 UTF-8 码点**（上限可能落在多字节字符中间）。
fn lossy_tail(bytes: Vec<u8>, truncated: bool) -> (String, bool) {
    match String::from_utf8(bytes) {
        Ok(s) => (s, truncated),
        Err(e) => {
            let valid = e.utf8_error().valid_up_to();
            let mut v = e.into_bytes();
            v.truncate(valid);
            (String::from_utf8_lossy(&v).into_owned(), true)
        }
    }
}

impl SandboxBackend for LocalSandbox {
    fn supports(&self, mode: SandboxMode) -> bool {
        match mode {
            SandboxMode::DangerFullAccess => true,
            _ => cfg!(target_os = "macos"),
        }
    }

    fn write_file(&self, mode: SandboxMode, path: &Path, content: &str) -> FileOutcome {
        // 与命令执行同一套档位判定：读只档一律拒写
        match mode {
            SandboxMode::ReadOnly => {
                return FileOutcome::Denied {
                    reason: "只读档禁止写文件（沙箱是技术边界，审批策略无法放行）".into(),
                }
            }
            SandboxMode::WorkspaceWrite => {
                if !is_within(&self.workspace, path) {
                    return FileOutcome::Denied {
                        reason: format!(
                            "路径在工作区之外：{}（工作区 {}）",
                            path.display(),
                            self.workspace.display()
                        ),
                    };
                }
            }
            SandboxMode::DangerFullAccess => {}
        }

        if let Some(dir) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                return FileOutcome::Failed { reason: format!("无法创建目录 {}：{e}", dir.display()) };
            }
        }
        match std::fs::write(path, content) {
            Ok(()) => FileOutcome::Written { bytes: content.len() },
            Err(e) => FileOutcome::Failed { reason: format!("写入 {} 失败：{e}", path.display()) },
        }
    }

    fn execute(&self, mode: SandboxMode, command: &str, limit_bytes: usize) -> SandboxOutcome {
        if !self.supports(mode) {
            return SandboxOutcome::Denied {
                reason: format!(
                    "本平台（{}）没有 {mode:?} 档的沙箱实现；拒绝在无沙箱保护下执行",
                    std::env::consts::OS
                ),
            };
        }

        let mut cmd = Command::new("sh");
        #[cfg(target_os = "macos")]
        {
            if let Some(profile) = seatbelt_profile(mode) {
                cmd = Command::new("sandbox-exec");
                cmd.arg("-p").arg(profile);
                // 可写根经参数注入（不拼字符串）
                if mode == SandboxMode::WorkspaceWrite {
                    // **必须传 canonicalize 后的路径**：Seatbelt 的 subpath 匹配
                    // 真实路径，而 macOS 上 /tmp 是指向 /private/tmp 的符号链接。
                    // 传符号链接路径会导致"工作区内写也被拒"（本实现踩过）。
                    let ws = std::fs::canonicalize(&self.workspace)
                        .unwrap_or_else(|_| self.workspace.clone());
                    cmd.arg("-D").arg(format!("WORKSPACE={}", ws.display()));
                }
                cmd.arg("sh").arg("-c").arg(command);
            } else {
                cmd.arg("-c").arg(command);
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            cmd.arg("-c").arg(command);
        }

        cmd.current_dir(&self.workspace)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return SandboxOutcome::Denied {
                    reason: format!("无法启动命令（沙箱不可用即拒绝）：{e}"),
                }
            }
        };

        let stdout_pipe = child.stdout.take();
        let stderr_pipe = child.stderr.take();

        // 并发读取：管道写满后子进程会阻塞，先 wait 再读必然死锁。
        let out_thread = std::thread::spawn(move || match stdout_pipe {
            Some(p) => read_capped(p, limit_bytes),
            None => (Vec::new(), false),
        });
        let err_thread = std::thread::spawn(move || match stderr_pipe {
            Some(p) => read_capped(p, limit_bytes),
            None => (Vec::new(), false),
        });

        // 超时兜底：读线程可能因为子进程既不退出也不产出而等待，
        // 这里用轮询 + kill 保证整体可终止。
        let deadline = Instant::now() + self.timeout;
        let mut timed_out = false;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        timed_out = true;
                        let _ = child.wait(); // 回收，避免僵尸
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }

        let (out_bytes, out_trunc) = out_thread.join().unwrap_or_default();
        let (err_bytes, err_trunc) = err_thread.join().unwrap_or_default();

        let (stdout, mut truncated) = lossy_tail(out_bytes, out_trunc);
        let (stderr, et) = lossy_tail(err_bytes, err_trunc);
        truncated |= et;

        if timed_out {
            return SandboxOutcome::Ran {
                stdout: format!("{stdout}\n[命令超时（{:?}）已被终止]", self.timeout),
                truncated,
            };
        }

        let merged = if stderr.is_empty() {
            stdout
        } else if stdout.is_empty() {
            stderr
        } else {
            format!("{stdout}\n{stderr}")
        };
        SandboxOutcome::Ran { stdout: merged, truncated }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_capped_returns_immediately_at_limit() {
        // 关键修复：达到上限立即返回，不 drain（否则 yes 会让读线程永不返回）
        let data = vec![b'x'; 100_000];
        let (kept, truncated) = read_capped(&data[..], 1000);
        assert_eq!(kept.len(), 1000);
        assert!(truncated);
    }

    #[test]
    fn read_capped_under_limit_not_truncated() {
        let (kept, truncated) = read_capped(&b"abc"[..], 1000);
        assert_eq!(kept, b"abc");
        assert!(!truncated);
    }

    #[test]
    fn lossy_tail_never_splits_a_codepoint() {
        let full = "好".repeat(10).into_bytes();
        let (s, truncated) = lossy_tail(full[..2].to_vec(), true);
        assert!(truncated);
        assert_eq!(s, "", "半个字符必须丢弃，而不是产出无效字符串");
    }

    #[test]
    fn is_within_handles_symlinked_tmp_and_new_files() {
        let root = std::env::temp_dir().join("neo-within-root");
        let _ = std::fs::create_dir_all(&root);

        // 根内已存在文件
        let inside = root.join("a.txt");
        std::fs::write(&inside, "x").unwrap();
        assert!(is_within(&root, &inside), "根内文件应被认作在内");

        // 根内**尚不存在**的文件（新建场景）
        let new_file = root.join("sub/b.txt");
        assert!(is_within(&root, &new_file), "父目录存在的新文件应被认作在内");

        // 根外
        assert!(!is_within(&root, Path::new("/etc/hosts")), "根外文件必须在外");

        // 符号链接逃逸：根内造一个指向 /etc 的链接
        #[cfg(unix)]
        {
            let link = root.join("escape");
            let _ = std::fs::remove_file(&link);
            if std::os::unix::fs::symlink("/etc", &link).is_ok() {
                assert!(
                    !is_within(&root, &link.join("hosts")),
                    "经符号链接逃逸到根外必须被识破"
                );
            }
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn write_file_respects_modes() {
        let root = std::env::temp_dir().join("neo-wf-root");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let sb = LocalSandbox::new(&root);

        // 只读档：拒
        assert!(matches!(
            sb.write_file(SandboxMode::ReadOnly, &root.join("x"), "y"),
            FileOutcome::Denied { .. }
        ));

        // 工作区内：允许
        let inside = root.join("ok.txt");
        assert!(matches!(
            sb.write_file(SandboxMode::WorkspaceWrite, &inside, "hello"),
            FileOutcome::Written { .. }
        ));
        assert_eq!(std::fs::read_to_string(&inside).unwrap(), "hello");

        // 工作区外：拒
        assert!(matches!(
            sb.write_file(SandboxMode::WorkspaceWrite, Path::new("/tmp/neo-escape-write.txt"), "x"),
            FileOutcome::Denied { .. }
        ));
        assert!(!Path::new("/tmp/neo-escape-write.txt").exists(), "被拒的写不得落盘");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn restricted_modes_fail_closed_off_macos() {
        let sb = LocalSandbox::new("/tmp");
        if cfg!(target_os = "macos") {
            assert!(sb.supports(SandboxMode::ReadOnly));
        } else {
            assert!(!sb.supports(SandboxMode::ReadOnly));
            assert!(matches!(
                sb.execute(SandboxMode::ReadOnly, "echo hi", 100),
                SandboxOutcome::Denied { .. }
            ));
        }
    }
}
