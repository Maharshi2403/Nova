-- Leases give one worker exclusive ownership of a task attempt. Times are Unix seconds
-- supplied by the caller so expiry is deterministic and testable.
CREATE TABLE leases (
    id          TEXT PRIMARY KEY NOT NULL CHECK (length(id) > 0),
    run_id      TEXT NOT NULL,
    task_id     TEXT NOT NULL,
    worker_id   TEXT NOT NULL CHECK (length(worker_id) > 0),
    attempt     INTEGER NOT NULL CHECK (attempt >= 1),
    granted_at  INTEGER NOT NULL,
    expires_at  INTEGER NOT NULL CHECK (expires_at > granted_at),
    released_at INTEGER,
    FOREIGN KEY (run_id, task_id) REFERENCES tasks (run_id, id) ON DELETE RESTRICT
) STRICT;

-- At most one live lease per task, enforced by the database itself.
CREATE UNIQUE INDEX idx_leases_one_active ON leases (run_id, task_id) WHERE released_at IS NULL;
CREATE INDEX idx_leases_expiry ON leases (expires_at) WHERE released_at IS NULL;
