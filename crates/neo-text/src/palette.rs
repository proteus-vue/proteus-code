//! 语义色调 → RGB 的调色板（宿主中立）
//!
//! # 为什么这个在共享层，而"主题"不在
//!
//! [`Theme`](crate) 的边界是"**换一个宿主，这行代码还成立吗**"。拆开看：
//!
//! - `Tone → RGB`（本模块）：终端把它转成 ANSI SGR，GUI 把它转成 `Color32`，
//!   但"强调色是 violet 400"这件事两边**必须**一致 —— 否则同一个回复在 TUI 与
//!   桌面里长得不一样，而"宿主等价"正是本项目的铁律。→ **宿主中立，放这里**。
//! - 背景纹理、星场、方角/圆角、亮色主题的整屏铺底：这些是**某个宿主的呈现手法**，
//!   各宿主的主题里各自持有。→ **不放这里**。
//!
//! # 与 `neo-host-tui::theme` 的关系（防漂移）
//!
//! TUI 的 `theme.rs` 有 10 套主题，其中 NEO 那套的品牌色与本模块的 [`NEO`]
//! 是**同一组值**。两份并存的原因很实际：A3 禁止宿主之间互相依赖，而 `Theme`
//! 还带着星场/方角等终端专有字段，整体搬进共享层会把这些也带进来。
//!
//! 但"同一组值写在两处"就是漂移的温床，所以不靠约定而靠**机器检查**：
//! `neo-host-tui` 里有一条守卫测试，逐字段比对它的 NEO 主题与本模块的 [`NEO`]，
//! 不一致就编译测试失败。改任何一边而忘了另一边，CI 会当场拦住。
//!
//! （长期更干净的做法是让 `Theme` 内嵌一个 [`Palette`]，但那要动 TUI 的
//! 300+ 测试与大量字段访问点；在 GUI 宿主刚起步、主题尚未落地时先不做，
//! 如实记在 `docs/desktop-plan.md`。）

use crate::Tone;

/// 一套语义调色板（RGB）。
///
/// 字段与 [`Tone`] 的语义色一一对应，另有几个面板底色（界面分层用：
/// 页面 → 面板 → 元素，靠亮度阶梯区分"块与块"）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub primary: (u8, u8, u8),
    pub accent: (u8, u8, u8),
    pub success: (u8, u8, u8),
    pub error: (u8, u8, u8),
    pub warning: (u8, u8, u8),
    pub info: (u8, u8, u8),
    pub text: (u8, u8, u8),
    pub muted: (u8, u8, u8),
    pub border: (u8, u8, u8),
    pub border_active: (u8, u8, u8),
    /// 最弱的结构文字。
    ///
    /// ⚠️ 与终端**不完全等价**：TUI 用 ANSI 的 faint 属性（`SGR 2`）表达它，
    /// 那是个"把当前前景变暗"的开关、本身没有颜色；GUI 没有对应机制，只能用
    /// 一个固定 RGB 近似。所以这个值只保证"看起来足够弱"，不保证与 TUI
    /// 逐像素一致 —— 这条不对称是宿主能力差异，不是实现偷懒。
    pub dim: (u8, u8, u8),
    /// 面板底（侧栏、设置页）。比页面底略亮，才看得出"这是一块面"。
    pub bg_panel: (u8, u8, u8),
    /// 选中行底（低饱和强调色，不抢前景可读性）。
    pub bg_selected: (u8, u8, u8),
    /// 元素底：比面板再亮一档，用于二级容器（选项条、输入区）。
    pub bg_element: (u8, u8, u8),
    /// 页面底（最底层的整屏底色）。
    pub bg_base: (u8, u8, u8),
}

impl Palette {
    /// 语义色调 → RGB。
    ///
    /// `Tone::None` 退回正文色：它是"未指定"而非一种颜色，调用方若要表达
    /// "什么都不画"应自己判断（`Tone::is_unset()`），不要依赖这里的兜底。
    pub fn rgb(&self, tone: Tone) -> (u8, u8, u8) {
        match tone {
            Tone::None | Tone::Text => self.text,
            Tone::Primary => self.primary,
            Tone::Accent => self.accent,
            Tone::Success => self.success,
            Tone::Error => self.error,
            Tone::Warning => self.warning,
            Tone::Info => self.info,
            Tone::Muted => self.muted,
            Tone::Border => self.border,
            Tone::BorderActive => self.border_active,
            Tone::Dim => self.dim,
            Tone::Rgb(r, g, b) => (r, g, b),
        }
    }
}

/// NEO 品牌配色：紫为主色，紫→品红渐变。
///
/// 值必须与 `neo-host-tui::theme` 的 `ThemeName::Neo` 一致 ——
/// 由 TUI 侧的守卫测试强制（见模块注释）。
pub const NEO: Palette = Palette {
    primary: (0xa7, 0x8b, 0xfa),   // violet 400，主品牌紫
    accent: (0xe8, 0x79, 0xf9),    // fuchsia 400，渐变终点 / 标题
    success: (0x6e, 0xe7, 0xb7),
    error: (0xf8, 0x71, 0x71),
    warning: (0xfb, 0xbf, 0x24),
    info: (0x7d, 0xd3, 0xfc),
    text: (0xed, 0xed, 0xed),
    muted: (0x8a, 0x8a, 0x94),     // 略带紫调的灰，与主色同族
    border: (0x45, 0x45, 0x52),
    border_active: (0x5c, 0x5c, 0x6e),
    // 终端用 SGR faint 表达 Dim；这里给一个"够弱"的固定灰
    dim: (0x5a, 0x5a, 0x66),
    bg_panel: (0x20, 0x1e, 0x2a),
    bg_selected: (0x33, 0x2f, 0x45),
    bg_element: (0x2a, 0x27, 0x36),
    bg_base: (0x00, 0x00, 0x00),
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_semantic_tone_resolves_to_a_color() {
        // 关键不变量：**没有色调会落空**。漏一个就是"某个界面元素在不同宿主里
        // 颜色不同"或直接画不出来，而这类问题只在真机才看得见。
        let p = NEO;
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
        ] {
            let (r, g, b) = p.rgb(tone);
            // 纯黑在深色底上等于"看不见"，不该被任何语义色解析出来
            assert!(
                (r, g, b) != (0, 0, 0),
                "{tone:?} 解析成了纯黑，在深色底上不可见"
            );
        }
    }

    #[test]
    fn rgb_escape_hatch_passes_the_value_through() {
        // 装饰色（星场、渐变）由调用方给具体 RGB，必须原样透传 ——
        // 否则 GUI 与 TUI 的装饰色会各自被调色板改写
        assert_eq!(NEO.rgb(Tone::Rgb(1, 2, 3)), (1, 2, 3));
    }

    #[test]
    fn semantic_colors_are_distinct_so_hierarchy_is_visible() {
        // 层次靠颜色区分；两个语义色相同就等于其中一层白做了
        let p = NEO;
        assert_ne!(p.primary, p.accent, "主色与强调色必须可区分");
        assert_ne!(p.text, p.muted, "正文与弱化文字必须可区分");
        assert_ne!(p.border, p.border_active, "边框两档必须可区分");
        assert_ne!(p.bg_panel, p.bg_element, "面板与元素底必须可区分");
    }
}
