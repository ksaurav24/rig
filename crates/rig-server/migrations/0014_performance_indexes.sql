-- Performance indexes for metrics retention pruning, org/project listing,
-- keyset-ordered audit/deployment reads, and queued-deployment polling.

CREATE INDEX IF NOT EXISTS idx_metrics_ts ON metrics(ts);

CREATE INDEX IF NOT EXISTS idx_domains_org_host ON domains(org_id, host);
CREATE INDEX IF NOT EXISTS idx_domains_app_host ON domains(app_id, host);

DROP INDEX IF EXISTS idx_audit_org;
CREATE INDEX idx_audit_org ON audit_log(org_id, created_at DESC, id DESC);

DROP INDEX IF EXISTS idx_deployments_app;
CREATE INDEX idx_deployments_app ON deployments(app_id, created_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_deployments_queued
  ON deployments(created_at, id)
  WHERE status = 'queued';

CREATE INDEX IF NOT EXISTS idx_databases_project ON databases(project_id);
