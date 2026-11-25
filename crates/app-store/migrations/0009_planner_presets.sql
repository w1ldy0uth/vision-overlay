CREATE TABLE planner_presets (
 profile_id TEXT NOT NULL REFERENCES profiles(profile_id),
 preset_id TEXT NOT NULL,
 name TEXT NOT NULL,
 PRIMARY KEY(profile_id,preset_id),
 UNIQUE(profile_id,name COLLATE NOCASE)
);
CREATE TABLE planner_preset_goals (
 profile_id TEXT NOT NULL,
 goal_id TEXT NOT NULL,
 preset_id TEXT NOT NULL,
 PRIMARY KEY(profile_id,goal_id),
 FOREIGN KEY(profile_id,goal_id) REFERENCES planner_goals(profile_id,goal_id) ON DELETE CASCADE,
 FOREIGN KEY(profile_id,preset_id) REFERENCES planner_presets(profile_id,preset_id)
);
