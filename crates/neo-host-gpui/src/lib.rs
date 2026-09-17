//! L5 HOST · 桌面原生 GUI（GPUI / gpui-kit）
//!
//! # 与 `neo-host-egui` 的关系
//!
//! 同一个位置的两种实现：都消费同一份事件流、都用 `neo-driver` 驱动、
//! 都复用 `neo-text` 的文本语义与 `neo-ui` 的品牌色。差别只在"谁来画"。
//!
//! ADR-0006「不押注单一方案」在这里的落地是：**gpui 转正、egui 冻结**
//! （只修 bug、不加功能），等 gpui 覆盖 parity 清单后删除 egui ——
//! 而不是长期双维护（那会让每个界面改动做两遍）。
//!
//! # 它一行 gpui 依赖都不声明
//!
//! 全部经 `neo-ui-kit` 取用（门禁 U2 强制）。理由不是形式主义：
//! 工程里若有两个 crate 各自声明 gpui，Cargo 会编译两份引擎，报出
//! `expected gpui::WindowContext, found gpui::WindowContext` 这种几乎无法
//! 凭直觉定位的错误 —— 类型名字一样、身份不同。
//!
//! # T6 宿主等价
//!
//! [`facts::GpuiFacts`] 用**协议层同一个** `facts_of` 抽取事实，所以
//! "四宿主等价"是构造上成立的，不是巧合。

pub mod app;
pub mod facts;

pub use app::run;
// composer 状态的**生产构造点**（`app::new_composer_state` 的转发）。
// 导出给回归用例，让它们钉住"我们实际用的那份配置"，而不是在测试里手写一份
// —— 否则产品代码改了配置，用例照样绿。见 `tests/composer_multiline.rs`。
pub use app::{composer_input, composer_state_for_test};
