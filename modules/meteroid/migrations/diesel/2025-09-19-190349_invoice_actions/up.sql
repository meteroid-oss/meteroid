ALTER TABLE invoice ADD COLUMN IF NOT EXISTS voided_at timestamp(3);
ALTER TABLE invoice ADD COLUMN IF NOT EXISTS marked_as_uncollectible_at timestamp(3);
