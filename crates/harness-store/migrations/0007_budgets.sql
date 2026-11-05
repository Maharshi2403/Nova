-- Integer microdollars avoid floating point budget comparisons.
CREATE TABLE budget_reservations (
    lease_id TEXT PRIMARY KEY NOT NULL REFERENCES leases(id),
    run_id TEXT NOT NULL,
    task_id TEXT NOT NULL,
    utc_day INTEGER NOT NULL,
    amount INTEGER NOT NULL CHECK (amount >= 0),
    released INTEGER NOT NULL DEFAULT 0 CHECK (released IN (0, 1)),
    FOREIGN KEY (run_id, task_id) REFERENCES tasks(run_id, id)
) STRICT;
CREATE INDEX budget_run ON budget_reservations(run_id, released);
CREATE INDEX budget_day ON budget_reservations(utc_day, released);
