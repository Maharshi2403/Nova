CREATE TABLE test_results (
    id            INTEGER PRIMARY KEY,
    run_id        TEXT NOT NULL,
    task_id       TEXT NOT NULL,
    command_json  TEXT NOT NULL CHECK (json_valid(command_json)),
    exit_code     INTEGER,
    duration_ms   INTEGER NOT NULL CHECK (duration_ms >= 0),
    artifact_json TEXT NOT NULL CHECK (json_valid(artifact_json)),
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    FOREIGN KEY (run_id, task_id) REFERENCES tasks (run_id, id) ON DELETE RESTRICT
) STRICT;

CREATE INDEX test_results_task_idx ON test_results (run_id, task_id, id);
