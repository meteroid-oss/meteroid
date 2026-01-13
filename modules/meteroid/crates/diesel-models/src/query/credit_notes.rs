use crate::credit_notes::{CreditNoteRow, CreditNoteRowNew, CreditNoteRowPatch};
use crate::enums::CreditNoteStatus;
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};
use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};

use common_domain::ids::{CreditNoteId, CustomerId, InvoiceId, TenantId};
use diesel::{
    BoolExpressionMethods, ExpressionMethods, NullableExpressionMethods, PgTextExpressionMethods,
    QueryDsl, debug_query,SelectableHelper, JoinOnDsl
};
use error_stack::ResultExt;
use crate::extend::order::OrderByRequest;
use crate::invoices::InvoiceWithCustomerRow;

impl CreditNoteRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<CreditNoteRow> {
        use crate::schema::credit_note::dsl::credit_note;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(credit_note).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while inserting credit note")
            .into_db_result()
    }
}

impl CreditNoteRow {
    pub async fn find_by_id(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_credit_note_id: CreditNoteId,
    ) -> DbResult<CreditNoteRow> {
        use crate::schema::credit_note::dsl as cn_dsl;
        use diesel_async::RunQueryDsl;

        let query = cn_dsl::credit_note
            .filter(cn_dsl::tenant_id.eq(param_tenant_id))
            .filter(cn_dsl::id.eq(param_credit_note_id))
            .select(CreditNoteRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach("Error while finding credit note by id")
            .into_db_result()
    }

    pub async fn list_by_invoice_id(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_invoice_id: InvoiceId,
    ) -> DbResult<Vec<CreditNoteRow>> {
        use crate::schema::credit_note::dsl as cn_dsl;
        use diesel_async::RunQueryDsl;

        let query = cn_dsl::credit_note
            .filter(cn_dsl::tenant_id.eq(param_tenant_id))
            .filter(cn_dsl::invoice_id.eq(param_invoice_id))
            .order(cn_dsl::created_at.desc())
            .select(CreditNoteRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .load(conn)
            .await
            .attach("Error while listing credit notes by invoice id")
            .into_db_result()
    }

    pub async fn list_by_customer_id(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_customer_id: CustomerId,
    ) -> DbResult<Vec<CreditNoteRow>> {
        use crate::schema::credit_note::dsl as cn_dsl;
        use diesel_async::RunQueryDsl;

        let query = cn_dsl::credit_note
            .filter(cn_dsl::tenant_id.eq(param_tenant_id))
            .filter(cn_dsl::customer_id.eq(param_customer_id))
            .order(cn_dsl::created_at.desc())
            .select(CreditNoteRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .load(conn)
            .await
            .attach("Error while listing credit notes by customer id")
            .into_db_result()
    }

    pub async fn finalize(
        conn: &mut PgConn,
        id: CreditNoteId,
        tenant_id: TenantId,
    ) -> DbResult<usize> {
        use crate::schema::credit_note::dsl as cn_dsl;
        use diesel_async::RunQueryDsl;

        let now = chrono::Utc::now().naive_utc();

        let query = diesel::update(cn_dsl::credit_note)
            .filter(cn_dsl::id.eq(id))
            .filter(cn_dsl::tenant_id.eq(tenant_id))
            .filter(cn_dsl::status.eq(CreditNoteStatus::Draft))
            .set((
                cn_dsl::status.eq(CreditNoteStatus::Finalized),
                cn_dsl::updated_at.eq(now),
                cn_dsl::finalized_at.eq(now),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while finalizing credit note")
            .into_db_result()
    }

    pub async fn finalize_with_number(
        conn: &mut PgConn,
        id: CreditNoteId,
        tenant_id: TenantId,
        credit_note_number: &str,
    ) -> DbResult<usize> {
        use crate::schema::credit_note::dsl as cn_dsl;
        use diesel_async::RunQueryDsl;

        let now = chrono::Utc::now().naive_utc();

        let query = diesel::update(cn_dsl::credit_note)
            .filter(cn_dsl::id.eq(id))
            .filter(cn_dsl::tenant_id.eq(tenant_id))
            .filter(cn_dsl::status.eq(CreditNoteStatus::Draft))
            .set((
                cn_dsl::status.eq(CreditNoteStatus::Finalized),
                cn_dsl::credit_note_number.eq(credit_note_number),
                cn_dsl::updated_at.eq(now),
                cn_dsl::finalized_at.eq(now),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while finalizing credit note with number")
            .into_db_result()
    }

    pub async fn void(
        conn: &mut PgConn,
        id: CreditNoteId,
        tenant_id: TenantId,
    ) -> DbResult<usize> {
        use crate::schema::credit_note::dsl as cn_dsl;
        use diesel_async::RunQueryDsl;

        let now = chrono::Utc::now().naive_utc();

        let query = diesel::update(cn_dsl::credit_note)
            .filter(cn_dsl::id.eq(id))
            .filter(cn_dsl::tenant_id.eq(tenant_id))
            .filter(cn_dsl::status.eq(CreditNoteStatus::Finalized))
            .set((
                cn_dsl::status.eq(CreditNoteStatus::Voided),
                cn_dsl::updated_at.eq(now),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while voiding credit note")
            .into_db_result()
    }


    #[allow(clippy::too_many_arguments)]
    pub async fn list(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_customer_id: Option<CustomerId>,
        param_invoice_id: Option<InvoiceId>,
        param_status: Option<CreditNoteStatus>,
        param_search: Option<String>,
        order_by: OrderByRequest,
        pagination: PaginationRequest,
    ) -> DbResult<PaginatedVec<CreditNoteRow>> {
        use crate::schema::credit_note::dsl as cn_dsl;
        use crate::schema::customer::dsl as c_dsl;

        let mut query = cn_dsl::credit_note
            .inner_join(c_dsl::customer.on(cn_dsl::customer_id.eq(c_dsl::id)))
            .filter(cn_dsl::tenant_id.eq(param_tenant_id))
            .select(CreditNoteRow::as_select())
            .into_boxed();

        // Apply filters
        if let Some(customer_id) = param_customer_id {
            query = query.filter(cn_dsl::customer_id.eq(customer_id));
        }

        if let Some(invoice_id) = param_invoice_id {
            query = query.filter(cn_dsl::invoice_id.eq(invoice_id));
        }

        if let Some(status) = param_status {
            query = query.filter(cn_dsl::status.eq(status));
        }

        if let Some(search) = param_search {
            let search_pattern = format!("%{}%", search);
            query = query.filter(
                cn_dsl::credit_note_number.ilike(search_pattern.clone())
                    .or(c_dsl::name.ilike(search_pattern))
            );
        }

        query = match order_by {
            OrderByRequest::DateAsc => query.order(cn_dsl::created_at.asc()),
            OrderByRequest::DateDesc => query.order(cn_dsl::created_at.desc()),
            OrderByRequest::IdAsc => query.order(cn_dsl::id.asc()),
            OrderByRequest::IdDesc => query.order(cn_dsl::id.desc()),
            OrderByRequest::NameAsc => query.order(cn_dsl::credit_note_number.asc()),
            OrderByRequest::NameDesc => query.order(cn_dsl::credit_note_number.desc()),
        };


        let paginated_query = query.paginate(pagination);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&paginated_query));

        paginated_query
            .load_and_count_pages(conn)
            .await
            .attach("Error while listing credit notes")
            .into_db_result()
    }
}

impl CreditNoteRowPatch {
    pub async fn update(
        &self,
        id: CreditNoteId,
        tenant_id: TenantId,
        conn: &mut PgConn,
    ) -> DbResult<usize> {
        use crate::schema::credit_note::dsl as cn_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(cn_dsl::credit_note)
            .filter(cn_dsl::id.eq(id).and(cn_dsl::tenant_id.eq(tenant_id)))
            .set(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while updating credit note")
            .into_db_result()
    }
}
