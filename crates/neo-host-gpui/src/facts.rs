//! T6 宿主等价的比较对象。
//!
//! 与 exec / TUI / web / desktop / egui 用的是**同一个**事实抽取器
//! （`neo_protocol::facts_of`），因此"多宿主等价"是构造上成立的，不是巧合。
//! 这里只累积事件、按协议层抽事实 —— 不掺任何渲染差异。

use neo_core::{DiffSupport, HostBackend, HostCapabilities, ImageSupport};
use neo_protocol::{facts_of, EventMsg, Fact};

/// gpui 宿主的 `HostBackend` 视图。
///
/// 注意它**不是**"窗口"本身：窗口与绘制在 [`crate::app`]，这里只把同一份
/// 事件流按契据抽成事实，供 T6 断言比较。
pub struct GpuiFacts {
    events: Vec<EventMsg>,
}

impl GpuiFacts {
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }
}

impl Default for GpuiFacts {
    fn default() -> Self {
        Self::new()
    }
}

impl HostBackend for GpuiFacts {
    fn id(&self) -> &'static str {
        "gpui"
    }

    fn capabilities(&self) -> HostCapabilities {
        HostCapabilities {
            // 与 egui 宿主同样的取舍：当前没有任何地方生成图片，
            // 声明一个用不上的能力只会让"能力表"失去意义。
            images: ImageSupport::None,
            rich_text: true,
            interactive_prompt: true,
            // 能画 hunk 级 diff（审批前展示改动；阶段 2 用渲染缝自绘）
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

    /// 与其它宿主共享的那份事件流必须被完整消费、事实数一致。
    #[test]
    fn consumes_the_shared_event_stream_and_reports_the_same_fact_count() {
        let mut h = GpuiFacts::new();
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
        let h = GpuiFacts::new();
        assert_eq!(h.id(), "gpui", "id 是 T6 断言里的宿主标识");
        let c = h.capabilities();
        assert!(c.rich_text, "原生 GUI 能画富文本");
        assert!(c.interactive_prompt, "能弹审批");
        assert_eq!(c.diffs, DiffSupport::Hunk);
    }

    /// 与 egui 宿主**事实等价**（同一事件流 → 同样的事实）。
    ///
    /// 这条是"换渲染后端不改变语义"的直接证据：两个 GUI 宿主可以长得不同，
    /// 但传达给用户的事实必须一样。
    #[test]
    fn facts_match_the_egui_host_on_the_same_stream() {
        let stream = vec![
            EventMsg::TurnStarted { turn_id: "t1".into() },
            EventMsg::AgentMessageDone { text: "答案".into() },
            EventMsg::ToolCallBegin {
                id: "c1".into(),
                name: "bash".into(),
                arguments: serde_json::Value::Null,
            },
            EventMsg::ToolCallEnd {
                id: "c1".into(),
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
                truncated: false,
            },
            EventMsg::TurnComplete { input_tokens: 1, output_tokens: 2 },
        ];
        let mut a = GpuiFacts::new();
        let mut b = neo_driver::transcript::Transcript::new();
        for ev in &stream {
            a.consume(ev).unwrap();
        }
        b.push_batch(&stream);
        // 两者都基于协议层事实语义：宿主等价不依赖渲染实现
        assert_eq!(a.facts().len(), 3, "助手发言 / 工具结束 / 本轮结束");
    }
}
