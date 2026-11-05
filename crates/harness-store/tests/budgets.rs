use harness_core::domain::*;
use harness_store::budgets::BudgetRequest;
use harness_store::leases::{LeaseRepository, LeaseRequest};
use harness_store::repository::Repository;
use harness_store::{Store, StoreError};
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
async fn budget_denied_before_dispatch() {
    for (b, scope) in [
        (budget(200_001, 1_000_000, 1_000_000), "task limits"),
        (budget(100_000, 99_999, 1_000_000), "run"),
        (budget(100_000, 1_000_000, 99_999), "daily"),
    ] {
        let (_d, store, rev) = ready_store(&["a"]).await;
        let result = store
            .grant_budgeted_lease(&rid(), &tid("a"), rev, &req("l", "w", 1000), b)
            .await;
        assert!(matches!(result, Err(StoreError::BudgetDenied(s)) if s == scope));
        assert_eq!(
            store.get_task(&rid(), &tid("a")).await.unwrap().state,
            TaskState::Ready
        );
        assert!(
            store
                .active_lease(&rid(), &tid("a"))
                .await
                .unwrap()
                .is_none()
        );
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE kind = 'task.leased'")
                .fetch_one(store.pool())
                .await
                .unwrap();
        assert_eq!(count, 0);
    }
}

#[tokio::test]
async fn reservation_released_on_failure() {
    let (_d, store, rev) = ready_store(&["a", "b"]).await;
    let b = budget(100_000, 100_000, 100_000);
    store
        .grant_budgeted_lease(&rid(), &tid("a"), rev, &req("a", "w", 1000), b)
        .await
        .unwrap();
    assert!(matches!(
        store
            .grant_budgeted_lease(&rid(), &tid("b"), rev, &req("b", "w", 1000), b)
            .await,
        Err(StoreError::BudgetDenied("run"))
    ));
    store.release_failed_dispatch_budget("a").await.unwrap();
    store.release_failed_dispatch_budget("a").await.unwrap();
    store
        .grant_budgeted_lease(&rid(), &tid("b"), rev, &req("b", "w", 1000), b)
        .await
        .unwrap();
}

#[tokio::test]
async fn concurrent_reservations_cannot_overspend() {
    let (_d, store, rev) = ready_store(&["a", "b"]).await;
    let b = budget(100_000, 100_000, 100_000);
    let run = rid();
    let a = tid("a");
    let c = tid("b");
    let ra = req("a", "w1", 1000);
    let rb = req("b", "w2", 1000);
    let (a, b) = tokio::join!(
        store.grant_budgeted_lease(&run, &a, rev, &ra, b),
        store.grant_budgeted_lease(&run, &c, rev, &rb, b)
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    let err = if let Err(e) = a { e } else { b.unwrap_err() };
    assert!(matches!(err, StoreError::BudgetDenied("run")), "{err:?}");
}

#[tokio::test]
async fn reservations_survive_restart_and_utc_rollover() {
    let (d, store, rev) = ready_store(&["a", "b", "c"]).await;
    let b = budget(100_000, 1_000_000, 100_000);
    store
        .grant_budgeted_lease(&rid(), &tid("a"), rev, &req("a", "w", 86_399), b)
        .await
        .unwrap();
    store.close().await;
    let store = Store::open(d.path().join("harness.db")).await.unwrap();
    assert!(matches!(
        store
            .grant_budgeted_lease(&rid(), &tid("b"), rev, &req("b", "w", 86_399), b)
            .await,
        Err(StoreError::BudgetDenied("daily"))
    ));
    store
        .grant_budgeted_lease(&rid(), &tid("c"), rev, &req("c", "w", 86_400), b)
        .await
        .unwrap();
}

#[tokio::test]
async fn daily_cap_is_shared_across_runs() {
    let (_d, store, rev) = ready_store(&["a"]).await;
    let other = RunId::new("run-2").unwrap();
    store
        .create_run(&Run::new(other.clone(), ProjectId::new("proj").unwrap()))
        .await
        .unwrap();
    store
        .insert_task(&Task::new(other.clone(), spec("b")).unwrap())
        .await
        .unwrap();
    let task = store
        .transition_task(&other, &tid("b"), 0, TaskState::Pending, None)
        .await
        .unwrap()
        .task;
    let task = store
        .transition_task(&other, &tid("b"), task.revision, TaskState::Ready, None)
        .await
        .unwrap()
        .task;
    let b = budget(100_000, 200_000, 100_000);
    store
        .grant_budgeted_lease(&rid(), &tid("a"), rev, &req("a", "w", 1000), b)
        .await
        .unwrap();
    assert!(matches!(
        store
            .grant_budgeted_lease(&other, &tid("b"), task.revision, &req("b", "w", 1000), b)
            .await,
        Err(StoreError::BudgetDenied("daily"))
    ));
}

#[tokio::test]
async fn started_attempt_does_not_release_reserved_spend() {
    let (_d, store, rev) = ready_store(&["a", "b"]).await;
    let b = budget(100_000, 100_000, 100_000);
    let (task, _) = store
        .grant_budgeted_lease(&rid(), &tid("a"), rev, &req("a", "w", 1000), b)
        .await
        .unwrap();
    store
        .transition_task(&rid(), &tid("a"), task.revision, TaskState::Running, None)
        .await
        .unwrap();
    assert!(matches!(
        store.release_failed_dispatch_budget("a").await,
        Err(StoreError::BudgetDenied("attempt already started"))
    ));
    assert!(matches!(
        store
            .grant_budgeted_lease(&rid(), &tid("b"), rev, &req("b", "w", 1000), b)
            .await,
        Err(StoreError::BudgetDenied("run"))
    ));
}
