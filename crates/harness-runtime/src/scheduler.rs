//! Persisted scheduler tick: advance dependency-ready tasks without dispatching workers.

use harness_core::domain::{RunId, RunState, TaskId, TaskState};
use harness_store::repository::Repository;
use harness_store::{Store, StoreError};

use crate::readiness::{RunGate, TaskView, ready_tasks};

/// Move all currently eligible tasks to `ready`. Dispatch remains the responsibility of a
/// later worker scheduler; this tick only persists dependency readiness.
pub async fn tick(store: &Store, run_id: &RunId, now: i64) -> Result<Vec<TaskId>, StoreError> {
    let run = store.get_run(run_id).await?;
    let tasks = store.list_tasks(run_id).await?;
    let views: Vec<TaskView> = tasks
        .iter()
        .map(|task| TaskView {
            id: task.id().clone(),
            state: task.state,
            dependencies: task.spec.dependencies.clone(),
            retry_not_before: None,
        })
        .collect();
    let ready = ready_tasks(
        RunGate {
            state: run.state,
            plan_approved: run.state == RunState::Running,
        },
        &views,
        now,
    );
    for id in &ready {
        let task = tasks
            .iter()
            .find(|task| task.id() == id)
            .expect("ready task belongs to the task snapshot");
        if task.state == TaskState::Pending {
            let result = store
                .transition_task(
                    run_id,
                    id,
                    task.revision,
                    TaskState::Ready,
                    Some(&format!("scheduler:ready:{run_id}:{id}")),
                )
                .await;
            if let Err(error) = result
                && !matches!(error, StoreError::RevisionConflict { .. })
            {
                return Err(error);
            }
        }
    }
    Ok(ready)
}
