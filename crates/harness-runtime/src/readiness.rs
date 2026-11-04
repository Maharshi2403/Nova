//! Dependency-aware readiness evaluation. Pure: no clock, no I/O, no processes.

use std::collections::HashMap;

use harness_core::domain::{RunState, TaskId, TaskState};

/// Snapshot of one task as seen by the scheduler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskView {
    pub id: TaskId,
    pub state: TaskState,
    pub dependencies: Vec<TaskId>,
    /// Unix seconds before which a `retry_wait` task must not run.
    pub retry_not_before: Option<i64>,
}

/// Run-level gates that apply to every task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunGate {
    pub state: RunState,
    /// Placeholder for the plan-approval flow: unapproved plans never dispatch.
    pub plan_approved: bool,
}

/// Tasks that may be moved to / dispatched from `ready` right now, in input order.
///
/// A task qualifies when the run is `running` and approved, the task is `pending`,
/// `ready` or a `retry_wait` whose retry time has arrived, and every dependency exists
/// and is `completed`. Anything referencing an unknown dependency is never ready.
pub fn ready_tasks(gate: RunGate, tasks: &[TaskView], now: i64) -> Vec<TaskId> {
    if gate.state != RunState::Running || !gate.plan_approved {
        return Vec::new();
    }
    let states: HashMap<&TaskId, TaskState> = tasks.iter().map(|t| (&t.id, t.state)).collect();
    tasks
        .iter()
        .filter(|t| eligible_state(t, now))
        .filter(|t| {
            t.dependencies
                .iter()
                .all(|d| states.get(d) == Some(&TaskState::Completed))
        })
        .map(|t| t.id.clone())
        .collect()
}

fn eligible_state(t: &TaskView, now: i64) -> bool {
    match t.state {
        TaskState::Pending | TaskState::Ready => true,
        TaskState::RetryWait => t.retry_not_before.is_none_or(|at| at <= now),
        _ => false,
    }
}
