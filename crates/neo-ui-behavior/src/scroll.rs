//! 滚动跟随：判断"新内容到达时该不该自动滚到底部"。
//!
//! # 为什么这件事需要单独的逻辑
//!
//! 流式输出（模型边生成边显示）的界面里，"自动跟随"看起来是个视觉细节，
//! 实际上有一个**容易做错且做错就很难用**的语义：
//!
//! - 单纯"每次新内容都滚到底" → 用户想往上翻看之前的输出时，会被不断拽回底部，
//!   等于**没法读历史**。流式输出持续几秒到几分钟，这个窗口里界面是锁死的。
//! - 单纯"从不自动滚" → 用户得手动追着滚动条跑，长回答里体验很差。
//!
//! 正确语义是"**只有用户本来就在底部时才跟随**"：他在底部 = 他在看最新内容，
//! 这时跟随符合意图；他往上翻了 = 他在读历史，这时任何自动滚动都是干扰。
//!
//! 这个判断与渲染框架无关（换成任何 GUI 都是一样的规则），
//! 所以放在行为层，并且做成**纯函数 + 可注入的输入**：
//! 调用方给"当前偏移 / 最大偏移 / 视口高度"，它回答"该不该跟随"。
//!
//! # 为什么做成纯逻辑而不是塞进宿主
//!
//! 塞进宿主的后果是它只能靠真机试：滚动状态要在真实窗口里才有，
//! 而"往上翻时被拽回去"这种问题在截图里**完全看不出来**（截图是静止的）。
//! 做成纯函数后，边界条件（刚好在底部、底部阈值、内容不足以滚动）都能断言。

/// "算作在底部"的容差。
///
/// # 为什么不能要求严格等于
///
/// 浮点偏移与整数化后的像素高度几乎永远不会精确相等：一次滚轮滚动会过冲
/// 或欠冲若干分之一像素，设备像素比还会引入小数。若判定条件是
/// `offset == max_offset`，结果是**自动跟随几乎永远不生效** ——
/// 而症状（"有时候跟随有时候不跟随"）看起来像随机 bug，极难定位。
///
/// 取 4 逻辑像素：小于"一行文字"的高度，用户视觉上仍在底部；
/// 又大到足以吸收滚动过冲与 DPI 取整误差。
const AT_BOTTOM_TOLERANCE: f32 = 4.0;

/// 滚动位置。
///
/// # 符号契约（**务必读**）
///
/// 本类型的两个字段都是**非负**的"从上往下的距离"：
/// `offset` = 已经滚下去多少，`max_offset` = 最多能滚多少。
/// 到底时 `offset == max_offset`。
///
/// 而多数 GUI 框架内部的偏移是**负值**（"往下滚"记为坐标减小）。
/// 这个差异会造成一类很难查的 bug：**判定条件写成 `offset >= max_offset`
/// 时，若误把框架的负值直接传进来，条件恒为假 —— 于是自动跟随永远不生效，
/// 而纯逻辑单测全绿**（测试里两个输入都是正的）。
///
/// 所以不要在调用方手写转换：用 [`ScrollPos::from_raw`]，
/// 让符号约定只在一处表达。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ScrollPos {
    /// 已滚动的距离（**非负**，从顶部算）。
    pub offset: f32,
    /// 最大可滚动距离（内容高度 − 视口高度，**非负**）；内容不足一屏时为 0。
    pub max_offset: f32,
}

impl ScrollPos {
    /// 用"从上往下量的距离"构造（本类型的原生契约）。
    pub fn new(offset: f32, max_offset: f32) -> Self {
        Self { offset, max_offset }
    }

    /// 从框架的原始偏移构造。
    ///
    /// `raw_offset` 是框架约定的偏移（**往下滚为负**，如 gpui 的
    /// `ScrollHandle::offset()`），`raw_max` 是最大滚动量（正值，
    /// 如 `max_offset()`）。本函数负责把符号翻正 ——
    /// 于是"符号搞反"这件事不可能在调用点发生。
    pub fn from_raw(raw_offset: f32, raw_max: f32) -> Self {
        Self {
            // 取绝对值：框架的 offset 到底时等于 -max，翻正即为 max。
            // 用 abs 而不是取负，是为了兼容"已经是正值"的实现（幂等），
            // 也让 offset 超过 max（视口变大）时不会变成负数。
            offset: raw_offset.abs(),
            max_offset: raw_max.abs(),
        }
    }

    /// 内容是否还不足以产生滚动条。
    ///
    /// 这一条不能漏：内容不足一屏时 `offset` 与 `max_offset` 都是 0，
    /// 会被判定为"在底部"从而触发跟随 —— 而跟随本身无害（没有可滚动的
    /// 空间），但它会让"是否应该跟随"这个判断在启动初期恒为真，
    /// 掩盖真正的判定逻辑。显式区分出来更好读、也更好测。
    pub fn content_fits_viewport(&self) -> bool {
        self.max_offset <= 0.0
    }

    /// 用户是否在（或接近）底部。
    pub fn at_bottom(&self) -> bool {
        if self.content_fits_viewport() {
            return true;
        }
        // 反向超出（内容变短、或视口突然变大）也算在底部：
        // 此时 `offset > max_offset`，差值可能很大，但语义上就是"到底了"
        self.offset >= self.max_offset - AT_BOTTOM_TOLERANCE
    }
}

/// 自动跟随状态。
///
/// 用法：每帧渲染前取一次 [`ScrollPos`] 交给 [`Self::should_follow`]，
/// 得到 `true` 就调用后端的"滚到底"。
///
/// ```text
/// // 宿主侧（伪代码）
/// let pos = scroll_handle_to_pos(&handle);
/// if follow.should_follow(pos) {
///     handle.scroll_to_bottom();
/// }
/// ```
#[derive(Debug, Clone, Default)]
pub struct FollowTail {
    pos: ScrollPos,
    /// 是否已经有过一次成功跟随。用于区分"初始状态"与"用户主动滚离底部"。
    followed_once: bool,
}

impl FollowTail {
    pub fn new() -> Self {
        Self::default()
    }

    /// 是否应该跟随到底部。
    ///
    /// **返回 `false` 不是失败**：它表示"用户正在读历史，别打扰他"。
    pub fn should_follow(&self, pos: ScrollPos) -> bool {
        pos.at_bottom()
    }

    /// 记录当前滚动位置（宿主每帧调用）。
    ///
    /// 它只用于诊断（[`Self::last_pos`]）：判定本身是无状态的纯函数
    /// （只看这一帧的位置），所以"记住上一帧"不参与决策 ——
    /// 有状态会让行为依赖于"宿主是否每次都调了它"，是多余的失败面。
    pub fn observe(&mut self, pos: ScrollPos) {
        self.pos = pos;
        if pos.at_bottom() {
            self.followed_once = true;
        }
    }

    /// 最近一次观察到的位置（诊断用）。
    pub fn last_pos(&self) -> ScrollPos {
        self.pos
    }

    /// 是否曾经跟随过（诊断用）。
    pub fn has_followed(&self) -> bool {
        self.followed_once
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **符号契约**：框架的负偏移必须被翻正 ——
    /// 若把负值直接喂进判定，`offset >= max_offset` 恒为假，
    /// 自动跟随永远不生效，而纯逻辑单测全绿（测试里输入都是正的）。
    #[test]
    fn from_raw_flips_the_frameworks_negative_offset() {
        // gpui 的约定：往下滚为负，到底时 offset == -max
        let pos = ScrollPos::from_raw(-1000.0, 1000.0);
        assert_eq!(pos.offset, 1000.0, "负的原始偏移必须翻正");
        assert_eq!(pos.max_offset, 1000.0);
        assert!(pos.at_bottom(), "翻正之后才判得出'在底部'");

        // 往上滚了一部分（原始值仍为负，绝对值更小）
        let up = ScrollPos::from_raw(-400.0, 1000.0);
        assert!(!up.at_bottom());
    }

    /// `from_raw` 对已经是正值的实现也要成立（幂等），
    /// 否则换后端时同一份逻辑会突然失效。
    #[test]
    fn from_raw_is_idempotent_for_positive_inputs() {
        let pos = ScrollPos::from_raw(500.0, 1000.0);
        assert_eq!(pos.offset, 500.0);
        assert!(!pos.at_bottom());
    }

    #[test]
    fn at_the_bottom_follows() {
        let pos = ScrollPos::new(1000.0, 1000.0);
        assert!(pos.at_bottom());
        assert!(FollowTail::new().should_follow(pos));
    }

    /// **核心用例**：用户往上翻了，就不该跟随 ——
    /// 否则流式输出期间用户没法读历史（会被不断拽回底部）。
    #[test]
    fn scrolled_up_does_not_follow() {
        let pos = ScrollPos::new(400.0, 1000.0);
        assert!(!pos.at_bottom());
        assert!(
            !FollowTail::new().should_follow(pos),
            "用户在看历史时，自动滚动是干扰"
        );
    }

    /// 容差：滚动过冲/DPI 取整产生的几像素误差不该让跟随失效。
    /// 若改成严格等于，症状是"跟随有时有有时没有"，极难定位。
    #[test]
    fn small_offset_gaps_still_count_as_bottom() {
        for gap in [0.5_f32, 1.0, 2.0, 3.99] {
            let pos = ScrollPos::new(1000.0 - gap, 1000.0);
            assert!(pos.at_bottom(), "差 {gap}px 应仍算在底部");
        }
        // 明确超出容差就不算了
        let pos = ScrollPos::new(1000.0 - AT_BOTTOM_TOLERANCE - 0.01, 1000.0);
        assert!(!pos.at_bottom());
    }

    #[test]
    fn content_shorter_than_viewport_is_at_bottom() {
        let pos = ScrollPos::new(0.0, 0.0);
        assert!(pos.content_fits_viewport());
        assert!(pos.at_bottom(), "没有可滚动的空间，等于在底部");
    }

    /// 视口突然变大（窗口被拉伸）或内容变短，会让 `offset > max_offset`。
    /// 那也应当算"在底部"（语义上就是到底了），而不是因为差值为负就不跟随。
    #[test]
    fn offset_beyond_max_still_counts_as_bottom() {
        let pos = ScrollPos::new(1200.0, 1000.0);
        assert!(pos.at_bottom(), "偏移超过最大值也是到底了");
    }

    #[test]
    fn observe_remembers_for_diagnostics_without_affecting_decision() {
        let mut f = FollowTail::new();
        assert!(!f.has_followed());
        f.observe(ScrollPos::new(500.0, 1000.0));
        assert_eq!(f.last_pos(), ScrollPos::new(500.0, 1000.0));
        assert!(!f.has_followed(), "没到过底部就不算跟随过");
        // 判定与"有没有 observe 过"无关（无状态决策）
        assert!(!f.should_follow(ScrollPos::new(500.0, 1000.0)));
        f.observe(ScrollPos::new(1000.0, 1000.0));
        assert!(f.has_followed());
    }
}
