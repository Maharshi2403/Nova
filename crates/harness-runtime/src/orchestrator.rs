//! Main orchestration agent: one user goal -> a validated, approval-gated subtask plan.
//!
//! Rust chooses the planning model, supplies bounded repository context and the exact
//! set of test runners, holds credentials, and re-validates everything the model
//! proposes. The resulting plan is only *proposed*: a human approves it (including the
//! exact test commands) before any subagent runs, and each subtask is then routed to a
//! model by its complexity and risk.
use crate::{git::git, live::invoke, provider_config::ProviderStore, worker::WorkerSpec};
use harness_core::{
    domain::{Acceptance, Complexity, Project, Risk, UseCase},
    messages::ToolRequest,
    plan::PlanProposal,
    routing::route,
};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

/// Upper bound on subtasks a single goal may expand into.
pub const MAX_SUBTASKS: usize = 8;
const MAX_LISTED_FILES: usize = 300;
const MAX_FILE_LIST_BYTES: usize = 8 * 1024;
const MAX_README_BYTES: usize = 6 * 1024;
const PLANNER_TIMEOUT_SECONDS: u64 = 120;
/// Planning prompt bound: role text, goal, rules, context snippets and schema.
const PLANNER_INPUT_BYTES: u64 = 64 * 1024;
const DEFAULT_TASK_CAP_USD: f64 = 0.5;
const RUNNER_NAMES: &[&str] = &[
    "python3",
    "python",
    "pytest",
    "node",
    "npm",
    "npx",
    "pnpm",
    "yarn",
    "bun",
    "deno",
    "cargo",
    "go",
    "make",
    "swift",
    "xcodebuild",
    "ruby",
    "bundle",
    "php",
    "java",
    "mvn",
    "gradle",
    "dotnet",
    "git",
];
const EXTRA_BIN_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"];

#[derive(Debug, Clone, serde::Serialize)]
pub struct PlannedGoal {
    pub plan: PlanProposal,
    /// Registry id (`provider/model`) of the planning model.
    pub model: String,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
}

/// Absolute paths of common test runners found on this machine, in a stable order.
pub fn test_runners() -> Vec<String> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    dirs.extend(EXTRA_BIN_DIRS.iter().map(PathBuf::from));
    let mut found = Vec::new();
    for name in RUNNER_NAMES {
        let exe = if cfg!(windows) {
            format!("{name}.exe")
        } else {
            (*name).to_owned()
        };
        if let Some(path) = dirs
            .iter()
            .filter(|d| d.is_absolute())
            .map(|d| d.join(&exe))
            .find(|p| p.is_file())
        {
            let path = path.to_string_lossy().into_owned();
            if !found.contains(&path) {
                found.push(path);
            }
        }
    }
    found
}

/// Tracked file names (bounded) and a README excerpt. Never other file contents.
async fn repository_context(repo: &Path) -> serde_json::Value {
    let listing = git(repo, &["ls-files", "-z"]).await.unwrap_or_default();
    let mut files = String::new();
    let mut total = 0;
    for path in listing.split('\0').filter(|p| !p.is_empty()) {
        total += 1;
        if total <= MAX_LISTED_FILES && files.len() + path.len() < MAX_FILE_LIST_BYTES {
            files.push_str(path);
            files.push('\n');
        }
    }
    if total > MAX_LISTED_FILES || files.len() + 64 >= MAX_FILE_LIST_BYTES {
        files.push_str(&format!("... ({total} tracked files in total)\n"));
    }
    let readme = std::fs::read(repo.join("README.md"))
        .map(|bytes| {
            let end = bytes.len().min(MAX_README_BYTES);
            String::from_utf8_lossy(&bytes[..end]).into_owned()
        })
        .unwrap_or_default();
    serde_json::json!({ "files": files, "readme": readme })
}

/// Ask the configured planning model to decompose `goal`. `settings_root` holds
/// `providers.json`. Nothing is persisted here.
pub async fn plan_goal(
    settings_root: &Path,
    project: &Project,
    goal: &str,
    run_id: &str,
) -> Result<PlannedGoal, String> {
    let store = ProviderStore::new(settings_root);
    let settings = store.load()?;
    let python = store.python(None)?;
    let live = settings.live_config()?;
    let policy = live.roles.get(&UseCase::Planning).unwrap_or(&live.routing);
    let selected =
        route(&live.registry, policy, Complexity::High, Risk::Medium).map_err(|e| e.to_string())?;
    let model = live
        .registry
        .require(&selected.model_id, None, true)
        .map_err(|e| e.to_string())?
        .clone();
    let run_cap = live.run_cap_microdollars as f64 / 1_000_000.0;
    let max_output = model.max_output_tokens.min(8192);
    let estimate = (PLANNER_INPUT_BYTES as f64 * model.input_usd_per_million_tokens
        + f64::from(max_output) * model.output_usd_per_million_tokens)
        / 1_000_000.0;
    if !estimate.is_finite() || estimate > run_cap {
        return Err("planning call could exceed the run spend cap".into());
    }
    let runners = test_runners();
    if runners.is_empty() {
        return Err("no test runner (python3, node, cargo, ...) found on this machine".into());
    }
    let connections = store.connections(&live)?;
    let connection = connections
        .get(&model.provider)
        .ok_or("planning model provider is not connected")?;
    let task_cap = DEFAULT_TASK_CAP_USD
        .min(run_cap / MAX_SUBTASKS as f64)
        .max(0.01);
    let request = ToolRequest {
        tool: "plan.goal".into(),
        arguments: serde_json::json!({
            "project_id": project.id.as_str(),
            "goal": goal,
            "model_id": model.model_id,
            "repository": repository_context(Path::new(&project.repo_path)).await,
            "test_runners": runners,
            "max_tasks": MAX_SUBTASKS,
            "task_cost_cap_usd": task_cap,
            "max_output_tokens": max_output,
            "timeout_seconds": PLANNER_TIMEOUT_SECONDS,
        })
        .as_object()
        .expect("object literal")
        .clone(),
    };
    let mut spec = WorkerSpec::python_worker(python);
    spec.args = ["-I", "-m", "harness_ai", "provider"]
        .map(Into::into)
        .to_vec();
    // Only this adapter process receives the credential; its cwd is never the repository.
    spec.env = connection.profile.adapter_env(&connection.key);
    let response = invoke(
        spec,
        &request,
        run_id,
        "main-agent",
        1,
        Duration::from_secs(PLANNER_TIMEOUT_SECONDS + 10),
    )
    .await
    .map_err(|e| format!("main agent: {e}"))?;
    if !response.ok {
        let error = response.error.unwrap_or_default();
        let detail = error.split_once(": ").map_or(error.as_str(), |(_, d)| d);
        return Err(format!("main agent: {detail}"));
    }
    let result = response.result.ok_or("main agent returned no plan")?;
    if result["provider"].as_str() != Some(model.provider.as_str()) {
        return Err("main agent response came from an unexpected provider".into());
    }
    let plan: PlanProposal = serde_json::from_value(result["plan"].clone())
        .map_err(|_| "main agent returned a malformed plan")?;
    validate(&plan, project, &runners)?;
    let input_tokens = result["usage"]["input_tokens"].as_u64();
    let output_tokens = result["usage"]["output_tokens"].as_u64();
    Ok(PlannedGoal {
        plan,
        cost_usd: input_tokens.zip(output_tokens).map(|(i, o)| {
            (i as f64 * model.input_usd_per_million_tokens
                + o as f64 * model.output_usd_per_million_tokens)
                / 1_000_000.0
        }),
        model: model.id,
        input_tokens,
        output_tokens,
    })
}

/// Rust-side checks independent of the Python conversion.
pub fn validate(plan: &PlanProposal, project: &Project, runners: &[String]) -> Result<(), String> {
    if plan.project_id != project.id.as_str() {
        return Err("main agent plan targets a different project".into());
    }
    if plan.tasks.is_empty() || plan.tasks.len() > MAX_SUBTASKS {
        return Err(format!(
            "main agent must propose 1..{MAX_SUBTASKS} subtasks"
        ));
    }
    plan.validate()
        .map_err(|e| format!("main agent plan: {e}"))?;
    for task in &plan.tasks {
        if task.candidate_paths.is_empty() || task.acceptance.is_empty() {
            return Err(format!("subtask {} lacks scope or a test", task.id));
        }
        for acceptance in &task.acceptance {
            match acceptance {
                Acceptance::Test { command }
                    if command.first().is_some_and(|p| runners.contains(p)) => {}
                _ => {
                    return Err(format!(
                        "subtask {} test must start with an available test runner",
                        task.id
                    ));
                }
            }
        }
    }
    Ok(())
}
