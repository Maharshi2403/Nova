use harness_core::domain::*;
use harness_store::repository::Repository;
use harness_store::{Store, StoreError};
use tempfile::TempDir;

async fn fresh() -> (TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("harness.db")).await.unwrap();
    (dir, store)
}

fn spec(id: &str, deps: &[&str]) -> TaskSpec {
    let mut v = serde_json::json!({
        "id": id,
        "title": "Add order endpoint validation",
        "description": "Implement 400 response when quantity <= 0",
        "complexity": "low", "risk": "low",
        "agent_priority": 60, "manual_priority": null,
        "dependencies": [],
        "candidate_paths": ["api/orders.py"],
        "acceptance": [{"kind": "assertion", "description": "quantity <= 0 yields HTTP 400"}],
        "limits": {"max_tool_calls": 20, "max_output_tokens": 3000, "timeout_seconds": 180, "cost_cap_usd": 0.2}
    });
    v["dependencies"] = deps.iter().map(|d| serde_json::json!(d)).collect();
    serde_json::from_value(v).unwrap()
}

fn rid() -> RunId {
    RunId::new("run-1").unwrap()
}

async fn seeded() -> (TempDir, Store) {
    let (dir, store) = fresh().await;
    let p = Project::new(ProjectId::new("proj").unwrap(), "Demo", "C:/repos/demo").unwrap();
    store.create_project(&p).await.unwrap();
    store.create_run(&Run::new(rid(), p.id)).await.unwrap();
    (dir, store)
}

#[tokio::test]
async fn task_roundtrip() {
    let (_d, store) = seeded().await;
    store
        .insert_task(&Task::new(rid(), spec("a", &[])).unwrap())
        .await
        .unwrap();
    let t = Task::new(rid(), spec("b", &["a"])).unwrap();
    store.insert_task(&t).await.unwrap();
    let loaded = store.get_task(&rid(), t.id()).await.unwrap();
    assert_eq!(loaded, t);
    assert_eq!(loaded.spec.dependencies, vec![TaskId::new("a").unwrap()]);

    let p = store
        .get_project(&ProjectId::new("proj").unwrap())
        .await
        .unwrap();
    assert_eq!(p.name, "Demo");
    let r = store.get_run(&rid()).await.unwrap();
    assert_eq!(r.state, RunState::Planning);
}

#[tokio::test]
async fn task_revision_conflict() {
    let (_d, store) = seeded().await;
    let t = Task::new(rid(), spec("a", &[])).unwrap();
    store.insert_task(&t).await.unwrap();

    let t1 = store
        .transition_task(&rid(), t.id(), 0, TaskState::Pending)
        .await
        .unwrap();
    assert_eq!((t1.state, t1.revision), (TaskState::Pending, 1));

    // Stale revision is rejected and leaves the row untouched.
    let err = store
        .transition_task(&rid(), t.id(), 0, TaskState::Ready)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        StoreError::RevisionConflict {
            expected: 0,
            actual: 1
        }
    ));
    assert_eq!(
        store.get_task(&rid(), t.id()).await.unwrap().state,
        TaskState::Pending
    );
}

#[tokio::test]
async fn concurrent_transitions_only_one_wins() {
    let (_d, store) = seeded().await;
    let t = Task::new(rid(), spec("a", &[])).unwrap();
    store.insert_task(&t).await.unwrap();
    let mut handles = Vec::new();
    for to in [TaskState::Pending, TaskState::Cancelled] {
        let s = store.clone();
        let id = t.id().clone();
        handles.push(tokio::spawn(async move {
            s.transition_task(&rid(), &id, 0, to).await.is_ok()
        }));
    }
    let wins = futures_count(handles).await;
    assert_eq!(wins, 1);
    assert_eq!(store.get_task(&rid(), t.id()).await.unwrap().revision, 1);
}

async fn futures_count(handles: Vec<tokio::task::JoinHandle<bool>>) -> usize {
    let mut n = 0;
    for h in handles {
        if h.await.unwrap() {
            n += 1;
        }
    }
    n
}

#[tokio::test]
async fn illegal_transition_rejected() {
    let (_d, store) = seeded().await;
    let t = Task::new(rid(), spec("a", &[])).unwrap();
    store.insert_task(&t).await.unwrap();
    let err = store
        .transition_task(&rid(), t.id(), 0, TaskState::Completed)
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::IllegalTransition(_)));
    assert_eq!(store.get_task(&rid(), t.id()).await.unwrap().revision, 0);
}

#[tokio::test]
async fn run_fk_enforced() {
    let (_d, store) = fresh().await;
    let orphan = Run::new(rid(), ProjectId::new("missing").unwrap());
    assert!(store.create_run(&orphan).await.is_err());

    let (_d2, store2) = seeded().await;
    let other = RunId::new("no-such-run").unwrap();
    let t = Task::new(other, spec("a", &[])).unwrap();
    assert!(store2.insert_task(&t).await.is_err());
}

#[tokio::test]
async fn missing_dependency_and_duplicates_rejected() {
    let (_d, store) = seeded().await;
    let bad = Task::new(rid(), spec("b", &["ghost"])).unwrap();
    assert!(store.insert_task(&bad).await.is_err());
    // The failed insert must not leave a partial task behind.
    assert!(matches!(
        store.get_task(&rid(), bad.id()).await,
        Err(StoreError::NotFound("task"))
    ));

    let a = Task::new(rid(), spec("a", &[])).unwrap();
    store.insert_task(&a).await.unwrap();
    assert!(matches!(
        store.insert_task(&a).await,
        Err(StoreError::AlreadyExists)
    ));
}

#[tokio::test]
async fn not_found_errors() {
    let (_d, store) = seeded().await;
    assert!(matches!(
        store.get_task(&rid(), &TaskId::new("zzz").unwrap()).await,
        Err(StoreError::NotFound("task"))
    ));
    assert!(matches!(
        store.get_run(&RunId::new("zzz").unwrap()).await,
        Err(StoreError::NotFound("run"))
    ));
}
