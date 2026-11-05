# Implementation progress

Latest locally verified implementation: H-061 — Nova single-command startup.
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

## H-054 verification

Rejects Windows drive/ADS/backslash paths, device names, control characters,
trailing-dot/space aliases and credential/config directories on every OS. Unicode
NFC is shared by path scopes and locks. Writes use create-new temporary files and
atomic replacement, preserving permissions and avoiding external hardlink inode
mutation. Diff scanning rejects symlink targets before reading them.
Runtime path_security (3), broker_write (7), broker_read (7), diff_gate (3) PASS;
workspace all-target Clippy/format/diff PASS. Existing argv injection tests remain.
Parent-directory TOCTOU cannot be eliminated by string canonicalization; trusted
local mode is not a sandbox. Next: H-055 redaction and bounded logging.

## H-055 verification

Shared diagnostic redaction masks registered runtime credentials, bearer tokens,
API-key/password/secret patterns and caps UTF-8 output at 64 KiB. Worker stderr,
command output, Git failure stderr, API error messages and persisted verifier
artifacts use it; prompt/message fields are removed from artifacts. Daemon tokens
are registered at load. Core redaction tests (2), runtime broker_command (7),
verifier (2), worker (7) PASS; workspace Clippy and format/diff checks PASS.
No normal log includes model prompts. Pattern matching cannot discover every
unknown secret; protected paths and minimal context remain required.
Next: H-056 durable typed approvals.

## H-056 verification

Typed command/destructive/network/push/deploy approvals bind exact argv, canonical
cwd and task attempt. Pending, decided and consumed events are transactional;
approvals are single-use, revision checked, not transferable to changed commands,
and invalidated by terminal tasks/new attempts. Broad prefixes cannot implicitly
approve recognized risky operations. Worker command requests can create durable
requests but cannot decide them. Authenticated API/CLI list and human-decision
commands are implemented; workers receive no API bearer token.
Store approvals (1), runtime approval_policy (1), broker_command (7), engine (1),
daemon API (11) PASS; workspace Clippy/format/diff PASS. Credential-like argv is
rejected rather than persisted. Next: H-057 explicit trust/isolation policy.

## H-057 verification

New/existing projects default to untrusted. Registration avoids Git before explicit
trust; `init --trust-local` records the authenticated local-user decision and checks
a clean Git repository. Engine refuses untrusted execution without a real sandbox.
Added capability/adapter interface and documented that this release ships no hard
isolation backend. Sandbox (2), engine (1), daemon API (11) tests PASS; workspace
Clippy/format/diff PASS. Windows execution is still unverified locally.

Before H-058, finish the newly documented H-043b/c backfill for live worker wiring;
the original adapter alone did not implement live coding through the daemon.

## H-043b verification

Added a structured coder loop over the existing opt-in Responses adapter. Every
attempt (including transient retries) asks the Rust broker for spend authorization;
all tools use JSONL. Prompts retain acceptance and a bounded recent handoff, usage
is emitted per call and unknown output usage stops further calls. Heartbeats run
while awaiting the provider. Tool, output, turn and wall-time limits fail closed.
Provider errors never echo inputs or credentials. Structured-output contract checked
against https://developers.openai.com/api/docs/guides/structured-outputs using the
OpenAI Docs skill. No model IDs/prices are hardcoded.

`cd intelligence && uv run --offline --locked pytest -q`: PASS (84), Ruff check
and format PASS (29 files), mypy PASS (28 files). All provider tests are fake/offline.
Next: H-043c Rust live routing/reservations and explicit CLI opt-in.

## H-043c verification

Explicit `execute --live --model-config` selects registry models by task complexity
and risk, reserves each task's full spend cap under run/daily limits and authorizes
each provider request against its exact model/input/output grant. Coder processes
have no credentials. A separate isolated-import Python adapter process receives
the key only for one bounded request; Rust records its observed usage and prices
it from the configured registry. Failed/uncertain calls remain unknown. A durable
in-flight call marker prevents cancellation/restart from refunding unknown spend.
No actual model IDs/prices are assumed and no live call was performed.

`cargo test --workspace --offline`: PASS (after correcting the expected table
inventory for approvals/trust/provider calls). Runtime live tests (3) include real
IPC with an empty key, proving refusal before network; engine/pool/usage tests pass.
Workspace Clippy/format/diff PASS. Python pytest PASS (84), Ruff PASS (30 files),
mypy PASS (29 files). Next: H-058 cross-platform release checks and regressions.

## H-058 local verification (remote gate pending)

CI now uses locked dependencies, offline runtime tests, Windows/macOS/Linux jobs,
100-task CLI evidence, kill/restart and cancel-during-verification fixtures, and
binary artifacts. No credentials/live-provider enablement exist in CI. Added
hostile-origin denial, atomic run-revision dispatch gate, cancellation while a
broker is pending, cancellation checks before commit/completion, and disabled
repository hooks for internal Git operations.

`cargo test --locked --offline --workspace`, workspace all-target Clippy, formatting,
`cargo build --locked --offline --workspace`: PASS. Current 100-task demo, forced
restart demo and cancellation demo all PASS. Workflow parsed with Ruby YAML.
Windows/macOS hosted CI has NOT been run from this environment; H-058's full gate
and G4..G7 cross-platform sign-off remain pending. H-059/H-060 documentation and
packaging are prepared locally without claiming a release or skipping that gate.

## H-058a workflow-review regressions

Repeated pause/resume audit keys now include the expected run revision. Concurrent
scheduler CAS losers no longer abort unrelated running jobs, dispatch skips tasks
already moved by another scheduler, and each run owns a distinct worktree registry.
Daemon API tests PASS (12, including three control cycles/concurrent ticks), runtime
engine/pool PASS, workspace Clippy/format/diff PASS. Next: H-059 operator docs.

## H-059 verification

Replaced stale scaffold README with working build/demo instructions, operator
workflows, live opt-in/configuration, threat model, troubleshooting, backup/migration
and future-client API contracts. Documented actual limitations: CLI submits typed
plan JSON, foundational YAML resolver is not automatically applied by the daemon,
capacity is two, review/conflict resolution is manual, no shipped hard sandbox,
no automatic push or deployment. Live example is deliberately unavailable until
the operator supplies reviewed account-specific model IDs/prices/capabilities.

Documented three-task quickstart PASS via `uv run --locked --offline --project
intelligence python scripts/offline_demo.py --tasks 3`. CLI/daemon help checked;
local documentation links/example JSON validated; format/diff checks PASS.
Hosted Windows/macOS and paid live smoke remain unverified. Next: H-060 local
packaging and extracted-bundle installation smoke, without release publication.

## H-060 verification — 2026-11-05, macOS arm64

Added stripped/thin-LTO native release builds, pinned Python build requirements,
hash-pinned runtime dependency export, deterministic ZIP packaging with per-file
and archive SHA-256 checksums, build/source metadata, bounded safe extraction,
fresh-wheel installation smoke, and a manual release checklist. CI packages and
smoke-tests each native platform. Generated bundles are ignored under dist/.

Actual final checks:

- `cargo test --locked --offline --workspace`: PASS, 207 tests across 56 test/doc binaries.
- `cargo build --release --locked --offline --workspace`: PASS.
- `cargo clippy --locked --offline --workspace --all-targets -- -D warnings`: PASS.
- `cargo fmt --all --check`: PASS.
- Python `uv run --offline --locked pytest -q`: PASS, 84 tests; Ruff check/format
  PASS (30 files), strict mypy PASS (29 files).
- Release/demo scripts Ruff formatting and E/F/I/B/W lint: PASS.
- `scripts/offline_demo.py --tasks 100 --bin-dir target/release`: PASS, unique
  commits/integration SHAs, verifier evidence and a clean original checkout.
- `scripts/package_release.py` and `scripts/smoke_bundle.py --archive-dir dist`:
  PASS, actual native binaries, fresh temporary venv, hash-verified dependencies,
  non-editable wheel installation, and real extracted-bundle CLI workflow.
- Repeated packaging of identical inputs: byte-identical SHA-256; content inspected
  for unexpected token/database/Git paths. Workflow YAML and documentation links PASS.

The first fresh-install smoke correctly failed because package metadata was missing
from the cache. A separate optional preparation script installs hash-verified
dependencies in a disposable venv/cache; the installation smoke remains offline.
No assertions/hashes were weakened. No live provider calls, push, tag or publication.
Bundles are rebuilt after this commit to record the final source revision and clean
tracked status. Archive determinism does not claim cross-host binary reproducibility.

## Remaining release evidence

The local implementation backlog is built and committed through H-060, including
the documented live-worker and execution backfills. Full H-058/G4..G7 sign-off still
requires hosted Windows/macOS evidence and a separately authorized budgeted live
smoke. Signing/notarization and release publication are not performed. No shipped
hard sandbox exists; untrusted execution is denied. CLI plans use reviewed JSON;
the foundational YAML resolver is not automatically loaded by the daemon. See the
operator/configuration docs for these explicit product limits.

The three original handoff documents remain unchanged and untracked. Only
ticket-owned implementation, tests, documentation and packaging files were committed.

## H-061 — user-requested Nova CLI usability

Canonical executable is now `nova`; bare `nova` and `nova start` launch the service
in the background or authenticate/connect to the existing instance. The single
binary embeds the existing Rust daemon; no separate daemon executable is required
for installation. `nova stop` is authenticated and refuses to interrupt active
runs. An atomic engine shutdown gate prevents a concurrent run launch. Existing
commands remain, with `harness` as a compatibility alias. Help does not start a
service. Startup never approves a plan, dispatches agents or calls a provider.

Default macOS state is ~/Library/Application Support/Nova (the existing local state
directory). Other platforms use Nova under LOCALAPPDATA/HOME. Existing legacy state
is retained and accessible with --daemon-dir; no automatic migration/deletion.

Verification actually run:

- Full offline Rust workspace: PASS, 212 tests (including new startup/authentication
  regressions and active-run shutdown protection).
- Final follow-up CLI suite: PASS, six unit/integration tests across both command
  names and real detached start/attach/stop/restart on disposable state directories.
- Workspace all-target Clippy with -D warnings, Rust formatting, script lint/format,
  documentation links and git diff --check: PASS.
- Native release build and extracted package smoke: PASS, including standalone
  Nova start/attach/stop before Python installation and the existing real CLI demo.
- Installed a standalone executable at ~/.local/bin/nova; confirmed it resolves
  from a normal login shell. Verified nova --version, nova stop, bare nova and
  repeated nova start. Nova is left running with no agents automatically started.

Original handoff files and unrelated .DS_Store are untouched. No shell-profile edits,
push, tag, paid calls or desktop UI were introduced. Windows detached-service
behavior remains subject to the configured hosted CI checks.

## H-062 — additional provider protocols

Added Anthropic Messages and explicitly configured OpenAI-compatible Chat Completions
adapters, normalized cached/input/output usage, bounded catalog discovery/pagination,
HTTPS/loopback URL checks and disabled redirects/proxies. Anthropic schema constraints
are moved to descriptions for transport and still validated locally against the
original schema. Provider helper selection remains credential-scoped and JSONL-only.
Official OpenAI, Claude and OpenRouter contracts were checked before implementation.
No model-generation calls were made.

Python pytest: PASS (101), Ruff: PASS, strict mypy: PASS (32 files). New offline
fixtures cover both request formats, cache accounting, missing usage, authentication/
transient errors, schema constraints, pagination, unsafe URLs, redirects and size caps.
Next: H-063 saved profiles/native credential setup and model management.
The original three untracked handoff documents are intentionally excluded from
the feature commit and left unchanged.

## H-063 — saved provider setup and catalog

Added atomic locked public provider profiles, macOS Keychain/Windows credential
vault references, hidden terminal key entry and environment references for headless
or unsupported native vault platforms. Added provider add/list/remove and model
list/sync/configure commands, credential-scoped discovery and conservative known
metadata. Unknown model prices/capabilities remain unavailable. No real API calls
or user credentials were used. Native vault interaction needs operator setup;
Windows native execution is not tested on this Mac.

Validation: provider settings tests 2 passed; CLI tests 6 passed (loopback lifecycle
requires sandbox escalation); workspace Clippy with warnings denied passed.
Next H-064 wires saved profiles into live execution and use-case routes.

## H-064 — saved multi-provider execution and task use cases

Task proposals now carry coding/planning/review/debugging/architecture/fast use cases,
with backward-compatible coding defaults. Saved models generate a validated live
registry and deterministic complexity/risk defaults; `nova routes set/list/reset`
adds explicit per-use-case model choices. The CLI generates live config from saved
profiles without an operator-authored JSON model registry. Rust resolves each
provider's credential reference, keeps it out of coder/verifier subprocesses,
and passes only the selected provider connection to its bounded adapter invocation.
Legacy explicit OpenAI/Anthropic env-key configs still work. Provider results retain
saved connection identity for usage and budget accounting. The complete provider
workflow is documented in docs/PROVIDERS.md. No paid calls or credentials were used.

Validation: Python 102 pytest passed with local provider fixtures; Ruff and strict
mypy passed (32 source files); workspace Rust tests, formatting and Clippy pass.
Offline provider CLI route/setup/removal and cross-provider selection tests pass.
Hosted Windows CI and an authorized live paid smoke remain release gates.

## H-065 — virtual environment interpreter path

Release-bundle smoke exposed a real regression: resolving the venv `python`
symlink selected the system interpreter, which could not import harness_ai. Nova
now validates an absolute existing interpreter while preserving the venv entry path.
A Unix symlink regression test and Clippy pass. The full package smoke is rerun
before considering the installation complete.

## H-066 — native dashboard read model

Added transactional bounded project/run inventory with counts and trust status,
and authenticated task verification/commit evidence. SQL remains in the store;
browser-origin rejection is preserved. Multi-provider execution now reports
"configured" instead of incorrectly labelling all live runs OpenAI.
Validation: daemon API 14 tests passed; workspace Clippy passed. New tests cover
paging, totals, auth, hostile origins, invalid limits and missing evidence.
Next H-067: native macOS dashboard and credential-free CLI bridge.

## H-067 — native macOS management interface

Added native SwiftUI project/run/task views, provider key setup, model routing,
guided operator-authored task plans, separate approval/execution review, controls,
usage, activity and verifier/commit evidence. The bounded stdin CLI bridge keeps
bearers and keys out of argv/URLs/UI responses; browser-origin rejection remains.
Desktop setup uses Keychain; no paid generation was performed during development.

User testing found dirty checkouts were rejected at project import. Import now
inspects the Git root without requiring clean files, while execution retains the
clean-tree gate. HTTP validation failures now show their message instead of being
mislabelled malformed HTTP. A regression test verifies dirty files remain intact.

Validation: Swift native compilation passed; native overview/provider/project forms
visually inspected. Desktop bridge tests 2 passed including a complete reviewed,
approved, verified/integrated mock task and key/header-injection rejection. Daemon
API tests 15 passed, Git repository tests 6 passed, workspace Clippy passed.
Next H-068: `nova ui`, durable application packaging/install and final smoke.
