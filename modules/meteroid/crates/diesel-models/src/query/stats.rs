use crate::stats::{
    ActiveSubscriptionsCountRow, DailyNewSignups90DaysRow, NewSignupsTrend90DaysRow,
    PendingInvoicesByCurrencyRow, SubscriptionTrialConversionRateRow,
    SubscriptionTrialToPaidConversionRow,
};
use crate::{DbResult, PgConn};
use diesel::{ExpressionMethods, QueryDsl, sql_types};
use diesel_async::RunQueryDsl;

use crate::errors::IntoDbResult;
use common_domain::ids::TenantId;
use error_stack::ResultExt;

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

impl PendingInvoicesByCurrencyRow {
    pub async fn list(
        conn: &mut PgConn,
        tenant_id: TenantId,
    ) -> DbResult<Vec<PendingInvoicesByCurrencyRow>> {
        let raw_sql = r"
        SELECT
            i.currency AS currency,
            COUNT(*)::bigint AS count,
            COALESCE(SUM(i.total), 0)::bigint AS total
        FROM invoice i
        WHERE i.tenant_id = $1
          AND i.status = 'FINALIZED'
          AND i.paid_at IS NULL
        GROUP BY i.currency;
        ";

        diesel::sql_query(raw_sql)
            .bind::<sql_types::Uuid, _>(tenant_id)
            .get_results::<PendingInvoicesByCurrencyRow>(conn)
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
    ) -> DbResult<ActiveSubscriptionsCountRow> {
        use crate::enums::SubscriptionStatusEnum;
        use crate::schema::subscription::dsl as s_dsl;

        // Mirrors the subscriptions list "active" default (pending + trialing +
        // active) so this card matches its drill-down. The legacy end_date
        // column is never populated, so status is the source of truth.
        let active_statuses = vec![
            SubscriptionStatusEnum::PendingActivation,
            SubscriptionStatusEnum::PendingCharge,
            SubscriptionStatusEnum::TrialActive,
            SubscriptionStatusEnum::Active,
        ];

        let query = s_dsl::subscription
            .filter(s_dsl::tenant_id.eq(tenant_id))
            .filter(s_dsl::status.eq_any(active_statuses))
            .count();

        crate::debug_sql(&query);

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
