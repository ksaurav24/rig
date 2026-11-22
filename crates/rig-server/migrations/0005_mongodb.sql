-- no-transaction
-- Adds MongoDB to the engines a managed database may use. SQLite cannot alter
-- a CHECK constraint, so the table is rebuilt. Foreign keys are switched off
-- for the rebuild (a pragma that has no effect inside a transaction), so
-- dropping the old table does not cascade into backups.
PRAGMA foreign_keys = OFF;
BEGIN;

CREATE TABLE databases_new (
  id           TEXT PRIMARY KEY,
  org_id       TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  project_id   TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  name         TEXT NOT NULL,
  engine       TEXT NOT NULL
               CHECK (engine IN ('postgres','mysql','mariadb','redis','mongodb')),
  version      TEXT NOT NULL,
  container_id TEXT,
  volume       TEXT NOT NULL,
  db_name      TEXT NOT NULL,
  username     TEXT NOT NULL,
  password_enc TEXT NOT NULL,
  status       TEXT NOT NULL DEFAULT 'creating'
               CHECK (status IN ('creating','running','stopped','failed')),
  error        TEXT,
  created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  slug         TEXT NOT NULL DEFAULT '',
  host_port    INTEGER,
  UNIQUE (project_id, name)
);

INSERT INTO databases_new
  (id, org_id, project_id, name, engine, version, container_id, volume, db_name,
   username, password_enc, status, error, created_at, slug, host_port)
SELECT id, org_id, project_id, name, engine, version, container_id, volume, db_name,
       username, password_enc, status, error, created_at, slug, host_port
  FROM databases;

DROP TABLE databases;
ALTER TABLE databases_new RENAME TO databases;
CREATE INDEX idx_databases_org ON databases(org_id);
CREATE UNIQUE INDEX databases_project_slug ON databases (project_id, slug);

COMMIT;
PRAGMA foreign_keys = ON;
