//! L1 · UI 门面层：**全仓唯一 pin GPUI 的地方**
//!
//! # 这一层的唯一职责是「单一 pin」
//!
//! GPUI 生态的头号杀手是**类型身份分裂**：工程里若有两个 crate 各自声明了不同
//! 来源/版本的 gpui，Cargo 会编译两份引擎，然后报出人类很难读的错误 ——
//! 形如 `expected gpui::WindowContext, found gpui::WindowContext`，
//! 类型名字一样但身份不同，靠读错误信息几乎定位不到。
//!
//! 所以约定：**只有本 crate 的 Cargo.toml 可以出现 gpui 系列依赖**，
//! 其余所有 crate（`neo-ui-render` / `neo-ui-behavior` / `neo-ui` / `neo-host-gpui`）
//! 一律 `use neo_ui_kit::gpui::...`。这条由 `check_ui_layering.py` 机器强制 ——
//! 靠约定守不住，因为"加一行依赖"看起来永远是无害的。
//!
//! # 取用方式
//!
//! ```ignore
//! use neo_ui_kit::gpui::{div, px, IntoElement, Render, Window};  // gpui 本体
//! use neo_ui_kit::component::{Button, Root, ActiveTheme};        // 60+ 带样式组件
//! use neo_ui_kit::{base, assets, platform};                      // 其余三层
//! ```
//!
//! # 与方案文档的一处差异（实测修正）
//!
//! `docs/gpui-方案` 主张 pin `gpui-ce`，但它的组件库 `gpui-kit` 实际依赖
//! `gpui-pre` + `gpui_platform`（后者未发布到 crates.io，手工声明会直接报错）。
//! 因此这里 pin 的是 **`gpui-kit` 一个依赖**：它把 gpui / platform / base /
//! component / assets 全带齐且版本自洽。详见本 crate 的 Cargo.toml 注释。

// ── gpui 本体（`gpui_kit` 内部 re-export 的 `gpui-pre`，lib 名就是 `gpui`）──
pub use gpui_kit::gpui;

// ── 其余三层 ──
pub use gpui_kit::assets;
pub use gpui_kit::base;
pub use gpui_kit::component;
pub use gpui_kit::platform;

/// 原始门面：需要精确路径（比如引用 `gpui-kit` 自己定义的类型）时用。
///
/// 直接 `pub use gpui_kit::*` 会与上面的分层 re-export 产生歧义，
/// 所以留一个显式出口。
pub use gpui_kit;

/// 初始化 GPUI 与各层全局状态。**必须在建任何窗口之前调用一次。**
///
/// 内部会装：主题（`Theme` 全局）、全局态、以及 dialog / sheet / notification /
/// input / list / command / menu / tooltip 等组件的初始化钩子。
pub fn init(cx: &mut gpui::App) {
    gpui_kit::init(cx);
}

/// 建一个已挂好平台后端的 `Application`。
///
/// 与 `gpui::Application::new()` 的区别：后者需要调用方自己提供
/// `Rc<dyn Platform>`，而 `gpui_platform` 并未在 crates.io 上单独发布 ——
/// 所以**必须**走这个入口（门面层的存在价值之一）。
///
/// 要显示图标就再串 `.with_assets(neo_ui_kit::assets::Assets)`。
pub fn application() -> gpui::Application {
    gpui_kit::application()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 门面必须真的把 gpui 本体与各层都透出来。
    ///
    /// 这条测试的意义：如果哪天 `gpui-kit` 改了 re-export 结构，
    /// 错误会在这里以"某个路径不存在"的形式出现，而不是在下游某个 UI 文件里
    /// 变成一堆难读的类型错误。
    #[test]
    fn the_facade_exposes_every_layer() {
        // gpui 本体：几个最常用的入口
        let _ = gpui::px(1.0);
        let _div = gpui::div();
        // 组件层：Root 是窗口首层的硬要求，必须可达
        let _ = std::any::type_name::<component::Root>();
        // 名字要在（用于单一 pin 守卫的自检）
        assert_eq!(std::any::type_name::<gpui::App>().contains("gpui"), true);
    }
}
