use harness_core::messages::{ToolRequest, ToolResponse};
use harness_runtime::broker::read::{MAX_OUTPUT_BYTES, RepoReadBroker};
use harness_runtime::supervisor::ToolBroker;
use serde_json::{Value, json};
use tempfile::TempDir;

struct Fx {
    _outer: TempDir,
    wt: std::path::PathBuf,
    broker: RepoReadBroker,
}

fn fx() -> Fx {
    let outer = TempDir::new().unwrap();
    let wt = outer.path().join("wt");
    std::fs::create_dir_all(wt.join("src")).unwrap();
    std::fs::create_dir_all(wt.join(".git")).unwrap();
    std::fs::write(
        wt.join("src/main.py"),
        "def main():\n    return 'hello'\n# TODO later\n",
    )
    .unwrap();
    std::fs::write(wt.join("README.md"), "# readme\nhello world\n").unwrap();
    std::fs::write(wt.join(".env"), "API_KEY=supersecret\n").unwrap();
    std::fs::write(wt.join("server.pem"), "-----BEGIN-----").unwrap();
    std::fs::write(wt.join(".git/config"), "[core]\n").unwrap();
    std::fs::write(outer.path().join("outside.txt"), "outside secret").unwrap();
    let tools = ["repo.read", "repo.glob", "repo.search"].map(String::from);
    let broker = RepoReadBroker::new(&wt, &tools).unwrap();
    Fx {
        _outer: outer,
        wt,
        broker,
    }
}

fn req(tool: &str, args: Value) -> ToolRequest {
    ToolRequest {
        tool: tool.into(),
        arguments: args.as_object().unwrap().clone(),
    }
}

async fn call(b: &mut RepoReadBroker, tool: &str, args: Value) -> ToolResponse {
    b.handle(&req(tool, args)).await
}

fn err(r: &ToolResponse) -> &str {
    assert!(!r.ok, "expected failure, got {r:?}");
    r.error.as_deref().unwrap()
}

#[tokio::test]
async fn read_within_worktree() {
    let mut f = fx();
    let r = call(&mut f.broker, "repo.read", json!({"path": "src/main.py"})).await;
    assert!(r.ok, "{r:?}");
    let v = r.result.unwrap();
    assert_eq!(
        v["content"],
        "def main():\n    return 'hello'\n# TODO later"
    );
    assert_eq!(v["truncated"], false);
    assert_eq!(v["start_line"], 1);
    assert_eq!(v["end_line"], 3);

    let r = call(
        &mut f.broker,
        "repo.read",
        json!({"path": "src/main.py", "start_line": 2, "max_lines": 1}),
    )
    .await;
    let v = r.result.unwrap();
    assert_eq!(v["content"], "    return 'hello'");
    assert_eq!(v["truncated"], true);

    let r = call(&mut f.broker, "repo.glob", json!({"pattern": "**/*.py"})).await;
    assert_eq!(r.result.unwrap()["paths"], json!(["src/main.py"]));

    let r = call(&mut f.broker, "repo.search", json!({"pattern": "hello"})).await;
    let v = r.result.unwrap();
    let paths: Vec<&str> = v["matches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths.len(), 2);
    assert!(paths.contains(&"README.md") && paths.contains(&"src/main.py"));
}

#[tokio::test]
async fn read_outside_denied() {
    let mut f = fx();
    for path in [
        "../outside.txt",
        "src/../../outside.txt",
        "/etc/passwd",
        ".git/config",
        ".env",
        "server.pem",
        "src/../.env",
    ] {
        let r = call(&mut f.broker, "repo.read", json!({"path": path})).await;
        assert!(err(&r).starts_with("error: path_denied"), "{path}: {r:?}");
    }
    #[cfg(windows)]
    for path in ["C:\\Windows\\win.ini", "C:foo", "\\\\server\\share\\x"] {
        let r = call(&mut f.broker, "repo.read", json!({"path": path})).await;
        assert!(err(&r).starts_with("error: path_denied"), "{path}: {r:?}");
    }
    let abs = f.wt.parent().unwrap().join("outside.txt");
    let r = call(
        &mut f.broker,
        "repo.read",
        json!({"path": abs.to_str().unwrap()}),
    )
    .await;
    assert!(err(&r).contains("path_denied"));
    let r = call(&mut f.broker, "repo.read", json!({"path": "missing.txt"})).await;
    assert!(err(&r).contains("not_found"));
}

#[tokio::test]
async fn protected_files_hidden_from_glob_and_search() {
    let mut f = fx();
    let r = call(&mut f.broker, "repo.glob", json!({"pattern": "**"})).await;
    let v = r.result.unwrap();
    let all = v["paths"].to_string();
    assert!(
        !all.contains(".env") && !all.contains(".pem") && !all.contains(".git/"),
        "{all}"
    );
    let r = call(
        &mut f.broker,
        "repo.search",
        json!({"pattern": "supersecret|BEGIN|core"}),
    )
    .await;
    assert_eq!(r.result.unwrap()["matches"], json!([]));
}

#[cfg(unix)]
#[tokio::test]
async fn symlink_escape_denied() {
    let mut f = fx();
    let outside = f.wt.parent().unwrap().join("outside.txt");
    std::os::unix::fs::symlink(&outside, f.wt.join("link.txt")).unwrap();
    let r = call(&mut f.broker, "repo.read", json!({"path": "link.txt"})).await;
    assert!(err(&r).contains("path_denied"));
}

#[tokio::test]
async fn large_read_truncated() {
    let mut f = fx();
    let big: String = (0..5000)
        .map(|i| format!("line number {i} {}\n", "x".repeat(60)))
        .collect();
    std::fs::write(f.wt.join("big.txt"), &big).unwrap();
    let r = call(
        &mut f.broker,
        "repo.read",
        json!({"path": "big.txt", "max_lines": 100000}),
    )
    .await;
    let v = r.result.unwrap();
    assert_eq!(v["truncated"], true);
    assert!(v["content"].as_str().unwrap().len() <= MAX_OUTPUT_BYTES);

    // A file bigger than the scan limit is flagged even when few lines are requested.
    std::fs::write(f.wt.join("huge.txt"), "a\n".repeat(700_000)).unwrap();
    let r = call(
        &mut f.broker,
        "repo.read",
        json!({"path": "huge.txt", "max_lines": 5}),
    )
    .await;
    assert_eq!(r.result.unwrap()["truncated"], true);

    std::fs::write(f.wt.join("blob.bin"), [0u8, 1, 2, 3]).unwrap();
    let r = call(&mut f.broker, "repo.read", json!({"path": "blob.bin"})).await;
    assert!(err(&r).contains("binary_file"));
}

#[tokio::test]
async fn results_are_capped() {
    let mut f = fx();
    for i in 0..600 {
        std::fs::write(f.wt.join(format!("f{i}.txt")), "needle\n").unwrap();
    }
    let r = call(&mut f.broker, "repo.glob", json!({"pattern": "*.txt"})).await;
    let v = r.result.unwrap();
    assert_eq!(v["paths"].as_array().unwrap().len(), 500);
    assert_eq!(v["truncated"], true);
    let r = call(
        &mut f.broker,
        "repo.search",
        json!({"pattern": "needle", "max_results": 10}),
    )
    .await;
    let v = r.result.unwrap();
    assert_eq!(v["matches"].as_array().unwrap().len(), 10);
    assert_eq!(v["truncated"], true);
}

#[tokio::test]
async fn tool_gating_and_bad_arguments() {
    let mut f = fx();
    let r = call(&mut f.broker, "repo.patch", json!({})).await;
    assert!(err(&r).starts_with("tool_not_allowed"));
    let r = call(&mut f.broker, "repo.read", json!({"path": "a", "extra": 1})).await;
    assert!(err(&r).starts_with("bad_arguments"));
    let r = call(&mut f.broker, "repo.search", json!({"pattern": "("})).await;
    assert!(err(&r).starts_with("bad_arguments"));
    let mut limited = RepoReadBroker::new(&f.wt, &["repo.glob".to_string()]).unwrap();
    let r = call(&mut limited, "repo.read", json!({"path": "README.md"})).await;
    assert!(err(&r).starts_with("tool_not_allowed"));
}
