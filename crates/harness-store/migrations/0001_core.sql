-- Core relational schema. Rust owns all writes; keep every constraint in the DB as well.
CREATE TABLE projects (
    id         TEXT PRIMARY KEY NOT NULL,
    name       TEXT NOT NULL CHECK (length(trim(name)) > 0),
    repo_path  TEXT NOT NULL CHECK (length(trim(repo_path)) > 0),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE TABLE runs (
    id         TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL REFERENCES projects (id) ON DELETE RESTRICT,
    state      TEXT NOT NULL CHECK (state IN
        ('planning', 'awaiting_approval', 'running', 'paused', 'completed', 'failed', 'cancelled')),
    revision   INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE INDEX idx_runs_project ON runs (project_id);

CREATE TABLE tasks (
    run_id          TEXT NOT NULL REFERENCES runs (id) ON DELETE RESTRICT,
    id              TEXT NOT NULL,
    state           TEXT NOT NULL CHECK (state IN
        ('proposed', 'pending', 'ready', 'leased', 'running', 'verifying', 'reviewing',
         'integrating', 'retry_wait', 'blocked', 'completed', 'failed', 'cancelled')),
    revision        INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
    attempt         INTEGER NOT NULL DEFAULT 0 CHECK (attempt >= 0),
    agent_priority  INTEGER NOT NULL CHECK (agent_priority BETWEEN 0 AND 100),
    manual_priority INTEGER CHECK (manual_priority IS NULL OR manual_priority BETWEEN 0 AND 100),
    spec_json       TEXT NOT NULL,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (run_id, id)
) STRICT;

CREATE TABLE task_dependencies (
    run_id        TEXT NOT NULL,
    task_id       TEXT NOT NULL,
    depends_on_id TEXT NOT NULL,
    PRIMARY KEY (run_id, task_id, depends_on_id),
    CHECK (task_id <> depends_on_id),
    FOREIGN KEY (run_id, task_id) REFERENCES tasks (run_id, id) ON DELETE CASCADE,
    FOREIGN KEY (run_id, depends_on_id) REFERENCES tasks (run_id, id) ON DELETE RESTRICT
) STRICT;
