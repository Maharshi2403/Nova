//! A run's source revision is pinned before workers start and retained on resume.
use crate::{Store, StoreError};
use harness_core::domain::RunId;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionSource {
    pub base_sha: String,
    pub committed_snapshot: bool,
}
impl Store {
    pub async fn execution_source(
        &self,
        run: &RunId,
    ) -> Result<Option<ExecutionSource>, StoreError> {
        let payload:Option<String>=sqlx::query_scalar("SELECT payload_json FROM events WHERE run_id=? AND kind='execution.source' ORDER BY sequence LIMIT 1").bind(run.as_str()).fetch_optional(self.pool()).await?;
        payload
            .map(|s| serde_json::from_str(&s).map_err(StoreError::from))
            .transpose()
    }
    pub async fn record_execution_launch(
        &self,
        run: &RunId,
        source: &ExecutionSource,
    ) -> Result<(), StoreError> {
        sqlx::query("INSERT INTO events(run_id,kind,payload_json) VALUES(?,'execution.launch',?)")
            .bind(run.as_str())
            .bind(serde_json::to_string(source)?)
            .execute(self.pool())
            .await?;
        Ok(())
    }
    pub async fn pin_execution_source(
        &self,
        run: &RunId,
        source: &ExecutionSource,
    ) -> Result<ExecutionSource, StoreError> {
        if !(40..=64).contains(&source.base_sha.len())
            || !source.base_sha.bytes().all(|c| c.is_ascii_hexdigit())
        {
            return Err(StoreError::Corrupt("invalid source revision".into()));
        }
        sqlx::query("INSERT INTO events(run_id,kind,idempotency_key,payload_json) VALUES(?,'execution.source',?,?) ON CONFLICT(idempotency_key) DO NOTHING").bind(run.as_str()).bind(format!("execution-source:{run}")).bind(serde_json::to_string(source)?).execute(self.pool()).await?;
        self.execution_source(run)
            .await?
            .ok_or(StoreError::NotFound("execution source"))
    }
}
