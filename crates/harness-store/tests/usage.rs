use harness_core::domain::*;
use harness_store::budgets::BudgetRequest;
use harness_store::leases::LeaseRequest;
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

fn usage(cost: Option<f64>) -> harness_core::messages::Usage {
    serde_json::from_value(serde_json::json!({"provider": "mock", "model_id": "m", "input_tokens": 100, "cached_input_tokens": 60, "output_tokens": 20, "cost_usd": cost, "price_source": "provider_billed"})).unwrap()
}

#[tokio::test]
async fn usage_aggregates_and_cached_tokens_accounted() {
    use harness_store::usage::UsageSummary;
    let (_d, store, rev) = ready_store(&["a"]).await;
    store
        .grant_budgeted_lease(
            &rid(),
            &tid("a"),
            rev,
            &req("l", "w", 0),
            budget(100_000, 100_000, 100_000),
        )
        .await
        .unwrap();
    store
        .record_usage("l", "one", &usage(Some(0.01)))
        .await
        .unwrap();
    store
        .record_usage("l", "one", &usage(Some(0.01)))
        .await
        .unwrap();
    store
        .record_usage("l", "two", &usage(Some(0.02)))
        .await
        .unwrap();
    let records = store.list_usage(&rid(), Some(&tid("a"))).await.unwrap();
    let sum = UsageSummary::from_records(&records);
    assert_eq!(sum.calls, 2);
    assert_eq!(sum.input_tokens, Some(200));
    assert_eq!(sum.cached_input_tokens, Some(120));
    assert_eq!(sum.output_tokens, Some(40));
    assert_eq!(sum.cost_usd, Some(0.03));
    assert!(matches!(
        store.record_usage("l", "one", &usage(Some(0.04))).await,
        Err(StoreError::IdempotencyKeyReused)
    ));
}

#[tokio::test]
async fn missing_usage_unknown_and_reservation_retained() {
    use harness_store::usage::UsageSummary;
    assert_eq!(UsageSummary::from_records(&[]).cost_usd, None);
    let (_d, store, rev) = ready_store(&["a"]).await;
    let (task, _) = store
        .grant_budgeted_lease(
            &rid(),
            &tid("a"),
            rev,
            &req("l", "w", 0),
            budget(100_000, 100_000, 100_000),
        )
        .await
        .unwrap();
    let mut unknown = usage(None);
    unknown.output_tokens = None;
    store.record_usage("l", "one", &unknown).await.unwrap();
    let sum = UsageSummary::from_records(&store.list_usage(&rid(), None).await.unwrap());
    assert_eq!(sum.cost_usd, None);
    assert_eq!(sum.output_tokens, None);
    store
        .transition_task(&rid(), &tid("a"), task.revision, TaskState::RetryWait, None)
        .await
        .unwrap();
    store.settle_usage("l").await.unwrap();
    let amount: i64 =
        sqlx::query_scalar("SELECT amount FROM budget_reservations WHERE lease_id = 'l'")
            .fetch_one(store.pool())
            .await
            .unwrap();
    assert_eq!(amount, 100_000);
}

#[tokio::test]
async fn known_cost_settles_once_after_worker_stops() {
    let (_d, store, rev) = ready_store(&["a"]).await;
    let (task, _) = store
        .grant_budgeted_lease(
            &rid(),
            &tid("a"),
            rev,
            &req("l", "w", 0),
            budget(100_000, 100_000, 100_000),
        )
        .await
        .unwrap();
    store
        .record_usage("l", "one", &usage(Some(0.025)))
        .await
        .unwrap();
    assert!(store.settle_usage("l").await.is_err());
    store
        .transition_task(&rid(), &tid("a"), task.revision, TaskState::RetryWait, None)
        .await
        .unwrap();
    store.settle_usage("l").await.unwrap();
    store.settle_usage("l").await.unwrap();
    let amount: i64 =
        sqlx::query_scalar("SELECT amount FROM budget_reservations WHERE lease_id = 'l'")
            .fetch_one(store.pool())
            .await
            .unwrap();
    assert_eq!(amount, 25_000);
}

#[tokio::test]
async fn interrupted_provider_call_keeps_reservation_even_with_prior_known_usage() {
    let (_d, store, rev) = ready_store(&["a"]).await;
    let (task, _) = store
        .grant_budgeted_lease(
            &rid(),
            &tid("a"),
            rev,
            &req("l", "w", 0),
            budget(100_000, 100_000, 100_000),
        )
        .await
        .unwrap();
    store
        .record_usage("l", "finished", &usage(Some(0.01)))
        .await
        .unwrap();
    store.begin_provider_call("interrupted", "l").await.unwrap();
    store
        .transition_task(&rid(), &tid("a"), task.revision, TaskState::RetryWait, None)
        .await
        .unwrap();
    store.settle_usage("l").await.unwrap();
    let amount: i64 =
        sqlx::query_scalar("SELECT amount FROM budget_reservations WHERE lease_id='l'")
            .fetch_one(store.pool())
            .await
            .unwrap();
    assert_eq!(amount, 100_000);
}
