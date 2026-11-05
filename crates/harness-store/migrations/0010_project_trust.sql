CREATE TABLE project_trust (
    project_id TEXT PRIMARY KEY NOT NULL REFERENCES projects(id),
    trusted_local INTEGER NOT NULL DEFAULT 0 CHECK(trusted_local IN (0,1)),
    decided_by TEXT NOT NULL DEFAULT 'local_user'
) STRICT;
