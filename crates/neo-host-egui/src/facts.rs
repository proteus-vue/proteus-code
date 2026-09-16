//! T6 宿主等价性的比较对象。
//!
//! 与 exec / TUI / web / desktop 用的是**同一个**事实抽取器
//! （`neo_protocol::facts_of`），因此"多宿主等价"是构造上成立的，不是巧合。
//! 这里只累积事件、按协议层抽事实 —— 不掺任何渲染差异。

use neo_core::{DiffSupport, HostBackend, HostCapabilities, ImageSupport};
use neo_protocol::{facts_of, EventMsg, Fact};

/// GUI 宿主的 `HostBackend` 视图。
///
/// 注意它**不是**"窗口"本身：窗口与绘制在 [`crate::ui`]，这里只是把同一份
/// 事件流按契据抽成事实，供 T6 断言比较。（`neo-host-desktop` 的 `DesktopHost`
/// 是同名定位 —— 那边的注释已明确它只是测试替身；这个则是真实界面用的。）
pub struct GuiFacts {
    events: Vec<EventMsg>,
}

impl GuiFacts {
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }
}

impl Default for GuiFacts {
    fn default() -> Self {
        Self::new()
    }
}

impl HostBackend for GuiFacts {
    fn id(&self) -> &'static str {
        "egui"
    }

    fn capabilities(&self) -> HostCapabilities {
        HostCapabilities {
            // 原生绘制能吃位图，但当前没有任何地方生成图片 ——
            // 声明一个用不上的能力只会让"能力表"失去意义
            images: ImageSupport::None,
            rich_text: true,
            interactive_prompt: true,
            // 能画 hunk 级 diff（审批前展示改动）
            diffs: DiffSupport::Hunk,
        }
    }

    fn consume(&mut self, event: &EventMsg) -> Result<(), String> {
        self.events.push(event.clone());
        Ok(())
    }

    fn facts(&self) -> Vec<Fact> {
        facts_of(&self.events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use neo_core::HostBackend;

    /// 与其它宿主共享的那份事件流必须被完整消费、事实数一致。
    ///
    /// 这条与 `neo-mock/tests/conformance.rs` 里的宿主契约测试**同源**，
    /// 但放在本 crate 内：契约测试用的是 `neo-mock` 的共享事件流，
    /// 这里补一个"本宿主自身"的守卫，避免改动本 crate 时才发现契约破了。
    #[test]
    fn consumes_the_shared_event_stream_and_reports_the_same_fact_count() {
        let mut h = GuiFacts::new();
        for ev in [
            EventMsg::TurnStarted { turn_id: "t1".into() },
            EventMsg::AgentMessageDone { text: "hello".into() },
            EventMsg::TurnComplete { input_tokens: 1, output_tokens: 2 },
        ] {
            h.consume(&ev).expect("宿主必须能消费完整事件流");
        }
        assert_eq!(h.facts().len(), 2, "助手发言 + 本轮结束：{:?}", h.facts());
    }

    #[test]
    fn identity_and_capabilities_are_declared() {
        let h = GuiFacts::new();
        assert_eq!(h.id(), "egui", "id 是 T6 断言里的宿主标识");
        let c = h.capabilities();
        assert!(c.rich_text, "原生 GUI 能画富文本");
        assert!(c.interactive_prompt, "能弹审批");
        // diffs 声明为 Hunk 就必须真的画 hunk 级 diff（ui 里有渲染实现）
        assert_eq!(c.diffs, DiffSupport::Hunk);
    }
}
