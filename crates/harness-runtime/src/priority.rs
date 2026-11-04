//! Priority ranking of ready tasks. A manual priority always overrides the agent's.

use harness_core::domain::{Task, TaskId};

/// Manual priority if set, otherwise the agent-proposed one.
pub fn effective_priority(task: &Task) -> u8 {
    task.spec
        .manual_priority
        .unwrap_or(task.spec.agent_priority)
        .get()
}

/// Order `ready` by effective priority (highest first). Ties keep the order of `tasks`
/// (creation order), which gives FIFO behaviour. Unknown ids are dropped.
pub fn rank(tasks: &[Task], ready: &[TaskId]) -> Vec<TaskId> {
    let mut picked: Vec<(usize, u8, &TaskId)> = tasks
        .iter()
        .enumerate()
        .filter(|(_, t)| ready.contains(t.id()))
        .map(|(i, t)| (i, effective_priority(t), t.id()))
        .collect();
    picked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    picked.into_iter().map(|(_, _, id)| id.clone()).collect()
}
