//! Bounded read model for native clients. SQL stays inside the store.
use crate::{Store, StoreError};
use harness_core::domain::{Project, ProjectId, Run, RunId};
use serde::Serialize;
use sqlx::Row;
#[derive(Serialize)]
pub struct Dashboard {
    pub projects: Vec<DashboardProject>,
    pub runs: Vec<DashboardRun>,
    pub project_count: i64,
    pub run_count: i64,
    pub offset: u32,
    pub limit: u32,
}
#[derive(Serialize)]
pub struct DashboardProject {
    #[serde(flatten)]
    pub project: Project,
    pub trusted_local: bool,
}
#[derive(Serialize)]
pub struct DashboardRun {
    #[serde(flatten)]
    pub run: Run,
    pub plan_version: Option<i64>,
}
impl Store {
    pub async fn dashboard(&self, offset: u32, limit: u32) -> Result<Dashboard, StoreError> {
        if limit == 0 || limit > 200 {
            return Err(StoreError::Corrupt("dashboard limit must be 1..200".into()));
        }
        let mut tx = self.pool().begin().await?;
        let project_count = sqlx::query_scalar("SELECT COUNT(*) FROM projects")
            .fetch_one(&mut *tx)
            .await?;
        let run_count = sqlx::query_scalar("SELECT COUNT(*) FROM runs")
            .fetch_one(&mut *tx)
            .await?;
        let rows=sqlx::query("SELECT p.id,p.name,p.repo_path,COALESCE(t.trusted_local,0) AS trusted_local FROM projects p LEFT JOIN project_trust t ON t.project_id=p.id ORDER BY p.rowid DESC LIMIT ? OFFSET ?").bind(limit).bind(offset).fetch_all(&mut *tx).await?;
        let projects = rows
            .into_iter()
            .map(|r| {
                Ok(DashboardProject {
                    project: Project {
                        id: ProjectId::new(r.try_get::<String, _>("id")?)?,
                        name: r.try_get("name")?,
                        repo_path: r.try_get("repo_path")?,
                    },
                    trusted_local: r.try_get("trusted_local")?,
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        let rows = sqlx::query(
            "SELECT id,project_id,state,revision,(SELECT MAX(version) FROM plans WHERE run_id=runs.id) AS plan_version FROM runs ORDER BY rowid DESC LIMIT ? OFFSET ?",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&mut *tx)
        .await?;
        let runs = rows
            .into_iter()
            .map(|r| {
                Ok(DashboardRun {
                    plan_version: r.try_get("plan_version")?,
                    run: Run {
                        id: RunId::new(r.try_get::<String, _>("id")?)?,
                        project_id: ProjectId::new(r.try_get::<String, _>("project_id")?)?,
                        state: super::repository::enum_from_text(r.try_get("state")?)?,
                        revision: super::repository::to_u64(r.try_get("revision")?)?,
                    },
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        tx.commit().await?;
        Ok(Dashboard {
            projects,
            runs,
            project_count,
            run_count,
            offset,
            limit,
        })
    }
}
