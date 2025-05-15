use crate::errors::{DatabaseError, DatabaseErrorContainer, IntoDbResult};
use crate::tenants::{TenantRow, TenantRowNew, TenantRowPatch, TenantWithOrganizationRow};
use crate::{DbResult, PgConn};

use common_domain::ids::{OrganizationId, TenantId};
use diesel::prelude::{ExpressionMethods, QueryDsl};
use diesel::{IntoSql, JoinOnDsl, SelectableHelper, debug_query};
use error_stack::ResultExt;

impl TenantRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<TenantRow> {
        use crate::schema::tenant::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(tenant).values(self);
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting tenant")
            .into_db_result()
    }
}

impl TenantRow {
    pub async fn find_by_id(conn: &mut PgConn, tenant_id: TenantId) -> DbResult<TenantRow> {
        use crate::schema::tenant::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = tenant.filter(id.eq(tenant_id));
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach_printable("Error while finding tenant by id")
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

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach_printable("Error while finding tenant by id")
            .into_db_result()
    }

    pub async fn get_reporting_currency_by_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
    ) -> DbResult<String> {
        use crate::schema::tenant::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = tenant.filter(id.eq(tenant_id)).select(reporting_currency);
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach_printable("Error while finding tenant by id")
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
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach_printable("Error while finding tenant by id")
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

        let query = t_dsl::tenant
            .inner_join(o_dsl::organization.on(t_dsl::organization_id.eq(o_dsl::id)))
            .filter(t_dsl::slug.eq(param_tenant_slug))
            .filter(o_dsl::slug.eq(organization_slug))
            .select(TenantRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach_printable("Error while finding tenant by slug")
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

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach_printable("Error while fetching tenants by user_id")
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
            .attach_printable("Error while fetching tenants by user_id")
            .into_db_result()?;

        let available_currencies: Vec<Option<String>> = t_dsl::tenant
            .filter(t_dsl::id.eq(tenant_id))
            .select(t_dsl::available_currencies)
            .first(conn)
            .await
            .attach_printable("Error while fetching tenants by user_id")
            .into_db_result()?;

        // merge the two lists
        let result: Vec<(String, u64)> = available_currencies
            .into_iter()
            .filter_map(|currency| {
                currency.map(|currency| {
                    let count = currency_stats
                        .iter()
                        .find(|(c, _)| c == &currency)
                        .map(|(_, count)| *count as u64)
                        .unwrap_or(0);
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
            .attach_printable("Error while fetching tenants by user_id")
            .into_db_result()?;

        Ok(result.into_iter().flatten().collect())
    }

    pub async fn update_available_currencies(
        conn: &mut PgConn,
        tenant_id: TenantId,
        new_currencies: Vec<String>,
    ) -> DbResult<Vec<String>> {
        use crate::schema::customer::dsl as c_dsl;
        use crate::schema::tenant::dsl as t_dsl;
        use diesel_async::RunQueryDsl;

        // Get current currencies and their usage counts in a single query
        let currency_stats: Vec<(String, i64)> = c_dsl::customer
            .filter(c_dsl::tenant_id.eq(tenant_id))
            .group_by(c_dsl::currency)
            .select((
                c_dsl::currency,
                diesel::dsl::count_star().into_sql::<diesel::sql_types::BigInt>(),
            ))
            .get_results(conn)
            .await
            .attach_printable("Error while fetching tenant currency usage")
            .into_db_result()?;

        // Check if any currency in use is being removed
        for (currency, count) in &currency_stats {
            if !new_currencies.contains(currency) {
                return Err(DatabaseErrorContainer::from(DatabaseError::CheckViolation(
                    format!(
                        "Cannot remove currency {} as it is being used by {} customers",
                        currency, count
                    ),
                )));
            }
        }

        // Convert to expected format for available_currencies field
        let new_currencies_option: Vec<Option<String>> =
            new_currencies.clone().into_iter().map(Some).collect();

        // Update tenant record
        diesel::update(t_dsl::tenant.filter(t_dsl::id.eq(tenant_id)))
            .set(t_dsl::available_currencies.eq(&new_currencies_option))
            .execute(conn)
            .await
            .attach_printable("Error while updating tenant currencies")
            .into_db_result()?;

        Ok(new_currencies)
    }
}

impl TenantRowPatch {
    pub async fn update(&self, conn: &mut PgConn, tenant_id: TenantId) -> DbResult<TenantRow> {
        use crate::schema::tenant::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(tenant.filter(id.eq(tenant_id)))
            .set(self)
            .returning(TenantRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach_printable("Error while updating tenant")
            .into_db_result()
    }
}
