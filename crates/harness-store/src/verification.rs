//! Durable evidence produced by the deterministic verifier.

use harness_core::domain::{RunId, TaskId};
use serde_json::Value;
use sqlx::Row;

use crate::{Store, StoreError};

#[derive(Debug, Clone, PartialEq)]
pub struct TestResult {
    pub id: i64,
    pub run_id: RunId,
    pub task_id: TaskId,
    pub command: Vec<String>,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub artifact: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewTestResult {
    pub run_id: RunId,
    pub task_id: TaskId,
    pub command: Vec<String>,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub artifact: Value,
}

#[allow(async_fn_in_trait)]
pub trait VerificationRepository {
    async fn record_test_result(&self, result: NewTestResult) -> Result<TestResult, StoreError>;
    async fn list_test_results(
        &self,
        run_id: &RunId,
        task_id: &TaskId,
    ) -> Result<Vec<TestResult>, StoreError>;
}

fn row_to_result(row: &sqlx::sqlite::SqliteRow) -> Result<TestResult, StoreError> {
    let duration_ms = u64::try_from(row.try_get::<i64, _>("duration_ms")?)
        .map_err(|_| StoreError::Corrupt("negative test result duration".into()))?;
    Ok(TestResult {
        id: row.try_get("id")?,
        run_id: RunId::new(row.try_get::<String, _>("run_id")?)
            .map_err(|e| StoreError::Corrupt(e.to_string()))?,
        task_id: TaskId::new(row.try_get::<String, _>("task_id")?)
            .map_err(|e| StoreError::Corrupt(e.to_string()))?,
        command: serde_json::from_str(row.try_get::<&str, _>("command_json")?)?,
        exit_code: row.try_get("exit_code")?,
        duration_ms,
        artifact: serde_json::from_str(row.try_get::<&str, _>("artifact_json")?)?,
    })
}

impl VerificationRepository for Store {
    async fn record_test_result(
        &self,
        mut result: NewTestResult,
    ) -> Result<TestResult, StoreError> {
        harness_core::redaction::redact_json(&mut result.artifact);
        result.command = result
            .command
            .iter()
            .map(|s| harness_core::redaction::redact(s))
            .collect();
        let duration_ms = i64::try_from(result.duration_ms)
            .map_err(|_| StoreError::Corrupt("test result duration out of range".into()))?;
        let command_json = serde_json::to_string(&result.command)?;
        let artifact_json = serde_json::to_string(&result.artifact)?;
        let insert = sqlx::query(
            "INSERT INTO test_results (run_id, task_id, command_json, exit_code, duration_ms, artifact_json) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(result.run_id.as_str())
        .bind(result.task_id.as_str())
        .bind(command_json)
        .bind(result.exit_code)
        .bind(duration_ms)
        .bind(artifact_json)
        .execute(self.pool())
        .await?;
        let id = insert.last_insert_rowid();
        let row = sqlx::query(
            "SELECT id, run_id, task_id, command_json, exit_code, duration_ms, artifact_json \
             FROM test_results WHERE id = ?",
        )
        .bind(id)
        .fetch_one(self.pool())
        .await?;
        row_to_result(&row)
    }

    async fn list_test_results(
        &self,
        run_id: &RunId,
        task_id: &TaskId,
    ) -> Result<Vec<TestResult>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, run_id, task_id, command_json, exit_code, duration_ms, artifact_json \
             FROM test_results WHERE run_id = ? AND task_id = ? ORDER BY id",
        )
        .bind(run_id.as_str())
        .bind(task_id.as_str())
        .fetch_all(self.pool())
        .await?;
        rows.iter().map(row_to_result).collect()
    }
}
