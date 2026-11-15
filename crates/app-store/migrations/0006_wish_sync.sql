CREATE TABLE wish_sync_state (
 profile_id TEXT PRIMARY KEY REFERENCES profiles(profile_id),
 capture_id TEXT NOT NULL,
 epoch TEXT NOT NULL,
 status TEXT NOT NULL CHECK(status IN ('requested','syncing','synced','needs_access','failed','cancelled')),
 updated_ms INTEGER NOT NULL,
 fetched_ms INTEGER
);
