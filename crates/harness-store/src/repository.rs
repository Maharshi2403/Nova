//! Repository for projects, runs and tasks. No SQL escapes this crate.

use harness_core::domain::{
    Priority, Project, ProjectId, Run, RunId, RunState, Task, TaskId, TaskSpec, TaskState,
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
    /// Validated run-level state change. A paused run cannot have new work dispatched.
    async fn transition_run(
        &self,
        id: &RunId,
        expected_revision: u64,
        to: RunState,
        idempotency_key: Option<&str>,
    ) -> Result<Run, StoreError>;
    /// Insert a task and its dependency edges atomically. Dependencies must already
    /// exist in the same run.
    async fn insert_task(&self, task: &Task) -> Result<(), StoreError>;
    async fn get_task(&self, run_id: &RunId, id: &TaskId) -> Result<Task, StoreError>;
    /// All tasks of a run in creation order (the FIFO tie-break order).
    async fn list_tasks(&self, run_id: &RunId) -> Result<Vec<Task>, StoreError>;
    /// Revision compare-and-set priority change. Only queued tasks (never leased or
    /// running ones) can be reprioritized, so a running task is never preempted.
    async fn set_priority(
        &self,
        run_id: &RunId,
        id: &TaskId,
        expected_revision: u64,
        update: PriorityUpdate,
    ) -> Result<Task, StoreError>;
    /// Validated compare-and-set state change. The state update and its audit event are
    /// committed in one transaction. With an `idempotency_key`, replaying the same request
    /// returns the current task with `replayed = true` and writes nothing.
    async fn transition_task(
        &self,
        run_id: &RunId,
        id: &TaskId,
        expected_revision: u64,
        to: TaskState,
        idempotency_key: Option<&str>,
    ) -> Result<Transitioned, StoreError>;
    /// Events of a run with `sequence > after`, ascending, at most `limit`.
    async fn list_events(
        &self,
        run_id: &RunId,
        after: i64,
        limit: u32,
    ) -> Result<Vec<EventRecord>, StoreError>;
}

/// `Manual(None)` clears the manual override.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriorityUpdate {
    Manual(Option<Priority>),
    Agent(Priority),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Transitioned {
    pub task: Task,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRecord {
    pub sequence: i64,
    pub run_id: String,
    pub task_id: Option<String>,
    pub kind: String,
    pub from_state: Option<String>,
    pub to_state: Option<String>,
    pub attempt: Option<i64>,
    pub idempotency_key: Option<String>,
}

pub(crate) fn enum_to_text<T: serde::Serialize>(v: &T) -> Result<String, StoreError> {
    match serde_json::to_value(v)? {
        serde_json::Value::String(s) => Ok(s),
        _ => Err(StoreError::Corrupt(
            "enum did not serialize to a string".into(),
        )),
    }
}

pub(crate) fn enum_from_text<T: serde::de::DeserializeOwned>(s: &str) -> Result<T, StoreError> {
    Ok(serde_json::from_value(serde_json::Value::String(
        s.to_string(),
    ))?)
}

pub(crate) fn to_i64(v: u64) -> Result<i64, StoreError> {
    i64::try_from(v).map_err(|_| StoreError::Corrupt("integer out of range".into()))
}

pub(crate) fn to_u64(v: i64) -> Result<u64, StoreError> {
    u64::try_from(v).map_err(|_| StoreError::Corrupt("negative integer in database".into()))
}

const TASK_COLUMNS: &str =
    "run_id, state, revision, attempt, spec_json, agent_priority, manual_priority";

fn row_to_task(row: &SqliteRow) -> Result<Task, StoreError> {
    let mut spec: TaskSpec = serde_json::from_str(row.try_get::<&str, _>("spec_json")?)?;
    // The priority columns are the source of truth; spec_json keeps the original proposal.
    spec.agent_priority = Priority::new(row.try_get("agent_priority")?)?;
    spec.manual_priority = row
        .try_get::<Option<i64>, _>("manual_priority")?
        .map(Priority::new)
        .transpose()?;
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

    async fn transition_run(
        &self,
        id: &RunId,
        expected_revision: u64,
        to: RunState,
        idempotency_key: Option<&str>,
    ) -> Result<Run, StoreError> {
        let current = self.get_run(id).await?;
        if current.revision != expected_revision {
            return Err(StoreError::RevisionConflict {
                expected: expected_revision,
                actual: current.revision,
            });
        }
        let allowed = matches!(
            (current.state, to),
            (RunState::Running, RunState::Paused)
                | (RunState::Running, RunState::Completed)
                | (RunState::Paused, RunState::Running)
                | (RunState::Planning, RunState::Cancelled)
                | (RunState::Planning, RunState::Failed)
                | (RunState::AwaitingApproval, RunState::Cancelled)
                | (RunState::Running, RunState::Cancelled)
                | (RunState::Paused, RunState::Cancelled)
        );
        if !allowed {
            return Err(StoreError::PlanNotDecidable("illegal run state transition"));
        }
        let mut tx = self.pool().begin().await?;
        let changed = sqlx::query(
            "UPDATE runs SET state = ?, revision = revision + 1 \
             WHERE id = ? AND revision = ?",
        )
        .bind(enum_to_text(&to)?)
        .bind(id.as_str())
        .bind(to_i64(expected_revision)?)
        .execute(&mut *tx)
        .await?;
        if changed.rows_affected() == 0 {
            tx.rollback().await?;
            let actual = self.get_run(id).await?.revision;
            return Err(StoreError::RevisionConflict {
                expected: expected_revision,
                actual,
            });
        }
        if to == RunState::Completed {
            let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tasks WHERE run_id = ?")
                .bind(id.as_str())
                .fetch_one(&mut *tx)
                .await?;
            let incomplete: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tasks t LEFT JOIN git_artifacts g ON g.run_id = t.run_id AND g.task_id = t.id WHERE t.run_id = ? AND (t.state <> 'completed' OR g.integration_commit_sha IS NULL)").bind(id.as_str()).fetch_one(&mut *tx).await?;
            if total == 0 || incomplete > 0 {
                return Err(StoreError::PlanNotDecidable(
                    "run requires completed tasks with integration evidence",
                ));
            }
        }
        sqlx::query(
            "INSERT INTO events (run_id, kind, from_state, to_state, idempotency_key) \
             VALUES (?, 'run.transition', ?, ?, ?)",
        )
        .bind(id.as_str())
        .bind(enum_to_text(&current.state)?)
        .bind(enum_to_text(&to)?)
        .bind(idempotency_key)
        .execute(&mut *tx)
        .await
        .map_err(StoreError::from_insert)?;
        tx.commit().await?;
        self.get_run(id).await
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
        let row = sqlx::query(&format!(
            "SELECT {TASK_COLUMNS} FROM tasks WHERE run_id = ? AND id = ?"
        ))
        .bind(run_id.as_str())
        .bind(id.as_str())
        .fetch_optional(self.pool())
        .await?
        .ok_or(StoreError::NotFound("task"))?;
        row_to_task(&row)
    }

    async fn list_tasks(&self, run_id: &RunId) -> Result<Vec<Task>, StoreError> {
        let rows = sqlx::query(&format!(
            "SELECT {TASK_COLUMNS} FROM tasks WHERE run_id = ? ORDER BY rowid ASC"
        ))
        .bind(run_id.as_str())
        .fetch_all(self.pool())
        .await?;
        rows.iter().map(row_to_task).collect()
    }

    async fn set_priority(
        &self,
        run_id: &RunId,
        id: &TaskId,
        expected_revision: u64,
        update: PriorityUpdate,
    ) -> Result<Task, StoreError> {
        let current = self.get_task(run_id, id).await?;
        if !matches!(
            current.state,
            TaskState::Proposed | TaskState::Pending | TaskState::Ready | TaskState::RetryWait
        ) {
            return Err(StoreError::NotReprioritizable(current.state));
        }
        let (column, value, kind) = match update {
            PriorityUpdate::Manual(p) => (
                "manual_priority",
                p.map(|p| i64::from(p.get())),
                "task.priority.manual",
            ),
            PriorityUpdate::Agent(p) => (
                "agent_priority",
                Some(i64::from(p.get())),
                "task.priority.agent",
            ),
        };
        let mut tx = self.pool().begin().await?;
        let result = sqlx::query(&format!(
            "UPDATE tasks SET {column} = ?, revision = revision + 1 \
             WHERE run_id = ? AND id = ? AND revision = ? \
             AND state IN ('proposed', 'pending', 'ready', 'retry_wait')"
        ))
        .bind(value)
        .bind(run_id.as_str())
        .bind(id.as_str())
        .bind(to_i64(expected_revision)?)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() == 0 {
            tx.rollback().await?;
            let now = self.get_task(run_id, id).await?;
            if now.revision == expected_revision {
                return Err(StoreError::NotReprioritizable(now.state));
            }
            return Err(StoreError::RevisionConflict {
                expected: expected_revision,
                actual: now.revision,
            });
        }
        sqlx::query(
            "INSERT INTO events (run_id, task_id, kind, attempt, payload_json) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(run_id.as_str())
        .bind(id.as_str())
        .bind(kind)
        .bind(i64::from(current.attempt))
        .bind(serde_json::json!({ "value": value }).to_string())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        self.get_task(run_id, id).await
    }

    async fn transition_task(
        &self,
        run_id: &RunId,
        id: &TaskId,
        expected_revision: u64,
        to: TaskState,
        idempotency_key: Option<&str>,
    ) -> Result<Transitioned, StoreError> {
        let to_text = enum_to_text(&to)?;
        if let Some(key) = idempotency_key
            && let Some(prior) = find_event_by_key(self, key).await?
        {
            let same = prior.run_id == run_id.as_str()
                && prior.task_id.as_deref() == Some(id.as_str())
                && prior.to_state.as_deref() == Some(to_text.as_str());
            if !same {
                return Err(StoreError::IdempotencyKeyReused);
            }
            let task = self.get_task(run_id, id).await?;
            return Ok(Transitioned {
                task,
                replayed: true,
            });
        }

        let current = self.get_task(run_id, id).await?;
        if current.revision != expected_revision {
            return Err(StoreError::RevisionConflict {
                expected: expected_revision,
                actual: current.revision,
            });
        }
        state::transition(current.state, to)?;

        // State change and audit event commit together or not at all.
        let mut tx = self.pool().begin().await?;
        let result = sqlx::query(
            "UPDATE tasks SET state = ?, revision = revision + 1 \
             WHERE run_id = ? AND id = ? AND revision = ?",
        )
        .bind(&to_text)
        .bind(run_id.as_str())
        .bind(id.as_str())
        .bind(to_i64(expected_revision)?)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() == 0 {
            tx.rollback().await?;
            // Lost a race between the read above and this write.
            let actual = self.get_task(run_id, id).await?.revision;
            return Err(StoreError::RevisionConflict {
                expected: expected_revision,
                actual,
            });
        }
        let inserted = sqlx::query(
            "INSERT INTO events (run_id, task_id, kind, from_state, to_state, attempt, \
             idempotency_key) VALUES (?, ?, 'task.transition', ?, ?, ?, ?)",
        )
        .bind(run_id.as_str())
        .bind(id.as_str())
        .bind(enum_to_text(&current.state)?)
        .bind(&to_text)
        .bind(i64::from(current.attempt))
        .bind(idempotency_key)
        .execute(&mut *tx)
        .await;
        match inserted {
            Ok(_) => {}
            Err(e) => {
                tx.rollback().await?;
                return Err(match StoreError::from_insert(e) {
                    // Another request with the same key won the race.
                    StoreError::AlreadyExists => StoreError::IdempotencyKeyReused,
                    other => other,
                });
            }
        }
        tx.commit().await?;
        let task = self.get_task(run_id, id).await?;
        Ok(Transitioned {
            task,
            replayed: false,
        })
    }

    async fn list_events(
        &self,
        run_id: &RunId,
        after: i64,
        limit: u32,
    ) -> Result<Vec<EventRecord>, StoreError> {
        let rows = sqlx::query(
            "SELECT sequence, run_id, task_id, kind, from_state, to_state, attempt, \
             idempotency_key FROM events WHERE run_id = ? AND sequence > ? \
             ORDER BY sequence ASC LIMIT ?",
        )
        .bind(run_id.as_str())
        .bind(after)
        .bind(i64::from(limit.min(1000)))
        .fetch_all(self.pool())
        .await?;
        rows.iter().map(row_to_event).collect()
    }
}

fn row_to_event(row: &SqliteRow) -> Result<EventRecord, StoreError> {
    Ok(EventRecord {
        sequence: row.try_get("sequence")?,
        run_id: row.try_get("run_id")?,
        task_id: row.try_get("task_id")?,
        kind: row.try_get("kind")?,
        from_state: row.try_get("from_state")?,
        to_state: row.try_get("to_state")?,
        attempt: row.try_get("attempt")?,
        idempotency_key: row.try_get("idempotency_key")?,
    })
}

async fn find_event_by_key(store: &Store, key: &str) -> Result<Option<EventRecord>, StoreError> {
    let row = sqlx::query(
        "SELECT sequence, run_id, task_id, kind, from_state, to_state, attempt, \
         idempotency_key FROM events WHERE idempotency_key = ?",
    )
    .bind(key)
    .fetch_optional(store.pool())
    .await?;
    row.as_ref().map(row_to_event).transpose()
}

impl From<TransitionError> for StoreError {
    fn from(e: TransitionError) -> Self {
        Self::IllegalTransition(e)
    }
}
