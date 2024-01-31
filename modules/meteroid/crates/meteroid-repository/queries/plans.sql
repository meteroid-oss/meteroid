--: Plan(description?)
--: ListPlan(description?)
--! create_plan (description?) : Plan
INSERT INTO
  plan(
    id,
    name,
    external_id,
    description,
    tenant_id,
    created_by,
    status,
    plan_type,
    product_family_id
  )
VALUES
  (
    :id,
    :name,
    :external_id,
    :description,
    :tenant_id,
    :created_by,
    :status,
    :plan_type,
    (
      SELECT
        id
      FROM
        product_family
      WHERE
        external_id = :product_family_external_id
    )
  ) RETURNING id,
  name,
  external_id,
  description,
  status,
  plan_type;

--: PlanVersion(trial_duration_days?, trial_fallback_plan_id?, period_start_day?, billing_cycles?)
--: ListPlanVersion(trial_duration_days?, trial_fallback_plan_id?, period_start_day?, billing_cycles?)

--! get_plan_version_by_id () : PlanVersion
SELECT
  id,
  is_draft_version,
  plan_id,
  version,
  created_by,
  trial_duration_days,
  trial_fallback_plan_id,
  tenant_id,
  period_start_day,
  net_terms,
  currency,
  billing_cycles,
  billing_periods
FROM
  plan_version
WHERE
  id = :plan_version_id
  AND tenant_id = :tenant_id;

--! create_plan_version(net_terms?, currency?, trial_duration_days?, trial_fallback_plan_id?, period_start_day?, billing_cycles?) : PlanVersion
INSERT INTO
  plan_version (
    id,
    is_draft_version,
    plan_id,
    version,
    created_by,
    trial_duration_days,
    trial_fallback_plan_id,
    tenant_id,
    period_start_day,
    net_terms,
    currency,
    billing_cycles,
    billing_periods
  )
VALUES
  (
    :id,
    TRUE,
    :plan_id,
    :version,
    :created_by,
    :trial_duration_days,
    :trial_fallback_plan_id,
    :tenant_id,
    :period_start_day,
    COALESCE(:net_terms, 0),
    COALESCE(:currency, (SELECT currency FROM tenant WHERE id = :tenant_id)),
    :billing_cycles,
    :billing_periods
  ) RETURNING id,
  is_draft_version,
  plan_id,
  version,
  created_by,
  trial_duration_days,
  trial_fallback_plan_id,
  tenant_id,
  period_start_day,
  net_terms,
  currency,
  billing_cycles,
  billing_periods;

--! copy_version_to_draft : PlanVersion
WITH original_plan_version AS (
  SELECT
    *
  FROM
    plan_version
  WHERE
    id = :original_plan_version_id
    AND tenant_id = :tenant_id
),
new_plan_version AS (
  -- Create a new draft version of the plan
  INSERT INTO
    plan_version (
      id,
      is_draft_version,
      plan_id,
      version,
      created_by,
      trial_duration_days,
      trial_fallback_plan_id,
      tenant_id,
      period_start_day,
      net_terms,
      currency,
      billing_cycles,
      billing_periods
    )
  SELECT
    :new_plan_version_id,
    TRUE,
    plan_id,
    version + 1,
    :created_by,
    trial_duration_days,
    trial_fallback_plan_id,
    tenant_id,
    period_start_day,
    net_terms,
    currency,
    billing_cycles,
    billing_periods
  FROM
    original_plan_version RETURNING id,
  is_draft_version,
  plan_id,
  version,
  created_by,
  trial_duration_days,
  trial_fallback_plan_id,
  tenant_id,
  period_start_day,
  net_terms,
  currency,
  billing_cycles,
  billing_periods
),
duplicate_price_component AS (
  -- Duplicate 'price_component' records
  INSERT INTO
    price_component (id, name, fee, plan_version_id, product_item_id)
  SELECT
    gen_random_uuid(),
    name,
    fee,
    new_plan_version.id,
    product_item_id
  FROM
    price_component,
    new_plan_version
  WHERE
    plan_version_id = :original_plan_version_id
),
duplicate_schedule AS (
  INSERT INTO
    schedule (
      id,
      billing_period,
      plan_version_id,
      ramps
  )
  SELECT
      gen_random_uuid(),
      billing_period,
      new_plan_version.id,
      ramps
  FROM
      schedule,
      new_plan_version
  WHERE
      plan_version_id = :original_plan_version_id
)
SELECT
  *
FROM
  new_plan_version;


--! publish_plan_version : PlanVersion
UPDATE
  plan_version
SET
  is_draft_version = FALSE
WHERE
  id = :plan_version_id
  AND tenant_id = :tenant_id RETURNING id,
  is_draft_version,
  plan_id,
  version,
  created_by,
  trial_duration_days,
  trial_fallback_plan_id,
  tenant_id,
  period_start_day,
  net_terms,
  currency,
  billing_cycles,
  billing_periods;

--! activate_plan
UPDATE
  plan
SET
    status = 'ACTIVE'
WHERE
    id = :plan_id
    AND tenant_id = :tenant_id;


--! find_plan_by_external_id () : Plan
SELECT
  id,
  name,
  external_id,
  description,
  status,
  plan_type
FROM
  plan
WHERE
  tenant_id = :tenant_id
  AND external_id = :external_id;

--! list_plans (search?) : ListPlan
SELECT
  plan.id,
  plan.name,
  plan.external_id,
  plan.description,
  plan.status,
  plan.plan_type,
  COUNT(*) OVER() AS total_count
FROM
  plan
  JOIN product_family ON plan.product_family_id = product_family.id
WHERE
  plan.tenant_id = :tenant_id
  AND (
    :search :: TEXT IS NULL
    OR to_tsvector('english', plan.name || ' ' || plan.external_id) @@ to_tsquery('english', :search)
  )
  AND product_family.external_id = :product_family_external_id
ORDER BY
  CASE
    WHEN :order_by = 'DATE_DESC' THEN plan.id
  END DESC,
  CASE
    WHEN :order_by = 'DATE_ASC' THEN plan.id
  END ASC,
  CASE
    WHEN :order_by = 'NAME_DESC' THEN plan.name
  END DESC,
  CASE
    WHEN :order_by = 'NAME_ASC' THEN plan.name
  END ASC
LIMIT
  :limit OFFSET :offset;

--! list_plans_versions : ListPlanVersion
SELECT
  id,
  is_draft_version,
  plan_id,
  version,
  created_by,
  trial_duration_days,
  trial_fallback_plan_id,
  tenant_id,
  period_start_day,
  net_terms,
  currency,
  billing_cycles,
  billing_periods,
  COUNT(*) OVER() AS total_count
FROM
  plan_version
WHERE
  plan_version.tenant_id = :tenant_id
  AND plan_version.plan_id = :plan_id
ORDER BY
  plan_version.version DESC
LIMIT
  :limit OFFSET :offset;

--! last_plan_version(is_draft?) : PlanVersion
SELECT
    id,
    is_draft_version,
    plan_id,
    version,
    created_by,
    trial_duration_days,
    trial_fallback_plan_id,
    tenant_id,
    period_start_day,
    net_terms,
    currency,
    billing_cycles,
    billing_periods
FROM
    plan_version
WHERE
        plan_version.tenant_id = :tenant_id
  AND plan_version.plan_id = :plan_id
  -- only if is_draft is not null, check is_draft_version
    AND (
        -- below does not work, we need to cast to bool
        :is_draft::bool IS NULL
        OR plan_version.is_draft_version = :is_draft
    )
ORDER BY
    plan_version.version DESC
    LIMIT
  1;

--! delete_draft_plan_version
DELETE
FROM
  plan_version
WHERE
  id = :plan_version_id
  AND tenant_id = :tenant_id
  AND is_draft_version = TRUE;

--! delete_all_draft_versions_of_same_plan
DELETE
FROM
    plan_version pv1
USING
    plan_version pv2
WHERE
    pv1.plan_id = pv2.plan_id
  AND pv1.tenant_id = pv2.tenant_id
  AND pv1.is_draft_version = TRUE
  AND pv2.id = :plan_version_id
  AND pv2.tenant_id = :tenant_id;

--! update_plan_version_overview
UPDATE plan_version
SET
    currency = :currency,
    net_terms = :net_terms,
    billing_periods = :billing_periods
WHERE
        tenant_id = :tenant_id
    AND id = :plan_version_id
    AND is_draft_version = TRUE;

--! update_plan_overview(description?)
UPDATE plan
SET
    name = :name,
    description = :description
WHERE
    tenant_id = :tenant_id
  AND id = :plan_id;

--: PlanOverview(description?)
--! get_plan_overview_by_external_id : PlanOverview
SELECT
    p.id,
    p.name,
    p.description,
    pv.id as plan_version_id,
    pv.is_draft_version,
    pv.currency,
    pv.version,
    pv.net_terms,
    pv.billing_periods
FROM
    plan_version pv
JOIN
    plan p ON pv.plan_id = p.id
WHERE
    p.external_id = :external_id
  AND p.tenant_id = :tenant_id
ORDER BY pv.version DESC
LIMIT 1;


--! get_plan_overview_by_id : PlanOverview
SELECT
    p.id,
    p.name,
    p.description,
    pv.id as plan_version_id,
    pv.is_draft_version,
    pv.version,
    pv.currency,
    pv.net_terms,
    pv.billing_periods
FROM
    plan_version pv
        JOIN
    plan p ON pv.plan_id = p.id
WHERE
        pv.id = :plan_version_id
  AND p.tenant_id = :tenant_id;


--! delete_plan_if_no_versions
DELETE
FROM
    plan
WHERE
    id = :plan_id
  AND tenant_id = :tenant_id
  AND NOT EXISTS (
    SELECT
        1
    FROM
        plan_version
    WHERE
            plan_version.plan_id = plan.id
        AND plan_version.tenant_id = plan.tenant_id
  );