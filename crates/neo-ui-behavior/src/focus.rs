//! 焦点意图：**单一请求**，而不是几个各自为政的布尔。
//!
//! # 它修掉的那个真实 bug
//!
//! egui 版里，焦点意图曾用两个独立布尔表达（"任务输入框该拿焦点"、
//! "命令台输入框该拿焦点"），各自在绘制时 `request_focus()`。问题在于
//! **它们是竞争关系，而胜负取决于面板的绘制顺序** —— 后画的赢。
//!
//! 真机后果不是"焦点跑错地方"这么轻：命令台里敲的命令被当成任务**发给了模型**，
//! 白花一次真实模型请求；而用户看到的是"我明明在终端里敲的，怎么变成聊天了"。
//!
//! 根因是**意图表达方式**错了：多个独立布尔无法表达"同一时刻只该有一个焦点目标"，
//! 而这个约束恰恰是焦点系统的本意。改成单一请求后，竞争天然消失：
//! 每帧至多一个控件去抢焦点，**谁最后设置谁生效，与绘制顺序无关**。
//!
//! # 为什么在行为层而不是某个宿主里
//!
//! 与渲染后端无关：换成 gpui 后同样会写"多个地方各自请求焦点"这种代码。
//! 把它做成有测试的小类型，比在每个宿主里重复一遍"记得用单一请求"的纪律可靠。

/// 焦点意图（单一槽位）。
///
/// `T` 是"焦点目标"标记类型 —— 由调用方定义（枚举最合适），
/// 本类型不关心具体有哪些目标，只保证**同时至多一个待处理请求**。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusIntent<T> {
    pending: Option<T>,
}

impl<T> Default for FocusIntent<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> FocusIntent<T> {
    /// 新建（无待处理请求）。
    pub const fn new() -> Self {
        Self { pending: None }
    }

    /// 请求把焦点交给 `target`。**覆盖**已有请求（后设置者生效）。
    ///
    /// "覆盖"是有意的：意图表达的是"这一帧结束时焦点该在哪"，
    /// 而不是"历史上谁请求过"。若改成"先到先得"，就又回到了顺序耦合。
    pub fn request(&mut self, target: T) {
        self.pending = Some(target);
    }

    /// 仅在当前**没有**待处理请求时才请求（用作"归还焦点"这类兜底）。
    ///
    /// 这个方法是修掉第二个 bug 的关键：动作自己指定的焦点目标，
    /// 不该被随后的"关闭覆盖层就归还焦点"覆盖掉。当时的写法是无条件归还，
    /// 于是 `/terminal` 指定的目标被覆盖，命令又跑回了模型。
    pub fn request_if_none(&mut self, target: T) {
        if self.pending.is_none() {
            self.pending = Some(target);
        }
    }

    /// 有没有待处理请求（调试/断言用；正常流程用 [`Self::take`]）。
    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// 取走待处理请求（消费掉）。
    ///
    /// 取走即清空：这样"抢焦点"每帧最多发生一次，不会因为组件重绘
    /// 反复抢（那会让用户无法把焦点移到别处）。
    pub fn take(&mut self) -> Option<T> {
        self.pending.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Target {
        Composer,
        Terminal,
    }

    #[test]
    fn a_single_request_is_taken_once() {
        let mut f = FocusIntent::new();
        assert!(!f.is_pending());
        f.request(Target::Composer);
        assert!(f.is_pending());
        assert_eq!(f.take(), Some(Target::Composer));
        assert_eq!(f.take(), None, "取走后不该再吐出同一个请求");
        assert!(!f.is_pending());
    }

    /// **顺序无关**：后请求的赢。
    ///
    /// 这正是旧实现（两个独立布尔 + 绘制顺序决定胜负）错掉的地方：
    /// 意图不该依赖"谁先谁后画的"。
    #[test]
    fn the_last_request_wins_regardless_of_order() {
        let mut a = FocusIntent::new();
        a.request(Target::Composer);
        a.request(Target::Terminal);
        assert_eq!(a.take(), Some(Target::Terminal), "后设置者生效");

        // 反向顺序 → 反向结果。说明结果只由"最后设置的是谁"决定。
        let mut b = FocusIntent::new();
        b.request(Target::Terminal);
        b.request(Target::Composer);
        assert_eq!(b.take(), Some(Target::Composer));
    }

    /// **兜底不覆盖显式请求** —— 这是修掉"命令发错通道"的那一条。
    #[test]
    fn a_fallback_does_not_override_an_explicit_request() {
        let mut f = FocusIntent::new();
        f.request(Target::Terminal); // 动作显式指定：焦点给命令台
        f.request_if_none(Target::Composer); // 关闭覆盖层的兜底"归还焦点"
        assert_eq!(
            f.take(),
            Some(Target::Terminal),
            "兜底绝不能覆盖显式请求（否则命令台里敲的命令会被当成任务发给模型）"
        );
    }

    #[test]
    fn a_fallback_applies_when_nothing_was_requested() {
        let mut f = FocusIntent::new();
        f.request_if_none(Target::Composer);
        assert_eq!(f.take(), Some(Target::Composer), "没有显式请求时兜底生效");
    }

    #[test]
    fn requesting_twice_keeps_only_one() {
        // 不变量：槽位永远只有一个 —— 这是"单一请求"的字面含义，
        // 也是它相比"多个布尔"能消除竞争的原因。
        let mut f = FocusIntent::new();
        for _ in 0..5 {
            f.request(Target::Composer);
        }
        assert_eq!(f.take(), Some(Target::Composer));
        assert_eq!(f.take(), None, "无论请求几次，都只留一个待处理项");
    }
}
