use harness_core::{
    approval::{Action, Operation},
    domain::*,
};
use harness_store::{Store, repository::Repository};

#[tokio::test]
async fn risky_tool_blocked_without_approval_and_decisions_audited() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("db")).await.unwrap();
    let p = Project::new(ProjectId::new("p").unwrap(), "fixture", "/fixture").unwrap();
    store.create_project(&p).await.unwrap();
    let run = Run::new(RunId::new("r").unwrap(), p.id);
    store.create_run(&run).await.unwrap();
    let mut plan: harness_core::plan::PlanProposal = serde_json::from_str(include_str!(
        "../../../tests/fixtures/plans/valid_plan.json"
    ))
    .unwrap();
    let mut task = Task::new(run.id.clone(), plan.tasks.remove(0)).unwrap();
    task.state = TaskState::Running;
    task.attempt = 1;
    store.insert_task(&task).await.unwrap();
    let action = Action {
        operation: Operation::Push,
        argv: vec!["git".into(), "push".into()],
        cwd: "/fixture".into(),
    };
    let pending = store
        .request_approval("a", &run.id, task.id(), 1, &action)
        .await
        .unwrap();
    assert!(!store.consume_approval(&pending.id, &action).await.unwrap());
    assert_eq!(
        store
            .request_approval("duplicate", &run.id, task.id(), 1, &action)
            .await
            .unwrap()
            .id,
        "a"
    );
    store.decide_approval("a", 0, true).await.unwrap();
    assert!(store.decide_approval("a", 0, false).await.is_err());
    let mut changed = action.clone();
    changed.argv.push("other-remote".into());
    assert!(!store.consume_approval("a", &changed).await.unwrap());
    assert!(store.consume_approval("a", &action).await.unwrap());
    assert!(!store.consume_approval("a", &action).await.unwrap());
    let events = store.list_events(&run.id, 0, 100).await.unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|e| e.kind.starts_with("approval."))
            .count(),
        3
    );
    let mut denied = action;
    denied.argv = vec!["rm".into(), "file".into()];
    denied.operation = Operation::Destructive;
    store
        .request_approval("b", &run.id, task.id(), 1, &denied)
        .await
        .unwrap();
    store.decide_approval("b", 0, false).await.unwrap();
    assert!(!store.consume_approval("b", &denied).await.unwrap());
}
