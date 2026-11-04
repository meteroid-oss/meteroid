-- The cycle the invoice's lines were computed for. A refresh must gate components on this cycle
-- rather than the subscription's current one: `applies_this_period` keys off cycle_index, so a
-- draft recomputed after a renewal would otherwise be gated against a cycle it does not belong to.
-- Null on rows predating this column; those fall back to the subscription's current cycle_index.
ALTER TABLE invoice ADD COLUMN cycle_index INTEGER;
