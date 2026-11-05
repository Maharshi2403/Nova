use harness_core::messages::{ToolRequest, ToolResponse};
use harness_runtime::broker::write::RepoWriteBroker;
use harness_runtime::supervisor::ToolBroker;
use serde_json::{Value, json};
use tempfile::TempDir;

struct Fx {
    outer: TempDir,
    wt: std::path::PathBuf,
    broker: RepoWriteBroker,
}

fn fx() -> Fx {
    let outer = TempDir::new().unwrap();
    let wt = outer.path().join("wt");
    std::fs::create_dir_all(wt.join("src")).unwrap();
    std::fs::create_dir_all(wt.join(".git")).unwrap();
    std::fs::write(wt.join("src/app.py"), "x = 1\nx = 1b\n").unwrap();
    std::fs::write(wt.join("other.py"), "keep\n").unwrap();
    std::fs::write(wt.join(".git/config"), "[core]\n").unwrap();
    let tools = ["repo.read", "repo.patch"].map(String::from);
    let assigned = ["src/", "docs/notes.md", "tests/*.py"].map(String::from);
    let broker = RepoWriteBroker::new(&wt, &tools, &assigned).unwrap();
    Fx { outer, wt, broker }
}

async fn patch(b: &mut RepoWriteBroker, args: Value) -> ToolResponse {
    b.handle(&ToolRequest {
        tool: "repo.patch".into(),
        arguments: args.as_object().unwrap().clone(),
    })
    .await
}

fn err(r: &ToolResponse) -> &str {
    assert!(!r.ok, "expected failure, got {r:?}");
    r.error.as_deref().unwrap()
}

#[tokio::test]
async fn write_allowed_path() {
    let mut f = fx();
    let r = patch(
        &mut f.broker,
        json!({"path": "src/new/mod.py", "content": "y = 2\n"}),
    )
    .await;
    assert!(r.ok, "{r:?}");
    assert_eq!(
        std::fs::read_to_string(f.wt.join("src/new/mod.py")).unwrap(),
        "y = 2\n"
    );
    let r = patch(
        &mut f.broker,
        json!({"path": "docs/notes.md", "content": "n"}),
    )
    .await;
    assert!(r.ok, "{r:?}");
    let r = patch(
        &mut f.broker,
        json!({"path": "tests/test_a.py", "content": "t"}),
    )
    .await;
    assert!(r.ok, "{r:?}");
    let r = patch(
        &mut f.broker,
        json!({"path": "./src/app.py", "old": "x = 1\n", "new": "x = 9\n"}),
    )
    .await;
    assert!(r.ok, "{r:?}");
    assert_eq!(
        std::fs::read_to_string(f.wt.join("src/app.py")).unwrap(),
        "x = 9\nx = 1b\n"
    );
}

#[tokio::test]
async fn unassigned_path_rejected() {
    let mut f = fx();
    let r = patch(
        &mut f.broker,
        json!({"path": "other.py", "content": "pwned"}),
    )
    .await;
    assert!(err(&r).contains("path_not_assigned"));
    assert_eq!(
        std::fs::read_to_string(f.wt.join("other.py")).unwrap(),
        "keep\n"
    );
    let r = patch(
        &mut f.broker,
        json!({"path": "tests/sub/x.py", "content": "t"}),
    )
    .await;
    assert!(
        err(&r).contains("path_not_assigned"),
        "single * must not cross directories"
    );
}

#[tokio::test]
async fn parent_traversal_rejected() {
    let mut f = fx();
    for path in [
        "../escape.txt",
        "src/../../escape.txt",
        "src/../other.py",
        "/tmp/abs.txt",
    ] {
        let r = patch(&mut f.broker, json!({"path": path, "content": "x"})).await;
        assert!(!r.ok, "{path} should fail");
    }
    assert!(!f.outer.path().join("escape.txt").exists());
    assert_eq!(
        std::fs::read_to_string(f.wt.join("other.py")).unwrap(),
        "keep\n"
    );
}

#[tokio::test]
async fn git_internal_write_rejected() {
    let mut f = fx();
    let mut wide = RepoWriteBroker::new(
        &f.wt,
        &["repo.patch".to_string()],
        &[".git/**".to_string(), "**".to_string(), ".env".to_string()],
    )
    .unwrap();
    for path in [
        ".git/config",
        ".git/hooks/pre-commit",
        "src/.git/x",
        ".env",
        "src/server.pem",
    ] {
        let r = patch(&mut wide, json!({"path": path, "content": "x"})).await;
        assert!(err(&r).contains("path_denied"), "{path}: {r:?}");
    }
    assert_eq!(
        std::fs::read_to_string(f.wt.join(".git/config")).unwrap(),
        "[core]\n"
    );
    assert!(!f.wt.join(".git/hooks").exists());
    let _ = &mut f.broker;
}

#[cfg(unix)]
#[tokio::test]
async fn symlink_write_rejected() {
    let mut f = fx();
    let target = f.outer.path().join("outside.txt");
    std::fs::write(&target, "orig").unwrap();
    std::os::unix::fs::symlink(&target, f.wt.join("src/link.py")).unwrap();
    let r = patch(
        &mut f.broker,
        json!({"path": "src/link.py", "content": "x"}),
    )
    .await;
    assert!(err(&r).contains("path_denied"));
    std::os::unix::fs::symlink(f.outer.path(), f.wt.join("src/dir")).unwrap();
    let r = patch(
        &mut f.broker,
        json!({"path": "src/dir/new.txt", "content": "x"}),
    )
    .await;
    assert!(err(&r).contains("path_denied"));
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "orig");
}

#[tokio::test]
async fn replacement_edge_cases_and_limits() {
    let mut f = fx();
    let r = patch(
        &mut f.broker,
        json!({"path": "src/app.py", "old": "x = ", "new": "z = "}),
    )
    .await;
    assert!(err(&r).starts_with("ambiguous_match"));
    let r = patch(
        &mut f.broker,
        json!({"path": "src/app.py", "old": "nope", "new": "z"}),
    )
    .await;
    assert!(err(&r).starts_with("no_match"));
    let r = patch(
        &mut f.broker,
        json!({"path": "src/app.py", "old": "x", "content": "z"}),
    )
    .await;
    assert!(err(&r).starts_with("bad_arguments"));
    let r = patch(&mut f.broker, json!({"path": "src/app.py"})).await;
    assert!(err(&r).starts_with("bad_arguments"));
    let big = "a".repeat(1024 * 1024 + 1);
    let r = patch(
        &mut f.broker,
        json!({"path": "src/big.txt", "content": big}),
    )
    .await;
    assert!(err(&r).starts_with("too_large"));
    assert!(!f.wt.join("src/big.txt").exists());
    assert_eq!(
        std::fs::read_to_string(f.wt.join("src/app.py")).unwrap(),
        "x = 1\nx = 1b\n"
    );
}

#[tokio::test]
async fn patch_requires_tool_grant_and_reads_still_work() {
    let f = fx();
    let mut ro =
        RepoWriteBroker::new(&f.wt, &["repo.read".to_string()], &["src".to_string()]).unwrap();
    let r = patch(&mut ro, json!({"path": "src/app.py", "content": "x"})).await;
    assert!(err(&r).starts_with("tool_not_allowed"));
    let r = ro
        .handle(&ToolRequest {
            tool: "repo.read".into(),
            arguments: json!({"path": "src/app.py"}).as_object().unwrap().clone(),
        })
        .await;
    assert!(r.ok);
}
