//! 目标编排的 L4 实现：把 [`GoalEngine`] 的四阶段状态机接到内核契据上。
//!
//! # 职责边界（与内核的分工）
//!
//! 内核提供机制：子任务轮就是**普通 turn**（沙箱/审批/上限/落盘全复用），
//! 轮次结束时把控制权交回编排器。这里提供策略：
//! - **拆解**：目标文本按行拆成子任务（用户写清单，引擎照单执行）。
//!   模型驱动的自动拆解是更大的一步，当前刻意不做 —— 它需要一次
//!   额外的模型往返来生成结构化输出，且拆错了比不拆更糟。
//! - **阶段提示词**：每个阶段给模型一句对应职责的指令（计划阶段
//!   明确禁止改文件 —— 阶段的意义就是约束行为）。
//! - **推进与判停**：一轮结束 = 引擎 step 一次。"不通过"有两路信号：
//!   硬失败（轮内 Error 事件 / 非零退出工具调用，内核单点记账，审批拒绝
//!   也算）+ **模型自评**（审查轮答复含显式否定词）。模型未表态不算
//!   失败 —— 离线桩与含糊模型不会被重试预算误烧。
//!
//! # 停止条件与确定性
//!
//! 生效：max_iterations、max_review_retries、max_consecutive_failures、
//! max_token_budget（内核报上每轮用量，这里累计）。
//! **不生效：max_wall_clock_secs** —— 挂钟会破坏回放确定性（同一 Op 序列
//! 必须产出同一事件序列），宁可少一条保护，也不能让回放对不上。
//!
//! # 全部状态都在快照里
//!
//! 快照缺一个字段，重启续跑就会做出与在线推进不同的决策
//! （consecutive_failures 就是为这个理由补进快照的）。

use neo_core::GoalOrchestrator;
use neo_protocol::{EventMsg, GoalId, GoalPhase, GoalSnapshot, GoalSubtask};

use crate::{GoalEngine, Phase, StepResult, StopConditions, Subtask};

/// 基于 [`GoalEngine`] 的编排器。
pub struct EngineGoalOrchestrator {
    /// 目标编号计数器（确定性派生，不用随机/时钟）
    goal_seq: usize,
    stop: StopConditions,
    state: Option<GoalState>,
}

struct GoalState {
    engine: GoalEngine,
    goal_text: String,
    paused: bool,
    stopped: Option<String>,
    budget_used: u64,
}

impl EngineGoalOrchestrator {
    pub fn new() -> Self {
        Self { goal_seq: 0, stop: StopConditions::default(), state: None }
    }

    pub fn with_stop_conditions(stop: StopConditions) -> Self {
        Self { goal_seq: 0, stop, state: None }
    }

    fn snapshot_event(&self) -> EventMsg {
        EventMsg::GoalUpdated { snapshot: self.snapshot_impl() }
    }

    fn snapshot_impl(&self) -> GoalSnapshot {
        let st = self.state.as_ref().expect("snapshot 仅在有状态时调用");
        let subtasks = st
            .engine
            .subtasks
            .iter()
            .map(|s| GoalSubtask {
                id: s.id,
                title: s.title.clone(),
                phase: to_proto_phase(s.phase),
                retries: s.retries,
            })
            .collect();
        GoalSnapshot {
            goal_id: st.engine.goal_id.clone(),
            goal: st.goal_text.clone(),
            paused: st.paused,
            stopped: st.stopped.clone(),
            subtasks,
            iterations: st.engine.iterations,
            consecutive_failures: st.engine.consecutive_failures,
            turns_remaining: self.turns_remaining(),
            budget_used: st.budget_used,
        }
    }

    /// 剩余子任务轮数的**下限**估计：每个未完成子任务按阶段阶梯还差几轮
    /// （Plan=4 / Code=3 / Review=2 / Learn=1）。审查失败的重试会让实际
    /// 轮数更多 —— 字段语义是下限，宿主只该用它判断"还有没有"。
    fn turns_remaining(&self) -> usize {
        let st = match self.state.as_ref() {
            Some(s) => s,
            None => return 0,
        };
        st.engine
            .subtasks
            .iter()
            .filter(|s| s.phase != Phase::Done)
            .map(|s| match s.phase {
                Phase::Plan => 4,
                Phase::Code => 3,
                Phase::Review => 2,
                Phase::Learn => 1,
                Phase::Done => 0,
            })
            .sum()
    }

    /// 当前应执行的子任务（第一个未完成的）。
    fn current(&self) -> Option<(usize, &Subtask)> {
        let st = self.state.as_ref()?;
        st.engine
            .subtasks
            .iter()
            .enumerate()
            .find(|(_, s)| s.phase != Phase::Done)
            .map(|(i, s)| (i, s))
    }
}

impl Default for EngineGoalOrchestrator {
    fn default() -> Self {
        Self::new()
    }
}

/// 阶段 → 协议枚举。穷尽匹配：两边各有新变体时编译器会拦。
fn to_proto_phase(p: Phase) -> GoalPhase {
    match p {
        Phase::Plan => GoalPhase::Plan,
        Phase::Code => GoalPhase::Code,
        Phase::Review => GoalPhase::Review,
        Phase::Learn => GoalPhase::Learn,
        Phase::Done => GoalPhase::Done,
    }
}

impl GoalOrchestrator for EngineGoalOrchestrator {
    fn goal_id(&self) -> Option<GoalId> {
        self.state.as_ref().map(|s| s.engine.goal_id.clone())
    }

    fn snapshot(&self) -> Option<GoalSnapshot> {
        // 有状态才调用内在 snapshot（它 panic 于 None）
        self.state.as_ref().map(|_| self.snapshot_impl())
    }

    fn set_goal(&mut self, goal: &str) -> Vec<EventMsg> {
        self.goal_seq += 1;
        let gid = format!("goal-{}", self.goal_seq);
        let titles: Vec<&str> =
            goal.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        // 空目标兜底成单子任务 —— 引擎需要至少一个可执行项
        let titles: Vec<&str> = if titles.is_empty() { vec![goal.trim()] } else { titles };
        let subtasks = titles
            .into_iter()
            .enumerate()
            .map(|(i, t)| Subtask {
                id: i + 1,
                title: t.to_string(),
                depends_on: Vec::new(),
                phase: Phase::Plan,
                retries: 0,
            })
            .collect();
        self.state = Some(GoalState {
            engine: GoalEngine {
                goal_id: gid,
                subtasks,
                stop: self.stop.clone(),
                iterations: 0,
                consecutive_failures: 0,
            },
            goal_text: goal.to_string(),
            paused: false,
            stopped: None,
            budget_used: 0,
        });
        vec![self.snapshot_event()]
    }

    fn pause(&mut self) -> Vec<EventMsg> {
        match self.state.as_mut() {
            Some(s) if !s.paused => {
                s.paused = true;
                vec![self.snapshot_event()]
            }
            _ => Vec::new(), // 重复暂停 = 无操作（幂等，不发重复事件）
        }
    }

    fn resume(&mut self) -> Vec<EventMsg> {
        match self.state.as_mut() {
            Some(s) if s.paused => {
                s.paused = false;
                vec![self.snapshot_event()]
            }
            _ => Vec::new(),
        }
    }

    fn clear(&mut self) -> Vec<EventMsg> {
        match self.state.take() {
            Some(s) => vec![EventMsg::GoalCleared { goal_id: s.engine.goal_id }],
            None => Vec::new(),
        }
    }

    fn has_pending_turn(&self) -> bool {
        match self.state.as_ref() {
            Some(s) => !s.paused && s.stopped.is_none() && s.engine.subtasks.iter().any(|s| s.phase != Phase::Done),
            None => false,
        }
    }

    fn next_turn_prompt(&mut self) -> Option<String> {
        let (_, st) = self.current()?;
        let gid = self.state.as_ref()?.engine.goal_id.clone();
        let n = self.state.as_ref()?.engine.subtasks.len();
        // 用子任务自己的 id（1 基），不是 vec 下标 —— 显示"子任务 0/2"
        // 会让人以为编号从 0 开始
        let header = format!("【目标 {gid} · 子任务 {}/{n}】", st.id);
        let prompt = match st.phase {
            Phase::Plan => format!(
                "{header}请为「{}」制定实施计划。本轮只规划，不要修改文件。",
                st.title
            ),
            Phase::Code => format!("{header}请执行「{}」。需要改文件时用 apply_patch。", st.title),
            Phase::Review => format!(
                "{header}请审查上一步是否正确完成了「{}」。发现问题就直接修复；\
                 确认无误后明确说明「{REVIEW_PASS_MARKER}」；有问题且无法修复时\
                 明确说明「审查未通过」及原因。",
                st.title
            ),
            Phase::Learn => format!(
                "{header}请用两三句话复盘「{}」的产出与可复用经验。",
                st.title
            ),
            // Done 子任务不会被选中（current 跳过它）
            Phase::Done => return None,
        };
        Some(prompt)
    }

    fn on_turn_complete(
        &mut self,
        usage: (u64, u64),
        failed: bool,
        review_text: &str,
    ) -> Vec<EventMsg> {
        if self.state.is_none() {
            return Vec::new();
        }
        {
            let s = self.state.as_mut().expect("上面已判 is_some");
            s.budget_used = s.budget_used.saturating_add(usage.0.saturating_add(usage.1));
            // 预算判停先于引擎步进：预算耗尽连当前阶段都不再推进
            if s.stopped.is_none() && s.budget_used > s.engine.stop.max_token_budget {
                s.stopped = Some("max_token_budget".into());
            }
            if s.stopped.is_none() {
                // 引擎 step 一次：失败 → 审查不过 → 回退重做。
                // 通过 = 无硬失败信号 **且** 模型没有显式否定（自评）。
                // 单一可变借用内完成取当前子任务 + 步进，不和 self.current()
                // 的不可变借用打架。
                let in_review = s.engine.subtasks.iter().any(|t| t.phase == Phase::Review);
                let review_rejected = in_review && explicit_rejection(review_text);
                let current_id = s
                    .engine
                    .subtasks
                    .iter()
                    .find(|t| t.phase != Phase::Done)
                    .map(|t| t.id);
                if let Some(id) = current_id {
                    let passed = !failed && !review_rejected;
                    if let StepResult::Stopped(reason) = s.engine.step(id, passed) {
                        s.stopped = Some(reason.to_string());
                    }
                }
            }
        }
        // 只要目标在，每轮结束都发快照 —— 推进、重试、判停都该让宿主看到
        vec![self.snapshot_event()]
    }

    fn observe(&mut self, event: &EventMsg) {
        match event {
            EventMsg::GoalUpdated { snapshot } => {
                self.goal_seq = snapshot
                    .goal_id
                    .strip_prefix("goal-")
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(self.goal_seq);
                let subtasks = snapshot
                    .subtasks
                    .iter()
                    .map(|s| Subtask {
                        id: s.id,
                        title: s.title.clone(),
                        depends_on: Vec::new(),
                        phase: from_proto_phase(s.phase),
                        retries: s.retries,
                    })
                    .collect();
                self.state = Some(GoalState {
                    engine: GoalEngine {
                        goal_id: snapshot.goal_id.clone(),
                        subtasks,
                        stop: self.stop.clone(),
                        iterations: snapshot.iterations,
                        consecutive_failures: snapshot.consecutive_failures,
                    },
                    goal_text: snapshot.goal.clone(),
                    paused: snapshot.paused,
                    stopped: snapshot.stopped.clone(),
                    budget_used: snapshot.budget_used,
                });
            }
            EventMsg::GoalCleared { .. } => self.state = None,
            _ => {}
        }
    }
}

/// 审查结论的显式**否定**词。与 Review 提示词成对 —— 提示词让模型用
/// 这些词表态，解析器只认这些词；两处在同一模块，改词汇必同改。
const REVIEW_REJECT_MARKERS: [&str; 3] = ["审查未通过", "审查不通过", "审查失败"];

/// 审查结论的显式**肯定**词（Review 提示词要求模型用它确认）。
const REVIEW_PASS_MARKER: &str = "审查通过";

/// 模型在审查轮里是否**显式否定**。
///
/// 判定语义（刻意的不对称）：
/// - 显式否定 → 不通过，回退 Code 重做（这是自评的价值：模型能叫停）；
/// - 显式肯定**或没有表态** → 结合硬失败信号判定。没有表态不判失败 ——
///   否则含糊的模型会把重试预算烧光，离线桩（固定答复）也会永远过不了审查。
fn explicit_rejection(review_text: &str) -> bool {
    REVIEW_REJECT_MARKERS.iter().any(|m| review_text.contains(m))
}

fn from_proto_phase(p: GoalPhase) -> Phase {
    match p {
        GoalPhase::Plan => Phase::Plan,
        GoalPhase::Code => Phase::Code,
        GoalPhase::Review => Phase::Review,
        GoalPhase::Learn => Phase::Learn,
        GoalPhase::Done => Phase::Done,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn orch() -> EngineGoalOrchestrator {
        EngineGoalOrchestrator::new()
    }

    /// 有状态时取快照（`GoalOrchestrator::snapshot` 返回 Option）。
    fn snap(o: &EngineGoalOrchestrator) -> GoalSnapshot {
        o.snapshot().expect("有活动目标时应有快照")
    }

    #[test]
    fn decomposes_goal_by_lines_into_subtasks() {
        let mut o = orch();
        let events = o.set_goal("实现登录\n写测试\n更新文档");
        assert_eq!(events.len(), 1, "设定目标发一条快照");
        let snap = match &events[0] {
            EventMsg::GoalUpdated { snapshot } => snapshot.clone(),
            other => panic!("应为快照事件：{other:?}"),
        };
        assert_eq!(snap.goal_id, "goal-1", "确定性编号");
        assert_eq!(snap.subtasks.len(), 3, "每个非空行一个子任务");
        assert!(snap.subtasks.iter().all(|s| s.phase == GoalPhase::Plan));
        assert_eq!(snap.turns_remaining, 12, "3 个子任务 × 4 阶段（下限估计）");
    }

    #[test]
    fn phases_advance_turn_by_turn_with_matching_prompts() {
        let mut o = orch();
        o.set_goal("写个Hello");
        // 计划轮：提示词说"只规划"，完成后进 Code
        let p1 = o.next_turn_prompt().unwrap();
        assert!(p1.contains("计划") && p1.contains("不要修改文件"), "{p1}");
        o.on_turn_complete((10, 5), false, "");
        // 执行轮
        let p2 = o.next_turn_prompt().unwrap();
        assert!(p2.contains("执行"), "{p2}");
        o.on_turn_complete((10, 5), false, "");
        // 审查轮
        let p3 = o.next_turn_prompt().unwrap();
        assert!(p3.contains("审查"), "{p3}");
        o.on_turn_complete((10, 5), false, "");
        // 复盘轮
        assert!(o.next_turn_prompt().unwrap().contains("复盘"));
        o.on_turn_complete((10, 5), false, "");
        // 单子任务全部完成
        assert!(!o.has_pending_turn(), "单子任务走完四阶段应无剩余");
        let snap = snap(&o);
        assert_eq!(snap.done_count(), 1);
        assert_eq!(snap.turns_remaining, 0);
    }

    #[test]
    fn failed_review_sends_the_subtask_back_to_code() {
        let mut o = orch();
        o.set_goal("做一件事");
        for _ in 0..2 {
            o.on_turn_complete((0, 0), false, ""); // Plan → Code → Review
        }
        // 审查轮出错（failed = true）→ 回退 Code
        o.on_turn_complete((0, 0), true, "");
        assert_eq!(snap(&o).subtasks[0].phase, GoalPhase::Code, "审查失败应回退到 Code");
        assert_eq!(snap(&o).subtasks[0].retries, 1);
        // 重做：Code → Review
        o.on_turn_complete((0, 0), false, "");
        assert_eq!(snap(&o).subtasks[0].phase, GoalPhase::Review);
    }

    #[test]
    fn stops_at_the_review_retry_limit() {
        let mut o = EngineGoalOrchestrator::with_stop_conditions(StopConditions {
            max_review_retries: 1,
            max_consecutive_failures: 99,
            ..StopConditions::default()
        });
        o.set_goal("做一件事");
        o.on_turn_complete((0, 0), false, ""); // Plan → Code
        o.on_turn_complete((0, 0), false, ""); // Code → Review
        o.on_turn_complete((0, 0), true, "");  // 审查失败 → retries 1 达上限
        let snap = snap(&o);
        assert_eq!(snap.stopped.as_deref(), Some("max_review_retries"), "应判停并上报原因");
        assert!(!o.has_pending_turn(), "停止后不得再推进");
    }

    #[test]
    fn stops_when_the_token_budget_is_exhausted() {
        let mut o = EngineGoalOrchestrator::with_stop_conditions(StopConditions {
            max_token_budget: 100,
            ..StopConditions::default()
        });
        o.set_goal("做一件事");
        let events = o.on_turn_complete((80, 30), false, ""); // 110 > 100
        let snap = match &events[0] {
            EventMsg::GoalUpdated { snapshot } => snapshot.clone(),
            other => panic!("{other:?}"),
        };
        assert_eq!(snap.stopped.as_deref(), Some("max_token_budget"));
        assert!(!o.has_pending_turn());
    }

    #[test]
    fn pause_resume_clear_are_idempotent_and_explicit() {
        let mut o = orch();
        o.set_goal("a\nb");
        assert!(o.has_pending_turn());
        let events = o.pause();
        assert!(matches!(events[0], EventMsg::GoalUpdated { .. }));
        assert!(!o.has_pending_turn(), "暂停后不得推进");
        assert!(o.pause().is_empty(), "重复暂停是空操作（不发重复事件）");
        o.resume();
        assert!(o.has_pending_turn());
        // 清除：发 GoalCleared，之后一切归零
        let events = o.clear();
        assert!(matches!(&events[0], EventMsg::GoalCleared { goal_id } if goal_id == "goal-1"));
        assert!(!o.has_pending_turn());
        assert!(o.goal_id().is_none());
        assert!(o.clear().is_empty(), "重复清除是空操作");
    }

    #[test]
    fn redirect_sets_a_new_goal_with_a_sequential_id() {
        let mut o = orch();
        o.set_goal("旧目标");
        let events = o.set_goal("新目标");
        assert!(matches!(&events[0], EventMsg::GoalUpdated { snapshot } if snapshot.goal_id == "goal-2"));
        assert_eq!(o.goal_id().as_deref(), Some("goal-2"));
    }

    #[test]
    fn model_explicit_rejection_triggers_rework() {
        // 自评的核心价值：模型能叫停。审查轮答复含显式否定 → 回退 Code。
        let mut o = orch();
        o.set_goal("做一件事");
        o.on_turn_complete((0, 0), false, ""); // Plan → Code
        o.on_turn_complete((0, 0), false, ""); // Code → Review
        // 审查轮：无硬失败，但模型明确否定
        o.on_turn_complete((0, 0), false, "实现有越界问题，审查未通过：越界原因……");
        assert_eq!(
            snap(&o).subtasks[0].phase,
            GoalPhase::Code,
            "显式否定必须回退重做"
        );
        assert_eq!(snap(&o).subtasks[0].retries, 1);
    }

    #[test]
    fn model_explicit_pass_and_silence_both_advance() {
        // 刻意的不对称：显式肯定推进；未表态也推进（不判失败）——
        // 否则离线桩（固定答复无结论词）会烧光重试预算。
        let mut o = orch();
        o.set_goal("做一件事");
        o.on_turn_complete((0, 0), false, "");                       // Plan → Code
        o.on_turn_complete((0, 0), false, "");                       // Code → Review
        o.on_turn_complete((0, 0), false, "细节核对无误，审查通过。"); // 显式肯定
        assert_eq!(snap(&o).subtasks[0].phase, GoalPhase::Learn);
        o.on_turn_complete((0, 0), false, "总结：完成。");            // 未表态 → 放行
        assert_eq!(snap(&o).subtasks[0].phase, GoalPhase::Done);
    }

    #[test]
    fn rejection_outside_the_review_phase_is_ignored() {
        // 否定词只在审查轮有效：Code 轮的正文提到"审查未通过"不该触发重试
        let mut o = orch();
        o.set_goal("做一件事");
        o.on_turn_complete((0, 0), false, "先声明：审查未通过这个词出现在这里只是引用。"); // Plan
        assert_eq!(snap(&o).subtasks[0].phase, GoalPhase::Code, "非审查轮的否定词应被忽略");
    }

    #[test]
    fn hard_failure_beats_an_explicit_pass() {
        // 工具崩了还"审查通过"—— 硬信号优先于模型的乐观结论
        let mut o = orch();
        o.set_goal("做一件事");
        o.on_turn_complete((0, 0), false, ""); // Plan → Code
        o.on_turn_complete((0, 0), false, ""); // Code → Review
        o.on_turn_complete((0, 0), true, "审查通过"); // 但轮内有硬失败
        assert_eq!(snap(&o).subtasks[0].phase, GoalPhase::Code, "硬失败必须压过显式肯定");
    }

    #[test]
    fn observe_replays_snapshots_into_identical_state() {
        // kill 进程后续跑的核心：新编排器消费同样的快照事件流，
        // 状态必须与在线推进完全一致。
        let mut live = orch();
        let mut history = Vec::new();
        history.extend(live.set_goal("任务一\n任务二"));
        history.extend(live.on_turn_complete((10, 5), false, "")); // Plan → Code
        history.extend(live.on_turn_complete((10, 5), false, "")); // Code → Review
        history.extend(live.pause());

        let mut fresh = orch();
        for ev in &history {
            fresh.observe(ev);
        }
        assert_eq!(fresh.goal_id(), live.goal_id());
        assert_eq!(fresh.has_pending_turn(), live.has_pending_turn());
        assert_eq!(fresh.snapshot(), live.snapshot(), "快照必须逐字段一致");
        assert_eq!(fresh.next_turn_prompt(), live.next_turn_prompt());
    }

    #[test]
    fn observe_clear_resets_to_no_goal() {
        let mut o = orch();
        o.set_goal("x");
        o.observe(&EventMsg::GoalCleared { goal_id: "goal-1".into() });
        assert!(o.goal_id().is_none());
        assert!(!o.has_pending_turn());
    }
}

#[cfg(test)]
mod smoke_probe {
    use super::*;

    /// 复现 pty 冒烟场景：双子任务应消耗恰好 8 轮。
    #[test]
    fn two_subtasks_consume_exactly_eight_turns() {
        let mut o = EngineGoalOrchestrator::new();
        let mut events = o.set_goal("写报告A\n写报告B");
        let mut prompts = Vec::new();
        while o.has_pending_turn() {
            let p = o.next_turn_prompt().expect("有待执行轮就应有提示词");
            prompts.push(p);
            events.extend(o.on_turn_complete((0, 0), false, ""));
        }
        assert_eq!(prompts.len(), 8, "2 子任务 × 4 阶段 = 8 轮，实际 {}", prompts.len());
        assert!(
            prompts.iter().any(|p| p.contains("子任务 1/2")) && prompts.iter().any(|p| p.contains("子任务 2/2")),
            "两个子任务都应有各自的轮：{prompts:?}"
        );
    }
}
