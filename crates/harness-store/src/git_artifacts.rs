//! Durable task-commit identities.

use harness_core::domain::{RunId, TaskId};
use sqlx::Row;

use crate::{Store, StoreError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskCommitRecord {
    pub run_id: RunId,
    pub task_id: TaskId,
    pub task_commit_sha: String,
    pub integration_commit_sha: Option<String>,
    pub diff_paths: Vec<String>,
}

#[allow(async_fn_in_trait)]
pub trait GitArtifactRepository {
    async fn get_task_commit(
        &self,
        run_id: &RunId,
        task_id: &TaskId,
    ) -> Result<Option<TaskCommitRecord>, StoreError>;
    async fn record_task_commit(&self, record: &TaskCommitRecord) -> Result<(), StoreError>;
    async fn record_integration_commit(
        &self,
        run_id: &RunId,
        task_id: &TaskId,
        integration_commit_sha: &str,
    ) -> Result<(), StoreError>;
}

impl GitArtifactRepository for Store {
    async fn get_task_commit(
        &self,
        run_id: &RunId,
        task_id: &TaskId,
    ) -> Result<Option<TaskCommitRecord>, StoreError> {
        let row = sqlx::query(
            "SELECT task_commit_sha, integration_commit_sha, diff_paths_json FROM git_artifacts \
             WHERE run_id = ? AND task_id = ?",
        )
        .bind(run_id.as_str())
        .bind(task_id.as_str())
        .fetch_optional(self.pool())
        .await?;
        row.map(|row| {
            Ok(TaskCommitRecord {
                run_id: run_id.clone(),
                task_id: task_id.clone(),
                task_commit_sha: row.try_get("task_commit_sha")?,
                integration_commit_sha: row.try_get("integration_commit_sha")?,
                diff_paths: serde_json::from_str(row.try_get::<&str, _>("diff_paths_json")?)?,
            })
        })
        .transpose()
    }

    async fn record_task_commit(&self, record: &TaskCommitRecord) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO git_artifacts (run_id, task_id, task_commit_sha, diff_paths_json) \
             VALUES (?, ?, ?, ?)",
        )
        .bind(record.run_id.as_str())
        .bind(record.task_id.as_str())
        .bind(&record.task_commit_sha)
        .bind(serde_json::to_string(&record.diff_paths)?)
        .execute(self.pool())
        .await
        .map_err(StoreError::from_insert)?;
        Ok(())
    }

    async fn record_integration_commit(
        &self,
        run_id: &RunId,
        task_id: &TaskId,
        integration_commit_sha: &str,
    ) -> Result<(), StoreError> {
        let changed = sqlx::query(
            "UPDATE git_artifacts SET integration_commit_sha = ? \
             WHERE run_id = ? AND task_id = ? \
             AND (integration_commit_sha IS NULL OR integration_commit_sha = ?)",
        )
        .bind(integration_commit_sha)
        .bind(run_id.as_str())
        .bind(task_id.as_str())
        .bind(integration_commit_sha)
        .execute(self.pool())
        .await?;
        if changed.rows_affected() == 0 {
            return Err(StoreError::AlreadyExists);
        }
        Ok(())
    }
}
