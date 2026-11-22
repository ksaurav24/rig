-- no-transaction
-- GitHub integration: one GitHub App per organization (created through
-- GitHub's manifest flow), push deliveries remembered for de-duplication,
-- a per-app "deploy on push" switch, and two new deployment triggers
-- ('github' for App webhooks, 'poll' for the branch poller). SQLite cannot
-- alter a CHECK, so deployments is rebuilt with foreign keys off.
PRAGMA foreign_keys = OFF;
BEGIN;

CREATE TABLE deployments_new (
  id           TEXT PRIMARY KEY,
  org_id       TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  app_id       TEXT NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
  status       TEXT NOT NULL DEFAULT 'queued'
               CHECK (status IN ('queued','building','deploying','running','superseded','failed','cancelled')),
  trigger      TEXT NOT NULL DEFAULT 'manual'
               CHECK (trigger IN ('manual','webhook','rollback','api','github','poll')),
  image_ref    TEXT,
  commit_sha   TEXT,
  commit_msg   TEXT,
  container_id TEXT,
  host_port    INTEGER,
  error        TEXT,
  created_by   TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  started_at   TEXT,
  finished_at  TEXT,
  image_id     TEXT
);

INSERT INTO deployments_new (id, org_id, app_id, status, trigger, image_ref, commit_sha,
  commit_msg, container_id, host_port, error, created_by, created_at, started_at,
  finished_at, image_id)
SELECT id, org_id, app_id, status, trigger, image_ref, commit_sha, commit_msg,
  container_id, host_port, error, created_by, created_at, started_at, finished_at, image_id
FROM deployments;

DROP TABLE deployments;
ALTER TABLE deployments_new RENAME TO deployments;
CREATE INDEX idx_deployments_app ON deployments(app_id, created_at DESC);
CREATE INDEX idx_deployments_status ON deployments(status);

-- Secrets (client secret, webhook secret, private key) are vault-encrypted.
CREATE TABLE github_apps (
  org_id         TEXT PRIMARY KEY REFERENCES organizations(id) ON DELETE CASCADE,
  app_id         INTEGER NOT NULL,
  slug           TEXT NOT NULL,
  name           TEXT NOT NULL,
  html_url       TEXT NOT NULL,
  owner          TEXT NOT NULL,
  client_id      TEXT NOT NULL,
  client_secret  TEXT NOT NULL,
  webhook_secret TEXT NOT NULL,
  private_key    TEXT NOT NULL,
  created_by     TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

-- X-GitHub-Delivery ids already handled, so a redelivery is not deployed twice.
CREATE TABLE github_deliveries (
  id          TEXT PRIMARY KEY,
  received_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
) WITHOUT ROWID;

ALTER TABLE apps ADD COLUMN auto_deploy INTEGER NOT NULL DEFAULT 0;

COMMIT;
PRAGMA foreign_keys = ON;
