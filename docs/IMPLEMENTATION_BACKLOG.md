# Remaining implementation scope

The supplied TASK_BACKLOG.md remains the governing ticket order. Inspection after
H-052 found that the original H-038 E2E bypassed CLI, worker processes and broker
writes. The daemon only advanced readiness. G4/G5 therefore cannot be considered
fully demonstrated by the earlier commits.

Before implementation, split H-053 into these bounded children:

- H-053a: connect a real supervised worker, broker, verifier and integrator through
  the bounded pool to an explicit authenticated run execution endpoint and CLI.
  Include offline execution evidence on a disposable repository. Depends H-049..052.
- H-053b: add deterministic 100-task load and fault/restart coverage for the
  connected runtime. Depends H-053a. This completes H-053's original test scope.

H-054..H-060 retain their original scope and depend on both H-053 children wherever
the original dependencies named H-053. Security and release gates require actual
Windows/macOS CI evidence; local macOS results alone never establish those gates.
Live provider requests remain opt-in; no paid API requests are authorized by this
implementation run. Existing adapter capabilities are tested with fake HTTP.

## Live execution gap discovered before release checks

H-043's original provider adapter was not connected to a coder or daemon dispatch.
Complete these bounded backfill tickets after H-057, before H-058 release checks:

- H-043b: structured live coder loop using the existing adapter, bounded prompt
  builder and retry policy; broker authorization before every provider attempt;
  fake-provider unit tests and protocol-only stdout. Depends H-043..046,H-056.
- H-043c: Rust-owned live model routing, conservative per-attempt spend reservation,
  usage pricing and explicit CLI opt-in, credential isolation and command broker
  wiring. Depends H-043b,H-057. No real provider calls during implementation.

H-058's full validation includes both backfill tickets. This is recorded before
implementation so an adapter-only build is not presented as a complete live harness.

## H-058a release-review fixes

Before documenting operator workflows, fix repeated pause/resume audit keys,
concurrent scheduler compare-and-set races, and per-run worktree-registry ownership.
These were found during H-059 workflow review; verify repeat control cycles and
keep each run's registry separate. No new product scope or remote release.

## H-061 — Nova command and local lifecycle

User-requested follow-up: canonical `nova` binary, bare-command/start launcher,
authenticated attach, safe idle shutdown, user-local PATH installation, and lifecycle
regression tests. Keep `harness` compatibility and existing state paths accessible.
Do not start coding agents, approve plans, or enable a paid provider on startup.

## Multi-provider follow-up (user authorized)

- H-062: Anthropic Messages and OpenAI-compatible Chat Completions adapters,
  bounded model discovery, strict response validation and offline HTTP fixtures.
- H-063: saved provider profiles and native credential-vault setup, hidden key
  prompts, model catalog and CLI provider/model management. No keys in JSON/logs.
- H-064: task use-case routing across configured providers, saved defaults and
  overrides, integration into budgeted execution, docs, native install and smoke.

Every provider attempt remains Rust-authorized; unsupported/unpriced models must
be explicit rather than silently assigned zero cost. No paid test calls. Arbitrary
protocols need adapters; compatible endpoints can use the generic adapter.

## H-065 — preserve configured venv interpreter path

Release bundle smoke found that canonicalizing a Python venv's executable symlink
selected the system interpreter and lost its installed package. Keep the checked
absolute venv entry path, test symlink behavior, rebuild the bundle and rerun smoke.

## Native desktop follow-up (user authorized)

- H-066: bounded authenticated dashboard project/run inventory and task verification
  evidence. Keep SQL in the store and preserve browser-origin rejection.
- H-067: native macOS dashboard with CLI stdin bridge, provider/key setup, model
  routes, guided project/task-plan creation, explicit approve/execute controls,
  live run/task/event/usage/evidence views. No model generation during development.
- H-068: desktop app packaging, `nova ui` launch, desktop installation and workflow
  smoke. Closing the window leaves the independent daemon running. Other desktop
  platforms remain explicit follow-up work; do not claim a cross-platform GUI.

Native transport keeps the daemon credential out of the UI, browser and URLs.
The task form constructs an operator-authored plan; automatic goal decomposition
is a later feature and must not be presented as implemented here.
