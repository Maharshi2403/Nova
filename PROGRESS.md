# Implementation progress

Latest completed ticket: H-053b — load and crash evidence.
Prior implementation: H-001..H-044 appear in Git history; their original platform
evidence has not been independently audited. Current stage: G5.

## H-045 verification

Local platform: macOS arm64. Commit: the commit containing this record
(`git log --grep='feat(H-045)'`).

- `cd intelligence && uv run --offline --locked pytest -q`: PASS, 78 tests.
- `cd intelligence && UV_CACHE_DIR=/private/tmp/nova-uv-cache uv run --offline --locked ruff check .`: PASS.
- `cd intelligence && UV_CACHE_DIR=/private/tmp/nova-uv-cache uv run --offline --locked ruff format --check .`: PASS, 25 files.
- `cd intelligence && UV_CACHE_DIR=/private/tmp/nova-uv-cache uv run --offline --locked mypy`: PASS, 24 source files.
- `cargo test --workspace --offline`: PASS, including offline mock E2E and usage accounting.
- `cargo fmt --all --check`: PASS.
- `cargo clippy --workspace --all-targets --offline -- -D warnings`: PASS.
- `git diff --check`: PASS.

Initial Python/Rust test runs were blocked by sandbox restrictions on localhost
binding. Both full suites passed after authorized reruns outside the sandbox.
No live provider calls were made. Windows execution and live prompt-cache hits
were not tested; no cross-platform milestone is claimed.

Prompts preserve mandatory acceptance and enforce serialized byte caps before
provider invocation. Snippets remain untrusted data from Rust-authorized retrieval;
this module grants no tools and performs no filesystem reads or persistence.
Planning uses the builder. Coder prompts are available to a future live worker loop;
the existing worker remains mock-only. Byte sizes are not observed token counts.

## H-046 verification

`RetryingProvider` retries only typed transient failures, up to ten configured
attempts (default three), with capped exponential full jitter and one overall
deadline. Cancellation and permanent errors propagate immediately. Every attempt,
including an explicitly configured fallback, requires an async authorization
callback supplied by the Rust-facing caller; no default approval is provided.
This is an opt-in adapter; live execution remains disabled by default.

Commands: `cd intelligence && uv run --offline --locked pytest -q` PASS (82 tests),
`uv run --offline --locked ruff check .` PASS, `uv run --offline --locked mypy`
PASS (26 files); changed files formatted with Ruff; `git diff --check` PASS.
No live calls. Cross-platform CI has not been executed remotely.

## H-047 verification

Added shared project/path locks, conservative case-insensitive directory overlap,
exclusive project operations and RAII cleanup on error/unwind. Invalid scopes fail
closed. Durable leases and broker canonical-path authorization are still required.
`cargo test --offline -p harness-runtime --test locks`: PASS (3 tests).
`cargo clippy --offline -p harness-runtime --all-targets -- -D warnings`: PASS.
`cargo fmt --all` and `git diff --check`: PASS.

## H-048 verification

Added bounded dispatch using the minimum configured/provider/infrastructure
capacity. Priority/readiness, resource locks and durable spend reservations precede
the execution callback. Failures block tasks, release leases and settle usage;
completion still requires the existing verifier/integrator. No process spawns for
blocked dependencies or denied budgets. `cargo test --offline -p harness-runtime
--test pool`: PASS; `cargo clippy --offline -p harness-runtime --all-targets --
-D warnings`: PASS; format and diff checks PASS.

## H-049 verification

Integration instances share one FIFO async mutex per canonical repository, including
across run IDs. Existing worktree identity/cleanliness and task state are checked
before changes. Safe merge retains conflict branches; ancestor detection no longer
depends on bounded Git log output. No force, reset, user checkout changes, or push.
`cargo test --offline -p harness-runtime --test integration`: PASS (4 tests, including
two concurrent instances); runtime all-target Clippy, formatting and diff checks PASS.

## H-050 verification

Startup reconciles expired leases without deleting worktrees or trusting arbitrary
PIDs. Ambiguous attempts are blocked. Crash-after-commit recovery recognizes exact
task/attempt trailers at HEAD, rechecks committed paths and reruns acceptance before
recording the existing SHA. No second commit is created. Tests: runtime `committer`
(3), `diff_gate` (3), `recovery` (1) PASS; workspace all-target Clippy and format PASS.
Recovery intentionally requires inspection of ambiguous/orphan attempts; it does
not kill PIDs that may have been reused or silently replay side effects.

## H-051 verification

Worker pool exposes per-run cancellation signals; pause remains dispatch-only.
Unix workers have separate process groups and timeout/kill uses the command
broker's tree termination path (Windows taskkill /T /F). Stderr cleanup is bounded
even when a descendant holds the pipe. Closed cancellation channels cannot starve
protocol reads. Worker (7), pool (1), supervisor (12) tests PASS; workspace all-target
Clippy and formatting PASS. Windows tree termination requires Windows CI; no Job
Object guarantee is claimed. Uncommitted worktrees are preserved.

## H-052 verification

Authenticated `/v1/events` accepts `wait_seconds=0..25`, with at most 64 concurrent
waiters, 1000 events per response and durable sequence cursors. Disconnect drops
the permit. There is no unbounded per-client event buffer. Daemon API tests PASS
(11), including delayed production, reconnect without duplicates and invalid waits.
Workspace all-target Clippy, formatting and diff checks PASS.

Next: H-053a execution wiring, then H-053b fault/load evidence (see implementation backlog).

## H-053a verification

Connected explicit authenticated execute endpoint/CLI to the bounded pool, real
Python IPC mock coder, brokered path-scoped writes, recorded usage, real verifier,
task commits and serialized integration. Exact acceptance argv must be approved by
the API caller before dispatch. Run completion requires all tasks integrated.
The mock coder writes synthetic fixture content; it does not pretend to solve
arbitrary coding requests. Integration branches do not change the user's checkout.

`cargo test --workspace --offline`: PASS, including new real-worker execution test;
`cargo clippy --offline --workspace --all-targets -- -D warnings`: PASS;
Python pytest PASS (82), Ruff check/format PASS (27 files), mypy PASS (26 files).
Local macOS only, no live API calls. Next: H-053b load and CLI fault evidence.

## H-053b verification

Added `scripts/offline_demo.py`: real CLI/daemon/Python/broker/SQLite/Git execution,
dependency chains, exact content assertions, unique SHA and evidence inspection,
clean original checkout, and forced daemon kill during verification. SQLite reads
in the Python fixture use mode=ro; all runtime writes are Rust-owned. Startup holds
a cross-platform OS file lock and binds before reconciling old-supervisor leases.

- `cargo build --offline --workspace`: PASS.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 100`: PASS, 100
  integrated tasks with unique commits and real verifier evidence.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 1 --fault`: PASS
  (also rerun after adding OS lock), interrupted task blocked without replay.
- Store lease tests (5), runtime recovery (1), daemon server (4): PASS.
- Workspace Clippy/format/diff checks: PASS.

No cross-platform gate sign-off yet; Windows CI still required. A verifier already
running when a daemon is forcibly killed can finish independently; recovery blocks
its task rather than silently repeating it. Next: H-054 path/command hardening.
The original three untracked handoff documents are intentionally excluded from
the feature commit and left unchanged.
