# Implementation progress

Latest completed ticket: H-048 — bounded worker pool.
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

Next unblocked ticket: H-049 — shared serialized integration.
The original three untracked handoff documents are intentionally excluded from
the feature commit and left unchanged.
