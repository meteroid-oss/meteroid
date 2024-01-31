--: PriceComponent(product_item_id?, product_item_name?)

--! upsert_price_component(product_item_id?, billable_metric_id?)
INSERT INTO price_component (id, name, fee, plan_version_id, product_item_id, billable_metric_id)
SELECT :id,
       :name,
       :fee,
       :plan_version_id,
       :product_item_id,
       :billable_metric_id
FROM plan_version
WHERE plan_version.id = :plan_version_id
  AND plan_version.tenant_id = :tenant_id
  AND plan_version.is_draft_version = true
ON CONFLICT (id) DO UPDATE SET name               = EXCLUDED.name,
                               fee                = EXCLUDED.fee,
                               product_item_id    = EXCLUDED.product_item_id,
                               billable_metric_id = EXCLUDED.billable_metric_id;


--! list_price_components : PriceComponent
SELECT pc.id, pc.name, pc.fee, pc.product_item_id, pi.name as product_item_name
FROM price_component pc
         JOIN plan_version pv ON pc.plan_version_id = pv.id
         LEFT JOIN product pi ON pc.product_item_id = pi.id
WHERE pv.id = :plan_version_id
  AND pv.tenant_id = :tenant_id;

--! get_price_component : PriceComponent
SELECT pc.id, pc.name, pc.fee, pc.product_item_id, pi.name as product_item_name
FROM price_component pc
         JOIN plan_version pv ON pc.plan_version_id = pv.id
         LEFT JOIN product pi ON pc.product_item_id = pi.id
WHERE pc.id = :component_id
  AND pv.tenant_id = :tenant_id;


--: PriceComponentWithMetric(product_item_id?, product_item_name?, billable_metric_id?)
--! list_price_components_by_subscription : PriceComponentWithMetric
SELECT pc.id,
       pc.name,
       pc.fee,
       pc.product_item_id,
       pi.name as product_item_name,
       bm.id   as billable_metric_id
FROM price_component pc
         JOIN subscription ss ON pc.plan_version_id = ss.plan_version_id
         JOIN plan_version pv ON pc.plan_version_id = pv.id
         LEFT JOIN product pi ON pc.product_item_id = pi.id
         LEFT JOIN billable_metric bm ON pc.billable_metric_id = bm.id
WHERE ss.id = :subscription_id;

--! delete_price_component
DELETE
FROM price_component pc
    USING plan_version pv
WHERE pc.id = :id
  AND pc.plan_version_id = pv.id
  AND pv.tenant_id = :tenant_id
  AND pv.is_draft_version = true;

