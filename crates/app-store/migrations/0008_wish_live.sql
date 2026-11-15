CREATE TABLE wish_live (
 profile_id TEXT NOT NULL REFERENCES profiles(profile_id),
 live_id TEXT PRIMARY KEY,
 capture_id TEXT NOT NULL,
 epoch TEXT NOT NULL,
 source_sequence INTEGER NOT NULL,
 result_index INTEGER NOT NULL,
 observed_ms INTEGER NOT NULL,
 gacha_type TEXT NOT NULL,
 gacha_id TEXT NOT NULL,
 item_id INTEGER NOT NULL,
 anchor_id TEXT NOT NULL,
 eligible INTEGER NOT NULL CHECK(eligible IN (0,1)),
 api_wish_id TEXT,
 UNIQUE(capture_id,source_sequence,result_index),
 FOREIGN KEY(capture_id,source_sequence) REFERENCES events(capture_id,source_sequence)
);
CREATE UNIQUE INDEX wish_live_api_match ON wish_live(profile_id,api_wish_id) WHERE api_wish_id IS NOT NULL;
