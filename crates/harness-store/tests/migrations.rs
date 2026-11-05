use harness_store::Store;
use sqlx::Row;
use tempfile::TempDir;

async fn fresh() -> (TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("harness.db")).await.unwrap();
    (dir, store)
}

async fn table_names(store: &Store) -> Vec<String> {
    sqlx::query(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' \
         AND name <> '_sqlx_migrations' ORDER BY name",
    )
    .fetch_all(store.pool())
    .await
    .unwrap()
    .iter()
    .map(|r| r.get::<String, _>(0))
    .collect()
}

#[tokio::test]
async fn fresh_db_migrates() {
    let (_dir, store) = fresh().await;
    assert_eq!(
        table_names(&store).await,
        [
            "approvals",
            "budget_reservations",
            "events",
            "git_artifacts",
            "leases",
            "plans",
            "project_trust",
            "projects",
            "provider_calls",
            "runs",
            "task_dependencies",
            "tasks",
            "test_results",
            "usage_records"
        ]
    );
    let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(mode, "wal");
    let timeout: i64 = sqlx::query_scalar("PRAGMA busy_timeout")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert!(timeout >= 5000);
}

#[tokio::test]
async fn migration_twice_idempotent() {
    let (dir, store) = fresh().await;
    store.migrate().await.unwrap();
    let applied: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
        .fetch_one(store.pool())
        .await
        .unwrap();
    store.close().await;

    // Reopening an existing database must not re-apply or fail.
    let again = Store::open(dir.path().join("harness.db")).await.unwrap();
    let applied_again: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
        .fetch_one(again.pool())
        .await
        .unwrap();
    assert_eq!(applied, applied_again);
    assert!(applied >= 1);
}

#[tokio::test]
async fn foreign_key_enforced() {
    let (_dir, store) = fresh().await;
    // Every pooled connection must enforce foreign keys; hit several.
    for _ in 0..4 {
        let err = sqlx::query(
            "INSERT INTO runs (id, project_id, state) VALUES ('r1', 'nope', 'planning')",
        )
        .execute(store.pool())
        .await
        .unwrap_err();
        assert!(
            err.to_string().to_lowercase().contains("foreign key"),
            "{err}"
        );
    }
    let fk: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(fk, 1);
}

#[tokio::test]
async fn schema_rejects_bad_values() {
    let (_dir, store) = fresh().await;
    sqlx::query("INSERT INTO projects (id, name, repo_path) VALUES ('p','n','/r')")
        .execute(store.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO runs (id, project_id, state) VALUES ('r','p','planning')")
        .execute(store.pool())
        .await
        .unwrap();
    let bad_state = sqlx::query(
        "INSERT INTO tasks (run_id, id, state, agent_priority, spec_json) VALUES ('r','t','exploded',1,'{}')",
    )
    .execute(store.pool())
    .await;
    assert!(bad_state.is_err());
    let bad_priority = sqlx::query(
        "INSERT INTO tasks (run_id, id, state, agent_priority, spec_json) VALUES ('r','t','pending',101,'{}')",
    )
    .execute(store.pool())
    .await;
    assert!(bad_priority.is_err());
}
