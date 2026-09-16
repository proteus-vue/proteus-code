//! 轮次计时：把"这一轮跑了多久"做成**可注入时间**的纯逻辑。
//!
//! # 为什么不能把耗时写进共享转录模型
//!
//! 共享模型（`neo-driver::transcript`）必须**确定性**：同一份会话日志回放
//! 必须得到同一个转录（这是 T2 与"kill 后重启续跑"的基础）。
//! 挂钟时间天然不确定 —— 把它塞进共享模型会让回放出来的转录与当时不同。
//!
//! 本项目已经因为同一件事拒绝过一个特性：Goal 的**挂钟停止条件**未实现，
//! 理由写在诚实清单里（"会破坏回放确定性"）。这里是同一条边界的另一个面。
//!
//! # 所以：时间由宿主提供，逻辑放共享层
//!
//! 本模块不认识任何协议类型，只做一件事：**给一个时刻，算出这一段有多长**。
//! 时间从外面注入（`now`），于是：
//! - 测试可以用假时间做确定性断言（不必 sleep）；
//! - 两个 GUI 宿主各传自己的 `Instant::now()`，得到同一套计算；
//! - 共享转录模型保持确定性，不受影响。
//!
//! # 与"必须靠真机看"的关系
//!
//! 计时是**数字**，不是观感 —— 所以它比颜色/尺寸那类问题更适合纯测试。
//! 这也是它值得从宿主里抽出来的理由：留在宿主里就只能靠真机读秒。

use std::time::Duration;

/// 一轮的计时器。
///
/// 用法：收到"轮次开始"时 [`Self::start`]，收到"轮次结束"时 [`Self::elapsed_since_start`]。
/// 两个调用点由宿主决定（它才知道哪个事件是边界）。
#[derive(Debug, Clone, Copy, Default)]
pub struct TurnClock {
    start: Option<Duration>,
}

impl TurnClock {
    pub const fn new() -> Self {
        Self { start: None }
    }

    /// 记录轮次开始。
    ///
    /// **重复调用取最早那次**：一轮里 `TurnStarted` 只会发一次，但宿主可能因
    /// 重连/重放而多调一次 —— 那时保留更早的起点才符合"这一轮从什么时候开始"。
    /// 反过来的话（覆盖成更晚），耗时会被低估，而低估比高估更难被发现。
    pub fn start(&mut self, now: Duration) {
        if self.start.is_none() {
            self.start = Some(now);
        }
    }

    /// 从开始到 `now` 的耗时。没有开始过则 `None`（**不是 0**）。
    ///
    /// 区分 `None` 与 `0` 是刻意的：`0` 是一个真实答案（"不到 1ms"），
    /// 而 UI 上把"没计过时"显示成"0s"会让用户以为这一轮是瞬时的 ——
    /// 那是在编造信息。调用方应按"没有耗时数据"处理 `None`。
    pub fn elapsed_since_start(&self, now: Duration) -> Option<Duration> {
        let start = self.start?;
        // 时钟回拨/乱序（`now < start`）时给 0 而不是 panic 或负数：
        // 计时是展示信息，不该让界面因为一次反常读数崩掉。
        Some(now.saturating_sub(start))
    }

    /// 结束并复位（下一轮重新计时）。
    pub fn finish(&mut self) {
        self.start = None;
    }

    /// 是否正在计时。
    pub fn is_running(&self) -> bool {
        self.start.is_some()
    }
}

/// 把耗时格式化成界面用的短串。
///
/// 规则（对齐终端与 GUI 的常见做法，且**不编造精度**）：
/// - < 1s → `"123ms"`（毫秒有意义，因为模型首字常在几百毫秒级）
/// - 1s–60s → `"4.2s"`
/// - ≥ 60s → `"1m23s"`
///
/// 不显示"0.0s"：那会让"很快"与"没测到"看起来一样。
pub fn format_duration(d: Duration) -> String {
    let ms = d.as_millis();
    if ms < 1000 {
        return format!("{ms}ms");
    }
    let secs = d.as_secs_f64();
    if secs < 60.0 {
        return format!("{secs:.1}s");
    }
    let m = d.as_secs() / 60;
    let s = d.as_secs() % 60;
    format!("{m}m{s}s")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn measures_from_start_to_now() {
        let mut c = TurnClock::new();
        assert!(!c.is_running());
        c.start(ms(1000));
        assert!(c.is_running());
        assert_eq!(c.elapsed_since_start(ms(3500)), Some(ms(2500)));
    }

    /// 没开始过就是 `None`，**不是 0**。
    ///
    /// 这条防的是"界面显示 0s 让人以为这一轮是瞬时的" —— 那是在编造信息。
    #[test]
    fn without_a_start_there_is_no_duration_not_zero() {
        let c = TurnClock::new();
        assert_eq!(c.elapsed_since_start(ms(500)), None);
    }

    /// 重复 `start` 取**最早**那次（耗时宁高估不低估）。
    #[test]
    fn repeated_starts_keep_the_earliest() {
        let mut c = TurnClock::new();
        c.start(ms(100));
        c.start(ms(900)); // 后到的起点不该覆盖
        assert_eq!(c.elapsed_since_start(ms(1000)), Some(ms(900)));
    }

    /// 时钟反常（`now` 早于 `start`）不 panic、不给负数。
    #[test]
    fn a_backwards_clock_is_clamped_not_panicking() {
        let mut c = TurnClock::new();
        c.start(ms(5000));
        assert_eq!(c.elapsed_since_start(ms(1000)), Some(Duration::ZERO));
    }

    /// `finish` 复位，下一轮重新计时（不能把两轮累加）。
    #[test]
    fn finishing_resets_for_the_next_turn() {
        let mut c = TurnClock::new();
        c.start(ms(0));
        assert_eq!(c.elapsed_since_start(ms(1000)), Some(ms(1000)));
        c.finish();
        assert!(!c.is_running());
        assert_eq!(c.elapsed_since_start(ms(2000)), None, "上一轮的起点必须清掉");
        c.start(ms(3000));
        assert_eq!(
            c.elapsed_since_start(ms(3500)),
            Some(ms(500)),
            "新轮应从新起点算，而不是接着上一轮"
        );
    }

    #[test]
    fn formatting_picks_a_sensible_unit() {
        assert_eq!(format_duration(ms(0)), "0ms");
        assert_eq!(format_duration(ms(123)), "123ms");
        assert_eq!(format_duration(ms(999)), "999ms");
        // 1s 起用秒（毫秒在长耗时里是噪声）
        assert_eq!(format_duration(ms(1000)), "1.0s");
        assert_eq!(format_duration(ms(4200)), "4.2s");
        assert_eq!(format_duration(ms(59_900)), "59.9s");
        // 1 分钟起用 m+s
        assert_eq!(format_duration(Duration::from_secs(60)), "1m0s");
        assert_eq!(format_duration(Duration::from_secs(83)), "1m23s");
    }

    /// 格式化**不显示 0.0s**：那会让"很快"与"没测到"看起来一样。
    #[test]
    fn formatting_never_claims_a_zero_second() {
        // 只要是 0 就落在毫秒分支（"0ms"），不会出现"0.0s"
        let zero = format_duration(Duration::ZERO);
        assert_eq!(zero, "0ms");
        assert!(!zero.contains("0.0s"), "不能出现 0.0s");
    }
}
