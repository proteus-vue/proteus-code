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

/// 桌面宿主的输入解析：复用 ZCode 的 @ / # / / / $ 引用体系。
/// 与 TUI / Web 共享同一解析语义 —— 解析属**语义**，不属宿主实现。
pub fn parse_refs(input: &str) -> Vec<(char, String)> {
    let mut out = Vec::new();
    for token in input.split_whitespace() {
        let Some(first) = token.chars().next() else { continue };
        if matches!(first, '@' | '#' | '/' | '$') && token.len() > 1 {
            out.push((first, token[1..].to_string()));
        }
    }
    out
}

pub struct DesktopHost { facts: Vec<String> }

impl DesktopHost {
    pub fn new() -> Self { Self { facts: Vec::new() } }
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

    fn consume(&mut self, event_json: &str) -> Result<(), String> {
        // 真实实现：解析 EventMsg 并驱动 webview 渲染。
        // 原型只登记"用户可见事实"，供 T6 等价性断言使用。
        self.facts.push(format!("[desktop] {event_json}"));
        Ok(())
    }

    fn rendered_facts(&self) -> Vec<String> { self.facts.clone() }
}
