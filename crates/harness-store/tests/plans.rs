use harness_core::domain::*;
use harness_core::plan::PlanProposal;
use harness_store::leases::{LeaseRepository, LeaseRequest};
use harness_store::plans::{PlanRepository, PlanStatus};
use harness_store::repository::Repository;
use harness_store::{Store, StoreError};
use tempfile::TempDir;

const VALID: &str = include_str!("../../../tests/fixtures/plans/valid_plan.json");

fn rid() -> RunId {
    RunId::new("run-1").unwrap()
}

async fn store_with_run() -> (TempDir, Store) {
    let dir = TempDir::new().unwrap();
    let store = Store::open(&dir.path().join("t.db")).await.unwrap();
    let pid = ProjectId::new("proj-demo").unwrap();
    store
        .create_project(&Project::new(pid.clone(), "demo", "/tmp/demo").unwrap())
        .await
        .unwrap();
    store.create_run(&Run::new(rid(), pid)).await.unwrap();
    (dir, store)
}

fn plan() -> PlanProposal {
    PlanProposal::from_json(VALID).unwrap()
}

#[tokio::test]
async fn unapproved_plan_cannot_run() {
    let (_d, store) = store_with_run().await;
    let rec = store.propose_plan(&rid(), &plan()).await.unwrap();
    assert_eq!(rec.version, 1);
    assert_eq!(rec.status, PlanStatus::Proposed);
    assert_eq!(
        store.get_run(&rid()).await.unwrap().state,
        RunState::AwaitingApproval
    );
    let tasks = store.list_tasks(&rid()).await.unwrap();
    assert_eq!(tasks.len(), 2);
    assert!(tasks.iter().all(|t| t.state == TaskState::Proposed));

    let req = LeaseRequest {
        lease_id: "l1",
        worker_id: "w1",
        now: 0,
        ttl_seconds: 30,
    };
    let err = store
        .grant_lease(&rid(), &tasks[0].id().clone(), tasks[0].revision, &req)
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::IllegalTransition(_)));

    let approved = store.approve_plan(&rid(), 1).await.unwrap();
    assert_eq!(approved.status, PlanStatus::Approved);
    assert_eq!(
        store.get_run(&rid()).await.unwrap().state,
        RunState::Running
    );
    let tasks = store.list_tasks(&rid()).await.unwrap();
    assert!(tasks.iter().all(|t| t.state == TaskState::Pending));
    assert!(matches!(
        store.approve_plan(&rid(), 1).await.unwrap_err(),
        StoreError::PlanNotDecidable(_)
    ));
}

#[tokio::test]
async fn invalid_plan_not_persisted() {
    let (_d, store) = store_with_run().await;
    let mut cyclic = plan();
    cyclic.tasks[0].dependencies = vec![cyclic.tasks[1].id.clone()];
    assert!(matches!(
        store.propose_plan(&rid(), &cyclic).await.unwrap_err(),
        StoreError::Plan(_)
    ));

    let mut missing = plan();
    missing.tasks[1].dependencies = vec![TaskId::new("ghost").unwrap()];
    assert!(store.propose_plan(&rid(), &missing).await.is_err());

    let mut bad_task = plan();
    bad_task.tasks[0].acceptance.clear();
    assert!(store.propose_plan(&rid(), &bad_task).await.is_err());

    assert!(store.list_tasks(&rid()).await.unwrap().is_empty());
    assert!(matches!(
        store.get_plan(&rid(), 1).await.unwrap_err(),
        StoreError::NotFound(_)
    ));
    assert_eq!(
        store.get_run(&rid()).await.unwrap().state,
        RunState::Planning
    );
}

#[tokio::test]
async fn plan_content_is_immutable() {
    let (_d, store) = store_with_run().await;
    store.propose_plan(&rid(), &plan()).await.unwrap();
    let err = sqlx_update(&store).await;
    assert!(err.contains("immutable"), "{err}");
    // Second proposal is refused while awaiting approval.
    assert!(store.propose_plan(&rid(), &plan()).await.is_err());
}

async fn sqlx_update(store: &Store) -> String {
    sqlx::query("UPDATE plans SET content_json = '{}' WHERE run_id = 'run-1'")
        .execute(store.pool())
        .await
        .unwrap_err()
        .to_string()
}
