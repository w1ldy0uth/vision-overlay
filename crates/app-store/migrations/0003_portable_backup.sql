CREATE TABLE portable_preferences (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    settings_json TEXT NOT NULL
);
CREATE TABLE restored_capture_guards (
    capture_id TEXT PRIMARY KEY
);
CREATE TABLE restore_barrier (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    restored_unix_ms INTEGER NOT NULL
);
