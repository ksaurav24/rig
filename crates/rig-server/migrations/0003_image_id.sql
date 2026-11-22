-- The engine's content ID of the image a deployment ran. A rollback runs
-- exactly this image, even if the tag it came from has moved since.
ALTER TABLE deployments ADD COLUMN image_id TEXT;
