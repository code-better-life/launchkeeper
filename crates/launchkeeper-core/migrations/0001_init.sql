-- Launchkeeper schema v1 (PRD §6.3, plus runs.pid).

CREATE TABLE tasks (
  name         TEXT PRIMARY KEY,
  display_name TEXT NOT NULL,
  description  TEXT,
  script_path  TEXT NOT NULL,
  args         TEXT NOT NULL DEFAULT '[]',   -- JSON array
  working_dir  TEXT,
  env          TEXT NOT NULL DEFAULT '{}',   -- JSON object
  "trigger"    TEXT NOT NULL,                -- JSON, tagged enum
  keep_alive   INTEGER NOT NULL DEFAULT 0,
  timeout_secs INTEGER,                      -- NULL = 不限时
  tags         TEXT NOT NULL DEFAULT '[]',
  favorite     INTEGER NOT NULL DEFAULT 0,
  notify_on_fail INTEGER NOT NULL DEFAULT 1,
  created_at   TEXT NOT NULL,
  updated_at   TEXT NOT NULL
);

CREATE TABLE runs (
  id          INTEGER PRIMARY KEY,
  task_name   TEXT NOT NULL REFERENCES tasks(name) ON DELETE CASCADE,
  started_at  TEXT NOT NULL,
  finished_at TEXT,
  exit_code   INTEGER,
  pid         INTEGER,
  stdout_path TEXT,
  stderr_path TEXT,
  trigger_kind TEXT NOT NULL                 -- 'scheduled' | 'manual'
);

CREATE INDEX runs_task_started ON runs(task_name, started_at DESC);
