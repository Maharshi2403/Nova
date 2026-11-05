//! Append-only, idempotent per-call usage. Unknown measurements propagate to totals.
use crate::{Store, StoreError};
use harness_core::domain::{RunId, TaskId};
use harness_core::messages::Usage;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UsageRecord {
    pub message_id: String,
    pub lease_id: String,
    pub task_id: String,
    pub usage: Usage,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UsageSummary {
    pub calls: usize,
    pub input_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
}

impl UsageSummary {
    pub fn from_records(records: &[UsageRecord]) -> Self {
        fn total(records: &[UsageRecord], field: impl Fn(&Usage) -> Option<u64>) -> Option<u64> {
            if records.is_empty() {
                return None;
            }
            records
                .iter()
                .try_fold(0u64, |sum, r| sum.checked_add(field(&r.usage)?))
        }
        let cost = if records.is_empty() {
            None
        } else {
            records.iter().try_fold(0.0, |sum, r| {
                let result: f64 = sum + r.usage.cost_usd?;
                result.is_finite().then_some(result)
            })
        };
        Self {
            calls: records.len(),
            input_tokens: total(records, |u| u.input_tokens),
            cached_input_tokens: total(records, |u| u.cached_input_tokens),
            output_tokens: total(records, |u| u.output_tokens),
            cost_usd: cost,
        }
    }
}

impl Store {
    pub async fn begin_provider_call(&self, id: &str, lease: &str) -> Result<(), StoreError> {
        sqlx::query("INSERT INTO provider_calls(id,lease_id) VALUES(?,?)")
            .bind(id)
            .bind(lease)
            .execute(self.pool())
            .await?;
        Ok(())
    }
    pub async fn finish_provider_call(&self, id: &str) -> Result<(), StoreError> {
        sqlx::query("UPDATE provider_calls SET finished=1 WHERE id=?")
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }
    pub async fn record_usage(
        &self,
        lease_id: &str,
        message_id: &str,
        usage: &Usage,
    ) -> Result<(), StoreError> {
        if message_id.is_empty()
            || usage.provider.trim().is_empty()
            || usage.model_id.trim().is_empty()
            || usage.cost_usd.is_some_and(|c| !c.is_finite() || c < 0.0)
            || matches!((usage.cached_input_tokens, usage.input_tokens), (Some(c), Some(i)) if c > i)
        {
            return Err(StoreError::Corrupt("invalid usage record".into()));
        }
        let payload = serde_json::to_string(usage)?;
        let mut tx = self.pool().begin().await?;
        let inserted = sqlx::query("INSERT INTO usage_records(message_id, lease_id, payload) VALUES (?, ?, ?) ON CONFLICT(message_id) DO NOTHING")
            .bind(message_id).bind(lease_id).bind(&payload).execute(&mut *tx).await?;
        if inserted.rows_affected() > 0 {
            let settled: Option<i64> =
                sqlx::query_scalar("SELECT settled FROM budget_reservations WHERE lease_id = ?")
                    .bind(lease_id)
                    .fetch_optional(&mut *tx)
                    .await?;
            if settled == Some(1) {
                return Err(StoreError::BudgetDenied("usage already settled"));
            }
        }
        let existing: (String, String) =
            sqlx::query_as("SELECT lease_id, payload FROM usage_records WHERE message_id = ?")
                .bind(message_id)
                .fetch_one(&mut *tx)
                .await?;
        if existing != (lease_id.to_string(), payload) {
            return Err(StoreError::IdempotencyKeyReused);
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_usage(
        &self,
        run_id: &RunId,
        task_id: Option<&TaskId>,
    ) -> Result<Vec<UsageRecord>, StoreError> {
        let rows: Vec<(String, String, String, String)> = sqlx::query_as(
            "SELECT u.message_id, u.lease_id, l.task_id, u.payload FROM usage_records u JOIN leases l ON l.id = u.lease_id WHERE l.run_id = ? AND (? IS NULL OR l.task_id = ?) ORDER BY u.rowid")
            .bind(run_id.as_str()).bind(task_id.map(TaskId::as_str)).bind(task_id.map(TaskId::as_str))
            .fetch_all(self.pool()).await?;
        rows.into_iter()
            .map(|(message_id, lease_id, task_id, payload)| {
                Ok(UsageRecord {
                    message_id,
                    lease_id,
                    task_id,
                    usage: serde_json::from_str(&payload)?,
                })
            })
            .collect()
    }

    /// Once an attempt is no longer running, replace the reservation with known cost.
    /// Missing cost retains the full reservation; repeated settlement is harmless.
    pub async fn settle_usage(&self, lease_id: &str) -> Result<(), StoreError> {
        let mut tx = self.pool().begin().await?;
        sqlx::query("UPDATE budget_reservations SET amount = amount WHERE lease_id = ?")
            .bind(lease_id)
            .execute(&mut *tx)
            .await?;
        let state: Option<(i64, i64, String)> = sqlx::query_as(
            "SELECT b.released, b.settled, t.state FROM budget_reservations b JOIN leases l ON l.id = b.lease_id JOIN tasks t ON t.run_id = l.run_id AND t.id = l.task_id WHERE b.lease_id = ?")
            .bind(lease_id).fetch_optional(&mut *tx).await?;
        let (released, settled, state) = state.ok_or(StoreError::NotFound("budget reservation"))?;
        if released == 1 || settled == 1 {
            return Ok(());
        }
        if matches!(state.as_str(), "ready" | "leased" | "running") {
            return Err(StoreError::BudgetDenied("attempt still active"));
        }
        let payloads: Vec<(String,)> =
            sqlx::query_as("SELECT payload FROM usage_records WHERE lease_id = ?")
                .bind(lease_id)
                .fetch_all(&mut *tx)
                .await?;
        let unfinished: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM provider_calls WHERE lease_id=? AND finished=0",
        )
        .bind(lease_id)
        .fetch_one(&mut *tx)
        .await?;
        let mut cost = if payloads.is_empty() || unfinished > 0 {
            None
        } else {
            Some(0.0)
        };
        for (payload,) in payloads {
            let usage: Usage = serde_json::from_str(&payload)?;
            cost = cost.zip(usage.cost_usd).map(|(a, b)| a + b);
        }
        if let Some(cost) = cost {
            let micros = (cost * 1_000_000.0).ceil();
            if !micros.is_finite() || micros >= i64::MAX as f64 {
                return Err(StoreError::BudgetDenied("settlement overflow"));
            }
            sqlx::query(
                "UPDATE budget_reservations SET amount = ?, settled = 1 WHERE lease_id = ?",
            )
            .bind(micros as i64)
            .bind(lease_id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }
}
