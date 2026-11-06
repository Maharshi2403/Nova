use harness_runtime::context::{retrieve, retrieve_with};
#[tokio::test]
async fn context_avoids_entire_repo_and_obeys_cap() {
    let dir = tempfile::tempdir().unwrap();
    for n in 0..100 {
        std::fs::write(dir.path().join(format!("file{n}.rs")), "x".repeat(10_000)).unwrap();
    }
    let context = retrieve(dir.path(), &["file1.rs".into()], &[], 128)
        .await
        .unwrap();
    assert_eq!(context.snippets.len(), 1);
    assert_eq!(context.snippets[0].content.len(), 128);
    assert!(context.truncated);
}
#[tokio::test]
async fn symbol_located_and_secret_path_excluded() {
    let dir = tempfile::tempdir().unwrap();
    let text = format!("{}fn target_symbol() {{}}\n", "// padding\n".repeat(60));
    std::fs::write(dir.path().join("code.rs"), text).unwrap();
    std::fs::write(dir.path().join(".env"), "target_symbol=secret").unwrap();
    let context = retrieve(
        dir.path(),
        &["code.rs".into(), ".env".into(), "../outside".into()],
        &["target_symbol".into()],
        1024,
    )
    .await
    .unwrap();
    assert_eq!(context.snippets.len(), 1);
    assert!(context.snippets[0].content.contains("fn target_symbol"));
    assert!(context.snippets[0].start_line > 1);
}

#[tokio::test]
async fn symbol_found_without_ripgrep() {
    let dir = tempfile::tempdir().unwrap();
    let source = format!("{}fn fallback_symbol() {{}}\n", "// padding\n".repeat(80));
    std::fs::write(dir.path().join("code.rs"), source).unwrap();
    std::fs::write(dir.path().join(".env"), "fallback_symbol=secret").unwrap();
    let context = retrieve_with(
        dir.path(),
        &["code.rs".into(), ".env".into()],
        &["fallback_symbol".into()],
        1024,
        false,
    )
    .await
    .unwrap();
    assert_eq!(context.snippets.len(), 1);
    assert_eq!(context.snippets[0].path, "code.rs");
    assert!(context.snippets[0].content.contains("fn fallback_symbol"));
    assert!(context.snippets[0].start_line > 1);
}

#[tokio::test]
async fn fallback_finds_symbol_beyond_broker_scan_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let source = format!(
        "{}fn late_symbol() {{}}\n",
        "// padding line\n".repeat(70_000)
    );
    std::fs::write(dir.path().join("large.rs"), source).unwrap();
    let context = retrieve_with(dir.path(), &[], &["late_symbol".into()], 512, false)
        .await
        .unwrap();
    assert_eq!(context.snippets.len(), 1);
    assert!(context.snippets[0].content.contains("fn late_symbol"));
    assert!(context.snippets[0].content.len() <= 512);
}
