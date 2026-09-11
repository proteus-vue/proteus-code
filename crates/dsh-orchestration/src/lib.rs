//! L4 ORCHESTRATION —— Goal 引擎（吸收 ZCode Goal Mode）
//!
//! 四阶段闭环 Plan -> Code -> Review -> Learn（吸收自开源 zcode CLI）

use dsh_protocol::GoalId;
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
