-- Per-user preferences bag. An opaque JSON object owned by the frontend,
-- holding low-stakes UI state (e.g. the changelog "seen" watermark, table
-- column ordering). Updates shallow-merge via `||` so independent features
-- never clobber each other's slice.
ALTER TABLE "user"
    ADD COLUMN preferences JSONB NOT NULL DEFAULT '{}'::jsonb;
