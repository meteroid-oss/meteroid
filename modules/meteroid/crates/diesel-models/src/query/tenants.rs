use crate::enums::{TenantEnvironmentEnum, TenantStatusEnum};
use crate::errors::IntoDbResult;
use crate::tenants::{TenantRow, TenantRowNew, TenantRowPatch, TenantWithOrganizationRow};
use crate::{DbResult, PgConn};

use common_domain::ids::{OrganizationId, TenantId};
use diesel::prelude::{BoolExpressionMethods, ExpressionMethods, QueryDsl};
use diesel::{IntoSql, JoinOnDsl, SelectableHelper};
use error_stack::ResultExt;

impl TenantRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<TenantRow> {
        use crate::schema::tenant::dsl::tenant;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(tenant).values(self);
        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while inserting tenant")
            .into_db_result()
    }
}

impl TenantRow {
    pub async fn find_by_id(conn: &mut PgConn, tenant_id: TenantId) -> DbResult<TenantRow> {
        use crate::schema::tenant::dsl::{id, tenant};
        use diesel_async::RunQueryDsl;

        let query = tenant.filter(id.eq(tenant_id));
        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while finding tenant by id")
            .into_db_result()
    }

    pub async fn find_by_id_for_update(
        conn: &mut PgConn,
        tenant_id: TenantId,
    ) -> DbResult<TenantRow> {
        use crate::schema::tenant::dsl::{id, tenant};
        use diesel_async::RunQueryDsl;

        let query = tenant.filter(id.eq(tenant_id)).for_update();
        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while locking tenant by id")
            .into_db_result()
    }

    pub async fn find_by_id_with_org(
        conn: &mut PgConn,
        tenant_id: TenantId,
    ) -> DbResult<TenantWithOrganizationRow> {
        use crate::schema::organization::dsl as o_dsl;
        use crate::schema::tenant::dsl as t_dsl;
        use diesel_async::RunQueryDsl;

        let query = t_dsl::tenant
            .inner_join(o_dsl::organization.on(t_dsl::organization_id.eq(o_dsl::id)))
            .filter(t_dsl::id.eq(tenant_id))
            .select(TenantWithOrganizationRow::as_select());

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while finding tenant by id")
            .into_db_result()
    }

    pub async fn list_by_ids_with_org(
        conn: &mut PgConn,
        tenant_ids: &[TenantId],
    ) -> DbResult<Vec<TenantWithOrganizationRow>> {
        use crate::schema::organization::dsl as o_dsl;
        use crate::schema::tenant::dsl as t_dsl;
        use diesel_async::RunQueryDsl;

        if tenant_ids.is_empty() {
            return Ok(vec![]);
        }

        let query = t_dsl::tenant
            .inner_join(o_dsl::organization.on(t_dsl::organization_id.eq(o_dsl::id)))
            .filter(t_dsl::id.eq_any(tenant_ids))
            .select(TenantWithOrganizationRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing tenants by ids")
            .into_db_result()
    }

    pub async fn get_reporting_currency_by_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
    ) -> DbResult<String> {
        use crate::schema::tenant::dsl::{id, reporting_currency, tenant};
        use diesel_async::RunQueryDsl;

        let query = tenant.filter(id.eq(tenant_id)).select(reporting_currency);
        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while finding tenant by id")
            .into_db_result()
    }

    pub async fn find_keep_last_events_by_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
    ) -> DbResult<bool> {
        use crate::schema::tenant::dsl::{id, keep_last_events, tenant};
        use diesel_async::RunQueryDsl;

        let query = tenant.filter(id.eq(tenant_id)).select(keep_last_events);
        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while finding tenant keep-last flags")
            .into_db_result()
    }

    pub async fn find_by_id_and_organization_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        organization_id: OrganizationId,
    ) -> DbResult<TenantRow> {
        use crate::schema::tenant::dsl as t_dsl;
        use diesel_async::RunQueryDsl;

        let query = t_dsl::tenant
            .filter(t_dsl::id.eq(tenant_id))
            .filter(t_dsl::organization_id.eq(organization_id));
        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while finding tenant by id")
            .into_db_result()
    }

    pub async fn find_by_slug_and_organization_slug(
        conn: &mut PgConn,
        param_tenant_slug: String,
        organization_slug: String,
    ) -> DbResult<TenantRow> {
        use crate::schema::organization::dsl as o_dsl;
        use crate::schema::tenant::dsl as t_dsl;
        use diesel_async::RunQueryDsl;

        // Tenants pending/marked for deletion, or under an archived org, are not
        // resolvable — blocks access/login.
        let query = t_dsl::tenant
            .inner_join(o_dsl::organization.on(t_dsl::organization_id.eq(o_dsl::id)))
            .filter(t_dsl::slug.eq(param_tenant_slug))
            .filter(o_dsl::slug.eq(organization_slug))
            .filter(t_dsl::status.ne(TenantStatusEnum::PendingDeletion))
            .filter(t_dsl::status.ne(TenantStatusEnum::MarkedForDeletion))
            .filter(o_dsl::archived_at.is_null())
            .select(TenantRow::as_select());

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while finding tenant by slug")
            .into_db_result()
    }

    pub async fn list_by_organization_id(
        conn: &mut PgConn,
        organization_id: OrganizationId,
    ) -> DbResult<Vec<TenantRow>> {
        use crate::schema::organization::dsl as o_dsl;
        use crate::schema::tenant::dsl as t_dsl;
        use diesel_async::RunQueryDsl;

        let query = t_dsl::tenant
            .inner_join(o_dsl::organization.on(t_dsl::organization_id.eq(o_dsl::id)))
            .filter(o_dsl::id.eq(organization_id))
            .filter(t_dsl::archived_at.is_null())
            .select(TenantRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while fetching tenants by user_id")
            .into_db_result()
    }

    /// Every slug in the org, archived rows included — the slug unique index
    /// `(organization_id, slug)` covers archived tenants, so slug de-duplication
    /// must too.
    pub async fn list_slugs_by_organization_id(
        conn: &mut PgConn,
        organization_id: OrganizationId,
    ) -> DbResult<Vec<String>> {
        use crate::schema::tenant::dsl as t_dsl;
        use diesel_async::RunQueryDsl;

        let query = t_dsl::tenant
            .filter(t_dsl::organization_id.eq(organization_id))
            .select(t_dsl::slug);

        query
            .get_results(conn)
            .await
            .attach("Error while listing tenant slugs by organization")
            .into_db_result()
    }

    pub async fn list_by_environment(
        conn: &mut PgConn,
        environment: crate::enums::TenantEnvironmentEnum,
    ) -> DbResult<Vec<TenantRow>> {
        use crate::schema::tenant::dsl as t_dsl;
        use diesel_async::RunQueryDsl;

        let query = t_dsl::tenant
            .filter(t_dsl::environment.eq(environment))
            .filter(t_dsl::archived_at.is_null())
            .select(TenantRow::as_select());

        query
            .get_results(conn)
            .await
            .attach("Error while fetching tenants by environment")
            .into_db_result()
    }

    /// Non-Production tenants still in the worker's scope (ACTIVE or PAUSED) —
    /// the ones it may pause or archive. Production is excluded here AND in every
    /// mutation below.
    pub async fn list_lifecycle_managed(conn: &mut PgConn) -> DbResult<Vec<TenantRow>> {
        use crate::schema::tenant::dsl as t_dsl;
        use diesel_async::RunQueryDsl;

        let query = t_dsl::tenant
            .filter(t_dsl::environment.ne(TenantEnvironmentEnum::Production))
            .filter(t_dsl::status.eq_any([TenantStatusEnum::Active, TenantStatusEnum::Paused]))
            .select(TenantRow::as_select());

        query
            .get_results(conn)
            .await
            .attach("Error while fetching lifecycle-managed tenants")
            .into_db_result()
    }

    /// Set a non-prod tenant's lifecycle status. The `environment != Production`
    /// guard makes it impossible to change a production tenant's status even if
    /// called in error. Resuming also clears any `archived_at` marker.
    pub async fn set_status(
        conn: &mut PgConn,
        tenant_id: TenantId,
        status: TenantStatusEnum,
        at: chrono::NaiveDateTime,
    ) -> DbResult<usize> {
        use crate::schema::tenant::dsl as t_dsl;
        use diesel_async::RunQueryDsl;

        // Keep the legacy archived_at timestamp consistent with the status so
        // existing archived_at-based listings stay correct.
        let archived_at = match status {
            TenantStatusEnum::Archived
            | TenantStatusEnum::MarkedForDeletion
            | TenantStatusEnum::PendingDeletion => Some(at),
            TenantStatusEnum::Active | TenantStatusEnum::Paused => None,
        };

        diesel::update(
            t_dsl::tenant
                .filter(t_dsl::id.eq(tenant_id))
                .filter(t_dsl::environment.ne(TenantEnvironmentEnum::Production)),
        )
        .set((t_dsl::status.eq(status), t_dsl::archived_at.eq(archived_at)))
        .execute(conn)
        .await
        .attach("Error while updating tenant status")
        .into_db_result()
    }

    /// Hard-delete a non-Production tenant. Every tenant-scoped row cascades
    /// away via the ON DELETE CASCADE foreign keys. Production is refused in the
    /// query layer as a last line of defense.
    pub async fn delete(conn: &mut PgConn, tenant_id: TenantId) -> DbResult<usize> {
        use crate::schema::tenant::dsl as t_dsl;
        use diesel_async::RunQueryDsl;

        diesel::delete(
            t_dsl::tenant
                .filter(t_dsl::id.eq(tenant_id))
                .filter(t_dsl::environment.ne(TenantEnvironmentEnum::Production)),
        )
        .execute(conn)
        .await
        .attach("Error while deleting tenant")
        .into_db_result()
    }

    /// Hard-delete many non-Production tenants in one statement (the weekly
    /// cleanup worker). Cascades wipe each tenant's scoped rows.
    pub async fn delete_many(conn: &mut PgConn, ids: &[TenantId]) -> DbResult<usize> {
        use crate::schema::tenant::dsl as t_dsl;
        use diesel_async::RunQueryDsl;

        diesel::delete(
            t_dsl::tenant
                .filter(t_dsl::id.eq_any(ids))
                .filter(t_dsl::environment.ne(TenantEnvironmentEnum::Production)),
        )
        .execute(conn)
        .await
        .attach("Error while deleting tenants")
        .into_db_result()
    }

    /// Up to `limit` tenant ids currently pending deletion (the cleanup worker's
    /// work list). The worker drains in batches, re-querying until fewer than
    /// `limit` come back.
    pub async fn list_ids_pending_deletion(
        conn: &mut PgConn,
        limit: i64,
    ) -> DbResult<Vec<TenantId>> {
        use crate::schema::tenant::dsl as t_dsl;
        use diesel_async::RunQueryDsl;

        t_dsl::tenant
            .filter(t_dsl::status.eq(TenantStatusEnum::PendingDeletion))
            .select(t_dsl::id)
            .limit(limit)
            .get_results(conn)
            .await
            .attach("Error while listing tenants marked for deletion")
            .into_db_result()
    }

    /// Set `last_active_at = now`, but only if it is unset or older than
    /// `stale_before` — bounds writes to at most one per staleness window.
    pub async fn touch_activity(
        conn: &mut PgConn,
        tenant_id: TenantId,
        now: chrono::NaiveDateTime,
        stale_before: chrono::NaiveDateTime,
    ) -> DbResult<usize> {
        use crate::schema::tenant::dsl as t_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(t_dsl::tenant.filter(t_dsl::id.eq(tenant_id)))
            .filter(
                t_dsl::last_active_at
                    .is_null()
                    .or(t_dsl::last_active_at.lt(stale_before)),
            )
            .set(t_dsl::last_active_at.eq(now));

        query
            .execute(conn)
            .await
            .attach("Error while touching tenant activity")
            .into_db_result()
    }

    pub async fn list_tenant_currencies_with_customer_count(
        conn: &mut PgConn,
        tenant_id: TenantId,
    ) -> DbResult<Vec<(String, u64)>> {
        use crate::schema::customer::dsl as c_dsl;
        use crate::schema::tenant::dsl as t_dsl;
        use diesel_async::RunQueryDsl;

        let currency_stats: Vec<(String, i64)> = c_dsl::customer
            .filter(c_dsl::tenant_id.eq(tenant_id))
            .group_by(c_dsl::currency)
            .select((
                c_dsl::currency,
                diesel::dsl::count_star().into_sql::<diesel::sql_types::BigInt>(),
            ))
            .get_results(conn)
            .await
            .attach("Error while fetching tenants by user_id")
            .into_db_result()?;

        let available_currencies: Vec<Option<String>> = t_dsl::tenant
            .filter(t_dsl::id.eq(tenant_id))
            .select(t_dsl::available_currencies)
            .first(conn)
            .await
            .attach("Error while fetching tenants by user_id")
            .into_db_result()?;

        // merge the two lists
        let result: Vec<(String, u64)> = available_currencies
            .into_iter()
            .filter_map(|currency| {
                currency.map(|currency| {
                    let count = currency_stats
                        .iter()
                        .find(|(c, _)| c == &currency)
                        .map_or(0, |(_, count)| *count as u64);
                    (currency, count)
                })
            })
            .collect();

        Ok(result)
    }

    pub async fn list_tenant_currencies(
        conn: &mut PgConn,
        tenant_id: TenantId,
    ) -> DbResult<Vec<String>> {
        use crate::schema::tenant::dsl as t_dsl;
        use diesel_async::RunQueryDsl;

        let result: Vec<Option<String>> = t_dsl::tenant
            .filter(t_dsl::id.eq(tenant_id))
            .select(t_dsl::available_currencies)
            .first(conn)
            .await
            .attach("Error while fetching tenants by user_id")
            .into_db_result()?;

        Ok(result.into_iter().flatten().collect())
    }

    pub async fn update_available_currencies(
        conn: &mut PgConn,
        tenant_id: TenantId,
        new_currencies: Vec<String>,
    ) -> DbResult<Vec<String>> {
        use crate::schema::tenant::dsl as t_dsl;
        use diesel_async::RunQueryDsl;

        // TODO : commented out as we transition to invoicing-entity default currency
        // Get current currencies and their usage counts in a single query
        // let currency_stats: Vec<(String, i64)> = c_dsl::customer
        //     .filter(c_dsl::tenant_id.eq(tenant_id))
        //     .group_by(c_dsl::currency)
        //     .select((
        //         c_dsl::currency,
        //         diesel::dsl::count_star().into_sql::<diesel::sql_types::BigInt>(),
        //     ))
        //     .get_results(conn)
        //     .await
        //     .attach("Error while fetching tenant currency usage")
        //     .into_db_result()?;
        //
        // // Check if any currency in use is being removed
        // for (currency, count) in &currency_stats {
        //     if !new_currencies.contains(currency) {
        //         return Err(DatabaseErrorContainer::from(DatabaseError::CheckViolation(
        //             format!(
        //                 "Cannot remove currency {currency} as it is being used by {count} customers"
        //             ),
        //         )));
        //     }
        // }

        // Convert to expected format for available_currencies field
        let new_currencies_option: Vec<Option<String>> =
            new_currencies.clone().into_iter().map(Some).collect();

        // Update tenant record
        diesel::update(t_dsl::tenant.filter(t_dsl::id.eq(tenant_id)))
            .set(t_dsl::available_currencies.eq(&new_currencies_option))
            .execute(conn)
            .await
            .attach("Error while updating tenant currencies")
            .into_db_result()?;

        Ok(new_currencies)
    }

    pub async fn add_currency_if_missing(
        conn: &mut PgConn,
        tenant_id: TenantId,
        currency: String,
    ) -> DbResult<()> {
        use crate::schema::tenant::dsl as t_dsl;
        use diesel_async::RunQueryDsl;

        let current: Vec<Option<String>> = t_dsl::tenant
            .filter(t_dsl::id.eq(tenant_id))
            .select(t_dsl::available_currencies)
            .first(conn)
            .await
            .attach("Error while fetching tenant currencies")
            .into_db_result()?;

        if !current
            .iter()
            .any(|c| c.as_deref().is_some() && c.as_deref().unwrap() == currency)
        {
            let mut updated = current;
            updated.push(Some(currency));

            diesel::update(t_dsl::tenant.filter(t_dsl::id.eq(tenant_id)))
                .set(t_dsl::available_currencies.eq(&updated))
                .execute(conn)
                .await
                .attach("Error while updating tenant currencies")
                .into_db_result()?;
        }
        Ok(())
    }
}

impl TenantRowPatch {
    pub async fn update(&self, conn: &mut PgConn, tenant_id: TenantId) -> DbResult<TenantRow> {
        use crate::schema::tenant::dsl::{id, tenant};
        use diesel_async::RunQueryDsl;

        let query = diesel::update(tenant.filter(id.eq(tenant_id)))
            .set(self)
            .returning(TenantRow::as_select());

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while updating tenant")
            .into_db_result()
    }
}
