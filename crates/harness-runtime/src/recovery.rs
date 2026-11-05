//! Restart reconciliation never deletes worktrees or kills unverified/reused PIDs.
use harness_core::domain::{RunId, TaskState};
use harness_store::{Store, StoreError, leases::LeaseRepository, repository::Repository};

/// Expired attempts have ambiguous side effects. Preserve their files and block
/// automatic re-execution until a human inspects them. Recorded commit replay is
/// handled by TaskCommitter/Integrator, which do not create a duplicate commit.
pub async fn reconcile(store: &Store, now: i64) -> Result<Vec<(RunId, String)>, StoreError> {
    let expired = store.reconcile_expired_leases(now).await?;
    let mut result = Vec::new();
    for lease in expired {
        let task = store.get_task(&lease.run_id, &lease.task_id).await?;
        if !task.state.is_terminal() && task.state != TaskState::Blocked {
            store
                .transition_task(
                    &lease.run_id,
                    &lease.task_id,
                    task.revision,
                    TaskState::Blocked,
                    Some(&format!("recovery:block:{}", lease.lease_id)),
                )
                .await?;
        }
        result.push((lease.run_id, lease.task_id.to_string()));
    }
    Ok(result)
}
