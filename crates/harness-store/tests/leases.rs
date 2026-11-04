use harness_core::domain::*;
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

#[tokio::test]
async fn one_active_lease_per_task() {
    let (_d, store, rev) = ready_store(&["a"]).await;
    let (task, lease) = store
        .grant_lease(&rid(), &tid("a"), rev, &req("l1", "w1", 1000))
        .await
        .unwrap();
    assert_eq!(task.state, TaskState::Leased);
    assert_eq!((task.attempt, lease.attempt), (1, 1));
    assert_eq!(lease.expires_at, 1060);

    // A second grant fails whether it reuses the old revision or the new one.
    let stale = store
        .grant_lease(&rid(), &tid("a"), rev, &req("l2", "w2", 1001))
        .await
        .unwrap_err();
    assert!(matches!(stale, StoreError::RevisionConflict { .. }));
    let fresh = store
        .grant_lease(&rid(), &tid("a"), task.revision, &req("l3", "w2", 1001))
        .await
        .unwrap_err();
    assert!(matches!(fresh, StoreError::IllegalTransition(_)));

    // The database refuses a second live lease even if application checks are bypassed.
    let raw = sqlx::query(
        "INSERT INTO leases (id, run_id, task_id, worker_id, attempt, granted_at, expires_at) \
         VALUES ('x', 'run-1', 'a', 'w', 1, 1, 2)",
    )
    .execute(store.pool())
    .await;
    assert!(raw.is_err());

    let active = store
        .active_lease(&rid(), &tid("a"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(active.id, "l1");
    let events = store.list_events(&rid(), 0, 100).await.unwrap();
    assert_eq!(events.iter().filter(|e| e.kind == "task.leased").count(), 1);
}

#[tokio::test]
async fn expired_lease_reconciles() {
    let (_d, store, rev) = ready_store(&["a", "b"]).await;
    let (a, _) = store
        .grant_lease(&rid(), &tid("a"), rev, &req("la", "w1", 1000))
        .await
        .unwrap();
    // b has a live lease that has not expired.
    let b_rev = store.get_task(&rid(), &tid("b")).await.unwrap().revision;
    store
        .grant_lease(&rid(), &tid("b"), b_rev, &req("lb", "w2", 1050))
        .await
        .unwrap();
    // a is running when its lease expires.
    store
        .transition_task(&rid(), &tid("a"), a.revision, TaskState::Running, None)
        .await
        .unwrap();

    assert!(
        store
            .reconcile_expired_leases(1059)
            .await
            .unwrap()
            .is_empty()
    );
    let done = store.reconcile_expired_leases(1060).await.unwrap();
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].task_id, tid("a"));
    assert_eq!(
        (done[0].from, done[0].to),
        (TaskState::Running, TaskState::RetryWait)
    );

    let a_now = store.get_task(&rid(), &tid("a")).await.unwrap();
    assert_eq!(a_now.state, TaskState::RetryWait);
    assert!(
        store
            .active_lease(&rid(), &tid("a"))
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store.get_task(&rid(), &tid("b")).await.unwrap().state,
        TaskState::Leased
    );

    // Idempotent, and the recovered task can be re-leased after going back to ready.
    assert!(store.reconcile_expired_leases(5000).await.unwrap().len() == 1);
    let r = store
        .transition_task(&rid(), &tid("a"), a_now.revision, TaskState::Ready, None)
        .await
        .unwrap();
    let (again, lease) = store
        .grant_lease(&rid(), &tid("a"), r.task.revision, &req("la2", "w3", 6000))
        .await
        .unwrap();
    assert_eq!((again.attempt, lease.attempt), (2, 2));
}

#[tokio::test]
async fn leases_survive_restart() {
    let (dir, store, rev) = ready_store(&["a"]).await;
    store
        .grant_lease(&rid(), &tid("a"), rev, &req("l1", "w1", 1000))
        .await
        .unwrap();
    store.close().await;

    let reopened = Store::open(dir.path().join("harness.db")).await.unwrap();
    let lease = reopened
        .active_lease(&rid(), &tid("a"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(lease.id, "l1");
    let t = reopened.get_task(&rid(), &tid("a")).await.unwrap();
    let err = reopened
        .grant_lease(&rid(), &tid("a"), t.revision, &req("l2", "w2", 1010))
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::IllegalTransition(_)));
}

#[tokio::test]
async fn release_lease_is_single_use() {
    let (_d, store, rev) = ready_store(&["a"]).await;
    store
        .grant_lease(&rid(), &tid("a"), rev, &req("l1", "w1", 1000))
        .await
        .unwrap();
    store.release_lease("l1", 1005).await.unwrap();
    assert!(matches!(
        store.release_lease("l1", 1006).await.unwrap_err(),
        StoreError::NotFound(_)
    ));
    assert!(
        store
            .active_lease(&rid(), &tid("a"))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_lease_race_single_winner() {
    let (_d, store, rev) = ready_store(&["a"]).await;
    let mut handles = Vec::new();
    for i in 0..8 {
        let store = store.clone();
        handles.push(tokio::spawn(async move {
            let lease_id = format!("lease-{i}");
            let worker = format!("worker-{i}");
            store
                .grant_lease(&rid(), &tid("a"), rev, &req(&lease_id, &worker, 1000))
                .await
                .map(|(_, l)| l.id)
        }));
    }
    let mut winners = 0;
    for h in handles {
        match h.await.unwrap() {
            Ok(_) => winners += 1,
            Err(e) => assert!(
                matches!(
                    e,
                    StoreError::RevisionConflict { .. } | StoreError::IllegalTransition(_)
                ),
                "{e}"
            ),
        }
    }
    assert_eq!(winners, 1);
    let live: i64 = sqlx::query_scalar("SELECT count(*) FROM leases WHERE released_at IS NULL")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(live, 1);
}
