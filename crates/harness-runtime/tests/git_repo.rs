use std::path::Path;

use harness_runtime::git::{RepoError, git, register_repo, validate_repo_path};
use tempfile::TempDir;

async fn init_repo(commit: bool) -> TempDir {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]).await.unwrap();
    if commit {
        std::fs::write(p.join("a.txt"), "one\n").unwrap();
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
    }
    dir
}

fn s(p: &Path) -> String {
    p.to_str().unwrap().to_owned()
}

#[tokio::test]
async fn registers_clean_repo() {
    let dir = init_repo(true).await;
    let repo = register_repo(&s(dir.path())).await.unwrap();
    assert_eq!(repo.branch.as_deref(), Some("main"));
    assert_eq!(repo.head_sha.len(), 40);
    assert!(repo.root.is_absolute());
}

#[tokio::test]
async fn rejects_non_git_repo() {
    let dir = TempDir::new().unwrap();
    assert!(matches!(
        register_repo(&s(dir.path())).await,
        Err(RepoError::NotGit(_))
    ));
}

#[tokio::test]
async fn dirty_repo_not_reset() {
    let dir = init_repo(true).await;
    let file = dir.path().join("a.txt");
    std::fs::write(&file, "edited\n").unwrap();
    std::fs::write(dir.path().join("new.txt"), "untracked\n").unwrap();
    let err = register_repo(&s(dir.path())).await.unwrap_err();
    let RepoError::Dirty { entries } = err else {
        panic!("expected Dirty, got {err:?}");
    };
    assert_eq!(entries.len(), 2);
    // The user's edits and untracked file are exactly as they were.
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "edited\n");
    assert!(dir.path().join("new.txt").exists());
}

#[tokio::test]
async fn repo_without_commits_is_rejected() {
    let dir = init_repo(false).await;
    assert_eq!(
        register_repo(&s(dir.path())).await.unwrap_err(),
        RepoError::NoCommits
    );
}

#[tokio::test]
async fn repo_path_validation() {
    assert_eq!(validate_repo_path("").unwrap_err(), RepoError::InvalidPath);
    assert_eq!(
        validate_repo_path("a\0b").unwrap_err(),
        RepoError::InvalidPath
    );
    assert!(matches!(
        validate_repo_path("relative/path"),
        Err(RepoError::NotAbsolute(_))
    ));
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("missing");
    assert!(matches!(
        validate_repo_path(&s(&missing)),
        Err(RepoError::NotFound(_))
    ));
    let file = dir.path().join("f.txt");
    std::fs::write(&file, "x").unwrap();
    assert!(matches!(
        validate_repo_path(&s(&file)),
        Err(RepoError::NotDirectory(_))
    ));
    // `..` segments are resolved to a canonical path.
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    let dotted = dir.path().join("sub").join("..");
    let canonical = validate_repo_path(&s(&dotted)).unwrap();
    assert_eq!(canonical, validate_repo_path(&s(dir.path())).unwrap());
    assert!(!canonical.to_string_lossy().contains(".."));
}

#[tokio::test]
async fn subdirectory_is_not_a_repo_root() {
    let dir = init_repo(true).await;
    std::fs::create_dir(dir.path().join("pkg")).unwrap();
    assert!(matches!(
        register_repo(&s(&dir.path().join("pkg"))).await,
        Err(RepoError::NotRepoRoot { .. })
    ));
}
