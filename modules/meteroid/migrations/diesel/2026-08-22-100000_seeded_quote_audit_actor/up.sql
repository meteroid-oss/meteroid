-- The seeder called insert_quote_with_details with a nil created_by, so its
-- quotes were audited as Actor::User rather than System and passed the
-- sandbox gate. 240 of 252 quote.created rows are the seeder's.
UPDATE entity_activity
SET actor_type = 'SYSTEM', actor_uuid = NULL
WHERE activity_type = 'quote.created'
  AND actor_uuid = '00000000-0000-0000-0000-000000000000';

-- Milestone rows are write-once, so the ones derived from those quotes stay
-- until deleted. Dropping all of them lets the hourly sweep re-derive; orgs
-- with a real quote get theirs back with the right date, which the seeded
-- quote had been pre-dating.
DELETE FROM organization_milestone WHERE milestone = 'FIRST_QUOTE_CREATED';
