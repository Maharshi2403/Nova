use harness_runtime::{broker::paths::PathPolicy, locks::ResourceLocks};

#[test]
fn portable_windows_paths_denied_on_every_platform() {
    let dir = tempfile::tempdir().unwrap();
    let policy = PathPolicy::new(dir.path()).unwrap();
    for path in [
        "C:secret",
        "a:stream",
        ".git/config",
        ".git./config",
        ".git /config",
        "src\\..\\secret",
        "NUL.txt",
        "con",
        "COM1.log",
        "a\n.txt",
        ".aws/config",
        ".ssh/config",
        "../escape",
    ] {
        assert!(policy.check_relative(path).is_err(), "{path:?}");
    }
}

#[test]
fn unicode_path_normalization_and_lock_aliases() {
    let dir = tempfile::tempdir().unwrap();
    let policy = PathPolicy::new(dir.path()).unwrap();
    assert_eq!(
        policy.check_relative("cafe\u{301}.rs").unwrap(),
        std::path::PathBuf::from("café.rs")
    );
    let locks = ResourceLocks::default();
    let _guard = locks.try_acquire("p", &["café.rs".into()], false).unwrap();
    assert!(
        locks
            .try_acquire("p", &["cafe\u{301}.rs".into()], false)
            .is_none()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn atomic_write_does_not_modify_external_hardlink() {
    use harness_core::messages::ToolRequest;
    use harness_runtime::{broker::write::RepoWriteBroker, supervisor::ToolBroker};
    let outer = tempfile::tempdir().unwrap();
    let root = outer.path().join("root");
    std::fs::create_dir(&root).unwrap();
    let external = outer.path().join("outside.txt");
    std::fs::write(&external, "original").unwrap();
    std::fs::hard_link(&external, root.join("allowed.txt")).unwrap();
    let mut broker =
        RepoWriteBroker::new(&root, &["repo.patch".into()], &["allowed.txt".into()]).unwrap();
    assert!(
        broker
            .handle(&ToolRequest {
                tool: "repo.patch".into(),
                arguments: serde_json::json!({"path":"allowed.txt", "content":"updated"})
                    .as_object()
                    .unwrap()
                    .clone()
            })
            .await
            .ok
    );
    assert_eq!(std::fs::read_to_string(&external).unwrap(), "original");
}
