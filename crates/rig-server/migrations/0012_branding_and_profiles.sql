-- Organization branding, user avatars/profiles, and server-backed media blobs.
CREATE TABLE media (
  id           TEXT PRIMARY KEY,
  org_id       TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  content_type TEXT NOT NULL,
  data         BLOB NOT NULL,
  is_public    INTEGER NOT NULL DEFAULT 0,
  created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX idx_media_org ON media(org_id);

ALTER TABLE organizations ADD COLUMN logo_url TEXT;
ALTER TABLE organizations ADD COLUMN accent_color TEXT NOT NULL DEFAULT 'emerald';
ALTER TABLE organizations ADD COLUMN env_label TEXT;
ALTER TABLE organizations ADD COLUMN env_color TEXT;

ALTER TABLE users ADD COLUMN avatar_url TEXT;
