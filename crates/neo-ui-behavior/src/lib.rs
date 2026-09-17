//! L3 · 行为层：无样式、不可见，但**难做对**的东西
//!
//! # 判据（决定一段代码该不该进这层）
//!
//! - **去掉颜色与尺寸后仍然有意义** → 放这里
//!   （"本质在描述长什么样" → 属于更上面的设计系统层）
//! - **与具体渲染后端无关** → 放这里（所以本 crate 不依赖 `neo-ui-kit`）
//!
//! # 内容来源：不是凭空设计，是从真机踩坑里提炼的
//!
//! 这两个模块分别对应 egui 版**实际踩过**的两个 bug。它们被提到行为层而不是
//! 留在那个宿主里，理由是这些坑**与渲染后端无关** —— 换成 gpui 一样会遇到
//! （gpui 有自己的焦点系统与按键处理，同类冲突会以另一种形式重现）：
//!
//! | 模块 | 修掉的那个真实 bug |
//! |---|---|
//! | [`focus`] | 两个控件争抢焦点 → 命令台里敲的命令被当成任务发给模型 |
//! | [`keys`] | 界面导航键被框架抢先消费 → `Shift+Tab` / `Esc` 行为与预期不符 |
//! | [`clock`] | 耗时不能进共享转录模型（破坏回放确定性）→ 改成可注入时间的纯逻辑 |

//! # 完整用法与边界
//!
//! 下面的内容直接来自本 crate 的 `README.md` —— **同一份文本**而不是抄一遍。
//! 好处是其中的示例代码会被 `cargo test` 当作文档测试执行：
//! 文档与实现一旦不一致，测试就会红（README 里写着过时 API 是发布的常见事故）。
//!
#![doc = include_str!("../README.md")]

pub mod clock;
pub mod fold;
pub mod focus;
pub mod keys;
pub mod scroll;

pub use clock::{format_duration, TurnClock};
pub use fold::{
    display_line_count, fold_rows, folded_line_count, FoldRow, FOLD_THRESHOLD,
    KEEP_AROUND_CHANGE,
};
pub use focus::FocusIntent;
pub use scroll::{FollowTail, ScrollPos};
// `Verdict` 必须导出：它是 `KeyArbiter::verdict()` 的**返回类型**，
// 调用方要 `match` 它就得能命名它 —— 只导出 `KeyArbiter` 与 `Layer`
// 会让"怎么用裁决结果"变成一个无法表达的问题（文档测试抓到过：
// README 示例 `use neo_ui_behavior::Verdict` 编译失败）。
pub use keys::{KeyArbiter, Layer, Verdict};
