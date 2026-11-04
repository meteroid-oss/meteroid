-- The last day an installation's usage report was ingested for, signed into its license.
ALTER TABLE issued_license ADD COLUMN stats_reported_through DATE;
