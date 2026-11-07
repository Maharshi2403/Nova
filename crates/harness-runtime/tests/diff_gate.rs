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

#[tokio::test]
async fn secret_detection_table() {
    let repo = fixture().await;
    let task = task(Risk::Low);
    let gate = DiffGate::new(&task.spec.candidate_paths).unwrap();
    let cases = [
        ("DB_PASSWORD = \"aB3dE5gH7jK9mN1pQ2sT4vW6\"", true),
        ("password: Optional[str]", false),
        (
            "password_description = \"Describe the configured authentication process\"",
            false,
        ),
        ("password = os.environ[\"PASSWORD\"]", false),
        ("secret = env::var(\"SECRET_KEY\")", false),
        ("api_key = \"changeme\"", false),
        ("password = \"<your-long-application-password>\"", false),
        ("password = \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"", false),
        ("password = \"abcdabcdabcdabcdabcdabcd\"", false),
        ("api_key = None", false),
        ("secret: str | None = None", false),
        ("token = \"short\"", false),
        ("sk-A1b2C3d4E5f6G7h8I9j0K1l2", true),
        ("AKIA1234567890ABCDEF", true),
        ("ghp_abcdefghijklmnopqrstuvwxyz0123456789", true),
        ("xoxb-1234567890-ABCdefGHIjkl", true),
        ("-----BEGIN PRIVATE KEY-----", true),
        ("-----BEGIN RSA PRIVATE KEY-----", true),
        (
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJmaXh0dXJlIn0.abcdefghijklmnopqrstuv",
            true,
        ),
        ("password = \"aB3dE5gH7jK9mN1pQ2sT4vW6\"", true),
        ("api_key: str = 'aB3dE5gH7jK9mN1pQ2sT4vW6'", true),
        ("{\"secret\":\"aB3dE5gH7jK9mN1pQ2sT4vW6\"}", true),
    ];
    for (index, (contents, expected)) in cases.iter().enumerate() {
        std::fs::write(repo.path().join("allowed.txt"), format!("{contents}\n")).unwrap();
        let result = gate.check(&task, repo.path()).await;
        assert_eq!(
            matches!(
                result,
                Err(harness_runtime::diff_gate::DiffGateError::SecretDetected(_))
            ),
            *expected,
            "case {index}"
        );
    }
}

#[tokio::test]
async fn finding_never_echoes_secret() {
    let repo = fixture().await;
    let secret = "sk-A1b2C3d4E5f6G7h8I9j0K1l2";
    std::fs::write(repo.path().join("allowed.txt"), format!("safe\n{secret}\n")).unwrap();
    let task = task(Risk::Low);
    let err = DiffGate::new(&task.spec.candidate_paths)
        .unwrap()
        .check(&task, repo.path())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("allowed.txt:2"));
    assert!(!err.to_string().contains(secret));
}
