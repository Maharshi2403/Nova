use harness_core::domain::{
    Acceptance, Complexity, Priority, Project, ProjectId, Risk, Run, RunId, Task, TaskId,
    TaskLimits, TaskSpec, TaskState,
};
use harness_runtime::committer::{CommitError, TaskCommitter};
use harness_runtime::diff_gate::DiffGate;
use harness_runtime::git::git;
use harness_runtime::verifier::Verifier;
use harness_store::Store;
use harness_store::repository::Repository;
use harness_store::verification::VerificationRepository;
use tempfile::TempDir;

struct Fixture {
    _root: TempDir,
    repo: std::path::PathBuf,
    store: Store,
    task: Task,
}

async fn fixture(command: Vec<String>) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let repo = root.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]).await.unwrap();
    std::fs::write(repo.join("allowed.txt"), "base\n").unwrap();
    git(&repo, &["add", "allowed.txt"]).await.unwrap();
    git(&repo, &["config", "user.name", "Harness Test"])
        .await
        .unwrap();
    git(&repo, &["config", "user.email", "harness@example.test"])
        .await
        .unwrap();
    git(&repo, &["commit", "-q", "-m", "init"]).await.unwrap();

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
            use_case: Default::default(),
            id: TaskId::new("task-1").unwrap(),
            title: "Commit verified change".into(),
            description: "Commit only a validated task change.".into(),
            complexity: Complexity::Low,
            risk: Risk::Low,
            agent_priority: Priority::new(1).unwrap(),
            manual_priority: None,
            dependencies: vec![],
            candidate_paths: vec!["allowed.txt".into()],
            acceptance: vec![Acceptance::Test { command }],
            limits: TaskLimits {
                max_tool_calls: 1,
                max_output_tokens: 1,
                timeout_seconds: 30,
                cost_cap_usd: 0.0,
            },
        },
        state: TaskState::Verifying,
        revision: 0,
        attempt: 1,
    };
    store.insert_task(&task).await.unwrap();
    Fixture {
        _root: root,
        repo,
        store,
        task,
    }
}

#[tokio::test]
async fn failed_checks_create_no_commit() {
    let fixture = fixture(vec!["git".into(), "not-a-command".into()]).await;
    std::fs::write(fixture.repo.join("allowed.txt"), "changed\n").unwrap();
    let verifier = Verifier::new(vec![vec!["git".into()]]);
    let gate = DiffGate::new(&fixture.task.spec.candidate_paths).unwrap();
    let err = TaskCommitter::new(&verifier, &gate)
        .commit(&fixture.store, &fixture.task, &fixture.repo)
        .await
        .unwrap_err();

    assert!(matches!(err, CommitError::VerificationFailed));
    assert_eq!(
        git(&fixture.repo, &["rev-list", "--count", "HEAD"])
            .await
            .unwrap()
            .trim(),
        "1"
    );
    assert_eq!(
        fixture
            .store
            .list_test_results(&fixture.task.run_id, fixture.task.id())
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn retry_does_not_double_commit() {
    let fixture = fixture(vec!["git".into(), "--version".into()]).await;
    std::fs::write(fixture.repo.join("allowed.txt"), "changed\n").unwrap();
    let verifier = Verifier::new(vec![vec!["git".into()]]);
    let gate = DiffGate::new(&fixture.task.spec.candidate_paths).unwrap();
    let committer = TaskCommitter::new(&verifier, &gate);

    let first = committer
        .commit(&fixture.store, &fixture.task, &fixture.repo)
        .await
        .unwrap();
    let retry = committer
        .commit(&fixture.store, &fixture.task, &fixture.repo)
        .await
        .unwrap();
    assert!(!first.replayed);
    assert!(retry.replayed);
    assert_eq!(first.sha, retry.sha);
    assert_eq!(
        git(&fixture.repo, &["rev-list", "--count", "HEAD"])
            .await
            .unwrap()
            .trim(),
        "2"
    );
}

#[tokio::test]
async fn kill_restart_no_duplicate_commit() {
    use harness_store::git_artifacts::GitArtifactRepository;
    let f = fixture(vec!["git".into(), "--version".into()]).await;
    std::fs::write(f.repo.join("allowed.txt"), "recovered change\n").unwrap();
    git(&f.repo, &["add", "allowed.txt"]).await.unwrap();
    git(
        &f.repo,
        &[
            "commit",
            "-qm",
            "interrupted",
            "-m",
            "Harness-Task: run-1/task-1",
            "-m",
            "Harness-Attempt: 1",
        ],
    )
    .await
    .unwrap();
    let verifier = Verifier::new(vec![vec!["git".into(), "--version".into()]]);
    let gate = DiffGate::new(&f.task.spec.candidate_paths).unwrap();
    let result = TaskCommitter::new(&verifier, &gate)
        .commit(&f.store, &f.task, &f.repo)
        .await
        .unwrap();
    assert!(result.replayed);
    assert_eq!(
        git(&f.repo, &["rev-list", "--count", "HEAD"])
            .await
            .unwrap()
            .trim(),
        "2"
    );
    assert_eq!(
        f.store
            .get_task_commit(&f.task.run_id, f.task.id())
            .await
            .unwrap()
            .unwrap()
            .diff_paths,
        vec!["allowed.txt"]
    );
    assert_eq!(
        f.store
            .list_test_results(&f.task.run_id, f.task.id())
            .await
            .unwrap()
            .len(),
        1
    );
}
