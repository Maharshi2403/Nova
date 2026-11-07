# Control API v1 and native desktop client

The daemon binds loopback. `daemon.json` holds the port. Every `/v1` request
requires `Authorization: Bearer <token>` with the 256-bit token stored in
`daemon.token` (mode 0600). The data directory is mode 0700 on Unix. `/healthz`
is open and minimal. Requests with a foreign Host header or a browser Origin
are denied; there is no permissive CORS. The native desktop uses the CLI stdin
transport and never receives the token in argv or a URL. Provider credentials
remain separate and are required for live model calls.

JSON rejects unknown fields. Typical statuses: 422 invalid values/plan; 409 stale
revision/conflict; 404 missing resource; 401 auth; 403 hostile origin; 429 event
capacity. Execution 202 means accepted, not verified completion.

| Method / path | Contract |
|---|---|
| GET `/healthz` | Minimal status |
| GET `/v1/control` | Authenticated status |
| POST `/v1/shutdown` | Authenticated idle shutdown; 409 while execution is active |
| POST `/v1/projects` | `{name,repo_path,trusted_local:false}` (trust optional) |
| POST `/v1/runs` | `{project_id}` creates planning run |
| GET `/v1/runs/{run}` | Project/state/revision |
| POST `/v1/goals` | `{project_id,goal}` creates a run; the main agent plans subtasks in the background (202) |
| GET `/v1/goals/{run}` | Planning status: `planning`, `proposed` (model, subtasks, tokens, cost) or `failed` (reason) |
| POST `/v1/plans` | `{run_id,plan}` validates schema/DAG and persists version |
| GET `/v1/plans/{run}/{version}` | Inspect proposal/approval; `requires_attention` lists task IDs whose acceptance uses `make`, `npm`, `pnpm` or `yarn` scripts/targets |
| POST `/v1/plans/{run}/{version}/approve` | Approve version; materialize tasks |
| POST `/v1/runs/{run}/execute` | `{allowed_test_commands,concurrency,live:false,committed_snapshot:false,run_cap_microdollars:null,daily_cap_microdollars:null}`. Interpreter and live registry come from daemon settings; optional request caps can only lower saved ceilings. Unknown fields, including `python` and `model_config`, return 422. |
| POST `/v1/runs/{run}/pause` | Prevent new leases; assigned work continues |
| POST `/v1/runs/{run}/resume` | Restore eligibility; re-execute if loop drained |
| POST `/v1/runs/{run}/cancel` | Cancel eligible tasks/signal workers |
| GET `/v1/runs/{run}/tasks` | Persisted specs/states/revisions/attempts |
| PATCH `/v1/tasks/{run}/{task}/priority` | `{revision,manual_priority:0..100 or null}` |
| GET `/v1/events?run_id=R&after=N&limit=100&wait_seconds=0` | Durable events after N |
| GET `/v1/usage?run_id=R` | Per-call usage; missing counts/cost remain null |
| GET `/v1/approvals?run_id=R` | Up to 1000 scoped approval records |
| POST `/v1/approvals/{id}/decision` | `{revision,approved:true/false}`; human audit |

Long-poll reconnects with the last sequence, so clients retain only events greater
than their cursor. Empty timeout pages are normal. Database pages and waiter
capacity bound memory; no unbounded in-memory fan-out exists.

A worker's completed message is a claim. Displays must use durable state and
verifier/commit/integration evidence. Revisions are compare-and-set: reload on 409.
Windows/Linux desktop UI, OS sandbox backends, remote hosting and automatic deployment are deferred.
Any future client must preserve Rust state authority and this trust boundary.

Native dashboard additions: GET `/v1/dashboard?offset=0&limit=100` returns
newest-first projects/runs with total counts (limit 1..200); projects include their
persisted trusted-local decision. GET `/v1/tasks/{run}/{task}/evidence` returns
recorded verifier command/exit/duration, task/integration commit identities,
and the quarantine ref/SHA when an integration check failed.
Both require bearer authentication and reject browser-origin requests.

Dashboard runs include their latest immutable `plan_version` when available.
Trusted project import checks the Git root/base but permits uncommitted changes;
execution still enforces a clean checkout.

Project controls: POST `/v1/projects/{id}/trust` with `{trusted_local:true/false}`
records an explicit trust decision; enabling checks the Git root and base without
requiring a clean checkout. DELETE `/v1/projects/{id}` removes only an empty
registration (409 when it has runs), never repository files. Both require bearer authentication and retain origin rejection.

Execute accepts optional `committed_snapshot` (default false). True uses the pinned
committed source without requiring local edits to be stashed; no dirty/untracked
files are read into worker worktrees. GET `/v1/runs/{run}/source` returns null before
first accepted execution, then `{base_sha,committed_snapshot}`. The first source is
immutable for the run; subsequent launches reuse its hash. Each accepted launch
also records an execution.launch event with its selected mode and pinned hash. Validation errors are
redacted and reported directly to the local operator instead of a generic
checklist.

Run detail (`GET /v1/runs/{run_id}`) includes nullable `reason`. Idle runs
with blocked or failed tasks transition to `failed`, with task IDs recorded in
a `run.failed` event. Budget or path-lock denial leaves the run `running` and
records a `run.stalled` reason once per unchanged reason. Execution can be
started again after the constraint is resolved. `nova status` and the desktop
run panel display this diagnostic.

Goal status reads the run state after its planning event. A terminal planning
outcome therefore cannot be paired with a stale pre-transition run state during
background planning completion.
