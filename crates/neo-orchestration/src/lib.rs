//! L4 ORCHESTRATION —— Goal 引擎（吸收 ZCode Goal Mode）
//!
//! 四阶段闭环 Plan -> Code -> Review -> Learn（吸收自开源 zcode CLI）

use neo_protocol::GoalId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase { Plan, Code, Review, Learn, Done }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subtask {
    pub id: usize,
    pub title: String,
    pub depends_on: Vec<usize>,
    pub phase: Phase,
    pub retries: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub id: String,
    pub goal_id: GoalId,
    pub turn_index: usize,
    pub parent: Option<String>,
    pub pending: Vec<Subtask>,
}

/// 停止条件（防长程任务失控）
#[derive(Debug, Clone)]
pub struct StopConditions {
    pub max_iterations: usize,
    pub max_wall_clock_secs: u64,
    pub max_token_budget: u64,
    pub max_consecutive_failures: u32,
    pub max_review_retries: u32,
}

impl Default for StopConditions {
    fn default() -> Self {
        Self {
            max_iterations: 50,
            max_wall_clock_secs: 4 * 3600,
            max_token_budget: 500_000,
            max_consecutive_failures: 3,
            max_review_retries: 3,
        }
    }
}

pub struct GoalEngine {
    pub goal_id: GoalId,
    pub subtasks: Vec<Subtask>,
    pub stop: StopConditions,
    pub iterations: usize,
    pub consecutive_failures: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepResult { Continue, Completed, Stopped(&'static str) }

impl GoalEngine {
    /// 推进一个子任务。Review 失败 -> 回退到 Code；超过重试上限 -> 停止并上报。
    pub fn step(&mut self, subtask_id: usize, review_passed: bool) -> StepResult {
        self.iterations += 1;
        if self.iterations > self.stop.max_iterations {
            return StepResult::Stopped("max_iterations");
        }
        let Some(st) = self.subtasks.iter_mut().find(|s| s.id == subtask_id) else {
            return StepResult::Stopped("unknown_subtask");
        };
        match st.phase {
            Phase::Plan => { st.phase = Phase::Code; StepResult::Continue }
            Phase::Code => { st.phase = Phase::Review; StepResult::Continue }
            Phase::Review => {
                if review_passed {
                    st.phase = Phase::Learn;
                    self.consecutive_failures = 0;
                    StepResult::Continue
                } else {
                    self.consecutive_failures += 1;
                    st.retries += 1;
                    if st.retries >= self.stop.max_review_retries {
                        return StepResult::Stopped("max_review_retries");
                    }
                    if self.consecutive_failures >= self.stop.max_consecutive_failures {
                        return StepResult::Stopped("max_consecutive_failures");
                    }
                    st.phase = Phase::Code; // 回退重做
                    StepResult::Continue
                }
            }
            Phase::Learn => { st.phase = Phase::Done; StepResult::Continue }
            Phase::Done => StepResult::Completed,
        }
    }

    pub fn progress(&self) -> (usize, usize) {
        let total = self.subtasks.len();
        let done = self.subtasks.iter().filter(|s| s.phase == Phase::Done).count();
        (done, total)
    }

    pub fn checkpoint(&self, turn_index: usize, parent: Option<String>) -> Checkpoint {
        Checkpoint {
            id: format!("cp-{}-{}", self.goal_id, turn_index),
            goal_id: self.goal_id.clone(),
            turn_index,
            parent,
            pending: self.subtasks.iter().filter(|s| s.phase != Phase::Done).cloned().collect(),
        }
    }
}

// ══════════════════════════════════════════════════════════════════════
// 上下文压缩（Compact）—— L4 的职责：**决定压什么**，不负责怎么压
// ══════════════════════════════════════════════════════════════════════
//
// 内核报 `ContextBudgetExceeded` 时的解法是压缩，而不是丢弃 ——
// 但"丢哪几条"是个**策略**问题，属 L4；"怎么生成摘要、怎么落日志"是
// 内核的机制。这里只做策略：给定消息序列与预算，产出压缩计划。

/// 压缩计划：要摘要掉的部分 + 原样保留的部分。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionPlan {
    /// 将被摘要替换的消息（**按轮次整体**取出）
    pub summarize: Vec<String>,
    /// 原样保留的消息数（从这里开始保留到末尾）
    pub keep_from: usize,
    /// 被压缩的轮数（用于事件与日志）
    pub turns: usize,
}

/// 压缩策略参数。
#[derive(Debug, Clone, Copy)]
pub struct CompactionPolicy {
    /// 至少保留最近多少条消息（不被压缩）
    pub keep_recent: usize,
    /// **不值得压缩**的最小条数下限。
    ///
    /// 语义是"低于这个规模，压缩没有意义"（压不出多少、摘要反而占位），
    /// **不是**"自动压缩的触发线" —— 这里没有自动压缩：`Op::Compact` 是
    /// 显式请求（用户敲 `/compact`），此时应当**能压就压**。
    ///
    /// 曾经设成 64 并被我当成"自动触发阈值"，结果是 40 条消息的会话
    /// 敲 `/compact` 静默无操作 —— 用户明确要求压缩却被拒绝，是明显的 UX bug。
    pub trigger_at: usize,
}

impl Default for CompactionPolicy {
    fn default() -> Self {
        // 12 条保留 + 最小 8 条：交互式会话攒两三轮就足以压出空间。
        // 这个下限只为挡"完全没必要压"的极小会话，不该挡真实请求。
        Self { keep_recent: 12, trigger_at: 8 }
    }
}

/// 一条消息的"可摘要文本"（用于生成摘要）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summarizable {
    /// 角色（user / assistant / tool）
    pub role: &'static str,
    /// 文本内容
    pub text: String,
}

/// 计算压缩计划。
///
/// # 为什么必须按**轮次**整体压缩
///
/// 一轮 = 一条用户消息 + 其后的助手/工具消息。若只摘要掉"助手消息"却
/// 保留它的工具结果，请求里会出现**没有对应 tool_call 的 tool 消息** ——
/// OpenAI 规范下这是非法请求，真实模型会直接报 400。
/// 所以切点只能在用户消息处，不能在轮的中间。
///
/// # 参数
/// - `msgs`：`(角色, 文本)` 序列，顺序与内核一致
/// - `roles` 里 `"user"` 标识轮次起点
///
/// 返回 `None` 表示**不需要压缩**（未到阈值，或切不出安全的轮次边界）。
pub fn plan_compaction(
    msgs: &[Summarizable],
    policy: CompactionPolicy,
) -> Option<CompactionPlan> {
    if msgs.len() < policy.trigger_at {
        return None;
    }
    // 轮次起点：用户消息的下标
    let user_at: Vec<usize> = msgs
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == "user")
        .map(|(i, _)| i)
        .collect();
    // 至少要留下 keep_recent 条；切点必须在某个用户消息处，
    // 且切点之前至少有 1 轮可压（否则压了个寂寞）。
    let want_cut_before = msgs.len().saturating_sub(policy.keep_recent);
    let cut = user_at
        .iter()
        .copied()
        .filter(|i| *i <= want_cut_before && *i > 0)
        .max();
    let Some(cut) = cut else {
        // 切不出安全的边界（例如整个人类历史都不到一轮）
        return None;
    };
    let summarize: Vec<String> = msgs[..cut]
        .iter()
        .map(|m| format!("[{}] {}", m.role, m.text))
        .collect();
    // 被压掉的轮数 = 切点之前的用户消息条数
    let turns = msgs[..cut].iter().filter(|m| m.role == "user").count();
    Some(CompactionPlan { summarize, keep_from: cut, turns })
}

/// L4 的 `Compactor` 实现：把策略接到内核契据上。
///
/// 这就是"契据在 L2、实现在 L4"的落地点 —— 内核不知道压缩策略长什么样，
/// 只知道"给我摘要与保留点"。换策略（例如改成模型生成的摘要）只需换这个实现。
pub struct PolicyCompactor {
    pub policy: CompactionPolicy,
    /// 兜底摘要每条保留多少字符
    pub per_msg_chars: usize,
}

impl PolicyCompactor {
    pub fn new(policy: CompactionPolicy) -> Self {
        Self { policy, per_msg_chars: 200 }
    }
}

impl Default for PolicyCompactor {
    fn default() -> Self {
        Self::new(CompactionPolicy::default())
    }
}

impl neo_core::Compactor for PolicyCompactor {
    fn plan(&self, messages: &[neo_core::Message]) -> Option<(String, usize)> {
        // 把内核消息转成"可摘要"形态（只取文本；工具调用/结果的文本也带上，
        // 否则摘要会丢掉"当时做了什么"）
        let items: Vec<Summarizable> = messages
            .iter()
            .map(|m| match m {
                neo_core::Message::System(t) => Summarizable { role: "system", text: t.clone() },
                neo_core::Message::User(t) => Summarizable { role: "user", text: t.clone() },
                neo_core::Message::Assistant { text, tool_calls } => Summarizable {
                    role: "assistant",
                    text: if tool_calls.is_empty() {
                        text.clone()
                    } else {
                        format!(
                            "{text} [调用工具: {}]",
                            tool_calls.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", ")
                        )
                    },
                },
                neo_core::Message::ToolResult { name, output, .. } => Summarizable {
                    role: "tool",
                    text: format!("{name}: {}", output.stdout.lines().next().unwrap_or("")),
                },
            })
            .collect();
        let plan = plan_compaction(&items, self.policy)?;
        // 内核侧的下标必须**对齐原始 messages**，而 system 消息与 tool 角色
        // 在上面的映射里没有增删条目，故 keep_from 直接可用。
        let summary = deterministic_summary(&plan, self.per_msg_chars);
        Some((summary, plan.keep_from))
    }
}

/// 生成**确定性兜底摘要**（模型不可用/未配置时使用）。
///
/// 为什么不直接丢：丢掉的是"模型知道而摘要里没有"的信息，用户无从察觉。
/// 兜底摘要至少保留每条消息的开头，让后续对话仍能大致衔接。
/// **必须如实标注这是兜底摘要**（不是模型生成的），否则用户以为模型
/// 真的读过并总结过历史。
pub fn deterministic_summary(plan: &CompactionPlan, per_msg_chars: usize) -> String {
    let mut out = String::from("[上下文压缩 · 兜底摘要（非模型生成）]\n");
    for line in &plan.summarize {
        let cut: String = line.chars().take(per_msg_chars).collect();
        out.push_str(&cut);
        if line.chars().count() > per_msg_chars {
            out.push('…');
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(role: &'static str, text: &str) -> Summarizable {
        Summarizable { role, text: text.to_string() }
    }

    /// 造 n 轮对话：每轮 = 1 条 user + 1 条 assistant。
    fn turns(n: usize) -> Vec<Summarizable> {
        (0..n)
            .flat_map(|i| vec![m("user", &format!("问题{i}")), m("assistant", &format!("回答{i}"))])
            .collect()
    }

    #[test]
    fn nothing_to_do_below_the_minimum_worthwhile_size() {
        // 下限的意义是"太小了不值得压"，不是"自动触发线"
        let pol = CompactionPolicy { keep_recent: 4, trigger_at: 10 };
        assert!(plan_compaction(&turns(2), pol).is_none(), "太小不该压缩");
    }

    #[test]
    fn a_manual_compact_on_a_normal_session_actually_compacts() {
        // 40 条消息是正常交互式会话的规模，敲 /compact 必须真的压 ——
        // 曾经阈值设 64，这种会话会被静默拒绝（用户明确要求却无操作）。
        let msgs = turns(20); // 40 条
        let plan = plan_compaction(&msgs, CompactionPolicy::default())
            .expect("默认策略下 40 条消息应可压缩");
        assert!(plan.turns > 0);
        assert!(msgs.len() - plan.keep_from >= 12, "应保留至少 keep_recent 条");
    }

    #[test]
    fn cut_lands_on_a_turn_boundary_never_mid_turn() {
        // 关键契约：切点必须是**用户消息**处，否则会留下没有 tool_call 的
        // tool 消息 —— 真实 provider 会直接报 400。
        let msgs = turns(20); // 40 条
        let pol = CompactionPolicy { keep_recent: 10, trigger_at: 10 };
        let plan = plan_compaction(&msgs, pol).expect("应产出计划");
        assert_eq!(msgs[plan.keep_from].role, "user", "切点必须落在用户消息上");
        // 被压掉的部分也必须是完整的轮
        assert_eq!(plan.summarize.len(), plan.keep_from);
        let first_kept_is_user = msgs[plan.keep_from].role == "user";
        assert!(first_kept_is_user);
    }

    #[test]
    fn keeps_at_least_the_requested_recent_messages() {
        let msgs = turns(20);
        let pol = CompactionPolicy { keep_recent: 10, trigger_at: 10 };
        let plan = plan_compaction(&msgs, pol).unwrap();
        let kept = msgs.len() - plan.keep_from;
        assert!(kept >= 10, "至少保留 keep_recent 条，实际 {kept}");
    }

    #[test]
    fn counts_compacted_turns() {
        let msgs = turns(20);
        let pol = CompactionPolicy { keep_recent: 10, trigger_at: 10 };
        let plan = plan_compaction(&msgs, pol).unwrap();
        assert_eq!(plan.turns, plan.keep_from / 2, "每轮 2 条 → 轮数 = 切点/2");
        assert!(plan.turns > 0, "至少要压掉一轮，否则是空操作");
    }

    #[test]
    fn refuses_to_plan_when_no_safe_boundary_exists() {
        // 只有一条用户消息、后面全是很长的助手消息：切不出"完整的轮"，
        // 宁可返回 None（不压）也不要产出非法请求。
        let mut msgs = vec![m("user", "唯一的问题")];
        msgs.extend((0..50).map(|i| m("assistant", &format!("很长的回答{i}"))));
        let pol = CompactionPolicy { keep_recent: 5, trigger_at: 10 };
        assert!(
            plan_compaction(&msgs, pol).is_none(),
            "切点会在第 0 位（>0 才允许），故不该产出计划"
        );
    }

    #[test]
    fn never_summarizes_an_empty_prefix() {
        // cut 必须 > 0：压掉 0 条是无意义操作
        let msgs = turns(3);
        let pol = CompactionPolicy { keep_recent: 1, trigger_at: 2 };
        if let Some(plan) = plan_compaction(&msgs, pol) {
            assert!(plan.keep_from > 0, "切点不该是 0");
            assert!(!plan.summarize.is_empty());
        }
    }

    #[test]
    fn fallback_summary_is_labeled_and_bounded() {
        // 兜底摘要必须①标注"非模型生成"②每条截断，避免摘要本身撑爆上下文
        let msgs = turns(20);
        let pol = CompactionPolicy { keep_recent: 4, trigger_at: 4 };
        let plan = plan_compaction(&msgs, pol).unwrap();
        let sum = deterministic_summary(&plan, 4);
        assert!(sum.contains("非模型生成"), "必须如实标注来源：{sum}");
        // 每条被截到 4 字符（+可能一个省略号）
        for line in sum.lines().skip(1) {
            assert!(line.chars().count() <= 5, "每行应被截断：{line:?}");
        }
        assert!(sum.len() < 2000, "摘要本身要有界");
    }

    #[test]
    fn plan_is_deterministic() {
        // 同输入同输出（T2 可回放的前提）
        let msgs = turns(20);
        let pol = CompactionPolicy::default();
        let a = plan_compaction(&msgs, pol);
        let b = plan_compaction(&msgs, pol);
        assert_eq!(a, b);
    }

    /// 走一次"审查失败"的完整循环：Review --fail--> Code --step--> Review。
    ///
    /// **一次重试要两个 step** —— 失败后先回退到 Code，再走一步才回到 Review。
    /// 这一点是第一版测试写错的地方（我只调一次就期待它累积一次 retry）。
    fn fail_one_review(e: &mut GoalEngine, id: usize) {
        let r = e.step(id, false);
        assert!(matches!(r, StepResult::Continue), "回退到 Code 应是 Continue");
        let r = e.step(id, false); // Code -> Review
        assert!(matches!(r, StepResult::Continue));
    }

    #[test]
    fn goal_engine_stops_on_retry_limit() {
        // 保留既有 Goal 语义（防止本次改动碰坏）。
        // 把"连续失败上限"调高，专门验证"重试上限"这条路径 ——
        // 否则先触发的是另一条限制（第一版测试就混了这两条）。
        let mut e = GoalEngine {
            goal_id: "g".into(),
            subtasks: vec![Subtask {
                id: 1, title: "t".into(), depends_on: vec![],
                phase: Phase::Review, retries: 0,
            }],
            stop: StopConditions {
                max_review_retries: 2,
                max_consecutive_failures: 99,
                ..StopConditions::default()
            },
            iterations: 0,
            consecutive_failures: 0,
        };
        // 第一次失败：retries 1 → 回退重做
        fail_one_review(&mut e, 1);
        assert_eq!(e.consecutive_failures, 1);
        // 第二次失败：retries 2 → 达到上限
        let r = e.step(1, false);
        assert!(
            matches!(r, StepResult::Stopped("max_review_retries")),
            "retries 达上限应停止，实际 {r:?}"
        );
    }

    #[test]
    fn goal_engine_stops_on_consecutive_failures() {
        // 另一条限制：连续失败达上限即停（比重试上限更早保护）。
        // 把重试上限调高，专门验证这条。
        let mut e = GoalEngine {
            goal_id: "g".into(),
            subtasks: vec![Subtask {
                id: 1, title: "t".into(), depends_on: vec![],
                phase: Phase::Review, retries: 0,
            }],
            stop: StopConditions {
                max_review_retries: 99,
                max_consecutive_failures: 3,
                ..StopConditions::default()
            },
            iterations: 0,
            consecutive_failures: 0,
        };
        fail_one_review(&mut e, 1);
        assert_eq!(e.consecutive_failures, 1);
        fail_one_review(&mut e, 1);
        assert_eq!(e.consecutive_failures, 2);
        let r = e.step(1, false);
        assert!(
            matches!(r, StepResult::Stopped("max_consecutive_failures")),
            "连续失败达 3 次应停止，实际 {r:?}"
        );
    }

    #[test]
    fn a_successful_review_resets_the_failure_counter() {
        // 成功一次就清零：否则偶发失败会累积到上限，把正常任务误停
        let mut e = GoalEngine {
            goal_id: "g".into(),
            subtasks: vec![Subtask {
                id: 1, title: "t".into(), depends_on: vec![],
                phase: Phase::Review, retries: 0,
            }],
            stop: StopConditions { max_consecutive_failures: 3, ..StopConditions::default() },
            iterations: 0,
            consecutive_failures: 0,
        };
        fail_one_review(&mut e, 1);
        assert_eq!(e.consecutive_failures, 1, "失败一次后计数为 1");
        // 现在处于 Review：通过 → Learn 且计数清零
        assert!(matches!(e.step(1, true), StepResult::Continue));
        assert_eq!(e.consecutive_failures, 0, "通过审查应清零失败计数");
    }

    #[test]
    fn goal_progress_counts_done() {
        let e = GoalEngine {
            goal_id: "g".into(),
            subtasks: vec![
                Subtask { id: 1, title: "a".into(), depends_on: vec![], phase: Phase::Done, retries: 0 },
                Subtask { id: 2, title: "b".into(), depends_on: vec![], phase: Phase::Code, retries: 0 },
            ],
            stop: StopConditions::default(),
            iterations: 0,
            consecutive_failures: 0,
        };
        assert_eq!(e.progress(), (1, 2));
        let cp = e.checkpoint(3, None);
        assert_eq!(cp.pending.len(), 1, "checkpoint 只含未完成项");
    }
}
