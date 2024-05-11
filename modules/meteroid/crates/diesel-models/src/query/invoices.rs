use crate::errors::IntoDbResult;
use crate::invoices::{Invoice, InvoiceNew, InvoiceWithPlanDetails};

use crate::{DbResult, PgConn};

use crate::enums::InvoiceStatusEnum;
use crate::extend::order::OrderByRequest;
use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};
use diesel::{debug_query, JoinOnDsl, PgTextExpressionMethods, SelectableHelper};
use diesel::{ExpressionMethods, QueryDsl};
use error_stack::{Report, ResultExt};

impl InvoiceNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<Invoice> {
        use crate::schema::invoice::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(invoice).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting invoice")
            .into_db_result()
    }
}
impl Invoice {
    pub async fn find_by_id(
        conn: &mut PgConn,
        param_tenant_id: uuid::Uuid,
        param_invoice_id: uuid::Uuid,
    ) -> DbResult<InvoiceWithPlanDetails> {
        use crate::schema::customer::dsl as c_dsl;
        use crate::schema::invoice::dsl as i_dsl;
        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::plan_version::dsl as pv_dsl;
        use crate::schema::subscription::dsl as s_dsl;
        use diesel::prelude::*;
        use diesel_async::RunQueryDsl;

        let query = i_dsl::invoice
            .inner_join(c_dsl::customer.on(i_dsl::customer_id.eq(c_dsl::id)))
            .inner_join(s_dsl::subscription.on(i_dsl::subscription_id.eq(s_dsl::id)))
            .inner_join(pv_dsl::plan_version.on(s_dsl::plan_version_id.eq(pv_dsl::id)))
            .inner_join(p_dsl::plan.on(pv_dsl::plan_id.eq(p_dsl::id)))
            .filter(i_dsl::tenant_id.eq(param_tenant_id))
            .filter(i_dsl::id.eq(param_invoice_id))
            .select((
                i_dsl::id,
                i_dsl::status,
                i_dsl::external_status,
                i_dsl::created_at,
                i_dsl::updated_at,
                i_dsl::tenant_id,
                i_dsl::customer_id,
                i_dsl::subscription_id,
                i_dsl::currency,
                i_dsl::days_until_due,
                i_dsl::external_invoice_id,
                i_dsl::invoice_id,
                i_dsl::invoicing_provider,
                i_dsl::line_items,
                i_dsl::issued,
                i_dsl::issue_attempts,
                i_dsl::last_issue_attempt_at,
                i_dsl::last_issue_error,
                i_dsl::data_updated_at,
                i_dsl::invoice_date,
                i_dsl::amount_cents,
                c_dsl::name,
                p_dsl::name,
                p_dsl::external_id,
                pv_dsl::version,
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while finding invoice by id")
            .into_db_result()
    }

    pub async fn list(
        conn: &mut PgConn,
        param_tenant_id: uuid::Uuid,
        param_customer_id: Option<uuid::Uuid>,
        param_status: Option<InvoiceStatusEnum>,
        param_query: Option<String>,
        order_by: OrderByRequest,
        pagination: PaginationRequest,
    ) -> DbResult<PaginatedVec<Invoice>> {
        use crate::schema::customer::dsl as c_dsl;
        use crate::schema::invoice::dsl as i_dsl;

        let mut query = i_dsl::invoice
            .inner_join(c_dsl::customer.on(i_dsl::customer_id.eq(c_dsl::id)))
            .filter(i_dsl::tenant_id.eq(param_tenant_id))
            .select(Invoice::as_select())
            .into_boxed();

        if let Some(param_customer_id) = param_customer_id {
            query = query.filter(i_dsl::customer_id.eq(param_customer_id))
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

        log::debug!(
            "{}",
            debug_query::<diesel::pg::Pg, _>(&paginated_query).to_string()
        );

        paginated_query
            .load_and_count_pages(conn)
            .await
            .attach_printable("Error while fetching invoices")
            .into_db_result()
    }

    pub async fn insert_invoice_batch(
        conn: &mut PgConn,
        invoices: Vec<InvoiceNew>,
    ) -> DbResult<Vec<Invoice>> {
        use crate::schema::invoice::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(invoice).values(&invoices);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while inserting invoice")
            .into_db_result()
    }
}
