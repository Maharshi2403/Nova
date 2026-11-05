use std::path::PathBuf;

use harness_core::domain::{
    Acceptance, Complexity, Priority, Project, ProjectId, Risk, Run, RunId, Task, TaskId,
    TaskLimits, TaskSpec, TaskState,
};
use harness_runtime::git::git;
use harness_runtime::integration::{IntegrationError, Integrator};
use harness_store::Store;
use harness_store::git_artifacts::{GitArtifactRepository, TaskCommitRecord};
use harness_store::repository::Repository;
use tempfile::TempDir;

struct Fixture {
    _root: TempDir,
    repo: PathBuf,
    integration_base: PathBuf,
    store: Store,
    task: Task,
    task_sha: String,
}

#[tokio::test]
async fn parallel_integrator_instances_are_serialized() {
    let f = fixture().await;
    let a = Integrator::new(
        &f.repo,
        &f.integration_base,
        "run-1",
        "main",
        vec![vec!["git".into(), "--version".into()]],
    )
    .unwrap();
    let b = Integrator::new(
        &f.repo,
        &f.integration_base,
        "run-1",
        "main",
        vec![vec!["git".into(), "--version".into()]],
    )
    .unwrap();
    let (a, b) = tokio::join!(
        a.integrate(&f.store, &f.task),
        b.integrate(&f.store, &f.task)
    );
    assert_ne!(a.unwrap().replayed, b.unwrap().replayed);
    assert_eq!(
        f.store
            .get_task(&f.task.run_id, f.task.id())
            .await
            .unwrap()
            .state,
        TaskState::Completed
    );
}

async fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let repo = root.path().join("repo");
    let integration_base = root.path().join("integrations");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]).await.unwrap();
    git(&repo, &["config", "user.name", "Harness Test"])
        .await
        .unwrap();
    git(&repo, &["config", "user.email", "harness@example.test"])
        .await
        .unwrap();
    std::fs::write(repo.join("value.txt"), "base\n").unwrap();
    git(&repo, &["add", "value.txt"]).await.unwrap();
    git(&repo, &["commit", "-q", "-m", "init"]).await.unwrap();
    git(&repo, &["checkout", "-q", "-b", "task-branch"])
        .await
        .unwrap();
    std::fs::write(repo.join("value.txt"), "task change\n").unwrap();
    git(&repo, &["commit", "-am", "task change"]).await.unwrap();
    let task_sha = git(&repo, &["rev-parse", "HEAD"])
        .await
        .unwrap()
        .trim()
        .to_owned();
    git(&repo, &["checkout", "-q", "main"]).await.unwrap();

    let store = Store::open(root.path().join("harness.db")).await.unwrap();
    let project_id = ProjectId::new("project-1").unwrap();
    let run_id = RunId::new("run-1").unwrap();
    store
        .create_project(
            &Project::new(project_id.clone(), "demo", repo.display().to_string()).unwrap(),
        )
        .await
        .unwrap();
    store
        .create_run(&Run::new(run_id.clone(), project_id))
        .await
        .unwrap();
    let task = Task {
        run_id,
        spec: TaskSpec {
            id: TaskId::new("task-1").unwrap(),
            title: "Integrate change".into(),
            description: "Integrate a verified task commit.".into(),
            complexity: Complexity::Low,
            risk: Risk::Low,
            agent_priority: Priority::new(1).unwrap(),
            manual_priority: None,
            dependencies: vec![],
            candidate_paths: vec!["value.txt".into()],
            acceptance: vec![Acceptance::Test {
                command: vec!["git".into(), "--version".into()],
            }],
            limits: TaskLimits {
                max_tool_calls: 1,
                max_output_tokens: 1,
                timeout_seconds: 30,
                cost_cap_usd: 0.0,
            },
        },
        state: TaskState::Reviewing,
        revision: 0,
        attempt: 1,
    };
    store.insert_task(&task).await.unwrap();
    store
        .record_task_commit(&TaskCommitRecord {
            run_id: task.run_id.clone(),
            task_id: task.id().clone(),
            task_commit_sha: task_sha.clone(),
            integration_commit_sha: None,
            diff_paths: vec!["value.txt".into()],
        })
        .await
        .unwrap();
    Fixture {
        _root: root,
        repo,
        integration_base,
        store,
        task,
        task_sha,
    }
}

#[tokio::test]
async fn conflict_blocks_completion() {
    let fixture = fixture().await;
    std::fs::write(fixture.repo.join("value.txt"), "main change\n").unwrap();
    git(&fixture.repo, &["commit", "-am", "main change"])
        .await
        .unwrap();
    let integrator = Integrator::new(
        &fixture.repo,
        &fixture.integration_base,
        fixture.task.run_id.as_str(),
        "main",
        vec![vec!["git".into(), "--version".into()]],
    )
    .unwrap();

    let err = integrator
        .integrate(&fixture.store, &fixture.task)
        .await
        .unwrap_err();
    assert!(matches!(err, IntegrationError::Conflict(_)));
    assert_eq!(
        fixture
            .store
            .get_task(&fixture.task.run_id, fixture.task.id())
            .await
            .unwrap()
            .state,
        TaskState::Reviewing
    );
    assert_eq!(
        git(&fixture.repo, &["branch", "--show-current"])
            .await
            .unwrap()
            .trim(),
        "main"
    );
}

#[tokio::test]
async fn integration_failure_retains_branch() {
    let fixture = fixture().await;
    let integrator = Integrator::new(
        &fixture.repo,
        &fixture.integration_base,
        fixture.task.run_id.as_str(),
        "main",
        vec![vec!["git".into(), "not-a-command".into()]],
    )
    .unwrap();

    let err = integrator
        .integrate(&fixture.store, &fixture.task)
        .await
        .unwrap_err();
    assert!(matches!(err, IntegrationError::FinalCheckFailed(_)));
    assert!(
        git(
            &fixture.repo,
            &[
                "merge-base",
                "--is-ancestor",
                &fixture.task_sha,
                integrator.branch(),
            ],
        )
        .await
        .is_ok()
    );
    assert_eq!(
        fixture
            .store
            .get_task(&fixture.task.run_id, fixture.task.id())
            .await
            .unwrap()
            .state,
        TaskState::Reviewing
    );
}

#[tokio::test]
async fn success_records_sha() {
    let fixture = fixture().await;
    let integrator = Integrator::new(
        &fixture.repo,
        &fixture.integration_base,
        fixture.task.run_id.as_str(),
        "main",
        vec![vec!["git".into(), "--version".into()]],
    )
    .unwrap();

    let outcome = integrator
        .integrate(&fixture.store, &fixture.task)
        .await
        .unwrap();
    let record = fixture
        .store
        .get_task_commit(&fixture.task.run_id, fixture.task.id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        record.integration_commit_sha.as_deref(),
        Some(outcome.integration_sha.as_str())
    );
    assert_eq!(
        fixture
            .store
            .get_task(&fixture.task.run_id, fixture.task.id())
            .await
            .unwrap()
            .state,
        TaskState::Completed
    );
}
