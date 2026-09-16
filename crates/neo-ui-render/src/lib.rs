//! L2 · 渲染缝（render seam）：后端中立的绘制描述
//!
//! # 这一层为什么存在
//!
//! 若组件的签名里直接透出 GPUI 的渲染类型（`Element` / `PaintQuad` / `Window`），
//! 那么将来换后端时组件库要重写。设一条中立缝，是"渲染后端可替换"这件事
//! 在架构上**已经被固定**的保证。
//!
//! # 但它现在**不是**成熟抽象（据实标注）
//!
//! 只有一个实现（[`GpuiBackend`]）。方案文档里那条纪律值得照抄：
//! **"只有一个实现的抽象是信仰，两个实现的抽象才是设计。"**
//!
//! 因此本 crate 刻意保持小：
//! - 只有中立类型（[`Color`] / [`Rect`] / [`Op`] / [`Scene`]）与一个 trait；
//! - **不**提前写第二个 backend（在只有一个实现时，第二个实现的所有假设都会错）；
//! - **不**给常规组件用 —— 走了会丢掉 GPUI 的 element diff、脏区剔除、
//!   文本整形与字形缓存（方案称之为"烂尾最常见的起点"）。只有**自绘表面**
//!   （diff 视图 / 图表 / 自绘画布）才走它。
//!
//! 阶段 1 只落一件事：**中立场景能被翻译成某个后端的绘制产物**。
//! 让自绘组件真正消费它（把产物嵌进 element 树）是阶段 2 接 diff 视图时做的，
//! 现在不假装已经做到。

//! # 完整用法与边界
//!
//! 以下内容来自本 crate 的 `README.md` —— **同一份文本**。示例会被
//! `cargo test` 当作文档测试执行，文档漂移会立刻暴露。
//!
#![doc = include_str!("../README.md")]

pub mod backend;
pub mod gutter;
pub mod progress;
pub mod scene;
pub mod usage;

pub use backend::GpuiBackend;
pub use backend::RenderBackend;
pub use gutter::{change_gutter, GutterMark};
pub use scene::{Color, Op, Point, Rect, Scene, Size};
pub use progress::{segmented_progress, ProgressSegment};
pub use usage::{usage_bars, UsageBar};
