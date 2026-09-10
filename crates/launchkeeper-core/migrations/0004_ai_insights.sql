-- M4 §1: one stored AI explanation per task.
--
-- `task_name` is both the primary key and the foreign key: a task has at most
-- one insight, re-explaining overwrites it (`INSERT ... ON CONFLICT DO
-- UPDATE`), and deleting the task takes it with it (`ON DELETE CASCADE`, with
-- `PRAGMA foreign_keys = ON`, which `Store::finish_open` sets).
--
-- `prompt_hash` is the sha256 of the exact prompt that produced `content`, so
-- a caller can tell whether a stored insight still describes the task as it
-- is now without having to store the prompt itself.
CREATE TABLE ai_insights (
  task_name TEXT PRIMARY KEY REFERENCES tasks(name) ON DELETE CASCADE,
  created_at TEXT NOT NULL,
  model TEXT NOT NULL,
  prompt_hash TEXT NOT NULL,
  content TEXT NOT NULL
);
