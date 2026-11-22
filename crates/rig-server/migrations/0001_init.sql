-- Rig core schema. Every tenant-owned row carries org_id; ids are UUIDv7 text.
PRAGMA foreign_keys = ON;

CREATE TABLE settings (
  key        TEXT PRIMARY KEY,
  value      TEXT NOT NULL,
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
) WITHOUT ROWID;

CREATE TABLE organizations (
  id         TEXT PRIMARY KEY,
  name       TEXT NOT NULL,
  slug       TEXT NOT NULL UNIQUE,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE TABLE users (
  id            TEXT PRIMARY KEY,
  email         TEXT NOT NULL UNIQUE COLLATE NOCASE,
  name          TEXT NOT NULL,
  password_hash TEXT NOT NULL,
  disabled      INTEGER NOT NULL DEFAULT 0,
  created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  last_login_at TEXT
);

CREATE TABLE memberships (
  org_id     TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  role       TEXT NOT NULL CHECK (role IN ('owner','admin','developer','viewer')),
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  PRIMARY KEY (org_id, user_id)
) WITHOUT ROWID;
CREATE INDEX idx_memberships_user ON memberships(user_id);

-- Browser sessions. Only the SHA-256 of the token is stored.
CREATE TABLE sessions (
  token_hash TEXT PRIMARY KEY,
  user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  org_id     TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  csrf_token TEXT NOT NULL,
  user_agent TEXT,
  ip         TEXT,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  expires_at TEXT NOT NULL
) WITHOUT ROWID;
CREATE INDEX idx_sessions_user ON sessions(user_id);
CREATE INDEX idx_sessions_expires ON sessions(expires_at);

-- API tokens for the CLI and MCP. Only the hash is stored; prefix is for display.
CREATE TABLE api_tokens (
  id           TEXT PRIMARY KEY,
  org_id       TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  user_id      TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name         TEXT NOT NULL,
  token_hash   TEXT NOT NULL UNIQUE,
  prefix       TEXT NOT NULL,
  role         TEXT NOT NULL CHECK (role IN ('owner','admin','developer','viewer')),
  last_used_at TEXT,
  expires_at   TEXT,
  created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX idx_api_tokens_org ON api_tokens(org_id);

CREATE TABLE servers (
  id         TEXT PRIMARY KEY,
  org_id     TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  name       TEXT NOT NULL,
  kind       TEXT NOT NULL DEFAULT 'local' CHECK (kind IN ('local','remote')),
  engine     TEXT NOT NULL DEFAULT 'docker' CHECK (engine IN ('docker','podman')),
  socket     TEXT,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  UNIQUE (org_id, name)
);

CREATE TABLE projects (
  id          TEXT PRIMARY KEY,
  org_id      TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  name        TEXT NOT NULL,
  slug        TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  UNIQUE (org_id, slug)
);

-- environment is reserved for staging/production later; MVP uses 'production'.
CREATE TABLE apps (
  id              TEXT PRIMARY KEY,
  org_id          TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  server_id       TEXT REFERENCES servers(id) ON DELETE SET NULL,
  name            TEXT NOT NULL,
  slug            TEXT NOT NULL,
  environment     TEXT NOT NULL DEFAULT 'production',
  source_kind     TEXT NOT NULL CHECK (source_kind IN ('image','git','compose')),
  image           TEXT,
  git_url         TEXT,
  git_branch      TEXT,
  dockerfile_path TEXT NOT NULL DEFAULT 'Dockerfile',
  build_context   TEXT NOT NULL DEFAULT '.',
  compose_yaml    TEXT,
  port            INTEGER NOT NULL DEFAULT 80,
  replicas        INTEGER NOT NULL DEFAULT 1,
  memory_mb       INTEGER,
  cpu_millis      INTEGER,
  health_path     TEXT,
  restart_policy  TEXT NOT NULL DEFAULT 'unless-stopped',
  status          TEXT NOT NULL DEFAULT 'idle'
                  CHECK (status IN ('idle','deploying','running','stopped','failed')),
  current_deployment_id TEXT,
  webhook_secret  TEXT NOT NULL,
  created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  UNIQUE (project_id, slug)
);
CREATE INDEX idx_apps_org ON apps(org_id);
CREATE INDEX idx_apps_project ON apps(project_id);

CREATE TABLE env_vars (
  id         TEXT PRIMARY KEY,
  org_id     TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  app_id     TEXT NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
  key        TEXT NOT NULL,
  -- AEAD ciphertext (nonce || ct), base64. Never returned unmasked for secrets.
  value_enc  TEXT NOT NULL,
  is_secret  INTEGER NOT NULL DEFAULT 1,
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  UNIQUE (app_id, key)
);

CREATE TABLE domains (
  id         TEXT PRIMARY KEY,
  org_id     TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  app_id     TEXT NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
  host       TEXT NOT NULL UNIQUE COLLATE NOCASE,
  tls_mode   TEXT NOT NULL DEFAULT 'auto' CHECK (tls_mode IN ('auto','internal','off')),
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX idx_domains_app ON domains(app_id);

CREATE TABLE deployments (
  id           TEXT PRIMARY KEY,
  org_id       TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  app_id       TEXT NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
  status       TEXT NOT NULL DEFAULT 'queued'
               CHECK (status IN ('queued','building','deploying','running','superseded','failed','cancelled')),
  trigger      TEXT NOT NULL DEFAULT 'manual'
               CHECK (trigger IN ('manual','webhook','rollback','api')),
  image_ref    TEXT,
  commit_sha   TEXT,
  commit_msg   TEXT,
  container_id TEXT,
  host_port    INTEGER,
  error        TEXT,
  created_by   TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  started_at   TEXT,
  finished_at  TEXT
);
CREATE INDEX idx_deployments_app ON deployments(app_id, created_at DESC);
CREATE INDEX idx_deployments_status ON deployments(status);

-- Build and deploy log lines. seq is monotonic per deployment for SSE resume.
CREATE TABLE deployment_logs (
  deployment_id TEXT NOT NULL REFERENCES deployments(id) ON DELETE CASCADE,
  seq           INTEGER NOT NULL,
  ts            TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  stream        TEXT NOT NULL DEFAULT 'build',
  line          TEXT NOT NULL,
  PRIMARY KEY (deployment_id, seq)
) WITHOUT ROWID;

CREATE TABLE databases (
  id           TEXT PRIMARY KEY,
  org_id       TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  project_id   TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  name         TEXT NOT NULL,
  engine       TEXT NOT NULL CHECK (engine IN ('postgres','mysql','mariadb','redis')),
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
  UNIQUE (project_id, name)
);
CREATE INDEX idx_databases_org ON databases(org_id);

CREATE TABLE backups (
  id          TEXT PRIMARY KEY,
  org_id      TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  database_id TEXT NOT NULL REFERENCES databases(id) ON DELETE CASCADE,
  status      TEXT NOT NULL DEFAULT 'running' CHECK (status IN ('running','ok','failed')),
  path        TEXT,
  size_bytes  INTEGER,
  error       TEXT,
  created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  finished_at TEXT
);
CREATE INDEX idx_backups_db ON backups(database_id, created_at DESC);

-- Coarse host/app metric samples, pruned on a retention window.
CREATE TABLE metrics (
  ts        INTEGER NOT NULL,
  scope     TEXT NOT NULL,
  cpu_pct   REAL NOT NULL,
  mem_bytes INTEGER NOT NULL,
  mem_total INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (scope, ts)
) WITHOUT ROWID;

CREATE TABLE audit_log (
  id         TEXT PRIMARY KEY,
  org_id     TEXT REFERENCES organizations(id) ON DELETE CASCADE,
  user_id    TEXT REFERENCES users(id) ON DELETE SET NULL,
  actor      TEXT NOT NULL,
  action     TEXT NOT NULL,
  target     TEXT,
  detail     TEXT,
  ip         TEXT,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX idx_audit_org ON audit_log(org_id, created_at DESC);

-- Failed-login throttle, keyed by ip and by email.
CREATE TABLE login_attempts (
  key        TEXT NOT NULL,
  ts         INTEGER NOT NULL
);
CREATE INDEX idx_login_attempts ON login_attempts(key, ts);
