use harness_core::domain::{Project, ProjectId, Run, RunId, RunState, TaskId, TaskState};
use harness_core::plan::PlanProposal;
use harness_core::state;
use harness_runtime::committer::TaskCommitter;
use harness_runtime::diff_gate::DiffGate;
use harness_runtime::git::{git, register_repo};
use harness_runtime::integration::Integrator;
use harness_runtime::priority::rank;
use harness_runtime::scheduler::tick;
use harness_runtime::verifier::Verifier;
use harness_runtime::worktree::WorktreeManager;
use harness_store::Store;
use harness_store::git_artifacts::GitArtifactRepository;
use harness_store::plans::PlanRepository;
use harness_store::repository::Repository;

const PLAN: &str = r#"{
  "schema_version": "1.0",
  "project_id": "project-1",
  "idea": "Build an offline fixture through the complete harness workflow.",
  "assumptions": [],
  "requirements": ["The fixture must become integrated."],
  "architecture": {"recommended": "Use a bounded task worktree and deterministic verification.", "alternatives": [], "risks": []},
  "tasks": [{
    "id": "task-1",
    "title": "Create fixture output",
    "description": "Create the task-owned fixture output.",
    "complexity": "low",
    "risk": "low",
    "agent_priority": 90,
    "manual_priority": null,
    "dependencies": [],
    "candidate_paths": ["output.txt"],
    "acceptance": [{"kind": "test", "command": ["git", "--version"]}],
    "limits": {"max_tool_calls": 1, "max_output_tokens": 1, "timeout_seconds": 30, "cost_cap_usd": 0.0}
  }]
}"#;

#[tokio::test]
async fn e2e_mock_project_completes() {
    let root = tempfile::tempdir().unwrap();
    let repo = root.path().join("repo");
    let worktrees = root.path().join("worktrees");
    let integration_dir = root.path().join("integration");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]).await.unwrap();
    git(&repo, &["config", "user.name", "Harness Test"])
        .await
        .unwrap();
    git(&repo, &["config", "user.email", "harness@example.test"])
        .await
        .unwrap();
    std::fs::write(repo.join("README.md"), "fixture\n").unwrap();
    git(&repo, &["add", "README.md"]).await.unwrap();
    git(&repo, &["commit", "-q", "-m", "init"]).await.unwrap();

    let trusted = register_repo(repo.to_str().unwrap()).await.unwrap();
    let store = Store::open(root.path().join("harness.db")).await.unwrap();
    let project_id = ProjectId::new("project-1").unwrap();
    let run_id = RunId::new("run-1").unwrap();
    store
        .create_project(
            &Project::new(project_id.clone(), "fixture", repo.display().to_string()).unwrap(),
        )
        .await
        .unwrap();
    store
        .create_run(&Run::new(run_id.clone(), project_id))
        .await
        .unwrap();
    store
        .propose_plan(
            &run_id,
            &serde_json::from_str::<PlanProposal>(PLAN).unwrap(),
        )
        .await
        .unwrap();
    store.approve_plan(&run_id, 1).await.unwrap();
    assert_eq!(
        store.get_run(&run_id).await.unwrap().state,
        RunState::Running
    );

    let ready = tick(&store, &run_id, 0).await.unwrap();
    let tasks = store.list_tasks(&run_id).await.unwrap();
    assert_eq!(rank(&tasks, &ready), vec![TaskId::new("task-1").unwrap()]);
    let mut task = tasks.into_iter().next().unwrap();
    assert_eq!(task.state, TaskState::Ready);
    for state in [
        TaskState::Leased,
        TaskState::Running,
        TaskState::Verifying,
        TaskState::Reviewing,
    ] {
        task = store
            .transition_task(&run_id, task.id(), task.revision, state, None)
            .await
            .unwrap()
            .task;
    }

    let manager = WorktreeManager::new(&trusted, &worktrees).unwrap();
    let worktree = manager
        .create(run_id.as_str(), task.id().as_str(), 1, &trusted.head_sha)
        .await
        .unwrap();
    // The mock coder's bounded output: only its task-owned path is modified.
    std::fs::write(worktree.path.join("output.txt"), "mock worker completed\n").unwrap();
    let verifier = Verifier::new(vec![vec!["git".into()]]);
    let gate = DiffGate::new(&task.spec.candidate_paths).unwrap();
    let committed = TaskCommitter::new(&verifier, &gate)
        .commit(&store, &task, &worktree.path)
        .await
        .unwrap();
    assert!(!committed.replayed);

    let integrator = Integrator::new(
        &repo,
        &integration_dir,
        run_id.as_str(),
        "main",
        vec![vec!["git".into(), "--version".into()]],
    )
    .unwrap();
    let integrated = integrator.integrate(&store, &task).await.unwrap();
    assert_eq!(
        store.get_task(&run_id, task.id()).await.unwrap().state,
        TaskState::Completed
    );
    assert_eq!(
        std::fs::read_to_string(integrator.worktree_path().join("output.txt"))
            .unwrap()
            .replace("\r\n", "\n"),
        "mock worker completed\n"
    );
    let artifact = store
        .get_task_commit(&run_id, task.id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        artifact.integration_commit_sha,
        Some(integrated.integration_sha)
    );
    assert!(!store.list_events(&run_id, 0, 100).await.unwrap().is_empty());
    assert!(state::is_allowed(
        TaskState::Reviewing,
        TaskState::Integrating
    ));
}
