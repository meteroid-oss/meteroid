-- PostgreSQL cannot drop an enum value: 'MIGRATED' stays on "ScheduledEventStatus", unused. The
-- revert of 2026-11-15-195248 refuses first while a migrated event exists.
SELECT 1;
