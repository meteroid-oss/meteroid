-- Stamped when an operator edits a draft's billing content (line items or discount) through the
-- console/API. A manual finalize of such a draft keeps the stored lines instead of settling with a
-- refresh: the operator approved exactly what they previewed, and a recompute would regenerate
-- usage lines and drop the manual discount from the total while the discount column keeps it.
ALTER TABLE invoice ADD COLUMN manually_edited_at TIMESTAMP;
