--: BillableMetric(description?, aggregation_key?, unit_conversion_factor?, unit_conversion_rounding?, segmentation_matrix?, usage_group_key?, archived_at?)

--! create_billable_metric (description?, aggregation_key?, unit_conversion_factor?, unit_conversion_rounding?, segmentation_matrix?, usage_group_key?) : BillableMetric
INSERT INTO billable_metric (id,
                             name,
                             description,
                             code,
                             aggregation_type,
                             aggregation_key,
                             unit_conversion_factor,
                             unit_conversion_rounding,
                             segmentation_matrix,
                             usage_group_key,
                             tenant_id,
                             created_by,
                             product_family_id)
VALUES (:id,
        :name,
        :description,
        :code,
        :aggregation_type,
        :aggregation_key,
        :unit_conversion_factor,
        :unit_conversion_rounding,
        :segmentation_matrix,
        :usage_group_key,
        :tenant_id,
        :created_by,
        (SELECT id
         FROM product_family
         WHERE external_id = :product_family_external_id
           AND tenant_id = :tenant_id))
RETURNING id,
    name,
    description,
    code,
    aggregation_type,
    aggregation_key,
    unit_conversion_factor,
    unit_conversion_rounding,
    segmentation_matrix,
    usage_group_key,
    created_at,
    created_by,
    archived_at;

--! list_billable_metrics () : (aggregation_key?, archived_at?)
SELECT bm.id,
       bm.name,
       bm.description,
       bm.code,
       bm.aggregation_type,
       bm.aggregation_key,
       bm.created_at,
       bm.created_by,
       bm.archived_at,
       COUNT(*) OVER () AS total_count
FROM billable_metric AS bm
         JOIN product_family AS pf ON bm.product_family_id = pf.id
WHERE pf.external_id = :product_family_external_id
  AND bm.tenant_id = :tenant_id
ORDER BY bm.created_at ASC
LIMIT :limit OFFSET :offset;


--! get_billable_metric_by_id (id) : BillableMetric
SELECT bm.id,
       bm.name,
       bm.description,
       bm.code,
       bm.aggregation_type,
       bm.aggregation_key,
       bm.unit_conversion_factor,
       bm.unit_conversion_rounding,
       bm.segmentation_matrix,
       bm.usage_group_key,
       bm.created_at,
       bm.created_by,
       bm.archived_at
FROM billable_metric AS bm
WHERE bm.id = :id
  AND bm.tenant_id = :tenant_id;


--! get_billable_metric_by_ids () : BillableMetric
SELECT bm.id,
       bm.name,
       bm.description,
       bm.code,
       bm.aggregation_type,
       bm.aggregation_key,
       bm.unit_conversion_factor,
       bm.unit_conversion_rounding,
       bm.segmentation_matrix,
       bm.usage_group_key,
       bm.created_at,
       bm.created_by,
       bm.archived_at
FROM billable_metric AS bm
WHERE bm.id = ANY (:ids)
  AND bm.tenant_id = :tenant_id;