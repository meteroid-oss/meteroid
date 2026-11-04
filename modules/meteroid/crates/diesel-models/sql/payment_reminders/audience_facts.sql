-- Audience facts for a batch of invoices: $1 = tenant id, $2 = invoice ids (uuid[]); the caller
-- appends `SELECT ... FROM f WHERE <compiled predicate>` with binds from $3. A consolidated parent's
-- plan arrays are the union over its members; payment_method_type is for targeting only.
WITH member AS (
  SELECT i.id AS invoice_id, m.subscription_id, m.plan_version_id
  FROM invoice i JOIN invoice m ON m.id = i.id OR m.consolidated_into_invoice_id = i.id
  WHERE i.tenant_id = $1 AND i.id = ANY($2::uuid[])
), f AS (
  SELECT i.id AS invoice_id, i.customer_id, i.currency, i.amount_due,
         CASE WHEN i.subscription_id IS NOT NULL THEN 'subscription'
              WHEN EXISTS (SELECT 1 FROM invoice c WHERE c.consolidated_into_invoice_id = i.id) THEN 'consolidated'
              ELSE 'one_off' END AS invoice_kind,
         COALESCE(c.billing_address ->> 'country', '') AS country,
         c.custom_properties AS props,
         COALESCE((SELECT array_agg(DISTINCT p.plan_type::text) FROM member mm JOIN plan_version pv ON pv.id = mm.plan_version_id
                   JOIN plan p ON p.id = pv.plan_id WHERE mm.invoice_id = i.id), '{}') AS plan_types,
         COALESCE((SELECT array_agg(DISTINCT pv.plan_id) FROM member mm JOIN plan_version pv ON pv.id = mm.plan_version_id
                   WHERE mm.invoice_id = i.id), '{}') AS plan_ids,
         CASE
           WHEN s.payment_methods_config ->> 'type' = 'external' THEN 'NONE'
           WHEN s.payment_methods_config ->> 'type' = 'bank_transfer'
                THEN CASE WHEN COALESCE(s.payment_methods_config ->> 'account_id', ie.bank_account_id::text) IS NOT NULL THEN 'TRANSFER' ELSE 'NONE' END
           WHEN COALESCE(s.payment_methods_config -> 'config' -> 'card' ->> 'enabled', 'true') = 'true'
                AND EXISTS (SELECT 1 FROM customer_payment_method pm JOIN customer_connection cc ON cc.id = pm.connection_id
                            JOIN connector k ON k.id = cc.connector_id AND k.status = 'ACTIVE'
                            WHERE pm.customer_id = i.customer_id AND pm.archived_at IS NULL
                              AND pm.payment_method_type = 'CARD' AND cc.connector_id = ie.card_provider_id) THEN 'CARD'
           WHEN COALESCE(s.payment_methods_config -> 'config' -> 'direct_debit' ->> 'enabled', 'true') = 'true'
                AND EXISTS (SELECT 1 FROM customer_payment_method pm JOIN customer_connection cc ON cc.id = pm.connection_id
                            JOIN connector k ON k.id = cc.connector_id AND k.status = 'ACTIVE'
                            WHERE pm.customer_id = i.customer_id AND pm.archived_at IS NULL
                              AND pm.payment_method_type IN ('DIRECT_DEBIT_SEPA','DIRECT_DEBIT_ACH','DIRECT_DEBIT_BACS')
                              AND cc.connector_id = ie.direct_debit_provider_id) THEN 'DIRECT_DEBIT'
           ELSE 'NONE'
         END AS payment_method_type
  FROM invoice i
  JOIN customer c ON c.id = i.customer_id
  JOIN invoicing_entity ie ON ie.id = i.invoicing_entity_id
  LEFT JOIN subscription s ON s.id = COALESCE(i.subscription_id,
       (SELECT mm.subscription_id FROM member mm WHERE mm.invoice_id = i.id AND mm.subscription_id IS NOT NULL ORDER BY mm.subscription_id LIMIT 1))
  WHERE i.tenant_id = $1 AND i.id = ANY($2::uuid[])
)
