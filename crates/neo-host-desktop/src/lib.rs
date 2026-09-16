//! L5 HOST · 桌面（系统 webview，替代 Electron）
//!
//! 铁律：不含业务逻辑。**不含 Electron、不捆绑 Chromium/Node。**
//!
//! 技术选择：`wry`（系统 webview）而非纯 Rust GUI。
//! ⚠️ 当年选它的理由是"保住现有 React + 液态玻璃界面零改动"（见
//! `03-模块规格/L5-host.md` 第一节）——**该前提已不成立**：那个 React 界面在
//! `legacy/`，本 crate 实际加载的是 `neo-host-web` 那份 184 行内联页面
//! （无 React、无液态玻璃），`crates/` 里对 `legacy/` 的引用数为 0。
//! 因此本项目已决定**新增 Rust 原生 GUI 宿主**，webview 保留为第二后端，
//! 理由与代价见 `ADR-0006-桌面原生GUI.md` 与 `docs/desktop-plan.md`。
//!
//! 平台矩阵（诚实边界，不得声称三平台视觉一致）：
//! - macOS   : WKWebView，系统自带
//! - Windows : WebView2，Win11 自带；Win10 需装运行时
//! - Linux   : WebKitGTK，需系统包
//!
//! 窗口层已接入（`window` 模块）：wry + tao，窗口只是**壳** ——
//! 系统 webview 指向 Web 宿主服务的页面（本地回环端口），完整复用
//! Web 宿主的界面与事件流，因此没有第三套消费逻辑要证明等价。
//! 关于依赖：wry/tao 内部含平台 unsafe（FFI 绑定），本项目各 crate
//! 自身代码保持零 unsafe —— 引入的是依赖而非本项目的不安全代码，
//! 这条边界由门禁的 unsafe 扫描持续守护。

pub mod window;

use neo_core::{DiffSupport, HostBackend, HostCapabilities, ImageSupport};
use neo_protocol::{EventMsg, Fact};

/// 输入解析：与 TUI / Web 共用 L0 协议层的同一份实现（自造副本会漂移）。
pub use neo_protocol::parse_refs;

/// **测试替身，不是运行路径。**
///
/// 真实桌面窗口跑的是 `neo-host-web`（见 `window` 模块与 `neo-code-cli` 的
/// `cmd_desktop`），它才是那个消费事件流的宿主。本类型存在的唯一理由是让
/// T6 契约测试里有一个"desktop"身份的后端参与比对
/// （`neo-mock/tests/conformance.rs`）—— 换成真实实现会让那个测试依赖
/// 图形栈，而它要验的是**契约**、不是渲染。
///
/// 因此它只做两件事：按顺序累积事件、交给协议层的 `facts_of` 抽事实。
/// ⚠️ 别把它当成"桌面渲染器"去接线 —— 它没有窗口、没有 webview，
/// 也不会把任何东西画出来。新增 Rust 原生 GUI 宿主时，要么让它接上真实
/// 渲染（届时本注释与 `id()` 都需重新审视），要么保持它测试替身的定位。
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
        // 只累积 —— 详见类型注释。这里**没有**渲染，也不应有人把它当渲染器接线。
        self.events.push(event.clone());
        Ok(())
    }

    fn facts(&self) -> Vec<Fact> { neo_protocol::facts_of(&self.events) }
}
