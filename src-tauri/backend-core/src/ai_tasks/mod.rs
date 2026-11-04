//! AI tasks: requests too long for one turn. The agent creates a task (only
//! the agent, from a chat of the app or Telegram), and Notia works it out of
//! the turn in three phases, each a run of the agent with its own prompt
//! file of `.agent/promps`:
//!
//! 1. the **planner** turns the goal into steps;
//! 2. the **executor** runs once per step until every step is done;
//! 3. the **review** checks the work and approves it, or asks for more
//!    steps, which the executor runs before another review.
//!
//! A step that depends on something that has not happened yet (a reply by
//! mail, a date) leaves the task waiting until it happens (`waits`).
//!
//! This module holds the model, what comes next, how the answers of the
//! planner and the review are read, the requests of each phase, the
//! workflow drawn as Mermaid and the agent's tools. The adapter
//! (`app/src/ai_task_runner.rs`) stores the tasks in the library's SQLite
//! database for a week and runs them.

pub mod prompts;
pub mod tools;
pub mod views;
pub mod waits;
pub mod workflow;

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};

pub use waits::AiTaskWait;

/// Default prompt files of the three phases, written to `.agent/promps`
/// when missing; the person may edit them.
pub const PLANNER_PROMPT: &str = include_str!("../defaults/agent_prompt_planner.md");
pub const EXECUTOR_PROMPT: &str = include_str!("../defaults/agent_prompt_executor.md");
pub const REVIEW_PROMPT: &str = include_str!("../defaults/agent_prompt_review.md");

/// A task is kept a week after it was created, then deleted.
pub const RETENTION_MS: i64 = 7 * 24 * 60 * 60 * 1000;
pub const MAX_STEPS: usize = 30;
/// Reviews that ask to rework before the task stops for the person.
pub const MAX_REWORK_REVIEWS: usize = 5;
pub const MAX_TITLE_CHARS: usize = 120;
pub const MAX_GOAL_CHARS: usize = 12_000;
pub const MAX_STEP_TITLE_CHARS: usize = 200;
pub const MAX_STEP_DETAIL_CHARS: usize = 2_000;
/// Characters kept of a step's report and of the result.
pub const MAX_REPORT_CHARS: usize = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AiTaskStatus {
    Planning,
    Running,
    Reviewing,
    AwaitingApproval,
    /// A step waits for something to happen (`waits`); no runner holds it.
    Waiting,
    Completed,
    Failed,
    Cancelled,
    /// Notia closed while the task worked; it can be resumed.
    Interrupted,
}

impl AiTaskStatus {
    pub fn id(self) -> &'static str {
        match self {
            Self::Planning => "planning",
            Self::Running => "running",
            Self::Reviewing => "reviewing",
            Self::AwaitingApproval => "awaiting-approval",
            Self::Waiting => "waiting",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        [
            Self::Planning,
            Self::Running,
            Self::Reviewing,
            Self::AwaitingApproval,
            Self::Waiting,
            Self::Completed,
            Self::Failed,
            Self::Cancelled,
            Self::Interrupted,
        ]
        .into_iter()
        .find(|status| status.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Planning => "Planificando",
            Self::Running => "Ejecutando",
            Self::Reviewing => "En revisión",
            Self::AwaitingApproval => "Esperando aprobación",
            Self::Waiting => "Esperando",
            Self::Completed => "Completada",
            Self::Failed => "Falló",
            Self::Cancelled => "Cancelada",
            Self::Interrupted => "Interrumpida",
        }
    }

    /// It has not ended: a runner works on it, or it waits.
    pub fn is_active(self) -> bool {
        matches!(self, Self::Planning | Self::Running | Self::Reviewing | Self::AwaitingApproval | Self::Waiting)
    }

    /// It stopped before ending well and can go on from where it was, or it
    /// waits and can go on without waiting more.
    pub fn is_resumable(self) -> bool {
        matches!(self, Self::Failed | Self::Cancelled | Self::Interrupted | Self::Waiting)
    }
}

/// The three agents of a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AiTaskPhase {
    Planner,
    Executor,
    Review,
}

impl AiTaskPhase {
    pub const ALL: [AiTaskPhase; 3] = [AiTaskPhase::Planner, AiTaskPhase::Executor, AiTaskPhase::Review];

    /// Its prompt file in `.agent/promps`.
    pub fn prompt_file(self) -> &'static str {
        match self {
            Self::Planner => "planificador.md",
            Self::Executor => "ejecutor.md",
            Self::Review => "review.md",
        }
    }

    pub fn default_prompt(self) -> &'static str {
        match self {
            Self::Planner => PLANNER_PROMPT,
            Self::Executor => EXECUTOR_PROMPT,
            Self::Review => REVIEW_PROMPT,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Planner => "Planificador",
            Self::Executor => "Ejecutor",
            Self::Review => "Review",
        }
    }

    /// The phase may change data: the planner and the review only read.
    pub fn writes(self) -> bool {
        self == Self::Executor
    }
}

/// Where the agent created the task: approvals and the result go back there
/// (Telegram), or stay in the panel (the app).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AiTaskOrigin {
    App,
    Telegram,
}

impl AiTaskOrigin {
    pub fn label(self) -> &'static str {
        match self {
            Self::App => "Notia",
            Self::Telegram => "Telegram",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StepStatus {
    Pending,
    Running,
    /// It waits for something to happen before it goes on.
    Waiting,
    Done,
    Failed,
}

impl StepStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => "Pendiente",
            Self::Running => "En curso",
            Self::Waiting => "Esperando",
            Self::Done => "Hecho",
            Self::Failed => "Falló",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiTaskStep {
    /// `s1`, `s2`… in the order they were planned.
    pub id: String,
    /// 0 for the first plan; each review that asks to rework adds steps of
    /// the next round.
    pub round: u32,
    pub title: String,
    pub detail: String,
    pub status: StepStatus,
    #[serde(default)]
    pub report: Option<String>,
    /// What the executor did before each wait of the step, and how each
    /// wait ended.
    #[serde(default)]
    pub notes: Vec<String>,
    #[serde(default)]
    pub started_at_ms: Option<i64>,
    #[serde(default)]
    pub finished_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiTaskReview {
    /// The round of steps it checked.
    pub round: u32,
    pub approved: bool,
    pub feedback: String,
    pub at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalKind {
    Confirmation,
    Clarification,
    Plan,
}

/// What a phase waits for from the person: a change to confirm, a question
/// to answer or a plan to approve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiTaskApproval {
    pub id: String,
    pub phase: AiTaskPhase,
    #[serde(default)]
    pub step_id: Option<String>,
    pub kind: ApprovalKind,
    pub title: String,
    /// The change (its preview) or the plan, as text.
    #[serde(default)]
    pub detail: String,
    /// The choices of a question.
    #[serde(default)]
    pub options: Vec<String>,
    pub created_at_ms: i64,
    /// The Telegram message that asked, to mark it answered.
    #[serde(default)]
    pub telegram_message_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiTask {
    pub id: String,
    pub title: String,
    pub goal: String,
    pub origin: AiTaskOrigin,
    /// The Owner's Telegram chat of a task created there.
    #[serde(default)]
    pub telegram_chat_id: Option<i64>,
    /// The person chose «Confirmar todos» when the task was created: its
    /// changes are applied without asking (questions are still asked).
    pub approve_all: bool,
    pub status: AiTaskStatus,
    /// The phase that works now.
    #[serde(default)]
    pub phase: Option<AiTaskPhase>,
    #[serde(default)]
    pub steps: Vec<AiTaskStep>,
    /// Its goal changed (`resume_ai_task` with `goal`): the planner plans
    /// what is still missing after the steps already done.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub replan: bool,
    #[serde(default)]
    pub reviews: Vec<AiTaskReview>,
    /// What the agent asked a new review to check (`review_ai_task`).
    #[serde(default)]
    pub review_request: Option<String>,
    #[serde(default)]
    pub pending: Option<AiTaskApproval>,
    /// What a step waits for, while the task waits or after the executor
    /// asked for it in the run that works now.
    #[serde(default)]
    pub wait: Option<AiTaskWait>,
    /// How the last wait ended, for the executor's next run of the step.
    #[serde(default)]
    pub wake: Option<String>,
    #[serde(default)]
    pub result: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    #[serde(default)]
    pub finished_at_ms: Option<i64>,
}

/// What the runner does next with a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NextWork {
    Plan,
    /// The index of the first step not done.
    Execute(usize),
    Review,
    Done,
}

impl AiTask {
    /// A new task, waiting for its plan.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: String,
        title: &str,
        goal: &str,
        origin: AiTaskOrigin,
        telegram_chat_id: Option<i64>,
        approve_all: bool,
        now_ms: i64,
    ) -> Self {
        Self {
            id,
            title: clipped(title, MAX_TITLE_CHARS),
            goal: goal.trim().to_string(),
            origin,
            telegram_chat_id,
            approve_all,
            status: AiTaskStatus::Planning,
            phase: Some(AiTaskPhase::Planner),
            steps: Vec::new(),
            replan: false,
            reviews: Vec::new(),
            review_request: None,
            pending: None,
            wait: None,
            wake: None,
            result: None,
            error: None,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            finished_at_ms: None,
        }
    }

    /// The round of the newest steps.
    pub fn current_round(&self) -> u32 {
        self.steps.iter().map(|step| step.round).max().unwrap_or(0)
    }

    pub fn next_work(&self) -> NextWork {
        if self.steps.is_empty() || self.replan {
            return NextWork::Plan;
        }
        if let Some(index) = self.steps.iter().position(|step| step.status != StepStatus::Done) {
            return NextWork::Execute(index);
        }
        let round = self.current_round();
        let approved = self.reviews.iter().any(|review| review.round == round && review.approved);
        if approved && self.review_request.is_none() {
            NextWork::Done
        } else {
            NextWork::Review
        }
    }

    /// Steps done, and all of them.
    pub fn progress(&self) -> (usize, usize) {
        (self.steps.iter().filter(|step| step.status == StepStatus::Done).count(), self.steps.len())
    }

    /// The plan the planner returned, as the steps of the first round. A
    /// task planned again after its goal changed keeps the steps already
    /// done and adds the new ones after them, in a new round (so a review
    /// of the old goal does not approve them).
    pub fn set_plan(&mut self, planned: Vec<PlannedStep>) {
        if !self.replan {
            self.steps = numbered_steps(planned, 0, 0);
            return;
        }
        self.replan = false;
        let start = self.steps.iter().filter_map(|step| step.id.strip_prefix('s')?.parse::<usize>().ok()).max().unwrap_or(0);
        let round = self.current_round() + 1;
        self.steps.extend(numbered_steps(planned, round, start));
    }

    /// Points the task at a new `goal` (when given) and plans it again,
    /// from the person's correction: the steps already done stay, with their
    /// reports, and the planner adds only what is missing. `restart` also
    /// forgets the steps and reviews, so the task starts over. Whatever the
    /// old plan waited for or asked is dropped.
    pub fn replan(&mut self, goal: Option<&str>, restart: bool) {
        if let Some(goal) = goal {
            self.goal = goal.trim().to_string();
        }
        if restart {
            self.steps.clear();
            self.reviews.clear();
        } else {
            self.steps.retain(|step| step.status == StepStatus::Done);
        }
        self.replan = !self.steps.is_empty();
        self.status = AiTaskStatus::Planning;
        self.phase = Some(AiTaskPhase::Planner);
        self.review_request = None;
        self.pending = None;
        self.wait = None;
        self.wake = None;
        self.result = None;
        self.error = None;
        self.finished_at_ms = None;
    }

    /// Ends the review of the current round. An approval completes the task
    /// with its result; a rework adds the steps it asks for (or one that
    /// asks to fix what it said), until `MAX_REWORK_REVIEWS`.
    pub fn apply_review(&mut self, outcome: ReviewOutcome, now_ms: i64) -> ReviewEffect {
        let round = self.current_round();
        self.review_request = None;
        self.reviews.push(AiTaskReview { round, approved: outcome.approved, feedback: outcome.feedback.clone(), at_ms: now_ms });
        if outcome.approved {
            let result = Some(outcome.result).filter(|result| !result.trim().is_empty()).unwrap_or(outcome.feedback);
            self.result = Some(clipped_report(&result));
            return ReviewEffect::Approved;
        }
        let reworks = self.reviews.iter().filter(|review| !review.approved).count();
        if reworks >= MAX_REWORK_REVIEWS {
            return ReviewEffect::Exhausted;
        }
        let steps = if outcome.steps.is_empty() {
            vec![PlannedStep {
                title: "Corregir lo que pidió la revisión".to_string(),
                detail: clipped(&outcome.feedback, MAX_STEP_DETAIL_CHARS),
            }]
        } else {
            outcome.steps
        };
        let start = self.steps.len();
        self.steps.extend(numbered_steps(steps, round + 1, start));
        ReviewEffect::Rework
    }
}

/// What a review did to the task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewEffect {
    Approved,
    Rework,
    /// It asked to rework too many times: the task stops for the person.
    Exhausted,
}

/// A step as the planner or the review wrote it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedStep {
    pub title: String,
    pub detail: String,
}

/// What a review answered.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReviewOutcome {
    pub approved: bool,
    pub feedback: String,
    /// The steps to rework.
    pub steps: Vec<PlannedStep>,
    /// The result for the person, when it approves.
    pub result: String,
}

/// Identifies a phase's run in the agent request: the runtime gives it the
/// phase's tools and guidance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiTaskTurn {
    pub task_id: String,
    pub phase: AiTaskPhase,
}

fn numbered_steps(planned: Vec<PlannedStep>, round: u32, start: usize) -> Vec<AiTaskStep> {
    planned
        .into_iter()
        .take(MAX_STEPS.saturating_sub(start).max(1))
        .enumerate()
        .map(|(index, step)| AiTaskStep {
            id: format!("s{}", start + index + 1),
            round,
            title: clipped(&step.title, MAX_STEP_TITLE_CHARS),
            detail: clipped(&step.detail, MAX_STEP_DETAIL_CHARS),
            status: StepStatus::Pending,
            report: None,
            notes: Vec::new(),
            started_at_ms: None,
            finished_at_ms: None,
        })
        .collect()
}

/// `value` trimmed to `max` characters, with an ellipsis when cut.
pub fn clipped(value: &str, max: usize) -> String {
    let value = value.trim();
    if value.chars().count() <= max {
        return value.to_string();
    }
    format!("{}…", value.chars().take(max.saturating_sub(1)).collect::<String>().trim_end())
}

/// A step's report or the result, within `MAX_REPORT_CHARS`.
pub fn clipped_report(value: &str) -> String {
    clipped(value, MAX_REPORT_CHARS)
}
