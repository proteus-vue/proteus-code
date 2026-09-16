//! L1 BASE · 宿主中立的文本语义
//!
//! # 这个 crate 解决什么问题
//!
//! 宿主（终端 / 桌面窗口 / 浏览器）都需要同一套东西：语义色调、按显示宽度
//! 排版、把模型回复里的 Markdown 渲染成带层次的文字。这些语义**与宿主无关**
//! —— 终端把它翻译成 ANSI SGR，GUI 把它翻译成 `Color32`，但"这是强调色"
//! 这件事两边必须一致，否则同一个回复在两个宿主里长得不一样（T6 宿主等价）。
//!
//! 架构守卫 A3 禁止宿主之间互相依赖，所以共享层必须**沉到宿主之下** ——
//! 这就是本 crate 存在的位置。
//!
//! # 边界：什么该放进来
//!
//! 判据是"**换一个宿主，这行代码还成立吗**"：
//!
//! - ✅ 放进来：语义色调（`Tone`）、Unicode 显示宽度、Markdown 解析、词法高亮
//! - ❌ 不放：ANSI 转义序列、网格/窗口绘制、主题的具体 RGB、星场、鼠标事件
//!
//! 主题（`neo-host-tui::theme`）**不在这里**：它是"语义色 → 终端 SGR"的翻译表，
//! 每个宿主有自己的那一份。本 crate 只定义语义，不定义外观。

pub mod markdown;
pub mod palette;
pub mod syntax;
pub mod width;

pub use palette::Palette;

/// 语义色调。**只表达"这段文字在信息层次里是什么角色"**，不表达"用什么颜色"。
///
/// # 为什么是语义而不是颜色
///
/// 网格只存色调，具体转义由各宿主的调色板在输出时展开 —— 这样换主题、
/// 换宿主（终端 SGR / GUI Color32）、做能力降级都只需改一处，
/// 不必在每个渲染点判断"这里该用什么颜色"。
///
/// # 迁移记录：`StarDim` / `StarBright` 为什么被删掉
///
/// 它们原本在这里，语义是"星场里的暗星/亮星"—— 那是**终端背景装饰**，
/// 不是文本语义：GUI 宿主没有星场，也不会去查 `theme.star_dim`。
/// 一个"宿主中立"的色调枚举里带着某个宿主的装饰，就是把宿主概念漏进了
/// 共享层（正是本 crate 要消除的东西）。
///
/// 现在改由调用方用 [`Tone::Rgb`] 传具体颜色：TUI 在铺星场时从自己的主题
/// 取 `star_dim` / `star_bright`。**装饰不得进入语义** —— 这条在
/// PROJECT_MEMORY §4.32 已经写过一次，这里是它在跨 crate 边界上的又一次应用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// 未指定（继承宿主默认前景 / 透空）。
    ///
    /// 终端侧的用法：网格以 `None` 初始化，表示"这格还没写过"，
    /// 于是背景层（星场）只填这些格子、不覆盖已有内容。
    None,
    /// 正文
    Text,
    /// 需要突出的正文（加粗、列表符号）
    Primary,
    /// 强调（标题、关键字）
    Accent,
    /// 成功（字符串字面量、通过）
    Success,
    /// 失败（错误、删除行）
    Error,
    /// 警告（待审批、需注意）
    Warning,
    /// 提示信息（次级标题）
    Info,
    /// 弱化的辅助文字（注释、元信息）
    Muted,
    /// 结构边框（非焦点）
    Border,
    /// 结构边框（焦点）
    BorderActive,
    /// 最弱的结构文字
    Dim,
    /// 任意 RGB —— **宿主中立的逃逸口**：装饰性配色（星场、渐变）由调用方
    /// 从自己的主题取值传进来，而不是在语义枚举里开一个宿主专有的变体。
    Rgb(u8, u8, u8),
}

impl Tone {
    /// 是否是"未写入"（终端侧用于判断该格能否被背景层填充）。
    ///
    /// 单独开一个方法而不是让调用方直接比较：语义枚举里这种"状态性"变体
    /// 容易在新增变体时被漏掉，集中一处便于审计。
    pub fn is_unset(self) -> bool {
        matches!(self, Tone::None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_tones_are_distinct_and_copyable() {
        // 色调是网格里逐格存储的，必须 Copy（否则渲染热路径要 clone 字符串）
        let t = Tone::Accent;
        let u = t;
        assert_eq!(t, u);
        assert_ne!(Tone::Text, Tone::Accent);
    }

    #[test]
    fn only_none_counts_as_unset() {
        assert!(Tone::None.is_unset());
        // 装饰色不是"未设置" —— 它已经是一段具体的颜色，
        // 背景层不该再覆盖它
        assert!(!Tone::Rgb(1, 2, 3).is_unset());
        assert!(!Tone::Text.is_unset());
    }
}
