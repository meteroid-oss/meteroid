use crate::stats::{
    ActiveSubscriptionsCountRow, CustomerTopRevenueRow, DailyNewSignups90DaysRow,
    LastMrrMovementsRow, MrrBreakdownRow, NewSignupsTrend90DaysRow, PendingInvoicesTotalRow,
    RevenueChartRow, RevenueTrendRow, SubscriptionTrialConversionRateRow,
    SubscriptionTrialToPaidConversionRow, TotalMrrByPlanRow, TotalMrrChartRow, TotalMrrRow,
};
use crate::{DbResult, PgConn};
use chrono::{NaiveDate, NaiveDateTime};
use diesel::{
    BoolExpressionMethods, ExpressionMethods, OptionalExtension, QueryDsl, debug_query, sql_types,
};
use diesel_async::RunQueryDsl;
use uuid::Uuid;

use crate::errors::IntoDbResult;
use common_domain::ids::TenantId;
use error_stack::ResultExt;

impl RevenueTrendRow {
    pub async fn get(
        conn: &mut PgConn,
        period_days: i32,
        tenant_id: TenantId,
    ) -> DbResult<RevenueTrendRow> {
        let raw_sql = r"
    WITH period AS (SELECT CURRENT_DATE - INTERVAL '1 day' * $1::integer       AS start_current_period,
                       CURRENT_DATE - INTERVAL '1 day' * ($2::integer * 2) AS start_previous_period),
    conversion_rates AS (SELECT id,
                                 (rates ->> (SELECT reporting_currency FROM tenant WHERE id = $3))::NUMERIC AS conversion_rate
                          FROM historical_rates_from_usd),
    revenue_ytd AS (SELECT COALESCE(SUM(net_revenue_cents * cr.conversion_rate), 0)::bigint AS total_ytd
                     FROM bi_revenue_daily
                              JOIN conversion_rates cr ON bi_revenue_daily.historical_rate_id = cr.id
                     WHERE revenue_date BETWEEN DATE_TRUNC('year', CURRENT_DATE) AND CURRENT_DATE
                       AND bi_revenue_daily.tenant_id = $4),
    current_period AS (SELECT COALESCE(SUM(net_revenue_cents_usd * cr.conversion_rate), 0)::bigint AS total
                        FROM bi_revenue_daily
                                 JOIN
                             period ON revenue_date BETWEEN period.start_current_period AND CURRENT_DATE
                                 JOIN
                             conversion_rates cr ON bi_revenue_daily.historical_rate_id = cr.id
                        WHERE bi_revenue_daily.tenant_id = $5),
    previous_period AS (SELECT COALESCE(SUM(net_revenue_cents_usd * cr.conversion_rate), 0)::bigint AS total
                         FROM bi_revenue_daily
                                  JOIN
                              period
                              ON revenue_date BETWEEN period.start_previous_period AND period.start_current_period
                                  JOIN
                              conversion_rates cr ON bi_revenue_daily.historical_rate_id = cr.id
                         WHERE bi_revenue_daily.tenant_id = $6)
    SELECT COALESCE(revenue_ytd.total_ytd, 0) AS total_ytd,
           COALESCE(current_period.total, 0)  AS total_current_period,
           COALESCE(previous_period.total, 0) AS total_previous_period
    FROM revenue_ytd,
         current_period,
         previous_period;
    ";

        diesel::sql_query(raw_sql)
            .bind::<sql_types::Integer, _>(period_days)
            .bind::<sql_types::Integer, _>(period_days)
            .bind::<sql_types::Uuid, _>(tenant_id)
            .bind::<sql_types::Uuid, _>(tenant_id)
            .bind::<sql_types::Uuid, _>(tenant_id)
            .bind::<sql_types::Uuid, _>(tenant_id)
            .get_result::<RevenueTrendRow>(conn)
            .await
            .attach("Error while fetching revenue trend")
            .into_db_result()
    }
}

impl NewSignupsTrend90DaysRow {
    pub async fn get(conn: &mut PgConn, tenant_id: TenantId) -> DbResult<NewSignupsTrend90DaysRow> {
        let raw_sql = r"
        WITH signup_counts AS (SELECT DATE(created_at) AS signup_date, COUNT(*) AS daily_signups
                       FROM customer
                       WHERE tenant_id = $1
                         AND created_at >= CURRENT_DATE - INTERVAL '180 days'
                       GROUP BY signup_date)
        SELECT COALESCE(SUM(daily_signups) FILTER (WHERE signup_date > CURRENT_DATE - INTERVAL '90 days'),
                        0)::bigint  AS total_last_90_days,
               COALESCE(SUM(daily_signups) FILTER (WHERE signup_date <= CURRENT_DATE - INTERVAL '90 days' AND
                        signup_date > CURRENT_DATE - INTERVAL '180 days'), 0)::bigint  AS total_previous_90_days
        FROM signup_counts;
        ";

        diesel::sql_query(raw_sql)
            .bind::<sql_types::Uuid, _>(tenant_id)
            .get_result::<NewSignupsTrend90DaysRow>(conn)
            .await
            .attach("Error while fetching revenue trend")
            .into_db_result()
    }
}

impl PendingInvoicesTotalRow {
    pub async fn get(conn: &mut PgConn, tenant_id: TenantId) -> DbResult<PendingInvoicesTotalRow> {
        let raw_sql = r"
        WITH tenant_currency AS (
            SELECT reporting_currency AS currency FROM tenant WHERE id = $1
        ),
        latest_rate AS (
            SELECT
                rates
            FROM
                historical_rates_from_usd
            WHERE
                date  <= CURRENT_DATE
            ORDER BY date DESC
            LIMIT 1
        ),
        converted_invoices AS (
            SELECT
                convert_currency(
                        i.total,
                        (SELECT (rates->>i.currency)::NUMERIC FROM latest_rate),
                        (SELECT (rates->>(SELECT currency FROM tenant_currency))::NUMERIC FROM latest_rate)
                )::BIGINT AS converted_amount_cents
            FROM
                invoice i,
                latest_rate,
                tenant_currency
            WHERE
                i.tenant_id = $2
              AND i.status = 'FINALIZED'
              AND i.paid_at IS NULL
        )
        SELECT
            COUNT(*)::integer AS total,
            COALESCE(SUM(converted_amount_cents), 0) AS total_cents
        FROM
            converted_invoices;
        ";

        diesel::sql_query(raw_sql)
            .bind::<sql_types::Uuid, _>(tenant_id)
            .bind::<sql_types::Uuid, _>(tenant_id)
            .get_result::<PendingInvoicesTotalRow>(conn)
            .await
            .attach("Error while fetching pending invoices totals")
            .into_db_result()
    }
}

impl DailyNewSignups90DaysRow {
    pub async fn list(
        conn: &mut PgConn,
        tenant_id: TenantId,
    ) -> DbResult<Vec<DailyNewSignups90DaysRow>> {
        let raw_sql = r"
        WITH date_series AS (SELECT DATE(current_date - INTERVAL '1 day' * generate_series(0, 89)) AS date),
        daily_signups AS (SELECT DATE(created_at) AS signup_date,
                              COUNT(*)         AS daily_signups
                       FROM customer
                       WHERE tenant_id = $1
                         AND created_at >= CURRENT_DATE - INTERVAL '90 days'
                       GROUP BY signup_date)
        SELECT ds.date                                                                        as signup_date,
               COALESCE(d.daily_signups, 0)                                                   AS daily_signups,
               COALESCE(SUM(COALESCE(d.daily_signups, 0)) OVER (ORDER BY ds.date), 0)::bigint AS total_signups_over_30_days
        FROM date_series ds
                 LEFT JOIN daily_signups d ON ds.date = d.signup_date
        ORDER BY ds.date;
        ";

        diesel::sql_query(raw_sql)
            .bind::<sql_types::Uuid, _>(tenant_id)
            .get_results::<DailyNewSignups90DaysRow>(conn)
            .await
            .attach("Error while fetching daily new signups 90 days")
            .into_db_result()
    }
}

impl ActiveSubscriptionsCountRow {
    pub async fn get(
        conn: &mut PgConn,
        tenant_id: TenantId,
        at_ts: Option<NaiveDateTime>,
    ) -> DbResult<ActiveSubscriptionsCountRow> {
        use crate::schema::subscription::dsl as s_dsl;

        let ts = at_ts.unwrap_or_else(|| chrono::Utc::now().naive_utc());

        let query = s_dsl::subscription
            .filter(s_dsl::tenant_id.eq(tenant_id))
            .filter(s_dsl::activated_at.le(ts))
            .filter(s_dsl::end_date.is_null().or(s_dsl::end_date.ge(ts.date())))
            .count();

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while get active subscriptions count")
            .into_db_result()
            .map(|c: i64| ActiveSubscriptionsCountRow { count: c as i32 })
    }
}

impl SubscriptionTrialConversionRateRow {
    pub async fn get(
        conn: &mut PgConn,
        tenant_id: TenantId,
    ) -> DbResult<SubscriptionTrialConversionRateRow> {
        let raw_sql = r"
        SELECT CASE
           WHEN COUNT(*) > 0 THEN
               ROUND((COUNT(*) FILTER (WHERE s.activated_at IS NOT NULL)::DECIMAL / COUNT(*)) * 100, 2)
           ELSE
               0
           END AS all_time_conversion_rate_percentage
        FROM subscription s
        WHERE s.tenant_id = $1
           AND s.trial_duration IS NOT NULL;
        ";

        diesel::sql_query(raw_sql)
            .bind::<sql_types::Uuid, _>(tenant_id)
            .get_result::<SubscriptionTrialConversionRateRow>(conn)
            .await
            .attach("Error while fetching subscription trial conversion rate")
            .into_db_result()
    }
}

impl SubscriptionTrialToPaidConversionRow {
    pub async fn list(
        _conn: &mut PgConn,
        _tenant_id: TenantId,
    ) -> DbResult<Vec<SubscriptionTrialToPaidConversionRow>> {
        //         let raw_sql = r#"
        // WITH month_series AS (SELECT generate_series(
        //                                      DATE_TRUNC('month', COALESCE(MIN(trial_start_date), CURRENT_DATE)),
        //                                      CURRENT_DATE,
        //                                      '1 month'
        //                              ) AS month
        //                       FROM subscription
        //                       WHERE tenant_id = $1),
        //      monthly_trials AS (SELECT ms.month,
        //                                COALESCE(COUNT(s.trial_start_date), 0)                                                AS total_trials,
        //                                COALESCE(COUNT(s.activated_at)
        //                                         FILTER (WHERE s.activated_at - s.trial_start_date <= INTERVAL '30 days'),
        //                                         0)                                                                           AS conversions_30,
        //                                COALESCE(COUNT(s.activated_at)
        //                                         FILTER (WHERE s.activated_at - s.trial_start_date <= INTERVAL '90 days'),
        //                                         0)                                                                           AS conversions_90,
        //                                COALESCE(COUNT(s.activated_at), 0)                                                    AS conversions
        //                         FROM month_series ms
        //                                  LEFT JOIN subscription s ON DATE_TRUNC('month', s.trial_start_date) = ms.month
        //                             AND s.tenant_id = $2
        //                         GROUP BY ms.month
        //                         ORDER BY ms.month)
        // SELECT month,
        //        total_trials,
        //        conversions,
        //        CASE
        //            WHEN total_trials > 0 THEN ROUND((conversions::DECIMAL / total_trials) * 100, 2)
        //            ELSE 0 END                                                                                      AS conversion_rate_percentage,
        //        conversions_30,
        //        CASE
        //            WHEN total_trials > 0 THEN ROUND((conversions_30::DECIMAL / total_trials) * 100, 2)
        //            ELSE 0 END                                                                                      AS conversion_rate_30_percentage,
        //        conversions_90,
        //        CASE
        //            WHEN total_trials > 0 THEN ROUND((conversions_90::DECIMAL / total_trials) * 100, 2)
        //            ELSE 0 END                                                                                      AS conversion_rate_90_percentage
        // FROM monthly_trials;
        //         "#;
        //
        //         diesel::sql_query(raw_sql)
        //             .bind::<sql_types::Uuid, _>(tenant_id)
        //             .bind::<sql_types::Uuid, _>(tenant_id)
        //             .get_results::<SubscriptionTrialToPaidConversionRow>(conn)
        //             .await
        //             .attach("Error while fetching subscription trial to paid conversion")
        //             .into_db_result()
        Ok(vec![])
    }
}

impl CustomerTopRevenueRow {
    pub async fn list(
        conn: &mut PgConn,
        tenant_id: TenantId,
        currency: &str,
        limit: i32,
    ) -> DbResult<Vec<CustomerTopRevenueRow>> {
        let raw_sql = r"
        SELECT c.id,
        c.name,
        COALESCE(bi.total_revenue_cents, 0)::bigint AS total_revenue_cents,
        $1                                  AS currency
        FROM customer c
                 LEFT JOIN bi_customer_ytd_summary bi ON bi.customer_id = c.id
        WHERE c.tenant_id = $2
          AND (bi.revenue_year IS NULL OR bi.currency = $3)
          AND (bi.revenue_year IS NULL OR bi.revenue_year = DATE_PART('year', CURRENT_DATE))
        ORDER BY total_revenue_cents DESC
        LIMIT $4;
        ";

        diesel::sql_query(raw_sql)
            .bind::<sql_types::Text, _>(currency)
            .bind::<sql_types::Uuid, _>(tenant_id)
            .bind::<sql_types::Text, _>(currency)
            .bind::<sql_types::Integer, _>(limit)
            .get_results::<CustomerTopRevenueRow>(conn)
            .await
            .attach("Error while fetching customer top revenue")
            .into_db_result()
    }
}

impl TotalMrrRow {
    pub async fn get(
        conn: &mut PgConn,
        tenant_id: TenantId,
        date: NaiveDate,
    ) -> DbResult<TotalMrrRow> {
        let raw_sql = r"
        SELECT
            COALESCE(
                SUM(
                    CASE
                        WHEN bd.currency = t.reporting_currency
                        THEN bd.net_mrr_cents
                        ELSE bd.net_mrr_cents_usd * (hr.rates->>t.reporting_currency)::NUMERIC
                    END
                ),
                0
            )::bigint AS total_net_mrr_cents
       FROM
           bi_delta_mrr_daily bd
           JOIN historical_rates_from_usd hr ON bd.historical_rate_id = hr.id
           JOIN tenant t ON bd.tenant_id = t.id
       WHERE
           bd.tenant_id = $1
           AND bd.date <= $2;
       ";

        diesel::sql_query(raw_sql)
            .bind::<sql_types::Uuid, _>(tenant_id)
            .bind::<sql_types::Date, _>(date)
            .get_result::<TotalMrrRow>(conn)
            .await
            .attach("Error while fetching total mrr")
            .into_db_result()
    }
}

impl TotalMrrChartRow {
    pub async fn list(
        conn: &mut PgConn,
        tenant_id: TenantId,
        start_date: NaiveDate,
        end_date: NaiveDate,
    ) -> DbResult<Vec<TotalMrrChartRow>> {
        let raw_sql = r"
        WITH conversion_rates AS (
            SELECT
                id,
                (rates->>(SELECT reporting_currency FROM tenant WHERE id = $1))::NUMERIC AS conversion_rate
            FROM
                historical_rates_from_usd
        ),
        data_bounds AS (
            SELECT COALESCE(MIN(date), $2::date) AS min_date
            FROM bi_delta_mrr_daily
            WHERE tenant_id = $3
        ),
        effective_start AS (
            SELECT GREATEST($4::date, (SELECT min_date - INTERVAL '1 month' FROM data_bounds)::date) AS start_date
        ),
        initial_mrr AS (
            SELECT
                COALESCE(FLOOR(SUM(bd.net_mrr_cents_usd * cr.conversion_rate)), 0)::BIGINT AS total_net_mrr_cents
            FROM
                bi_delta_mrr_daily bd
                    JOIN
                conversion_rates cr ON bd.historical_rate_id = cr.id
            WHERE
                bd.date < (SELECT start_date FROM effective_start)
              AND bd.tenant_id = $5
        ),
        date_series AS (
            SELECT generate_series((SELECT start_date FROM effective_start), $6::date, '1 day'::interval)::date AS period
        ),
        daily_mrr AS (
            SELECT
                bi.date AS period,
                FLOOR(SUM(bi.net_mrr_cents_usd) * MAX(cr.conversion_rate))::BIGINT AS net_new_mrr,
                FLOOR(SUM(bi.new_business_cents_usd) * MAX(cr.conversion_rate))::BIGINT AS new_business_mrr,
                SUM(bi.new_business_count)::INTEGER AS new_business_count,
                FLOOR(SUM(bi.expansion_cents_usd) * MAX(cr.conversion_rate))::BIGINT AS expansion_mrr,
                SUM(bi.expansion_count)::INTEGER AS expansion_count,
                FLOOR(SUM(bi.contraction_cents_usd) * MAX(cr.conversion_rate))::BIGINT AS contraction_mrr,
                SUM(bi.contraction_count)::INTEGER AS contraction_count,
                FLOOR(SUM(bi.churn_cents_usd) * MAX(cr.conversion_rate))::BIGINT AS churn_mrr,
                SUM(bi.churn_count)::INTEGER AS churn_count,
                FLOOR(SUM(bi.reactivation_cents_usd) * MAX(cr.conversion_rate))::BIGINT AS reactivation_mrr,
                SUM(bi.reactivation_count)::INTEGER AS reactivation_count
            FROM
                bi_delta_mrr_daily bi
                    JOIN
                conversion_rates cr ON bi.historical_rate_id = cr.id
            WHERE
                bi.date BETWEEN (SELECT start_date FROM effective_start) AND $7
              AND bi.tenant_id = $8
            GROUP BY bi.date
        ),
        filled_mrr AS (
            SELECT
                ds.period,
                COALESCE(dm.net_new_mrr, 0) AS net_new_mrr,
                COALESCE(dm.new_business_mrr, 0) AS new_business_mrr,
                COALESCE(dm.new_business_count, 0) AS new_business_count,
                COALESCE(dm.expansion_mrr, 0) AS expansion_mrr,
                COALESCE(dm.expansion_count, 0) AS expansion_count,
                COALESCE(dm.contraction_mrr, 0) AS contraction_mrr,
                COALESCE(dm.contraction_count, 0) AS contraction_count,
                COALESCE(dm.churn_mrr, 0) AS churn_mrr,
                COALESCE(dm.churn_count, 0) AS churn_count,
                COALESCE(dm.reactivation_mrr, 0) AS reactivation_mrr,
                COALESCE(dm.reactivation_count, 0) AS reactivation_count
            FROM
                date_series ds
                    LEFT JOIN daily_mrr dm ON ds.period = dm.period
        )
        SELECT
            fm.period,
            (im.total_net_mrr_cents + COALESCE(SUM(fm.net_new_mrr) OVER (ORDER BY fm.period), 0))::BIGINT AS total_net_mrr,
            fm.net_new_mrr,
            fm.new_business_mrr,
            fm.new_business_count::INTEGER,
            fm.expansion_mrr,
            fm.expansion_count::INTEGER,
            fm.contraction_mrr,
            fm.contraction_count::INTEGER,
            fm.churn_mrr,
            fm.churn_count::INTEGER,
            fm.reactivation_mrr,
            fm.reactivation_count::INTEGER
        FROM
            filled_mrr fm
                CROSS JOIN
            initial_mrr im
        ORDER BY fm.period";

        let query = diesel::sql_query(raw_sql)
            .bind::<sql_types::Uuid, _>(tenant_id)   // $1: conversion_rates
            .bind::<sql_types::Date, _>(start_date)  // $2: data_bounds fallback
            .bind::<sql_types::Uuid, _>(tenant_id)   // $3: data_bounds
            .bind::<sql_types::Date, _>(start_date)  // $4: effective_start GREATEST
            .bind::<sql_types::Uuid, _>(tenant_id)   // $5: initial_mrr
            .bind::<sql_types::Date, _>(end_date)    // $6: date_series end
            .bind::<sql_types::Date, _>(end_date)    // $7: daily_mrr end
            .bind::<sql_types::Uuid, _>(tenant_id);  // $8: daily_mrr

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results::<TotalMrrChartRow>(conn)
            .await
            .map_err(|e| {
                log::error!("Error while fetching total mrr: {e:?}");
                e
            })
            .attach("Error while fetching total mrr")
            .into_db_result()
    }
}

impl TotalMrrByPlanRow {
    pub async fn list(
        conn: &mut PgConn,
        tenant_id: TenantId,
        plan_ids: &Vec<Uuid>,
        start_date: NaiveDate,
        end_date: NaiveDate,
    ) -> DbResult<Vec<TotalMrrByPlanRow>> {
        let raw_sql = r"
        WITH conversion_rates AS (
            SELECT
                id,
                (rates->>(SELECT reporting_currency FROM tenant WHERE id = $1))::NUMERIC AS conversion_rate
            FROM
                historical_rates_from_usd
        ),
        selected_plans AS (
            SELECT id, name FROM plan WHERE id = ANY($2)
        ),
        data_bounds AS (
            SELECT COALESCE(MIN(date), $3::date) AS min_date
            FROM bi_delta_mrr_daily
            WHERE tenant_id = $4
        ),
        effective_start AS (
            SELECT GREATEST($5::date, (SELECT min_date - INTERVAL '1 month' FROM data_bounds)::date) AS start_date
        ),
        initial_mrr AS (
            SELECT
                COALESCE(FLOOR(SUM(bi.net_mrr_cents_usd * cr.conversion_rate)), 0)::BIGINT AS total_net_mrr_usd,
                pv.plan_id
            FROM
                bi_delta_mrr_daily bi
                    JOIN
                plan_version pv ON bi.plan_version_id = pv.id
                    JOIN
                conversion_rates cr ON bi.historical_rate_id = cr.id
            WHERE
                bi.date < (SELECT start_date FROM effective_start)
                AND bi.tenant_id = $6
                AND pv.plan_id = ANY ($7)
            GROUP BY
                pv.plan_id
        ),
        date_series AS (
            SELECT generate_series((SELECT start_date FROM effective_start), $8::date, '1 day'::interval)::date AS period
        ),
        date_plan_series AS (
            SELECT ds.period, sp.id AS plan_id, sp.name AS plan_name
            FROM date_series ds
            CROSS JOIN selected_plans sp
        ),
        daily_mrr AS (
            SELECT
                bi.date AS period,
                pv.plan_id,
                FLOOR(SUM(bi.net_mrr_cents_usd) * MAX(cr.conversion_rate))::BIGINT AS net_new_mrr,
                FLOOR(SUM(bi.new_business_cents_usd) * MAX(cr.conversion_rate))::BIGINT AS new_business_mrr,
                SUM(bi.new_business_count)::INTEGER AS new_business_count,
                FLOOR(SUM(bi.expansion_cents_usd) * MAX(cr.conversion_rate))::BIGINT AS expansion_mrr,
                SUM(bi.expansion_count)::INTEGER AS expansion_count,
                FLOOR(SUM(bi.contraction_cents_usd) * MAX(cr.conversion_rate))::BIGINT AS contraction_mrr,
                SUM(bi.contraction_count)::INTEGER AS contraction_count,
                FLOOR(SUM(bi.churn_cents_usd) * MAX(cr.conversion_rate))::BIGINT AS churn_mrr,
                SUM(bi.churn_count)::INTEGER AS churn_count,
                FLOOR(SUM(bi.reactivation_cents_usd) * MAX(cr.conversion_rate))::BIGINT AS reactivation_mrr,
                SUM(bi.reactivation_count)::INTEGER AS reactivation_count
            FROM
                bi_delta_mrr_daily bi
                    JOIN plan_version pv ON bi.plan_version_id = pv.id
                    JOIN conversion_rates cr ON bi.historical_rate_id = cr.id
            WHERE
                bi.date BETWEEN (SELECT start_date FROM effective_start) AND $9
              AND bi.tenant_id = $10
              AND pv.plan_id = ANY($11)
            GROUP BY bi.date, pv.plan_id
        ),
        filled_mrr AS (
            SELECT
                dps.period,
                dps.plan_id,
                dps.plan_name,
                COALESCE(dm.net_new_mrr, 0) AS net_new_mrr,
                COALESCE(dm.new_business_mrr, 0) AS new_business_mrr,
                COALESCE(dm.new_business_count, 0) AS new_business_count,
                COALESCE(dm.expansion_mrr, 0) AS expansion_mrr,
                COALESCE(dm.expansion_count, 0) AS expansion_count,
                COALESCE(dm.contraction_mrr, 0) AS contraction_mrr,
                COALESCE(dm.contraction_count, 0) AS contraction_count,
                COALESCE(dm.churn_mrr, 0) AS churn_mrr,
                COALESCE(dm.churn_count, 0) AS churn_count,
                COALESCE(dm.reactivation_mrr, 0) AS reactivation_mrr,
                COALESCE(dm.reactivation_count, 0) AS reactivation_count
            FROM
                date_plan_series dps
                    LEFT JOIN daily_mrr dm ON dps.period = dm.period AND dps.plan_id = dm.plan_id
        )
        SELECT
            fm.period AS date,
            fm.plan_id,
            fm.plan_name,
            (COALESCE(im.total_net_mrr_usd, 0) + COALESCE(SUM(fm.net_new_mrr) OVER (PARTITION BY fm.plan_id ORDER BY fm.period), 0))::BIGINT AS total_net_mrr,
            fm.net_new_mrr,
            fm.new_business_mrr,
            fm.new_business_count::INTEGER,
            fm.expansion_mrr,
            fm.expansion_count::INTEGER,
            fm.contraction_mrr,
            fm.contraction_count::INTEGER,
            fm.churn_mrr,
            fm.churn_count::INTEGER,
            fm.reactivation_mrr,
            fm.reactivation_count::INTEGER
        FROM
            filled_mrr fm
                LEFT JOIN initial_mrr im ON fm.plan_id = im.plan_id
        ORDER BY fm.period, fm.plan_id";

        let query = diesel::sql_query(raw_sql)
            .bind::<sql_types::Uuid, _>(tenant_id)        // $1: conversion_rates
            .bind::<sql_types::Array<sql_types::Uuid>, _>(plan_ids) // $2: selected_plans
            .bind::<sql_types::Date, _>(start_date)       // $3: data_bounds fallback
            .bind::<sql_types::Uuid, _>(tenant_id)        // $4: data_bounds
            .bind::<sql_types::Date, _>(start_date)       // $5: effective_start GREATEST
            .bind::<sql_types::Uuid, _>(tenant_id)        // $6: initial_mrr
            .bind::<sql_types::Array<sql_types::Uuid>, _>(plan_ids) // $7: initial_mrr plan filter
            .bind::<sql_types::Date, _>(end_date)         // $8: date_series end
            .bind::<sql_types::Date, _>(end_date)         // $9: daily_mrr end
            .bind::<sql_types::Uuid, _>(tenant_id)        // $10: daily_mrr
            .bind::<sql_types::Array<sql_types::Uuid>, _>(plan_ids); // $11: daily_mrr plan filter

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results::<TotalMrrByPlanRow>(conn)
            .await
            .attach("Error while fetching total mrr by plan")
            .into_db_result()
    }
}

impl MrrBreakdownRow {
    pub async fn get(
        conn: &mut PgConn,
        tenant_id: TenantId,
        start_date: NaiveDate,
        end_date: NaiveDate,
    ) -> DbResult<Option<MrrBreakdownRow>> {
        let raw_sql = r"
        WITH conversion_rates AS (
            SELECT
                id,
                (rates->>(SELECT reporting_currency FROM tenant WHERE id = $1))::NUMERIC AS rate
            FROM
                historical_rates_from_usd
        )
        SELECT
            COALESCE(SUM(bi.net_mrr_cents_usd * cr.rate), 0)::BIGINT AS net_new_mrr,
            COALESCE(SUM(bi.new_business_cents_usd * cr.rate), 0)::BIGINT AS new_business_mrr,
            COALESCE(SUM(bi.new_business_count), 0)::INTEGER AS new_business_count,
            COALESCE(SUM(bi.expansion_cents_usd * cr.rate), 0)::BIGINT AS expansion_mrr,
            COALESCE(SUM(bi.expansion_count), 0)::INTEGER AS expansion_count,
            COALESCE(SUM(bi.contraction_cents_usd * cr.rate), 0)::BIGINT AS contraction_mrr,
            COALESCE(SUM(bi.contraction_count), 0)::INTEGER AS contraction_count,
            COALESCE(SUM(bi.churn_cents_usd * cr.rate), 0)::BIGINT AS churn_mrr,
            COALESCE(SUM(bi.churn_count), 0)::INTEGER AS churn_count,
            COALESCE(SUM(bi.reactivation_cents_usd * cr.rate), 0)::BIGINT AS reactivation_mrr,
            COALESCE(SUM(bi.reactivation_count), 0)::INTEGER AS reactivation_count
        FROM
            bi_delta_mrr_daily bi
                JOIN conversion_rates cr ON bi.historical_rate_id = cr.id
        WHERE
            bi.date BETWEEN $2 AND $3
          AND bi.tenant_id = $4
        GROUP BY
            bi.tenant_id;
        ";

        diesel::sql_query(raw_sql)
            .bind::<sql_types::Uuid, _>(tenant_id)
            .bind::<sql_types::Date, _>(start_date)
            .bind::<sql_types::Date, _>(end_date)
            .bind::<sql_types::Uuid, _>(tenant_id)
            .get_result::<MrrBreakdownRow>(conn)
            .await
            .optional()
            .attach("Error while fetching mrr breakdown")
            .into_db_result()
    }
}

impl LastMrrMovementsRow {
    pub async fn list(
        conn: &mut PgConn,
        tenant_id: TenantId,
        before: Option<Uuid>,
        after: Option<Uuid>,
        limit: i32,
    ) -> DbResult<Vec<LastMrrMovementsRow>> {
        let raw_sql = r"
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
        WHERE bi.tenant_id = $1
          AND (bi.id < $2 OR $3 IS NULL)
          AND (bi.id > $4 OR $5 IS NULL)
        ORDER BY bi.id DESC
        LIMIT $6;
        ";

        let query = diesel::sql_query(raw_sql)
            .bind::<sql_types::Uuid, _>(tenant_id)
            .bind::<sql_types::Nullable<sql_types::Uuid>, _>(before)
            .bind::<sql_types::Nullable<sql_types::Uuid>, _>(before)
            .bind::<sql_types::Nullable<sql_types::Uuid>, _>(after)
            .bind::<sql_types::Nullable<sql_types::Uuid>, _>(after)
            .bind::<sql_types::Integer, _>(limit);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results::<LastMrrMovementsRow>(conn)
            .await
            .attach("Error while fetching last mrr movements")
            .into_db_result()
    }
}

impl RevenueChartRow {
    pub async fn list(
        conn: &mut PgConn,
        tenant_id: TenantId,
        start_date: NaiveDate,
        end_date: NaiveDate,
        plan_ids: Option<&Vec<Uuid>>,
    ) -> DbResult<Vec<RevenueChartRow>> {
        // If plan_ids is provided and not empty, filter by plans
        let has_plan_filter = plan_ids.map(|p| !p.is_empty()).unwrap_or(false);

        if has_plan_filter {
            let plan_ids = plan_ids.unwrap();
            let raw_sql = r"
            WITH conversion_rates AS (
                SELECT
                    id,
                    (rates->>(SELECT reporting_currency FROM tenant WHERE id = $1))::NUMERIC AS conversion_rate
                FROM
                    historical_rates_from_usd
            ),
            data_bounds AS (
                SELECT COALESCE(MIN(revenue_date), $2::date) AS min_date
                FROM bi_revenue_daily
                WHERE tenant_id = $3
            ),
            effective_start AS (
                SELECT GREATEST($4::date, (SELECT min_date - INTERVAL '1 month' FROM data_bounds)::date) AS start_date
            ),
            initial_revenue AS (
                SELECT
                    COALESCE(ROUND(SUM(bi.net_revenue_cents_usd * cr.conversion_rate)), 0)::BIGINT AS total_revenue
                FROM
                    bi_revenue_daily bi
                        JOIN conversion_rates cr ON bi.historical_rate_id = cr.id
                        LEFT JOIN plan_version pv ON bi.plan_version_id = pv.id
                WHERE
                    bi.revenue_date < (SELECT start_date FROM effective_start)
                  AND bi.tenant_id = $5
                  AND (bi.plan_version_id IS NULL OR pv.plan_id = ANY($9))
            ),
            date_series AS (
                SELECT generate_series((SELECT start_date FROM effective_start), $6::date, '1 day'::interval)::date AS period
            ),
            daily_revenue AS (
                SELECT
                    bi.revenue_date AS period,
                    COALESCE(ROUND(SUM(bi.net_revenue_cents_usd * cr.conversion_rate)), 0)::BIGINT AS daily_revenue
                FROM
                    bi_revenue_daily bi
                        JOIN conversion_rates cr ON bi.historical_rate_id = cr.id
                        LEFT JOIN plan_version pv ON bi.plan_version_id = pv.id
                WHERE
                    bi.revenue_date BETWEEN (SELECT start_date FROM effective_start) AND $7
                  AND bi.tenant_id = $8
                  AND (bi.plan_version_id IS NULL OR pv.plan_id = ANY($10))
                GROUP BY bi.revenue_date
            ),
            filled_revenue AS (
                SELECT
                    ds.period,
                    COALESCE(dr.daily_revenue, 0) AS daily_revenue
                FROM
                    date_series ds
                        LEFT JOIN daily_revenue dr ON ds.period = dr.period
            )
            SELECT
                fr.period,
                (ir.total_revenue + COALESCE(SUM(fr.daily_revenue) OVER (ORDER BY fr.period), 0))::BIGINT AS total_revenue
            FROM
                filled_revenue fr
                    CROSS JOIN
                initial_revenue ir
            ORDER BY fr.period";

            let query = diesel::sql_query(raw_sql)
                .bind::<sql_types::Uuid, _>(tenant_id)   // $1: conversion_rates
                .bind::<sql_types::Date, _>(start_date)  // $2: data_bounds fallback
                .bind::<sql_types::Uuid, _>(tenant_id)   // $3: data_bounds
                .bind::<sql_types::Date, _>(start_date)  // $4: effective_start GREATEST
                .bind::<sql_types::Uuid, _>(tenant_id)   // $5: initial_revenue
                .bind::<sql_types::Date, _>(end_date)    // $6: date_series end
                .bind::<sql_types::Date, _>(end_date)    // $7: daily_revenue end
                .bind::<sql_types::Uuid, _>(tenant_id)   // $8: daily_revenue
                .bind::<sql_types::Array<sql_types::Uuid>, _>(plan_ids)  // $9: initial_revenue plan filter
                .bind::<sql_types::Array<sql_types::Uuid>, _>(plan_ids); // $10: daily_revenue plan filter

            log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

            query
                .get_results::<RevenueChartRow>(conn)
                .await
                .map_err(|e| {
                    log::error!("Error while fetching revenue chart: {e:?}");
                    e
                })
                .attach("Error while fetching revenue chart")
                .into_db_result()
        } else {
            let raw_sql = r"
            WITH conversion_rates AS (
                SELECT
                    id,
                    (rates->>(SELECT reporting_currency FROM tenant WHERE id = $1))::NUMERIC AS conversion_rate
                FROM
                    historical_rates_from_usd
            ),
            data_bounds AS (
                SELECT COALESCE(MIN(revenue_date), $2::date) AS min_date
                FROM bi_revenue_daily
                WHERE tenant_id = $3
            ),
            effective_start AS (
                SELECT GREATEST($4::date, (SELECT min_date - INTERVAL '1 month' FROM data_bounds)::date) AS start_date
            ),
            initial_revenue AS (
                SELECT
                    COALESCE(ROUND(SUM(bi.net_revenue_cents_usd * cr.conversion_rate)), 0)::BIGINT AS total_revenue
                FROM
                    bi_revenue_daily bi
                        JOIN
                    conversion_rates cr ON bi.historical_rate_id = cr.id
                WHERE
                    bi.revenue_date < (SELECT start_date FROM effective_start)
                  AND bi.tenant_id = $5
            ),
            date_series AS (
                SELECT generate_series((SELECT start_date FROM effective_start), $6::date, '1 day'::interval)::date AS period
            ),
            daily_revenue AS (
                SELECT
                    bi.revenue_date AS period,
                    COALESCE(ROUND(SUM(bi.net_revenue_cents_usd * cr.conversion_rate)), 0)::BIGINT AS daily_revenue
                FROM
                    bi_revenue_daily bi
                        JOIN
                    conversion_rates cr ON bi.historical_rate_id = cr.id
                WHERE
                    bi.revenue_date BETWEEN (SELECT start_date FROM effective_start) AND $7
                  AND bi.tenant_id = $8
                GROUP BY bi.revenue_date
            ),
            filled_revenue AS (
                SELECT
                    ds.period,
                    COALESCE(dr.daily_revenue, 0) AS daily_revenue
                FROM
                    date_series ds
                        LEFT JOIN daily_revenue dr ON ds.period = dr.period
            )
            SELECT
                fr.period,
                (ir.total_revenue + COALESCE(SUM(fr.daily_revenue) OVER (ORDER BY fr.period), 0))::BIGINT AS total_revenue
            FROM
                filled_revenue fr
                    CROSS JOIN
                initial_revenue ir
            ORDER BY fr.period";

            let query = diesel::sql_query(raw_sql)
                .bind::<sql_types::Uuid, _>(tenant_id)   // $1: conversion_rates
                .bind::<sql_types::Date, _>(start_date)  // $2: data_bounds fallback
                .bind::<sql_types::Uuid, _>(tenant_id)   // $3: data_bounds
                .bind::<sql_types::Date, _>(start_date)  // $4: effective_start GREATEST
                .bind::<sql_types::Uuid, _>(tenant_id)   // $5: initial_revenue
                .bind::<sql_types::Date, _>(end_date)    // $6: date_series end
                .bind::<sql_types::Date, _>(end_date)    // $7: daily_revenue end
                .bind::<sql_types::Uuid, _>(tenant_id);  // $8: daily_revenue

            log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

            query
                .get_results::<RevenueChartRow>(conn)
                .await
                .map_err(|e| {
                    log::error!("Error while fetching revenue chart: {e:?}");
                    e
                })
                .attach("Error while fetching revenue chart")
                .into_db_result()
        }
    }
}
