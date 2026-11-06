# Operator workflows and recovery

## Plan and execute

Run the self-contained README demo first. The following command forms use IDs
returned by earlier commands; replace PROJECT_ID, RUN_ID and TASK_ID with those
values. Put `--daemon-dir PATH` before the command when needed; Windows uses `.exe`.

1. `nova --json init /absolute/repo --name Demo --trust-local` returns a project ID.
2. `nova --json run PROJECT_ID` creates a planning run and returns its ID.
3. Create a plan matching `schemas/plan.schema.json`, with that project's ID and
   executable acceptance test argv. Python's `harness_ai.planner.propose_plan` can
   generate typed proposals. The current CLI submits JSON files; it does not accept
   an idea string directly.
4. `nova plan RUN_ID plan.json` persists an immutable proposed version.
5. Review scope, risk and tests before `nova approve RUN_ID 1`. Plan approval
   makes tasks eligible; it does not authorize arbitrary commands.
6. Write `approved-tests.json`, an array of exact argv arrays, for example
   `[["/absolute/venv/bin/python","-m","pytest","-q"]]`. Windows uses the absolute
   `Scripts/python.exe` path. Approving a test runner/interpreter authorizes code
   execution with your privileges in trusted-local mode.
7. `nova config set python ABSOLUTE_INTELLIGENCE_PYTHON` (once), then `nova execute RUN_ID --test-commands approved-tests.json`
   runs the synthetic mock coder. Its candidate paths must be exact fixture files;
   it cannot solve arbitrary coding tasks. See live mode for actual coding.

Completed tasks have verifier evidence and task/integration SHAs. Task branches are
`harness/RUN_ID/TASK_ID-aATTEMPT`; integration is on `harness/integration/RUN_ID`.
The original checkout is not advanced. Inspect the integration branch and merge it
yourself when satisfied. Nothing is pushed automatically.

## Inspect and control

- `nova status RUN_ID` shows run state; `nova task RUN_ID` lists task specs,
  states, revisions and attempts. Put `--json` before the command for JSON output.
- `nova priority RUN_ID TASK_ID REVISION --manual-priority 90` changes queued
  order. Omit the priority to clear it. A stale revision returns 409.
- `nova pause RUN_ID` prevents new leases and lets assigned work finish.
  `nova resume RUN_ID` restores eligibility. If the execution loop already drained
  and exited while paused, repeat `execute` with the same reviewed policy.
- `nova cancel RUN_ID` cancels eligible tasks, signals workers and prevents later
  commit/integration. Verification already running may finish; files are retained.
  Tasks in states without a direct cancelled transition become blocked.
- `nova logs RUN_ID --after 0 --limit 100` reads audit events. Pass the last
  sequence as the next cursor. The API also offers bounded long-poll.
- `nova approvals RUN_ID` lists scoped requests. Review argv, cwd and task attempt.
  `nova decision APPROVAL_ID REVISION --approve` approves once; omit `--approve`
  to deny. Workers cannot decide requests.

## Live coding

Connect providers with `nova providers add openai` and/or
`nova providers add anthropic`. The terminal hides each API key and stores it in
the native credential vault. See [provider setup](PROVIDERS.md) for compatible
endpoints, account discovery and per-use-case routing. Coder processes receive no
credentials; only a dedicated provider helper gets its selected connection's key.

```text
nova execute RUN_ID --test-commands approved-tests.json --live
```

The daemon selects the saved or bundled Python interpreter. Set a custom one with
`nova config set python ABSOLUTE_PYTHON` before execution. Live routing, prices,
and spend ceilings come from saved provider settings. Set ceilings with
`nova budget set --run USD --daily USD`; `nova execute --run-cap USD --daily-cap USD`
can lower them for one launch. Legacy native
OpenAI/Anthropic configurations can use daemon-scoped OPENAI_API_KEY or
ANTHROPIC_API_KEY when no saved profile exists. Keys never belong in argv or plans.

Rust chooses the configured complexity/risk route and reserves each task's full cap
before dispatch. Every request/retry must fit its input/output grant and remaining
estimated spend. Unknown/interrupted costs retain conservative reservations. Prices
are registry estimates, not a billing guarantee; cached counts are measured only
when supplied by the provider. Missing output usage stops further coder calls.

The coder reads/searches/patches assigned paths and requests commands through the
broker. An unapproved command creates a durable request and stops the attempt; it
does not execute or wait indefinitely. High-risk/large diffs stay blocked for review.
There is no automatic conflict/review resolution UI: inspect retained branches and
create a revised reviewed run as needed.

## Upgrade and recovery

Stop the daemon before backing up its private state directory. For a live filesystem
snapshot, preserve SQLite with its WAL/SHM files; copying only the active main file
is unsafe. Never put SQLite/WAL on a network filesystem. Startup applies checked-in
SQLx migrations once. Back up before upgrading; no automatic downgrade/deletion is
provided. Old projects without a trust record remain untrusted; explicitly register
a trusted project again.

Restart using the same directory. An OS lock rejects a second daemon, the saved
port is reused, and interrupted leases are reconciled before serving. Uncertain
attempts are blocked instead of replayed. Worktrees and uncommitted files remain.
Exact task/attempt commit trailers support no-duplicate recovery; acceptance and
committed diff checks rerun before acknowledging an unrecorded commit. Arbitrary
saved PIDs are not killed because they may have been reused.

- **Connection refused:** start the daemon using the directory selected by the
  CLI; inspect startup errors for an occupied port or held lock.
- **401/403:** token mismatch or browser-origin request. Native CLI authentication
  uses the protected token; browser-origin API access is denied.
- **Execution denied:** check explicit trust, approved/running plan, clean original
  repo, absolute Python path, exact test argv, routing capabilities and budgets.
- **Module missing:** run `uv sync --locked --project intelligence` and pass its venv
  Python. Workers use isolated imports so repo modules cannot shadow harness_ai.
- **Blocked/test failed:** inspect tasks, approvals, events and retained branches.
  SQLite `test_results` records commands, exit codes, duration and bounded redacted
  artifacts. Never manually edit task states or erase worktrees to fake completion.

Hosted Windows/macOS CI and an authorized budgeted live smoke are required before
claiming a fully validated release. Local tests alone do not establish those gates.

## Working with unfinished local files

Use `--committed-snapshot` with CLI execute, or leave Use committed snapshot selected
in desktop execution review. Nova runs the pinned last committed version in its
isolated worktrees. Tracked edits and untracked files stay untouched and are not
included as task input. The first accepted execution records its commit in an
append-only execution.source event; resumes reuse that revision. Create a new run
to use a newer commit. Trust, plan approval, exact command approvals, budgets and
verification remain enforced. Normal API/CLI mode still requires a clean checkout.
