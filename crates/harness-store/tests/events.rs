use harness_core::domain::*;
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

async fn seeded() -> (TempDir, Store, TaskId) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("harness.db")).await.unwrap();
    let p = Project::new(ProjectId::new("proj").unwrap(), "Demo", "C:/repos/demo").unwrap();
    store.create_project(&p).await.unwrap();
    store.create_run(&Run::new(rid(), p.id)).await.unwrap();
    let t = Task::new(rid(), spec("a")).unwrap();
    store.insert_task(&t).await.unwrap();
    let id = t.id().clone();
    (dir, store, id)
}

#[tokio::test]
async fn event_and_transition_atomic() {
    let (_d, store, id) = seeded().await;

    // Success: exactly one event, same revision bump.
    store
        .transition_task(&rid(), &id, 0, TaskState::Pending, None)
        .await
        .unwrap();
    let events = store.list_events(&rid(), 0, 100).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].from_state.as_deref(), Some("proposed"));
    assert_eq!(events[0].to_state.as_deref(), Some("pending"));
    assert_eq!(events[0].task_id.as_deref(), Some("a"));

    // Force the event insert to fail: the state change must roll back with it.
    sqlx::query(
        "CREATE TRIGGER boom BEFORE INSERT ON events BEGIN SELECT RAISE(ABORT, 'boom'); END",
    )
    .execute(store.pool())
    .await
    .unwrap();
    let err = store
        .transition_task(&rid(), &id, 1, TaskState::Ready, None)
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::Db(_)), "{err}");
    let task = store.get_task(&rid(), &id).await.unwrap();
    assert_eq!((task.state, task.revision), (TaskState::Pending, 1));
    assert_eq!(store.list_events(&rid(), 0, 100).await.unwrap().len(), 1);
}

#[tokio::test]
async fn replayed_event_idempotent() {
    let (_d, store, id) = seeded().await;
    let first = store
        .transition_task(&rid(), &id, 0, TaskState::Pending, Some("req-1"))
        .await
        .unwrap();
    assert!(!first.replayed);

    // Same key, same request (even with a now-stale revision): no-op replay.
    let again = store
        .transition_task(&rid(), &id, 0, TaskState::Pending, Some("req-1"))
        .await
        .unwrap();
    assert!(again.replayed);
    assert_eq!(again.task.revision, 1);
    assert_eq!(store.list_events(&rid(), 0, 100).await.unwrap().len(), 1);

    // Same key for a different request is refused, not silently ignored.
    let reused = store
        .transition_task(&rid(), &id, 1, TaskState::Ready, Some("req-1"))
        .await
        .unwrap_err();
    assert!(matches!(reused, StoreError::IdempotencyKeyReused));
    assert_eq!(
        store.get_task(&rid(), &id).await.unwrap().state,
        TaskState::Pending
    );
}

#[tokio::test]
async fn events_monotonic() {
    let (_d, store, id) = seeded().await;
    let path = [
        TaskState::Pending,
        TaskState::Ready,
        TaskState::Leased,
        TaskState::Running,
    ];
    for (rev, to) in path.into_iter().enumerate() {
        store
            .transition_task(&rid(), &id, rev as u64, to, None)
            .await
            .unwrap();
    }
    let events = store.list_events(&rid(), 0, 100).await.unwrap();
    assert_eq!(events.len(), 4);
    assert!(events.windows(2).all(|w| w[0].sequence < w[1].sequence));

    // Resume from a cursor.
    let tail = store
        .list_events(&rid(), events[1].sequence, 100)
        .await
        .unwrap();
    assert_eq!(tail.len(), 2);
    assert_eq!(tail[0].sequence, events[2].sequence);
    assert_eq!(store.list_events(&rid(), 0, 2).await.unwrap().len(), 2);
}

#[tokio::test]
async fn failed_transitions_write_no_events() {
    let (_d, store, id) = seeded().await;
    assert!(
        store
            .transition_task(&rid(), &id, 0, TaskState::Completed, None)
            .await
            .is_err()
    );
    assert!(
        store
            .transition_task(&rid(), &id, 9, TaskState::Pending, None)
            .await
            .is_err()
    );
    assert!(store.list_events(&rid(), 0, 100).await.unwrap().is_empty());
}

#[tokio::test]
async fn events_are_append_only() {
    let (_d, store, id) = seeded().await;
    store
        .transition_task(&rid(), &id, 0, TaskState::Pending, None)
        .await
        .unwrap();
    assert!(
        sqlx::query("UPDATE events SET kind = 'x'")
            .execute(store.pool())
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM events")
            .execute(store.pool())
            .await
            .is_err()
    );
}
