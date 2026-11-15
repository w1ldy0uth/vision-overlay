CREATE TABLE profiles (
    profile_id TEXT PRIMARY KEY,
    game_id TEXT NOT NULL CHECK(game_id='genshin'),
    account_namespace TEXT NOT NULL,
    server_region TEXT NOT NULL CHECK(server_region IN ('os_euro','os_asia','os_usa','os_cht')),
    uid TEXT NOT NULL,
    UNIQUE(game_id, account_namespace, server_region, uid)
);
CREATE TABLE captures (
    capture_id TEXT PRIMARY KEY,
    artifact_sha256 TEXT NOT NULL UNIQUE,
    binding_json TEXT NOT NULL,
    source_json TEXT NOT NULL,
    adapter_revision TEXT NOT NULL,
    event_count INTEGER NOT NULL,
    terminal_status TEXT NOT NULL,
    imported_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE TABLE epochs (
    capture_id TEXT NOT NULL REFERENCES captures(capture_id),
    epoch TEXT NOT NULL,
    profile_id TEXT REFERENCES profiles(profile_id),
    evidence TEXT NOT NULL,
    PRIMARY KEY(capture_id,epoch)
);
CREATE TABLE events (
    capture_id TEXT NOT NULL,
    source_sequence INTEGER NOT NULL,
    epoch TEXT NOT NULL,
    profile_id TEXT REFERENCES profiles(profile_id),
    kind TEXT NOT NULL,
    observed_ms INTEGER,
    event_json TEXT NOT NULL,
    projection_result TEXT NOT NULL,
    revision INTEGER,
    PRIMARY KEY(capture_id,source_sequence),
    FOREIGN KEY(capture_id,epoch) REFERENCES epochs(capture_id,epoch)
);
CREATE TABLE snapshots (
    snapshot_id TEXT PRIMARY KEY,
    capture_id TEXT NOT NULL,
    source_sequence INTEGER NOT NULL,
    profile_id TEXT NOT NULL REFERENCES profiles(profile_id),
    collection TEXT NOT NULL,
    scope TEXT NOT NULL CHECK(scope IN ('full','partial','unverified')),
    snapshot_json TEXT NOT NULL,
    FOREIGN KEY(capture_id,source_sequence) REFERENCES events(capture_id,source_sequence),
    UNIQUE(capture_id,source_sequence,collection)
);
CREATE TABLE current_entities (
    profile_id TEXT NOT NULL REFERENCES profiles(profile_id),
    collection TEXT NOT NULL,
    entity_key TEXT NOT NULL,
    value_json TEXT NOT NULL,
    confirmed_in_baseline INTEGER NOT NULL CHECK(confirmed_in_baseline IN (0,1)),
    capture_id TEXT NOT NULL,
    epoch TEXT NOT NULL,
    source_sequence INTEGER NOT NULL,
    observed_ms INTEGER,
    revision INTEGER NOT NULL,
    PRIMARY KEY(profile_id,collection,entity_key),
    FOREIGN KEY(capture_id,source_sequence) REFERENCES events(capture_id,source_sequence)
);
CREATE TABLE domain_state (
    profile_id TEXT NOT NULL REFERENCES profiles(profile_id),
    collection TEXT NOT NULL,
    capture_id TEXT NOT NULL,
    epoch TEXT NOT NULL,
    scope TEXT NOT NULL,
    freshness TEXT NOT NULL CHECK(freshness IN ('saved','stale','error')),
    reason TEXT NOT NULL,
    observed_ms INTEGER NOT NULL,
    revision INTEGER NOT NULL,
    PRIMARY KEY(profile_id,collection),
    FOREIGN KEY(capture_id,epoch) REFERENCES epochs(capture_id,epoch)
);
CREATE TABLE store_meta (singleton INTEGER PRIMARY KEY CHECK(singleton=1), revision INTEGER NOT NULL);
INSERT INTO store_meta VALUES(1,0);
CREATE TABLE change_log (
    revision INTEGER PRIMARY KEY,
    capture_id TEXT NOT NULL,
    source_sequence INTEGER NOT NULL,
    profile_id TEXT REFERENCES profiles(profile_id),
    topics_json TEXT NOT NULL,
    FOREIGN KEY(capture_id,source_sequence) REFERENCES events(capture_id,source_sequence)
);
CREATE INDEX events_profile_history ON events(profile_id,capture_id,source_sequence);
