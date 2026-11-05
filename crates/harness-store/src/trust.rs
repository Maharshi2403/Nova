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
