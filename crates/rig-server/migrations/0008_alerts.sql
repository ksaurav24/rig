-- Outbound alert channels: a URL Rig POSTs a JSON event to.
-- The URL often embeds a token (Slack, Discord), so it is stored encrypted.
CREATE TABLE alert_channels (
  id             TEXT PRIMARY KEY,
  org_id         TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  name           TEXT NOT NULL,
  url_enc        TEXT NOT NULL,
  url_host       TEXT NOT NULL,
  signing_secret TEXT NOT NULL,
  events         TEXT NOT NULL,
  enabled        INTEGER NOT NULL DEFAULT 1,
  last_sent_at   TEXT,
  last_error     TEXT,
  created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX idx_alert_channels_org ON alert_channels(org_id);
