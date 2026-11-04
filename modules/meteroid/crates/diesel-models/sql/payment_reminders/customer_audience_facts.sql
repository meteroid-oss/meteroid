-- Customer-level audience facts: $1 = tenant id, $2 = customer ids (uuid[]); same `f` projection
-- contract as audience_facts.sql but only the columns customer-level leaves read.
WITH f AS (
  SELECT c.id AS customer_id,
         COALESCE(c.billing_address ->> 'country', '') AS country,
         c.custom_properties AS props
  FROM customer c
  WHERE c.tenant_id = $1 AND c.id = ANY($2::uuid[])
)
