-- A price's rates change only from a date, as a successor in its lineage (R-097): the row a
-- component, add-on or subscription references is the lineage's root, never rewritten; each
-- successor names it and the day its rates take over.
SET LOCAL lock_timeout = '5s';

ALTER TABLE price
    ADD COLUMN effective_from DATE,
    ADD COLUMN supersedes     UUID REFERENCES price (id) ON DELETE CASCADE,
    ADD CONSTRAINT price_successor_is_dated CHECK ((supersedes IS NULL) = (effective_from IS NULL));

CREATE INDEX price_supersedes_idx ON price (supersedes) WHERE supersedes IS NOT NULL;
