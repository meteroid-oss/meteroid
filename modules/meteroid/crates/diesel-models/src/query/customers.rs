use crate::customers::{
    CustomerBriefRow, CustomerLifecycleSignalRow, CustomerRow, CustomerRowNew, CustomerRowPatch,
    CustomerRowUpdate, CustomerWithMetricsRow,
};
use crate::enums::ConnectorProviderEnum;
use crate::errors::IntoDbResult;
use crate::extend::connection_metadata;
use crate::extend::filtering::{ListFilter, render_list_filter};
use crate::extend::order::{OrderByParam, OrderDirection};
use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};
use crate::lock::CustomerLock;
use crate::{DbResult, PgConn};
use common_domain::ids::{
    AliasOr, BaseId, ConnectedAccountId, ConnectorId, CustomerId, CustomerPaymentMethodId,
    OrganizationId, PaymentReminderSequenceId, TenantId,
};
use diesel::{
    BoolExpressionMethods, ExpressionMethods, JoinOnDsl, NullableExpressionMethods,
    OptionalExtension, PgExpressionMethods, PgSortExpressionMethods, PgTextExpressionMethods,
    QueryDsl, SelectableHelper,
};
use error_stack::ResultExt;
use itertools::Itertools;
use tap::TapFallible;

impl CustomerRowNew {
    pub async fn insert(self, conn: &mut PgConn) -> DbResult<CustomerRow> {
        use crate::schema::customer::dsl::customer;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(customer).values(&self);
        crate::debug_sql(&query);

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

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while finding customers by ids or aliases")
            .into_db_result()
    }

    /// Customers whose VAT number is due for a best-effort VIES (re)check:
    /// format-valid number, not archived, never checked or last checked before
    /// `checked_before`. Excludes recently created rows so the event-driven
    /// initial validation stays the owner of fresh customers. Oldest first,
    /// never-checked rows leading, across all tenants.
    pub async fn list_vat_revalidation_candidates(
        conn: &mut PgConn,
        checked_before: chrono::NaiveDateTime,
        created_before: chrono::NaiveDateTime,
        limit: i64,
    ) -> DbResult<Vec<CustomerRow>> {
        use crate::schema::customer::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let query = c_dsl::customer
            .filter(c_dsl::vat_number.is_not_null())
            .filter(c_dsl::vat_number_format_valid.eq(true))
            .filter(c_dsl::archived_at.is_null())
            .filter(
                c_dsl::vat_number_checked_at
                    .is_null()
                    .or(c_dsl::vat_number_checked_at.lt(checked_before)),
            )
            .filter(c_dsl::created_at.lt(created_before))
            .order(c_dsl::vat_number_checked_at.asc().nulls_first())
            .limit(limit)
            .select(CustomerRow::as_select());

        query
            .get_results(conn)
            .await
            .attach("Error while listing vat revalidation candidates")
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

        crate::debug_sql(&query);

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

        crate::debug_sql(&query);

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

        crate::debug_sql(&query);

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

        crate::debug_sql(&query);

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

        crate::debug_sql(&query);

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

        crate::debug_sql(&query);

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
        order_by: Option<&str>,
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

        let order = OrderByParam::parse(order_by, "name.asc");

        match (order.column.as_str(), order.direction) {
            ("name", OrderDirection::Asc) => query = query.order((name.asc(), id.asc())),
            ("name", OrderDirection::Desc) => query = query.order((name.desc(), id.desc())),
            ("email", OrderDirection::Asc) => query = query.order((billing_email.asc(), id.asc())),
            ("email", OrderDirection::Desc) => {
                query = query.order((billing_email.desc(), id.desc()))
            }
            ("alias", OrderDirection::Asc) => query = query.order((alias.asc(), id.asc())),
            ("alias", OrderDirection::Desc) => query = query.order((alias.desc(), id.desc())),
            ("created_at", OrderDirection::Asc) => {
                query = query.order((created_at.asc(), id.asc()))
            }
            ("created_at", OrderDirection::Desc) => {
                query = query.order((created_at.desc(), id.desc()))
            }
            _ => query = query.order((name.asc(), id.asc())),
        }

        let paginated_query = query.paginate(pagination);

        crate::debug_sql(&paginated_query);

        paginated_query
            .load_and_count_pages(conn)
            .await
            .attach("Error while fetching customers")
            .into_db_result()
    }

    /// Ids of the tenant's customers matching a set of resolved dynamic filters (custom
    /// properties and native columns). Typed and boxed so dynamic filter logic stays out of the
    /// raw metrics SQL; the result is fed to [`CustomerRow::list_with_metrics`] as an id filter.
    pub async fn list_matching_ids(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        filters: Vec<ListFilter>,
    ) -> DbResult<Vec<CustomerId>> {
        use crate::schema::customer::dsl::{customer, id, tenant_id};
        use diesel_async::RunQueryDsl;

        let mut query = customer
            .filter(tenant_id.eq(param_tenant_id))
            .select(id)
            .into_boxed();

        for f in filters {
            query = query.filter(render_list_filter(f, "customer.custom_properties"));
        }

        crate::debug_sql(&query);

        query
            .load::<CustomerId>(conn)
            .await
            .attach("Error while resolving customer filter ids")
            .into_db_result()
    }

    /// List with derived per-customer MRR and lifecycle_state, supporting
    /// server-side lifecycle/country filtering and sort-by-MRR. The lifecycle
    /// `CASE` mirrors `common_domain::lifecycle::derive_lifecycle_state`; the signal
    /// `EXISTS` predicates mirror `CustomerLifecycleSignalRow::fetch_for_customer`.
    /// MRR sums `subscription.mrr_cents` over active (`ACTIVE`/`PENDING_CHARGE`) subs.
    pub async fn list_with_metrics(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        pagination: PaginationRequest,
        order_by: Option<&str>,
        param_query: Option<String>,
        param_archived: Option<bool>,
        param_lifecycle: Vec<String>,
        param_countries: Vec<String>,
        // `Some` restricts to a pre-resolved id set (dynamic custom-property / native filters,
        // computed by `list_matching_ids`); `None` applies no id filter.
        param_ids: Option<Vec<CustomerId>>,
    ) -> DbResult<PaginatedVec<CustomerWithMetricsRow>> {
        use diesel::sql_types;
        use diesel_async::RunQueryDsl;

        let (has_id_filter, filter_ids) = match param_ids {
            Some(ids) => (true, ids),
            None => (false, Vec::new()),
        };

        let order = OrderByParam::parse(order_by, "name.asc");
        let order_sql = match (order.column.as_str(), order.direction) {
            ("name", OrderDirection::Asc) => "d.name ASC, d.id ASC",
            ("name", OrderDirection::Desc) => "d.name DESC, d.id DESC",
            ("email", OrderDirection::Asc) => "d.billing_email ASC NULLS LAST, d.id ASC",
            ("email", OrderDirection::Desc) => "d.billing_email DESC NULLS LAST, d.id DESC",
            ("alias", OrderDirection::Asc) => "d.alias ASC NULLS LAST, d.id ASC",
            ("alias", OrderDirection::Desc) => "d.alias DESC NULLS LAST, d.id DESC",
            ("created_at", OrderDirection::Asc) => "d.created_at ASC, d.id ASC",
            ("created_at", OrderDirection::Desc) => "d.created_at DESC, d.id DESC",
            ("mrr", OrderDirection::Asc) => "d.mrr_cents ASC, d.id ASC",
            ("mrr", OrderDirection::Desc) => "d.mrr_cents DESC, d.id DESC",
            ("lifecycle", OrderDirection::Asc) => "d.lifecycle_state ASC, d.id ASC",
            ("lifecycle", OrderDirection::Desc) => "d.lifecycle_state DESC, d.id DESC",
            _ => "d.name ASC, d.id ASC",
        };

        let per_page = i64::from(pagination.per_page.unwrap_or(25)).max(1);
        let offset = i64::from(pagination.page) * per_page;
        // None => all records, Some(true) => archived only, Some(false) => active only.
        let archived_mode = match param_archived {
            None => "all",
            Some(true) => "archived",
            Some(false) => "active",
        }
        .to_string();
        let search = param_query.unwrap_or_default();

        let raw_sql = format!(
            r#"
            -- Lifecycle flags are grouped per customer in one pass rather than as a
            -- dozen correlated EXISTS/subqueries per row: the planner prices a
            -- correlated subplan (and a LATERAL is one) per output row, so a dozen of
            -- them estimate into the high six figures, trip jit_above_cost (100k), and
            -- make every call pay seconds of LLVM compilation for milliseconds of work.
            -- Grouping keeps the estimate low so JIT never engages. See analytics
            -- customer projection for the same pattern. plan_version_id is NOT NULL, so
            -- the inner plan join drops no subscription.
            WITH base AS (
                SELECT c.*
                FROM customer c
                WHERE c.tenant_id = $1
                  AND ($5 = 'all'
                       OR ($5 = 'active' AND c.archived_at IS NULL)
                       OR ($5 = 'archived' AND c.archived_at IS NOT NULL))
                  AND ($2 = '' OR (c.name ILIKE '%' || $2 || '%'
                                   OR c.alias ILIKE '%' || $2 || '%'
                                   OR c.billing_email ILIKE '%' || $2 || '%'))
                  AND (cardinality($3) = 0 OR (c.billing_address ->> 'country') = ANY($3))
                  AND ($8 = FALSE OR c.id = ANY($9))
            ),
            sub_flags AS (
                SELECT s.customer_id,
                    -- SUM(int8) is numeric in Postgres; cast back to bigint for the i64 mapping.
                    COALESCE(SUM(s.mrr_cents) FILTER (
                        WHERE s.status IN ('ACTIVE', 'PENDING_CHARGE')), 0)::bigint   AS mrr_cents,
                    bool_or(s.status IN ('ACTIVE', 'PENDING_CHARGE')
                            AND p.plan_type <> 'FREE')                                AS has_active_paid_sub,
                    bool_or(s.status = 'TRIAL_ACTIVE')                                AS is_trialing,
                    bool_or(s.status IN ('PENDING_ACTIVATION', 'PENDING_CHARGE', 'TRIAL_ACTIVE', 'ACTIVE', 'PAUSED')
                            AND p.plan_type = 'FREE')                                 AS has_active_free_sub,
                    bool_or(s.status IN ('PENDING_ACTIVATION', 'PENDING_CHARGE', 'TRIAL_ACTIVE', 'ACTIVE', 'PAUSED'))
                                                                                      AS has_active_sub,
                    bool_or(s.status = 'SUSPENDED')                                   AS has_suspended_sub,
                    bool_or(s.status = 'PAUSED')                                      AS has_paused_sub,
                    bool_or(s.status IN ('CANCELLED', 'COMPLETED')
                            AND p.plan_type <> 'FREE')                                AS had_paid_sub_ended,
                    bool_or(p.plan_type <> 'FREE')                                    AS ever_paid,
                    bool_or(s.status = 'TRIAL_EXPIRED')                               AS trial_expired
                FROM subscription s
                JOIN plan_version pv ON pv.id = s.plan_version_id
                JOIN plan p          ON p.id  = pv.plan_id
                WHERE s.customer_id IN (SELECT id FROM base)
                GROUP BY s.customer_id
            ),
            quote_flags AS (
                SELECT q.customer_id,
                    bool_or(q.status IN ('DECLINED', 'EXPIRED', 'CANCELLED')
                            AND q.converted_to_subscription_id IS NULL)              AS has_closed_lost_quote,
                    bool_or(q.status IN ('DRAFT', 'PENDING'))                        AS has_open_quote
                FROM quote q
                WHERE q.customer_id IN (SELECT id FROM base)
                GROUP BY q.customer_id
            ),
            derived AS (
                SELECT b.*,
                    COALESCE(sf.mrr_cents, 0)::bigint               AS mrr_cents,
                    COALESCE(sf.has_active_paid_sub, false)         AS has_active_paid_sub,
                    COALESCE(sf.is_trialing, false)                 AS is_trialing,
                    COALESCE(sf.has_active_free_sub, false)         AS has_active_free_sub,
                    COALESCE(sf.has_active_sub, false)              AS has_active_sub,
                    COALESCE(sf.has_suspended_sub, false)           AS has_suspended_sub,
                    COALESCE(sf.has_paused_sub, false)              AS has_paused_sub,
                    COALESCE(sf.had_paid_sub_ended, false)          AS had_paid_sub_ended,
                    COALESCE(sf.ever_paid, false)                   AS ever_paid,
                    COALESCE(sf.trial_expired, false)               AS trial_expired,
                    COALESCE(qf.has_closed_lost_quote, false)       AS has_closed_lost_quote,
                    COALESCE(qf.has_open_quote, false)              AS has_open_quote,
                    CASE
                        WHEN COALESCE(sf.has_suspended_sub, false)
                             AND (COALESCE(sf.has_active_paid_sub, false) OR COALESCE(sf.ever_paid, false)) THEN 'past_due'
                        WHEN COALESCE(sf.has_active_paid_sub, false) THEN 'active_paying'
                        WHEN COALESCE(sf.is_trialing, false) THEN 'trialing'
                        WHEN COALESCE(sf.has_active_free_sub, false) THEN 'free'
                        WHEN COALESCE(sf.has_paused_sub, false) THEN 'dormant'
                        WHEN COALESCE(sf.had_paid_sub_ended, false) AND NOT COALESCE(sf.has_active_sub, false) THEN 'churned'
                        WHEN COALESCE(sf.trial_expired, false) AND NOT COALESCE(sf.ever_paid, false) AND NOT COALESCE(sf.has_active_sub, false) THEN 'churned_trial'
                        WHEN COALESCE(qf.has_closed_lost_quote, false) AND NOT COALESCE(sf.has_active_sub, false) AND NOT COALESCE(sf.ever_paid, false) THEN 'closed_lost'
                        WHEN COALESCE(qf.has_open_quote, false) AND NOT COALESCE(sf.has_active_sub, false) THEN 'lead'
                        ELSE 'unknown'
                    END AS lifecycle_state
                FROM base b
                LEFT JOIN sub_flags   sf ON sf.customer_id = b.id
                LEFT JOIN quote_flags qf ON qf.customer_id = b.id
            )
            SELECT d.*, COUNT(*) OVER() AS total_count
            FROM derived d
            WHERE (cardinality($4) = 0 OR d.lifecycle_state = ANY($4))
            ORDER BY {order_sql}
            LIMIT $6 OFFSET $7
            "#
        );

        let items: Vec<CustomerWithMetricsRow> = diesel::sql_query(raw_sql)
            .bind::<sql_types::Uuid, _>(param_tenant_id)
            .bind::<sql_types::Text, _>(search)
            .bind::<sql_types::Array<sql_types::Text>, _>(param_countries)
            .bind::<sql_types::Array<sql_types::Text>, _>(param_lifecycle)
            .bind::<sql_types::Text, _>(archived_mode)
            .bind::<sql_types::BigInt, _>(per_page)
            .bind::<sql_types::BigInt, _>(offset)
            .bind::<sql_types::Bool, _>(has_id_filter)
            .bind::<sql_types::Array<sql_types::Uuid>, _>(filter_ids)
            .get_results::<CustomerWithMetricsRow>(conn)
            .await
            .attach("Error while fetching customers with metrics")
            .into_db_result()?;

        // `.iter().next()` (not `.first()`) to avoid colliding with the in-scope
        // diesel_async `RunQueryDsl::first`.
        let total_results = items.iter().map(|r| r.total_count).next().unwrap_or(0);
        let total_pages = (total_results as f64 / per_page as f64).ceil() as u32;

        Ok(PaginatedVec {
            items,
            total_pages,
            total_results: total_results as u64,
        })
    }

    /// Distinct ISO country codes in use by the tenant's non-archived customers
    /// (from the billing address), for seeding the country filter's suggestions.
    pub async fn list_distinct_countries(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
    ) -> DbResult<Vec<String>> {
        use diesel::sql_types;
        use diesel_async::RunQueryDsl;

        #[derive(diesel::QueryableByName)]
        struct CountryRow {
            #[diesel(sql_type = sql_types::Text)]
            country: String,
        }

        let rows: Vec<CountryRow> = diesel::sql_query(
            r#"
            SELECT DISTINCT (billing_address ->> 'country') AS country
            FROM customer
            WHERE tenant_id = $1
              AND archived_at IS NULL
              AND billing_address ->> 'country' IS NOT NULL
              AND billing_address ->> 'country' <> ''
            ORDER BY country
            "#,
        )
        .bind::<sql_types::Uuid, _>(param_tenant_id)
        .get_results::<CountryRow>(conn)
        .await
        .attach("Error while listing customer countries")
        .into_db_result()?;

        Ok(rows.into_iter().map(|r| r.country).collect())
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

        crate::debug_sql(&query);

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

        crate::debug_sql(&query);

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
        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while inserting customer batch")
            .into_db_result()
    }

    /// Rows are written in conflict-key order, so two batches lock the customers they update in
    /// one global order.
    pub async fn upsert_customer_batch(
        conn: &mut PgConn,
        mut batch: Vec<CustomerRowNew>,
    ) -> DbResult<Vec<CustomerRow>> {
        use crate::schema::customer::dsl::*;
        use diesel::upsert::excluded;
        use diesel_async::RunQueryDsl;

        batch.sort_by(|a, b| {
            (a.tenant_id.as_uuid(), &a.alias).cmp(&(b.tenant_id.as_uuid(), &b.alias))
        });

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
                vat_number.eq(excluded(vat_number)),
                invoicing_emails.eq(excluded(invoicing_emails)),
                tax_status.eq(excluded(tax_status)),
                exemption_reason.eq(excluded(exemption_reason)),
                custom_taxes.eq(excluded(custom_taxes)),
                vat_number_format_valid.eq(excluded(vat_number_format_valid)),
                // A row that declares custom properties replaces the stored bag. A row that
                // declares none carries a jsonb null and leaves the stored bag untouched.
                custom_properties.eq(diesel::dsl::sql::<diesel::sql_types::Jsonb>(
                    r#"CASE
                        WHEN jsonb_typeof(excluded.custom_properties) = 'object'
                            THEN excluded.custom_properties
                        ELSE "customer"."custom_properties"
                    END"#,
                )),
                // Override usually set via API/UI: apply the CSV value when the column carries
                // one, but leave the stored preferences untouched when it is absent or blank.
                preferred_locales.eq(diesel::dsl::sql::<
                    diesel::sql_types::Array<diesel::sql_types::Nullable<diesel::sql_types::Text>>,
                >(
                    r#"CASE
                        WHEN cardinality(excluded.preferred_locales) > 0
                            THEN excluded.preferred_locales
                        ELSE "customer"."preferred_locales"
                    END"#,
                )),
                updated_at.eq(common_domain::clock::now().naive_utc()),
            ));

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while upserting customer batch")
            .into_db_result()
    }

    /// The first lock of a customer's transaction (`crate::lock`).
    pub async fn lock(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: CustomerId,
    ) -> DbResult<CustomerLock> {
        use crate::schema::customer::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let query = c_dsl::customer
            .for_no_key_update()
            .select(c_dsl::id)
            .filter(c_dsl::id.eq(id))
            .filter(c_dsl::tenant_id.eq(tenant_id));

        crate::debug_sql(&query);

        let _: CustomerId = query
            .first(conn)
            .await
            .attach("Error while locking a customer")
            .into_db_result()?;
        Ok(CustomerLock::taken(conn, tenant_id, id))
    }

    /// `lock` for several customers of a tenant, in id order. Missing ids are skipped.
    pub async fn lock_many(
        conn: &mut PgConn,
        tenant_id: TenantId,
        ids: &[CustomerId],
    ) -> DbResult<Vec<CustomerLock>> {
        use crate::schema::customer::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let locked: Vec<CustomerId> = c_dsl::customer
            .for_no_key_update()
            .select(c_dsl::id)
            .filter(c_dsl::tenant_id.eq(tenant_id))
            .filter(c_dsl::id.eq_any(ids))
            .order(c_dsl::id.asc())
            .load(conn)
            .await
            .attach("Error while locking customers")
            .into_db_result()?;
        Ok(locked
            .into_iter()
            .map(|id| CustomerLock::taken(conn, tenant_id, id))
            .collect())
    }

    /// `lock_many` skipping the customers another transaction holds: the locked ones, in id order.
    pub async fn lock_many_skip_locked(
        conn: &mut PgConn,
        tenant_id: TenantId,
        ids: &[CustomerId],
    ) -> DbResult<Vec<CustomerLock>> {
        use crate::schema::customer::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let locked: Vec<CustomerId> = c_dsl::customer
            .for_no_key_update()
            .skip_locked()
            .select(c_dsl::id)
            .filter(c_dsl::tenant_id.eq(tenant_id))
            .filter(c_dsl::id.eq_any(ids))
            .order(c_dsl::id.asc())
            .load(conn)
            .await
            .attach("Error while locking customers")
            .into_db_result()?;
        Ok(locked
            .into_iter()
            .map(|id| CustomerLock::taken(conn, tenant_id, id))
            .collect())
    }

    /// `lock`, returning the customer row as well.
    pub async fn select_for_update(
        conn: &mut PgConn,
        id: CustomerId,
        tenant_id: TenantId,
    ) -> DbResult<(CustomerLock, CustomerRow)> {
        use crate::schema::customer::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let query = c_dsl::customer
            .for_no_key_update()
            .filter(c_dsl::id.eq(id))
            .filter(c_dsl::tenant_id.eq(tenant_id));

        crate::debug_sql(&query);

        let row: CustomerRow = query
            .first(conn)
            .await
            .attach("Error while selecting for update customer by id")
            .into_db_result()?;
        Ok((CustomerLock::taken(conn, tenant_id, id), row))
    }

    pub async fn archive(
        conn: &mut PgConn,
        id: CustomerId,
        tenant_id: TenantId,
    ) -> DbResult<usize> {
        use crate::schema::customer::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(c_dsl::customer)
            .filter(c_dsl::id.eq(id))
            .filter(c_dsl::tenant_id.eq(tenant_id))
            .set(c_dsl::archived_at.eq(common_domain::clock::now().naive_utc()));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while archiving customer")
            .into_db_result()
    }

    /// Null out `current_payment_method_id` on any customer still pointing at
    /// a detached method, so payment resolution stops retrying a dead mandate.
    /// Makes `payment_method_id` the default of the locked customer. Unless `replace`, only when
    /// there is no usable default: none, archived, or on a connector that is no longer active.
    /// `None` when nothing changed.
    pub async fn adopt_current_payment_method(
        conn: &mut PgConn,
        lock: &crate::lock::CustomerLock,
        payment_method_id: CustomerPaymentMethodId,
        replace: bool,
    ) -> DbResult<Option<CustomerRow>> {
        use crate::enums::ConnectorStatusEnum;
        use crate::schema::connector::dsl as k_dsl;
        use crate::schema::customer::dsl as c_dsl;
        use crate::schema::customer_connection::dsl as cc_dsl;
        use crate::schema::customer_payment_method::dsl as pm_dsl;
        use diesel_async::RunQueryDsl;

        let unusable = pm_dsl::customer_payment_method
            .inner_join(cc_dsl::customer_connection.on(cc_dsl::id.eq(pm_dsl::connection_id)))
            .inner_join(k_dsl::connector.on(k_dsl::id.eq(cc_dsl::connector_id)))
            .filter(
                pm_dsl::archived_at
                    .is_not_null()
                    .or(k_dsl::status.ne(ConnectorStatusEnum::Active)),
            )
            .select(pm_dsl::id.nullable());

        let mut query = diesel::update(c_dsl::customer)
            .filter(c_dsl::tenant_id.eq(lock.tenant_id()))
            .filter(c_dsl::id.eq(lock.customer_id()))
            .filter(c_dsl::current_payment_method_id.is_distinct_from(payment_method_id))
            .into_boxed();
        if !replace {
            query = query.filter(
                c_dsl::current_payment_method_id
                    .is_null()
                    .or(c_dsl::current_payment_method_id.eq_any(unusable)),
            );
        }
        let query = query
            .set((
                c_dsl::current_payment_method_id.eq(Some(payment_method_id)),
                c_dsl::updated_at.eq(Some(common_domain::clock::now().naive_utc())),
            ))
            .returning(CustomerRow::as_returning());

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .optional()
            .attach("Error while adopting the customer's default payment method")
            .into_db_result()
    }

    pub async fn clear_current_payment_method(
        conn: &mut PgConn,
        tenant_id: TenantId,
        payment_method_id: &CustomerPaymentMethodId,
    ) -> DbResult<usize> {
        use crate::schema::customer::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(c_dsl::customer)
            .filter(c_dsl::tenant_id.eq(tenant_id))
            .filter(c_dsl::current_payment_method_id.eq(payment_method_id))
            .set(c_dsl::current_payment_method_id.eq(None::<CustomerPaymentMethodId>));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while clearing customer current payment method")
            .into_db_result()
    }

    /// The reminder preference has its own writer so `CustomerRowPatch` stays untouched.
    pub async fn set_payment_reminder_preference(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: CustomerId,
        opt_out: bool,
        sequence_id: Option<PaymentReminderSequenceId>,
    ) -> DbResult<usize> {
        use crate::schema::customer::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(c_dsl::customer)
            .filter(c_dsl::tenant_id.eq(tenant_id))
            .filter(c_dsl::id.eq(id))
            .set((
                c_dsl::payment_reminders_opt_out.eq(opt_out),
                c_dsl::payment_reminder_sequence_id.eq(sequence_id),
                c_dsl::updated_at.eq(common_domain::clock::now().naive_utc()),
            ));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while setting customer payment reminder preference")
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
            .set(c_dsl::archived_at.eq(None::<chrono::NaiveDateTime>));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while unarchiving customer")
            .into_db_result()
    }

    /// Sets or clears the owner; the caller checks org membership, which the FK cannot.
    pub async fn assign_owner(
        conn: &mut PgConn,
        tenant_id_param: TenantId,
        customer_ids: Vec<CustomerId>,
        owner_user_id_param: Option<common_domain::ids::UserId>,
    ) -> DbResult<usize> {
        use crate::schema::customer::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(c_dsl::customer)
            .filter(c_dsl::tenant_id.eq(tenant_id_param))
            .filter(c_dsl::id.eq_any(customer_ids))
            .set((
                c_dsl::owner_user_id.eq(owner_user_id_param),
                c_dsl::updated_at.eq(common_domain::clock::now().naive_utc()),
            ));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while assigning customer owner")
            .into_db_result()
    }

    /// Member-removal cleanup: the user row survives, so `ON DELETE SET NULL`
    /// never fires. Call inside the member-removal transaction.
    pub async fn clear_owner_for_user_in_org(
        conn: &mut PgConn,
        org_id: common_domain::ids::OrganizationId,
        user_id: common_domain::ids::UserId,
    ) -> DbResult<usize> {
        use diesel::sql_types;
        use diesel_async::RunQueryDsl;

        diesel::sql_query(
            r#"
            UPDATE customer
            SET owner_user_id = NULL, updated_at = $3
            WHERE owner_user_id = $1
              AND tenant_id IN (SELECT id FROM tenant WHERE organization_id = $2)
            "#,
        )
        .bind::<sql_types::Uuid, _>(user_id)
        .bind::<sql_types::Uuid, _>(org_id)
        .bind::<sql_types::Timestamp, _>(common_domain::clock::now().naive_utc())
        .execute(conn)
        .await
        .attach("Error while clearing customer owner for removed member")
        .into_db_result()
    }

    /// The tenant's customers among `customer_ids`: id, name and whether it is archived. An id
    /// that is not one of the tenant's customers is left out.
    pub async fn list_archived_flags_by_ids(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_ids: &[CustomerId],
    ) -> DbResult<Vec<(CustomerId, String, bool)>> {
        use crate::schema::customer::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let query = c_dsl::customer
            .filter(c_dsl::tenant_id.eq(tenant_id))
            .filter(c_dsl::id.eq_any(customer_ids.to_vec()))
            .select((c_dsl::id, c_dsl::name, c_dsl::archived_at.is_not_null()));

        crate::debug_sql(&query);

        query
            .load(conn)
            .await
            .attach("Error while loading customers by ids")
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

        crate::debug_sql(&query);

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

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .optional()
            .tap_err(|e| log::error!("Error while patching customer: {e:?}"))
            .attach("Error while patching customer")
            .into_db_result()
    }

    /// Persists the outcome of an external (VIES) VAT check for a single customer.
    ///
    /// The update is conditional on the row still holding the exact number we
    /// validated, so a concurrent VAT-number change (which resets the status)
    /// is never clobbered with a stale result. `vies_check` (audit evidence of a
    /// definitive answer) is only written when present, so an outage never
    /// erases the evidence of the last real answer.
    pub async fn update_vat_validation(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        customer_id: CustomerId,
        expected_vat_number: &str,
        status: crate::enums::CustomerVatValidationStatusEnum,
        checked_at: chrono::NaiveDateTime,
        vies_check: Option<serde_json::Value>,
    ) -> DbResult<()> {
        use crate::schema::customer::dsl as c;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(c::customer)
            .filter(c::id.eq(customer_id))
            .filter(c::tenant_id.eq(param_tenant_id))
            .filter(c::vat_number.eq(expected_vat_number));

        let res = match vies_check {
            Some(check) => {
                query
                    .set((
                        c::vat_number_validation_status.eq(status),
                        c::vat_number_checked_at.eq(checked_at),
                        c::vat_number_vies_check.eq(check),
                    ))
                    .execute(conn)
                    .await
            }
            None => {
                query
                    .set((
                        c::vat_number_validation_status.eq(status),
                        c::vat_number_checked_at.eq(checked_at),
                    ))
                    .execute(conn)
                    .await
            }
        };

        res.tap_err(|e| log::error!("Error while updating vat validation: {e:?}"))
            .attach("Error while updating vat validation")
            .into_db_result()
            .map(|_| ())
    }

    /// Moves a customer's VAT validation back to `PENDING` ahead of a manual
    /// re-check, guarded on the number so a concurrent edit is not overridden.
    /// Returns the number of rows updated (0 = number changed under us).
    pub async fn mark_vat_validation_pending(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        customer_id: CustomerId,
        expected_vat_number: &str,
    ) -> DbResult<usize> {
        use crate::enums::CustomerVatValidationStatusEnum;
        use crate::schema::customer::dsl as c;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(c::customer)
            .filter(c::id.eq(customer_id))
            .filter(c::tenant_id.eq(param_tenant_id))
            .filter(c::vat_number.eq(expected_vat_number))
            .set(c::vat_number_validation_status.eq(CustomerVatValidationStatusEnum::Pending));

        query
            .execute(conn)
            .await
            .tap_err(|e| log::error!("Error while marking vat validation pending: {e:?}"))
            .attach("Error while marking vat validation pending")
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

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .optional()
            .tap_err(|e| log::error!("Error while updating customer: {e:?}"))
            .attach("Error while updating customer")
            .into_db_result()
    }
}

impl CustomerLifecycleSignalRow {
    /// Single-customer variant of `CustomerProjectionRow::fetch_after`'s signal
    /// columns: the same aggregate passes, scoped to one `customer_id`.
    pub async fn fetch_for_customer(
        conn: &mut PgConn,
        customer_id: uuid::Uuid,
        tenant_id: uuid::Uuid,
    ) -> DbResult<Option<CustomerLifecycleSignalRow>> {
        use diesel_async::RunQueryDsl;

        let raw_sql = r"
            SELECT COALESCE(sub.has_active_paid_sub, false)       AS has_active_paid_sub,
                   COALESCE(sub.is_trialing, false)               AS is_trialing,
                   COALESCE(sub.has_active_free_sub, false)       AS has_active_free_sub,
                   COALESCE(sub.has_active_sub, false)            AS has_active_sub,
                   COALESCE(sub.has_suspended_sub, false)         AS has_suspended_sub,
                   COALESCE(sub.has_paused_sub, false)            AS has_paused_sub,
                   COALESCE(sub.had_paid_sub_ended, false)        AS had_paid_sub_ended,
                   COALESCE(sub.ever_paid, false)                 AS ever_paid,
                   COALESCE(sub.trial_expired, false)             AS trial_expired,
                   COALESCE(qt.has_closed_lost_quote, false)      AS has_closed_lost_quote,
                   COALESCE(qt.has_open_quote, false)             AS has_open_quote,
                   COALESCE(sub.min_paid_ended_end IS NOT NULL
                            AND sub.max_active_start IS NOT NULL
                            AND sub.min_paid_ended_end <= sub.max_active_start, false)
                                                                  AS reactivated
            FROM customer c
            LEFT JOIN LATERAL (
                SELECT
                    bool_or(s.status IN ('ACTIVE', 'PENDING_CHARGE') AND p.plan_type <> 'FREE') AS has_active_paid_sub,
                    bool_or(s.status = 'TRIAL_ACTIVE') AS is_trialing,
                    bool_or(s.status IN ('PENDING_ACTIVATION', 'PENDING_CHARGE', 'TRIAL_ACTIVE', 'ACTIVE', 'PAUSED') AND p.plan_type = 'FREE') AS has_active_free_sub,
                    bool_or(s.status IN ('PENDING_ACTIVATION', 'PENDING_CHARGE', 'TRIAL_ACTIVE', 'ACTIVE', 'PAUSED')) AS has_active_sub,
                    bool_or(s.status = 'SUSPENDED') AS has_suspended_sub,
                    bool_or(s.status = 'PAUSED') AS has_paused_sub,
                    bool_or(s.status IN ('CANCELLED', 'COMPLETED') AND p.plan_type <> 'FREE') AS had_paid_sub_ended,
                    bool_or(p.plan_type <> 'FREE') AS ever_paid,
                    bool_or(s.status = 'TRIAL_EXPIRED') AS trial_expired,
                    min(s.end_date) FILTER (WHERE s.status IN ('CANCELLED', 'COMPLETED') AND p.plan_type <> 'FREE' AND s.end_date IS NOT NULL) AS min_paid_ended_end,
                    max(s.start_date) FILTER (WHERE s.status IN ('PENDING_ACTIVATION', 'PENDING_CHARGE', 'TRIAL_ACTIVE', 'ACTIVE', 'PAUSED')) AS max_active_start
                FROM subscription s
                JOIN plan_version pv ON pv.id = s.plan_version_id
                JOIN plan p ON p.id = pv.plan_id
                WHERE s.customer_id = c.id
            ) sub ON true
            LEFT JOIN LATERAL (
                SELECT
                    bool_or(q.status IN ('DECLINED', 'EXPIRED', 'CANCELLED') AND q.converted_to_subscription_id IS NULL) AS has_closed_lost_quote,
                    bool_or(q.status IN ('DRAFT', 'PENDING')) AS has_open_quote
                FROM quote q
                WHERE q.customer_id = c.id
            ) qt ON true
            WHERE c.id = $1 AND c.tenant_id = $2
        ";

        diesel::sql_query(raw_sql)
            .bind::<diesel::sql_types::Uuid, _>(customer_id)
            .bind::<diesel::sql_types::Uuid, _>(tenant_id)
            .get_result::<CustomerLifecycleSignalRow>(conn)
            .await
            .optional()
            .attach("Error fetching customer lifecycle signals")
            .into_db_result()
    }
}

pub async fn connected_account_platform_org(
    conn: &mut PgConn,
    account_id: ConnectedAccountId,
) -> DbResult<Option<OrganizationId>> {
    use crate::schema::connect_connected_account::dsl as cca;
    use diesel_async::RunQueryDsl;

    let query = cca::connect_connected_account
        .filter(cca::id.eq(account_id))
        .select(cca::platform_organization_id);

    crate::debug_sql(&query);

    query
        .first(conn)
        .await
        .optional()
        .attach("Error while getting platform org id for connected account")
        .into_db_result()
}
