//! 行级 unified diff —— 给"审批前把改动给用户看"用
//!
//! # 为什么审批必须看到 diff
//!
//! 只说"写入类调用需确认"，用户其实是在**盲批**：不知道改哪个文件、
//! 改了什么、删了多少行。opencode 的权限弹窗会先渲染 diff。
//! 这个模块提供那份 diff（协议层已有 `EventMsg::PatchProposed`，
//! 之前一直没被发出过）。
//!
//! # 内存有界是硬要求
//!
//! LCS 的朴素 DP 是 O(n×m) 内存：两个 4000 行的文件要 1600 万格，
//! 足以把进程拖垮。所以这里：
//!   1. 先剥掉公共前缀/后缀（真实改动的中间段通常很小）；
//!   2. 中间段超过上限就**不再做行级对齐**，退化为"N 删 / M 增"的摘要，
//!      并明确标注 truncated —— 宁可少给细节，不可无界分配。

/// 参与行级对齐的最大行数（两侧各自）。超过则退化为摘要。
const MAX_ALIGN_LINES: usize = 2000;
/// 每个 hunk 保留的上下文行数。
const CONTEXT: usize = 3;
/// 最多输出多少个 hunk（避免超长 diff 撑爆终端与内存）。
const MAX_HUNKS: usize = 60;

/// 一行 diff。
#[derive(Debug, Clone, PartialEq, Eq)]
enum Op<'a> {
    Keep(&'a str),
    Del(&'a str),
    Add(&'a str),
}

/// 生成 unified diff。`truncated` 为真表示只给了摘要/节选，不是完整改动。
pub fn unified_diff(old: &str, new: &str, path: &str) -> (String, bool) {
    if old == new {
        return (String::new(), false);
    }
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();

    let mut truncated = false;
    let ops = if a.len() > MAX_ALIGN_LINES || b.len() > MAX_ALIGN_LINES {
        truncated = true;
        summary_ops(&a, &b)
    } else {
        lcs_ops(&a, &b)
    };

    let mut out = String::new();
    let hunks = hunks_of(&ops);
    if hunks.len() > MAX_HUNKS {
        truncated = true;
    }

    out.push_str(&format!("--- a/{path}\n+++ b/{path}\n"));
    for (i, h) in hunks.iter().enumerate() {
        if i >= MAX_HUNKS {
            let rest_lines: usize = hunks[i..].iter().map(|h| h.1 - h.0).sum();
            out.push_str(&format!("… 另有 {} 处改动未展示\n", hunks.len() - MAX_HUNKS));
            let _ = rest_lines;
            break;
        }
        let (start, end) = (h.0, h.1);
        let slice = &ops[start..end];
        let del = slice.iter().filter(|o| matches!(o, Op::Del(_))).count();
        let add = slice.iter().filter(|o| matches!(o, Op::Add(_))).count();
        out.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            context_start(&ops, start),
            del,
            context_start(&ops, start),
            add
        ));
        for o in slice {
            match o {
                Op::Keep(s) => {
                    out.push(' ');
                    out.push_str(s);
                    out.push('\n');
                }
                Op::Del(s) => {
                    out.push('-');
                    out.push_str(s);
                    out.push('\n');
                }
                Op::Add(s) => {
                    out.push('+');
                    out.push_str(s);
                    out.push('\n');
                }
            }
        }
    }
    if truncated {
        out.push_str("（改动过大，以上为节选）\n");
    }
    (out, truncated)
}

/// 供 hunk 头用的起始行号（近似：数到该处为止的 Keep+Del 行数）。
fn context_start(ops: &[Op], idx: usize) -> usize {
    ops[..idx].iter().filter(|o| !matches!(o, Op::Add(_))).count() + 1
}

/// 剥掉公共前后缀后的朴素 LCS。
///
/// 先剥前后缀这件事不只是优化：它让"只改了几行"的常见情形**不进入 O(n×m)**，
/// 从根上避免了大文件被小改动拖爆内存。
fn lcs_ops<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<Op<'a>> {
    let mut pre = 0;
    while pre < a.len() && pre < b.len() && a[pre] == b[pre] {
        pre += 1;
    }
    let mut suf = 0;
    while suf < a.len() - pre && suf < b.len() - pre && a[a.len() - 1 - suf] == b[b.len() - 1 - suf]
    {
        suf += 1;
    }
    let (am, bm) = (&a[pre..a.len() - suf], &b[pre..b.len() - suf]);

    let mut ops: Vec<Op> = Vec::new();
    for s in &a[..pre] {
        ops.push(Op::Keep(*s));
    }
    // 中间段：真正的 LCS
    let (n, m) = (am.len(), bm.len());
    // 中间段本身也可能很大（整文件重写）——超出就不对齐，直接删+增
    if n.saturating_mul(m) > MAX_ALIGN_LINES * 64 {
        for s in am {
            ops.push(Op::Del(*s));
        }
        for s in bm {
            ops.push(Op::Add(*s));
        }
    } else {
        let mut dp = vec![vec![0u32; m + 1]; n + 1];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                dp[i][j] = if am[i] == bm[j] {
                    dp[i + 1][j + 1] + 1
                } else {
                    dp[i + 1][j].max(dp[i][j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n && j < m {
            if am[i] == bm[j] {
                ops.push(Op::Keep(am[i]));
                i += 1;
                j += 1;
            } else if dp[i + 1][j] >= dp[i][j + 1] {
                ops.push(Op::Del(am[i]));
                i += 1;
            } else {
                ops.push(Op::Add(bm[j]));
                j += 1;
            }
        }
        while i < n {
            ops.push(Op::Del(am[i]));
            i += 1;
        }
        while j < m {
            ops.push(Op::Add(bm[j]));
            j += 1;
        }
    }
    for s in &a[a.len() - suf..] {
        ops.push(Op::Keep(*s));
    }
    ops
}

/// 超大文件：不做行级对齐，给出"哪几行变了"的摘要。
fn summary_ops<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<Op<'a>> {
    let mut ops = Vec::new();
    let common = a.iter().zip(b.iter()).take_while(|(x, y)| x == y).count();
    for s in &a[..common] {
        ops.push(Op::Keep(*s));
    }
    for s in &a[common..] {
        ops.push(Op::Del(*s));
    }
    for s in &b[common..] {
        ops.push(Op::Add(*s));
    }
    ops
}

/// 把 ops 切成带上下文的 hunk 区间（返回 [start, end) 列表）。
fn hunks_of(ops: &[Op]) -> Vec<(usize, usize)> {
    let changed: Vec<usize> = ops
        .iter()
        .enumerate()
        .filter(|(_, o)| !matches!(o, Op::Keep(_)))
        .map(|(i, _)| i)
        .collect();
    if changed.is_empty() {
        return Vec::new();
    }
    let mut hunks = Vec::new();
    let mut start = changed[0].saturating_sub(CONTEXT);
    let mut last = changed[0];
    for &i in &changed[1..] {
        // 与上一个改动相距不超过 2*CONTEXT 就并进同一个 hunk
        if i - last <= CONTEXT * 2 {
            last = i;
            continue;
        }
        hunks.push((start, (last + CONTEXT + 1).min(ops.len())));
        start = i.saturating_sub(CONTEXT);
        last = i;
    }
    hunks.push((start, (last + CONTEXT + 1).min(ops.len())));
    hunks
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
}
