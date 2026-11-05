CREATE TABLE git_artifacts (
    run_id                 TEXT NOT NULL,
    task_id                TEXT NOT NULL,
    task_commit_sha        TEXT NOT NULL CHECK (length(task_commit_sha) = 40),
    integration_commit_sha TEXT,
    diff_paths_json        TEXT NOT NULL CHECK (json_valid(diff_paths_json)),
    verified_at            TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (run_id, task_id),
    FOREIGN KEY (run_id, task_id) REFERENCES tasks (run_id, id) ON DELETE RESTRICT
) STRICT;

CREATE UNIQUE INDEX git_artifacts_task_commit_sha_idx ON git_artifacts (task_commit_sha);
