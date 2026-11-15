CREATE TABLE wish_records (
 profile_id TEXT NOT NULL REFERENCES profiles(profile_id),
 wish_id TEXT NOT NULL,
 record_json TEXT NOT NULL,
 PRIMARY KEY(profile_id,wish_id)
);
CREATE TABLE wish_imports (
 import_id TEXT PRIMARY KEY,
 profile_id TEXT NOT NULL REFERENCES profiles(profile_id),
 sha256 TEXT NOT NULL,
 fetched_ms INTEGER NOT NULL,
 imported_ms INTEGER NOT NULL,
 metadata_json TEXT NOT NULL,
 record_count INTEGER NOT NULL,
 UNIQUE(profile_id,sha256)
);
