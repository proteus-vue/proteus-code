//! diff **未改区块折叠**：把长 diff 里成片的上下文收成一行把手。
//!
//! # 为什么需要它（不是"少显示几行"）
//!
//! 一段真实的改动，上下文往往远多于改动本身（`apply_patch` 生成的 diff
//! 默认带 3 行上下文；一整段新函数就是一条纯新增 hunk）。默认全部展开时，
//! **改动被上下文淹掉** —— 而看 diff 要找的是改动。GitHub / Zed / ZCode
//! 默认都折起成片未改内容，理由就是这个。
//!
//! # 三条必须做对的语义（都是踩过或极易做错的地方）
//!
//! ## 1. 只折**上下文**，不折结构行
//!
//! 可折的只有 [`DiffBand::Context`]。`Header`（`---`/`+++`）与 `Meta`
//! （我们截断时追加的"…另有 N 处…"）**都不折**：
//! - 文件头折了，用户会不知道这段 diff 属于哪个文件；
//! - 截断说明折了，**"这个 diff 被截断过"这件事就消失了** —— 那是最危险的一类
//!   隐藏：用户以为看到了全部改动。
//!
//! （这正是 [`DiffBand`] 必须**完整**分类、不能"改动色 / 其它"二分的原因。）
//!
//! ## 2. 阈值与"折了是否真省"
//!
//! 只有**连续未改超过折叠阈值**（`fold_threshold`）才折，且折叠后必须**真的净省**。
//! 否则会出现"折掉 3 行、却多出一行把手"这种既不省地方又碍眼的结果 ——
//! 尤其在小窗口里，把手本身占一行，折了反而更长。
//!
//! ## 3. 改动**相邻的**上下文要保留
//!
//! 折起一大片未改时，紧邻改动的那几行（`keep` 行）必须留在屏幕上：
//! 它们是理解改动的上下文（"这个函数在哪、前面的语句是什么"）。
//! 全折掉会让人看不出改动落在什么位置。
//!
//! # 为什么是纯逻辑
//!
//! 折叠决定"屏幕上第几行显示什么"，而屏幕上的行数由折叠**自己**改变 ——
//! 这类"输入影响输出行数"的逻辑放在渲染回调里就只能靠真机试。
//! 做成纯函数后可以逐条断言（含空输入、全上下文、改动贴在首尾等边界）。
//!
//! # ⚠️ 当前**没有触发条件**（据实标注，别当成已生效的功能）
//!
//! 实测（`neo-capability::diff::unified_diff`）：每个 hunk 只带
//! **`CONTEXT = 3`** 行上下文，因此连续上下文最长就是 **3 行**，而本模块的
//! 阈值是 [`FOLD_THRESHOLD`] = 6 —— **我们自己的 diff 永远不会触发折叠**。
//!
//! 所以本模块现在的定位是**已验证但休眠的基建**，不是"用户能看到的改进"。
//! 它会在下列任一情况出现时立刻起作用（三者都不需要改本模块）：
//!
//! 1. hunk 上下文半径调大（`CONTEXT` 提到 > 6）—— 那时 diff 更接近真实
//!    diff 查看器的"宽松上下文"观感，折叠才有东西可折；
//! 2. 喂进**完整上下文**的 diff（用户粘贴的 `git diff -U100`、外部工具输出）；
//! 3. 并排视图（每侧上下文会比单栏多）。
//!
//! 反过来，**不要**为了"让它看起来在工作"而调低阈值：折 3 行上下文要多出
//! 一行把手，净收益是负的（那正是模块头部第 2 条禁止的情形）。

use neo_ui_render::DiffBand;

/// 折叠后的一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FoldRow {
    /// 照原样显示某条输入行（下标指向 `bands`）。
    Line(usize),
    /// 把 `range` 这段**上下文行**收成一行把手。`range` 是半开区间。
    ///
    /// 渲染层据此画一行"⋯ 未改 N 行"，点击后展开（展开即不传 `collapsed`
    /// 里的这个下标，见 [`fold_rows`] 的 `expanded` 参数）。
    Fold { range: std::ops::Range<usize> },
}

/// 连续未改超过这个行数才折。
///
/// 取 6：`apply_patch` 默认上下文是每侧 3 行，改动之间夹 6 行以内未改属于
/// "同一片改动区"，折了反而看不出改动分布；超过 6 行才是真正的"间隔"。
pub const FOLD_THRESHOLD: usize = 6;

/// 折起后、紧邻改动处保留的上下文行数（两侧各留）。
///
/// 取 3 与 `apply_patch` 默认上下文一致：正好是"这段改动前后长什么样"。
pub const KEEP_AROUND_CHANGE: usize = 3;

/// 计算折叠后的显示行。
///
/// - `bands`：逐行的种类（与 diff 文本行一一对应）。
/// - `expanded`：用户已点开的那几个折叠区（用**被折区间的起点**标识）。
///   传入的起点若不在本次结果里，静默忽略 —— 它来自上一次渲染，位置可能已变。
///
/// 返回的每个 [`FoldRow::Line`] 都带原下标，渲染层据此取原文。
pub fn fold_rows(
    bands: &[DiffBand],
    expanded: &std::collections::HashSet<usize>,
) -> Vec<FoldRow> {
    let mut out = Vec::with_capacity(bands.len());
    let mut i = 0;
    while i < bands.len() {
        if bands[i] != DiffBand::Context {
            out.push(FoldRow::Line(i));
            i += 1;
            continue;
        }
        // 一段连续上下文
        let start = i;
        while i < bands.len() && bands[i] == DiffBand::Context {
            i += 1;
        }
        let end = i; // 半开
        push_context_run(&mut out, bands, start..end, expanded);
    }
    out
}

/// 处理一段连续上下文 `run`：可能整体留下、可能两侧留 `keep` 后折中间、
/// 也可能整段折起。
fn push_context_run(
    out: &mut Vec<FoldRow>,
    bands: &[DiffBand],
    run: std::ops::Range<usize>,
    expanded: &std::collections::HashSet<usize>,
) {
    let len = run.end - run.start;

    // 未达阈值：原样显示（不折 —— 折了不省地方，还多一行把手）
    if len <= FOLD_THRESHOLD {
        for i in run {
            out.push(FoldRow::Line(i));
        }
        return;
    }

    // 段首/段尾是否**紧邻改动**：只有紧邻的那一侧才值得留上下文。
    // 段首紧邻改动 = 前一行是 Add/Del/Hunk；段尾同理。
    let touches_before = run.start > 0 && is_change(bands[run.start - 1]);
    let touches_after = run.end < bands.len() && is_change(bands[run.end]);

    // 两侧各留多少：紧邻改动的一侧留 `keep`，另一侧不留（它贴着 diff 的首/尾，
    // 没有改动要交代上下文 —— 留反而是白占屏幕）。
    let head = if touches_before { KEEP_AROUND_CHANGE.min(len) } else { 0 };
    let tail = if touches_after {
        KEEP_AROUND_CHANGE.min(len.saturating_sub(head))
    } else {
        0
    };

    let mid_start = run.start + head;
    let mid_end = run.end - tail;

    // 前后保留的行
    for i in run.start..mid_start {
        out.push(FoldRow::Line(i));
    }
    // 中间：够长才折（折后必须真的净省一行，见模块头部第 2 条）
    if mid_end > mid_start && mid_end - mid_start > 1 {
        if expanded.contains(&mid_start) {
            // 用户点开过：整段铺开，但**把手仍占一行**（否则没有"收起"的入口）
            out.push(FoldRow::Fold { range: mid_start..mid_end });
            for i in mid_start..mid_end {
                out.push(FoldRow::Line(i));
            }
        } else {
            out.push(FoldRow::Fold { range: mid_start..mid_end });
        }
    } else {
        // 折了不省：原样留下
        for i in mid_start..mid_end {
            out.push(FoldRow::Line(i));
        }
    }
    for i in mid_end..run.end {
        out.push(FoldRow::Line(i));
    }
}

/// 一行是否属于"改动或结构标记"（即不可折的、需要交代上下文的位置）。
///
/// `Hunk` 也算：hunk 头之后紧跟的就是改动区，它是位置锚点。
/// `Fold` 是合成行，不会作为输入出现，这里归到"改动"侧更安全
/// （万一有人把折叠结果再喂回来，也不会把结构行吃掉）。
fn is_change(band: DiffBand) -> bool {
    matches!(
        band,
        DiffBand::Add | DiffBand::Del | DiffBand::Hunk | DiffBand::Fold
    )
}

/// 折叠区里被藏起来的行数（渲染层用它显示"⋯ 未改 N 行"）。
pub fn folded_line_count(range: &std::ops::Range<usize>) -> usize {
    range.end.saturating_sub(range.start)
}

/// 显示行数（含折叠把手）—— 供"折叠后是否真的变短"这类断言使用。
pub fn display_line_count(rows: &[FoldRow]) -> usize {
    rows.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn fold(bands: &[DiffBand]) -> Vec<FoldRow> {
        fold_rows(bands, &HashSet::new())
    }

    /// 短 diff 完全不折：折了不省地方，还多一行把手。
    #[test]
    fn a_short_diff_is_left_alone() {
        let bands = [
            DiffBand::Context,
            DiffBand::Add,
            DiffBand::Context,
            DiffBand::Del,
            DiffBand::Context,
        ];
        let rows = fold(&bands);
        assert_eq!(rows, (0..5).map(FoldRow::Line).collect::<Vec<_>>());
    }

    /// 成片上下文被折起，且**折叠真的净省行数**。
    #[test]
    fn a_long_context_run_is_folded_and_really_saves_lines() {
        let mut bands = vec![DiffBand::Add];
        bands.extend(std::iter::repeat_n(DiffBand::Context, 30));
        bands.push(DiffBand::Del);

        let rows = fold(&bands);
        assert!(
            display_line_count(&rows) < bands.len(),
            "折叠后应更短：{} vs {}",
            display_line_count(&rows),
            bands.len()
        );
        let folds: Vec<_> = rows
            .iter()
            .filter_map(|r| match r {
                FoldRow::Fold { range } => Some(range.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(folds.len(), 1, "应恰好一个折叠区");
    }

    /// **紧邻改动的上下文要留下**：那几行是理解改动的上下文。
    #[test]
    fn context_next_to_a_change_is_kept() {
        let mut bands = vec![DiffBand::Add];
        bands.extend(std::iter::repeat_n(DiffBand::Context, 30));
        bands.push(DiffBand::Del);

        let rows = fold(&bands);
        // 改动（下标 0）之后应紧跟 KEEP_AROUND_CHANGE 行上下文
        let after_change: Vec<_> = rows[1..=KEEP_AROUND_CHANGE].to_vec();
        for (k, r) in after_change.iter().enumerate() {
            assert_eq!(
                r,
                &FoldRow::Line(1 + k),
                "改动后第 {} 行应保留为上下文",
                k + 1
            );
        }
        // 改动前（末尾 Del 之前）也要留
        let tail: Vec<_> = rows.iter().rev().take(KEEP_AROUND_CHANGE + 1).collect();
        assert!(
            tail.iter().any(|r| matches!(r, FoldRow::Line(_))),
            "Del 之前应保留上下文行"
        );
    }

    /// **`Meta` 绝不被折** —— 折了会让"这个 diff 被截断过"消失（最危险的隐藏）。
    #[test]
    fn meta_lines_are_never_folded() {
        let mut bands = vec![DiffBand::Add];
        bands.extend(std::iter::repeat_n(DiffBand::Context, 30));
        bands.push(DiffBand::Meta); // 我们追加的"…另有 N 处…"

        let rows = fold(&bands);
        let meta_idx = bands.len() - 1;
        assert!(
            rows.contains(&FoldRow::Line(meta_idx)),
            "截断说明行必须始终可见（它承载'被截断'这个事实）"
        );
    }

    /// **`Header` 绝不被折** —— 折了会不知道这段 diff 属于哪个文件。
    #[test]
    fn header_lines_are_never_folded() {
        let bands = vec![
            DiffBand::Header,
            DiffBand::Header,
            DiffBand::Hunk,
            DiffBand::Add,
            DiffBand::Del,
        ];
        let rows = fold(&bands);
        assert!(rows.contains(&FoldRow::Line(0)), "文件头必须可见");
        assert!(rows.contains(&FoldRow::Line(1)), "文件头必须可见");
    }

    /// 全上下文（没有任何改动）：整段折起，不留上下文（没有改动需要交代）。
    #[test]
    fn an_all_context_diff_folds_entirely() {
        let bands = vec![DiffBand::Context; 40];
        let rows = fold(&bands);
        assert_eq!(rows.len(), 1, "只应剩一行把手");
        assert!(matches!(rows[0], FoldRow::Fold { .. }));
        // 且把手报告的行数要对得上（渲染层据此显示"N 行"）
        if let FoldRow::Fold { range } = &rows[0] {
            assert_eq!(folded_line_count(range), 40);
        }
    }

    /// 展开态：被点开的折叠区**整段铺开**，但把手仍占一行（否则没有收起入口）。
    #[test]
    fn expanding_a_fold_shows_every_line_and_keeps_a_handle() {
        let bands = vec![DiffBand::Context; 40];
        let mut expanded = HashSet::new();
        expanded.insert(0);

        let rows = fold_rows(&bands, &expanded);
        assert_eq!(rows.len(), 41, "40 行 + 1 个把手");
        assert!(matches!(rows[0], FoldRow::Fold { .. }), "把手仍在第一行");
        for (k, r) in rows[1..].iter().enumerate() {
            assert_eq!(r, &FoldRow::Line(k), "其余是原文行，顺序不变");
        }
    }

    /// **顺序与内容不变**：折叠只做"隐藏"，绝不重排或丢行。
    ///
    /// 这条是折叠最根本的契约 —— 一个会重排的折叠会让 diff 读起来是错的。
    #[test]
    fn folding_only_hides_it_never_reorders() {
        let mut bands = vec![DiffBand::Header, DiffBand::Hunk, DiffBand::Add];
        bands.extend(std::iter::repeat_n(DiffBand::Context, 20));
        bands.push(DiffBand::Del);
        bands.extend(std::iter::repeat_n(DiffBand::Context, 20));
        bands.push(DiffBand::Meta);

        let rows = fold(&bands);
        let shown: Vec<usize> = rows
            .iter()
            .filter_map(|r| match r {
                FoldRow::Line(i) => Some(*i),
                _ => None,
            })
            .collect();
        // 必须严格递增 —— 不被重排
        assert!(
            shown.windows(2).all(|w| w[0] < w[1]),
            "显示行必须保持原顺序：{shown:?}"
        );
    }

    /// 退化输入：空、单行、恰好等于阈值。
    #[test]
    fn boundary_inputs_do_not_panic() {
        assert!(fold(&[]).is_empty());
        assert_eq!(fold(&[DiffBand::Add]), vec![FoldRow::Line(0)]);

        // 恰好等于阈值 → 不折（阈值是"超过才折"）
        let exact = vec![DiffBand::Context; FOLD_THRESHOLD];
        assert_eq!(fold(&exact).len(), FOLD_THRESHOLD, "等于阈值不折");

        // 阈值 + 1 条，且无改动紧邻 → 折成一行
        let over = vec![DiffBand::Context; FOLD_THRESHOLD + 1];
        assert_eq!(fold(&over).len(), 1, "超过阈值应折成一行");
    }

    /// **钉住当前的真实行为**：`apply_patch` 风格的 diff（每 hunk 3 行上下文）
    /// 不触发折叠。
    ///
    /// 这条测试的价值是**防止误解**：模块实现了折叠，但它对"我们自己生成的
    /// diff"是休眠的（见模块头部"当前没有触发条件"）。若哪天有人调小了阈值
    /// 让它"看起来在工作"，这条会红 —— 那正是需要停下来重新权衡的信号
    /// （折 3 行多一个把手，净收益为负）。
    #[test]
    fn our_own_diff_shape_does_not_fold() {
        // 一个 apply_patch 典型 hunk：3 行上下文 + 改动 + 3 行上下文
        let bands = [
            DiffBand::Header,
            DiffBand::Header,
            DiffBand::Hunk,
            DiffBand::Context,
            DiffBand::Context,
            DiffBand::Context,
            DiffBand::Del,
            DiffBand::Add,
            DiffBand::Context,
            DiffBand::Context,
            DiffBand::Context,
        ];
        let rows = fold(&bands);
        assert!(
            !rows.iter().any(|r| matches!(r, FoldRow::Fold { .. })),
            "3 行上下文的 hunk 不该触发折叠（阈值 {FOLD_THRESHOLD}）—— \
             若这条红了，说明阈值被调小，请重新评估净收益"
        );
        assert_eq!(rows.len(), bands.len(), "不折则逐行原样显示");
    }

    /// 折叠不会把"折了不省"的情况折掉（小窗口里的边界）。
    #[test]
    fn never_folds_when_it_would_not_save_a_line() {
        // 一个改动 + 一段刚过阈值的上下文：两侧 keep 之后中间只剩 1~2 行时，
        // 折了等于白多一个把手 → 不折。
        for pad in 0..12usize {
            let mut bands = vec![DiffBand::Add];
            bands.extend(std::iter::repeat_n(DiffBand::Context, FOLD_THRESHOLD + pad));
            let rows = fold(&bands);
            assert!(
                display_line_count(&rows) <= bands.len(),
                "折叠后不得比原 diff 更长（pad={pad}）：{} vs {}",
                display_line_count(&rows),
                bands.len()
            );
        }
    }
}
