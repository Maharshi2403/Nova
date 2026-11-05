use harness_core::domain::*;
use harness_store::Store;
use harness_store::budgets::BudgetRequest;
use harness_store::leases::LeaseRequest;
use harness_store::repository::Repository;
use tempfile::TempDir;

fn spec(id: &str) -> TaskSpec {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "title": "Add order endpoint validation",
        "description": "Implement 400 response when quantity <= 0",
        "complexity": "low", "risk": "low",
        "agent_priority": 60, "manual_priority": null,
        "dependencies": [], "candidate_paths": [],
        "acceptance": [{"kind": "assertion", "description": "quantity <= 0 yields HTTP 400"}],
        "limits": {"max_tool_calls": 20, "max_output_tokens": 3000, "timeout_seconds": 180, "cost_cap_usd": 0.2}
    }))
    .unwrap()
}

fn rid() -> RunId {
    RunId::new("run-1").unwrap()
}

fn tid(s: &str) -> TaskId {
    TaskId::new(s).unwrap()
}

fn req<'a>(lease: &'a str, worker: &'a str, now: i64) -> LeaseRequest<'a> {
    LeaseRequest {
        lease_id: lease,
        worker_id: worker,
        now,
        ttl_seconds: 60,
    }
}

/// Create tasks and walk each to `ready`; returns the store and the ready revision.
async fn ready_store(ids: &[&str]) -> (TempDir, Store, u64) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("harness.db")).await.unwrap();
    let p = Project::new(ProjectId::new("proj").unwrap(), "Demo", "C:/repos/demo").unwrap();
    store.create_project(&p).await.unwrap();
    store.create_run(&Run::new(rid(), p.id)).await.unwrap();
    let mut rev = 0;
    for id in ids {
        store
            .insert_task(&Task::new(rid(), spec(id)).unwrap())
            .await
            .unwrap();
        rev = 0;
        for s in [TaskState::Pending, TaskState::Ready] {
            rev = store
                .transition_task(&rid(), &tid(id), rev, s, None)
                .await
                .unwrap()
                .task
                .revision;
        }
    }
    (dir, store, rev)
}

fn budget(estimate: u64, run: u64, daily: u64) -> BudgetRequest {
    BudgetRequest {
        estimated_microdollars: estimate,
        run_cap_microdollars: run,
        daily_cap_microdollars: daily,
    }
}

#[tokio::test]
async fn failed_spawn_releases_budget_and_lease() {
    use harness_runtime::budget::{DispatchError, dispatch};
    use harness_runtime::worker::WorkerSpec;
    use harness_store::leases::LeaseRepository;
    let (dir, store, rev) = ready_store(&["a", "b"]).await;
    let b = budget(100_000, 100_000, 100_000);
    let spec = WorkerSpec::python_worker(dir.path().join("missing-python"));
    let error = dispatch(
        &store,
        &rid(),
        &tid("a"),
        rev,
        &req("a", "w", 1000),
        b,
        &spec,
    )
    .await
    .err()
    .unwrap();
    assert!(matches!(error, DispatchError::Worker(_)));
    assert_eq!(
        store.get_task(&rid(), &tid("a")).await.unwrap().state,
        TaskState::RetryWait
    );
    assert!(
        store
            .active_lease(&rid(), &tid("a"))
            .await
            .unwrap()
            .is_none()
    );
    // Another task can consume the released capacity immediately.
    store
        .grant_budgeted_lease(&rid(), &tid("b"), rev, &req("b", "w", 1000), b)
        .await
        .unwrap();
}

#[tokio::test]
async fn denied_budget_does_not_launch_worker() {
    use harness_runtime::budget::{DispatchError, dispatch};
    use harness_runtime::worker::WorkerSpec;
    use harness_store::StoreError;
    let (dir, store, rev) = ready_store(&["a"]).await;
    // An invalid executable would produce a spawn error if dispatch reached it.
    let spec = WorkerSpec::python_worker(dir.path().join("missing-python"));
    let error = dispatch(
        &store,
        &rid(),
        &tid("a"),
        rev,
        &req("a", "w", 1000),
        budget(1, 0, 0),
        &spec,
    )
    .await
    .err()
    .unwrap();
    assert!(matches!(
        error,
        DispatchError::Store(StoreError::BudgetDenied("run"))
    ));
    assert_eq!(
        store.get_task(&rid(), &tid("a")).await.unwrap().state,
        TaskState::Ready
    );
}
