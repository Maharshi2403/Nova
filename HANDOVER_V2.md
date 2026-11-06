# Nova Harness: Feature Improvement Handover (v2)

Audit baseline: `main` at `3f4c4ec` (H-072) plus the **uncommitted working tree**
(main-agent orchestrator, live-failure diagnostics, removal of daemon auth).
Audit date: 2026-11-06. Line references are approximate. Re-locate symbols by name
before editing.

Companion file: [`AGENT_PROMPT_V2.md`](AGENT_PROMPT_V2.md). Give it to every
implementing agent together with one ticket ID from this file.

---

## 0. Product target

Nova should take a goal or bug report and then:

1. decompose it into bounded, dependency-ordered subtasks run in parallel where
   their file scopes are disjoint;
2. code each subtask with a model matched to its complexity and risk;
3. test it, with real commands executed by Rust and never trusting what a model
   says;
4. review it, with an independent reviewer agent plus a human gate for risky diffs;
5. commit and integrate it, then land and push only with approval;
6. debug and fix bugs by reproducing them (red), localizing the cause, fixing
   (green) and adding a regression test;
7. generate documentation and READMEs that are verified, not just written.

All of this runs under hard budgets, deny-by-default tools, and durable evidence
for every claim.

## 1. Current state (verified)

### What works
- **Rust is the single state authority.** SQLite WAL, a validated task state
  machine, an append-only event log, leases, budgets and approvals.
- **Rust↔Python JSONL protocol** with handshake, framing caps and a supervisor.
- **Isolated Git worktrees per task.** Brokered read/write/command tools with
  path, symlink and secret guards.
- **Verify, diff-gate, commit (idempotent trailer), then serial `merge --no-ff`
  integration.**
- **Providers:** OpenAI Responses, Anthropic Messages, OpenAI-compatible, and
  mock. Retries with jitter; credentials are scoped to the adapter subprocess.
- **Routing** by use case, complexity and risk, across saved providers.
- **Offline demos:** 100-task load, crash/fault and cancel scenarios. CI runs on
  Windows, macOS and Linux.
- **WIP:** a one-shot main agent turns a goal into up to 8 subtasks
  (`POST /v1/goals`, `nova goal`).

### Baseline test result (this machine)
- Python: 117 passed.
- Rust: 1 failure, `harness-runtime --test context ::
  symbol_located_and_secret_path_excluded`. Cause: `rg` is not installed and
  the fallback path in `context.rs` (~L128) reads only the first 40 lines, so it
  cannot find symbols further down. This is a real bug, not just the
  environment. CI passes only because the runners have `rg`. Fixed by H-074.

### Critical findings (fix before any other feature work)

| # | Severity | Finding | Evidence |
|---|---|---|---|
| C1 | **Critical** | The WIP removes daemon bearer auth. Any local process can call `POST /v1/runs/{id}/execute` with a **caller-chosen absolute `python` path**. That interpreter then runs the provider adapter **with the stored API key in its env**: arbitrary code execution plus key theft. It can also self-trust projects, self-approve plans and approvals, and set its own spend caps. | `apps/daemon/src/lib.rs` `reject_browser_requests` (~L404) is the only guard; `engine.rs` ~L131 accepts any absolute file as `python`; `live.rs` ~L184-188 puts the key in that interpreter's env |
| C2 | High | No Host-header allowlist, so a DNS-rebinding page can read `/v1/dashboard`, events, usage, plans and evidence. | `lib.rs` router |
| C3 | High | Heartbeats count toward `max_messages = max_tool_calls*3+20`. With a 2 s heartbeat, a 900 s task is killed with `MessageFlood` after about 10 minutes. | `supervisor.rs` ~L163; `engine.rs` ~L556; `coder.py` ~L41 |
| C4 | High | `model.authorize` and `model.complete` are ToolRequests that count toward `max_tool_calls`. About ⅓ of the planned turns actually run, then the attempt is hard-killed. | `supervisor.rs` ~L189; `coder.py` ~L71 |
| C5 | High | A failed integration final check leaves the merge commit on `harness/integration/<run>`, and later tasks branch from that broken tip. | `integration.rs` ~L188-199; `engine.rs` ~L451 |
| C6 | High | When remaining tasks are Blocked or cannot be dispatched, `drive` returns Ok and the run stays `Running` forever. | `engine.rs` ~L321-331 |
| C7 | High | The model call in `BrokerProvider.complete` is a synchronous blocking stdin read inside an async function. Heartbeats and `asyncio.timeout` stall for the whole call. | `provider_stdio.py` ~L39; `assignment.py` ~L119 |
| C8 | Med-High | The pool failure path uses `?` before `release_lease` and `settle_usage`, so leases and budget reservations leak. | `pool.rs` ~L127-160 |
| C9 | Med-High | Path locks compare literal prefixes, but scopes are globs. `src/*.py` and `src/app.py` can run in parallel. | `locks.rs` ~L48 |
| C10 | Med-High | A daemon crash during background goal planning leaves the run in `Planning` forever. | `lib.rs` ~L716 `tokio::spawn`; recovery only covers tasks |
| C11 | Med | A whole-file `content` rewrite is allowed while reads are cut to ~2 KB, so models can silently truncate files. | `broker/write.rs` ~L98; `context.py` ~L115 |
| C12 | Med | Anthropic usage: a null `cache_creation_input_tokens` raises `TypeError`, which is treated as a non-retryable error. The header `anthropic-version: 2026-10-21` must be checked against the official docs. | `providers/multi.py` ~L85, ~L159 |
| C13 | Med | Planner "test runners" include `git`, `make` and `npm`, so a plan "test" like `git push` passes validation. | `orchestrator.rs` ~L29-53 |
| C14 | Med | The data dir and DB are created with the default umask (probably world-readable). | `lib.rs` ~L58 |

### Capability gaps against the product target
- **No review stage.** `Reviewing` is a pass-through. The review and debugging
  use cases only change model routing.
- **No closed loop.** A failed check makes the task Blocked forever. Test output
  is never fed back to the coder, and `RetryWait` is never produced.
- **The coder cannot run tests.** `CommandPolicy::default()` grants no prefixes
  (`engine.rs` ~L479).
- **The coder forgets almost everything.** It sees only the last action and
  2 KB of the last tool output. `context::retrieve` is never used by the engine.
- **Weak quality gates.** No lint, format or typecheck gate. Integration reruns
  only the current task's checks. Commits use `--no-verify`.
- **No debug/bug-fix workflow** and no docs role. Assertion-only acceptance is
  rejected.
- **The main agent plans once.** No exploration, no repair pass, no dynamic
  delegation (`subtask_requests` is unused), no final synthesis. Planning spend
  is not recorded in the ledger.
- **Parallelism is hard-capped at 2** (semaphore, run concurrency and provider
  capacity). Plans are limited to 8 subtasks.
- **No land, push or PR.** Work stops on `harness/integration/<run>`.
- **Ops gaps:** no `logs --follow`, `evidence`, `usage` or `runs` CLI commands.
  Store lists are unbounded. Worktrees are never GC'd. Lease expiry is not
  enforced at runtime.
- **CI gaps:** no Swift build, no docs-verifier run, no `cargo-deny` or
  `pip-audit`, unpinned actions. `check.sh` skips the test suites.

---

## 2. How agents work on this backlog

- **One ticket = one commit** (sub-tickets `H-0xxa/b` allowed if split *before*
  coding). Prescribed subject format: `type(H-0xx): imperative summary`.
- **Tested before commit.** Every ticket lists exact tests to add and the global
  gate. A commit is created only after the full gate passes (see AGENT_PROMPT_V2
  §4).
- **Evidence** is appended to `PROGRESS.md` under `## H-0xx — title`, with the
  exact commands and PASS/FAIL counts.
- **No paid provider calls, no push, no tags** unless the human explicitly
  authorizes them for that ticket.

### Parallel lanes
Agents may run **concurrently** only when (a) all dependencies are merged on
`main` and (b) their tickets are in **different lanes**. Lanes are defined by
file ownership, which avoids merge conflicts.

| Lane | Owns (primary files) |
|---|---|
| **S** Security / control plane | `apps/daemon/**`, `apps/cli/src/startup.rs`, `apps/cli/src/main.rs` (transport), `docs/TRUST_BOUNDARY.md`, `docs/CONTROL_API.md` |
| **R** Runtime core | `crates/harness-runtime/src/{engine,pool,supervisor,scheduler,integration,locks,recovery,worktree}.rs` |
| **Q** Quality gates | `crates/harness-runtime/src/{verifier,diff_gate,committer}.rs`, `broker/**` |
| **P** Python intelligence | `intelligence/**` |
| **D** Data / store | `crates/harness-store/**` (migrations), `crates/harness-core/**` (contracts, schemas) |
| **U** UX / CI | `apps/cli/src/{ui,providers,desktop}.rs`, `apps/desktop/**`, `.github/**`, `scripts/**`, `README.md`, `docs/*.md` (non-security) |

Cross-lane tickets list every lane they touch. They must run **alone** in each of
those lanes. Protocol/schema changes (Lane D + P + R) always run alone.

Each agent works in its own worktree: `git worktree add ../nova-H-0xx -b agent/H-0xx main`.
The **coordinator** (a human or lead agent) merges finished tickets into `main`
one at a time: rebase onto `main`, rerun the full gate, then `git merge --ff-only`.

### Recommended waves
| Wave | Tickets (concurrent within a wave if lanes differ) |
|---|---|
| 0 (serial) | H-073 → H-074 |
| 1 | H-075 (S) · H-077 (R) · H-086 (P) · H-096 (Q) · H-119 (D) · H-121 (U) |
| 2 | H-076 (S+R) · H-085 (P) · H-084 (Q) · H-122 (U) |
| 3 | H-078 (R+P) · H-095 (Q) · H-083 (S+D) · H-118 (U) |
| 4 | H-079 (R) · H-087 (P) · H-097 (Q+D+S) |
| 5+ | Pick the **lowest-numbered unblocked P0/P1 ticket** first, then P2 and P3. Assign a ticket to a free agent only if its lanes are not held by a running ticket. Critical paths: H-078 → H-089 → H-090 → H-101 → H-102 (review loop) and H-076 → H-097 → H-098 → H-115 → H-116 → H-117 (ship). |

---

## 3. Ticket specifications

Each ticket has the following fields:
- **Pri / Lane / Depends**
- **Problem:** the evidence for the change.
- **Implement:** bounded steps.
- **Tests:** names to add, which must fail before the change and pass after it.
- **Accept:** extra acceptance beyond the global gate.
- **Commit:** the prescribed subject.
- **Out of scope:** what not to change.

### Phase 0: Baseline and security (P0)

#### H-073: Land the in-progress working tree as reviewable commits
- **Pri** P0 · **Lane** all (run alone) · **Depends** —
- **Problem:** 36 modified and 4 untracked source files are uncommitted on
  `main`. They mix three concerns:
  - (a) live-failure diagnostics: failure reasons and check-artifact cleanup;
  - (b) main-agent goal orchestration;
  - (c) removal of daemon bearer auth.
- **Implement:**
  1. `git stash` nothing. Do not discard anything. First copy the working tree
     as a backup: `git diff > /tmp/nova-wip.patch` and copy the untracked files.
  2. Using `git add -p`, create:
     - **H-073a** `fix(H-073a): record sanitized provider and worker failure reasons`:
       provider_stdio/http_json/openai/multi error-code plumbing, store
       `record_task_failure`/`latest_task_failure`, pool failure category,
       evidence `failure` field, the related tests.
     - **H-073b** `fix(H-073b): remove acceptance-check artifacts before diff gate`:
       `diff_gate.rs` `remove_check_artifacts`, committer/integration calls,
       `tests/live_e2e.rs` scripted loopback provider test.
     - **H-073c** `feat(H-073c): decompose user goals with a budgeted main agent`:
       `orchestrator.rs`, `orchestrator.py`, `test_orchestrator.py`,
       `/v1/goals` routes, `nova goal`/`goal-status`, panel "New goal" sheet,
       `docs/ORCHESTRATION_TESTING.md`.
     - Auth-removal hunks that cannot be separated from H-073c go in H-073c.
       State plainly in that commit body: "Known regression: control API is
       unauthenticated until H-075."
  3. Do **not** delete `COMPLETE_HANDOFF.md`, `TASK_BACKLOG.md`,
     `MASTER_IMPLEMENTER_PROMPT.md`, `.DS_Store`. Add `.DS_Store` to `.gitignore`
     in H-073a.
- **Tests:** none new. Each sub-commit must build, and the full gate must pass at
  H-073c, apart from the known `context` failure if `rg` is absent. Record that
  failure as pre-existing.
- **Accept:** `git status` shows no modified tracked source files, and
  `git diff HEAD /tmp` against the backup is empty.
- **Out of scope:** any behavior change.

#### H-074: Find symbols without ripgrep
- **Pri** P0 · **Lane** R · **Depends** H-073
- **Problem:** without `rg`, `context::retrieve` reads only lines 1–40 and misses
  symbols. The test fails on this machine.
- **Implement:** in `context.rs`, when `rg` is unavailable, do a bounded
  pure-Rust line scan through the read broker's policy:
  - same deny/secret rules;
  - same byte cap;
  - stop at the first match per symbol;
  - max file size 2 MiB.

  Return a window around the match (±20 lines).
- **Tests:** `symbol_found_without_ripgrep`. Force the fallback with a
  `retrieve_with(…, use_ripgrep: false)` seam. The existing test must pass with
  and without `rg` on PATH.
- **Commit:** `fix(H-074): locate context symbols without ripgrep`

#### H-075: Restore authenticated local control plane
- **Pri** P0 · **Lane** S · **Depends** H-073
- **Problem:** C1, C2, C14 and daemon impersonation (`startup.rs` trusts any
  `{"status":"ok"}`).
- **Implement:**
  1. Data dir created `0700` (Unix). Keep Windows ACL behavior as before.
     `harness.db` is `0600`.
  2. A 256-bit random token in `<data>/daemon.token` (`0600`). Required as
     `Authorization: Bearer` on every `/v1/*` route; `/healthz` stays open but
     minimal. Compare in constant time (restore the removed implementation from
     git history: `git show 3f4c4ec:apps/daemon/src/lib.rs`).
  3. Host header allowlist: `127.0.0.1:<port>`, `localhost:<port>`,
     `[::1]:<port>`. Anything else returns 403 (DNS-rebinding defence). Keep
     `reject_browser_requests`.
  4. CLI `startup.rs` reads the token and verifies daemon identity:
     `GET /v1/control` must succeed with the token before reuse. Otherwise
     report "another process owns port".
  5. The desktop app keeps using the CLI stdin bridge. Verify it still works, and
     never pass the token through argv or URLs.
  6. Update `docs/CONTROL_API.md` (401 semantics), `TRUST_BOUNDARY.md` and
     ADR-002 (reverse the "no auth" amendment and record why).
- **Tests:**
  - daemon: `missing_token_401`, `wrong_token_401`, `foreign_host_header_403`,
    `rebinding_host_get_403`, `healthz_needs_no_token`;
  - Unix-only: `data_dir_is_0700`, `token_file_is_0600`;
  - cli: `startup_rejects_impersonating_listener`.
- **Commit:** `security(H-075): restore authenticated loopback control plane`

#### H-076: Daemon-owned interpreter and spend ceilings for execution
- **Pri** P0 · **Lane** S+R · **Depends** H-075
- **Problem:** the caller supplies `python`, `run_cap` and `daily_cap` in the
  execute request. The provider key flows into that interpreter.
- **Implement:**
  - The daemon resolves the worker interpreter itself, from the bundled runtime
    or a daemon config value set by `nova config set python PATH` (persisted,
    audited). Remove `python` from the execute request body, and reject it with
    422 if present.
  - Request caps may only **lower** the saved defaults. Saved defaults come from
    `nova budget set --run X --daily Y`, stored in settings and audited.
  - The adapter env holds only the provider key it needs, and only for that
    adapter. Assert this in a test using an env-dump fixture.
- **Tests:** `execute_rejects_caller_interpreter`, `request_cap_cannot_exceed_saved_cap`,
  `adapter_env_contains_only_scoped_key`.
- **Commit:** `security(H-076): own worker interpreter and spend ceilings in daemon`

#### H-077: Classify planner test commands through the approval policy
- **Pri** P0 · **Lane** R · **Depends** H-073
- **Problem:** C13.
- **Implement:**
  - Remove `git` from `RUNNER_NAMES`.
  - Run every planned argv through `harness_core::approval::classify`. Network,
    push, deploy and destructive classes are rejected in `orchestrator::validate`.
  - `make`/`npm`/`pnpm`/`yarn` are allowed only with a script/target argument,
    and are flagged `requires_attention` in the plan shown at approval.
- **Tests:** unit tests for `orchestrator::validate` (none exist):
  `git_push_rejected_as_test`, `npm_publish_rejected`, `pytest_allowed`,
  `relative_runner_rejected`, `cyclic_plan_rejected`, `absolute_or_parent_path_in_files_rejected`.
- **Commit:** `security(H-077): validate planned test commands against approval policy`

### Phase 1: Runtime correctness (P0)

#### H-078: Separate model turns, tool calls and heartbeats in supervision
- **Pri** P0 · **Lane** R+P · **Depends** H-073
- **Problem:** C3 and C4.
- **Implement:**
  - In `supervisor.rs`:
    - Heartbeats and Progress do **not** count toward `max_messages`. Instead,
      limit them to at most one per 500 ms. A faster stream is a flood.
    - `model.authorize` and `model.complete` count toward a new
      `max_model_turns` limit, which defaults to `max_tool_calls`, instead of
      `max_tool_calls`.
  - Add `max_model_turns` to `StartTask` (protocol minor field, default when
    absent) in Rust and Python.
  - In `coder.py`, loop over `max_model_turns`. When one turn remains, instruct
    the model to `completed`/`failed`, and end with
    `failed(reason="turn budget exhausted", retryable=true)` rather than being
    killed.
- **Tests:**
  - Rust: `heartbeats_do_not_exhaust_message_budget` (fake worker sending 1000
    heartbeats over a long deadline), `model_calls_do_not_consume_tool_budget`,
    `heartbeat_flood_rejected`.
  - Python: `coder_ends_cleanly_on_turn_budget`.
- **Commit:** `fix(H-078): budget model turns separately from tool calls and heartbeats`

#### H-079: Pool always releases leases; cancellation is terminal Cancelled
- **Pri** P0 · **Lane** R · **Depends** H-073
- **Problem:** C8. Cancelled tasks currently end up as `Blocked`.
- **Implement:**
  - In the pool job, run the failure bookkeeping, then always attempt
    `release_lease` and `settle_usage` (a finally pattern). Aggregate errors and
    return them.
  - Map the cancellation outcome to `TaskState::Cancelled`. Check the allowed
    transition table in `harness-core/src/state.rs` and extend it if needed,
    with a state-table test.
- **Tests:** `failure_bookkeeping_error_still_releases_lease` (inject a store
  error via a failing-store seam), `cancelled_task_is_cancelled_not_blocked`,
  `budget_reservation_settled_on_failure`.
- **Commit:** `fix(H-079): release leases on every pool exit and mark cancellations`

#### H-080: Quarantine failed integration merges
- **Pri** P0 · **Lane** R · **Depends** H-073
- **Problem:** C5.
- **Implement:**
  - When the final check fails, record the merge SHA in a durable
    `refs/harness/quarantine/<run>/<task>` ref and evidence.
  - Then reset the integration branch **inside the harness-owned integration
    worktree only** to its pre-merge SHA (`git reset --hard <pre>`, allowed
    because the worktree is harness-owned and clean before the merge).
  - Never touch user branches.
- **Tests:** `failed_final_check_restores_integration_tip`,
  `next_task_does_not_inherit_failed_merge`, `quarantine_ref_retains_failed_merge`.
  Update the existing assertion in `tests/integration.rs` (~L185) and document
  the change in PROGRESS.
- **Commit:** `fix(H-080): quarantine merges that fail integration checks`

#### H-081: Runs always reach a terminal or explained state
- **Pri** P0 · **Lane** R · **Depends** H-079
- **Problem:** C6.
- **Implement:** when `pool.active()==0` and no task can make progress:
  - tasks Blocked or Failed → the run moves to `Failed` with a reason
    (`blocked tasks: [...]`);
  - tasks undispatchable because of budget or a lock → the run stays `Running`,
    and a `run.stalled` event with the reason is emitted once.

  If the state machine lacks a needed state, add one (Lane D, coordinate).
  Show the reason in `nova status` and in the panel.
- **Tests:** `all_blocked_run_fails_with_reason`, `budget_denied_run_emits_stalled_once`.
- **Commit:** `fix(H-081): finish or explain runs that cannot progress`

#### H-082: Glob-aware path lock overlap
- **Pri** P1 · **Lane** R · **Depends** H-073
- **Problem:** C9.
- **Implement:**
  - Treat two scopes as overlapping when either glob can match a path the other
    matches. For each pair, compare the literal prefixes up to the first glob
    metacharacter; if either is a prefix of the other, they overlap.
  - Mismatched literal prefixes (`src/a/**` vs `docs/**`) are disjoint.
  - Be conservative: false positives serialize, which is safe.
- **Tests:** `glob_and_literal_overlap`, `disjoint_globs_parallel`, `double_star_overlaps_child`.
- **Commit:** `fix(H-082): detect overlapping glob scopes in path locks`

#### H-083: Recover runs interrupted during goal planning
- **Pri** P1 · **Lane** S+D · **Depends** H-073
- **Problem:**
  - C10.
  - The `planning.completed` event is written with `let _ =` outside the plan
    transaction.
  - `propose_plan` checks `Planning` state outside its transaction (TOCTOU).
- **Implement:**
  - At daemon start, move runs in `Planning` with no live planner to `Failed`,
    reason `planning interrupted`. The user may resubmit the goal.
  - Write the plan and the `planning.completed` event in one transaction.
  - Move the state check inside the `propose_plan` transaction.
- **Tests:** `restart_fails_orphaned_planning_run`, `plan_and_event_atomic`,
  `concurrent_propose_on_cancelled_run_rejected`.
- **Commit:** `fix(H-083): recover interrupted goal planning atomically`

#### H-084: Guard whole-file rewrites against truncation
- **Pri** P1 · **Lane** Q · **Depends** H-073
- **Problem:** C11.
- **Implement:** in `broker/write.rs`, a `content` rewrite of an **existing** file
  is allowed only if one of these holds:
  - (a) the file is ≤ the read cap; or
  - (b) the worker read the entire file in this attempt. Track the read ranges
    per path in the broker session.

  Otherwise deny with the message "use old/new edits or read whole file first".
  New files are unrestricted.
- **Tests:** `large_file_blind_rewrite_denied`, `rewrite_after_full_read_allowed`,
  `new_file_content_allowed`.
- **Commit:** `fix(H-084): prevent blind whole-file rewrites of large files`

### Phase 2: Provider and coder robustness (P0/P1, Lane P)

#### H-085: Non-blocking broker calls and clean cancellation in workers
- **Pri** P0 · **Lane** P · **Depends** H-073
- **Problem:**
  - C7.
  - A `cancel` frame arriving during `tool()` raises `AssignmentError` and the
    worker exits with code 2.
- **Implement:**
  - Run the blocking frame reader on a dedicated reader thread. It dispatches
    responses to awaiting futures by `request_id` and handles `cancel` by
    cancelling the main task.
  - `BrokerProvider.complete` awaits its future.
  - Heartbeats keep flowing during model calls.
  - On `cancel`, exit 0 after sending `failed(reason="cancelled")` if the
    protocol allows it.
- **Tests:** `heartbeat_continues_during_slow_model_call` (fake slow broker),
  `asyncio_timeout_interrupts_model_wait`, `cancel_frame_during_tool_exits_cleanly`.
- **Commit:** `fix(H-085): keep workers responsive during brokered model calls`

#### H-086: Anthropic usage null-safety and API version
- **Pri** P0 · **Lane** P · **Depends** H-073
- **Problem:** C12.
- **Implement:**
  - Treat `None` as 0 for cache token fields, but keep `input_tokens: None` as
    unknown.
  - Check the `anthropic-version` value against the official Anthropic API docs
    (`https://docs.anthropic.com` → API versioning). Do not guess.
  - Put the version in a single constant with a comment citing the doc and the
    check date.
- **Tests:** `anthropic_null_cache_fields_parse`, `anthropic_missing_usage_is_unknown`,
  `anthropic_non_end_turn_stop_reason_mapped`.
- **Commit:** `fix(H-086): parse Anthropic usage safely and pin documented API version`

#### H-087: Cache-friendly prompt layout and cache-aware cost
- **Pri** P1 · **Lane** P · **Depends** H-086
- **Problem:**
  - `_json(sort_keys=True)` puts volatile `previous_summary` before the stable
    `required` block, which defeats prefix caching.
  - Cost ignores cache discounts and the cache-write premium.
- **Implement:**
  - Fixed ordering: system role, then tools/contract, then task handoff
    (stable), then context snippets (semi-stable), then history and last result
    (volatile).
  - Anthropic: add `cache_control: {"type":"ephemeral"}` on the last stable
    block.
  - Cost: use per-model `cached_input` and `cache_write` prices from the
    registry when present. When they are missing, record the cost as
    **unknown**, never 0.
- **Tests:** `stable_prefix_identical_across_turns` (byte-compare the prefix of
  turn 1 and turn 3), `anthropic_cache_control_on_stable_block`,
  `cached_tokens_priced_at_cached_rate`.
- **Commit:** `perf(H-087): order prompts for prefix caching and price cached tokens`

#### H-088: Coder working memory with bounded compaction
- **Pri** P1 · **Lane** P · **Depends** H-078, H-085
- **Problem:** the coder sees only the last step plus 2 KB.
- **Implement:**
  - Keep a rolling transcript of `{action, tool, args summary, result excerpt}`.
  - Keep the last K=6 turns verbatim (each result ≤ 8 KB).
  - Compact older turns into a deterministic summary: files read (path and
    ranges), files changed, commands run with exit codes, and open hypotheses
    from the model's `summary`.
  - The total memory block is capped (default 24 KB) and the cap is enforced
    before sending.
  - Expose a `notes` field in the action schema for the model's scratchpad,
    capped at 2 KB.
- **Tests:** `memory_retains_changed_files_after_compaction`, `memory_block_within_cap`,
  `last_k_turns_verbatim`.
- **Commit:** `feat(H-088): give the coder bounded multi-turn working memory`

### Phase 3: Closed-loop coding (P1)

#### H-089: Coder may run its approved acceptance commands
- **Pri** P1 · **Lane** R+P · **Depends** H-078
- **Problem:** `CommandPolicy::default()` grants nothing, so the coder cannot test
  its own work.
- **Implement:**
  - Seed the per-task `CommandPolicy` with the **exact argv** of the task's
    human-approved acceptance commands and the project gate commands (H-097, if
    present).
  - Exact match only: no prefixes, no extra args.
  - Output is capped and the timeout is the task's timeout.
  - Coder prompt: "Before `completed`, run each acceptance command via `cmd.run`;
    if it fails, fix and rerun."
  - `claimed_tests` is populated, but Rust still re-verifies independently.
- **Tests:** `approved_acceptance_argv_runs_without_prompt`, `modified_argv_still_requires_approval`,
  python `coder_runs_tests_before_completing` (scripted provider).
- **Commit:** `feat(H-089): let coders run their approved acceptance checks`

#### H-090: Verification-failure feedback and bounded automatic retry
- **Pri** P1 · **Lane** R+D+P · **Depends** H-089, H-079
- **Problem:** a failed check makes the task Blocked forever, and failure output
  is never returned to the coder.
- **Implement:**
  - On verifier failure with `attempt < max_attempts` (default 3, configurable
    per run):
    - persist a `failure_digest`: argv, exit code and the last 4 KB of
      redacted stdout/stderr;
    - transition to `RetryWait` with exponential `retry_not_before`.
  - The scheduler honours `retry_not_before`. `scheduler.rs` currently passes
    `None`.
  - On the next attempt:
    - include the `previous_attempts` digests in `task.describe`;
    - create the worktree from the previous attempt's work branch (resume) or
      fresh (configurable; default fresh plus the digest).
  - After `max_attempts`, the task becomes Blocked with reason `exhausted retries`.
- **Tests:**
  - `failed_verification_schedules_retry_with_digest`, `retry_honours_not_before`,
    `max_attempts_blocks`;
  - live_e2e: `second_attempt_fixes_failing_test` (scripted provider fails, then
    fixes).
- **Commit:** `feat(H-090): retry failed verification with bounded failure feedback`

#### H-091: Manual retry of blocked tasks
- **Pri** P1 · **Lane** S+U · **Depends** H-090
- **Implement:**
  - `POST /v1/tasks/{run}/{task}/retry` with `{revision, note?}`, which moves
    Blocked to Ready, resets the attempt budget, and attaches the operator note
    to the next `task.describe`. It is audited.
  - `nova retry RUN TASK [--note TEXT]`.
  - A panel button on blocked tasks.
- **Tests:** `retry_blocked_task_requires_revision`, `retry_note_reaches_worker`,
  `retry_non_blocked_409`.
- **Commit:** `feat(H-091): let operators retry blocked tasks with guidance`

#### H-092: Seed coders with retrieved context
- **Pri** P1 · **Lane** R · **Depends** H-074, H-088
- **Problem:** the engine never calls `context::retrieve`, and
  `context_manifest_hash` is a constant.
- **Implement:**
  - Before spawning, call `retrieve(worktree, candidate_paths ∪ files named in
    the description, symbols extracted from the description, cap=32 KB)`.
  - Pass the snippets in `task.describe`.
  - Compute the manifest hash as sha256 over the sorted
    (path, range, content-sha) entries and persist it in evidence.
- **Tests:** `describe_includes_scoped_snippets`, `manifest_hash_changes_with_content`,
  `secret_paths_never_in_context`.
- **Commit:** `feat(H-092): seed coder tasks with bounded retrieved context`

#### H-093: Multi-hunk unified-diff patch tool
- **Pri** P2 · **Lane** Q+P · **Depends** H-084
- **Implement:**
  - New tool `repo.apply_patch {diff}`: unified diff, exact-context apply, no
    fuzz.
  - Every target path passes the write-scope checks, the patch is
    all-or-nothing, and the result reports the hunks applied.
  - Add it to the coder's tool list with an example.
- **Tests:** `multi_hunk_patch_applies_atomically`, `patch_with_bad_context_rejected_no_partial_write`,
  `patch_outside_scope_rejected`, `patch_creating_file_in_scope`.
- **Commit:** `feat(H-093): add atomic unified-diff patch tool`

#### H-094: Repository symbol map tool
- **Pri** P2 · **Lane** Q+P · **Depends** H-074
- **Implement:**
  - New read tool `repo.symbols {path_glob, kind?}` returning `name`, `kind`,
    `path` and `line` for definitions.
  - Use language regexes for Rust, Python, TS/JS, Go, Swift and Java. No new
    heavy dependencies; Tree-sitter is optional behind a feature flag and needs
    its own ticket.
  - Output is bounded and deny-listed paths are excluded.
- **Tests:** `symbols_found_per_language` (fixture per language), `symbols_output_capped`,
  `symbols_skip_secret_paths`.
- **Commit:** `feat(H-094): expose a bounded repository symbol map`

### Phase 4: Quality gates (P1, Lane Q)

#### H-095: Protect acceptance tests from the coder
- **Pri** P1 · **Lane** Q · **Depends** H-073
- **Problem:** the coder can weaken the tests it is judged by.
- **Implement:**
  - Files referenced by acceptance argv, plus files matching configured test
    globs (`tests/**`, `**/test_*.py`, `**/*_test.go`, `**/*.spec.*`), become
    **read-only** for a task unless the task declares `writes_tests: true`
    (schema field, default false).
  - The planner sets `writes_tests` for test-authoring tasks.
  - When `writes_tests` is true, the diff gate requires that the added tests
    fail on the base SHA (see H-105 for the mechanism; a first version may just
    record the evidence).
- **Tests:** `coder_cannot_modify_acceptance_test`, `writes_tests_task_may_add_tests`.
- **Commit:** `security(H-095): keep acceptance tests immutable for coding tasks`

#### H-096: Precise secret detection in the diff gate
- **Pri** P1 · **Lane** Q · **Depends** H-073
- **Problem:** the regex flags `password: Optional[str]`.
- **Implement:**
  - Match known key formats: AWS `AKIA…`, `sk-…`, `ghp_…`, `xox…`, PEM private
    key headers, `-----BEGIN`, and JWTs.
  - Also match assignments of quoted literals whose Shannon entropy is ≥ 3.5 and
    whose length is ≥ 20, to secret-like names.
  - Allow type annotations, env lookups (`os.environ[...]`, `env::var(...)`) and
    placeholders (`"changeme"`, `"<...>"`).
  - Findings report the path and line only, never the value.
- **Tests:** a table test of ≥ 15 true and false positives, plus
  `finding_never_echoes_secret`.
- **Commit:** `fix(H-096): detect secrets precisely without blocking type annotations`

#### H-097: Per-project quality-gate profile
- **Pri** P1 · **Lane** Q+D+S · **Depends** H-076
- **Implement:**
  - Project settings `gates: [{name, argv, applies_to_globs?}]`, for example
    `cargo fmt --check`, `ruff check`, `mypy`, `cargo clippy -D warnings`.
  - Approved once per project by the operator (audited), and re-approved when
    changed.
  - The verifier runs the applicable gates after the acceptance commands. A gate
    failure is a verification failure with its own digest (feeds H-090).
  - `nova gates set PROJECT --name fmt -- cargo fmt --all --check`.
  - Add an auto-detect suggestion helper (`nova gates suggest`) that only
    *suggests* gates and never approves them.
- **Tests:** `gate_failure_blocks_commit`, `unapproved_gate_not_run`, `gate_scoped_by_glob`.
- **Commit:** `feat(H-097): enforce operator-approved project quality gates`

#### H-098: Run-level final verification at integration
- **Pri** P1 · **Lane** R · **Depends** H-080, H-097
- **Problem:** integration reruns only the current task's checks.
- **Implement:**
  - After each merge, run the union of all completed tasks' acceptance commands
    plus the project gates (deduped).
  - After the last task, run the whole union once more, plus an optional
    `final_suite` argv approved at plan time.
  - The run becomes `Completed` only if this passes. Otherwise quarantine (H-080).
- **Tests:** `regression_in_earlier_task_check_blocks_integration`, `final_suite_required_for_completion`.
- **Commit:** `feat(H-098): verify the integrated run against all accepted checks`

#### H-099: Opt-in repository Git hooks at task commit
- **Pri** P3 · **Lane** Q · **Depends** H-097
- **Implement:**
  - A project setting `run_git_hooks: false` (default).
  - When it is true, drop `--no-verify`. Hook failure becomes a verification
    failure with a digest.
  - Hooks run with the same env scrubbing as commands, and are treated as
    trusted-project code (only allowed for `trusted_local`).
- **Tests:** `hooks_skipped_by_default`, `failing_pre_commit_hook_blocks_when_enabled`.
- **Commit:** `feat(H-099): optionally run repository commit hooks`

### Phase 5: Review agent (P1)

#### H-100: Reviewer protocol and Python reviewer role
- **Pri** P1 · **Lane** D+P · **Depends** H-087
- **Implement:**
  - Add protocol role `reviewer` (minor version bump with negotiation; update
    `schemas/` and fixtures).
  - Input: task spec, bounded diff (≤ 48 KB; split by file with a manifest when
    larger), verification evidence summary, and project conventions (README
    excerpt).
  - Output schema `ReviewVerdict {decision: approve|request_changes|escalate,
    findings:[{path,line,severity:blocker|major|minor|nit,category,message,suggestion?}],
    summary}`.
  - Read-only tools only: `repo.read`, `repo.search`, `repo.symbols`.
  - Prompt: correctness, security, tests, scope creep, docs. "Do not request
    stylistic nits as blockers."
- **Tests:**
  - python: `reviewer_returns_valid_verdict` (mock), `reviewer_has_no_write_tools`;
  - cross-language fixture round-trip for `ReviewVerdict`.
- **Commit:** `feat(H-100): define structured code-review agent`

#### H-101: Engine review stage with bounded change-request rounds
- **Pri** P1 · **Lane** R · **Depends** H-100, H-090
- **Implement:**
  - In `Reviewing` (currently a pass-through), run the reviewer, routed by the
    `review` use case and preferring a **different model/provider than the
    coder** when one is available.
  - The policy is per run: `review: off|risky|all` (default `risky` = medium
    and high risk).
  - `request_changes` with blocker or major findings sends the task back to the
    coder with the findings as a digest. This counts as an attempt; allow at
    most 2 review rounds.
  - `escalate` leads to H-102 human review.
  - Persist the verdict as evidence and show it in the panel and CLI.
- **Tests:** `risky_task_reviewed_before_integration`, `request_changes_returns_to_coder_with_findings`,
  `review_rounds_bounded`, `review_off_skips_stage`, and a live_e2e scripted
  reviewer.
- **Commit:** `feat(H-101): gate integration on independent agent review`

#### H-102: Human diff review and approval of ReviewRequired
- **Pri** P1 · **Lane** S+U · **Depends** H-101
- **Problem:** `ReviewRequired` (large or high-risk diffs) leads to Blocked with
  no way forward.
- **Implement:**
  - `GET /v1/tasks/{run}/{task}/diff` (bounded, paged by file).
  - `POST …/review {decision, revision, comment}`: approve continues to
    integration; reject sends the task back with the comment as feedback.
  - CLI `nova diff RUN TASK` and `nova review RUN TASK --approve|--reject -m TEXT`.
  - Panel diff viewer (Swift, plain text with +/- colouring).
- **Tests:** `review_required_task_awaits_human`, `human_approval_resumes_integration`,
  `human_reject_feeds_comment_to_coder`.
- **Commit:** `feat(H-102): let operators review and approve task diffs`

### Phase 6: Debugging and bug fixing (P2)

#### H-103: Bug intake with red reproduction evidence
- **Pri** P2 · **Lane** S+R+U · **Depends** H-090
- **Implement:**
  - `nova fix PROJECT "description" --repro -- ARGV…`, plus `POST /v1/bugs`.
  - Rust runs the repro argv on the pinned base (as an approved command) and
    **requires** a non-zero exit (red). The output digest is stored.
  - When the repro passes on the base, reject with "could not reproduce".
  - This creates a run whose main agent receives the bug description, the repro
    digest, and stack-trace-derived file:line candidates. Parse Python, Rust,
    JS and Go trace formats.
- **Tests:** `repro_must_fail_on_base`, `stack_trace_paths_extracted`, `non_reproducing_bug_rejected`.
- **Commit:** `feat(H-103): accept bug reports with verified failing reproductions`

#### H-104: Debugger role for fault localization
- **Pri** P2 · **Lane** P+D · **Depends** H-103, H-094
- **Implement:**
  - Role `debugger` with read, search and symbol tools, plus running the
    approved repro argv only.
  - Output `Diagnosis {root_cause, evidence:[{path,line,why}], fix_plan, confidence,
    suggested_regression_test}`.
  - The main agent turns the diagnosis into a coder task (scope = evidence
    paths) plus a `writes_tests` task for the regression test.
- **Tests:** a scripted-provider debugger localizes an off-by-one in a fixture
  repo; `debugger_cannot_write`.
- **Commit:** `feat(H-104): localize bug root causes with a debugger agent`

#### H-105: Fix verification (repro green, regression test red on base)
- **Pri** P2 · **Lane** Q+R · **Depends** H-104, H-095
- **Implement:**
  - A bug run completes only if both hold:
    - (a) the repro argv passes on the integrated tip;
    - (b) the new regression test **fails on the base SHA and passes on the
      tip**. Run it in a temporary worktree at the base with the test file
      copied in.
  - Record both as evidence.
- **Tests:** `fix_without_regression_test_not_completed`, `regression_test_passing_on_base_rejected`,
  `verified_fix_completes`.
- **Commit:** `feat(H-105): require red-green evidence for bug fixes`

### Phase 7: Documentation generation (P2)

#### H-106: Documentation use case with checker-based acceptance
- **Pri** P2 · **Lane** D+R+P · **Depends** H-073
- **Implement:**
  - Use case `documentation`, whose acceptance kind is `docs_check {paths,
    require_sections?, require_mermaid?}`. It runs the bundled
    `scripts/verify_documentation.py` under Rust's verifier. Replace the
    "assertion-only rejected" behavior for this kind only.
  - Writer prompt: audience, structure, verify every command and path against
    the repo, no invented APIs.
  - Write scope is limited to `**/*.md` plus the declared paths.
- **Tests:** `docs_task_verified_by_checker`, `docs_task_cannot_write_code`.
- **Commit:** `feat(H-106): generate documentation with verified acceptance`

#### H-107: Docs link, path and command verification
- **Pri** P2 · **Lane** U · **Depends** H-106
- **Implement:** extend `verify_documentation.py`:
  - relative links and anchors resolve;
  - referenced repo paths exist;
  - fenced `sh` commands reference real binaries and subcommands, checked
    against `nova --help` and `cargo metadata` where cheap.
  - Optional `--strict`.
- **Tests:** positive and negative cases in `tests/test_documentation_verifier.py`.
- **Commit:** `feat(H-107): verify documentation links paths and commands`

#### H-108: `nova docs` presets
- **Pri** P2 · **Lane** U+S · **Depends** H-106, H-107
- **Implement:**
  - `nova docs PROJECT --readme|--architecture|--api|--changelog [--paths …]`
    builds a goal with the preset structure and acceptance, and goes through the
    normal plan approval.
  - Changelog: built from the conventional commits since the last tag (read-only
    git).
- **Tests:** CLI golden tests per preset; `changelog_groups_commit_types`.
- **Commit:** `feat(H-108): add documentation generation presets`

### Phase 8: Orchestration and parallelism (P2)

#### H-109: Configurable concurrency and per-provider capacity
- **Pri** P2 · **Lane** R+S · **Depends** H-081, H-082
- **Problem:** the semaphore, run concurrency and provider capacity are all
  hard-coded to 2.
- **Implement:**
  - Daemon setting `max_workers` (1..16, default 4), run override ≤ the setting.
  - Per-provider `max_concurrent` (default 2), enforced in dispatch.
  - Lease TTL starts at worker start, not while waiting on the semaphore.
- **Tests:** `concurrency_cap_from_settings`, `provider_capacity_enforced`,
  `lease_ttl_not_consumed_while_queued`, plus the offline demo
  `--tasks 100 --concurrency 8` in CI.
- **Commit:** `feat(H-109): scale worker concurrency within configured limits`

#### H-110: Record planning spend in the usage ledger
- **Pri** P2 · **Lane** R+D · **Depends** H-083
- **Implement:**
  - Planning calls reserve against the run and daily budgets and write
    `usage_records` with `use_case=planning`.
  - The planner is denied before the call when the daily budget is exhausted.
- **Tests:** `planning_usage_recorded`, `planning_denied_when_daily_cap_spent`.
- **Commit:** `fix(H-110): account planning spend in run and daily budgets`

#### H-111: Planner exploration and validation-repair pass
- **Pri** P2 · **Lane** P+R · **Depends** H-110, H-094
- **Implement:**
  - The planner gets read-only tools (`repo.search`, `repo.symbols`, bounded
    `repo.read`) for ≤ N turns (default 6) before emitting its plan.
  - On validation failure, re-prompt once with the exact validation errors.
  - Fix the file-list truncation: send a directory-tree summary (counts per dir)
    plus the top files, rather than a list silently clipped at 8 KB.
- **Tests:** `planner_explores_before_planning` (scripted), `invalid_plan_repaired_once`,
  `second_invalid_plan_fails_with_errors`, `large_repo_tree_summarized`.
- **Commit:** `feat(H-111): let the main agent explore and repair its plan`

#### H-112: Durable plan amendments with lineage
- **Pri** P2 · **Lane** D · **Depends** H-111
- **Implement:**
  - Append-only `plan_amendments(run_id, version, parent_task_id, tasks_json,
    reason, status, idempotency_key)`.
  - A DAG re-validation over the combined plan.
  - Limits: depth ≤ 2, total tasks ≤ 64 per run, and an aggregate budget.
  - Approval is either human or **auto within the pre-approved envelope**: same
    or narrower paths, already-approved test argv, and within budget.
- **Tests:** `amendment_idempotent`, `amendment_cycle_rejected`, `depth_limit_enforced`,
  `auto_approve_only_within_envelope`.
- **Commit:** `feat(H-112): persist bounded plan amendments with lineage`

#### H-113: Worker subtask requests routed through the main agent
- **Pri** P2 · **Lane** R+P · **Depends** H-112
- **Implement:**
  - Coders may return `completed{subtask_requests}` or
    `failed{reason:"too_large", subtask_requests}`. The field already exists and
    is unused.
  - Rust forwards the requests to the main agent, which proposes an amendment
    (H-112).
  - Child results are linked to the parent, and the parent completes when all
    its children complete.
  - No duplicate delegation on retry or restart: use an idempotency key of
    `run`, `task`, `attempt` and a request hash.
- **Tests:** `large_task_split_into_children`, `children_dedup_after_restart`,
  `parent_completes_after_children`.
- **Commit:** `feat(H-113): delegate oversized tasks to child agents`

#### H-114: Main-agent run synthesis report
- **Pri** P2 · **Lane** P+R+U · **Depends** H-113
- **Implement:**
  - When a run reaches a terminal state, the main agent writes a report:
    - what changed (by task, with SHAs);
    - evidence (tests and gates);
    - review findings;
    - spend;
    - open risks and follow-ups.
  - Store it as an artifact and show it in `nova report RUN` and in the panel.
    It also becomes the PR body source (H-117).
- **Tests:** `report_cites_only_recorded_evidence` (every SHA and test in the
  report exists in the DB), `report_generated_offline_with_mock`.
- **Commit:** `feat(H-114): summarize completed runs from durable evidence`

### Phase 9: Ship (land, push, PR) (P2)

#### H-115: Land the integration branch onto a user branch
- **Pri** P2 · **Lane** R+S+U · **Depends** H-098
- **Implement:**
  - `nova land RUN --onto BRANCH [--squash]`, plus `POST /v1/runs/{id}/land`.
  - It requires a Completed run, final verification evidence, and a clean target
    checkout (or a target branch that is not checked out).
  - Fast-forward if possible; otherwise a merge commit. Never rebase or rewrite
    user history, and never force.
  - Approval is required (approval class `land`), and the action is audited.
- **Tests:** `land_fast_forwards_clean_branch`, `land_refuses_dirty_checkout`,
  `land_requires_completed_run`, `land_never_rewrites_target`.
- **Commit:** `feat(H-115): land verified runs onto user branches`

#### H-116: Approval-gated push
- **Pri** P2 · **Lane** R+S+U · **Depends** H-115
- **Implement:**
  - `nova push RUN --remote origin --branch NAME`. Every push needs a
    human-approved `push` approval (deny-by-default stays). Never `--force`, and
    refuse protected-name targets (`main`/`master`) unless `--allow-default-branch`
    is passed **and** approved.
  - Use the user's git credential helpers. Nova never stores git credentials.
  - Record the remote, ref and SHA in evidence.
- **Tests:** against a local bare-repo remote: `push_requires_approval`,
  `push_never_forces`, `default_branch_push_needs_explicit_flag`,
  `push_evidence_recorded`.
- **Commit:** `feat(H-116): push landed work only with explicit approval`

#### H-117: Opt-in pull request creation
- **Pri** P3 · **Lane** U+S · **Depends** H-116, H-114
- **Implement:**
  - `nova pr RUN --base main`. Only when `gh` is installed and authenticated;
    otherwise print the manual command.
  - The PR body comes from the H-114 report.
  - Approval is required, and the PR URL is recorded.
- **Tests:** `pr_command_built_with_report_body` (fake `gh` on PATH), `pr_requires_approval`,
  `missing_gh_prints_instructions`.
- **Commit:** `feat(H-117): open pull requests from verified runs`

### Phase 10: Operations, UX, store and CI (P2/P3)

#### H-118: CLI observability and safe URL building
- **Pri** P2 · **Lane** U · **Depends** H-075
- **Implement:**
  - New commands: `nova runs`, `nova projects [trust|remove]`,
    `nova evidence RUN TASK`, `nova usage [--run]`, `nova plan show RUN [V]`.
  - `nova logs --follow` uses the long-poll cursor.
  - Percent-encode every path and query segment. Reject CR/LF in IDs.
  - `--json` everywhere.
- **Tests:** a golden test per command, `crlf_in_id_rejected`, `logs_follow_resumes_cursor`.
- **Commit:** `feat(H-118): add CLI observability commands`

#### H-119: Store indices and pagination
- **Pri** P2 · **Lane** D · **Depends** H-073
- **Implement:**
  - Add a migration with indices on:
    - `events(run_id, task_id, kind)`
    - `provider_calls(lease_id)`
  - Add FK or trigger checks for `approvals.run_id`.
  - Add `LIMIT`/cursor to `list_tasks`, `list_usage` and `list_test_results`,
    and update the API with `?cursor&limit` (default 200, max 1000).
- **Tests:** `migration_adds_indices` (via `EXPLAIN QUERY PLAN` uses the index),
  `pagination_stable`, `limit_capped`.
- **Commit:** `perf(H-119): index hot queries and paginate store lists`

#### H-120: Worktree GC and runtime lease-expiry reconciliation
- **Pri** P2 · **Lane** R · **Depends** H-081
- **Implement:**
  - After a run is terminal and any land/quarantine is recorded, call
    `WorktreeManager::cleanup` for harness-owned worktrees only.
  - Branches are kept until `nova gc --older-than 14d`.
  - Run `recovery::reconcile` (expired leases) periodically, every 30 s, in the
    daemon.
- **Tests:** `terminal_run_worktrees_removed`, `user_worktrees_never_removed`,
  `expired_lease_reconciled_while_running`.
- **Commit:** `fix(H-120): clean up worktrees and reconcile expired leases`

#### H-121: CI hardening
- **Pri** P2 · **Lane** U · **Depends** H-073
- **Implement:** on macOS:
  - build Swift via `scripts/build_desktop.py --check`;
  - run `verify_documentation.py` on `docs/` and `README.md`.

  On all runners:
  - `cargo deny check` (licenses/advisories) and `uv run pip-audit`;
  - pin actions by SHA;
  - `concurrency: cancel-in-progress`;
  - Dependabot config;
  - install ripgrep explicitly **and** run one job without it (guards H-074).
- **Accept:** the workflow runs green on a fork or branch. Paste the run URL into
  PROGRESS.
- **Commit:** `ci(H-121): build desktop, audit dependencies and pin actions`

#### H-122: Local check scripts run the full gate
- **Pri** P3 · **Lane** U · **Depends** H-121
- **Implement:** `scripts/check.sh` and `check.ps1` run fmt, clippy, cargo test,
  ruff, mypy, pytest and the docs verifier. Add a `--fast` flag for the lint-only
  subset.
- **Commit:** `chore(H-122): run full quality gate from local check scripts`

---

## 4. Dependency summary

```
H-073 ─┬─ H-074 ─┬─ H-092
       │         └─ H-094 ─┬─ H-104
       │                   └─ H-111
       ├─ H-075 ─ H-076 ─ H-097 ─┬─ H-098 ─ H-115 ─ H-116 ─ H-117
       │                         └─ H-099
       ├─ H-077
       ├─ H-078 ─┬─ H-088
       │         └─ H-089 ─ H-090 ─┬─ H-091
       │                           ├─ H-101 ─ H-102
       │                           └─ H-103 ─ H-104 ─ H-105
       ├─ H-079 ─ H-081 ─┬─ H-109
       │                 └─ H-120
       ├─ H-080 ─ H-098
       ├─ H-082 ─ H-109
       ├─ H-083 ─ H-110 ─ H-111 ─ H-112 ─ H-113 ─ H-114
       ├─ H-084 ─ H-093
       ├─ H-085 ─ H-088
       ├─ H-086 ─ H-087 ─ H-100 ─ H-101
       ├─ H-095 ─ H-105
       ├─ H-096
       ├─ H-106 ─ H-107 ─ H-108
       ├─ H-118, H-119
       └─ H-121 ─ H-122
```
(The **Depends** field in each ticket is authoritative; this graph is a summary.)

## 5. Definition of done (every ticket)
1. New tests exist, fail without the change (demonstrated or reasoned), and pass
   with it.
2. The full global gate passes (AGENT_PROMPT_V2 §4). There are no new
   `#[ignore]`, `skip` or weakened assertions.
3. Docs are updated when behavior or interfaces change (`docs/CONTROL_API.md`,
   `PROVIDERS.md`, `CONFIGURATION.md`, `README.md`, `schemas/`).
4. `PROGRESS.md` entry with exact commands and results; known limitations
   stated.
5. One commit with the prescribed subject, staging only that ticket's files.
   No push.
