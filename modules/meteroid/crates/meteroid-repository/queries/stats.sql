--! top_revenue_per_customer
SELECT c.id,
       c.name,
       COALESCE(bi.total_revenue_cents, 0)::bigint AS total_revenue_cents
FROM customer c
         LEFT JOIN
     bi_customer_ytd_summary bi ON bi.customer_id = c.id
WHERE c.tenant_id = :tenant_id
  AND (bi.revenue_year IS NULL OR bi.currency = :currency)
  AND (bi.revenue_year IS NULL OR bi.revenue_year >= DATE_PART('year', CURRENT_DATE))
ORDER BY total_revenue_cents DESC
LIMIT :limit;

--! insert_mrr_movement_log
INSERT INTO bi_mrr_movement_log (id, movement_type, net_mrr_change, currency, applies_to, description, invoice_id,
                                 tenant_id, plan_version_id)
VALUES (:id, :movement_type, :net_mrr_change, :currency, :applies_to, :description, :invoice_id, :tenant_id,
        :plan_version_id);

--! new_mrr_at_date
SELECT net_mrr_cents
FROM bi_delta_mrr_daily
WHERE date = :date
  AND tenant_id = :tenant_id
  AND currency = :currency;

--! total_mrr_at_date
SELECT COALESCE(SUM(net_mrr_cents), 0)::bigint AS total_net_mrr
FROM bi_delta_mrr_daily
WHERE date <= :date
  AND tenant_id = :tenant_id
  AND currency = :currency;

--! total_mrr_at_date_by_plan
SELECT COALESCE(SUM(net_mrr_cents), 0)::bigint AS total_net_mrr, p.id as plan_id, p.name as plan_name
FROM bi_delta_mrr_daily bi
         JOIN plan_version pv on bi.plan_version_id = pv.id
         JOIN plan p on pv.plan_id = p.id
WHERE date <= :date
  AND bi.tenant_id = :tenant_id
  AND bi.currency = :currency
  AND p.id = ANY (:plan_ids)
GROUP BY p.id;

--! query_total_mrr
WITH initial_mrr AS (
    SELECT COALESCE(SUM(net_mrr_cents), 0)::bigint AS total_net_mrr
    FROM bi_delta_mrr_daily
    WHERE date < :start_date
      AND tenant_id = :tenant_id
      AND currency = :currency
)
SELECT date                       AS period,
       (im.total_net_mrr + COALESCE(SUM(net_mrr_cents) OVER (ORDER BY date), 0)::bigint) as total_net_mrr,
       net_mrr_cents::bigint      AS net_new_mrr,
       new_business_cents::bigint AS new_business_mrr,
       new_business_count,
       expansion_cents::bigint    AS expansion_mrr,
       expansion_count,
       contraction_cents::bigint  AS contraction_mrr,
       contraction_count,
       churn_cents::bigint        AS churn_mrr,
       churn_count,
       reactivation_cents::bigint AS reactivation_mrr,
       reactivation_count
FROM bi_delta_mrr_daily bi
         CROSS JOIN initial_mrr im
WHERE date BETWEEN :start_date AND :end_date
  AND bi.currency = :currency
  AND bi.tenant_id = :tenant_id
ORDER BY period;

--! query_total_mrr_by_plan
WITH initial_mrr AS (
    SELECT COALESCE(SUM(net_mrr_cents), 0)::bigint AS total_net_mrr, pv.plan_id as plan_id
    FROM bi_delta_mrr_daily bi
             JOIN plan_version pv on bi.plan_version_id = pv.id
    WHERE date < :start_date
      AND bi.tenant_id = :tenant_id
      AND bi.currency = :currency
      AND pv.plan_id = ANY (:plan_ids)
    GROUP BY pv.plan_id
)
SELECT date,
       p.id                       as plan_id,
       p.name                     as plan_name,
       (im.total_net_mrr + COALESCE(SUM(net_mrr_cents) OVER (ORDER BY date), 0)::bigint) as total_net_mrr,
       net_mrr_cents::bigint      AS net_new_mrr,
       new_business_cents::bigint AS new_business_mrr,
       new_business_count,
       expansion_cents::bigint    AS expansion_mrr,
       expansion_count,
       contraction_cents::bigint  AS contraction_mrr,
       contraction_count,
       churn_cents::bigint        AS churn_mrr,
       churn_count,
       reactivation_cents::bigint AS reactivation_mrr,
       reactivation_count
FROM bi_delta_mrr_daily bi
JOIN plan_version pv on bi.plan_version_id = pv.id
JOIN plan p on pv.plan_id = p.id
JOIN initial_mrr im on pv.plan_id = im.plan_id
WHERE date BETWEEN :start_date AND :end_date
  AND bi.currency = :currency
  AND bi.tenant_id = :tenant_id
  AND p.id = ANY (:plan_ids)
ORDER BY date;


--! get_mrr_breakdown
SELECT COALESCE(SUM(net_mrr_cents), 0)::bigint      AS net_new_mrr,
       COALESCE(SUM(new_business_cents), 0)::bigint AS new_business_mrr,
       COALESCE(SUM(new_business_count), 0)::integer AS new_business_count,
       COALESCE(SUM(expansion_cents), 0)::bigint    AS expansion_mrr,
       COALESCE(SUM(expansion_count), 0)::integer    AS expansion_count,
       COALESCE(SUM(contraction_cents), 0)::bigint  AS contraction_mrr,
       COALESCE(SUM(contraction_count), 0)::integer  AS contraction_count,
       COALESCE(SUM(churn_cents), 0)::bigint        AS churn_mrr,
       COALESCE(SUM(churn_count), 0)::integer        AS churn_count,
       COALESCE(SUM(reactivation_cents), 0)::bigint AS reactivation_mrr,
       COALESCE(SUM(reactivation_count), 0)::integer AS reactivation_count
FROM bi_delta_mrr_daily bi
         JOIN tenant t on bi.tenant_id = t.id
WHERE date BETWEEN :start_date AND :end_date
  AND bi.currency = t.currency
  AND bi.tenant_id = :tenant_id;


--! query_total_net_revenue(currency?)
SELECT COALESCE(SUM(net_revenue_cents), 0)::bigint AS total_net_revenue,
       currency
FROM bi_revenue_daily
WHERE revenue_date BETWEEN :start_date AND :end_date
  AND (currency = :currency OR :currency IS NULL)
  AND tenant_id = :tenant_id
GROUP BY currency
ORDER BY currency;

--! get_last_mrr_movements (before?, after?)
SELECT bi.id,
       bi.movement_type,
       bi.net_mrr_change,
       bi.currency,
       bi.applies_to,
       bi.created_at,
       bi.description,
       bi.invoice_id,
       bi.credit_note_id,
       bi.tenant_id,
       bi.plan_version_id,
       c.id   as customer_id,
       c.name as customer_name,
       s.id   as subscription_id,
       p.name as plan_name
FROM bi_mrr_movement_log bi
         LEFT JOIN invoice i on bi.invoice_id = i.id
         JOIN subscription s on i.subscription_id = s.id
         JOIN plan_version pv on bi.plan_version_id = pv.id
         JOIN plan p on pv.plan_id = p.id
         JOIN customer c on s.customer_id = c.id
WHERE bi.tenant_id = :tenant_id
  AND (bi.id < :before OR :before IS NULL)
  AND (bi.id > :after OR :after IS NULL)
ORDER BY bi.id DESC
LIMIT :limit;


--! query_revenue_trend
WITH period AS (SELECT CURRENT_DATE - INTERVAL '1 day' * :period_days::integer       AS start_current_period,
                       CURRENT_DATE - INTERVAL '1 day' * (:period_days::integer * 2) AS start_previous_period),
     revenue_ytd AS (SELECT COALESCE(SUM(net_revenue_cents), 0)::bigint AS total_ytd
                     FROM bi_revenue_daily
                              JOIN
                          tenant ON bi_revenue_daily.tenant_id = tenant.id
                     WHERE revenue_date BETWEEN DATE_TRUNC('year', CURRENT_DATE) AND CURRENT_DATE
                       AND bi_revenue_daily.tenant_id = :tenant_id
                       AND tenant.currency = bi_revenue_daily.currency),
     current_period AS (SELECT COALESCE(SUM(net_revenue_cents), 0)::bigint AS total
                        FROM bi_revenue_daily
                                 JOIN
                             period ON revenue_date BETWEEN period.start_current_period AND CURRENT_DATE
                                 JOIN
                             tenant ON bi_revenue_daily.tenant_id = tenant.id
                        WHERE bi_revenue_daily.tenant_id = :tenant_id
                          AND tenant.currency = bi_revenue_daily.currency),
     previous_period AS (SELECT COALESCE(SUM(net_revenue_cents), 0)::bigint AS total
                         FROM bi_revenue_daily
                                  JOIN
                              period
                              ON revenue_date BETWEEN period.start_previous_period AND period.start_current_period
                                  JOIN
                              tenant ON bi_revenue_daily.tenant_id = tenant.id
                         WHERE bi_revenue_daily.tenant_id = :tenant_id
                           AND tenant.currency = bi_revenue_daily.currency)
SELECT COALESCE(revenue_ytd.total_ytd, 0) AS total_ytd,
       COALESCE(current_period.total, 0)  AS total_current_period,
       COALESCE(previous_period.total, 0) AS total_previous_period
FROM revenue_ytd,
     current_period,
     previous_period;

--! count_active_subscriptions
SELECT COUNT(*) AS total
FROM subscription
WHERE tenant_id = :tenant_id
  AND status = 'ACTIVE';

--! query_pending_invoices
SELECT COUNT(*)::integer AS total, COALESCE(SUM(amount_cents), 0) AS total_cents
FROM invoice
WHERE tenant_id = :tenant_id
  AND status = 'PENDING';

--! daily_new_signups_30_days
WITH date_series AS (SELECT DATE(current_date - INTERVAL '1 day' * generate_series(0, 29)) AS date),
     daily_signups AS (SELECT DATE(created_at) AS signup_date,
                              COUNT(*)         AS daily_signups
                       FROM customer
                       WHERE tenant_id = :tenant_id
                         AND created_at >= CURRENT_DATE - INTERVAL '30 days'
                       GROUP BY signup_date)
SELECT ds.date                                                                        as signup_date,
       COALESCE(d.daily_signups, 0)                                                   AS daily_signups,
       COALESCE(SUM(COALESCE(d.daily_signups, 0)) OVER (ORDER BY ds.date), 0)::bigint AS total_signups_over_30_days
FROM date_series ds
         LEFT JOIN daily_signups d ON ds.date = d.signup_date
ORDER BY ds.date;


--! new_signups_trend_30_days
WITH signup_counts AS (SELECT DATE(created_at) AS signup_date,
                              COUNT(*)         AS daily_signups
                       FROM customer
                       WHERE tenant_id = :tenant_id
                         AND created_at >= CURRENT_DATE - INTERVAL '60 days'
                       GROUP BY signup_date)
SELECT COALESCE(SUM(daily_signups) FILTER (WHERE signup_date > CURRENT_DATE - INTERVAL '30 days'),
                0)::bigint                                                                                    AS total_last_30_days,
       COALESCE(SUM(daily_signups) FILTER (WHERE signup_date <= CURRENT_DATE - INTERVAL '30 days' AND
                                                 signup_date > CURRENT_DATE - INTERVAL '60 days'),
                0)::bigint                                                                                    AS total_previous_30_days
FROM signup_counts;

--! get_all_time_trial_conversion_rate
SELECT CASE
           WHEN COUNT(*) > 0 THEN
               ROUND((COUNT(*) FILTER (WHERE s.activated_at IS NOT NULL)::DECIMAL / COUNT(*)) * 100, 2)
           ELSE
               0
           END AS all_time_conversion_rate_percentage
FROM subscription s
WHERE s.tenant_id = :tenant_id
  AND s.trial_start_date IS NOT NULL;

--! query_trial_to_paid_conversion_over_time
WITH month_series AS (SELECT generate_series(
                                     DATE_TRUNC('month', COALESCE(MIN(trial_start_date), CURRENT_DATE)),
                                     CURRENT_DATE,
                                     '1 month'
                             ) AS month
                      FROM subscription
                      WHERE tenant_id = :tenant),
     monthly_trials AS (SELECT ms.month,
                               COALESCE(COUNT(s.trial_start_date), 0)                                                AS total_trials,
                               COALESCE(COUNT(s.activated_at)
                                        FILTER (WHERE s.activated_at - s.trial_start_date <= INTERVAL '30 days'),
                                        0)                                                                           AS conversions_30,
                               COALESCE(COUNT(s.activated_at)
                                        FILTER (WHERE s.activated_at - s.trial_start_date <= INTERVAL '90 days'),
                                        0)                                                                           AS conversions_90,
                               COALESCE(COUNT(s.activated_at), 0)                                                    AS conversions
                        FROM month_series ms
                                 LEFT JOIN subscription s ON DATE_TRUNC('month', s.trial_start_date) = ms.month
                            AND s.tenant_id = :tenant
                        GROUP BY ms.month
                        ORDER BY ms.month)
SELECT month,
       total_trials,
       conversions,
       CASE
           WHEN total_trials > 0 THEN ROUND((conversions::DECIMAL / total_trials) * 100, 2)
           ELSE 0 END                                                                                      AS conversion_rate_percentage,
       conversions_30,
       CASE
           WHEN total_trials > 0 THEN ROUND((conversions_30::DECIMAL / total_trials) * 100, 2)
           ELSE 0 END                                                                                      AS conversion_rate_30_percentage,
       conversions_90,
       CASE
           WHEN total_trials > 0 THEN ROUND((conversions_90::DECIMAL / total_trials) * 100, 2)
           ELSE 0 END                                                                                      AS conversion_rate_90_percentage
FROM monthly_trials;