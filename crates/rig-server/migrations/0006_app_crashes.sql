-- What the crash watcher last saw for an app's live container: the engine's
-- restart count (to notice restarts between two looks) and the latest crash.
ALTER TABLE apps ADD COLUMN restart_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE apps ADD COLUMN last_crash_at TEXT;
ALTER TABLE apps ADD COLUMN last_crash_reason TEXT;
