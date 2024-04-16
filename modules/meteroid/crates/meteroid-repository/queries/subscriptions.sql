-- TODO improve

--: SubscriptionToInvoice(billing_end_date?, activated_at?, canceled_at?)
--! subscription_to_invoice_candidates (input_date) : SubscriptionToInvoice
SELECT s.id AS subscription_id,
       s.tenant_id,
       s.customer_id,
       pp.plan_id,
       s.plan_version_id,
       s.billing_start_date,
       s.billing_end_date,
       s.billing_day,
       s.activated_at,
       s.canceled_at,
       s.effective_billing_period,
       s.input_parameters,
       pp.currency,
       pp.net_terms,
       pp.version
FROM subscription s
       JOIN plan_version pp ON s.plan_version_id = pp.id
       LEFT JOIN invoice i ON s.id = i.subscription_id AND i.invoice_date > :input_date
where (s.billing_end_date is null OR s.billing_end_date > :input_date)
  AND i.id IS NULL;

--: Subscription(billing_end_date?, activated_at?, canceled_at?, customer_external_id?, trial_start_date?)
--! get_subscription_by_id: Subscription
SELECT s.id,
       s.tenant_id,
       s.plan_version_id,
       s.billing_start_date,
       s.billing_end_date,
       s.billing_day,
       s.activated_at,
       s.canceled_at,
       s.trial_start_date,
       s.effective_billing_period,
       s.input_parameters,
       s.customer_id,
       c.alias as customer_external_id,
       c.name  AS customer_name,
       p.id    AS plan_id,
       p.name  AS plan_name,
       pp.currency,
       pp.version,
       s.net_terms
FROM subscription s
       JOIN plan_version pp ON s.plan_version_id = pp.id
       JOIN plan p ON pp.plan_id = p.id
       JOIN customer c ON s.customer_id = c.id
WHERE s.id = :subscription_id
  AND s.tenant_id = :tenant_id;

--! create_subscription (billing_end?, parameters?)
INSERT INTO subscription (id,
                          tenant_id,
                          customer_id,
                          created_by,
                          plan_version_id,
                          billing_start_date,
                          billing_end_date,
                          billing_day,
                          effective_billing_period,
                          input_parameters,
                          net_terms)
VALUES (:id,
        :tenant_id,
        :customer_id,
        :created_by,
        :plan_version_id,
        :billing_start,
        :billing_end,
        :billing_day,
        :effective_billing_period,
        :parameters,
        :net_terms)
RETURNING id
;


--: SubscriptionList(billing_end_date?, activated_at?, canceled_at?, trial_start_date?)
--! list_subscriptions(customer_id?, plan_id?) : SubscriptionList
SELECT s.id             AS subscription_id,
       s.tenant_id,
       s.customer_id,
       s.plan_version_id,
       s.billing_start_date,
       s.billing_end_date,
       s.billing_day,
       s.activated_at,
       s.canceled_at,
       s.trial_start_date,
       s.effective_billing_period,
       s.input_parameters,
       s.net_terms,
       pp.currency,
       pp.version,
       c.name           AS customer_name,
       p.id             AS plan_id,
       p.name           AS plan_name,
       count(*) OVER () AS total_count
FROM subscription s
       JOIN plan_version pp ON s.plan_version_id = pp.id
       JOIN plan p ON pp.plan_id = p.id
       JOIN customer c ON s.customer_id = c.id
WHERE s.tenant_id = :tenant_id
  AND (:plan_id :: UUID IS NULL OR pp.plan_id = :plan_id)
  AND (:customer_id :: UUID IS NULL OR s.customer_id = :customer_id)
ORDER BY s.id DESC
LIMIT :limit OFFSET :offset;

--! cancel_subscription()
UPDATE subscription
SET billing_end_date = :billing_end_date,
    canceled_at      = :canceled_at
WHERE id = :id
  and canceled_at is null;

--! activate_subscription()
UPDATE subscription
SET activated_at = :activated_at
WHERE id = :id
  and activated_at is null;
