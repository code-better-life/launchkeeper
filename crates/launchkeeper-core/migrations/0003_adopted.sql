-- Launchkeeper schema v3 (M3 §3): tasks adopted in place from a plist
-- somebody else wrote.
--
-- adopted    — 1 when the task's launchd label is `name` verbatim (the label
--              the original plist already carried) instead of
--              `com.launchkeeper.<name>`.
-- plist_path — where that plist lives. NULL for a task Launchkeeper created
--              itself, which always uses <launch_agents_dir>/<label>.plist.

ALTER TABLE tasks ADD COLUMN adopted INTEGER NOT NULL DEFAULT 0;
ALTER TABLE tasks ADD COLUMN plist_path TEXT;
