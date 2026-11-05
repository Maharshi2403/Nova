use harness_core::domain::{
    Acceptance, Complexity, Priority, Risk, RunId, Task, TaskId, TaskLimits, TaskSpec, TaskState,
};
use harness_runtime::diff_gate::DiffGate;
use harness_runtime::git::git;
use tempfile::TempDir;

async fn fixture() -> TempDir {
    let repo = tempfile::tempdir().unwrap();
    git(repo.path(), &["init", "-q", "-b", "main"])
        .await
        .unwrap();
    std::fs::write(repo.path().join("allowed.txt"), "base\n").unwrap();
    git(repo.path(), &["add", "allowed.txt"]).await.unwrap();
    git(
        repo.path(),
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            "init",
        ],
    )
    .await
    .unwrap();
    repo
}

fn task(risk: Risk) -> Task {
    Task {
        run_id: RunId::new("run-1").unwrap(),
        spec: TaskSpec {
            use_case: Default::default(),
            id: TaskId::new("task-1").unwrap(),
            title: "Gate changes".into(),
            description: "Validate a constrained worktree diff.".into(),
            complexity: Complexity::Low,
            risk,
            agent_priority: Priority::new(1).unwrap(),
            manual_priority: None,
            dependencies: vec![],
            candidate_paths: vec!["allowed.txt".into()],
            acceptance: vec![Acceptance::Test {
                command: vec!["test".into()],
            }],
            limits: TaskLimits {
                max_tool_calls: 1,
                max_output_tokens: 1,
                timeout_seconds: 1,
                cost_cap_usd: 0.0,
            },
        },
        state: TaskState::Verifying,
        revision: 0,
        attempt: 1,
    }
}

#[tokio::test]
async fn unexpected_file_change_rejected() {
    let repo = fixture().await;
    std::fs::write(repo.path().join("unexpected.txt"), "nope\n").unwrap();

    let err = DiffGate::new(&task(Risk::Low).spec.candidate_paths)
        .unwrap()
        .check(&task(Risk::Low), repo.path())
        .await
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "changed path is outside the task scope: unexpected.txt"
    );
}

#[tokio::test]
async fn large_diff_requires_review() {
    let repo = fixture().await;
    std::fs::write(repo.path().join("allowed.txt"), "x".repeat(128)).unwrap();
    let task = task(Risk::Low);

    let result = DiffGate::with_max_diff_bytes(&task.spec.candidate_paths, 64)
        .unwrap()
        .check(&task, repo.path())
        .await
        .unwrap();
    assert_eq!(result.changed_paths, ["allowed.txt"]);
    assert!(result.changed_bytes >= 128);
    assert!(result.requires_review);
}

#[tokio::test]
async fn high_risk_change_requires_review() {
    let repo = fixture().await;
    std::fs::write(repo.path().join("allowed.txt"), "changed\n").unwrap();
    let task = task(Risk::High);

    let result = DiffGate::new(&task.spec.candidate_paths)
        .unwrap()
        .check(&task, repo.path())
        .await
        .unwrap();
    assert!(result.requires_review);
}
