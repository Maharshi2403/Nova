CREATE TABLE usage_records (
    message_id TEXT PRIMARY KEY NOT NULL,
    lease_id TEXT NOT NULL REFERENCES leases(id),
    payload TEXT NOT NULL CHECK(json_valid(payload))
) STRICT;
CREATE INDEX usage_lease ON usage_records(lease_id);
ALTER TABLE budget_reservations ADD COLUMN settled INTEGER NOT NULL DEFAULT 0 CHECK(settled IN (0, 1));
