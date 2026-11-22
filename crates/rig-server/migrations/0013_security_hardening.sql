-- Security hardening: temporary password lifecycle, 2FA challenge tokens,
-- TOTP enrollment expiry, session idle tracking, atomic deployment lock, and
-- database operation locking.

ALTER TABLE users ADD COLUMN must_change_password INTEGER NOT NULL DEFAULT 0;
ALTER TABLE users ADD COLUMN temp_password_expires_at TEXT;
ALTER TABLE users ADD COLUMN totp_pending_expires_at TEXT;

ALTER TABLE sessions ADD COLUMN last_active_at TEXT;

CREATE TABLE pending_2fa (
  token_hash TEXT PRIMARY KEY,
  user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  ip         TEXT,
  user_agent TEXT,
  expires_at TEXT NOT NULL
) WITHOUT ROWID;
CREATE INDEX idx_pending_2fa_expires ON pending_2fa(expires_at);

CREATE UNIQUE INDEX IF NOT EXISTS idx_deployments_active_app
  ON deployments(app_id)
  WHERE status IN ('queued', 'building', 'deploying');

ALTER TABLE databases ADD COLUMN busy_op TEXT;
