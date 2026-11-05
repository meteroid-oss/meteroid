-- A closed period's value, frozen once the books close: late entries may restate
-- the live series, never the number the board saw.
ALTER TABLE metric_target
    ADD COLUMN final_value DOUBLE PRECISION,
    ADD COLUMN finalized_at TIMESTAMP;

-- Targets get their own permissions, apart from alerts. Roles that could see alerts keep seeing
-- targets; managing the plan is granted anew.
UPDATE role
SET permissions = array_append(permissions, 'targets:read'),
    updated_at  = now()
WHERE ('alerts:read' = ANY (permissions) OR 'alerts:manage' = ANY (permissions))
  AND NOT 'targets:read' = ANY (permissions);

-- Retention and churn ratios now read in percent points (102.0, not 1.02), over a trailing window
-- of customers. A monitor on one keeps its levels in the series' unit. `numeric` keeps 0.07 at
-- 7.00 (float8 gives 7.000000000000001); a level that is not a number is left as it is.
UPDATE alert_definition
SET condition  = CASE
                     WHEN jsonb_typeof(condition #> '{threshold,value}') = 'number'
                         THEN jsonb_set(condition, '{threshold,value}',
                                        to_jsonb((condition #>> '{threshold,value}')::numeric * 100))
                     WHEN jsonb_typeof(condition #> '{forecast_breach,value}') = 'number'
                         THEN jsonb_set(condition, '{forecast_breach,value}',
                                        to_jsonb((condition #>> '{forecast_breach,value}')::numeric * 100))
                     WHEN jsonb_typeof(condition #> '{limit,limit}') = 'number'
                         THEN jsonb_set(condition, '{limit,limit}',
                                        to_jsonb((condition #>> '{limit,limit}')::numeric * 100))
                     ELSE condition END,
    updated_at = now()
WHERE metric_key IN ('nrr', 'grr', 'gross_mrr_churn_rate', 'logo_churn_rate')
  AND (jsonb_typeof(condition #> '{threshold,value}') = 'number'
    OR jsonb_typeof(condition #> '{forecast_breach,value}') = 'number'
    OR jsonb_typeof(condition #> '{limit,limit}') = 'number');
