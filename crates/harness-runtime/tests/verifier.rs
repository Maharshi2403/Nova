use std::time::Duration;

use harness_core::domain::{
    Acceptance, Complexity, Priority, Project, ProjectId, Risk, Run, RunId, Task, TaskId,
    TaskLimits, TaskSpec,
};
use harness_runtime::verifier::Verifier;
use harness_store::Store;
use harness_store::repository::Repository;
use harness_store::verification::VerificationRepository;

async fn task_with_command(command: Vec<String>) -> (tempfile::TempDir, Store, Task) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("harness.db")).await.unwrap();
    let project_id = ProjectId::new("project-1").unwrap();
    let run_id = RunId::new("run-1").unwrap();
    store
        .create_project(&Project::new(project_id.clone(), "demo", "/tmp/demo").unwrap())
        .await
        .unwrap();
    store
        .create_run(&Run::new(run_id.clone(), project_id))
        .await
        .unwrap();
    let task = Task {
        run_id,
        spec: TaskSpec {
            use_case: Default::default(),
            id: TaskId::new("task-1").unwrap(),
            title: "Verify command".into(),
            description: "Run a declared test command.".into(),
            complexity: Complexity::Low,
            risk: Risk::Low,
            agent_priority: Priority::new(50).unwrap(),
            manual_priority: None,
            dependencies: vec![],
            candidate_paths: vec!["src/**".into()],
            acceptance: vec![Acceptance::Test { command }],
            limits: TaskLimits {
                max_tool_calls: 1,
                max_output_tokens: 1,
                timeout_seconds: 30,
                cost_cap_usd: 0.0,
            },
        },
        state: harness_core::domain::TaskState::Pending,
        revision: 0,
        attempt: 0,
    };
    store.insert_task(&task).await.unwrap();
    (dir, store, task)
}

#[tokio::test]
async fn failed_test_blocks_verification() {
    let binary = std::env::current_exe()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let command = vec![binary.clone(), "--not-a-real-test-argument".into()];
    let (dir, store, task) = task_with_command(command.clone()).await;
    let outcome = Verifier::with_timeout(vec![vec![binary]], Duration::from_secs(10))
        .verify(&store, &task, dir.path())
        .await
        .unwrap();

    assert!(!outcome.passed);
    assert_eq!(outcome.results.len(), 1);
    assert_ne!(outcome.results[0].exit_code, Some(0));
    let evidence = store
        .list_test_results(&task.run_id, task.id())
        .await
        .unwrap();
    assert_eq!(evidence, outcome.results);
    assert!(evidence[0].artifact.get("stderr").is_some());
}

#[tokio::test]
async fn successful_test_recorded() {
    let binary = std::env::current_exe()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let command = vec![binary.clone(), "--help".into()];
    let (dir, store, task) = task_with_command(command.clone()).await;
    let outcome = Verifier::with_timeout(vec![vec![binary]], Duration::from_secs(10))
        .verify(&store, &task, dir.path())
        .await
        .unwrap();

    assert!(outcome.passed);
    assert_eq!(outcome.results.len(), 1);
    assert_eq!(outcome.results[0].command, command);
    assert_eq!(outcome.results[0].exit_code, Some(0));
    assert!(outcome.results[0].duration_ms <= 10_000);
    assert!(outcome.results[0].artifact.get("stdout").is_some());
}
