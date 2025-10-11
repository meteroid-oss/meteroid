use crate::enums::QuoteStatusEnum;
use crate::errors::IntoDbResult;
use crate::extend::order::OrderByRequest;
use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};
use crate::quotes::{
    QuoteActivityRow, QuoteActivityRowNew, QuoteComponentRow, QuoteComponentRowNew, QuoteRow,
    QuoteRowNew, QuoteRowUpdate, QuoteSignatureRow, QuoteSignatureRowNew, QuoteWithCustomerRow,
};
use crate::{DbResult, PgConn};
use common_domain::ids::{CustomerId, QuoteId, StoredDocumentId, TenantId};
use diesel::{
    BoolExpressionMethods, JoinOnDsl, PgTextExpressionMethods, SelectableHelper, debug_query,
};
use diesel::{ExpressionMethods, QueryDsl};
use error_stack::ResultExt;

impl QuoteRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<QuoteRow> {
        use crate::schema::quote::dsl::quote;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(quote).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while inserting quote")
            .into_db_result()
    }

    pub async fn insert_batch(rows: &[QuoteRowNew], conn: &mut PgConn) -> DbResult<Vec<QuoteRow>> {
        use crate::schema::quote::dsl::quote;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(quote).values(rows);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while batch inserting quotes")
            .into_db_result()
    }
}

impl QuoteRow {
    pub async fn find_by_id(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_quote_id: QuoteId,
    ) -> DbResult<QuoteRow> {
        use crate::schema::quote::dsl::{id, quote, tenant_id};
        use diesel_async::RunQueryDsl;

        let query = quote
            .filter(tenant_id.eq(param_tenant_id))
            .filter(id.eq(param_quote_id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach("Error while finding quote by id")
            .into_db_result()
    }

    pub async fn find_with_customer_by_id(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_quote_id: QuoteId,
    ) -> DbResult<QuoteWithCustomerRow> {
        use crate::schema::customer::dsl as c_dsl;
        use crate::schema::quote::dsl as q_dsl;
        use diesel_async::RunQueryDsl;

        let query = q_dsl::quote
            .inner_join(c_dsl::customer.on(q_dsl::customer_id.eq(c_dsl::id)))
            .filter(q_dsl::tenant_id.eq(param_tenant_id))
            .filter(q_dsl::id.eq(param_quote_id))
            .select(QuoteWithCustomerRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach("Error while finding quote with customer by id")
            .into_db_result()
    }

    pub async fn list(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_customer_id: Option<CustomerId>,
        param_status: Option<QuoteStatusEnum>,
        search: Option<String>,
        order_by: OrderByRequest,
        pagination: PaginationRequest,
    ) -> DbResult<PaginatedVec<QuoteWithCustomerRow>> {
        use crate::schema::customer::dsl as c_dsl;
        use crate::schema::quote::dsl as q_dsl;

        let mut query = q_dsl::quote
            .inner_join(c_dsl::customer.on(q_dsl::customer_id.eq(c_dsl::id)))
            .filter(q_dsl::tenant_id.eq(param_tenant_id))
            .into_boxed();

        if let Some(customer_id) = param_customer_id {
            query = query.filter(q_dsl::customer_id.eq(customer_id));
        }

        if let Some(status) = param_status {
            query = query.filter(q_dsl::status.eq(status));
        }

        if let Some(search_str) = search {
            let search_pattern = format!("%{search_str}%");
            query = query.filter(
                q_dsl::quote_number
                    .ilike(search_pattern.clone())
                    .or(q_dsl::internal_notes.ilike(search_pattern)),
            );
        }

        let query = match order_by {
            OrderByRequest::DateAsc => query.order(q_dsl::created_at.asc()),
            OrderByRequest::DateDesc => query.order(q_dsl::created_at.desc()),
            OrderByRequest::IdAsc => query.order(q_dsl::id.asc()),
            OrderByRequest::IdDesc => query.order(q_dsl::id.desc()),
            OrderByRequest::NameAsc => query.order(q_dsl::quote_number.asc()),
            OrderByRequest::NameDesc => query.order(q_dsl::quote_number.desc()),
        };

        let query = query.select(QuoteWithCustomerRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .paginate(pagination)
            .load_and_count_pages(conn)
            .await
            .attach("Error while listing quotes")
            .into_db_result()
    }

    pub async fn list_by_ids(conn: &mut PgConn, ids: Vec<QuoteId>) -> DbResult<Vec<QuoteRow>> {
        use crate::schema::quote::dsl::{id, quote};
        use diesel_async::RunQueryDsl;

        let query = quote.filter(id.eq_any(ids));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .load(conn)
            .await
            .attach("Error while listing quotes by ids")
            .into_db_result()
    }

    pub async fn update_by_id(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_quote_id: QuoteId,
        update: QuoteRowUpdate,
    ) -> DbResult<QuoteRow> {
        use crate::schema::quote::dsl::{id, quote, tenant_id};
        use diesel_async::RunQueryDsl;

        let query = diesel::update(quote)
            .filter(id.eq(param_quote_id))
            .filter(tenant_id.eq(param_tenant_id))
            .set(update);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while updating quote")
            .into_db_result()
    }

    pub async fn update_documents(
        conn: &mut PgConn,
        param_quote_id: QuoteId,
        param_tenant_id: TenantId,
        pdf_id: StoredDocumentId,
        param_sharing_key: String,
    ) -> DbResult<()> {
        use crate::schema::quote::dsl::{
            id, pdf_document_id, quote, sharing_key, tenant_id, updated_at,
        };
        use diesel_async::RunQueryDsl;

        let query = diesel::update(quote)
            .filter(id.eq(param_quote_id))
            .filter(tenant_id.eq(param_tenant_id))
            .set((
                pdf_document_id.eq(Some(pdf_id)),
                sharing_key.eq(Some(param_sharing_key)),
                updated_at.eq(chrono::Utc::now().naive_utc()),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while updating quote documents")
            .into_db_result()
            .map(|_| ())
    }

    pub async fn mark_as_converted_to_subscription(
        conn: &mut PgConn,
        param_quote_id: QuoteId,
        param_tenant_id: TenantId,
        subscription_id: common_domain::ids::SubscriptionId,
    ) -> DbResult<()> {
        use crate::schema::quote::dsl::{
            converted_at, converted_to_subscription_id, id, quote, status, tenant_id, updated_at,
        };
        use diesel_async::RunQueryDsl;

        let now = chrono::Utc::now().naive_utc();

        let query = diesel::update(quote)
            .filter(id.eq(param_quote_id))
            .filter(tenant_id.eq(param_tenant_id))
            .filter(status.eq(QuoteStatusEnum::Accepted))
            .set((
                converted_to_subscription_id.eq(Some(subscription_id)),
                converted_at.eq(Some(now)),
                updated_at.eq(Some(now)),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while converting quote to subscription")
            .into_db_result()
            .map(|_| ())
    }
}

impl QuoteSignatureRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<QuoteSignatureRow> {
        use crate::schema::quote_signature::dsl::quote_signature;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(quote_signature).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while inserting quote signature")
            .into_db_result()
    }
}

impl QuoteSignatureRow {
    pub async fn list_by_quote_id(
        conn: &mut PgConn,
        param_quote_id: QuoteId,
    ) -> DbResult<Vec<QuoteSignatureRow>> {
        use crate::schema::quote_signature::dsl::{quote_id, quote_signature, signed_at};
        use diesel_async::RunQueryDsl;

        let query = quote_signature
            .filter(quote_id.eq(param_quote_id))
            .order(signed_at.desc());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .load(conn)
            .await
            .attach("Error while listing quote signatures")
            .into_db_result()
    }
}

impl QuoteActivityRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<QuoteActivityRow> {
        use crate::schema::quote_activity::dsl::quote_activity;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(quote_activity).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while inserting quote activity")
            .into_db_result()
    }
}

impl QuoteActivityRow {
    pub async fn list_by_quote_id(
        conn: &mut PgConn,
        param_quote_id: QuoteId,
        limit: Option<i64>,
    ) -> DbResult<Vec<QuoteActivityRow>> {
        use crate::schema::quote_activity::dsl::{created_at, quote_activity, quote_id};
        use diesel_async::RunQueryDsl;

        let mut query = quote_activity
            .filter(quote_id.eq(param_quote_id))
            .order(created_at.desc())
            .into_boxed();

        if let Some(limit_val) = limit {
            query = query.limit(limit_val);
        }

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .load(conn)
            .await
            .attach("Error while listing quote activities")
            .into_db_result()
    }
}

impl QuoteComponentRow {
    pub async fn list_by_quote_id(
        conn: &mut PgConn,
        param_quote_id: QuoteId,
    ) -> DbResult<Vec<QuoteComponentRow>> {
        use crate::schema::quote_component::dsl::{id, quote_component, quote_id};
        use diesel_async::RunQueryDsl;

        let query = quote_component
            .filter(quote_id.eq(param_quote_id))
            .order(id.asc());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .load(conn)
            .await
            .attach("Error while listing quote components")
            .into_db_result()
    }
}

impl QuoteComponentRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<QuoteComponentRow> {
        use crate::schema::quote_component::dsl::quote_component;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(quote_component).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while inserting quote component")
            .into_db_result()
    }

    pub async fn insert_batch(
        rows: &[QuoteComponentRowNew],
        conn: &mut PgConn,
    ) -> DbResult<Vec<QuoteComponentRow>> {
        use crate::schema::quote_component::dsl::quote_component;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(quote_component).values(rows);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while batch inserting quote components")
            .into_db_result()
    }
}
