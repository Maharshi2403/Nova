use harness_core::domain::*;
use harness_runtime::{
    engine::{Engine, default_python},
    git::git,
};
use harness_store::{
    Store, git_artifacts::GitArtifactRepository, plans::PlanRepository, repository::Repository,
    verification::VerificationRepository,
};
use std::{path::PathBuf, time::Duration};

#[tokio::test]
async fn real_worker_broker_verifier_commit_and_integration() {
    verify_workflow(false).await;
}
#[tokio::test]
async fn snapshot_execution_keeps_dirty_source_untouched() {
    verify_workflow(true).await;
}
async fn verify_workflow(snapshot: bool) {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]).await.unwrap();
    git(&repo, &["config", "user.name", "Harness Test"])
        .await
        .unwrap();
    git(&repo, &["config", "user.email", "test@example.test"])
        .await
        .unwrap();
    std::fs::write(repo.join("README.md"), "fixture\n").unwrap();
    git(&repo, &["add", "README.md"]).await.unwrap();
    git(&repo, &["commit", "-qm", "init"]).await.unwrap();
    let base_sha = git(&repo, &["rev-parse", "HEAD"])
        .await
        .unwrap()
        .trim()
        .to_owned();
    if snapshot {
        std::fs::write(repo.join("README.md"), "unfinished tracked work\n").unwrap();
        std::fs::write(
            repo.join("private-draft.txt"),
            "unfinished untracked work\n",
        )
        .unwrap();
    }
    let before = git(&repo, &["status", "--porcelain=v1"]).await.unwrap();
    let store = Store::open(dir.path().join("db")).await.unwrap();
    let p = Project::new(
        ProjectId::new("proj-demo").unwrap(),
        "fixture",
        repo.to_str().unwrap(),
    )
    .unwrap();
    store.create_project(&p).await.unwrap();
    assert!(!store.project_is_trusted(&p.id).await.unwrap());
    store.set_project_trust(&p.id, true).await.unwrap();
    let run = Run::new(RunId::new("r").unwrap(), p.id.clone());
    store.create_run(&run).await.unwrap();
    let python =
        default_python(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../intelligence/.venv"));
    let python = std::path::absolute(python).unwrap();
    let command = vec![python.to_str().unwrap().into(), "-c".into(), "from pathlib import Path; assert Path('output.txt').read_text().strip() == 'Mock task task-model'; assert Path('README.md').read_text().strip() == 'fixture'".into()];
    let mut plan: harness_core::plan::PlanProposal = serde_json::from_str(include_str!(
        "../../../tests/fixtures/plans/valid_plan.json"
    ))
    .unwrap();
    plan.tasks.truncate(1);
    plan.tasks[0].candidate_paths = vec!["output.txt".into()];
    plan.tasks[0].acceptance = vec![Acceptance::Test {
        command: command.clone(),
    }];
    store.propose_plan(&run.id, &plan).await.unwrap();
    store.approve_plan(&run.id, 1).await.unwrap();
    let engine = Engine::new(dir.path().join("execution"));
    if snapshot {
        assert!(
            engine
                .launch(
                    store.clone(),
                    run.id.clone(),
                    python.clone(),
                    vec![command.clone()],
                    2
                )
                .await
                .is_err()
        );
        assert!(store.execution_source(&run.id).await.unwrap().is_none());
        assert!(
            store
                .list_tasks(&run.id)
                .await
                .unwrap()
                .iter()
                .all(|t| t.attempt == 0)
        );
        store.set_project_trust(&p.id, false).await.unwrap();
        assert!(
            engine
                .launch_configured(
                    store.clone(),
                    run.id.clone(),
                    python.clone(),
                    vec![command.clone()],
                    2,
                    harness_runtime::engine::LaunchMode {
                        live: None,
                        committed_snapshot: true
                    }
                )
                .await
                .is_err()
        );
        store.set_project_trust(&p.id, true).await.unwrap();
    }
    engine
        .launch_configured(
            store.clone(),
            run.id.clone(),
            python,
            vec![command],
            2,
            harness_runtime::engine::LaunchMode {
                live: None,
                committed_snapshot: snapshot,
            },
        )
        .await
        .unwrap();
    assert!(
        !engine.prepare_shutdown(),
        "shutdown must not interrupt an active run"
    );
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let tasks = store.list_tasks(&run.id).await.unwrap();
            assert!(
                !tasks.iter().any(|t| t.state == TaskState::Blocked),
                "task blocked: {tasks:?}"
            );
            if store.get_run(&run.id).await.unwrap().state == RunState::Completed {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    let id = TaskId::new("task-model").unwrap();
    assert!(
        store
            .get_task_commit(&run.id, &id)
            .await
            .unwrap()
            .unwrap()
            .integration_commit_sha
            .is_some()
    );
    assert_eq!(
        store.list_test_results(&run.id, &id).await.unwrap()[0].exit_code,
        Some(0)
    );
    assert_eq!(store.list_usage(&run.id, None).await.unwrap().len(), 1);
    assert!(!repo.join("output.txt").exists());
    assert_eq!(
        git(&repo, &["status", "--porcelain=v1"]).await.unwrap(),
        before
    );
    let source = store.execution_source(&run.id).await.unwrap().unwrap();
    assert_eq!(source.base_sha, base_sha);
    assert_eq!(source.committed_snapshot, snapshot);
    assert_eq!(
        store
            .pin_execution_source(
                &run.id,
                &harness_store::execution::ExecutionSource {
                    base_sha: "a".repeat(40),
                    committed_snapshot: false
                }
            )
            .await
            .unwrap(),
        source
    );
    let events = store.list_events(&run.id, 0, 1000).await.unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|e| e.kind == "execution.source")
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| e.kind == "execution.launch")
            .count(),
        1
    );
    if snapshot {
        assert_eq!(
            std::fs::read_to_string(repo.join("README.md")).unwrap(),
            "unfinished tracked work\n"
        );
        assert_eq!(
            std::fs::read_to_string(repo.join("private-draft.txt")).unwrap(),
            "unfinished untracked work\n"
        );
    }
}
