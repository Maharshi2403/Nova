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

## H-068 — desktop distribution and Nova UI launch

Added `nova ui` / `nova ui --print-path`, native bundle discovery and macOS launch,
a vector application icon, locked intelligence runtime in Nova.app, local desktop
ZIP/checksum generation, and user-local installer retaining previous app/CLI builds.
The bundle's CLI can locate its sibling runtime without losing the venv entry path.
The GUI uses the same permanent Nova state as the CLI; closing it does not stop work.
No hosted GUI, paid generation, signing or notarization was performed.

Validation: full Rust workspace tests passed; scoped provider settings 4 passed;
workspace Clippy and Rust formatting passed. Native SwiftUI compilation at macOS
14 minimum and fresh bundled wheel/runtime version checks passed. Vector icon
render and ICNS validation passed. Python packaging script Ruff passed. Final
artifact workflow smoke/install is performed after this source commit.

Operator preview setup was adopted into the permanent user Nova directory after
confirming the main state had zero projects/runs/providers and both services were
idle. The previous empty state was retained as a backup; a compatibility symlink
keeps the preview window connected. No provider key was printed or placed in argv.

H-068 final artifact acceptance: PASS. Extracted desktop ZIP/checksum/path checks,
isolated bundled intelligence import, rejection before approval, three real mock
worker tasks with verifier exit 0 and integration SHAs, and clean fixture checkout
all passed. Nova.app installed at ~/Applications/Nova.app; ~/.local/bin/nova matches
its bundled CLI hash. Native installed overview and guided task-plan sheet inspected;
service connected and the operator's provider connection retained. `nova ui --print-path`
resolves the installed app from a normal login shell. No paid model generation used.

## H-069 — actionable project trust and empty-project cleanup

Added explicit authenticated project trust changes and removal of registration
metadata only for projects with no runs. Trust enable inspects the Git root and
permits unfinished files; execution keeps its separate trust/clean-base checks.
Removal is transactional, preserves repositories on disk, and rejects projects
with runs. Desktop stdin transport supports the corresponding DELETE request.
Validation: daemon API 16 tests passed; workspace Clippy passed. New regression
covers unauthenticated trust denial, explicit enable, dirty-file preservation,
run-preserving removal rejection, and removal of empty registration metadata.
Next H-070: compact island panel, logo and installed UI correction.

## H-070 — compact island panel and Nova logo correction

Replaced the oversized default dashboard with a native floating, dark top-center
panel: 210×44-point collapsed status pill (default), 360×340 expanded Work/Projects/
Models controls. Added a custom Nova N monogram across panel, menu bar, ICNS and
editable SVG. Setup/task sheets are compact and task editing is one column.
One panel per canonical state directory is enforced; repeated launches reveal it.

Execution prerequisites are actionable: untrusted projects offer Enable, unapproved
plans offer Approve, and live execution without ready models states the missing
metadata. The user-authored documentation plan in our QA project was preserved;
a copy was created as a reviewable draft under the real Nova repository, with no
approval/execution or paid calls. The QA registration now contains user history
and was retained instead of deleting that work.

Validation: 227 Rust workspace tests passed; native Swift compilation passed.
Compact preview expanded/collapsed controls visually checked. Final app build,
packaged mock smoke and installed appearance are checked after this commit.

H-070 acceptance: final extracted app archive passed its bundled-runtime and
three-task approved mock workflow with verifier/integration evidence. Installed
panel visually confirmed as a black 210×44-point Nova-logo pill, expanding to the
small Work/Projects/Models panel. No paid generation was initiated. Collapsed
status uses short labels to avoid clipping in the pill.

## H-071 — execution from a pinned committed snapshot

Fixed the repeated dirty-checkout execution blocker without changing user files.
Execution review now visibly defaults to committed snapshot mode; CLI exposes
--committed-snapshot and API defaults false for compatibility. Snapshot mode reads
a committed revision into isolated task/integration worktrees and excludes local
unfinished files. Trust, plan approval, exact verifier argv and model budgets stay
enforced. Source SHA is pinned durably per run, reused on resume and available in
Details; every accepted launch audits its mode/hash. Authenticated preflight errors
are redacted and reported directly rather than replaced with a generic checklist.
Approved-but-not-started work is labelled Ready instead of implying an agent runs.

Validation: full Rust suite 229 passed. Final scoped engine/native transport tests
5 passed, including tracked+untracked dirty files preserved byte-for-byte, worker
view containing committed content, strict-mode rejection, trust/approval rejection,
source pin idempotence, launch audit, verifier success and integrated commits.
Workspace Clippy/format and native Swift compilation passed. No paid model calls.
Final packaged-app dirty-source smoke and installation follow this source commit.

## H-072 — remove invalid pytest default for documentation tasks

Confirmed the saved documentation plan inherited python3 -m pytest -q while the
system interpreter has no pytest. The task editor now starts with empty explicit
acceptance and offers a bundled stdlib-only Documentation check preset. It appends
its artifact requirements to the task description and checks scoped Markdown,
minimum content/sections, Mermaid and architecture/improvement text. It excludes
external/symlink files and bounds input. Existing approved plans remain immutable;
a corrected reviewable draft is prepared after installation, with no paid execution.

Validation: three positive/negative/external-file stdlib tests passed; script CI
format/lint passed; native Swift compilation passed. CI and operator checks now
include the checker tests. Desktop packaging scripts were formatted to their CI
configuration. Final bundled-checker acceptance and install follow this commit.

# Nova handover v2 progress

## H-073 — Land the in-progress working tree

Baseline: `3f4c4ec` plus the uncommitted working tree described in `HANDOVER_V2.md`.
Backup: `/private/tmp/nova-h073-backup/tracked.patch` and its `untracked/` directory.

The working tree was divided into H-073a (failure diagnostics), H-073b
(acceptance-check artifact cleanup), and H-073c (main-agent goal planning and
remaining WIP). The landing also retains pre-existing provider key editing,
Anthropic header and coder turn-limit changes; these were present before H-073.

Verified on macOS (Apple Silicon), 2026-11-06:

- `cargo build --locked --offline --workspace` — PASS.
- `cargo fmt --all --check` — PASS after formatting the pre-existing desktop bridge changes.
- `cargo clippy --locked --offline --workspace --all-targets -- -D warnings` — PASS.
- `cargo test --locked --offline --workspace` — PASS.
- `cargo test --locked --offline -p harness-runtime --test live_e2e` — 4 passed.
- `UV_CACHE_DIR=/private/tmp/nova-uv-cache uv run --locked --offline ruff check .` from `intelligence/` — PASS.
- `UV_CACHE_DIR=/private/tmp/nova-uv-cache uv run --locked --offline ruff format --check .` from `intelligence/` — PASS.
- `UV_CACHE_DIR=/private/tmp/nova-uv-cache uv run --locked --offline mypy src` from `intelligence/` — PASS.
- `uv run --locked --offline --project intelligence pytest -q` — 117 passed.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 3` — PASS.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 100` — PASS.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 1 --fault` — PASS.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 1 --cancel` — PASS.
- `intelligence/.venv/bin/python scripts/build_desktop.py` — PASS.

The 100-task, fault and cancel demos needed execution outside the filesystem
sandbox because the sandbox denied the daemon's loopback bind. No paid provider
calls, push, or tags were used.

Known security regression pending H-075: the control API has no bearer
authentication. Until H-075, do not use this build with real provider keys.

## H-074 — Find symbols without ripgrep

Added a deterministic `retrieve_with(..., use_ripgrep: false)` seam and a
bounded pure Rust symbol scan. It checks paths with `PathPolicy`, rejects secret
and escaped paths, reads at most 2 MiB per file, and returns a 41-line window
subject to the existing context byte cap. Tests cover a symbol after line 80,
secret path exclusion, and a match beyond the broker's 1 MiB prefix.

Verified on macOS (Apple Silicon), 2026-11-06:

- `cargo test --locked --offline -p harness-runtime --test context` — 4 passed (new test initially failed to compile before implementation).
- `cargo fmt --all --check` — PASS.
- `cargo clippy --locked --offline --workspace --all-targets -- -D warnings` — PASS.
- `cargo test --locked --offline --workspace` — PASS.
- `cargo build --locked --offline --workspace` — PASS.
- `uv run --offline --locked pytest` from `intelligence/` — 114 passed.
- `UV_CACHE_DIR=/private/tmp/nova-uv-cache uv run --locked --offline ruff check .` from `intelligence/` — PASS.
- `UV_CACHE_DIR=/private/tmp/nova-uv-cache uv run --locked --offline ruff format --check .` from `intelligence/` — PASS.
- `UV_CACHE_DIR=/private/tmp/nova-uv-cache uv run --locked --offline mypy src` from `intelligence/` — PASS.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 3` — PASS.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 100` — PASS.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 1 --fault` — PASS.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 1 --cancel` — PASS.

Python socket tests and the demos required loopback access outside the sandbox.
No paid provider calls, push, or tags were used.

## H-075 — Restore authenticated local control plane

Every `/v1` route now requires a constant-time compared bearer token and a
loopback Host header with the daemon's bound port. The daemon creates 32 random
bytes from the OS RNG, stores them in `daemon.token`, and restricts the data
directory, database, and token permissions on Unix. The CLI reads the token
from the data directory and rejects an impersonating listener during startup.
The desktop continues to use the CLI stdin bridge. The API, trust-boundary,
and ADR documents describe the restored control plane.

Verified on macOS (Apple Silicon), 2026-11-06:

- `cargo test --locked --offline -p harness-daemon` — 18 API and 9 server tests passed before the final bound-port check; `cargo test --locked --offline -p harness-daemon --test server` — 10 server tests passed afterward.
- `cargo test --locked --offline -p harness-cli` — CLI tests passed, including `startup_rejects_impersonating_listener`.
- `cargo fmt --all --check` — PASS.
- `cargo clippy --locked --offline --workspace --all-targets -- -D warnings` — PASS.
- `cargo test --locked --offline --workspace` — PASS.
- `cargo build --locked --offline --workspace` — PASS.
- `uv run --offline --locked pytest` from `intelligence/` — 114 passed.
- `UV_CACHE_DIR=/private/tmp/nova-uv-cache uv run --locked --offline ruff check .` from `intelligence/` — PASS.
- `UV_CACHE_DIR=/private/tmp/nova-uv-cache uv run --locked --offline ruff format --check .` from `intelligence/` — PASS.
- `UV_CACHE_DIR=/private/tmp/nova-uv-cache uv run --locked --offline mypy src` from `intelligence/` — PASS.
- `UV_CACHE_DIR=/private/tmp/nova-uv-cache uv run --locked --offline --project intelligence ruff check scripts/smoke_bundle.py` — PASS.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 3` — PASS against the rebuilt authenticated daemon.
- `intelligence/.venv/bin/python scripts/build_desktop.py` — PASS.

No paid provider calls, push, or tags were used. H-076 remains necessary:
the authenticated execute request still accepts a caller-selected interpreter.

## H-076 — Daemon-owned interpreter and spend ceilings

The execute API now rejects caller-supplied `python` and `model_config` fields.
The daemon resolves its interpreter from saved or bundled settings and derives
live routing and prices from saved provider settings. Optional request run/daily
caps may only lower the saved ceilings. Operators set the interpreter with
`nova config set python PATH` and ceilings with
`nova budget set --run USD --daily USD`. Each change is recorded atomically in
the bounded settings audit history. Provider setup also audits interpreter
changes. The worker environment regression test executes a process and observes
only the selected provider key name among credential-shaped variables.

Verified on macOS (Apple Silicon), 2026-11-06:

- `cargo test --locked --offline -p harness-daemon --test api` — 20 passed; `execute_rejects_caller_interpreter` first failed with 404 before the fix, then passed with 422.
- `cargo test --locked --offline -p harness-cli --test settings` — 2 passed.
- `cargo test --locked --offline -p harness-runtime --test worker` — 8 passed.
- `cargo fmt --all --check` — PASS.
- `cargo clippy --locked --offline --workspace --all-targets -- -D warnings` — PASS.
- `cargo test --locked --offline --workspace` — PASS.
- `cargo build --locked --offline --workspace` — PASS.
- `uv run --offline --locked pytest` from `intelligence/` — 114 passed.
- Python ruff lint/format and mypy in `intelligence/` — PASS.
- `UV_CACHE_DIR=/private/tmp/nova-uv-cache uv run --locked --offline --project intelligence ruff check scripts/offline_demo.py` — PASS after formatting the touched demo.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 3` — PASS.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 100` — PASS.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 1 --fault` — PASS.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 1 --cancel` — PASS.
- `intelligence/.venv/bin/python scripts/build_desktop.py` — PASS.

No paid provider calls, push, or tags were used.

## H-077 — Validate planner test commands through approval policy

Plan validation now requires an absolute, discovered runner and passes every
acceptance argv through the approval classifier. Git is no longer a test runner;
push, publish, network, deploy, destructive, and shell command classes are
rejected. Package managers require a named test script, and make requires a
target. These tasks appear in the plan response's `requires_attention` list and
in the desktop approval review. Candidate file scopes reject absolute and
parent paths.

Verified on macOS (Apple Silicon), 2026-11-06:

- `cargo test --locked --offline -p harness-runtime --test orchestrator` — 7 passed, including all six required regressions.
- `cargo fmt --all --check` — PASS.
- `cargo clippy --locked --offline --workspace --all-targets -- -D warnings` — PASS.
- `cargo test --locked --offline --workspace` — PASS.
- `cargo build --locked --offline --workspace` — PASS.
- `uv run --offline --locked pytest` from `intelligence/` — 114 passed.
- Python ruff lint/format and mypy in `intelligence/` — PASS.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 3` — PASS.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 100` — PASS.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 1 --fault` — PASS.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 1 --cancel` — PASS.
- `intelligence/.venv/bin/python scripts/build_desktop.py` — PASS.

No paid provider calls, push, or tags were used.

## H-078 — Separate model turns, tool calls, and heartbeats

The assignment protocol carries an optional `max_model_turns` that defaults to
`max_tool_calls` when absent. Model authorization and completion requests no
longer spend repository tool calls; model completions have their own limit.
Heartbeat and progress frames are excluded from the message count and limited
to one per 500 ms. On its final turn, the coder asks the model to finish and
emits a retryable `turn budget exhausted` failure if it still requests a tool.

Verified on macOS (Apple Silicon), 2026-11-06:

- `cargo test --locked --offline -p harness-runtime --test supervisor` — 17 passed, including heartbeat flood, model turn, and tool budget regressions.
- `uv run --offline --locked pytest -q tests/test_coder.py` — 4 passed, including clean turn exhaustion.
- `cargo fmt --all --check` — PASS.
- `cargo clippy --locked --offline --workspace --all-targets -- -D warnings` — PASS.
- `cargo test --locked --offline --workspace` — PASS.
- `cargo build --locked --offline --workspace` — PASS.
- `uv run --offline --locked pytest` from `intelligence/` — 115 passed.
- Python ruff lint/format and mypy in `intelligence/` — PASS.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 3` — PASS.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 100` — PASS.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 1 --fault` — PASS.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 1 --cancel` — PASS.
- `intelligence/.venv/bin/python scripts/build_desktop.py` — PASS.

No paid provider calls, push, or tags were used.

## H-079 — Always release leases and preserve cancellation state

Pool jobs now attempt failure recording, task transition, lease release, and
usage settlement independently, returning the combined errors. Cancelled jobs
transition to `Cancelled`; the state table permits cancellation from verifying,
reviewing, and integrating. Tests inject a failed failure-event write and
confirm cleanup still runs, and verify that known usage settles on failure.

Verified on macOS (Apple Silicon), 2026-11-06:

- `cargo test --offline -p harness-runtime --test pool` — 4 passed, including all three required regressions.
- `cargo test --locked --offline -p harness-core --test state` — 5 passed.
- `cargo fmt --all --check` — PASS.
- `cargo clippy --locked --offline --workspace --all-targets -- -D warnings` — PASS.
- `cargo test --locked --offline --workspace` — PASS.
- `cargo build --locked --offline --workspace` — PASS.
- `uv run --offline --locked pytest` from `intelligence/` — 115 passed.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 3` — PASS.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 100` — PASS.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 1 --fault` — PASS.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 1 --cancel` — PASS.

No paid provider calls, push, or tags were used.

## H-080 — Quarantine failed integration merges

When an integration final check fails (non-zero exit, timeout, failure to start,
or artifact cleanup failure), the merge SHA is pinned at
`refs/harness/quarantine/<run>/<task>` and an `integration.quarantined` event
records the ref, SHA, and redacted reason. The harness-owned integration
worktree is then reset to its pre-merge SHA, so later tasks never branch from
the broken tip. User branches are never touched. Task evidence
(`GET /v1/tasks/{run}/{task}/evidence`) now includes the quarantine record.
The former `integration_failure_retains_branch` assertion was inverted: the
integration branch no longer contains the failed merge.

Verified on macOS (Apple Silicon), 2026-11-06:

- `cargo test --offline -p harness-runtime --test integration` — 6 passed, including `failed_final_check_restores_integration_tip`, `next_task_does_not_inherit_failed_merge`, `quarantine_ref_retains_failed_merge`.
- `cargo fmt --all --check` — PASS.
- `cargo clippy --locked --offline --workspace --all-targets -- -D warnings` — PASS.
- `cargo test --locked --offline --workspace` — PASS.
- `uv run --offline --locked pytest` from `intelligence/` — 115 passed.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 3` — PASS.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 1 --fault` — PASS.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 1 --cancel` — PASS.

No paid provider calls or tags were used.

## H-081 — Finish or explain runs that cannot progress

Implemented on branch `agent/H-081` in `/Users/maharshi/Documents/nova-H-081`.
The drained-pool path now fails runs with blocked/failed tasks and records task
IDs in `run.failed`. Budget and lock denials produce a deduplicated `run.stalled`
reason while keeping the run Running. Run detail exposes nullable `reason`,
which appears in `nova status` and the desktop panel. Paused/cancelled runs are
left alone. Added the already-supported core Running→Failed transition to the
store allowlist. No migration or dependency changes.

Regression tests: `all_blocked_run_fails_with_reason` and
`budget_denied_run_emits_stalled_once` both PASS. Before this change, the first
would observe Running instead of Failed and the second would find zero stall
events. They exercise real store, scheduler and pool dispatch on temporary DBs.

Validation on macOS arm64, 2026-11-07 (commands from this worktree):
- `cargo fmt --all --check` — PASS.
- `CARGO_TARGET_DIR=/Users/maharshi/Documents/Nova/target cargo clippy --locked --offline --workspace --all-targets -- -D warnings` — PASS.
- `CARGO_TARGET_DIR=/Users/maharshi/Documents/Nova/target cargo build --locked --offline --workspace` — PASS (release build also PASS).
- `CARGO_TARGET_DIR=/Users/maharshi/Documents/Nova/target cargo test --locked --offline -p harness-runtime --lib --test pool --test integration --test engine` — PASS: 2 unit, 2 engine, 6 integration, 4 pool tests.
- `CARGO_TARGET_DIR=/Users/maharshi/Documents/Nova/target cargo test --locked --offline --workspace` — FAIL: CLI desktop daemon startup tests cannot start service in the restricted sandbox. Full suite stops there.
- From `intelligence/`: `uv run --locked --offline ruff check .`, `uv run --locked --offline ruff format --check .`, `uv run --locked --offline mypy` — PASS (34 typed source files).
- From `intelligence/`: `uv run --locked --offline pytest -q` — 106 passed, 9 failed. All failures are localhost binding PermissionError: operation not permitted.
- `intelligence/.venv/bin/python scripts/offline_demo.py --bin-dir ../Nova/target/debug --tasks 3` — FAIL: daemon exits at startup. The 100-task, fault and cancellation demos also fail at daemon startup using the target symlink.
- `SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX15.4.sdk CLANG_MODULE_CACHE_PATH=/private/tmp/nova-swift-module-cache intelligence/.venv/bin/python scripts/build_desktop.py` — Swift compile PASS; packaging FAIL at iconutil (invalid iconset). Initial invocation before target/release was available also failed its prerequisite check.
- `git diff --check` — PASS.

Logs: `/private/tmp/nova-h081-*.log`. No tests skipped or assertions weakened.
No paid calls, push, or tags. Changes remain uncommitted and unmerged because
the handover requires a passing full gate before commit. Follow-up: rerun the
full gate in an environment permitting loopback sockets and desktop icon tools.

After the user's explicit permission to proceed, retried both full suites:
- `uv run --locked --offline pytest -q` from `intelligence/` — 106 passed,
  9 failed again with localhost bind PermissionError (2026-11-07).
- `CARGO_TARGET_DIR=/Users/maharshi/Documents/Nova/target cargo test --locked --offline --workspace`
  — stopped again at the same two CLI desktop daemon startup failures.
- `git diff --check` — PASS.
User authorization is established; execution restrictions still apply to this
session. No permission escalation path is enabled by the session policy.


Full-access validation, 2026-11-07: the earlier environment blockers are resolved.
From `/Users/maharshi/Documents/nova-H-081`:
- `cargo fmt --all --check` — PASS.
- `cargo clippy --locked --offline --workspace --all-targets -- -D warnings` — PASS.
- `cargo test --locked --offline --workspace` — PASS, 268 tests.
- From `intelligence/`, `uv run --locked --offline ruff check .`,
  `uv run --locked --offline ruff format --check .`,
  `uv run --locked --offline mypy`, `uv run --locked --offline pytest -q`
  — all PASS; 115 Python tests.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 3`,
  `--tasks 100`, `--tasks 1 --fault`, `--tasks 1 --cancel` — all PASS.
- `SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX15.4.sdk CLANG_MODULE_CACHE_PATH=/private/tmp/nova-swift-module-cache intelligence/.venv/bin/python scripts/build_desktop.py`
  — PASS, including Swift compilation, icons and desktop packaging.
- `git diff --check` — PASS.
The implementation is ready for its H-081 commit and fast-forward integration.
No paid provider calls, push, or tags.

## H-082 — Detect overlapping glob scopes in path locks

Compare normalized fixed prefixes up to glob metacharacters (`*`, `?`, `[`,
`{`) whenever either scope is a glob. Potential overlap serializes
conservatively; literal directory boundaries retain their existing behavior.
No dependencies or protocol changes. Operations docs describe this behavior.

macOS arm64, 2026-11-07:
- Baseline: H-081 full workspace gate passed (268 tests), including existing locks.
- `CARGO_TARGET_DIR=../Nova/target cargo test --locked --offline -p harness-runtime --test locks`
  before implementation — 4 passed, 2 failed (`glob_and_literal_overlap`,
  `double_star_overlaps_child`), demonstrating the bug.
- `cargo test --locked --offline -p harness-runtime --test locks`
  after implementation — all 6 passed, including `disjoint_globs_parallel`.
- `cargo fmt --all --check` — PASS.
- `cargo clippy --locked --offline --workspace --all-targets -- -D warnings` — PASS.
- `cargo test --locked --offline --workspace` — PASS, 271 tests.
- Python gate from `intelligence/` (unchanged Python): `uv run --locked --offline ruff check .`, `uv run --locked --offline ruff format --check .`, `uv run --locked --offline mypy`, `uv run --locked --offline pytest -q` — PASS, 115 tests.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 3`, `--tasks 100`, `--tasks 1 --fault`, `--tasks 1 --cancel` — all PASS.
- `git diff --check` — PASS. No paid calls, push, tags or dependencies added.

## H-085 — Keep workers responsive during brokered model calls

Live coder requests now use a dedicated daemon reader, bounded JSONL buffering,
and correlation-indexed asyncio futures. Model and repository calls await those
futures. Cancellation handles Rust's session-wide cancel envelope, emits a
cancelled failure and exits 0. The worker owns one live assignment and stops its
reader before exit; mock assignment behavior remains compatible. The entire
live assignment, including initial context retrieval, has a deadline. No new
dependencies or protocol/schema changes. Intelligence README updated.

Tests added: `heartbeat_continues_during_slow_model_call`,
`asyncio_timeout_interrupts_model_wait`, `cancel_frame_during_tool_exits_cleanly`.
The first run failed collection because the asynchronous broker did not exist.
Against the original synchronous broker, heartbeat/timeout checks cannot advance
while stdin blocks, and cancellation raises a protocol error instead of exiting
successfully. The cancellation test exercises a real worker subprocess with
Rust's session-wide cancel shape. All three now pass on macOS arm64.

Validation on macOS arm64, 2026-11-07:
- `cargo fmt --all --check` — PASS.
- `cargo clippy --locked --offline --workspace --all-targets -- -D warnings` — PASS.
- `cargo test --locked --offline --workspace` — PASS, 268 tests (H-081 base).
- From `intelligence/`: `uv run --locked --offline ruff check .`, `uv run --locked --offline ruff format --check .`, `uv run --locked --offline mypy`, `uv run --locked --offline pytest -q` — PASS, 118 tests, 36 typed source/test files.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 3` — PASS.
- `git diff --check` — PASS. No paid calls, push, tags or dependencies added.

## H-083 — Recover interrupted goal planning atomically

Daemon startup reconciles orphaned Planning runs to Failed with durable
`planning interrupted` evidence, atomically with their state transitions.
Generated plans and `planning.completed` metadata are committed together. The
Planning state check now occurs inside the proposal transaction. Manual plans
retain their existing API and evidence. No migration or schema changes.

Added `restart_fails_orphaned_planning_run`, `plan_and_event_atomic`, and
`concurrent_propose_on_cancelled_run_rejected`. The initial test run failed to
compile because recovery and atomic proposal APIs did not exist. All six plan
store tests pass. The atomicity test injects a SQLite trigger failure while
writing planning.completed and verifies rollback of plan, tasks and run state.
The race test accepts only a successful proposal or a successful cancellation,
never a stored plan on a cancelled run. Operations docs updated.

Validation on macOS arm64, 2026-11-07:
- `cargo fmt --all --check` — PASS.
- `cargo clippy --locked --offline --workspace --all-targets -- -D warnings` — PASS (removed an unused import caught by the first clippy run).
- `cargo test --locked --offline --workspace` — PASS, 274 tests.
- Python full gate on main (unchanged Python for this ticket): ruff lint/format, mypy and pytest — PASS, 118 tests.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 3` — PASS.
- `git diff --check` — PASS. No paid calls, push, tags or dependencies added.

## H-086 — Anthropic null-safe usage and documented API version

Cache read/creation nulls contribute zero without converting missing input or
output usage to known values. API version remains `2026-10-21`, now centralized
in ANTHROPIC_API_VERSION with a dated official source comment. Verified against
https://platform.claude.com/docs/en/api/versioning on 2026-11-07: the current
version history explicitly lists 2026-10-21. No live provider calls were made.

Added `anthropic_null_cache_fields_parse`, `anthropic_missing_usage_is_unknown`,
and parameterized `anthropic_non_end_turn_stop_reason_mapped`. The initial
null-cache regression failed as expected; null creation tokens raised TypeError
(and null cached usage was not zero). Other cases protect unknown usage and
existing rejection of nonterminal/truncated responses. Provider docs updated.

Validation on macOS arm64, 2026-11-07:
- `uv run --locked --offline --project intelligence pytest -q intelligence/tests/test_multi_provider.py` — before fix: 1 failed, 22 passed; after fix: 23 passed.
- From `intelligence/`: `uv run --locked --offline ruff check .`, `uv run --locked --offline ruff format --check .`, `uv run --locked --offline mypy`, `uv run --locked --offline pytest -q` — all PASS, 124 tests.
- `cargo fmt --all --check` — PASS.
- `cargo clippy --locked --offline --workspace --all-targets -- -D warnings` — PASS.
- `cargo test --locked --offline --workspace` — PASS, 271 tests.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 3` — PASS.
- `git diff --check` — PASS. No paid calls, push or tags.

## H-084 — Prevent blind whole-file rewrites of large files

Existing files above the 64 KiB read-output cap require complete line coverage
in the current broker session before a content rewrite. Coverage uses canonical
paths and session-keyed content fingerprints; changed files invalidate previous
coverage. Byte-clipped, scan-truncated or invalid-UTF8 reads do not authorize a
rewrite. Coverage is merged and the tracked path count is capped at 500. New
files and exact old/new replacement edits retain their existing size limits.
No dependencies, protocol or schema changes. Operations docs updated.

Tests: the first run of `cargo test --locked --offline -p harness-runtime --test broker_write`
failed `large_file_blind_rewrite_denied` as expected (9 passed, 1 failed).
After implementation, all 12 write tests and 7 read tests pass, including
`rewrite_after_full_read_allowed`, `new_file_content_allowed`, partial/stale
read coverage and byte-clipped-read denial. Existing traversal, symlink, secret
and write-size adversarial tests remain intact.

Validation on macOS arm64, 2026-11-07:
- `cargo fmt --all --check` — PASS.
- `cargo clippy --locked --offline --workspace --all-targets -- -D warnings` — PASS.
- `cargo test --locked --offline --workspace` — PASS, 276 tests (pre-H-083 base).
- Python full gate on main (unchanged Python for this ticket): ruff lint/format, mypy and pytest — PASS, 124 tests.
- `uv run --locked --offline --project intelligence python scripts/offline_demo.py --tasks 3` — PASS.
- `git diff --check` — PASS. No paid calls, push, tags or dependencies added.

After rebasing onto H-083/H-086, the full Rust gate passed again (279 tests).
The 100-task, fault and cancel offline demos also passed. The earlier H-085
post-merge rustdoc failure was caused by simultaneous worktrees replacing
shared target artifacts; serialized H-083 and H-086 main workspace gates both
passed in full, superseding that artifact failure.

## H-083b — Keep terminal planning status paired with fresh run state

The H-096 workspace gate exposed an existing read-order race in goal_status:
`planning.status` was failed while `run.state` was still the earlier Planning
snapshot. Evidence: `/private/tmp/nova-h096-rust.log` from the failed daemon API
suite before H-083b (19 passed, 1 failed). The endpoint now reloads the run after
reading its planning event; terminal events follow their associated transition.
No schema or protocol changes and no state-machine changes.

The original `goal_without_connected_provider_fails_with_visible_reason` test
passes unchanged. Added `terminal_planning_status_never_has_stale_run_state`,
which repeatedly polls ten isolated goals through failure and checks both
states together. All 21 daemon API tests pass. The new test protects the same
race observed in the existing test; without the reordered read, the first run
snapshot can predate the terminal event. Control API docs updated.

Validation on macOS arm64, 2026-11-07:
- `cargo fmt --all --check` — PASS.
- `cargo clippy --locked --offline --workspace --all-targets -- -D warnings` — PASS.
- `cargo test --locked --offline --workspace` — PASS, 280 tests.
- From `intelligence/`: ruff lint/format, mypy and `uv run --locked --offline pytest -q` — PASS, 124 tests.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 3` — PASS.
- `git diff --check` — PASS. No paid calls, push, tags or dependencies added.

## H-087a — Stable prompt ordering and Anthropic cache boundary

H-087 split before implementation: H-087a owns prompt layout/cache boundaries;
H-087b owns cross-language cache token accounting and registry pricing, which
must update Rust contracts/runtime and Python together. H-087 is not complete
until both parts land.

Prompts serialize root sections in fixed order: tools/contract, required task
handoff, snippets, then volatile history. Anthropic converts that stable prefix
into a text block with ephemeral cache_control while preserving the complete
original prompt bytes across the stable and volatile blocks. Generic provider
messages remain compatible. Provider docs updated; no dependencies or schema
changes in this sub-ticket.

Initial focused tests: 2 failed, 34 passed, reproducing volatile history before
the stable prefix and absence of a cache boundary. After implementation,
`stable_prefix_identical_across_turns` and
`anthropic_cache_control_on_stable_block` pass (36 focused tests total).
Full Python gate passes: `uv run --locked --offline ruff check .`,
`uv run --locked --offline ruff format --check .`, `uv run --locked --offline mypy`,
`uv run --locked --offline pytest -q` from intelligence/ — 126 tests, 36 typed files.

Cache control is opt-in and disabled by default in H-087a, so deploying prompt
ordering cannot introduce cache-write charges before Rust accounts for them.
H-087b must wire the option only with cache-aware accounting.

macOS arm64, 2026-11-07:
- `cargo fmt --all --check` — PASS.
- `cargo clippy --locked --offline --workspace --all-targets -- -D warnings` — PASS.
- `cargo test --locked --offline --workspace` — PASS, 274 tests (pre-H-084 base).
- Final Python gate — PASS, 127 tests, including default-disabled cache control.
- `intelligence/.venv/bin/python scripts/offline_demo.py --tasks 3` — PASS.
- `git diff --check` — PASS. No paid calls, push, tags or dependencies added.

## H-096 — Precise secret detection without blocking type annotations

Known AWS, OpenAI-style, GitHub, Slack, PEM and JWT formats are detected.
Assignments of quoted literals to secret-like names require at least 20
characters and Shannon entropy >= 3.5; typed declarations, env lookups, low
entropy values and angle-bracket placeholders remain allowed. Findings include
only path and line, never the matched value. Existing scope, protected-path,
diff-size and human review checks remain unchanged. Operations docs updated.

Tests added: `secret_detection_table` (over 20 positive/negative cases) and
`finding_never_echoes_secret`. Before implementation: both failed (3 passed,
2 failed); after implementation: all 5 diff gate tests passed. Fixtures use
synthetic strings only. No dependencies or protocol/schema changes.

Final validation on macOS arm64, 2026-11-07, after H-083b and H-087a rebase:
- `CARGO_TARGET_DIR=/private/tmp/nova-h096-target cargo fmt --all --check` — PASS.
- `CARGO_TARGET_DIR=/private/tmp/nova-h096-target cargo clippy --locked --offline --workspace --all-targets -- -D warnings` — PASS.
- `CARGO_TARGET_DIR=/private/tmp/nova-h096-target cargo test --locked --offline --workspace` — PASS, 282 tests.
- Python ruff lint/format, mypy, pytest gate — PASS; 127 tests after rebase.
- Offline demo using `/private/tmp/nova-h096-target/debug`: tasks 3, 100, fault and cancel — all PASS.
- `git diff --check` — PASS. No paid calls, push, tags or dependencies added.
Initial gate attempts exposed a missing worktree venv (fixed with uv sync) and
the planning-status race fixed separately as H-083b. Separate target directories
now isolate worktree compiler artifacts from main.

## H-087b — Compatible cache price and token metadata

Added optional cached-input/cache-write prices to the model registry and saved
model metadata, validating finite non-negative values and preserving prices
when constructing the live registry. Existing JSON configurations deserialize
without new fields. Catalog/CLI-created models leave unknown prices absent.
Provider results carry an optional cache_creation_input_tokens count; this is
inside the existing arbitrary tool result, not a new worker envelope field.
Anthropic preserves it independently from total and cached-read input tokens.

Added optional_cache_prices_roundtrip_and_validate and
anthropic_cache_creation_count_preserved. The Python regression failed before
the implementation (1 failed, 25 passed), then all 26 provider tests passed.
The Rust registry regression references missing fields on the baseline; it
covers old-config compatibility, roundtrip preservation and negative prices.
H-087c still needs to apply these prices authoritatively and enable caching.

Validation on macOS arm64, 2026-11-07, isolated build target:
- `CARGO_TARGET_DIR=/private/tmp/nova-h087b-target cargo fmt --all --check` — PASS.
- `CARGO_TARGET_DIR=/private/tmp/nova-h087b-target cargo clippy --locked --offline --workspace --all-targets -- -D warnings` — PASS.
- `CARGO_TARGET_DIR=/private/tmp/nova-h087b-target cargo test --locked --offline --workspace` — PASS, 281 tests (pre-H-096 base).
- From `intelligence/`, ruff lint/format, mypy, `uv run --locked --offline pytest -q` — PASS, 128 tests.
- Build plus offline demos using isolated debug binaries: tasks 3 and 100, fault, cancel — all PASS.
- `git diff --check` — PASS. No paid calls, push or tags.

## H-087c — Authoritative cache-aware costs and safe cache enablement

Rust computes cache-aware usage costs using registry prices and validates that
cache buckets do not exceed total input. Model grants reserve the highest
configured input/cache rate, including the write premium. Missing required
cache prices yield unknown cost, never zero. Python mirrors the estimate for
its usage report; Rust remains the sole accounting authority. Anthropic adapter
cache control is enabled only when both cache prices are configured. CLI model
configuration exposes optional cached-input/cache-write prices. Provider docs
updated; no dependency or worker envelope schema changes.

Tests added: cached_tokens_priced_at_cached_rate in Rust and Python,
cache_write_premium_is_reserved_before_call, missing-cache-price and invalid
cache-total regressions. Python pricing test initially failed collection because
estimate_cost did not exist. Rust pricing methods likewise did not exist on the
baseline; the grant test would previously allow a call exceeding its cap at the
cache-write rate. Focused tests pass: 4 registry, 5 live policy, 7 Python
pricing/coder tests. Full Python gate: ruff lint/format, mypy and pytest — PASS,
131 tests. No paid provider calls were used.

Final validation on macOS arm64, 2026-11-07:
- `CARGO_TARGET_DIR=/private/tmp/nova-h087c-target cargo fmt --all --check` — PASS.
- `CARGO_TARGET_DIR=/private/tmp/nova-h087c-target cargo clippy --locked --offline --workspace --all-targets -- -D warnings` — PASS.
- `CARGO_TARGET_DIR=/private/tmp/nova-h087c-target cargo test --locked --offline --workspace` — PASS, 285 tests.
- Full Python gate (ruff lint/format, mypy, pytest) — PASS, 131 tests.
- Build and offline demos (isolated target binaries): 3 tasks, 100 tasks, fault and cancel — all PASS.
- `git diff --check` — PASS. No paid calls, push or tags.

## H-088 — Bounded multi-turn coder working memory

The coder retains six bounded action/tool/args/result/summary/notes records,
compacting only older turns into deterministic file read ranges, changed files,
command argv/exit codes and hypotheses. Memory serialization is bounded to a
24 KiB default maximum, reduced for smaller runtime contexts and worst-case
JSON escaping. Record excerpts are bounded before insertion and last-six
records are never re-summarized. Summary collections are capped and omitted
facts counted, prioritizing changed-file retention. The action schema adds an
optional notes field with a strict 2 KiB UTF-8 cap, preserving older outputs.
The prompt builder supports an explicit bounded history allowance without
changing the default 2 KiB limit for other callers. Intelligence README updated.

Required tests memory_retains_changed_files_after_compaction,
memory_block_within_cap and last_k_turns_verbatim initially failed collection
because WorkingMemory did not exist. All now pass, including exact recent
args/results comparison, command exit-code retention, model-facing transcript
coverage and multibyte notes limits. No dependency or worker protocol changes.

Final validation on macOS arm64, 2026-11-07:
- `CARGO_TARGET_DIR=/private/tmp/nova-h088-target cargo fmt --all --check` — PASS.
- `CARGO_TARGET_DIR=/private/tmp/nova-h088-target cargo clippy --locked --offline --workspace --all-targets -- -D warnings` — PASS.
- `CARGO_TARGET_DIR=/private/tmp/nova-h088-target cargo test --locked --offline --workspace` — PASS, 285 tests.
- Full Python gate (ruff lint/format, mypy and pytest) — PASS, 137 tests, 39 typed files.
- Build plus isolated-binary offline demo, 3 tasks — PASS.
- `git diff --check` — PASS. No paid calls, push, tags or dependencies added.

## H-089 — Let coders run approved acceptance checks

Per-task broker policy now seeds exact human-approved acceptance argv without
prefix grants. These grants apply only at the canonical worktree root; extra
args or another cwd still require approval. Task timeout sets the broker's
default and maximum command timeout (validated task ceiling 24 hours), while
output caps stay unchanged. The coder prompt requires acceptance checks before
completion and fix/rerun on failure; claimed_tests records only observed exit-0,
non-timeout exact checks at the root, as lossless JSON argv strings. A later
failed rerun removes a previous passing claim. Rust still verifies independently.
Project gate commands will be added with H-097; no project gate profile exists
yet. Operations docs updated. No dependencies or protocol/schema changes.

Baseline was the passing H-088 gate (285 Rust, 137 Python). Initial Rust tests
failed to compile as expected (missing acceptance constructor and task timeout
method), log /private/tmp/nova-h089-red.log. Initial Python test failed on empty
claimed_tests (1 failed, 7 passed). New tests:
- approved_acceptance_argv_runs_without_prompt
- modified_argv_still_requires_approval (also tests alternate cwd)
- command_cannot_raise_task_timeout
- coder_runs_tests_before_completing and failed-check claim exclusion
- engine_seeds_exact_acceptance_commands_for_live_coder: disposable repo and
  scripted loopback provider, captures subsequent model context to verify actual
  stdout/exit code from the Rust command broker and no pending approvals.
Focused results: 10 broker command tests, 1 new engine e2e test, 8 coder tests
PASS. Full Python gate passes: ruff lint/format, mypy and pytest — 139 tests.

Final validation on macOS arm64, 2026-11-07:
- `CARGO_TARGET_DIR=/private/tmp/nova-h089-target cargo fmt --all --check` — PASS.
- `CARGO_TARGET_DIR=/private/tmp/nova-h089-target cargo clippy --locked --offline --workspace --all-targets -- -D warnings` — PASS.
- `CARGO_TARGET_DIR=/private/tmp/nova-h089-target cargo test --locked --offline --workspace` — PASS, 289 tests.
- Python gate: ruff lint/format, mypy, pytest — PASS, 139 tests, 39 typed files.
- Build and isolated offline demos: tasks 3 and 100, fault and cancel — all PASS.
- `git diff --check` — PASS. No paid calls, push, tags or dependencies added.
