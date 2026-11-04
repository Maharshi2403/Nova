use harness_core::domain::*;
use harness_store::repository::{PriorityUpdate, Repository};
use harness_store::{Store, StoreError};
use tempfile::TempDir;

fn spec(id: &str, agent: u8) -> TaskSpec {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "title": "Add order endpoint validation",
        "description": "Implement 400 response when quantity <= 0",
        "complexity": "low", "risk": "low",
        "agent_priority": agent, "manual_priority": null,
        "dependencies": [], "candidate_paths": [],
        "acceptance": [{"kind": "assertion", "description": "quantity <= 0 yields HTTP 400"}],
        "limits": {"max_tool_calls": 20, "max_output_tokens": 3000, "timeout_seconds": 180, "cost_cap_usd": 0.2}
    }))
    .unwrap()
}

fn rid() -> RunId {
    RunId::new("run-1").unwrap()
}

fn p(v: i64) -> Priority {
    Priority::new(v).unwrap()
}

async fn seeded(tasks: &[(&str, u8)]) -> (TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("harness.db")).await.unwrap();
    let proj = Project::new(ProjectId::new("proj").unwrap(), "Demo", "C:/repos/demo").unwrap();
    store.create_project(&proj).await.unwrap();
    store.create_run(&Run::new(rid(), proj.id)).await.unwrap();
    for (id, prio) in tasks {
        let t = Task::new(rid(), spec(id, *prio)).unwrap();
        store.insert_task(&t).await.unwrap();
    }
    (dir, store)
}

fn tid(s: &str) -> TaskId {
    TaskId::new(s).unwrap()
}

async fn ranked(store: &Store, ids: &[&str]) -> Vec<String> {
    let tasks = store.list_tasks(&rid()).await.unwrap();
    let ready: Vec<TaskId> = ids.iter().map(|i| tid(i)).collect();
    harness_runtime::priority::rank(&tasks, &ready)
        .iter()
        .map(|t| t.as_str().to_string())
        .collect()
}

#[tokio::test]
async fn manual_priority_wins() {
    let (_d, store) = seeded(&[("a", 90), ("b", 10), ("c", 50)]).await;
    assert_eq!(ranked(&store, &["a", "b", "c"]).await, ["a", "c", "b"]);

    let t = store
        .set_priority(&rid(), &tid("b"), 0, PriorityUpdate::Manual(Some(p(100))))
        .await
        .unwrap();
    assert_eq!(t.revision, 1);
    assert_eq!(t.spec.manual_priority, Some(p(100)));
    assert_eq!(t.spec.agent_priority, p(10));
    assert_eq!(ranked(&store, &["a", "b", "c"]).await, ["b", "a", "c"]);

    // A manual priority below the agent's still wins; clearing it restores the agent's.
    store
        .set_priority(&rid(), &tid("a"), 0, PriorityUpdate::Manual(Some(p(0))))
        .await
        .unwrap();
    assert_eq!(ranked(&store, &["a", "b", "c"]).await, ["b", "c", "a"]);
    store
        .set_priority(&rid(), &tid("a"), 1, PriorityUpdate::Manual(None))
        .await
        .unwrap();
    assert_eq!(ranked(&store, &["a", "b", "c"]).await, ["b", "a", "c"]);
}

#[tokio::test]
async fn equal_priority_is_fifo() {
    let (_d, store) = seeded(&[("a", 50), ("b", 50), ("c", 50)]).await;
    assert_eq!(ranked(&store, &["c", "a", "b"]).await, ["a", "b", "c"]);
}

#[tokio::test]
async fn stale_priority_update_rejected() {
    let (_d, store) = seeded(&[("a", 50)]).await;
    store
        .set_priority(&rid(), &tid("a"), 0, PriorityUpdate::Agent(p(70)))
        .await
        .unwrap();
    let err = store
        .set_priority(&rid(), &tid("a"), 0, PriorityUpdate::Manual(Some(p(5))))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        StoreError::RevisionConflict {
            expected: 0,
            actual: 1
        }
    ));
    let t = store.get_task(&rid(), &tid("a")).await.unwrap();
    assert_eq!(t.spec.manual_priority, None);
    assert_eq!(t.spec.agent_priority, p(70));
}

#[tokio::test]
async fn running_task_not_preempted() {
    let (_d, store) = seeded(&[("a", 50)]).await;
    let mut rev = 0;
    for s in [
        TaskState::Pending,
        TaskState::Ready,
        TaskState::Leased,
        TaskState::Running,
    ] {
        rev = store
            .transition_task(&rid(), &tid("a"), rev, s, None)
            .await
            .unwrap()
            .task
            .revision;
    }
    let err = store
        .set_priority(&rid(), &tid("a"), rev, PriorityUpdate::Manual(Some(p(100))))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        StoreError::NotReprioritizable(TaskState::Running)
    ));
    let t = store.get_task(&rid(), &tid("a")).await.unwrap();
    assert_eq!(
        (t.state, t.revision, t.spec.manual_priority),
        (TaskState::Running, rev, None)
    );
}

#[tokio::test]
async fn priority_change_is_audited() {
    let (_d, store) = seeded(&[("a", 50)]).await;
    store
        .set_priority(&rid(), &tid("a"), 0, PriorityUpdate::Manual(Some(p(80))))
        .await
        .unwrap();
    let ev = store.list_events(&rid(), 0, 10).await.unwrap();
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0].kind, "task.priority.manual");
}
