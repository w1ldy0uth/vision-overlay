CREATE TABLE planner_goals (
 profile_id TEXT NOT NULL REFERENCES profiles(profile_id),
 goal_id TEXT NOT NULL,
 goal_json TEXT NOT NULL,
 revision INTEGER NOT NULL,
 PRIMARY KEY(profile_id,goal_id)
);
