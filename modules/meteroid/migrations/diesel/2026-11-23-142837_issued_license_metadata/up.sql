ALTER TABLE issued_license ADD COLUMN metadata JSONB NOT NULL DEFAULT '{}'::jsonb;
