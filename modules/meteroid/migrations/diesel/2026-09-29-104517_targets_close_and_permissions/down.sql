UPDATE alert_definition
SET condition  = CASE
                     WHEN jsonb_typeof(condition #> '{threshold,value}') = 'number'
                         THEN jsonb_set(condition, '{threshold,value}',
                                        to_jsonb((condition #>> '{threshold,value}')::numeric / 100))
                     WHEN jsonb_typeof(condition #> '{forecast_breach,value}') = 'number'
                         THEN jsonb_set(condition, '{forecast_breach,value}',
                                        to_jsonb((condition #>> '{forecast_breach,value}')::numeric / 100))
                     WHEN jsonb_typeof(condition #> '{limit,limit}') = 'number'
                         THEN jsonb_set(condition, '{limit,limit}',
                                        to_jsonb((condition #>> '{limit,limit}')::numeric / 100))
                     ELSE condition END,
    updated_at = now()
WHERE metric_key IN ('nrr', 'grr', 'gross_mrr_churn_rate', 'logo_churn_rate')
  AND (jsonb_typeof(condition #> '{threshold,value}') = 'number'
    OR jsonb_typeof(condition #> '{forecast_breach,value}') = 'number'
    OR jsonb_typeof(condition #> '{limit,limit}') = 'number');

UPDATE role
SET permissions = array_remove(array_remove(permissions, 'targets:read'), 'targets:manage'),
    updated_at  = now()
WHERE 'targets:read' = ANY (permissions) OR 'targets:manage' = ANY (permissions);

ALTER TABLE metric_target
    DROP COLUMN finalized_at,
    DROP COLUMN final_value;
