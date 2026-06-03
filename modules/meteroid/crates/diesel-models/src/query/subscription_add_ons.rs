use crate::errors::IntoDbResult;
use crate::subscription_add_ons::{SubscriptionAddOnRow, SubscriptionAddOnRowNew};
use crate::{DbResult, PgConn};
use chrono::NaiveDate;
use common_domain::ids::{ProductId, SubscriptionAddOnId, SubscriptionId, TenantId};
use diesel::{BoolExpressionMethods, ExpressionMethods, OptionalExtension, SelectableHelper};
use diesel::{QueryDsl, debug_query};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl SubscriptionAddOnRow {
    /// Delete subscription add-ons whose add_on_id is NOT in the allowed set.
    pub async fn delete_incompatible(
        conn: &mut PgConn,
        subscription_id: &SubscriptionId,
        compatible_add_on_ids: &[common_domain::ids::AddOnId],
    ) -> DbResult<Vec<SubscriptionAddOnRow>> {
        use crate::schema::subscription_add_on::dsl as sao_dsl;

        let query = diesel::delete(
            sao_dsl::subscription_add_on
                .filter(sao_dsl::subscription_id.eq(subscription_id))
                .filter(diesel::dsl::not(
                    sao_dsl::add_on_id.eq_any(compatible_add_on_ids),
                )),
        )
        .returning(SubscriptionAddOnRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while deleting incompatible subscription add-ons")
            .into_db_result()
    }

    pub async fn insert_batch(
        conn: &mut PgConn,
        batch: Vec<&SubscriptionAddOnRowNew>,
    ) -> DbResult<Vec<SubscriptionAddOnRow>> {
        use crate::schema::subscription_add_on::dsl as sao_dsl;

        let query = diesel::insert_into(sao_dsl::subscription_add_on).values(batch);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while inserting SubscriptionAddOn batch")
            .into_db_result()
    }

    /// Fetch the lineage root for a set of add-on ids. Returns `(id, lineage_id)`
    /// pairs; a `None` lineage_id means the row is its own root.
    pub async fn find_lineage_by_ids(
        conn: &mut PgConn,
        ids: &[SubscriptionAddOnId],
    ) -> DbResult<Vec<(SubscriptionAddOnId, Option<SubscriptionAddOnId>)>> {
        use crate::schema::subscription_add_on::dsl as d;

        if ids.is_empty() {
            return Ok(vec![]);
        }

        let query = d::subscription_add_on
            .filter(d::id.eq_any(ids))
            .select((d::id, d::lineage_id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .load(conn)
            .await
            .attach("Error while fetching subscription add-on lineage")
            .into_db_result()
    }

    pub async fn list_by_subscription_id(
        conn: &mut PgConn,
        tenant_id: &TenantId,
        subscription_id: &SubscriptionId,
    ) -> DbResult<Vec<SubscriptionAddOnRow>> {
        use crate::schema::subscription::dsl as s_dsl;
        use crate::schema::subscription_add_on::dsl as sao_dsl;

        let query = sao_dsl::subscription_add_on
            .inner_join(s_dsl::subscription)
            .filter(sao_dsl::subscription_id.eq(subscription_id))
            .filter(s_dsl::tenant_id.eq(tenant_id))
            .select(SubscriptionAddOnRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while listing SubscriptionAddOn by subscription_id")
            .into_db_result()
    }

    /// List only currently-active add-ons (effective_to IS NULL).
    pub async fn list_by_subscription_id_active(
        conn: &mut PgConn,
        tenant_id: &TenantId,
        subscription_id: &SubscriptionId,
    ) -> DbResult<Vec<SubscriptionAddOnRow>> {
        use crate::schema::subscription::dsl as s_dsl;
        use crate::schema::subscription_add_on::dsl as sao_dsl;

        let query = sao_dsl::subscription_add_on
            .inner_join(s_dsl::subscription)
            .filter(sao_dsl::subscription_id.eq(subscription_id))
            .filter(s_dsl::tenant_id.eq(tenant_id))
            .filter(sao_dsl::effective_to.is_null())
            .select(SubscriptionAddOnRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while listing active SubscriptionAddOn by subscription_id")
            .into_db_result()
    }

    /// Close active add-ons by setting effective_to.
    pub async fn close_addons(
        conn: &mut PgConn,
        ids: &[SubscriptionAddOnId],
        effective_to: NaiveDate,
    ) -> DbResult<()> {
        use crate::schema::subscription_add_on::dsl as sao_dsl;

        if ids.is_empty() {
            return Ok(());
        }

        let query = diesel::update(sao_dsl::subscription_add_on)
            .filter(sao_dsl::id.eq_any(ids))
            .filter(sao_dsl::effective_to.is_null())
            .set(sao_dsl::effective_to.eq(effective_to));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while closing subscription add-ons")
            .map(|_| ())
            .into_db_result()
    }

    /// List all add-ons (active and closed) overlapping with [period_start, period_end].
    pub async fn list_add_on_history_for_period(
        conn: &mut PgConn,
        subscription_id: &SubscriptionId,
        period_start: NaiveDate,
        period_end: NaiveDate,
    ) -> DbResult<Vec<SubscriptionAddOnRow>> {
        use crate::schema::subscription_add_on::dsl as sao_dsl;

        // An add-on overlaps if: effective_from < period_end AND (effective_to IS NULL OR effective_to > period_start)
        let query = sao_dsl::subscription_add_on
            .filter(sao_dsl::subscription_id.eq(subscription_id))
            .filter(sao_dsl::effective_from.lt(period_end))
            .filter(
                sao_dsl::effective_to
                    .is_null()
                    .or(sao_dsl::effective_to.gt(period_start)),
            )
            .select(SubscriptionAddOnRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while listing add-on history for period")
            .into_db_result()
    }

    pub async fn delete_by_id(
        conn: &mut PgConn,
        id: SubscriptionAddOnId,
        subscription_id: &SubscriptionId,
        tenant_id: &TenantId,
    ) -> DbResult<()> {
        use crate::errors::{DatabaseError, DatabaseErrorContainer};
        use crate::schema::subscription::dsl as s_dsl;
        use crate::schema::subscription_add_on::dsl as sao_dsl;

        // Verify the subscription belongs to the tenant
        let sub_exists = s_dsl::subscription
            .filter(s_dsl::id.eq(subscription_id))
            .filter(s_dsl::tenant_id.eq(tenant_id))
            .select(s_dsl::id)
            .first::<SubscriptionId>(conn)
            .await
            .optional()
            .attach("Error verifying subscription tenant")
            .into_db_result()?;

        if sub_exists.is_none() {
            return Err(DatabaseErrorContainer::from(DatabaseError::NotFound));
        }

        let query = diesel::delete(
            sao_dsl::subscription_add_on
                .filter(sao_dsl::id.eq(id))
                .filter(sao_dsl::subscription_id.eq(subscription_id)),
        );

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        let affected = query
            .execute(conn)
            .await
            .attach("Error while deleting SubscriptionAddOn")
            .into_db_result()?;

        if affected == 0 {
            return Err(DatabaseErrorContainer::from(DatabaseError::NotFound));
        }

        Ok(())
    }

    pub async fn list_active_add_on_ids(
        conn: &mut PgConn,
        subscription_ids: &[SubscriptionId],
        tenant_id: &TenantId,
    ) -> DbResult<Vec<common_domain::ids::AddOnId>> {
        use crate::schema::subscription::dsl as s_dsl;
        use crate::schema::subscription_add_on::dsl as sao_dsl;
        use error_stack::ResultExt;

        if subscription_ids.is_empty() {
            return Ok(vec![]);
        }

        let query = sao_dsl::subscription_add_on
            .inner_join(s_dsl::subscription)
            .filter(sao_dsl::subscription_id.eq_any(subscription_ids))
            .filter(s_dsl::tenant_id.eq(tenant_id))
            .filter(sao_dsl::effective_to.is_null())
            .select(sao_dsl::add_on_id);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while fetching add-on ids by subscription ids")
            .into_db_result()
    }

    /// Fetch distinct product IDs from active subscription add-ons for the given subscriptions.
    pub async fn list_active_product_ids(
        conn: &mut PgConn,
        subscription_ids: &[SubscriptionId],
        tenant_id: &TenantId,
    ) -> DbResult<Vec<ProductId>> {
        use crate::schema::subscription::dsl as s_dsl;
        use crate::schema::subscription_add_on::dsl as sao_dsl;
        use diesel::dsl::not;
        use error_stack::ResultExt;

        if subscription_ids.is_empty() {
            return Ok(vec![]);
        }

        let query = sao_dsl::subscription_add_on
            .inner_join(s_dsl::subscription)
            .filter(sao_dsl::subscription_id.eq_any(subscription_ids))
            .filter(s_dsl::tenant_id.eq(tenant_id))
            .filter(sao_dsl::effective_to.is_null())
            .filter(not(sao_dsl::product_id.is_null()))
            .select(sao_dsl::product_id);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        let rows: Vec<Option<ProductId>> = query
            .get_results(conn)
            .await
            .attach("Error while fetching product ids from subscription add-ons")
            .into_db_result()?;

        Ok(rows.into_iter().flatten().collect())
    }
}
