//! L5 HOST · 桌面原生 GUI（egui/eframe）
//!
//! # 与 `neo-host-desktop` 的分工
//!
//! | | `neo-host-desktop` | 本 crate |
//! |---|---|---|
//! | 渲染 | 系统 webview（wry）加载 `neo-host-web` 的页面 | egui 原生绘制 |
//! | 依赖 | wry/tao（平台 GUI 栈） | eframe/egui（纯 Rust，数十个 crate） |
//! | 选它的理由 | 复用既有 Web 界面（T6 等价天然成立） | 脱离 DOM/CSS，直接画 TUI 已有的信息层次 |
//!
//! 两者**保留并存**（ADR-0006：不押注单一方案）：`neo desktop` 走原生，
//! `neo desktop --webview` 保留 webview 路径。
//!
//! # 铁律：不含业务逻辑
//!
//! 它只做三件事：把 Op 交给内核、消费事件流、把事件画出来。
//! 事件到"用户可见事实"的映射仍在协议层（`facts_of`），与其它宿主一致 ——
//! 这是 T6 宿主等价断言成立的前提。
//!
//! # 复用 `neo-text` 而不是自己一套
//!
//! 语义色调（`Tone`）、语义调色板（`neo_text::palette`）、Markdown 解析都来自
//! `neo-text` —— 与 TUI **同一份**。这不是为了省代码，是为了**观感一致**：
//! 若 GUI 自己解析一遍 Markdown，同一个回复在终端与窗口里就会长得不一样，
//! 而"同一事件流下语义等价"是项目的硬要求。

pub mod driver;
pub mod facts;
pub mod fonts;
pub mod theme;
pub mod ui;

pub use driver::KernelHandle;

pub use neo_text::palette;
pub use neo_text::Tone;

/// egui 的 `Color32` 转换：语义色调 → 实际颜色。
///
/// 单独一层而不是散在各处 `Color32::from_rgb(p.rgb(tone))`：这样换调色板、
/// 加主题、做"高对比度模式"都只改这里一处。
pub struct GuiPalette {
    pub palette: palette::Palette,
}

impl GuiPalette {
    pub fn neo() -> Self {
        Self { palette: palette::NEO }
    }

    /// 语义色调 → egui 颜色。
    pub fn color(&self, tone: Tone) -> egui::Color32 {
        let (r, g, b) = self.palette.rgb(tone);
        egui::Color32::from_rgb(r, g, b)
    }

    /// 面板/元素底色（界面分层用）。
    pub fn panel_bg(&self) -> egui::Color32 {
        let (r, g, b) = self.palette.bg_panel;
        egui::Color32::from_rgb(r, g, b)
    }

    pub fn element_bg(&self) -> egui::Color32 {
        let (r, g, b) = self.palette.bg_element;
        egui::Color32::from_rgb(r, g, b)
    }

    pub fn selected_bg(&self) -> egui::Color32 {
        let (r, g, b) = self.palette.bg_selected;
        egui::Color32::from_rgb(r, g, b)
    }

    pub fn base_bg(&self) -> egui::Color32 {
        let (r, g, b) = self.palette.bg_base;
        egui::Color32::from_rgb(r, g, b)
    }
}

impl Default for GuiPalette {
    fn default() -> Self {
        Self::neo()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tone_maps_to_a_gui_color() {
        // 与 palette 侧的测试同源：漏一个语义色就是"某处画不出来"
        let p = GuiPalette::neo();
        for tone in [
            Tone::None,
            Tone::Text,
            Tone::Primary,
            Tone::Accent,
            Tone::Success,
            Tone::Error,
            Tone::Warning,
            Tone::Info,
            Tone::Muted,
            Tone::Border,
            Tone::BorderActive,
            Tone::Dim,
            Tone::Rgb(9, 8, 7),
        ] {
            let c = p.color(tone);
            assert!(
                c.a() == 255,
                "{tone:?} 的 alpha 不是不透明 —— 面板上会半透明叠色"
            );
        }
        assert_eq!(p.color(Tone::Rgb(9, 8, 7)), egui::Color32::from_rgb(9, 8, 7));
    }

    #[test]
    fn layer_backgrounds_are_distinguishable() {
        // 界面层次靠底色亮度阶梯区分；两档相同就等于少了一层
        let p = GuiPalette::neo();
        assert_ne!(p.panel_bg(), p.element_bg(), "面板底与元素底必须可区分");
        assert_ne!(p.panel_bg(), p.base_bg(), "面板底与页面底必须可区分");
    }
}
