-- Device tokens and single-use device pairing codes for Rig Desktop & mobile.

ALTER TABLE api_tokens ADD COLUMN kind TEXT NOT NULL DEFAULT 'api';
ALTER TABLE api_tokens ADD COLUMN last_ip TEXT;

CREATE INDEX IF NOT EXISTS idx_api_tokens_user_kind
    ON api_tokens (org_id, user_id, kind, created_at DESC);

CREATE TABLE IF NOT EXISTS device_pairings (
    id          TEXT PRIMARY KEY,
    org_id      TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    code_hash   TEXT NOT NULL UNIQUE,
    role        TEXT NOT NULL,
    expires_at  TEXT NOT NULL,
    used_at     TEXT,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE INDEX IF NOT EXISTS idx_device_pairings_expires
    ON device_pairings (expires_at);
