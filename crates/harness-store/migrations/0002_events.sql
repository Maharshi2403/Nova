-- Append-only audit log. Written in the same transaction as the state change it records.
CREATE TABLE events (
    sequence        INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id          TEXT NOT NULL REFERENCES runs (id) ON DELETE RESTRICT,
    task_id         TEXT,
    kind            TEXT NOT NULL CHECK (length(kind) > 0),
    from_state      TEXT,
    to_state        TEXT,
    attempt         INTEGER,
    idempotency_key TEXT UNIQUE CHECK (idempotency_key IS NULL OR length(idempotency_key) > 0),
    payload_json    TEXT NOT NULL DEFAULT '{}',
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_events_run_sequence ON events (run_id, sequence);

CREATE TRIGGER events_no_update BEFORE UPDATE ON events
BEGIN
    SELECT RAISE(ABORT, 'events are append-only');
END;

CREATE TRIGGER events_no_delete BEFORE DELETE ON events
BEGIN
    SELECT RAISE(ABORT, 'events are append-only');
END;
