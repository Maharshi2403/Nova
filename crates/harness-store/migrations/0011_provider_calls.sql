CREATE TABLE provider_calls (
    id TEXT PRIMARY KEY NOT NULL,
    lease_id TEXT NOT NULL REFERENCES leases(id),
    finished INTEGER NOT NULL DEFAULT 0 CHECK(finished IN (0,1))
) STRICT;
