CREATE TABLE approvals (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL,
    task_id TEXT NOT NULL,
    attempt INTEGER NOT NULL,
    action_json TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','approved','denied','consumed')),
    revision INTEGER NOT NULL DEFAULT 0,
    decided_by TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    FOREIGN KEY(run_id, task_id) REFERENCES tasks(run_id,id),
    UNIQUE(run_id,task_id,attempt,action_json)
) STRICT;
