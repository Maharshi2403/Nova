use harness_core::domain::*;
use harness_store::{
    Store,
    leases::{LeaseRepository, LeaseRequest},
    plans::PlanRepository,
    repository::Repository,
};

#[tokio::test]
async fn expired_worker_recovered_without_reexecution() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let store = Store::open(&path).await.unwrap();
    let p = Project::new(ProjectId::new("proj-demo").unwrap(), "fixture", "/fixture").unwrap();
    store.create_project(&p).await.unwrap();
    let run = Run::new(RunId::new("r").unwrap(), p.id);
    store.create_run(&run).await.unwrap();
    let plan = serde_json::from_str(include_str!(
        "../../../tests/fixtures/plans/valid_plan.json"
    ))
    .unwrap();
    store.propose_plan(&run.id, &plan).await.unwrap();
    store.approve_plan(&run.id, 1).await.unwrap();
    harness_runtime::scheduler::tick(&store, &run.id, 0)
        .await
        .unwrap();
    let task = store.list_tasks(&run.id).await.unwrap().remove(0);
    store
        .grant_lease(
            &run.id,
            task.id(),
            task.revision,
            &LeaseRequest {
                lease_id: "expired",
                worker_id: "old-worker",
                now: 0,
                ttl_seconds: 1,
            },
        )
        .await
        .unwrap();
    store.close().await;
    let store = Store::open(&path).await.unwrap();
    assert_eq!(
        harness_runtime::recovery::reconcile(&store, 2)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        store.get_task(&run.id, task.id()).await.unwrap().state,
        TaskState::Blocked
    );
    assert!(
        harness_runtime::recovery::reconcile(&store, 3)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .active_lease(&run.id, task.id())
            .await
            .unwrap()
            .is_none()
    );
}
