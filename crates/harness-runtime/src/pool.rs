//! Bounded single-daemon dispatch. No process is created before path and spend claims.
use crate::{locks::ResourceLocks, priority::rank, scheduler::tick};
use harness_core::domain::{RunId, Task, TaskState};
use harness_store::budgets::BudgetRequest;
use harness_store::leases::{LeaseRepository, LeaseRequest};
use harness_store::repository::Repository;
use harness_store::{Store, StoreError};
use std::collections::HashMap;
use std::future::Future;
use tokio::sync::watch;
use tokio::task::JoinSet;
use uuid::Uuid;

pub struct WorkerPool {
    capacity: usize,
    locks: ResourceLocks,
    jobs: JoinSet<Result<(), String>>,
    cancellations: HashMap<String, (RunId, watch::Sender<bool>)>,
}

impl WorkerPool {
    pub fn new(
        concurrency: usize,
        provider_capacity: usize,
        infrastructure_capacity: usize,
        locks: ResourceLocks,
    ) -> Result<Self, &'static str> {
        let capacity = concurrency
            .min(provider_capacity)
            .min(infrastructure_capacity);
        if capacity == 0 || concurrency > 128 {
            return Err("worker capacity must be 1..=128");
        }
        Ok(Self {
            capacity,
            locks,
            jobs: JoinSet::new(),
            cancellations: HashMap::new(),
        })
    }
    pub fn active(&self) -> usize {
        self.jobs.len()
    }
    pub fn cancel_run(&mut self, run: &RunId) {
        self.cancellations
            .retain(|_, (_, sender)| !sender.is_closed());
        for (id, sender) in self.cancellations.values() {
            if id == run {
                let _ = sender.send(true);
            }
        }
    }
    pub async fn join_next(&mut self) -> Option<Result<(), String>> {
        self.jobs
            .join_next()
            .await
            .map(|r| r.unwrap_or_else(|_| Err("worker job panicked or was cancelled".into())))
    }
    /// Caller executes and verifies each leased task. The pool never equates successful
    /// worker output with completion. Errors block the task and retain its worktree.
    pub async fn dispatch<F, Fut>(
        &mut self,
        store: &Store,
        run: &RunId,
        now: i64,
        budget: BudgetRequest,
        execute: F,
    ) -> Result<usize, StoreError>
    where
        F: Fn(Task, String, watch::Receiver<bool>) -> Fut,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        let ready = tick(store, run, now).await?;
        let tasks = store.list_tasks(run).await?;
        let project = store.get_run(run).await?.project_id.to_string();
        let mut count = 0;
        for id in rank(&tasks, &ready) {
            if self.active() >= self.capacity {
                break;
            }
            let task = tasks.iter().find(|t| t.id() == &id).expect("ranked task");
            let Some(guard) = self
                .locks
                .try_acquire(&project, &task.spec.candidate_paths, false)
            else {
                continue;
            };
            let lease_id = Uuid::new_v4().to_string();
            let req = LeaseRequest {
                lease_id: &lease_id,
                worker_id: &lease_id,
                now,
                ttl_seconds: task.spec.limits.timeout_seconds.saturating_add(300),
            };
            let (leased, _) = match store
                .grant_budgeted_lease(run, &id, task.revision, &req, budget)
                .await
            {
                Ok(v) => v,
                Err(StoreError::BudgetDenied(_) | StoreError::RevisionConflict { .. }) => continue,
                Err(e) => return Err(e),
            };
            let task_run = leased.run_id.clone();
            let task_id = leased.id().clone();
            let (sender, receiver) = watch::channel(false);
            self.cancellations
                .retain(|_, (_, sender)| !sender.is_closed());
            self.cancellations
                .insert(lease_id.clone(), (run.clone(), sender));
            let future = execute(leased, lease_id.clone(), receiver);
            let store = store.clone();
            self.jobs.spawn(async move {
                let _guard = guard;
                let result = future.await;
                if result.is_err() {
                    let current = store
                        .get_task(&task_run, &task_id)
                        .await
                        .map_err(|e| e.to_string())?;
                    if !current.state.is_terminal() && current.state != TaskState::Blocked {
                        store
                            .transition_task(
                                &task_run,
                                &task_id,
                                current.revision,
                                TaskState::Blocked,
                                None,
                            )
                            .await
                            .map_err(|e| e.to_string())?;
                    }
                }
                let end = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(now, |d| d.as_secs() as i64);
                store
                    .release_lease(&lease_id, end)
                    .await
                    .map_err(|e| e.to_string())?;
                store
                    .settle_usage(&lease_id)
                    .await
                    .map_err(|e| e.to_string())?;
                result
            });
            count += 1;
        }
        Ok(count)
    }
}
