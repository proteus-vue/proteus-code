//! 行级 unified diff —— 给"审批前把改动给用户看"用
//!
//! # 为什么审批必须看到 diff
//!
//! 只说"写入类调用需确认"，用户其实是在**盲批**：不知道改哪个文件、
//! 改了什么、删了多少行。opencode 的权限弹窗会先渲染 diff。
//! 这个模块提供那份 diff（协议层已有 `EventMsg::PatchProposed`，
//! 之前一直没被发出过）。
//!
//! # 内存有界是硬要求（**上限由我们守，不由算法库守**）
//!
//! 对齐算法曾自写 LCS（朴素 DP 是 O(n×m) 内存：两个 4000 行的文件要
//! 1600 万格）。现改用 `similar`（Myers/Patience）——**但换算法不换策略**：
//! `similar` **不提供任何上限保证**，会老老实实展开整个 diff，所以
//! 「超限就不对齐」「hunk 数封顶」「如实标注 truncated」这三条必须留在**本模块**，
//! 不能寄望于底层库。三条防线：
//!
//!   1. 任一侧超过 `MAX_ALIGN_LINES` → **根本不进对齐引擎**，直接给有界摘要；
//!   2. hunk 数超过 `MAX_HUNKS` → 只输出前 N 个，其余只报数量；
//!   3. 摘要路径同样有界（`MAX_SUMMARY_SAMPLE`）：只给样本 + 未展示计数。
//!
//! 第 3 条是这次替换才补上的：旧实现的摘要路径会把**两侧全部行**原样吐出，
//! 所以"输出有界"当时只对 ≤`MAX_ALIGN_LINES` 的输入成立，
//! 而摘要路径恰恰只在**超过**该上限时才走 —— 等于最需要它的场景外没有保护。

use similar::udiff::UnifiedDiff;
use similar::TextDiff;

/// 参与行级对齐的最大行数（两侧各自）。超过则退化为摘要。
///
/// 它同时是**进不进对齐引擎**的开关：小于它才调 `similar`。
const MAX_ALIGN_LINES: usize = 2000;
/// 每个 hunk 保留的上下文行数。
const CONTEXT: usize = 3;
/// 最多输出多少个 hunk（避免超长 diff 撑爆终端与内存）。
const MAX_HUNKS: usize = 60;
/// 摘要路径每侧最多列出多少行样本（超出只报数量）。
const MAX_SUMMARY_SAMPLE: usize = 40;

/// 生成 unified diff。`truncated` 为真表示只给了摘要/节选，不是完整改动。
pub fn unified_diff(old: &str, new: &str, path: &str) -> (String, bool) {
    if old == new {
        return (String::new(), false);
    }

    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();

    // 防线 1：超大输入不进对齐引擎（similar 不会替我们拦）。
    if a.len() > MAX_ALIGN_LINES || b.len() > MAX_ALIGN_LINES {
        return (summary_diff(&a, &b, path), true);
    }

    // 行级对齐交给 similar：Myers/Patience，比自写 LCS 更快、块更贴合语义。
    // hunk 头由它按规范生成 —— 旧实现把新旧起始行号都写成同一个值，
    // 且把"改动行数"当成"hunk 宽度"，对解析方（TUI 的 diff 查看器按 hunk
    // 头定位与跳转）是错的。
    let diff = TextDiff::from_lines(old, new);
    let mut fmt = UnifiedDiff::from_text_diff(&diff);
    fmt.context_radius(CONTEXT)
        .header(&format!("a/{path}"), &format!("b/{path}"));

    // 防线 2：hunk 数封顶。先收集才能知道总数（要报"另有 N 处"）。
    let hunks: Vec<String> = fmt.iter_hunks().map(|h| h.to_string()).collect();
    let truncated = hunks.len() > MAX_HUNKS;

    let mut out = format!("--- a/{path}\n+++ b/{path}\n");
    for (i, h) in hunks.iter().enumerate() {
        if i >= MAX_HUNKS {
            out.push_str(&format!("… 另有 {} 处改动未展示\n", hunks.len() - MAX_HUNKS));
            break;
        }
        out.push_str(h);
    }
    if truncated {
        out.push_str("（改动过大，以上为节选）\n");
    }
    (out, truncated)
}

/// 超大文件：不做行级对齐，给**有界**的摘要（哪几行变了 + 样本）。
fn summary_diff(a: &[&str], b: &[&str], path: &str) -> String {
    let common = a.iter().zip(b.iter()).take_while(|(x, y)| x == y).count();
    let dels = &a[common..];
    let adds = &b[common..];

    let mut out = format!("--- a/{path}\n+++ b/{path}\n");
    // ⚠️ `@@` 行必须**干净**（只有 `-a,b +c,d`）。
    // 下游（TUI 的 diff 查看器）按空白切分这一行，把任何以 `+`/`-` 开头的
    // token 都当成行号 —— 曾把说明文字里的 "-7000 行 / +7000 行" 读成
    // 新侧起始行 7000。说明文字另起一行（与既有的"以上为节选"同一处理：
    // 解析器把不认识的行走上下文分支，不带行号）。
    out.push_str(&format!("@@ -1,{} +1,{} @@\n", common, common));
    out.push_str(&format!(
        "文件过大（>{MAX_ALIGN_LINES} 行），不做行级对齐：-{} 行 / +{} 行\n",
        dels.len(),
        adds.len()
    ));
    // 防线 3：样本有界 —— 旧实现在这里把两侧全部行都吐出来
    for s in dels.iter().take(MAX_SUMMARY_SAMPLE) {
        out.push('-');
        out.push_str(s);
        out.push('\n');
    }
    for s in adds.iter().take(MAX_SUMMARY_SAMPLE) {
        out.push('+');
        out.push_str(s);
        out.push('\n');
    }
    let hidden = dels.len().saturating_sub(MAX_SUMMARY_SAMPLE)
        + adds.len().saturating_sub(MAX_SUMMARY_SAMPLE);
    if hidden > 0 {
        out.push_str(&format!("… 另有 {hidden} 行未展示\n"));
    }
    out.push_str("（改动过大，以上为节选）\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_text_produces_no_diff() {
        let (d, t) = unified_diff("a\nb\n", "a\nb\n", "f.rs");
        assert!(d.is_empty(), "无改动不该输出 diff：{d}");
        assert!(!t);
    }

    #[test]
    fn a_single_changed_line_shows_as_del_plus_add() {
        let old = "one\ntwo\nthree\n";
        let new = "one\nTWO\nthree\n";
        let (d, t) = unified_diff(old, new, "f.rs");
        assert!(!t, "小改动不该被标为截断");
        assert!(d.contains("--- a/f.rs"), "应有文件头：{d}");
        assert!(d.contains("-two"), "应显示删除行：{d}");
        assert!(d.contains("+TWO"), "应显示新增行：{d}");
        assert!(d.contains(" one"), "应保留上下文行：{d}");
    }

    #[test]
    fn pure_addition_is_shown() {
        let (d, _) = unified_diff("a\n", "a\nb\n", "f.rs");
        assert!(d.contains("+b"), "{d}");
        assert!(!d.contains("-a"), "不该把已有行标为删除：{d}");
    }

    #[test]
    fn pure_deletion_is_shown() {
        let (d, _) = unified_diff("a\nb\n", "a\n", "f.rs");
        assert!(d.contains("-b"), "{d}");
    }

    #[test]
    fn distant_changes_are_split_into_separate_hunks() {
        // 相隔很远的改动应各自成 hunk，而不是把中间几百行全带上
        let mut old = String::new();
        let mut new = String::new();
        for i in 0..200 {
            old.push_str(&format!("l{i}\n"));
            new.push_str(&format!("l{i}\n"));
        }
        // 第 1 行与第 199 行各改一处
        let old = old.replace("l1\n", "OLD1\n").replace("l199\n", "OLD199\n");
        let (d, _) = unified_diff(&old, &new, "f.rs");
        // 按行数 hunk 头：每个头形如 "@@ -a,b +c,d @@"，
        // 直接数 "@@" 子串会把一个头算成两次
        let hunk_heads = d.lines().filter(|l| l.starts_with("@@")).count();
        assert_eq!(hunk_heads, 2, "应有两个 hunk：{d}");
        assert!(!d.contains("l100"), "无关的中间行不该被带上：{d}");
    }

    #[test]
    fn huge_files_degrade_to_a_summary_instead_of_blowing_memory() {
        // 内存有界：超过对齐上限时不再做 O(n×m) 的 LCS，而是给摘要
        let old: String = (0..(MAX_ALIGN_LINES + 50)).map(|i| format!("a{i}\n")).collect();
        let new: String = (0..(MAX_ALIGN_LINES + 50)).map(|i| format!("b{i}\n")).collect();
        let (d, truncated) = unified_diff(&old, &new, "big.txt");
        assert!(truncated, "超大改动必须如实标注截断");
        assert!(d.contains("节选"), "应说明只给了节选：{}", &d[..d.len().min(200)]);
    }

    #[test]
    fn output_is_bounded_for_pathological_input() {
        // 全文件重写：输出必须有界（不能把两侧内容都原样吐出来又叠加）
        let old: String = (0..1500).map(|i| format!("x{i}\n")).collect();
        let new: String = (0..1500).map(|i| format!("y{i}\n")).collect();
        let (d, _) = unified_diff(&old, &new, "f.txt");
        // 每侧 1500 行、每行 ≤ 8 字节 → 输出必有限（含两个 hunk 上限）
        assert!(d.len() < 64 * 1024, "输出应受 hunk 上限约束，实际 {} 字节", d.len());
    }

    #[test]
    fn empty_old_file_shows_all_as_additions() {
        // 新建文件的常见情形
        let (d, _) = unified_diff("", "line1\nline2\n", "new.txt");
        assert!(d.contains("+line1") && d.contains("+line2"), "{d}");
    }

    /// 解析出第一个 hunk 头里的四个数字：(旧起,旧数,新起,新数)。
    fn first_hunk_header(diff: &str) -> Option<(usize, usize, usize, usize)> {
        let line = diff.lines().find(|l| l.starts_with("@@"))?;
        let nums: Vec<usize> = line
            .split(|c: char| !c.is_ascii_digit())
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse().ok())
            .collect();
        (nums.len() >= 4).then(|| (nums[0], nums[1], nums[2], nums[3]))
    }

    /// hunk 头里的**新旧起始行号必须各自正确**。
    ///
    /// 这是回归测试：旧实现把两个起始行号都写成 `context_start(...)`（同一个值），
    /// 于是"之前有净增行"的改动里，新侧起始行号是错的。TUI 的 diff 查看器按
    /// hunk 头定位与跳转，这个错会直接变成跳错位置。
    #[test]
    fn hunk_header_tracks_old_and_new_line_numbers_separately() {
        // 早段插入一行，晚段（足够远，成另一个 hunk）改一行。
        // 于是末个 hunk 位于插入之后：新侧起始必须比旧侧**大 1**。
        // 旧实现把两侧起始都写成同一个值，这里会失败。
        let old: String = (1..=30).map(|i| format!("L{i}\n")).collect();
        let mut new = String::new();
        for i in 1..=30 {
            if i == 5 {
                new.push_str("INSERTED\n"); // 早段插入
            }
            if i == 25 {
                new.push_str("CHANGED\n"); // 晚段改动，替换原 L25
                continue;
            }
            new.push_str(&format!("L{i}\n"));
        }
        let (d, _) = unified_diff(&old, &new, "f.txt");
        let heads: Vec<&str> = d.lines().filter(|l| l.starts_with("@@")).collect();
        assert_eq!(heads.len(), 2, "应有两个相距较远的 hunk：{d}");

        let (os1, _, ns1, _) = first_hunk_header(heads[0]).expect("首个 hunk 头");
        assert_eq!(
            (os1, ns1),
            (2, 2),
            "首个 hunk 在插入之前，两侧起始一致（含 3 行上下文）：{}",
            heads[0]
        );

        let (os2, _, ns2, _) = first_hunk_header(heads[1]).expect("末个 hunk 头");
        assert_eq!(
            ns2,
            os2 + 1,
            "末个 hunk 在插入之后，新侧起始应比旧侧大 1：{}",
            heads[1]
        );
    }

    /// 超上限的输入，输出必须**有界**（旧实现在这里把两侧全部行都吐出来）。
    #[test]
    fn oversized_input_produces_bounded_output() {
        let n = MAX_ALIGN_LINES + 5_000;
        let old: String = (0..n).map(|i| format!("old line {i}\n")).collect();
        let new: String = (0..n).map(|i| format!("new line {i}\n")).collect();
        let (d, truncated) = unified_diff(&old, &new, "huge.txt");
        assert!(truncated, "超上限必须如实标注截断");
        assert!(d.contains("节选"), "应说明只给节选");
        // 有界性的硬断言：两侧各 7000 行，逐行吐出会是几百 KB
        assert!(
            d.len() < 16 * 1024,
            "摘要输出必须有界（样本上限），实际 {} 字节",
            d.len()
        );
        assert!(d.contains("未展示"), "被截掉的部分要报数量：{d}");
    }

    /// `@@` 行必须是**干净**的 `@@ -a,b +c,d @@`，不能带说明文字。
    ///
    /// 回归测试：下游解析器（`neo-host-tui` 的 diffview）按空白切这一行，
    /// 把任何以 `+`/`-` 开头的 token 当行号解析。曾把说明里的
    /// "-7000 行 / +7000 行" 读成新侧起始行 7000 —— 大文件 diff 的跳转会全错。
    #[test]
    fn hunk_header_lines_carry_no_prose_that_looks_like_line_numbers() {
        // 模拟下游：切分@@ 行的每个 token，凡以 +/- 开头的都必须形如 `数字[,数字]`
        fn assert_clean(header: &str, what: &str) {
            assert!(header.starts_with("@@ ") && header.ends_with(" @@"), "{what} 头格式不对：{header}");
            for tok in header.split_whitespace() {
                if let Some(rest) = tok.strip_prefix('-').or_else(|| tok.strip_prefix('+')) {
                    let ok = rest
                        .split(',')
                        .all(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
                    assert!(ok, "{what} 的 @@ 行含非行号 token {tok:?}：{header}");
                }
            }
        }

        // 正常路径（similar 生成的头）
        let (d, _) = unified_diff("a\nb\nc\n", "a\nX\nc\n", "f.txt");
        for h in d.lines().filter(|l| l.starts_with("@@")) {
            assert_clean(h, "正常");
        }

        // 摘要路径（我们手写的头）—— 这里曾是出问题的地方
        let n = MAX_ALIGN_LINES + 100;
        let old: String = (0..n).map(|i| format!("o{i}\n")).collect();
        let new: String = (0..n).map(|i| format!("p{i}\n")).collect();
        let (d2, truncated) = unified_diff(&old, &new, "huge.txt");
        assert!(truncated);
        let mut seen = 0;
        for h in d2.lines().filter(|l| l.starts_with("@@")) {
            assert_clean(h, "摘要");
            seen += 1;
        }
        assert_eq!(seen, 1, "摘要路径应有且仅有一个 hunk 头：{d2}");
    }

    /// 正常规模的输入不该被标为截断（宁可少给细节，但不能无故降级）。
    #[test]
    fn moderately_sized_input_is_not_truncated() {
        let old: String = (0..MAX_ALIGN_LINES - 10).map(|i| format!("l{i}\n")).collect();
        let mut new = old.clone();
        new = new.replace("l5\n", "L5-CHANGED\n");
        let (d, truncated) = unified_diff(&old, &new, "f.txt");
        assert!(!truncated, "未超上限不该标截断");
        assert!(d.contains("-l5") && d.contains("+L5-CHANGED"), "{d}");
    }
}
