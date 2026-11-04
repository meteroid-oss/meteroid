-- The instant a subscription terminated (immediate cancel captures the actual UTC time; a
-- date-effective cancel or natural completion stores that date at midnight). The terminal
-- period's usage is billed up to this instant, so usage consumed on the termination day is
-- not dropped by the date-granular, midnight-exclusive period window. Null while active.
ALTER TABLE subscription ADD COLUMN terminated_at TIMESTAMP;
