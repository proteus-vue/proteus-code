//! L4 · 设计系统：NEO 的品牌外观 + 带样式的组件
//!
//! # 这一层的边界
//!
//! 判据（与行为层相对）：
//! - **本质在描述"长什么样"** → 这里
//! - **去掉颜色尺寸后仍然有意义** → 上面那层的 [`neo_ui_behavior`]
//!
//! # 颜色只有一个来源
//!
//! 所有颜色从 `neo_text::palette::NEO` 派生 —— 与 TUI、egui 宿主用的是
//! **同一份**品牌色。这条不是省事，是"同一个回复在三个宿主里长得一样"的支点：
//! `Tone::Primary` 在终端是 ANSI 紫、在 gpui 窗口是 `#a78bfa`，
//! 但"它是哪一个语义色"由同一个调色板定义。
//!
//! 硬编码色值会同时破坏两件事：换主题、以及跨宿主的颜色一致性。
//! 所以本 crate **禁止**出现字面色值（`#[allow]` 之外的任何 `rgb(0x...)`）。

//! # 完整用法与边界
//!
//! 以下内容来自本 crate 的 `README.md` —— **同一份文本**。示例会被
//! `cargo test` 当作文档测试执行，文档漂移会立刻暴露。
//!
#![doc = include_str!("../README.md")]

pub mod text;
pub mod theme;

pub use text::{rich_text, RichText};
pub use theme::{apply_neo_theme, base_bg, neo_color, panel_bg, RADIUS};
