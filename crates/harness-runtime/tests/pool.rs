use harness_core::domain::*;
use harness_core::messages::Usage;
use harness_runtime::{locks::ResourceLocks, pool::WorkerPool};
use harness_store::leases::LeaseRepository;
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
    assert_eq!(
        store
            .latest_task_failure(&run.id, &TaskId::new("task-model").unwrap())
            .await
            .unwrap()
            .as_deref(),
        Some("injected failure")
    );
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

async fn setup() -> (tempfile::TempDir, Store, RunId, WorkerPool, BudgetRequest) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("db")).await.unwrap();
    let project = Project::new(ProjectId::new("proj-demo").unwrap(), "test", "/fixture").unwrap();
    store.create_project(&project).await.unwrap();
    let run = Run::new(RunId::new("r").unwrap(), project.id);
    store.create_run(&run).await.unwrap();
    let plan: harness_core::plan::PlanProposal = serde_json::from_str(include_str!(
        "../../../tests/fixtures/plans/valid_plan.json"
    ))
    .unwrap();
    store.propose_plan(&run.id, &plan).await.unwrap();
    store.approve_plan(&run.id, 1).await.unwrap();
    let pool = WorkerPool::new(1, 1, 1, ResourceLocks::default()).unwrap();
    let budget = BudgetRequest {
        estimated_microdollars: 100,
        run_cap_microdollars: 1_000,
        daily_cap_microdollars: 1_000,
    };
    (dir, store, run.id, pool, budget)
}

#[tokio::test]
async fn failure_bookkeeping_error_still_releases_lease() {
    let (_dir, store, run, mut pool, budget) = setup().await;
    sqlx::query("CREATE TRIGGER fail_failure BEFORE INSERT ON events WHEN NEW.kind = 'task.failure' BEGIN SELECT RAISE(FAIL, 'injected failure'); END")
        .execute(store.pool()).await.unwrap();
    assert_eq!(
        pool.dispatch(
            &store,
            &run,
            1,
            |_| budget,
            |_, _, _| async { Err("worker failed".into()) }
        )
        .await
        .unwrap(),
        1
    );
    let error = pool.join_next().await.unwrap().unwrap_err();
    assert!(error.contains("failure event"), "{error}");
    let task_id = TaskId::new("task-model").unwrap();
    assert!(store.active_lease(&run, &task_id).await.unwrap().is_none());
    assert_eq!(
        store.get_task(&run, &task_id).await.unwrap().state,
        TaskState::Blocked
    );
}

#[tokio::test]
async fn cancelled_task_is_cancelled_not_blocked() {
    let (_dir, store, run, mut pool, budget) = setup().await;
    assert_eq!(
        pool.dispatch(
            &store,
            &run,
            1,
            |_| budget,
            |_, _, mut cancel| async move {
                cancel.changed().await.unwrap();
                Err("assignment cancelled".into())
            }
        )
        .await
        .unwrap(),
        1
    );
    pool.cancel_run(&run);
    assert!(pool.join_next().await.unwrap().is_err());
    let task_id = TaskId::new("task-model").unwrap();
    assert_eq!(
        store.get_task(&run, &task_id).await.unwrap().state,
        TaskState::Cancelled
    );
    assert!(store.active_lease(&run, &task_id).await.unwrap().is_none());
}

#[tokio::test]
async fn budget_reservation_settled_on_failure() {
    let (_dir, store, run, mut pool, budget) = setup().await;
    let usage_store = store.clone();
    assert_eq!(
        pool.dispatch(
            &store,
            &run,
            1,
            |_| budget,
            move |_, lease, _| {
                let store = usage_store.clone();
                async move {
                    let usage: Usage = serde_json::from_value(serde_json::json!({
                        "provider":"mock", "model_id":"m", "input_tokens":1,
                        "output_tokens":1, "cost_usd":0.00005, "price_source":"provider_billed"
                    }))
                    .unwrap();
                    store.record_usage(&lease, "usage-1", &usage).await.unwrap();
                    Err("worker failed".into())
                }
            }
        )
        .await
        .unwrap(),
        1
    );
    assert!(pool.join_next().await.unwrap().is_err());
    let (amount, settled): (i64, i64) =
        sqlx::query_as("SELECT amount, settled FROM budget_reservations LIMIT 1")
            .fetch_one(store.pool())
            .await
            .unwrap();
    assert_eq!((amount, settled), (50, 1));
}
