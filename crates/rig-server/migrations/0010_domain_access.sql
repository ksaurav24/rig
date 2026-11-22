-- no-transaction
-- Domains gain an access mode (public, LAN, tailnet) and an optional
-- listener port of their own, and the tls_mode CHECK gains 'tailscale'.
-- A name can now repeat on different ports (a machine has one Tailscale
-- name), so uniqueness moves to (host, port). SQLite cannot alter a CHECK or
-- a UNIQUE constraint, so the table is rebuilt with foreign keys off.
PRAGMA foreign_keys = OFF;
BEGIN;

CREATE TABLE domains_new (
  id         TEXT PRIMARY KEY,
  org_id     TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  app_id     TEXT NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
  host       TEXT NOT NULL COLLATE NOCASE,
  tls_mode   TEXT NOT NULL DEFAULT 'auto'
             CHECK (tls_mode IN ('auto','internal','off','tailscale')),
  access     TEXT NOT NULL DEFAULT 'public' CHECK (access IN ('public','lan','tailscale')),
  port       INTEGER CHECK (port IS NULL OR port BETWEEN 1024 AND 65535),
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

INSERT INTO domains_new (id, org_id, app_id, host, tls_mode, created_at)
SELECT id, org_id, app_id, host, tls_mode, created_at FROM domains;

DROP TABLE domains;
ALTER TABLE domains_new RENAME TO domains;
CREATE INDEX idx_domains_app ON domains(app_id);
CREATE UNIQUE INDEX idx_domains_host_port ON domains(host COLLATE NOCASE, ifnull(port, 0));

COMMIT;
PRAGMA foreign_keys = ON;
