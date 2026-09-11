//! L0 PLATFORM —— 进程加固、文件监听、git worktree、系统剪贴板
//!
//! # 为什么剪贴板做成 SPI 而不是直接调 `pbcopy`
//!
//! 直接 `Command::new("pbcopy")` 会把 macOS 写死在调用点，于是：
//!   - Linux/Windows 上静默失效（用户以为复制成功了）；
//!   - 测试无法注入假实现（只能真去改系统剪贴板，测完还得还原）。
//!
//! 所以定义 `Clipboard` 契据 + 两个后端：
//!   - `SystemClipboard`：按平台选命令（macOS `pbcopy` / Linux `xclip|wl-copy`
//!     / Windows `clip`），**失败时明确返回 Err 而不是静默成功**；
//!   - `NoopClipboard`：无剪贴板环境（headless/CI）用，`copy` 返回明确原因。
//!
//! conformance 测试同一份用例跑两个后端（见 tests/clipboard.rs）。

use std::io::Write;
use std::process::{Command, Stdio};

/// 剪贴板契据。语义：把一段文本交给**系统剪贴板**。
pub trait Clipboard: Send + Sync {
    /// 后端名（用于日志与错误信息）。
    fn name(&self) -> &'static str;

    /// 把文本写入剪贴板。
    ///
    /// 返回 `Err` 表示**真的没写进去** —— 调用方据此告知用户，
    /// 而不是显示"已复制"然后让用户粘贴出旧内容。
    fn copy(&self, text: &str) -> Result<(), String>;

    /// 本后端在此平台是否可用（用于启动时提示，避免运行时才发现）。
    fn available(&self) -> bool;
}

/// 系统剪贴板后端：按平台选命令。
pub struct SystemClipboard;

impl SystemClipboard {
    pub fn new() -> Self { Self }
}

impl Default for SystemClipboard {
    fn default() -> Self { Self::new() }
}

/// 当前平台的剪贴板命令（可执行文件, 参数）。
///
/// 公开出来是为了**可测**：测试可以断言"在 macOS 上选的是 pbcopy"，
/// 而不必真的调用它。
pub fn clipboard_command() -> Option<(&'static str, &'static [&'static str])> {
    if cfg!(target_os = "macos") {
        Some(("pbcopy", &[]))
    } else if cfg!(target_os = "windows") {
        Some(("clip", &[]))
    } else {
        // Linux/BSD：Wayland 优先（wl-copy），回退 X11（xclip）
        // 具体选哪个在 `copy` 里按"命令是否存在"决定
        None
    }
}

/// Linux 候选命令（按优先级）。
fn linux_candidates() -> &'static [(&'static str, &'static [&'static str])] {
    &[
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
    ]
}

/// 命令是否在 PATH 上（用 `command -v`，不 fork 一个 shell 去 which）。
fn command_exists(cmd: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {cmd} >/dev/null 2>&1"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

impl Clipboard for SystemClipboard {
    fn name(&self) -> &'static str { "system" }

    fn available(&self) -> bool {
        if let Some((cmd, _)) = clipboard_command() {
            return command_exists(cmd);
        }
        linux_candidates().iter().any(|(c, _)| command_exists(c))
    }

    fn copy(&self, text: &str) -> Result<(), String> {
        // 选出要用的命令
        let (cmd, args) = match clipboard_command() {
            Some(v) => v,
            None => linux_candidates()
                .iter()
                .find(|(c, _)| command_exists(c))
                .copied()
                .ok_or_else(|| {
                    "未找到剪贴板命令（尝试过 wl-copy / xclip / xsel）".to_string()
                })?,
        };

        let mut child = Command::new(cmd)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("无法启动剪贴板命令 {cmd}：{e}"))?;

        child
            .stdin
            .as_mut()
            .ok_or("剪贴板命令的 stdin 不可用")?
            .write_all(text.as_bytes())
            .map_err(|e| format!("写入剪贴板失败：{e}"))?;
        // 关闭 stdin 让命令知道输入结束（pbcopy 等会一直等，不关就挂住）
        drop(child.stdin.take());

        let out = child
            .wait_with_output()
            .map_err(|e| format!("等待剪贴板命令失败：{e}"))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(format!("剪贴板命令 {cmd} 退出码非 0：{}", err.trim()));
        }
        Ok(())
    }
}

/// 无剪贴板后端（headless / CI / 显式禁用）。
///
/// `copy` 返回**明确原因**而不是假装成功 —— 假装成功会让用户在别处
/// 粘贴出旧内容，且完全不知道哪一步出了问题。
pub struct NoopClipboard {
    pub reason: String,
}

impl NoopClipboard {
    pub fn new(reason: impl Into<String>) -> Self {
        Self { reason: reason.into() }
    }
}

impl Clipboard for NoopClipboard {
    fn name(&self) -> &'static str { "noop" }

    fn available(&self) -> bool { false }

    fn copy(&self, _text: &str) -> Result<(), String> {
        Err(format!("剪贴板不可用：{}", self.reason))
    }
}

// ── 其余平台能力（尚未实现，保留契据）──────────────────────────────

pub fn harden_process() { /* pre-main 反调试 / 反转储 / 环境变量清理 */ }
pub fn watch(_root: &str) { /* inotify / FSEvents / ReadDirectoryChangesW */ }
pub fn worktree_path(root: &str, name: &str) -> String { format!("{root}/.neo/worktrees/{name}") }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_command_matches_the_os() {
        // 断言的是"选择逻辑"而不是"真的调用"：这样不必改系统剪贴板就能测
        if cfg!(target_os = "macos") {
            assert_eq!(clipboard_command().map(|(c, _)| c), Some("pbcopy"));
        } else if cfg!(target_os = "windows") {
            assert_eq!(clipboard_command().map(|(c, _)| c), Some("clip"));
        } else {
            assert!(clipboard_command().is_none(), "Linux 走候选列表，不是单一命令");
        }
    }

    #[test]
    fn noop_backend_reports_a_reason_instead_of_pretending() {
        // 假装成功比失败更糟：用户会在别处粘出旧内容且不知道哪步错了
        let c = NoopClipboard::new("测试环境无剪贴板");
        assert!(!c.available());
        let err = c.copy("x").unwrap_err();
        assert!(err.contains("测试环境无剪贴板"), "应带上原因：{err}");
    }

    #[test]
    fn system_backend_availability_is_a_bool_not_a_panic() {
        // 在无剪贴板的 CI 里 available() 应为 false，但**不得 panic**
        let c = SystemClipboard::new();
        let _ = c.available();
        assert_eq!(c.name(), "system");
    }

    #[test]
    fn linux_candidates_are_ordered_wayland_first() {
        // Wayland 优先：现代 Linux 桌面多为 Wayland，xclip 在纯 Wayland 下不可用
        let c = linux_candidates();
        assert_eq!(c[0].0, "wl-copy");
        assert!(c.iter().any(|(n, _)| *n == "xclip"));
    }
}
