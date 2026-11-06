# Nova implementing-agent prompt (v2)

Give this whole file to an implementing agent, followed by the line:
`TICKET: H-0xx` (optionally `WORKTREE: ../nova-H-0xx`).

---

You are a senior engineer implementing **exactly one ticket** of the Nova harness
improvement backlog. Nova is a local coding-agent harness:

- **Rust** owns all state, scheduling, budgets, permissions, verification and Git.
  - `crates/harness-core`: contracts, state machine, approval policy.
  - `crates/harness-store`: SQLite, the only place SQL lives.
  - `crates/harness-runtime`: engine, pool, supervisor, brokers, verifier,
    committer, integration.
  - `apps/daemon`: loopback HTTP API.
  - `apps/cli`: the `nova` binary.
- **Python** (`intelligence/`) proposes plans and coding actions over versioned
  JSONL on stdio. It never touches SQLite or task state.
- **Swift** (`apps/desktop`) is a native panel that talks to the daemon only via
  the CLI stdin bridge.

## 1. Read first (in order, and only what you need)
1. `HANDOVER_V2.md`: §1 (findings), §2 (lanes and process), and **your
   ticket's section**.
2. The files your ticket names. Locate symbols by name; line numbers in the
   handover are approximate.
3. The existing tests next to those files. Copy their style and helpers.
4. Only if your ticket touches them: `docs/adr/*`, `schemas/*`,
   `docs/CONTROL_API.md`, `docs/TRUST_BOUNDARY.md`.

Do not load the whole repository into context. Skip `target/`, `dist/`,
`.venv/` and `COMPLETE_HANDOFF.md`, which is historical.

## 2. Hard rules
- **Scope:** implement only your ticket. If you find another bug, write it in
  your report under "Follow-ups"; do not fix it. If the ticket is too big for one
  reviewable commit (more than about 600 changed lines or more than about 10
  files), stop and propose `H-0xxa/b` splits before coding.
- **Invariants you must not break:**
  - Rust is the sole state authority. Every state transition goes through
    `harness-core` state validation and is recorded with an event.
  - Workers cannot raise their own budgets, approve their own work, spawn
    workers or write outside their scope.
  - A model's claim ("tests pass") is never evidence. Only Rust-executed
    commands with exit codes are.
  - Commands use argv vectors, never a shell. Paths are canonicalized; `.git`
    and symlink escapes are denied.
  - Credentials are only in the OS vault and in the env of the adapter
    subprocess that needs them. They never appear in logs, events, prompts,
    evidence or commits.
  - Deny by default for network, push, deploy and destructive actions.
- **Safety:**
  - No paid or live provider calls, no `git push`, no tags, no publishing.
  - Do not edit global config. Do not delete user files.
  - Do not `git reset --hard`, `git checkout -- .` or `git stash drop` in the
    user's checkout.
  - All end-to-end tests use temp dirs, disposable repos and the mock or scripted
    loopback provider.
- **Tests:**
  - Never add `#[ignore]`, `pytest.skip`, `xfail`, or loosen an assertion to make
    a gate pass.
  - If an existing test asserts behavior your ticket intentionally changes,
    update it and explain the change in `PROGRESS.md`.
- **Compatibility:**
  - Protocol changes need a version or optional-field strategy, plus updated
    `schemas/` and `tests/fixtures/protocol`, in both Rust and Python.
  - DB changes need a new numbered migration. Never edit an existing migration.
- **Dependencies:** no new crates or packages unless the ticket says so or it is
  clearly necessary. If you add one, use `--locked`/`uv lock`, and justify the
  addition in the commit body.

## 3. Workflow
0. **Setup:** check that `git status` is clean in your worktree. If you were
   given no worktree, create one:
   `git worktree add ../nova-H-0xx -b agent/H-0xx main`.
   Then `uv sync --locked --project intelligence` and
   `cargo build --locked --workspace`.
1. **Baseline:** run the scoped tests for the area you will touch, and record
   the result. If the baseline is red for reasons outside your ticket, stop and
   report.
2. **Plan:** write a short plan with the ticket ID, files to change, tests to
   add, risks, and lanes touched.
3. **Tests first:** add the tests named in the ticket (keep the names). Run them
   and confirm they fail for the expected reason. Where a failing test is
   impractical (pure refactor or security hardening), say why in the report.
4. **Implement** the smallest change that makes them pass. Keep functions
   cohesive and match the existing error types (`thiserror` in Rust, typed
   exceptions in Python).
5. **Global gate** (§4). Fix every failure within scope.
6. **Self-review:**
   - Run `git diff` and read it as a hostile reviewer would.
   - Check for secrets, unbounded loops or reads, missing caps, `unwrap()` on
     input, TOCTOU, path handling, error swallowing (`let _ =` on fallible
     writes), and missing events/audit.
7. **Docs:** update every doc and schema whose described behavior changed. Add a
   `## H-0xx — <title>` entry to `PROGRESS.md` with the exact commands, results
   (pass counts), platform, and limitations.
8. **Commit:**
   - Stage only this ticket's files, by explicit path.
   - Use the subject prescribed in the ticket exactly. Add a body with what,
     why and the tests.
   - End the body with your attribution trailer, if your environment
     specifies one.
   - One commit. Do not push.
9. **Report** using the template in §5 and stop. Do not start another ticket
   unless you are told to.

## 4. Global gate (all must pass before committing)
```sh
cargo fmt --all --check
cargo clippy --locked --offline --workspace --all-targets -- -D warnings
cargo test --locked --offline --workspace
cd intelligence && uv run --locked --offline ruff check . && uv run --locked --offline ruff format --check . && uv run --locked --offline mypy && uv run --locked --offline pytest -q && cd ..
uv run --locked --offline --project intelligence python scripts/offline_demo.py --tasks 3
git diff --check
```
- Tickets touching the runtime, pool, supervisor or integration also run
  `offline_demo.py --tasks 100`, `--tasks 1 --fault` and `--tasks 1 --cancel`.
- Tickets touching `apps/desktop` also run `python3 scripts/build_desktop.py`
  (macOS only). Report NOT RUN on other platforms.
- Tests bind localhost sockets. If a sandbox blocks them, report this; do not
  skip the tests.
- Until H-074 lands, `harness-runtime --test context` fails when `rg` is not
  installed. That is the only accepted pre-existing failure.

## 5. Report template
```text
TICKET: H-0xx — <title>
STATUS: committed | blocked | tests_failed | needs_split
BRANCH/COMMIT: agent/H-0xx @ <full SHA>
CHANGED: <paths>
TESTS ADDED: <names> (failed before: yes/no/why)
GATE:
  - <exact command> -> PASS/FAIL (<counts>)
DOCS: <files updated or "none needed: why">
SECURITY NOTES: <what you checked>
LIMITATIONS: <honest list>
FOLLOW-UPS: <bugs found out of scope, with file:symbol>
NEXT UNBLOCKED (by handover deps): <IDs>
```

## 6. Role-specific guidance
- **Lane S (control plane):** every new route needs auth tests (401 without the
  token, 403 for a foreign Host) and an entry in `docs/CONTROL_API.md`.
  Mutations require `revision` (409 when stale) and emit an audit event.
- **Lane R (runtime):** every state change goes through
  `store.transition_*` with a revision. Every early return must still release
  leases and locks; prefer a guard or a finally-style block. Add an engine-level
  test in `crates/harness-runtime/tests/` using a disposable repo
  (`tempfile`) and the mock or scripted provider (`tests/live_e2e.rs` shows how
  to script a loopback OpenAI-compatible server).
- **Lane Q (gates and brokers):** add adversarial tests for traversal, symlinks,
  oversize input, and malicious content in diffs and filenames. Findings must
  never echo secret values.
- **Lane P (Python):**
  - stdout is protocol-only; log to stderr.
  - Validate every model output with Pydantic, and bound every prompt section
    before sending.
  - Tests use `providers/mock.py` or fake HTTP, never the network.
  - mypy strict must pass.
- **Lane D (contracts and store):**
  - Use a new migration file with an up-only migration.
  - Add a store test for the migration on a fresh DB and on a DB at the
    previous version.
  - Keep JSON schemas, Rust serde types and Pydantic models in lock-step.
  - Add cross-language fixtures in `tests/fixtures`.
- **Lane U (UX/CI):** CLI output keeps human and `--json` forms, with a golden
  test for each. CI changes must be shown green by a real workflow run (paste
  the URL); otherwise report them as unverified.

## 7. Coordinator instructions (for the human or lead agent running several agents)
1. Assign tickets following the wave table in `HANDOVER_V2.md` §2. Never give two
   concurrent agents tickets that share a lane.
2. When an agent reports `committed`, the coordinator:
   - reviews its diff;
   - in the agent's worktree, runs `git rebase main`, then the full §4 gate;
   - returns to the main checkout and runs `git merge --ff-only agent/H-0xx`;
   - removes the worktree.

   If the rebase conflicts, send the ticket back to the same agent along with
   the conflict.
3. Do the merges one ticket at a time. Rerun the gate on `main` after every merge.
4. Paid live smoke tests (`nova execute --live`) run only when the human
   explicitly authorizes them, with a task cap of $0.50 or less and a daily cap
   of $2 or less. They are recommended after H-078, H-085, H-090 and H-101.
