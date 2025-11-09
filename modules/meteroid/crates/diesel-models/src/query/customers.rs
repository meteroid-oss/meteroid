use crate::customers::{
    CustomerBriefRow, CustomerRow, CustomerRowNew, CustomerRowPatch, CustomerRowUpdate,
};
use crate::enums::ConnectorProviderEnum;
use crate::errors::IntoDbResult;
use crate::extend::connection_metadata;
use crate::extend::order::OrderByRequest;
use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};
use crate::{DbResult, PgConn};
use common_domain::ids::{AliasOr, BaseId, ConnectorId, CustomerId, TenantId};
use diesel::{
    BoolExpressionMethods, ExpressionMethods, OptionalExtension, PgTextExpressionMethods, QueryDsl,
    SelectableHelper, debug_query,
};
use error_stack::ResultExt;
use itertools::Itertools;
use std::ops::Add;
use tap::TapFallible;
use uuid::Uuid;

impl CustomerRowNew {
    pub async fn insert(self, conn: &mut PgConn) -> DbResult<CustomerRow> {
        use crate::schema::customer::dsl::customer;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(customer).values(&self);
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while inserting customer")
            .into_db_result()
    }
}

impl CustomerRow {
    pub async fn find_by_ids_or_aliases(
        conn: &mut PgConn,
        tenant_id: TenantId,
        ids_or_aliases: Vec<AliasOr<CustomerId>>,
    ) -> DbResult<Vec<CustomerRow>> {
        use crate::schema::customer::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let (ids, aliases): (Vec<CustomerId>, Vec<String>) = ids_or_aliases
            .into_iter()
            .partition_map(|id_or_alias| match id_or_alias {
                AliasOr::Id(id) => itertools::Either::Left(id),
                AliasOr::Alias(alias) => itertools::Either::Right(alias),
            });

        let query = c_dsl::customer
            .filter(c_dsl::tenant_id.eq(tenant_id))
            .filter(c_dsl::id.eq_any(ids).or(c_dsl::alias.eq_any(aliases)))
            .select(CustomerRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while finding customers by ids or aliases")
            .into_db_result()
    }

    pub async fn find_by_id_or_alias(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id_or_alias: AliasOr<CustomerId>,
    ) -> DbResult<CustomerRow> {
        use crate::schema::customer::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let mut query = c_dsl::customer
            .filter(c_dsl::tenant_id.eq(tenant_id))
            .select(CustomerRow::as_select())
            .into_boxed();

        match id_or_alias {
            AliasOr::Id(id) => {
                query = query.filter(c_dsl::id.eq(id));
            }
            AliasOr::Alias(alias) => {
                query = query.filter(c_dsl::alias.eq(alias));
            }
        }

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach("Error while finding customer by id or alias")
            .into_db_result()
    }

    pub async fn find_by_id_or_alias_including_archived(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id_or_alias: AliasOr<CustomerId>,
    ) -> DbResult<CustomerRow> {
        use crate::schema::customer::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let mut query = c_dsl::customer
            .filter(c_dsl::tenant_id.eq(tenant_id))
            .select(CustomerRow::as_select())
            .into_boxed();

        match id_or_alias {
            AliasOr::Id(id) => {
                query = query.filter(c_dsl::id.eq(id));
            }
            AliasOr::Alias(alias) => {
                query = query.filter(c_dsl::alias.eq(alias));
            }
        }

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach("Error while finding customer by id or alias (including archived)")
            .into_db_result()
    }

    pub async fn find_by_id(
        conn: &mut PgConn,
        customer_id: &CustomerId,
        tenant_id_param: &TenantId,
    ) -> DbResult<CustomerRow> {
        use crate::schema::customer::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let query = c_dsl::customer
            .filter(c_dsl::id.eq(customer_id))
            .filter(c_dsl::tenant_id.eq(tenant_id_param));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach("Error while finding customer by id")
            .into_db_result()
    }

    pub async fn find_by_alias(
        conn: &mut PgConn,
        customer_alias: String,
        tenant_id_param: TenantId,
    ) -> DbResult<CustomerRow> {
        use crate::schema::customer::dsl::{alias, customer, tenant_id};
        use diesel_async::RunQueryDsl;

        let query = customer
            .filter(alias.eq(customer_alias))
            .filter(tenant_id.eq(tenant_id_param));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach("Error while finding customer by alias")
            .into_db_result()
    }

    pub async fn resolve_ids_by_aliases(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_customer_aliases: Vec<String>,
    ) -> DbResult<Vec<CustomerBriefRow>> {
        use crate::schema::customer::dsl::{alias, customer, tenant_id};
        use diesel_async::RunQueryDsl;

        let query = customer
            .filter(tenant_id.eq(param_tenant_id))
            .filter(alias.eq_any(param_customer_aliases))
            .select(CustomerBriefRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while finding customer by aliases")
            .into_db_result()
    }

    pub async fn resolve_id_by_alias(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_customer_alias: String,
    ) -> DbResult<CustomerBriefRow> {
        use crate::schema::customer::dsl::{alias, customer, tenant_id};
        use diesel_async::RunQueryDsl;

        let query = customer
            .filter(tenant_id.eq(param_tenant_id))
            .filter(alias.eq(param_customer_alias))
            .select(CustomerBriefRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach("Error while finding customer by aliases")
            .into_db_result()
    }

    pub async fn list(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        pagination: PaginationRequest,
        order_by: OrderByRequest,
        param_query: Option<String>,
        param_archived: Option<bool>,
    ) -> DbResult<PaginatedVec<CustomerRow>> {
        use crate::schema::customer::dsl::{
            alias, archived_at, billing_email, created_at, customer, id, name, tenant_id,
        };

        let mut query = customer
            .filter(tenant_id.eq(param_tenant_id))
            .select(CustomerRow::as_select())
            .into_boxed();

        match param_archived {
            Some(true) => {
                query = query.filter(archived_at.is_not_null());
            }
            None | Some(false) => {
                query = query.filter(archived_at.is_null());
            }
        }

        if let Some(param_query) = param_query {
            query = query.filter(
                name.ilike(format!("%{param_query}%"))
                    .or(alias.ilike(format!("%{param_query}%")))
                    .or(billing_email.ilike(format!("%{param_query}%"))),
            );
        }

        match order_by {
            OrderByRequest::IdAsc => query = query.order(id.asc()),
            OrderByRequest::IdDesc => query = query.order(id.desc()),
            OrderByRequest::DateAsc => query = query.order(created_at.asc()),
            OrderByRequest::DateDesc => query = query.order(created_at.desc()),
            _ => query = query.order(id.asc()),
        }

        let paginated_query = query.paginate(pagination);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&paginated_query));

        paginated_query
            .load_and_count_pages(conn)
            .await
            .attach("Error while fetching customers")
            .into_db_result()
    }

    pub async fn list_by_ids_global(
        conn: &mut PgConn,
        ids: Vec<CustomerId>,
    ) -> DbResult<Vec<CustomerRow>> {
        use crate::schema::customer::dsl::{customer, id};
        use diesel_async::RunQueryDsl;

        let query = customer
            .filter(id.eq_any(ids))
            .select(CustomerRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while listing customers by ids")
            .into_db_result()
    }

    pub async fn list_by_ids(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        ids: Vec<CustomerId>,
    ) -> DbResult<Vec<CustomerRow>> {
        use crate::schema::customer::dsl::{customer, id, tenant_id};
        use diesel_async::RunQueryDsl;

        let query = customer
            .filter(tenant_id.eq(tenant_id_param))
            .filter(id.eq_any(ids))
            .select(CustomerRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while listing customers by ids")
            .into_db_result()
    }

    pub async fn insert_customer_batch(
        conn: &mut PgConn,
        batch: Vec<CustomerRowNew>,
    ) -> DbResult<Vec<CustomerRow>> {
        use crate::schema::customer::dsl::customer;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(customer).values(&batch);
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while inserting customer batch")
            .into_db_result()
    }

    pub async fn upsert_customer_batch(
        conn: &mut PgConn,
        batch: Vec<CustomerRowNew>,
    ) -> DbResult<Vec<CustomerRow>> {
        use crate::schema::customer::dsl::*;
        use diesel::upsert::excluded;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(customer)
            .values(&batch)
            .on_conflict((tenant_id, alias))
            .do_update()
            .set((
                name.eq(excluded(name)),
                billing_email.eq(excluded(billing_email)),
                phone.eq(excluded(phone)),
                currency.eq(excluded(currency)),
                billing_address.eq(excluded(billing_address)),
                shipping_address.eq(excluded(shipping_address)),
                invoicing_entity_id.eq(excluded(invoicing_entity_id)),
                bank_account_id.eq(excluded(bank_account_id)),
                vat_number.eq(excluded(vat_number)),
                invoicing_emails.eq(excluded(invoicing_emails)),
                is_tax_exempt.eq(excluded(is_tax_exempt)),
                custom_taxes.eq(excluded(custom_taxes)),
                vat_number_format_valid.eq(excluded(vat_number_format_valid)),
                updated_at.eq(diesel::dsl::now),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while upserting customer batch")
            .into_db_result()
    }

    pub async fn select_for_update(
        conn: &mut PgConn,
        id: CustomerId,
        tenant_id: TenantId,
    ) -> DbResult<CustomerRow> {
        use crate::schema::customer::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let query = c_dsl::customer
            .for_no_key_update()
            .filter(c_dsl::id.eq(id))
            .filter(c_dsl::tenant_id.eq(tenant_id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach("Error while selecting for update customer by id")
            .into_db_result()
    }

    pub async fn update_balance(
        conn: &mut PgConn,
        id: CustomerId,
        delta_cents: i64,
    ) -> DbResult<usize> {
        use crate::schema::customer::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(c_dsl::customer)
            .filter(c_dsl::id.eq(id))
            .set((
                c_dsl::balance_value_cents.eq(c_dsl::balance_value_cents.add(delta_cents)),
                c_dsl::updated_at.eq(diesel::dsl::now),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while update customer balance")
            .into_db_result()
    }

    pub async fn archive(
        conn: &mut PgConn,
        id: CustomerId,
        tenant_id: TenantId,
        archived_by: Uuid,
    ) -> DbResult<usize> {
        use crate::schema::customer::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(c_dsl::customer)
            .filter(c_dsl::id.eq(id))
            .filter(c_dsl::tenant_id.eq(tenant_id))
            .set((
                c_dsl::archived_at.eq(diesel::dsl::now),
                c_dsl::archived_by.eq(archived_by),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while archiving customer")
            .into_db_result()
    }

    pub async fn unarchive(
        conn: &mut PgConn,
        id: CustomerId,
        tenant_id: TenantId,
    ) -> DbResult<usize> {
        use crate::schema::customer::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(c_dsl::customer)
            .filter(c_dsl::id.eq(id))
            .filter(c_dsl::tenant_id.eq(tenant_id))
            .set((
                c_dsl::archived_at.eq(None::<chrono::NaiveDateTime>),
                c_dsl::archived_by.eq(None::<Uuid>),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while unarchiving customer")
            .into_db_result()
    }

    pub async fn find_archived_customer_in_batch(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_ids: Vec<CustomerId>,
    ) -> DbResult<Option<(CustomerId, String)>> {
        use crate::schema::customer::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let query = c_dsl::customer
            .filter(c_dsl::tenant_id.eq(tenant_id))
            .filter(c_dsl::id.eq_any(customer_ids))
            .filter(c_dsl::archived_at.is_not_null())
            .select((c_dsl::id, c_dsl::name))
            .limit(1);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first::<(CustomerId, String)>(conn)
            .await
            .optional()
            .attach("Error while checking for archived customers")
            .into_db_result()
    }
}

impl CustomerRowPatch {
    pub async fn update(
        &self,
        conn: &mut PgConn,
        param_tenant_id: TenantId,
    ) -> DbResult<Option<CustomerRow>> {
        use crate::schema::customer::dsl::{customer, id, tenant_id};
        use diesel_async::RunQueryDsl;

        let query = diesel::update(customer)
            .filter(id.eq(self.id))
            .filter(tenant_id.eq(param_tenant_id))
            .set(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .optional()
            .tap_err(|e| log::error!("Error while patching customer: {e:?}"))
            .attach("Error while patching customer")
            .into_db_result()
    }

    pub async fn upsert_conn_meta(
        conn: &mut PgConn,
        provider: ConnectorProviderEnum,
        customer_id: CustomerId,
        connector_id: ConnectorId,
        external_id: &str,
        external_company_id: &str,
    ) -> DbResult<()> {
        connection_metadata::upsert(
            conn,
            "customer",
            provider.as_meta_key(),
            customer_id.as_uuid(),
            connector_id,
            external_id,
            external_company_id,
        )
        .await
        .map(|_| ())
    }
}

impl CustomerRowUpdate {
    pub async fn update(
        &self,
        conn: &mut PgConn,
        param_tenant_id: TenantId,
    ) -> DbResult<Option<CustomerRow>> {
        use crate::schema::customer::dsl::{customer, id, tenant_id};
        use diesel_async::RunQueryDsl;

        let query = diesel::update(customer)
            .filter(id.eq(self.id))
            .filter(tenant_id.eq(param_tenant_id))
            .set(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .optional()
            .tap_err(|e| log::error!("Error while updating customer: {e:?}"))
            .attach("Error while updating customer")
            .into_db_result()
    }
}
