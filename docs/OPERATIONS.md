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
7. `nova execute RUN_ID --python ABSOLUTE_INTELLIGENCE_PYTHON --test-commands approved-tests.json`
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

Set OPENAI_API_KEY only in the environment that starts the daemon. Never put it in
a plan, command argument, registry, stdout or committed file. Coder workers receive
no credentials: Rust invokes a separate credential-scoped adapter for each bounded
provider request. Test/generated command environments are scrubbed.

Copy `config/live.example.json` and review it. Replace the placeholder model ID,
capabilities, context limits and prices with values valid for your account. The
placeholder is deliberately unavailable. Config budgets are integer microdollars;
each task's `cost_cap_usd` is USD. Include authorization/completion broker calls in
the task's tool-call limit.

```text
nova execute RUN_ID --python ABSOLUTE_PYTHON --test-commands approved-tests.json --live --model-config reviewed-models.json
```

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
