//! Budget-gated worker startup. No child process exists until the durable reservation.
use crate::worker::{Worker, WorkerError, WorkerSpec};
use harness_core::domain::{RunId, TaskId};
use harness_store::budgets::BudgetRequest;
use harness_store::leases::LeaseRequest;
use harness_store::{Store, StoreError};

#[derive(Debug, thiserror::Error)]
pub enum DispatchError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Worker(#[from] WorkerError),
}

/// Launch an attempt only after reserving budget. A spawn failure atomically releases
/// both lease and reservation; once a process exists its reservation is kept conservatively.
/// Handshake/runtime failures may have incurred spend and require later usage settlement.
pub async fn dispatch(
    store: &Store,
    run: &RunId,
    task: &TaskId,
    revision: u64,
    lease: &LeaseRequest<'_>,
    budget: BudgetRequest,
    spec: &WorkerSpec,
) -> Result<Worker, DispatchError> {
    store
        .grant_budgeted_lease(run, task, revision, lease, budget)
        .await?;
    match Worker::spawn(spec) {
        Ok(worker) => Ok(worker),
        Err(error) => {
            store.release_failed_dispatch_budget(lease.lease_id).await?;
            Err(error.into())
        }
    }
}
