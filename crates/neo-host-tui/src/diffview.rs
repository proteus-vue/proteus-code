//! Diff 查看器（TUI 侧）—— **纯逻辑已上移到 `neo-capability::diff_view`**。
//!
//! # 这里为什么只剩一行再导出
//!
//! 解析、光标/滚动、hunk 与文件导航、视图模式（unified/split）原本都在本文件。
//! 桌面宿主也要同一套导航时，选择是"抄一份"还是"上移"——选了后者：
//! 同一个 `@@` 头在两个宿主里被解析出不同的 hunk，是最难解释的一类不一致
//!（TUI 说"下一处改动在这"，GUI 跳去别处）。
//!
//! 而且解析器**必须与生产者同处一地**：`unified_diff` 就在 `neo_capability::diff`。
//! 所以上移到那里，两个宿主共用；本模块保留再导出，让 `lib.rs` 里的
//! `diffview::Kind` 这类引用与渲染代码一行都不用改。
//!
//! **渲染仍在本 crate**（`lib.rs` 用 `Grid` 画）：终端与服务端无关的排版细节
//! 不该污染共享层 —— 这正是"纯逻辑上移、渲染留下"的边界。
//!
//! 测试也随之移走（它们是这套逻辑的主要保障，跟着代码走才不会漂移）：
//! 见 `crates/neo-capability/src/diff_view.rs` 的 `mod tests`。
pub use neo_capability::diff_view::*;
