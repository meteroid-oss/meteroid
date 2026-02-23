ALTER TABLE plan ADD COLUMN self_service_rank INTEGER;
UPDATE plan SET self_service_rank = 1 WHERE plan_type = 'STANDARD';
