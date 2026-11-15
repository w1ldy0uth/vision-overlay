ALTER TABLE current_entities ADD COLUMN value_format TEXT NOT NULL DEFAULT 'legacy_capture_v1';
ALTER TABLE snapshots ADD COLUMN value_format TEXT NOT NULL DEFAULT 'legacy_capture_v1';
ALTER TABLE domain_state RENAME TO old_domain_state;
CREATE TABLE domain_state (
    profile_id TEXT NOT NULL REFERENCES profiles(profile_id),
    collection TEXT NOT NULL, capture_id TEXT NOT NULL, epoch TEXT NOT NULL,
    scope TEXT NOT NULL,
    freshness TEXT NOT NULL CHECK(freshness IN ('saved','awaiting_snapshot','synchronized','stale','error')),
    reason TEXT NOT NULL, observed_ms INTEGER NOT NULL, revision INTEGER NOT NULL,
    PRIMARY KEY(profile_id,collection),
    FOREIGN KEY(capture_id,epoch) REFERENCES epochs(capture_id,epoch)
);
INSERT INTO domain_state SELECT * FROM old_domain_state;
DROP TABLE old_domain_state;
CREATE TABLE live_checkpoints (
    capture_id TEXT PRIMARY KEY REFERENCES captures(capture_id),
    last_sequence INTEGER NOT NULL DEFAULT 0,
    current_epoch TEXT,
    closed INTEGER NOT NULL DEFAULT 0,
    last_contact_ms INTEGER NOT NULL
);
CREATE TABLE delivery_results (
    capture_id TEXT NOT NULL,
    source_sequence INTEGER NOT NULL,
    event_hash TEXT NOT NULL,
    result_json TEXT NOT NULL,
    PRIMARY KEY(capture_id,source_sequence),
    FOREIGN KEY(capture_id,source_sequence) REFERENCES events(capture_id,source_sequence)
);
CREATE TABLE live_leases (
    profile_id TEXT PRIMARY KEY REFERENCES profiles(profile_id),
    capture_id TEXT NOT NULL, epoch TEXT NOT NULL, expires_ms INTEGER NOT NULL,
    FOREIGN KEY(capture_id,epoch) REFERENCES epochs(capture_id,epoch)
);
CREATE TABLE active_context (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    profile_id TEXT REFERENCES profiles(profile_id), capture_id TEXT, epoch TEXT
);
INSERT INTO active_context(singleton) VALUES(1);
