use crate::{Store, StoreError};
use harness_core::domain::ProjectId;
impl Store {
    /// Called only from an explicit authenticated local-user registration decision.
    pub async fn set_project_trust(
        &self,
        project: &ProjectId,
        trusted_local: bool,
    ) -> Result<(), StoreError> {
        sqlx::query("INSERT INTO project_trust(project_id,trusted_local) VALUES(?,?) ON CONFLICT(project_id) DO UPDATE SET trusted_local=excluded.trusted_local").bind(project.as_str()).bind(trusted_local).execute(self.pool()).await?;
        Ok(())
    }
    pub async fn project_is_trusted(&self, project: &ProjectId) -> Result<bool, StoreError> {
        let trusted: Option<bool> =
            sqlx::query_scalar("SELECT trusted_local FROM project_trust WHERE project_id=?")
                .bind(project.as_str())
                .fetch_optional(self.pool())
                .await?;
        Ok(trusted.unwrap_or(false))
    }
}

impl Store {
    /// Remove only registration metadata for a project with no runs. Files are untouched.
    pub async fn remove_empty_project(&self, project: &ProjectId) -> Result<(), StoreError> {
        let mut tx = self.pool().begin().await?;
        let runs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM runs WHERE project_id=?")
            .bind(project.as_str())
            .fetch_one(&mut *tx)
            .await?;
        if runs != 0 {
            return Err(StoreError::PlanNotDecidable(
                "projects with runs cannot be removed",
            ));
        }
        sqlx::query("DELETE FROM project_trust WHERE project_id=?")
            .bind(project.as_str())
            .execute(&mut *tx)
            .await?;
        let deleted = sqlx::query("DELETE FROM projects WHERE id=?")
            .bind(project.as_str())
            .execute(&mut *tx)
            .await?;
        if deleted.rows_affected() == 0 {
            return Err(StoreError::NotFound("project"));
        }
        tx.commit().await?;
        Ok(())
    }
}
