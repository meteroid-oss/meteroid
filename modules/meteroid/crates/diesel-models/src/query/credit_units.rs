use crate::credit_units::{
    CreditUnitPriceRow, CreditUnitPriceRowNew, CreditUnitRow, CreditUnitRowNew, CreditUnitRowPatch,
};
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};
use chrono::Utc;
use common_domain::ids::{CreditUnitId, CreditUnitPriceId, CustomerId, SubscriptionId, TenantId};
use diesel::sql_types;
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, QueryableByName, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl CreditUnitRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<CreditUnitRow> {
        use crate::schema::credit_unit::dsl as u;

        let query = diesel::insert_into(u::credit_unit)
            .values(self)
            .returning(CreditUnitRow::as_returning());
        crate::debug_sql(&query);
        query
            .get_result(conn)
            .await
            .attach("Error while inserting credit unit")
            .into_db_result()
    }
}

impl CreditUnitRow {
    pub async fn find_by_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: CreditUnitId,
    ) -> DbResult<Option<CreditUnitRow>> {
        use crate::schema::credit_unit::dsl as u;

        let query = u::credit_unit
            .filter(u::tenant_id.eq(tenant_id))
            .filter(u::id.eq(id))
            .select(CreditUnitRow::as_select());
        crate::debug_sql(&query);
        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding credit unit")
            .into_db_result()
    }

    /// Archived units still resolve: their wallets and prices keep their denomination.
    pub async fn find_by_code(
        conn: &mut PgConn,
        tenant_id: TenantId,
        code: &str,
    ) -> DbResult<Option<CreditUnitRow>> {
        use crate::schema::credit_unit::dsl as u;

        let query = u::credit_unit
            .filter(u::tenant_id.eq(tenant_id))
            .filter(u::code.eq(code))
            .select(CreditUnitRow::as_select());
        crate::debug_sql(&query);
        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding credit unit by code")
            .into_db_result()
    }

    pub async fn list_by_ids(
        conn: &mut PgConn,
        tenant_id: TenantId,
        ids: &[CreditUnitId],
    ) -> DbResult<Vec<CreditUnitRow>> {
        use crate::schema::credit_unit::dsl as u;

        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let query = u::credit_unit
            .filter(u::tenant_id.eq(tenant_id))
            .filter(u::id.eq_any(ids))
            .select(CreditUnitRow::as_select());
        crate::debug_sql(&query);
        query
            .load(conn)
            .await
            .attach("Error while listing credit units by id")
            .into_db_result()
    }

    /// Ordered by code.
    pub async fn list_for_tenant(
        conn: &mut PgConn,
        tenant_id: TenantId,
        include_archived: bool,
    ) -> DbResult<Vec<CreditUnitRow>> {
        use crate::schema::credit_unit::dsl as u;

        let mut query = u::credit_unit
            .filter(u::tenant_id.eq(tenant_id))
            .order(u::code.asc())
            .select(CreditUnitRow::as_select())
            .into_boxed();
        if !include_archived {
            query = query.filter(u::archived_at.is_null());
        }
        crate::debug_sql(&query);
        query
            .load(conn)
            .await
            .attach("Error while listing credit units")
            .into_db_result()
    }

    /// Whether `customer_id` already uses this unit: a wallet in it, or an active subscription
    /// with a component or add-on priced in it. A budget in a unit the customer has no stake in
    /// would offer every tenant unit to every customer, so callers gate on this first.
    pub async fn is_used_by_customer(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        credit_unit_id: CreditUnitId,
    ) -> DbResult<bool> {
        #[derive(QueryableByName)]
        struct Row {
            #[diesel(sql_type = sql_types::Bool)]
            used: bool,
        }
        diesel::sql_query(
            "SELECT (
                 EXISTS (SELECT 1 FROM customer_credit_account a
                         WHERE a.tenant_id = $1 AND a.customer_id = $2 AND a.credit_unit_id = $3)
                 OR EXISTS (SELECT 1 FROM subscription_component sc
                            JOIN subscription s ON s.id = sc.subscription_id
                            JOIN price p ON p.id = sc.price_id
                            WHERE s.tenant_id = $1 AND s.customer_id = $2 AND p.credit_unit_id = $3
                              AND s.status::text IN ('ACTIVE', 'TRIAL_ACTIVE', 'PENDING_CHARGE'))
                 OR EXISTS (SELECT 1 FROM subscription_add_on sao
                            JOIN subscription s ON s.id = sao.subscription_id
                            JOIN price p ON p.id = sao.price_id
                            WHERE s.tenant_id = $1 AND s.customer_id = $2 AND p.credit_unit_id = $3
                              AND s.status::text IN ('ACTIVE', 'TRIAL_ACTIVE', 'PENDING_CHARGE'))
             ) AS used",
        )
        .bind::<sql_types::Uuid, _>(tenant_id)
        .bind::<sql_types::Uuid, _>(customer_id)
        .bind::<sql_types::Uuid, _>(credit_unit_id)
        .get_result::<Row>(conn)
        .await
        .map(|row| row.used)
        .attach("Error while checking whether a credit unit is used by a customer")
        .into_db_result()
    }

    /// Whether a component or add-on of the subscription is priced in the unit.
    pub async fn is_priced_by_subscription(
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        credit_unit_id: CreditUnitId,
    ) -> DbResult<bool> {
        #[derive(QueryableByName)]
        struct Row {
            #[diesel(sql_type = sql_types::Bool)]
            used: bool,
        }
        diesel::sql_query(
            "SELECT (
                 EXISTS (SELECT 1 FROM subscription_component sc
                         JOIN subscription s ON s.id = sc.subscription_id
                         JOIN price p ON p.id = sc.price_id
                         WHERE s.tenant_id = $1 AND s.id = $2 AND p.credit_unit_id = $3)
                 OR EXISTS (SELECT 1 FROM subscription_add_on sao
                            JOIN subscription s ON s.id = sao.subscription_id
                            JOIN price p ON p.id = sao.price_id
                            WHERE s.tenant_id = $1 AND s.id = $2 AND p.credit_unit_id = $3)
             ) AS used",
        )
        .bind::<sql_types::Uuid, _>(tenant_id)
        .bind::<sql_types::Uuid, _>(subscription_id)
        .bind::<sql_types::Uuid, _>(credit_unit_id)
        .get_result::<Row>(conn)
        .await
        .map(|row| row.used)
        .attach("Error while checking whether a subscription is priced in a credit unit")
        .into_db_result()
    }

    /// None when the unit does not exist for this tenant.
    pub async fn update(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: CreditUnitId,
        patch: &CreditUnitRowPatch,
    ) -> DbResult<Option<CreditUnitRow>> {
        use crate::schema::credit_unit::dsl as u;

        let query = diesel::update(
            u::credit_unit
                .filter(u::tenant_id.eq(tenant_id))
                .filter(u::id.eq(id)),
        )
        .set((patch, u::updated_at.eq(Utc::now())))
        .returning(CreditUnitRow::as_returning());
        crate::debug_sql(&query);
        query
            .get_result(conn)
            .await
            .optional()
            .attach("Error while updating credit unit")
            .into_db_result()
    }
}

impl CreditUnitRow {
    /// Serializes changes to the unit's selling prices.
    pub async fn lock_by_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: CreditUnitId,
    ) -> DbResult<Option<CreditUnitRow>> {
        use crate::schema::credit_unit::dsl as u;

        let query = u::credit_unit
            .filter(u::tenant_id.eq(tenant_id))
            .filter(u::id.eq(id))
            .select(CreditUnitRow::as_select())
            .for_update();
        crate::debug_sql(&query);
        query
            .first(conn)
            .await
            .optional()
            .attach("Error while locking credit unit")
            .into_db_result()
    }
}

impl CreditUnitPriceRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<CreditUnitPriceRow> {
        use crate::schema::credit_unit_price::dsl as p;

        let query = diesel::insert_into(p::credit_unit_price)
            .values(self)
            .returning(CreditUnitPriceRow::as_returning());
        crate::debug_sql(&query);
        query
            .get_result(conn)
            .await
            .attach("Error while inserting credit unit price")
            .into_db_result()
    }
}

impl CreditUnitPriceRow {
    pub async fn find_active(
        conn: &mut PgConn,
        tenant_id: TenantId,
        credit_unit_id: CreditUnitId,
        currency: &str,
    ) -> DbResult<Option<CreditUnitPriceRow>> {
        use crate::schema::credit_unit_price::dsl as p;

        let query = p::credit_unit_price
            .filter(p::tenant_id.eq(tenant_id))
            .filter(p::credit_unit_id.eq(credit_unit_id))
            .filter(p::currency.eq(currency))
            .filter(p::archived_at.is_null())
            .select(CreditUnitPriceRow::as_select());
        crate::debug_sql(&query);
        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding the active credit unit price")
            .into_db_result()
    }

    /// Archived prices still resolve: an operation that pinned one keeps its terms.
    pub async fn find_by_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: CreditUnitPriceId,
    ) -> DbResult<Option<CreditUnitPriceRow>> {
        use crate::schema::credit_unit_price::dsl as p;

        let query = p::credit_unit_price
            .filter(p::tenant_id.eq(tenant_id))
            .filter(p::id.eq(id))
            .select(CreditUnitPriceRow::as_select());
        crate::debug_sql(&query);
        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding credit unit price")
            .into_db_result()
    }

    /// By unit, then currency, newest first.
    pub async fn list_by_units(
        conn: &mut PgConn,
        tenant_id: TenantId,
        credit_unit_ids: &[CreditUnitId],
        include_archived: bool,
    ) -> DbResult<Vec<CreditUnitPriceRow>> {
        use crate::schema::credit_unit_price::dsl as p;

        if credit_unit_ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut query = p::credit_unit_price
            .filter(p::tenant_id.eq(tenant_id))
            .filter(p::credit_unit_id.eq_any(credit_unit_ids))
            .order((
                p::credit_unit_id.asc(),
                p::currency.asc(),
                p::created_at.desc(),
            ))
            .select(CreditUnitPriceRow::as_select())
            .into_boxed();
        if !include_archived {
            query = query.filter(p::archived_at.is_null());
        }
        crate::debug_sql(&query);
        query
            .load(conn)
            .await
            .attach("Error while listing credit unit prices")
            .into_db_result()
    }

    /// Archives the unit's active price in `currency`, if any.
    pub async fn archive_active(
        conn: &mut PgConn,
        tenant_id: TenantId,
        credit_unit_id: CreditUnitId,
        currency: &str,
    ) -> DbResult<Option<CreditUnitPriceRow>> {
        use crate::schema::credit_unit_price::dsl as p;

        let query = diesel::update(
            p::credit_unit_price
                .filter(p::tenant_id.eq(tenant_id))
                .filter(p::credit_unit_id.eq(credit_unit_id))
                .filter(p::currency.eq(currency))
                .filter(p::archived_at.is_null()),
        )
        .set(p::archived_at.eq(Utc::now()))
        .returning(CreditUnitPriceRow::as_returning());
        crate::debug_sql(&query);
        query
            .get_result(conn)
            .await
            .optional()
            .attach("Error while archiving credit unit price")
            .into_db_result()
    }
}
