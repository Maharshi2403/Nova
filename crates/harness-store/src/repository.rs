//! Repository for projects, runs and tasks. No SQL escapes this crate.

use harness_core::domain::{
    Project, ProjectId, Run, RunId, RunState, Task, TaskId, TaskSpec, TaskState,
};
use harness_core::state::{self, TransitionError};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use crate::{Store, StoreError};

#[allow(async_fn_in_trait)]
pub trait Repository {
    async fn create_project(&self, project: &Project) -> Result<(), StoreError>;
    async fn get_project(&self, id: &ProjectId) -> Result<Project, StoreError>;
    async fn create_run(&self, run: &Run) -> Result<(), StoreError>;
    async fn get_run(&self, id: &RunId) -> Result<Run, StoreError>;
    /// Insert a task and its dependency edges atomically. Dependencies must already
    /// exist in the same run.
    async fn insert_task(&self, task: &Task) -> Result<(), StoreError>;
    async fn get_task(&self, run_id: &RunId, id: &TaskId) -> Result<Task, StoreError>;
    /// Validated compare-and-set state change; returns the updated task.
    async fn transition_task(
        &self,
        run_id: &RunId,
        id: &TaskId,
        expected_revision: u64,
        to: TaskState,
    ) -> Result<Task, StoreError>;
}

fn enum_to_text<T: serde::Serialize>(v: &T) -> Result<String, StoreError> {
    match serde_json::to_value(v)? {
        serde_json::Value::String(s) => Ok(s),
        _ => Err(StoreError::Corrupt(
            "enum did not serialize to a string".into(),
        )),
    }
}

fn enum_from_text<T: serde::de::DeserializeOwned>(s: &str) -> Result<T, StoreError> {
    Ok(serde_json::from_value(serde_json::Value::String(
        s.to_string(),
    ))?)
}

fn to_i64(v: u64) -> Result<i64, StoreError> {
    i64::try_from(v).map_err(|_| StoreError::Corrupt("integer out of range".into()))
}

fn to_u64(v: i64) -> Result<u64, StoreError> {
    u64::try_from(v).map_err(|_| StoreError::Corrupt("negative integer in database".into()))
}

fn row_to_task(row: &SqliteRow) -> Result<Task, StoreError> {
    let spec: TaskSpec = serde_json::from_str(row.try_get::<&str, _>("spec_json")?)?;
    let attempt = u32::try_from(row.try_get::<i64, _>("attempt")?)
        .map_err(|_| StoreError::Corrupt("attempt out of range".into()))?;
    Ok(Task {
        run_id: RunId::new(row.try_get::<String, _>("run_id")?)
            .map_err(|e| StoreError::Corrupt(e.to_string()))?,
        spec,
        state: enum_from_text(row.try_get::<&str, _>("state")?)?,
        revision: to_u64(row.try_get("revision")?)?,
        attempt,
    })
}

impl Repository for Store {
    async fn create_project(&self, p: &Project) -> Result<(), StoreError> {
        sqlx::query("INSERT INTO projects (id, name, repo_path) VALUES (?, ?, ?)")
            .bind(p.id.as_str())
            .bind(&p.name)
            .bind(&p.repo_path)
            .execute(self.pool())
            .await
            .map_err(StoreError::from_insert)?;
        Ok(())
    }

    async fn get_project(&self, id: &ProjectId) -> Result<Project, StoreError> {
        let row = sqlx::query("SELECT id, name, repo_path FROM projects WHERE id = ?")
            .bind(id.as_str())
            .fetch_optional(self.pool())
            .await?
            .ok_or(StoreError::NotFound("project"))?;
        Ok(Project {
            id: id.clone(),
            name: row.try_get("name")?,
            repo_path: row.try_get("repo_path")?,
        })
    }

    async fn create_run(&self, run: &Run) -> Result<(), StoreError> {
        sqlx::query("INSERT INTO runs (id, project_id, state, revision) VALUES (?, ?, ?, ?)")
            .bind(run.id.as_str())
            .bind(run.project_id.as_str())
            .bind(enum_to_text(&run.state)?)
            .bind(to_i64(run.revision)?)
            .execute(self.pool())
            .await
            .map_err(StoreError::from_insert)?;
        Ok(())
    }

    async fn get_run(&self, id: &RunId) -> Result<Run, StoreError> {
        let row = sqlx::query("SELECT project_id, state, revision FROM runs WHERE id = ?")
            .bind(id.as_str())
            .fetch_optional(self.pool())
            .await?
            .ok_or(StoreError::NotFound("run"))?;
        let state: RunState = enum_from_text(row.try_get::<&str, _>("state")?)?;
        Ok(Run {
            id: id.clone(),
            project_id: ProjectId::new(row.try_get::<String, _>("project_id")?)
                .map_err(|e| StoreError::Corrupt(e.to_string()))?,
            state,
            revision: to_u64(row.try_get("revision")?)?,
        })
    }

    async fn insert_task(&self, task: &Task) -> Result<(), StoreError> {
        task.spec.validate()?;
        let spec_json = serde_json::to_string(&task.spec)?;
        let mut tx = self.pool().begin().await?;
        sqlx::query(
            "INSERT INTO tasks (run_id, id, state, revision, attempt, agent_priority, \
             manual_priority, spec_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(task.run_id.as_str())
        .bind(task.id().as_str())
        .bind(enum_to_text(&task.state)?)
        .bind(to_i64(task.revision)?)
        .bind(i64::from(task.attempt))
        .bind(i64::from(task.spec.agent_priority))
        .bind(task.spec.manual_priority.map(i64::from))
        .bind(spec_json)
        .execute(&mut *tx)
        .await
        .map_err(StoreError::from_insert)?;
        for dep in &task.spec.dependencies {
            sqlx::query(
                "INSERT OR IGNORE INTO task_dependencies (run_id, task_id, depends_on_id) \
                 VALUES (?, ?, ?)",
            )
            .bind(task.run_id.as_str())
            .bind(task.id().as_str())
            .bind(dep.as_str())
            .execute(&mut *tx)
            .await
            .map_err(StoreError::from_insert)?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn get_task(&self, run_id: &RunId, id: &TaskId) -> Result<Task, StoreError> {
        let row = sqlx::query(
            "SELECT run_id, state, revision, attempt, spec_json FROM tasks \
             WHERE run_id = ? AND id = ?",
        )
        .bind(run_id.as_str())
        .bind(id.as_str())
        .fetch_optional(self.pool())
        .await?
        .ok_or(StoreError::NotFound("task"))?;
        row_to_task(&row)
    }

    async fn transition_task(
        &self,
        run_id: &RunId,
        id: &TaskId,
        expected_revision: u64,
        to: TaskState,
    ) -> Result<Task, StoreError> {
        let current = self.get_task(run_id, id).await?;
        if current.revision != expected_revision {
            return Err(StoreError::RevisionConflict {
                expected: expected_revision,
                actual: current.revision,
            });
        }
        state::transition(current.state, to)?;
        let result = sqlx::query(
            "UPDATE tasks SET state = ?, revision = revision + 1 \
             WHERE run_id = ? AND id = ? AND revision = ?",
        )
        .bind(enum_to_text(&to)?)
        .bind(run_id.as_str())
        .bind(id.as_str())
        .bind(to_i64(expected_revision)?)
        .execute(self.pool())
        .await?;
        if result.rows_affected() == 0 {
            // Lost a race between the read above and this write.
            let actual = self.get_task(run_id, id).await?.revision;
            return Err(StoreError::RevisionConflict {
                expected: expected_revision,
                actual,
            });
        }
        self.get_task(run_id, id).await
    }
}

impl From<TransitionError> for StoreError {
    fn from(e: TransitionError) -> Self {
        Self::IllegalTransition(e)
    }
}
