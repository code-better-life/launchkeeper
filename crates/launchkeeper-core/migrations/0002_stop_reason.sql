-- Launchkeeper schema v2 (M2.5): why a run ended.
--
-- 'exited'  — the script finished on its own (the only thing v1 could record)
-- 'stopped' — the runner was asked to stop (SIGTERM/SIGINT) and terminated it
-- 'timeout' — the run hit its per-task timeout
--
-- NULL on rows written by a v1 runner, and on rows whose run has not finished
-- yet.

ALTER TABLE runs ADD COLUMN stop_reason TEXT;
