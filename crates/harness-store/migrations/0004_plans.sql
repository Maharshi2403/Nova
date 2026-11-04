-- Immutable plan versions. A plan's content never changes once stored; only its approval
-- status moves, and only once (proposed -> approved | rejected).
CREATE TABLE plans (
    run_id       TEXT NOT NULL REFERENCES runs (id) ON DELETE RESTRICT,
    version      INTEGER NOT NULL CHECK (version >= 1),
    status       TEXT NOT NULL CHECK (status IN ('proposed', 'approved', 'rejected')),
    content_json TEXT NOT NULL CHECK (length(content_json) > 0),
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    decided_at   TEXT,
    PRIMARY KEY (run_id, version)
) STRICT;

CREATE TRIGGER plans_content_immutable
BEFORE UPDATE OF run_id, version, content_json, created_at ON plans
BEGIN
    SELECT RAISE(ABORT, 'plan content is immutable');
END;

CREATE TRIGGER plans_decision_once
BEFORE UPDATE OF status ON plans
WHEN OLD.status <> 'proposed'
BEGIN
    SELECT RAISE(ABORT, 'plan already decided');
END;

CREATE TRIGGER plans_no_delete BEFORE DELETE ON plans
BEGIN
    SELECT RAISE(ABORT, 'plans are retained for audit');
END;
