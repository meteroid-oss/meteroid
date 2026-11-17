-- Postgres cannot drop an enum value in place: notes marked as corrections become goodwill again.
UPDATE credit_note SET origin = 'MANUAL' WHERE origin::text = 'CORRECTION';
