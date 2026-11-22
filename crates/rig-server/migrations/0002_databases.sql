-- Managed databases are reached by a per-project DNS name (`slug`) and, while
-- running, on a loopback port chosen by the engine.
ALTER TABLE databases ADD COLUMN slug TEXT NOT NULL DEFAULT '';
ALTER TABLE databases ADD COLUMN host_port INTEGER;
CREATE UNIQUE INDEX databases_project_slug ON databases (project_id, slug);
