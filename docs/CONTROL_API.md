# Control API v1 and future desktop client

The daemon binds loopback. `daemon.json` holds the port; `api-token` holds a random
credential. All /v1 routes require Authorization: Bearer TOKEN. Health returns only
minimal status. Origin-bearing/cross-site browser requests are denied; there is no
permissive CORS. A future desktop client needs native authenticated transport and
must not put tokens in URLs/logs. Workers never receive this credential.

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
| POST `/v1/plans` | `{run_id,plan}` validates schema/DAG and persists version |
| GET `/v1/plans/{run}/{version}` | Inspect proposal/approval |
| POST `/v1/plans/{run}/{version}/approve` | Approve version; materialize tasks |
| POST `/v1/runs/{run}/execute` | `{python,allowed_test_commands,concurrency,live:false,model_config:null}` |
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
Tauri UI, OS sandbox backends, remote hosting and automatic deployment are deferred.
Any future client must preserve Rust state authority and this trust boundary.

Native dashboard additions: GET `/v1/dashboard?offset=0&limit=100` returns
newest-first projects/runs with total counts (limit 1..200); projects include their
persisted trusted-local decision. GET `/v1/tasks/{run}/{task}/evidence` returns
recorded verifier command/exit/duration and task/integration commit identities.
Both require the same bearer authentication and reject browser-origin requests.
