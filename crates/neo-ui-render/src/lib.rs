//! L2 · 渲染缝（render seam）：后端中立的绘制描述
//!
//! # 这一层为什么存在
//!
//! 若组件的签名里直接透出 GPUI 的渲染类型（`Element` / `PaintQuad` / `Window`），
//! 那么将来换后端时组件库要重写。设一条中立缝，是"渲染后端可替换"这件事
//! 在架构上**已经被固定**的保证。
//!
//! # 它现在是**有两个实现**的缝（但还不是成熟抽象）
//!
//! 方案文档里那条纪律值得照抄：
//! **"只有一个实现的抽象是信仰，两个实现的抽象才是设计。"**
//! 本 crate 长期只有一个实现（[`GpuiBackend`]），那句话也就一直挂着。
//! 现在有了第二个 —— [`HeadlessBackend`]（无 GPU、可在 CI 跑），
//! 并且 `tests/conformance.rs` 用**同一份用例**跑它们。于是"后端可替换"
//! 第一次是被验证过的事实，而不是宣称。
//!
//! **但别把这一步读大了**：无头后端是**对照物 / 测试替身**，不是第二个
//! 生产渲染路径。方案 Phase 4 要的 VelloBackend 仍未做，且有明确触发条件
//! （主应用稳定 ≥ 6 个月 + ≥3 个自绘组件 + 专职人力）。
//! 本 crate 仍然刻意保持小：
//! - 只有中立类型（[`Color`] / [`Rect`] / [`Op`] / [`Scene`]）与一个 trait；
//! - **不**提前写生产用的第二个 GPU 后端（在只有一个生产实现时，
//!   第二个实现的所有假设都会错）；
//! - **不**给常规组件用 —— 走了会丢掉 GPUI 的 element diff、脏区剔除、
//!   文本整形与字形缓存（方案称之为"烂尾最常见的起点"）。只有**自绘表面**
//!   （diff 视图 / 图表 / 自绘画布）才走它。
//!
//! # 能力边界是**后端的事实**，不是中立层的属性
//!
//! [`RenderBackend::supported`] 报的是**该后端**画得出什么：`GpuiBackend`
//! 画不出文字（需字体上下文），`HeadlessBackend` 画得出。两个后端能力不同
//! 这件事本身，就是"这个查询该属于后端"的证明 —— 放进中立层就必然要按
//! 某一个后端写死。[`unsupported_ops`] 把"场景里有后端画不出的指令"
//! 变成可断言、可进门禁的事实，而不是一个静默消失的图形。

//! # 完整用法与边界
//!
//! 以下内容来自本 crate 的 `README.md` —— **同一份文本**。示例会被
//! `cargo test` 当作文档测试执行，文档漂移会立刻暴露。
//!
#![doc = include_str!("../README.md")]

pub mod backend;
pub mod diff;
pub mod gutter;
pub mod headless;
pub mod raster;
pub mod progress;
pub mod scene;
pub mod usage;

pub use backend::GpuiBackend;
pub use backend::RenderBackend;
// 「本后端画不画得出这条指令」+ 检查器：让"静默丢掉"变成可断言、可进门禁的事实。
pub use backend::{unsupported_ops, GpuiQuad, GpuiStroke};
// 中立 `Color` → gpui `Rgba`。宿主需要它把「缝里的中立颜色」交给 gpui 的
// element（`.bg()` / `.text_color()`）—— 那是**文字**所在的层，不走缝。
pub use backend::to_gpui_rgba;
// 渲染缝的**第二个实现**（无头）：把"只有一个实现的抽象是信仰"兑现成"两个
// 实现的设计"。它与 GpuiBackend 的能力矩阵刻意不同（文字正好相反）。
pub use headless::{color_seq_of_scene, HeadlessBackend, HeadlessCmd, HeadlessPaint};
// 自绘 diff 背景带（P2 自绘 DiffView 的第一层：底）。第四个缝的消费者。
pub use diff::{diff_backdrop, DiffBand, DiffBandStyle};
pub use gutter::{change_gutter, GutterMark};
pub use scene::{Color, Op, Point, Rect, Scene, Size};
pub use progress::{segmented_progress, ProgressSegment};
pub use usage::{usage_bars, UsageBar};
