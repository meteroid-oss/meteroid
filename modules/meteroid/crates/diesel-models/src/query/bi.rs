use crate::bi::{
    BiCustomerYtdSummaryRow, BiDeltaMrrDailyRow, BiMrrMovementLogRow, BiMrrMovementLogRowNew,
    BiRevenueDailyRow,
};
use crate::enums::MrrMovementType;
use crate::errors::IntoDbResult;

use crate::{DbResult, PgConn};

use chrono::NaiveDate;
use diesel::debug_query;
use diesel::upsert::excluded;
use diesel::{ExpressionMethods, QueryDsl};
use error_stack::ResultExt;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use uuid::Uuid;

impl BiMrrMovementLogRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<BiMrrMovementLogRow> {
        use crate::schema::bi_mrr_movement_log::dsl::bi_mrr_movement_log;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(bi_mrr_movement_log).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while inserting bi_mrr_movement_log")
            .into_db_result()
    }
}
impl BiMrrMovementLogRow {
    pub async fn insert_movement_log_batch(
        conn: &mut PgConn,
        invoices: Vec<BiMrrMovementLogRowNew>,
    ) -> DbResult<Vec<BiMrrMovementLogRow>> {
        use crate::schema::bi_mrr_movement_log::dsl::bi_mrr_movement_log;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(bi_mrr_movement_log).values(&invoices);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while inserting bi_mrr_movement_log")
            .into_db_result()
    }
}

impl BiRevenueDailyRow {
    /// Upsert a revenue record, adding to existing values on conflict
    pub async fn upsert(conn: &mut PgConn, row: BiRevenueDailyRow) -> DbResult<()> {
        use crate::schema::bi_revenue_daily::dsl as r_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(r_dsl::bi_revenue_daily)
            .values(&row)
            .on_conflict((
                r_dsl::tenant_id,
                r_dsl::plan_version_id,
                r_dsl::currency,
                r_dsl::revenue_date,
            ))
            .do_update()
            .set((
                r_dsl::net_revenue_cents
                    .eq(r_dsl::net_revenue_cents + excluded(r_dsl::net_revenue_cents)),
                r_dsl::net_revenue_cents_usd
                    .eq(r_dsl::net_revenue_cents_usd + excluded(r_dsl::net_revenue_cents_usd)),
                r_dsl::historical_rate_id.eq(excluded(r_dsl::historical_rate_id)),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .map(drop)
            .attach("Error while upserting bi_revenue_daily")
            .into_db_result()
    }
}

impl BiCustomerYtdSummaryRow {
    /// Upsert a customer YTD summary record, adding to existing total on conflict
    pub async fn upsert(conn: &mut PgConn, row: BiCustomerYtdSummaryRow) -> DbResult<()> {
        use crate::schema::bi_customer_ytd_summary::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(c_dsl::bi_customer_ytd_summary)
            .values(&row)
            .on_conflict((
                c_dsl::tenant_id,
                c_dsl::customer_id,
                c_dsl::currency,
                c_dsl::revenue_year,
            ))
            .do_update()
            .set(
                c_dsl::total_revenue_cents
                    .eq(c_dsl::total_revenue_cents + excluded(c_dsl::total_revenue_cents)),
            );

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .map(drop)
            .attach("Error while upserting bi_customer_ytd_summary")
            .into_db_result()
    }
}

/// Input for upserting MRR daily aggregates
pub struct MrrDailyUpsertInput {
    pub tenant_id: Uuid,
    pub plan_version_id: Uuid,
    pub date: NaiveDate,
    pub currency: String,
    pub movement_type: MrrMovementType,
    pub net_mrr_change: i64,
    pub net_mrr_change_usd: Decimal,
    pub historical_rate_id: Uuid,
}

impl BiDeltaMrrDailyRow {
    /// Upsert an MRR daily record, incrementing the appropriate movement type columns
    pub async fn upsert_mrr_movement(
        conn: &mut PgConn,
        input: MrrDailyUpsertInput,
    ) -> DbResult<()> {
        use crate::schema::bi_delta_mrr_daily::dsl as m_dsl;
        use diesel_async::RunQueryDsl;

        // Build the base row with zeros for all movement types
        let base_row = BiDeltaMrrDailyRow {
            tenant_id: input.tenant_id.into(),
            plan_version_id: input.plan_version_id.into(),
            date: input.date,
            currency: input.currency,
            net_mrr_cents: input.net_mrr_change,
            new_business_cents: 0,
            new_business_count: 0,
            expansion_cents: 0,
            expansion_count: 0,
            contraction_cents: 0,
            contraction_count: 0,
            churn_cents: 0,
            churn_count: 0,
            reactivation_cents: 0,
            reactivation_count: 0,
            historical_rate_id: input.historical_rate_id,
            net_mrr_cents_usd: input.net_mrr_change_usd,
            new_business_cents_usd: Decimal::ZERO,
            expansion_cents_usd: Decimal::ZERO,
            contraction_cents_usd: Decimal::ZERO,
            churn_cents_usd: Decimal::ZERO,
            reactivation_cents_usd: Decimal::ZERO,
        };

        // Set the appropriate movement type values
        let row = match input.movement_type {
            MrrMovementType::NewBusiness => BiDeltaMrrDailyRow {
                new_business_cents: input.net_mrr_change,
                new_business_count: 1,
                new_business_cents_usd: input.net_mrr_change_usd,
                ..base_row
            },
            MrrMovementType::Expansion => BiDeltaMrrDailyRow {
                expansion_cents: input.net_mrr_change,
                expansion_count: 1,
                expansion_cents_usd: input.net_mrr_change_usd,
                ..base_row
            },
            MrrMovementType::Contraction => BiDeltaMrrDailyRow {
                contraction_cents: input.net_mrr_change,
                contraction_count: 1,
                contraction_cents_usd: input.net_mrr_change_usd,
                ..base_row
            },
            MrrMovementType::Churn => BiDeltaMrrDailyRow {
                churn_cents: input.net_mrr_change,
                churn_count: 1,
                churn_cents_usd: input.net_mrr_change_usd,
                ..base_row
            },
            MrrMovementType::Reactivation => BiDeltaMrrDailyRow {
                reactivation_cents: input.net_mrr_change,
                reactivation_count: 1,
                reactivation_cents_usd: input.net_mrr_change_usd,
                ..base_row
            },
        };

        let query = diesel::insert_into(m_dsl::bi_delta_mrr_daily)
            .values(&row)
            .on_conflict((
                m_dsl::tenant_id,
                m_dsl::plan_version_id,
                m_dsl::currency,
                m_dsl::date,
            ))
            .do_update()
            .set((
                m_dsl::net_mrr_cents.eq(m_dsl::net_mrr_cents + excluded(m_dsl::net_mrr_cents)),
                m_dsl::net_mrr_cents_usd
                    .eq(m_dsl::net_mrr_cents_usd + excluded(m_dsl::net_mrr_cents_usd)),
                m_dsl::new_business_cents
                    .eq(m_dsl::new_business_cents + excluded(m_dsl::new_business_cents)),
                m_dsl::new_business_count
                    .eq(m_dsl::new_business_count + excluded(m_dsl::new_business_count)),
                m_dsl::new_business_cents_usd
                    .eq(m_dsl::new_business_cents_usd + excluded(m_dsl::new_business_cents_usd)),
                m_dsl::expansion_cents
                    .eq(m_dsl::expansion_cents + excluded(m_dsl::expansion_cents)),
                m_dsl::expansion_count
                    .eq(m_dsl::expansion_count + excluded(m_dsl::expansion_count)),
                m_dsl::expansion_cents_usd
                    .eq(m_dsl::expansion_cents_usd + excluded(m_dsl::expansion_cents_usd)),
                m_dsl::contraction_cents
                    .eq(m_dsl::contraction_cents + excluded(m_dsl::contraction_cents)),
                m_dsl::contraction_count
                    .eq(m_dsl::contraction_count + excluded(m_dsl::contraction_count)),
                m_dsl::contraction_cents_usd
                    .eq(m_dsl::contraction_cents_usd + excluded(m_dsl::contraction_cents_usd)),
                m_dsl::churn_cents.eq(m_dsl::churn_cents + excluded(m_dsl::churn_cents)),
                m_dsl::churn_count.eq(m_dsl::churn_count + excluded(m_dsl::churn_count)),
                m_dsl::churn_cents_usd
                    .eq(m_dsl::churn_cents_usd + excluded(m_dsl::churn_cents_usd)),
                m_dsl::reactivation_cents
                    .eq(m_dsl::reactivation_cents + excluded(m_dsl::reactivation_cents)),
                m_dsl::reactivation_count
                    .eq(m_dsl::reactivation_count + excluded(m_dsl::reactivation_count)),
                m_dsl::reactivation_cents_usd
                    .eq(m_dsl::reactivation_cents_usd + excluded(m_dsl::reactivation_cents_usd)),
                m_dsl::historical_rate_id.eq(excluded(m_dsl::historical_rate_id)),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .map(drop)
            .attach("Error while upserting bi_delta_mrr_daily")
            .into_db_result()
    }

    /// Update USD values for all rows on a specific date (called when new rates are available)
    pub async fn update_usd_values_for_date(
        conn: &mut PgConn,
        date: NaiveDate,
        rates: &std::collections::BTreeMap<String, f32>,
        historical_rate_id: Uuid,
    ) -> DbResult<usize> {
        use crate::schema::bi_delta_mrr_daily::dsl as m_dsl;
        use diesel_async::RunQueryDsl;

        let rows = m_dsl::bi_delta_mrr_daily
            .filter(m_dsl::date.eq(date))
            .load::<BiDeltaMrrDailyRow>(conn)
            .await
            .attach("Error loading bi_delta_mrr_daily for date")
            .into_db_result()?;

        let mut updated = 0;
        for row in rows {
            if let Some(rate) = rates.get(&row.currency) {
                if *rate == 0.0 {
                    continue;
                }
                let rate_decimal = Decimal::from_f32(*rate).unwrap_or(Decimal::ONE);

                let query = diesel::update(m_dsl::bi_delta_mrr_daily)
                    .filter(m_dsl::tenant_id.eq(row.tenant_id))
                    .filter(m_dsl::plan_version_id.eq(row.plan_version_id))
                    .filter(m_dsl::currency.eq(&row.currency))
                    .filter(m_dsl::date.eq(date))
                    .set((
                        m_dsl::net_mrr_cents_usd
                            .eq(Decimal::from(row.net_mrr_cents) / rate_decimal),
                        m_dsl::new_business_cents_usd
                            .eq(Decimal::from(row.new_business_cents) / rate_decimal),
                        m_dsl::expansion_cents_usd
                            .eq(Decimal::from(row.expansion_cents) / rate_decimal),
                        m_dsl::contraction_cents_usd
                            .eq(Decimal::from(row.contraction_cents) / rate_decimal),
                        m_dsl::churn_cents_usd.eq(Decimal::from(row.churn_cents) / rate_decimal),
                        m_dsl::reactivation_cents_usd
                            .eq(Decimal::from(row.reactivation_cents) / rate_decimal),
                        m_dsl::historical_rate_id.eq(historical_rate_id),
                    ));

                query
                    .execute(conn)
                    .await
                    .attach("Error updating bi_delta_mrr_daily USD values")
                    .into_db_result()?;
                updated += 1;
            }
        }
        Ok(updated)
    }
}

impl BiRevenueDailyRow {
    /// Update USD values for all rows on a specific date (called when new rates are available)
    pub async fn update_usd_values_for_date(
        conn: &mut PgConn,
        date: NaiveDate,
        rates: &std::collections::BTreeMap<String, f32>,
        historical_rate_id: Uuid,
    ) -> DbResult<usize> {
        use crate::schema::bi_revenue_daily::dsl as r_dsl;
        use diesel_async::RunQueryDsl;

        let rows = r_dsl::bi_revenue_daily
            .filter(r_dsl::revenue_date.eq(date))
            .load::<BiRevenueDailyRow>(conn)
            .await
            .attach("Error loading bi_revenue_daily for date")
            .into_db_result()?;

        let mut updated = 0;
        for row in rows {
            if let Some(rate) = rates.get(&row.currency) {
                if *rate == 0.0 {
                    continue;
                }
                let rate_decimal = Decimal::from_f32(*rate).unwrap_or(Decimal::ONE);

                let query = diesel::update(r_dsl::bi_revenue_daily)
                    .filter(r_dsl::tenant_id.eq(row.tenant_id))
                    .filter(r_dsl::plan_version_id.eq(row.plan_version_id))
                    .filter(r_dsl::currency.eq(&row.currency))
                    .filter(r_dsl::revenue_date.eq(date))
                    .set((
                        r_dsl::net_revenue_cents_usd
                            .eq(Decimal::from(row.net_revenue_cents) / rate_decimal),
                        r_dsl::historical_rate_id.eq(historical_rate_id),
                    ));

                query
                    .execute(conn)
                    .await
                    .attach("Error updating bi_revenue_daily USD values")
                    .into_db_result()?;
                updated += 1;
            }
        }
        Ok(updated)
    }
}
