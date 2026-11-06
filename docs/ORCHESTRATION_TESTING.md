# Main-agent orchestration target and baseline

Baseline checked on 2026-11-05.

The target is one main agent that takes a user task, decomposes long work into
bounded subagent tasks, selects suitable models automatically, and delegates
independent work across the user's configured providers. The main agent consumes
worker results and decides whether more work is needed. Rust retains enforcement
of permissions, concurrency, dependencies, verification, and spend caps.

## Verified foundations

- `cargo test --locked --offline --workspace`: passed.
- `uv run --locked --offline --project intelligence pytest -q`: 105 passed.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 100`: passed;
  real worker processes, unique task/integration commits, verification evidence,
  ordered events, released leases, and a clean original checkout.
- The offline demo with `--tasks 1 --fault`: passed; restarting after a crash
  blocked uncertain attempts without replaying them.
- The offline demo with `--tasks 1 --cancel`: passed; cancellation during
  verification retained changes without making a commit.
- `cargo test --locked --offline -p harness-runtime --test provider_settings`:
  5 passed, including new coverage for automatically selecting low/medium/high
  models across three configured providers, high-risk escalation, a review route
  override, and rejecting a pinned model with incomplete capability metadata.

No paid provider generation was performed. Routing tests validate selection and
policy, not model quality or successful remote completion. The 100-task demo uses
mock coding actions and a prebuilt plan; it does not demonstrate a reasoning main
agent or dynamic task decomposition.

## Native UI live smoke test

A subsequent UI test on 2026-11-05 used the workspace `dist/Nova.app`, with
the isolated project `/private/tmp/nova-ui-agent-test`. The installed Applications
bundle timed out when selected through UI automation; the workspace bundle opened.
No sign-in screen was presented.

Through the native UI, the tester registered the project, entered a goal and one
worker task, saved and approved the plan, then started live execution. The task
requested `greeting.py` with `greet(name)` returning `Hello, {name}!`, with two
assertions executed using `/usr/bin/python3`. The task cost cap was $0.50.

- Run: `a0406ac9-63a2-48b1-9ebf-a52cf7206c38`.
- Task: `task-238cb0c0`.
- The UI selected the saved route `openai/gpt-6.1-sol`.
- Activity showed `ready → leased → running → blocked` (events 37–40).
- Usage showed three provider attempts with unknown token counts and costs.
- The worker worktree contained only the initial README; no greeting artifact
  or verified completion was produced.

Result: **failed end-to-end completion**. Worker launch through the UI worked,
but this does not prove automatic main-agent delegation: task scope was entered
manually. The failure's underlying HTTP/transport cause cannot be established
from persisted evidence: `provider_stdio.py` reduces errors to transient/permanent
categories, and `engine.rs` reduces a failed worker to `worker failed`; the blocked
transition payload is empty. Preserve a sanitized failure category/status in
durable evidence and display it in the UI before repeating paid smoke attempts.

The UI requires no user account login. Local daemon bearer authentication has
subsequently been removed; the CLI no longer loads a token file. Provider API
credentials are separate and remain required for live calls.

## Live worker path diagnosis (2026-11-05, follow-up)

`crates/harness-runtime/tests/live_e2e.rs` now drives the real engine, Python coder,
credential-scoped adapter, verifier, committer and integrator against a scripted
loopback OpenAI-compatible server (no network, no spend). Findings:

- The live coder loop itself works: read, patch, complete, verify, commit, integrate.
- Bug fixed: files created *by acceptance checks* (e.g. `__pycache__/`) were judged
  as out-of-scope worker changes, blocking every Python task whose test imports a
  module, and left the integration worktree dirty so the next task's integration
  failed. Check-created untracked files are now removed after checks run.
- Bug fixed: provider failures were collapsed to `transient_provider`, so the UI
  could only say "worker failed". The recorded `task.failure` reason now carries
  the HTTP status plus the provider's error code (e.g. `provider HTTP 429
  insufficient_quota`); provider messages and bodies are never stored.
- The failed UI run above blocked 3.4 s after three attempts. That timing implies a
  fast transient-class failure (HTTP 429/5xx or connection error), not a timeout or
  a schema rejection. Repeat one cheap live run to read the now-recorded reason.

## Main agent (goal -> subtasks)

`POST /v1/goals`, `nova goal PROJECT "GOAL"` and the panel's **New goal** sheet hand
a goal to the main agent (`crates/harness-runtime/src/orchestrator.rs`,
`intelligence/src/harness_ai/orchestrator.py`):

- Planning model: the `planning` route, else the high-tier ready model. One call,
  output capped, estimated cost must fit the run cap; credentials stay in the
  adapter process.
- Context: up to 300 tracked file names and a README excerpt; no other contents.
- Output: up to 8 subtasks, each with files it may edit, one test argv whose
  program must be an absolute runner found on this machine, complexity/risk, and
  dependencies. Python and Rust both validate; the store re-validates the DAG.
- The plan is only proposed. The operator approves it and the exact test commands
  in Execute review; subtasks then route to models by complexity and risk.
- Planning failures move the run to `failed` with a recorded reason.

Covered offline by `live_e2e::user_goal_is_decomposed_by_main_agent_and_completed_by_subagents`
(goal -> high-tier planner -> 2 subtasks routed to low/medium models -> verified,
integrated) and `api::goal_without_connected_provider_fails_with_visible_reason`.
Planning spend is recorded in the `planning.completed` event, not yet in
`usage_records` or the daily budget ledger.

## Gaps to close

1. ~~Wire the planner into a Rust-owned, budgeted orchestration session.~~ Done
   (single planning pass; see above).
2. ~~Bounded repository context.~~ Done (file names + README). The main agent
   rates complexity; Rust maps it to configured models rather than letting the
   model name model IDs.
3. Add a durable delegation proposal contract with parent task, child tasks,
   dependencies, acceptance criteria, and allocated budgets. Current plans are
   immutable; extending a running plan needs an explicit persisted operation.
4. Support bounded expansion of long tasks with limits on depth, total task count,
   concurrency, elapsed time, and aggregate spend. Prevent duplicate delegation
   after retries or restart.
5. Return verified worker results to the main agent for follow-up decisions and
   final synthesis. Preserve lineage and model-selection reasons in events/UI.

## Acceptance scenarios for the target

- A short user task creates one worker; a multi-part task creates independent
  workers plus dependency-ordered integration work.
- The main agent receives a long-task result, proposes children, and resumes
  from their verified results without repeating completed work.
- A small task selects a cheaper ready model while a harder task selects a
  suitable higher tier; explicit user routes remain authoritative.
- Two configured provider fixtures receive only their assigned task and scoped
  credential. Recorded usage identifies each provider/model correctly.
- Invalid model choices, cyclic child dependencies, excess recursion, and
  exhausted parent/run budgets are rejected before worker dispatch.
- Cancellation and restart cover both planning calls and child creation;
  delegation and billing reservations remain idempotent.

Use local scripted provider fixtures for deterministic integration coverage,
then a separately budgeted live smoke run to verify actual provider behavior.
