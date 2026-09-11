//! L5 HOST · 桌面（系统 webview，替代 Electron）
//!
//! 铁律：不含业务逻辑。**不含 Electron、不捆绑 Chromium/Node。**
//!
//! 技术选择：`wry`（系统 webview）而非纯 Rust GUI，理由见
//! `03-模块规格/L5-host.md` 第一节 —— 保住现有 React + 液态玻璃界面零改动。
//!
//! 平台矩阵（诚实边界，不得声称三平台视觉一致）：
//! - macOS   : WKWebView，系统自带
//! - Windows : WebView2，Win11 自带；Win10 需装运行时
//! - Linux   : WebKitGTK，需系统包
//!
//! 注：本原型不引入 wry 依赖（避免原型阶段拉入平台图形栈），
//! 只固定契据与分层；真实实现时在 window 层接入。

use dsh_core::{DiffSupport, HostBackend, HostCapabilities, ImageSupport};
use dsh_protocol::{EventMsg, Fact};

/// 输入解析：与 TUI / Web 共用 L0 协议层的同一份实现（自造副本会漂移）。
pub use dsh_protocol::parse_refs;

pub struct DesktopHost { events: Vec<EventMsg> }

impl DesktopHost {
    pub fn new() -> Self { Self { events: Vec::new() } }
}

impl Default for DesktopHost {
    fn default() -> Self { Self::new() }
}

impl HostBackend for DesktopHost {
    fn id(&self) -> &'static str { "desktop" }

    fn capabilities(&self) -> HostCapabilities {
        HostCapabilities {
            images: ImageSupport::Inline,
            rich_text: true,
            interactive_prompt: true,
            diffs: DiffSupport::Hunk,
        }
    }

    fn consume(&mut self, event: &EventMsg) -> Result<(), String> {
        // 真实实现：把事件推给 webview 渲染。
        // 当前只累积，供 T6 等价性断言使用。
        self.events.push(event.clone());
        Ok(())
    }

    fn facts(&self) -> Vec<Fact> { dsh_protocol::facts_of(&self.events) }
}
