use std::path::Path;

use harness_runtime::git::{git, register_repo};
use harness_runtime::worktree::{WorktreeError, WorktreeManager};
use tempfile::TempDir;

struct Fixture {
    repo: TempDir,
    work: TempDir,
    mgr: WorktreeManager,
    sha: String,
}

async fn fixture() -> Fixture {
    let repo = TempDir::new().unwrap();
    let p = repo.path();
    git(p, &["init", "-q", "-b", "main"]).await.unwrap();
    std::fs::write(p.join("a.txt"), "base\n").unwrap();
    git(p, &["add", "a.txt"]).await.unwrap();
    git(
        p,
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
    let trusted = register_repo(p.to_str().unwrap()).await.unwrap();
    let work = TempDir::new().unwrap();
    let mgr = WorktreeManager::new(&trusted, work.path()).unwrap();
    Fixture {
        repo,
        work,
        mgr,
        sha: trusted.head_sha,
    }
}

fn p(path: &Path) -> &str {
    path.to_str().unwrap()
}

#[tokio::test]
async fn worktrees_isolated() {
    let f = fixture().await;
    let w1 = f.mgr.create("run-1", "task-a", 1, &f.sha).await.unwrap();
    let w2 = f.mgr.create("run-1", "task-b", 1, &f.sha).await.unwrap();
    assert_ne!(w1.path, w2.path);
    assert_ne!(w1.branch, w2.branch);
    assert_eq!(w1.branch, "harness/run-1/task-a-a1");
    assert!(w1.path.exists() && w2.path.exists());

    std::fs::write(w1.path.join("a.txt"), "changed in task a\n").unwrap();
    std::fs::write(w1.path.join("only-a.txt"), "x").unwrap();

    assert_eq!(
        std::fs::read_to_string(w2.path.join("a.txt")).unwrap(),
        "base\n"
    );
    assert!(!w2.path.join("only-a.txt").exists());
    // The user's checkout is untouched: same content, same branch, still clean.
    assert_eq!(
        std::fs::read_to_string(f.repo.path().join("a.txt")).unwrap(),
        "base\n"
    );
    let status = git(f.repo.path(), &["status", "--porcelain"])
        .await
        .unwrap();
    assert!(status.trim().is_empty(), "{status}");
    let head = git(f.repo.path(), &["symbolic-ref", "--short", "HEAD"])
        .await
        .unwrap();
    assert_eq!(head.trim(), "main");

    let listed = f.mgr.list().await.unwrap();
    assert_eq!(listed.len(), 3);
    assert_eq!(listed.iter().filter(|l| l.owned).count(), 2);
}

#[tokio::test]
async fn cleanup_never_deletes_user_worktree() {
    let f = fixture().await;
    let user_dir = TempDir::new().unwrap();
    let user_wt = user_dir.path().join("my-feature");
    git(
        f.repo.path(),
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "my-feature",
            p(&user_wt),
            &f.sha,
        ],
    )
    .await
    .unwrap();
    std::fs::write(user_wt.join("precious.txt"), "keep me").unwrap();

    assert!(matches!(
        f.mgr.cleanup(&user_wt, true).await,
        Err(WorktreeError::NotOwned(_))
    ));
    // Neither the main checkout nor a path outside the registry can be removed.
    assert!(matches!(
        f.mgr.cleanup(f.repo.path(), true).await,
        Err(WorktreeError::NotOwned(_))
    ));
    assert!(user_wt.join("precious.txt").exists());
    assert!(f.repo.path().join("a.txt").exists());

    let mine = f.mgr.create("run-1", "task-a", 1, &f.sha).await.unwrap();
    f.mgr.cleanup(&mine.path, true).await.unwrap();
    assert!(!mine.path.exists());
    let branches = git(f.repo.path(), &["branch", "--list"]).await.unwrap();
    assert!(!branches.contains("harness/run-1/task-a-a1"));
    assert!(branches.contains("my-feature"));
    assert!(user_wt.join("precious.txt").exists());
}

#[tokio::test]
async fn tampered_registry_cannot_escape_base_dir() {
    let f = fixture().await;
    let victim = TempDir::new().unwrap();
    std::fs::write(victim.path().join("keep.txt"), "x").unwrap();
    let forged = serde_json::json!([{
        "run_id": "r", "task_id": "t", "attempt": 1,
        "path": victim.path(), "branch": "harness/r/t-a1", "base_sha": f.sha
    }]);
    std::fs::write(f.work.path().join("worktrees.json"), forged.to_string()).unwrap();
    assert!(matches!(
        f.mgr.cleanup(victim.path(), true).await,
        Err(WorktreeError::NotOwned(_))
    ));
    assert!(victim.path().join("keep.txt").exists());
}

#[tokio::test]
async fn create_validates_inputs() {
    let f = fixture().await;
    assert_eq!(
        f.mgr.create("../evil", "t", 1, &f.sha).await.unwrap_err(),
        WorktreeError::BadId("run id")
    );
    assert_eq!(
        f.mgr.create("r", "a/b", 1, &f.sha).await.unwrap_err(),
        WorktreeError::BadId("task id")
    );
    assert_eq!(
        f.mgr.create("r", "t", 1, "main").await.unwrap_err(),
        WorktreeError::BadBaseSha
    );
    assert_eq!(
        f.mgr
            .create("r", "t", 1, &"0".repeat(40))
            .await
            .unwrap_err(),
        WorktreeError::BadBaseSha
    );
    f.mgr.create("r", "t", 1, &f.sha).await.unwrap();
    assert!(matches!(
        f.mgr.create("r", "t", 1, &f.sha).await.unwrap_err(),
        WorktreeError::AlreadyExists(_)
    ));
    // A new attempt gets a fresh branch and directory.
    f.mgr.create("r", "t", 2, &f.sha).await.unwrap();
}

#[tokio::test]
async fn base_dir_inside_repo_rejected() {
    let f = fixture().await;
    let trusted = register_repo(p(f.repo.path())).await.unwrap();
    let inside = f.repo.path().join("wt");
    assert!(matches!(
        WorktreeManager::new(&trusted, &inside),
        Err(WorktreeError::BaseInsideRepo)
    ));
}
