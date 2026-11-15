CREATE TABLE recovery_points (
    point_id TEXT PRIMARY KEY,
    profile_id TEXT NOT NULL REFERENCES profiles(profile_id),
    capture_id TEXT NOT NULL REFERENCES captures(capture_id),
    epoch TEXT NOT NULL,
    observed_ms INTEGER NOT NULL,
    end_sequence INTEGER NOT NULL,
    state_json TEXT NOT NULL,
    UNIQUE(capture_id,epoch,profile_id)
);
CREATE TABLE retention_epochs (
    capture_id TEXT NOT NULL REFERENCES captures(capture_id),
    epoch TEXT NOT NULL,
    result TEXT NOT NULL,
    PRIMARY KEY(capture_id,epoch)
);
CREATE TABLE retention_artifacts (
    artifact_id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    relative_path TEXT NOT NULL UNIQUE,
    capture_id TEXT,
    pinned INTEGER NOT NULL,
    state TEXT NOT NULL DEFAULT 'present'
);
CREATE TABLE automatic_backups (
    backup_id TEXT PRIMARY KEY,
    relative_path TEXT NOT NULL UNIQUE,
    sha256 TEXT NOT NULL,
    meaningful_hash TEXT NOT NULL,
    created_ms INTEGER NOT NULL,
    state TEXT NOT NULL DEFAULT 'verified'
);
CREATE TABLE automatic_backup_captures (
    capture_id TEXT PRIMARY KEY REFERENCES captures(capture_id),
    backup_id TEXT,
    result TEXT NOT NULL
);
CREATE TABLE retention_jobs (
    job_id TEXT PRIMARY KEY,
    state TEXT NOT NULL,
    manifest_json TEXT NOT NULL
);
CREATE TABLE retention_meta (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    initialized INTEGER NOT NULL
);
