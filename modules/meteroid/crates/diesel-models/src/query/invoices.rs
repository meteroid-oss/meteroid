use crate::errors::IntoDbResult;
use crate::invoices::{
    DetailedInvoiceRow, InvoiceLockRow, InvoiceRow, InvoiceRowLinesPatch, InvoiceRowNew,
    InvoiceWithCustomerRow,
};

use crate::{DbResult, PgConn};

use crate::enums::{ConnectorProviderEnum, InvoicePaymentStatus, InvoiceStatusEnum};
use crate::extend::connection_metadata;
use crate::extend::cursor_pagination::{
    CursorPaginate, CursorPaginatedVec, CursorPaginationRequest,
};
use crate::extend::order::OrderByRequest;
use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};
use common_domain::ids::{
    BaseId, ConnectorId, CustomerId, InvoiceId, StoredDocumentId, SubscriptionId, TenantId,
};
use diesel::dsl::IntervalDsl;
use diesel::{
    BoolExpressionMethods, JoinOnDsl, NullableExpressionMethods, PgTextExpressionMethods,
    SelectableHelper, debug_query,
};
use diesel::{ExpressionMethods, QueryDsl};
use error_stack::ResultExt;

impl InvoiceRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<InvoiceRow> {
        use crate::schema::invoice::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(invoice).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting invoice")
            .into_db_result()
    }
}

impl InvoiceRow {
    /// locks the invoice, the customer (for the balance) and the subscription
    pub async fn select_for_update_by_id(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_invoice_id: InvoiceId,
    ) -> DbResult<InvoiceLockRow> {
        use crate::schema::customer::dsl as c_dsl;
        use crate::schema::invoice::dsl as i_dsl;

        use diesel_async::RunQueryDsl;

        let query = i_dsl::invoice
            .inner_join(c_dsl::customer.on(i_dsl::customer_id.eq(c_dsl::id)))
            .select((
                InvoiceRow::as_select(),
                c_dsl::balance_value_cents,
                c_dsl::invoicing_entity_id,
            ))
            .filter(i_dsl::tenant_id.eq(param_tenant_id))
            .filter(i_dsl::id.eq(param_invoice_id))
            .for_update();

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach_printable("Error while locking invoice by id")
            .into_db_result()
    }

    pub async fn find_by_id(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_invoice_id: InvoiceId,
    ) -> DbResult<InvoiceRow> {
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        let query = i_dsl::invoice
            .filter(i_dsl::tenant_id.eq(param_tenant_id))
            .filter(i_dsl::id.eq(param_invoice_id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach_printable("Error while finding invoice by id")
            .into_db_result()
    }
    pub async fn find_detailed_by_id(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_invoice_id: InvoiceId,
    ) -> DbResult<DetailedInvoiceRow> {
        use crate::schema::customer::dsl as c_dsl;
        use crate::schema::invoice::dsl as i_dsl;
        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::plan_version::dsl as pv_dsl;
        use crate::schema::product_family::dsl as pf_dsl;
        use crate::schema::subscription::dsl as s_dsl;

        use diesel_async::RunQueryDsl;

        let query = i_dsl::invoice
            .inner_join(c_dsl::customer.on(i_dsl::customer_id.eq(c_dsl::id)))
            .left_join(s_dsl::subscription.on(i_dsl::subscription_id.eq(s_dsl::id.nullable())))
            .left_join(pv_dsl::plan_version.on(s_dsl::plan_version_id.eq(pv_dsl::id)))
            .left_join(p_dsl::plan.on(pv_dsl::plan_id.eq(p_dsl::id)))
            .left_join(pf_dsl::product_family.on(p_dsl::product_family_id.eq(pf_dsl::id)))
            .filter(i_dsl::tenant_id.eq(param_tenant_id))
            .filter(i_dsl::id.eq(param_invoice_id))
            .select(DetailedInvoiceRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach_printable("Error while finding invoice by id")
            .into_db_result()
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn list(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_customer_id: Option<CustomerId>,
        param_subscription_id: Option<SubscriptionId>,
        param_status: Option<InvoiceStatusEnum>,
        param_query: Option<String>,
        order_by: OrderByRequest,
        pagination: PaginationRequest,
    ) -> DbResult<PaginatedVec<InvoiceWithCustomerRow>> {
        use crate::schema::customer::dsl as c_dsl;
        use crate::schema::invoice::dsl as i_dsl;

        let mut query = i_dsl::invoice
            .inner_join(c_dsl::customer.on(i_dsl::customer_id.eq(c_dsl::id)))
            .filter(i_dsl::tenant_id.eq(param_tenant_id))
            .select(InvoiceWithCustomerRow::as_select())
            .into_boxed();

        if let Some(param_customer_id) = param_customer_id {
            query = query.filter(i_dsl::customer_id.eq(param_customer_id))
        }

        if let Some(param_subscription_id) = param_subscription_id {
            query = query.filter(i_dsl::subscription_id.eq(param_subscription_id))
        }

        if let Some(param_status) = param_status {
            query = query.filter(i_dsl::status.eq(param_status))
        }

        if let Some(param_query) = param_query {
            query = query.filter(c_dsl::name.ilike(format!("%{}%", param_query)))
        }

        match order_by {
            OrderByRequest::IdAsc => query = query.order(i_dsl::id.asc()),
            OrderByRequest::IdDesc => query = query.order(i_dsl::id.desc()),
            OrderByRequest::DateAsc => query = query.order(i_dsl::created_at.asc()),
            OrderByRequest::DateDesc => query = query.order(i_dsl::created_at.desc()),
            _ => query = query.order(i_dsl::id.asc()),
        }

        let paginated_query = query.paginate(pagination);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&paginated_query));

        paginated_query
            .load_and_count_pages(conn)
            .await
            .attach_printable("Error while fetching invoices")
            .into_db_result()
    }

    pub async fn list_by_ids(
        conn: &mut PgConn,
        param_invoice_ids: Vec<InvoiceId>,
    ) -> DbResult<Vec<InvoiceRow>> {
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        let query = i_dsl::invoice
            .filter(i_dsl::id.eq_any(param_invoice_ids))
            .select(InvoiceRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .load(conn)
            .await
            .attach_printable("Error while fetching invoices by ids")
            .into_db_result()
    }

    pub async fn list_detailed_by_ids(
        conn: &mut PgConn,
        param_invoice_ids: Vec<InvoiceId>,
    ) -> DbResult<Vec<DetailedInvoiceRow>> {
        use crate::schema::customer::dsl as c_dsl;
        use crate::schema::invoice::dsl as i_dsl;
        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::plan_version::dsl as pv_dsl;
        use crate::schema::product_family::dsl as pf_dsl;
        use crate::schema::subscription::dsl as s_dsl;

        use diesel_async::RunQueryDsl;

        let query = i_dsl::invoice
            .inner_join(c_dsl::customer.on(i_dsl::customer_id.eq(c_dsl::id)))
            .left_join(s_dsl::subscription.on(i_dsl::subscription_id.eq(s_dsl::id.nullable())))
            .left_join(pv_dsl::plan_version.on(s_dsl::plan_version_id.eq(pv_dsl::id)))
            .left_join(p_dsl::plan.on(pv_dsl::plan_id.eq(p_dsl::id)))
            .left_join(pf_dsl::product_family.on(p_dsl::product_family_id.eq(pf_dsl::id)))
            .filter(i_dsl::id.eq_any(param_invoice_ids))
            .select(DetailedInvoiceRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .load(conn)
            .await
            .attach_printable("Error while finding invoice by id")
            .into_db_result()
    }

    pub async fn insert_invoice_batch(
        conn: &mut PgConn,
        invoices: Vec<InvoiceRowNew>,
    ) -> DbResult<Vec<InvoiceRow>> {
        use crate::schema::invoice::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(invoice).values(&invoices);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach_printable("Error while inserting invoice")
            .into_db_result()
    }

    pub async fn list_to_finalize(
        conn: &mut PgConn,
        pagination: CursorPaginationRequest,
    ) -> DbResult<CursorPaginatedVec<InvoiceRow>> {
        use crate::schema::customer::dsl as c_dsl;
        use crate::schema::invoice::dsl as i_dsl;
        use crate::schema::invoicing_entity::dsl as ie_dsl;

        let query = i_dsl::invoice
            .inner_join(c_dsl::customer.on(i_dsl::customer_id.eq(c_dsl::id)))
            .inner_join(ie_dsl::invoicing_entity.on(c_dsl::invoicing_entity_id.eq(ie_dsl::id)))
            .filter(
                i_dsl::status.ne_all(vec![InvoiceStatusEnum::Void, InvoiceStatusEnum::Finalized]),
            )
            .filter(diesel::dsl::now.gt(i_dsl::invoice_date
                + diesel::dsl::sql::<diesel::sql_types::Interval>(
                    "\"invoicing_entity\".\"grace_period_hours\" * INTERVAL '1 hour'",
                )))
            .select(InvoiceRow::as_select())
            .cursor_paginate(pagination, "id");

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .load_and_get_next_cursor(conn, |a| a.id.as_uuid())
            .await
            .attach_printable("Error while paginating invoices to finalize")
            .into_db_result()
    }

    pub async fn finalize(
        conn: &mut PgConn,
        id: InvoiceId,
        tenant_id: TenantId,
        new_invoice_number: String,
        coupons: serde_json::Value,
    ) -> DbResult<usize> {
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        let now = chrono::Utc::now().naive_utc();

        let query = diesel::update(i_dsl::invoice)
            .filter(i_dsl::id.eq(id))
            .filter(i_dsl::tenant_id.eq(tenant_id))
            .filter(
                i_dsl::status.ne_all(vec![InvoiceStatusEnum::Finalized, InvoiceStatusEnum::Void]),
            )
            .set((
                i_dsl::status.eq(InvoiceStatusEnum::Finalized),
                i_dsl::updated_at.eq(now),
                i_dsl::data_updated_at.eq(now),
                i_dsl::finalized_at.eq(now),
                i_dsl::invoice_number.eq(new_invoice_number),
                i_dsl::coupons.eq(coupons),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach_printable("Error while finalizing invoice")
            .into_db_result()
    }

    pub async fn apply_payment_status(
        conn: &mut PgConn,
        id: InvoiceId,
        tenant_id: TenantId,
        payment_status: InvoicePaymentStatus,
    ) -> DbResult<InvoiceRow> {
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        let now = chrono::Utc::now().naive_utc();

        let paid_at = if payment_status == InvoicePaymentStatus::Paid {
            Some(now)
        } else {
            None
        };

        let query = diesel::update(i_dsl::invoice)
            .filter(i_dsl::id.eq(id))
            .filter(i_dsl::tenant_id.eq(tenant_id))
            .set((
                i_dsl::payment_status.eq(payment_status),
                i_dsl::paid_at.eq(paid_at),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach_printable("Error while applying payment status to invoice")
            .into_db_result()
    }

    pub async fn apply_transaction(
        conn: &mut PgConn,
        id: InvoiceId,
        tenant_id: TenantId,
        transaction_amount: i64,
    ) -> DbResult<InvoiceRow> {
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        let now = chrono::Utc::now().naive_utc();

        let query = diesel::update(i_dsl::invoice)
            .filter(i_dsl::id.eq(id))
            .filter(i_dsl::tenant_id.eq(tenant_id))
            .set((
                i_dsl::updated_at.eq(now),
                i_dsl::amount_due.eq(i_dsl::amount_due - transaction_amount),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach_printable("Error while applying transaction to invoice")
            .into_db_result()
    }

    pub async fn save_invoice_documents(
        conn: &mut PgConn,
        id: InvoiceId,
        tenant_id: TenantId,
        pdf_document_id: StoredDocumentId,
        xml_document_id: Option<StoredDocumentId>,
    ) -> DbResult<usize> {
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(i_dsl::invoice)
            .filter(i_dsl::id.eq(id))
            .filter(i_dsl::tenant_id.eq(tenant_id))
            .set((
                i_dsl::pdf_document_id.eq(pdf_document_id),
                i_dsl::xml_document_id.eq(xml_document_id),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach_printable("Error while saving invoice documents")
            .into_db_result()
    }

    pub async fn list_outdated(
        conn: &mut PgConn,
        pagination: CursorPaginationRequest,
    ) -> DbResult<CursorPaginatedVec<InvoiceRow>> {
        use crate::schema::invoice::dsl as i_dsl;

        let query = i_dsl::invoice
            .filter(
                i_dsl::status.ne_all(vec![InvoiceStatusEnum::Void, InvoiceStatusEnum::Finalized]),
            )
            .filter(
                i_dsl::data_updated_at
                    .is_null()
                    .or(diesel::dsl::now.gt(i_dsl::invoice_date + 1.hour())),
            )
            .select(InvoiceRow::as_select())
            .cursor_paginate(pagination, "id");

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .load_and_get_next_cursor(conn, |a| a.id.as_uuid())
            .await
            .attach_printable("Error while paginating outdated invoices")
            .into_db_result()
    }

    pub async fn upsert_conn_meta(
        conn: &mut PgConn,
        provider: ConnectorProviderEnum,
        invoice_id: InvoiceId,
        connector_id: ConnectorId,
        external_id: &str,
        external_company_id: &str,
    ) -> DbResult<()> {
        connection_metadata::upsert(
            conn,
            "invoice",
            provider.as_meta_key(),
            invoice_id.as_uuid(),
            connector_id,
            external_id,
            external_company_id,
        )
        .await
        .map(|_| ())
    }
}

impl InvoiceRowLinesPatch {
    pub async fn update_lines(
        &self,
        id: InvoiceId,
        tenant_id: TenantId,
        conn: &mut PgConn,
    ) -> DbResult<usize> {
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(i_dsl::invoice)
            .filter(i_dsl::id.eq(id).and(i_dsl::tenant_id.eq(tenant_id)))
            .set(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach_printable("Error while updating invoice lines")
            .into_db_result()
    }
}
