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

pub mod file_index;
pub mod file_watch;
pub mod secrets;
pub mod wiki;

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


// ══════════════════════════════════════════════════════════════════════
// 提醒（提示音 / 桌面通知）—— 第 7 个 SPI
// ══════════════════════════════════════════════════════════════════════
//
// # 为什么做成 SPI
//
// 与剪贴板同理：直接 `Command::new("osascript")` 会把 macOS 写死在调用点，
// 于是 Linux/Windows 静默失效，且测试无法注入假实现。
// 更重要的是**提醒是最不该失败却很爱失败的能力**：
// 它常在无 GUI、无音频设备、SSH、容器里被调用 —— 那时必须"安静地降级"
// 而不是崩或卡住。把它做成契据，降级行为就有明确位置可写、可测。
//
// # 语义边界：提醒失败不该影响主流程
//
// `Notify::notify` 返回 `Result`，但调用方应当**只用它来告知用户**，
// 绝不能因为提醒失败而中断任务。这一点在文档里写明，免得后人把
// `?` 加到调用链上。

/// 提醒的场景（决定声音/紧急程度；后端可据此选择不同表现）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attention {
    /// 一轮任务完成
    TurnComplete,
    /// 出错
    Error,
    /// 需要用户审批（最该打扰用户的场景）
    ApprovalNeeded,
}

impl Attention {
    /// 给桌面通知用的标题。
    pub fn title(self) -> &'static str {
        match self {
            Self::TurnComplete => "Neo · 完成",
            Self::Error => "Neo · 出错",
            Self::ApprovalNeeded => "Neo · 需要审批",
        }
    }
}

/// 提醒契据：让用户**注意到**某件事发生了。
pub trait Notify: Send + Sync {
    /// 后端名（日志与错误信息用）。
    fn name(&self) -> &'static str;

    /// 后端在此平台是否可用。
    fn available(&self) -> bool;

    /// 发一次提醒。`detail` 是给用户看的一句话说明。
    ///
    /// 返回 `Err` 表示没发出去；调用方**不得**因此中断主流程。
    fn notify(&self, kind: Attention, detail: &str) -> Result<(), String>;
}

/// 系统提醒后端：macOS 用 `osascript` 弹通知 + `afplay` 响铃；
/// Linux 用 `notify-send`；Windows 暂不支持（返回明确原因）。
pub struct SystemNotify {
    /// 是否同时发声。通知（视觉）与声音（听觉）分开控制 ——
    /// 用户在专注时可能只想要声音、或只要视觉。
    pub sound: bool,
}

impl SystemNotify {
    pub fn new(sound: bool) -> Self { Self { sound } }

    /// macOS 的系统提示音文件（`/System/Library/Sounds/`）。
    fn macos_sound(kind: Attention) -> &'static str {
        match kind {
            // 不同场景用不同音色：出错用低沉、完成用清脆、审批用温和
            Attention::TurnComplete => "Glass.aiff",
            Attention::Error => "Basso.aiff",
            Attention::ApprovalNeeded => "Ping.aiff",
        }
    }
}

impl Default for SystemNotify {
    fn default() -> Self { Self::new(true) }
}

impl Notify for SystemNotify {
    fn name(&self) -> &'static str { "system" }

    fn available(&self) -> bool {
        if cfg!(target_os = "macos") {
            command_exists("osascript")
        } else if cfg!(target_os = "linux") {
            command_exists("notify-send")
        } else {
            false
        }
    }

    fn notify(&self, kind: Attention, detail: &str) -> Result<(), String> {
        if !self.available() {
            // 不可用 = 安静地不做。契约（notify.rs conformance）：提醒不可用
            // 只是"少个便利"，报错会打断主流程 —— Linux CI/容器/SSH 里没有
            // notify-send 是常态，返回 Err 曾让 conformance 的"不可用不报错"
            // 分支在 CI 上失败（macOS 上 osascript 恒存在，本地测不出）。
            return Ok(());
        }
        if cfg!(target_os = "macos") {
            // 通知文案里的引号必须转义，否则 AppleScript 语法错
            let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
            let script = format!(
                r#"display notification "{detail}" with title "{title}""#,
                detail = esc(detail),
                title = esc(kind.title()),
            );
            let st = Command::new("osascript")
                .args(["-e", &script])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .output()
                .map_err(|e| format!("无法启动 osascript：{e}"))?;
            if !st.status.success() {
                return Err(format!(
                    "osascript 失败：{}",
                    String::from_utf8_lossy(&st.stderr).trim()
                ));
            }
            if self.sound {
                // 声音失败**不算整体失败**：没声音但弹出通知，仍是有用的提醒。
                let _ = Command::new("afplay")
                    .arg(format!("/System/Library/Sounds/{}", Self::macos_sound(kind)))
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn();
            }
            Ok(())
        } else {
            let st = Command::new("notify-send")
                .args([kind.title(), detail])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .output()
                .map_err(|e| format!("无法启动 notify-send：{e}"))?;
            if !st.status.success() {
                return Err(format!(
                    "notify-send 失败：{}",
                    String::from_utf8_lossy(&st.stderr).trim()
                ));
            }
            if self.sound && command_exists("paplay") {
                let _ = Command::new("paplay")
                    .arg("/usr/share/sounds/freedesktop/stereo/complete.oga")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn();
            }
            Ok(())
        }
    }
}

/// 无提醒后端：CI / headless / 用户显式关闭。
///
/// 它**不是"失败"**而是"按配置不提醒" —— 因此 `available()` 返回 false，
/// 但 `notify` 返回 Ok（没有出错，只是没做）。这是与 `NoopClipboard`
/// 有意的区别：剪贴板不可用意味着用户的操作没生效（必须报错），
/// 而提醒不可用只意味着"少了个便利"（不该报错打扰用户）。
pub struct NoopNotify {
    pub reason: String,
}

impl NoopNotify {
    pub fn new(reason: impl Into<String>) -> Self {
        Self { reason: reason.into() }
    }
}

impl Notify for NoopNotify {
    fn name(&self) -> &'static str { "noop" }
    fn available(&self) -> bool { false }
    fn notify(&self, _kind: Attention, _detail: &str) -> Result<(), String> {
        let _ = &self.reason;
        Ok(()) // 按配置不提醒，不是错误
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

    // ── 提醒 ──────────────────────────────────────────────────────────

    #[test]
    fn attention_titles_are_distinct_and_nonempty() {
        // 三种场景的标题必须能区分 —— 用户一眼要知道是"完成"还是"要审批"
        let ts: Vec<&str> = [
            Attention::TurnComplete,
            Attention::Error,
            Attention::ApprovalNeeded,
        ]
        .iter()
        .map(|a| a.title())
        .collect();
        for t in &ts {
            assert!(!t.is_empty());
        }
        let mut uniq = ts.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(uniq.len(), ts.len(), "标题必须互不相同：{ts:?}");
    }

    #[test]
    fn noop_notify_is_silent_but_not_an_error() {
        // 与 NoopClipboard 的关键区别：提醒不可用**不是错误** ——
        // 剪贴板失败意味着用户操作没生效（必须报错），
        // 提醒不可用只是少了个便利（不该报错打扰）。
        let n = NoopNotify::new("CI 环境");
        assert!(!n.available());
        assert!(n.notify(Attention::TurnComplete, "x").is_ok(), "不该报错");
        assert_eq!(n.name(), "noop");
    }

    #[test]
    fn system_notify_degrades_silently_when_unavailable() {
        // 契约（与 conformance 一致）：不可用 = 安静地不做，**不是错误**。
        // 这里只测"自称可用却失败"的分支；不可用分支在 Linux CI 上
        // （无 notify-send）由 conformance 的"不可用不报错"分支覆盖。
        let n = SystemNotify::new(true);
        if n.available() {
            let r = n.notify(Attention::TurnComplete, "neo 提醒自检");
            assert!(r.is_ok(), "自称可用却失败：{:?}", r.err());
        }
    }

    #[test]
    fn notification_text_with_quotes_does_not_break_the_backend() {
        // AppleScript 里未转义的引号会导致语法错 —— 这是最容易踩的坑。
        // 该用例断言"含引号的文案要么成功、要么报出真实错误（而非语法错）"。
        let n = SystemNotify::new(false); // 关声音，避免测试时响
        if n.available() {
            let r = n.notify(Attention::Error, r#"包含 "引号" 与 \ 反斜杠"#);
            if let Err(e) = &r {
                assert!(
                    !e.contains("syntax error"),
                    "文案里的引号/反斜杠必须被转义，实际：{e}"
                );
            }
        }
    }

    #[test]
    fn macos_sound_choices_differ_by_scenario() {
        // 同一个音色配所有场景会让人分不清发生了什么
        let a = SystemNotify::macos_sound(Attention::TurnComplete);
        let b = SystemNotify::macos_sound(Attention::Error);
        let c = SystemNotify::macos_sound(Attention::ApprovalNeeded);
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_ne!(a, c);
    }

    #[test]
    fn linux_candidates_are_ordered_wayland_first() {
        // Wayland 优先：现代 Linux 桌面多为 Wayland，xclip 在纯 Wayland 下不可用
        let c = linux_candidates();
        assert_eq!(c[0].0, "wl-copy");
        assert!(c.iter().any(|(n, _)| *n == "xclip"));
    }
}
