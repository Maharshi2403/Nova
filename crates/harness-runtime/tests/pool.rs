use harness_core::domain::*;
use harness_runtime::{locks::ResourceLocks, pool::WorkerPool};
use harness_store::{Store, budgets::BudgetRequest, plans::PlanRepository, repository::Repository};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[tokio::test]
async fn concurrency_cap_respected_and_blocked_never_dispatched() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("db")).await.unwrap();
    let project = Project::new(ProjectId::new("proj-demo").unwrap(), "test", "/fixture").unwrap();
    store.create_project(&project).await.unwrap();
    let run = Run::new(RunId::new("r").unwrap(), project.id);
    store.create_run(&run).await.unwrap();
    let mut plan: harness_core::plan::PlanProposal = serde_json::from_str(include_str!(
        "../../../tests/fixtures/plans/valid_plan.json"
    ))
    .unwrap();
    plan.tasks[0].candidate_paths = vec!["a".into()];
    store.propose_plan(&run.id, &plan).await.unwrap();
    store.approve_plan(&run.id, 1).await.unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut pool = WorkerPool::new(2, 1, 4, ResourceLocks::default()).unwrap();
    let budget = BudgetRequest {
        estimated_microdollars: 0,
        run_cap_microdollars: 0,
        daily_cap_microdollars: 0,
    };
    let c = calls.clone();
    assert_eq!(
        pool.dispatch(
            &store,
            &run.id,
            1,
            |_| budget,
            move |_, _, _| {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Err("injected failure".into())
                }
            }
        )
        .await
        .unwrap(),
        1
    );
    assert_eq!(pool.active(), 1);
    assert!(pool.join_next().await.unwrap().is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        store.list_tasks(&run.id).await.unwrap()[0].state,
        TaskState::Blocked
    );
    assert_eq!(
        pool.dispatch(
            &store,
            &run.id,
            2,
            |_| budget,
            |_, _, _| async { panic!("blocked task dispatched") }
        )
        .await
        .unwrap(),
        0
    );
    let snapshot = store.get_run(&run.id).await.unwrap();
    store
        .transition_run(&run.id, snapshot.revision, RunState::Paused, None)
        .await
        .unwrap();
    let task = store.list_tasks(&run.id).await.unwrap().remove(0);
    let task = store
        .transition_task(&run.id, task.id(), task.revision, TaskState::Ready, None)
        .await
        .unwrap()
        .task;
    let request = harness_store::leases::LeaseRequest {
        lease_id: "paused",
        worker_id: "worker",
        now: 3,
        ttl_seconds: 60,
    };
    assert!(
        store
            .grant_dispatch_lease(
                (&run.id, snapshot.revision),
                task.id(),
                task.revision,
                &request,
                budget
            )
            .await
            .is_err()
    );
}
