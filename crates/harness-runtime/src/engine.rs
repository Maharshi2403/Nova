//! Explicit offline execution: Python IPC -> broker -> verifier -> task commit -> integration.
use crate::{
    broker::{
        command::{CommandPolicy, RepoCommandBroker},
        write::RepoWriteBroker,
    },
    committer::TaskCommitter,
    diff_gate::DiffGate,
    git::{TrustedRepo, git, register_repo},
    integration::Integrator,
    locks::ResourceLocks,
    pool::WorkerPool,
    supervisor::{self, Outcome, ToolBroker},
    verifier::Verifier,
    worker::{Worker, WorkerSpec},
    worktree::WorktreeManager,
};
use harness_core::{
    domain::{RunId, RunState, Task, TaskState},
    messages::{AssignmentSession, ModelRef, StartTask, ToolRequest, ToolResponse, Usage},
};
use harness_store::{Store, budgets::BudgetRequest, repository::Repository};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{Semaphore, watch};

#[derive(Clone)]
pub struct Engine {
    root: PathBuf,
    locks: ResourceLocks,
    running: Arc<Mutex<HashSet<String>>>,
    capacity: Arc<Semaphore>,
}
#[derive(Clone)]
struct ExecutionOptions {
    python: PathBuf,
    commands: Vec<Vec<String>>,
    concurrency: usize,
    live: Option<(crate::live::LiveConfig, Arc<String>)>,
}
struct RunGuard {
    running: Arc<Mutex<HashSet<String>>>,
    id: String,
}
impl Drop for RunGuard {
    fn drop(&mut self) {
        self.running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.id);
    }
}

impl Engine {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            locks: ResourceLocks::default(),
            running: Default::default(),
            capacity: Arc::new(Semaphore::new(2)),
        }
    }
    pub async fn launch(
        &self,
        store: Store,
        run: RunId,
        python: PathBuf,
        commands: Vec<Vec<String>>,
        concurrency: usize,
    ) -> Result<(), String> {
        self.launch_with_live(store, run, python, commands, concurrency, None)
            .await
    }
    pub async fn launch_with_live(
        &self,
        store: Store,
        run: RunId,
        python: PathBuf,
        commands: Vec<Vec<String>>,
        concurrency: usize,
        live: Option<crate::live::LiveConfig>,
    ) -> Result<(), String> {
        if !python.is_absolute()
            || !python.is_file()
            || !(1..=2).contains(&concurrency)
            || commands.is_empty()
            || commands.iter().any(|c| c.is_empty())
        {
            return Err("absolute Python executable, exact test argv approvals and concurrency 1..2 required".into());
        }
        let record = store.get_run(&run).await.map_err(|e| e.to_string())?;
        let trust = if store
            .project_is_trusted(&record.project_id)
            .await
            .map_err(|e| e.to_string())?
        {
            crate::sandbox::Trust::TrustedLocal
        } else {
            crate::sandbox::Trust::Untrusted
        };
        crate::sandbox::authorize(trust, None).map_err(str::to_owned)?;
        if record.state != RunState::Running {
            return Err("run is not approved and running".into());
        }
        let project = store
            .get_project(&record.project_id)
            .await
            .map_err(|e| e.to_string())?;
        let repo = register_repo(&project.repo_path)
            .await
            .map_err(|e| e.to_string())?;
        for task in store.list_tasks(&run).await.map_err(|e| e.to_string())? {
            if let Some(config) = &live {
                config.select(&task)?;
            }
            for criterion in &task.spec.acceptance {
                match criterion {
                    harness_core::domain::Acceptance::Test { command }
                        if commands.contains(command) => {}
                    _ => {
                        return Err(
                            "each acceptance must be an explicitly approved executable test argv"
                                .into(),
                        );
                    }
                }
            }
        }
        let live = if let Some(config) = live {
            let key = std::env::var("OPENAI_API_KEY")
                .map_err(|_| "live execution requires a daemon-scoped API key")?;
            if key.trim().is_empty() {
                return Err("empty provider credentials".into());
            }
            harness_core::redaction::register_secret(&key);
            Some((config, Arc::new(key)))
        } else {
            None
        };
        let options = ExecutionOptions {
            python,
            commands,
            concurrency,
            live,
        };
        {
            let mut active = self.running.lock().unwrap_or_else(|e| e.into_inner());
            if !active.insert(run.to_string()) {
                return Err("run already executing".into());
            }
        }
        let guard = RunGuard {
            running: self.running.clone(),
            id: run.to_string(),
        };
        let engine = self.clone();
        tokio::spawn(async move {
            let _guard = guard;
            // Failure details remain in task state/evidence. No prompts or private data are logged.
            if engine.drive(&store, &run, repo, options).await.is_err() {
                eprintln!("harness: execution stopped; inspect task state and verifier evidence");
            }
        });
        Ok(())
    }
    async fn drive(
        &self,
        store: &Store,
        run: &RunId,
        repo: TrustedRepo,
        options: ExecutionOptions,
    ) -> Result<(), String> {
        let manager = Arc::new(
            WorktreeManager::new(&repo, &self.root.join("tasks").join(run.as_str()))
                .map_err(|e| e.to_string())?,
        );
        let integrator = Arc::new(
            Integrator::new(
                &repo.root,
                &self.root.join("integration"),
                run.as_str(),
                &repo.head_sha,
                options.commands.clone(),
            )
            .map_err(|e| e.to_string())?,
        );
        let mut pool = WorkerPool::new(options.concurrency, 2, 2, self.locks.clone())
            .map_err(str::to_owned)?;
        loop {
            let state = store.get_run(run).await.map_err(|e| e.to_string())?.state;
            if state == RunState::Cancelled {
                pool.cancel_run(run);
            }
            if state == RunState::Running {
                let now = unix_seconds();
                pool.dispatch(
                    store,
                    run,
                    now,
                    |task| {
                        options.live.as_ref().map_or(
                            BudgetRequest {
                                estimated_microdollars: 0,
                                run_cap_microdollars: 0,
                                daily_cap_microdollars: 0,
                            },
                            |(config, _)| BudgetRequest {
                                estimated_microdollars: (task.spec.limits.cost_cap_usd
                                    * 1_000_000.0)
                                    .floor()
                                    as u64,
                                run_cap_microdollars: config.run_cap_microdollars,
                                daily_cap_microdollars: config.daily_cap_microdollars,
                            },
                        )
                    },
                    |task, lease, cancel| {
                        let context = Execution {
                            store: store.clone(),
                            repo: repo.clone(),
                            python: options.python.clone(),
                            commands: options.commands.clone(),
                            live: options.live.clone(),
                            manager: manager.clone(),
                            integrator: integrator.clone(),
                            capacity: self.capacity.clone(),
                        };
                        async move { context.execute(task, lease, cancel).await }
                    },
                )
                .await
                .map_err(|e| e.to_string())?;
            }
            if pool.active() == 0 {
                let tasks = store.list_tasks(run).await.map_err(|e| e.to_string())?;
                if !tasks.is_empty() && tasks.iter().all(|t| t.state == TaskState::Completed) {
                    let record = store.get_run(run).await.map_err(|e| e.to_string())?;
                    store
                        .transition_run(run, record.revision, RunState::Completed, None)
                        .await
                        .map_err(|e| e.to_string())?;
                }
                // Paused runs are restarted explicitly via execute after resume.
                return Ok(());
            }
            tokio::select! {
                _ = pool.join_next() => {},
                _ = tokio::time::sleep(Duration::from_millis(100)) => {},
            }
        }
    }
}

struct Execution {
    store: Store,
    repo: TrustedRepo,
    python: PathBuf,
    commands: Vec<Vec<String>>,
    manager: Arc<WorktreeManager>,
    integrator: Arc<Integrator>,
    capacity: Arc<Semaphore>,
    live: Option<(crate::live::LiveConfig, Arc<String>)>,
}
struct Broker {
    inner: RepoCommandBroker,
    task: Task,
    store: Store,
    lease: String,
    live: Option<crate::live::LiveBroker>,
}
impl ToolBroker for Broker {
    async fn handle(&mut self, request: &ToolRequest) -> ToolResponse {
        if request.tool == "model.authorize" {
            return self.live.as_mut().map_or(
                ToolResponse {
                    ok: false,
                    result: None,
                    error: Some("live calls disabled".into()),
                },
                |live| live.authorize(request),
            );
        }
        if request.tool == "model.complete" {
            let Some(live) = self.live.as_mut() else {
                return ToolResponse {
                    ok: false,
                    result: None,
                    error: Some("live calls disabled".into()),
                };
            };
            let call_id = uuid::Uuid::new_v4().to_string();
            if self
                .store
                .begin_provider_call(&call_id, &self.lease)
                .await
                .is_err()
            {
                return ToolResponse {
                    ok: false,
                    result: None,
                    error: Some("provider call audit failed".into()),
                };
            }
            let (response, usage) = live.complete(request).await;
            if let Some(usage) = usage
                && self
                    .store
                    .record_usage(&self.lease, &call_id, &usage)
                    .await
                    .is_err()
            {
                return ToolResponse {
                    ok: false,
                    result: None,
                    error: Some("could not persist observed provider usage".into()),
                };
            }
            if self.store.finish_provider_call(&call_id).await.is_err() {
                return ToolResponse {
                    ok: false,
                    result: None,
                    error: Some("provider call settlement audit failed".into()),
                };
            }
            return response;
        }
        if request.tool == "task.describe" {
            let mut description = serde_json::to_value(&self.task.spec).expect("task serializes");
            if let Some(live) = &self.live {
                description["runtime"] = live.runtime_context();
            }
            return ToolResponse {
                ok: true,
                result: Some(description),
                error: None,
            };
        }
        self.inner.handle(request).await
    }
    async fn record_usage(&mut self, id: uuid::Uuid, usage: &Usage) -> Result<(), String> {
        if self.live.is_some() {
            return Ok(());
        }
        self.store
            .record_usage(&self.lease, &id.to_string(), usage)
            .await
            .map_err(|e| e.to_string())
    }
}
impl Execution {
    async fn execute(
        self,
        mut task: Task,
        lease: String,
        cancel: watch::Receiver<bool>,
    ) -> Result<(), String> {
        let _permit = self.capacity.acquire().await.map_err(|e| e.to_string())?;
        if *cancel.borrow() {
            return Err("cancelled before spawn".into());
        }
        let base = git(
            &self.repo.root,
            &["rev-parse", "--verify", self.integrator.branch()],
        )
        .await
        .unwrap_or_else(|_| self.repo.head_sha.clone());
        let worktree = self
            .manager
            .create(
                task.run_id.as_str(),
                task.id().as_str(),
                task.attempt,
                base.trim(),
            )
            .await
            .map_err(|e| e.to_string())?;
        let tools = vec![
            "task.describe".into(),
            "repo.read".into(),
            "repo.patch".into(),
            "cmd.run".into(),
            "repo.glob".into(),
            "repo.search".into(),
            "model.authorize".into(),
            "model.complete".into(),
        ];
        let inner = RepoWriteBroker::new(&worktree.path, &tools, &task.spec.candidate_paths)
            .map_err(|e| e.to_string())?;
        let inner = RepoCommandBroker::new(inner, &worktree.path, &tools, CommandPolicy::default())
            .map_err(|e| e.to_string())?
            .with_approval_store(
                self.store.clone(),
                task.run_id.clone(),
                task.id().clone(),
                task.attempt,
            );
        let live = self
            .live
            .as_ref()
            .map(|(config, key)| {
                config.select(&task).map(|model| {
                    crate::live::LiveBroker::new(
                        model,
                        &task,
                        self.python.clone(),
                        key.as_ref().clone(),
                    )
                })
            })
            .transpose()?;
        let model = live.as_ref().map_or(
            ModelRef {
                provider: "mock".into(),
                model_id: "mock-coder".into(),
            },
            |l| ModelRef {
                provider: l.model.provider.clone(),
                model_id: l.model.model_id.clone(),
            },
        );
        let mut broker = Broker {
            inner,
            task: task.clone(),
            store: self.store.clone(),
            lease,
            live,
        };
        let mut spec = WorkerSpec::python_worker(self.python);
        spec.cwd = Some(worktree.path.clone());
        let mut worker = Worker::spawn(&spec).map_err(|e| e.to_string())?;
        let result: Result<(), String> = async {
            worker
                .handshake(Duration::from_secs(10))
                .await
                .map_err(|e| e.to_string())?;
            task = transition(&self.store, &task, TaskState::Running).await?;
            let start = StartTask {
                role: if self.live.is_some() {
                    "coder"
                } else {
                    "mock_coder"
                }
                .into(),
                worktree_id: worktree.branch,
                model,
                context_manifest_hash: "offline-task-description".into(),
                allowed_tools: tools,
                max_tool_calls: task.spec.limits.max_tool_calls,
                max_output_tokens: task.spec.limits.max_output_tokens,
                timeout_seconds: task.spec.limits.timeout_seconds,
                cost_cap_usd: task.spec.limits.cost_cap_usd,
            };
            let mut session = AssignmentSession::new(
                task.run_id.as_str(),
                task.id().as_str(),
                task.attempt,
                start.max_tool_calls,
            );
            let limits = supervisor::Limits {
                idle_timeout: Duration::from_secs(10),
                deadline: Duration::from_secs(u64::from(start.timeout_seconds)),
                max_messages: start.max_tool_calls.saturating_mul(3).saturating_add(20),
            };
            match supervisor::supervise(
                &mut worker,
                &mut session,
                &start,
                limits,
                &mut broker,
                cancel,
            )
            .await
            .map_err(|e| e.to_string())?
            {
                Outcome::Completed(_) => Ok(()),
                Outcome::Failed(_) => Err("worker failed".into()),
            }
        }
        .await;
        worker.shutdown(Duration::from_millis(500)).await;
        result?;
        task = transition(&self.store, &task, TaskState::Verifying).await?;
        let checks: Vec<Vec<String>> = task
            .spec
            .acceptance
            .iter()
            .filter_map(|a| match a {
                harness_core::domain::Acceptance::Test { command } => Some(command.clone()),
                _ => None,
            })
            .collect();
        let verifier = Verifier::new(self.commands);
        let gate = DiffGate::new(&task.spec.candidate_paths).map_err(|e| e.to_string())?;
        TaskCommitter::new(&verifier, &gate)
            .commit(&self.store, &task, &worktree.path)
            .await
            .map_err(|e| e.to_string())?;
        task = transition(&self.store, &task, TaskState::Reviewing).await?;
        self.integrator
            .with_checks(checks)
            .integrate(&self.store, &task)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}
async fn transition(store: &Store, task: &Task, state: TaskState) -> Result<Task, String> {
    store
        .transition_task(&task.run_id, task.id(), task.revision, state, None)
        .await
        .map(|r| r.task)
        .map_err(|e| e.to_string())
}
pub fn unix_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

pub fn default_python(root: &Path) -> PathBuf {
    if cfg!(windows) {
        root.join("Scripts/python.exe")
    } else {
        root.join("bin/python")
    }
}
