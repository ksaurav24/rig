-- no-transaction
-- Support local folder/binary deploys (`rig up`) and container adoption (`rig adopt`):
--   * `apps.source_kind` allows `'local'` alongside `'image'`, `'git'`, `'compose'`
--   * `apps` gains optional `command`, `entrypoint`, `extra_networks`, `adopted_from`
--   * `deployments.trigger` allows `'local'` and `'adopt'`, plus `source_dir` for staged remote builds
--   * `app_volumes` supports both managed named volumes (`kind='volume'`, `owned=1`) and
--     preserved host bind mounts / pre-existing external volumes (`kind='bind'|'volume'`, `owned=0`, `read_only`)
PRAGMA foreign_keys = OFF;
BEGIN;

CREATE TABLE apps_new (
  id                    TEXT PRIMARY KEY,
  org_id                TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  project_id            TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  server_id             TEXT REFERENCES servers(id) ON DELETE SET NULL,
  name                  TEXT NOT NULL,
  slug                  TEXT NOT NULL,
  environment           TEXT NOT NULL DEFAULT 'production',
  source_kind           TEXT NOT NULL CHECK (source_kind IN ('image','git','compose','local')),
  image                 TEXT,
  git_url               TEXT,
  git_branch            TEXT,
  dockerfile_path       TEXT NOT NULL DEFAULT 'Dockerfile',
  build_context         TEXT NOT NULL DEFAULT '.',
  compose_yaml          TEXT,
  port                  INTEGER NOT NULL DEFAULT 80,
  replicas              INTEGER NOT NULL DEFAULT 1,
  memory_mb             INTEGER,
  cpu_millis            INTEGER,
  health_path           TEXT,
  restart_policy        TEXT NOT NULL DEFAULT 'unless-stopped',
  status                TEXT NOT NULL DEFAULT 'idle'
                        CHECK (status IN ('idle','deploying','running','stopped','failed')),
  current_deployment_id TEXT,
  webhook_secret        TEXT NOT NULL,
  created_at            TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at            TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  restart_count         INTEGER NOT NULL DEFAULT 0,
  last_crash_at         TEXT,
  last_crash_reason     TEXT,
  auto_deploy           INTEGER NOT NULL DEFAULT 0,
  command               TEXT,
  entrypoint            TEXT,
  extra_networks        TEXT,
  adopted_from          TEXT,
  UNIQUE (project_id, slug)
);

INSERT INTO apps_new (
  id, org_id, project_id, server_id, name, slug, environment, source_kind, image, git_url,
  git_branch, dockerfile_path, build_context, compose_yaml, port, replicas, memory_mb, cpu_millis,
  health_path, restart_policy, status, current_deployment_id, webhook_secret, created_at,
  updated_at, restart_count, last_crash_at, last_crash_reason, auto_deploy
)
SELECT
  id, org_id, project_id, server_id, name, slug, environment, source_kind, image, git_url,
  git_branch, dockerfile_path, build_context, compose_yaml, port, replicas, memory_mb, cpu_millis,
  health_path, restart_policy, status, current_deployment_id, webhook_secret, created_at,
  updated_at, restart_count, last_crash_at, last_crash_reason, auto_deploy
FROM apps;

DROP TABLE apps;
ALTER TABLE apps_new RENAME TO apps;
CREATE INDEX idx_apps_org ON apps(org_id);
CREATE INDEX idx_apps_project ON apps(project_id);

CREATE TABLE deployments_new (
  id           TEXT PRIMARY KEY,
  org_id       TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  app_id       TEXT NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
  status       TEXT NOT NULL DEFAULT 'queued'
               CHECK (status IN ('queued','building','deploying','running','superseded','failed','cancelled')),
  trigger      TEXT NOT NULL DEFAULT 'manual'
               CHECK (trigger IN ('manual','webhook','rollback','api','github','poll','local','adopt')),
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
  image_id     TEXT,
  source_dir   TEXT
);

INSERT INTO deployments_new (
  id, org_id, app_id, status, trigger, image_ref, commit_sha, commit_msg, container_id,
  host_port, error, created_by, created_at, started_at, finished_at, image_id
)
SELECT
  id, org_id, app_id, status, trigger, image_ref, commit_sha, commit_msg, container_id,
  host_port, error, created_by, created_at, started_at, finished_at, image_id
FROM deployments;

DROP TABLE deployments;
ALTER TABLE deployments_new RENAME TO deployments;
CREATE INDEX idx_deployments_app ON deployments(app_id, created_at DESC, id DESC);
CREATE INDEX idx_deployments_status ON deployments(status);
CREATE UNIQUE INDEX idx_deployments_active_app
  ON deployments(app_id)
  WHERE status IN ('queued', 'building', 'deploying');
CREATE INDEX idx_deployments_queued
  ON deployments(created_at, id)
  WHERE status = 'queued';

CREATE TABLE app_volumes_new (
  id          TEXT PRIMARY KEY,
  app_id      TEXT NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
  name        TEXT NOT NULL,
  mount_path  TEXT NOT NULL,
  kind        TEXT NOT NULL DEFAULT 'volume' CHECK (kind IN ('volume','bind')),
  read_only   INTEGER NOT NULL DEFAULT 0,
  owned       INTEGER NOT NULL DEFAULT 1,
  created_at  TEXT NOT NULL,
  UNIQUE (app_id, mount_path)
);

INSERT INTO app_volumes_new (id, app_id, name, mount_path, kind, read_only, owned, created_at)
SELECT id, app_id, name, mount_path, 'volume', 0, 1, created_at
FROM app_volumes;

DROP TABLE app_volumes;
ALTER TABLE app_volumes_new RENAME TO app_volumes;
CREATE INDEX idx_app_volumes_app ON app_volumes(app_id);

COMMIT;
PRAGMA foreign_keys = ON;
