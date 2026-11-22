-- Persistent storage for apps: one named engine volume per mount path.
CREATE TABLE app_volumes (
  id          TEXT PRIMARY KEY,
  app_id      TEXT NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
  name        TEXT NOT NULL UNIQUE,
  mount_path  TEXT NOT NULL,
  created_at  TEXT NOT NULL,
  UNIQUE (app_id, mount_path)
);
