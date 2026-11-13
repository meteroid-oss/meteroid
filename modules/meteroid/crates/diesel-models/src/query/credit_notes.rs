use crate::credit_notes::{CreditNoteRow, CreditNoteRowNew, CreditNoteRowPatch};
use crate::enums::{CreditNoteStatus, EInvoicingStatusEnum};
use crate::errors::IntoDbResult;
use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};
use crate::lock::CustomerLock;
use crate::{DbResult, PgConn};

use crate::extend::filtering::{ListFilter, render_list_filter};
use crate::extend::order::{OrderByParam, OrderDirection};
use common_domain::ids::{CreditNoteId, CustomerId, InvoiceId, StoredDocumentId, TenantId};
use diesel::{
    BoolExpressionMethods, ExpressionMethods, JoinOnDsl, PgTextExpressionMethods, QueryDsl,
    SelectableHelper,
};
use error_stack::ResultExt;

impl CreditNoteRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<CreditNoteRow> {
        use crate::schema::credit_note::dsl::credit_note;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(credit_note).values((
            self,
            crate::schema::credit_note::created_at.eq(common_domain::clock::now()),
        ));

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while inserting credit note")
            .into_db_result()
    }
}

impl CreditNoteRow {
    pub async fn set_balance_effects(
        conn: &mut PgConn,
        tenant: TenantId,
        id: CreditNoteId,
        credited: i64,
        debt: i64,
    ) -> DbResult<()> {
        use crate::schema::credit_note::dsl as note;
        use diesel_async::RunQueryDsl;
        diesel::update(
            note::credit_note
                .filter(note::tenant_id.eq(tenant))
                .filter(note::id.eq(id)),
        )
        .set((
            note::credited_amount_cents.eq(credited),
            note::debt_cancelled_amount_cents.eq(debt),
        ))
        .execute(conn)
        .await
        .attach("Error while recording credit note effects")
        .into_db_result()?;
        Ok(())
    }

    pub async fn find_by_id_for_update(
        conn: &mut PgConn,
        lock: &CustomerLock,
        param_credit_note_id: CreditNoteId,
    ) -> DbResult<CreditNoteRow> {
        use crate::schema::credit_note::dsl as cn_dsl;
        use diesel_async::RunQueryDsl;

        let row: CreditNoteRow = cn_dsl::credit_note
            .filter(cn_dsl::tenant_id.eq(lock.tenant_id()))
            .filter(cn_dsl::id.eq(param_credit_note_id))
            .for_no_key_update()
            .select(CreditNoteRow::as_select())
            .first(conn)
            .await
            .attach("Error while locking credit note by id")
            .into_db_result()?;
        lock.covers(conn, row.tenant_id, row.customer_id)?;
        Ok(row)
    }

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

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while finding credit note by id")
            .into_db_result()
    }

    /// Locks the credit note so concurrent refunds cannot each draw against the same remainder.
    pub async fn select_for_update_by_id(
        conn: &mut PgConn,
        lock: &CustomerLock,
        param_credit_note_id: CreditNoteId,
    ) -> DbResult<CreditNoteRow> {
        use crate::schema::credit_note::dsl as cn_dsl;
        use diesel_async::RunQueryDsl;

        let query = cn_dsl::credit_note
            .filter(cn_dsl::tenant_id.eq(lock.tenant_id()))
            .filter(cn_dsl::id.eq(param_credit_note_id))
            .select(CreditNoteRow::as_select())
            .for_update();

        crate::debug_sql(&query);

        let row: CreditNoteRow = query
            .first(conn)
            .await
            .attach("Error while locking credit note by id")
            .into_db_result()?;
        lock.covers(conn, row.tenant_id, row.customer_id)?;
        Ok(row)
    }

    /// The invoice's finalized Refund credit notes, oldest first: a refund made at the provider
    /// draws on them in this order.
    pub async fn list_finalized_refund_ids(
        conn: &mut PgConn,
        param_invoice_id: InvoiceId,
        param_tenant_id: TenantId,
    ) -> DbResult<Vec<CreditNoteId>> {
        use crate::schema::credit_note::dsl as cn_dsl;
        use diesel_async::RunQueryDsl;

        cn_dsl::credit_note
            .filter(cn_dsl::invoice_id.eq(param_invoice_id))
            .filter(cn_dsl::tenant_id.eq(param_tenant_id))
            .filter(cn_dsl::credit_type.eq(crate::enums::CreditTypeEnum::Refund))
            .filter(cn_dsl::status.eq(CreditNoteStatus::Finalized))
            .order((cn_dsl::finalized_at.asc(), cn_dsl::id.asc()))
            .select(cn_dsl::id)
            .load(conn)
            .await
            .attach("Error while listing refund credit notes")
            .into_db_result()
    }

    /// Σ refundable amount of the invoice's finalized Refund credit notes — money already
    /// promised back through an avoir, drawn or not.
    pub async fn sum_finalized_refundable(
        conn: &mut PgConn,
        param_invoice_id: InvoiceId,
        param_tenant_id: TenantId,
    ) -> DbResult<i64> {
        use crate::schema::credit_note::dsl as cn_dsl;
        use diesel_async::RunQueryDsl;

        let query = cn_dsl::credit_note
            .filter(cn_dsl::invoice_id.eq(param_invoice_id))
            .filter(cn_dsl::tenant_id.eq(param_tenant_id))
            .filter(cn_dsl::credit_type.eq(crate::enums::CreditTypeEnum::Refund))
            .filter(cn_dsl::status.eq(CreditNoteStatus::Finalized))
            .select(cn_dsl::refunded_amount_cents);

        crate::debug_sql(&query);

        let amounts: Vec<i64> = query
            .load(conn)
            .await
            .attach("Error while summing refundable credit notes")
            .into_db_result()?;
        Ok(amounts.iter().sum())
    }

    /// Σ |total| of the Finalized credit notes on an invoice that reduce what it collects:
    /// DebtCancellation (debt written off) and Refund (the avoir behind a refund — the money is
    /// given back by a separate REFUND transaction, so only the credit note may shrink the
    /// invoice). Totals are stored negative, hence abs. Shared by `exists_live_for_invoice` and
    /// `recompute_amount_due_from_settled_payments` so their netting can't drift apart.
    pub async fn sum_finalized_invoice_reducing(
        conn: &mut PgConn,
        param_invoice_id: InvoiceId,
        param_tenant_id: TenantId,
    ) -> DbResult<i64> {
        use crate::schema::credit_note::dsl as cn_dsl;
        use diesel_async::RunQueryDsl;

        let query = cn_dsl::credit_note
            .filter(cn_dsl::invoice_id.eq(param_invoice_id))
            .filter(cn_dsl::tenant_id.eq(param_tenant_id))
            .filter(cn_dsl::status.eq(CreditNoteStatus::Finalized))
            .select((
                cn_dsl::credit_type,
                cn_dsl::total,
                cn_dsl::debt_cancelled_amount_cents,
            ));

        crate::debug_sql(&query);

        let notes: Vec<(crate::enums::CreditTypeEnum, i64, i64)> = query
            .load(conn)
            .await
            .attach("Error while summing invoice-reducing credit notes")
            .into_db_result()?;
        // A credit-to-balance note reduces the invoice only by the debt it cancelled.
        Ok(notes
            .iter()
            .map(|(credit_type, total, debt)| match credit_type {
                crate::enums::CreditTypeEnum::CreditToBalance => *debt,
                crate::enums::CreditTypeEnum::DebtCancellation
                | crate::enums::CreditTypeEnum::Refund => total.abs(),
            })
            .sum())
    }

    /// Σ debt the Finalized credit notes of an invoice cancelled: DebtCancellation totals and
    /// the debt part of credit-to-balance notes. Refund notes are left out.
    pub async fn sum_finalized_debt_cancelled(
        conn: &mut PgConn,
        param_invoice_id: InvoiceId,
        param_tenant_id: TenantId,
    ) -> DbResult<i64> {
        use crate::schema::credit_note::dsl as cn_dsl;
        use diesel_async::RunQueryDsl;

        let query = cn_dsl::credit_note
            .filter(cn_dsl::invoice_id.eq(param_invoice_id))
            .filter(cn_dsl::tenant_id.eq(param_tenant_id))
            .filter(cn_dsl::status.eq(CreditNoteStatus::Finalized))
            .filter(cn_dsl::credit_type.ne(crate::enums::CreditTypeEnum::Refund))
            .select((
                cn_dsl::credit_type,
                cn_dsl::total,
                cn_dsl::debt_cancelled_amount_cents,
            ));

        crate::debug_sql(&query);

        let notes: Vec<(crate::enums::CreditTypeEnum, i64, i64)> = query
            .load(conn)
            .await
            .attach("Error while summing debt cancelled by credit notes")
            .into_db_result()?;
        Ok(notes
            .iter()
            .map(|(credit_type, total, debt)| match credit_type {
                crate::enums::CreditTypeEnum::DebtCancellation => total.abs(),
                crate::enums::CreditTypeEnum::CreditToBalance
                | crate::enums::CreditTypeEnum::Refund => *debt,
            })
            .sum())
    }

    /// [`Self::sum_finalized_debt_cancelled`] for each of `invoice_ids` that has any.
    pub async fn sum_finalized_debt_cancelled_by_invoices(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        invoice_ids: &[InvoiceId],
    ) -> DbResult<std::collections::HashMap<InvoiceId, i64>> {
        use crate::schema::credit_note::dsl as cn_dsl;
        use diesel_async::RunQueryDsl;

        if invoice_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let query = cn_dsl::credit_note
            .filter(cn_dsl::invoice_id.eq_any(invoice_ids))
            .filter(cn_dsl::tenant_id.eq(param_tenant_id))
            .filter(cn_dsl::status.eq(CreditNoteStatus::Finalized))
            .filter(cn_dsl::credit_type.ne(crate::enums::CreditTypeEnum::Refund))
            .select((
                cn_dsl::invoice_id,
                cn_dsl::credit_type,
                cn_dsl::total,
                cn_dsl::debt_cancelled_amount_cents,
            ));

        crate::debug_sql(&query);

        let notes: Vec<(InvoiceId, crate::enums::CreditTypeEnum, i64, i64)> = query
            .load(conn)
            .await
            .attach("Error while summing debt cancelled by credit notes")
            .into_db_result()?;
        let mut sums = std::collections::HashMap::new();
        for (invoice_id, credit_type, total, debt) in notes {
            let cancelled = match credit_type {
                crate::enums::CreditTypeEnum::DebtCancellation => total.abs(),
                crate::enums::CreditTypeEnum::CreditToBalance
                | crate::enums::CreditTypeEnum::Refund => debt,
            };
            *sums.entry(invoice_id).or_insert(0) += cancelled;
        }
        Ok(sums)
    }

    /// Sets only the `custom_properties` bag on a credit note. Allowed at any status: custom
    /// properties are external workflow metadata (e.g. tax-filing / accounting-export status) that
    /// legitimately changes after finalization, unlike the credit note's financial content.
    /// Returns `None` only when the credit note doesn't exist.
    pub async fn update_custom_properties(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_credit_note_id: CreditNoteId,
        custom_properties: serde_json::Value,
    ) -> DbResult<Option<CreditNoteRow>> {
        use crate::schema::credit_note::dsl as cn_dsl;
        use diesel::OptionalExtension;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(cn_dsl::credit_note)
            .filter(cn_dsl::tenant_id.eq(param_tenant_id))
            .filter(cn_dsl::id.eq(param_credit_note_id))
            .set(cn_dsl::custom_properties.eq(custom_properties));

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .optional()
            .attach("Error while updating credit note custom_properties")
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

        crate::debug_sql(&query);

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

        crate::debug_sql(&query);

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

        let now = common_domain::clock::now().naive_utc();

        let query = diesel::update(cn_dsl::credit_note)
            .filter(cn_dsl::id.eq(id))
            .filter(cn_dsl::tenant_id.eq(tenant_id))
            .filter(cn_dsl::status.eq(CreditNoteStatus::Draft))
            .set((
                cn_dsl::status.eq(CreditNoteStatus::Finalized),
                cn_dsl::updated_at.eq(now),
                cn_dsl::finalized_at.eq(now),
            ));

        crate::debug_sql(&query);

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

        let now = common_domain::clock::now().naive_utc();

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

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while finalizing credit note with number")
            .into_db_result()
    }

    pub async fn void(conn: &mut PgConn, id: CreditNoteId, tenant_id: TenantId) -> DbResult<usize> {
        use crate::schema::credit_note::dsl as cn_dsl;
        use diesel_async::RunQueryDsl;

        let now = common_domain::clock::now().naive_utc();

        let query = diesel::update(cn_dsl::credit_note)
            .filter(cn_dsl::id.eq(id))
            .filter(cn_dsl::tenant_id.eq(tenant_id))
            .filter(cn_dsl::status.eq(CreditNoteStatus::Finalized))
            .set((
                cn_dsl::status.eq(CreditNoteStatus::Voided),
                cn_dsl::updated_at.eq(now),
                cn_dsl::voided_at.eq(now),
            ));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while voiding credit note")
            .into_db_result()
    }

    pub async fn list(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_customer_id: Option<CustomerId>,
        param_invoice_id: Option<InvoiceId>,
        param_status: Option<CreditNoteStatus>,
        param_search: Option<String>,
        filters: Vec<ListFilter>,
        order_by: Option<&str>,
        pagination: PaginationRequest,
    ) -> DbResult<PaginatedVec<CreditNoteRow>> {
        use crate::schema::credit_note::dsl as cn_dsl;
        use crate::schema::customer::dsl as c_dsl;

        let mut query = cn_dsl::credit_note
            .inner_join(c_dsl::customer.on(cn_dsl::customer_id.eq(c_dsl::id)))
            .filter(cn_dsl::tenant_id.eq(param_tenant_id))
            .select(CreditNoteRow::as_select())
            .into_boxed();

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
                cn_dsl::credit_note_number
                    .ilike(search_pattern.clone())
                    .or(c_dsl::name.ilike(search_pattern)),
            );
        }

        for f in filters {
            query = query.filter(render_list_filter(f, "credit_note.custom_properties"));
        }

        let order = OrderByParam::parse(order_by, "created_at.desc");

        match (order.column.as_str(), order.direction) {
            ("credit_note_number", OrderDirection::Asc) => {
                query = query.order((cn_dsl::credit_note_number.asc(), cn_dsl::id.asc()))
            }
            ("credit_note_number", OrderDirection::Desc) => {
                query = query.order((cn_dsl::credit_note_number.desc(), cn_dsl::id.desc()))
            }
            ("customer_name", OrderDirection::Asc) => {
                query = query.order((c_dsl::name.asc(), cn_dsl::id.asc()))
            }
            ("customer_name", OrderDirection::Desc) => {
                query = query.order((c_dsl::name.desc(), cn_dsl::id.desc()))
            }
            ("amount", OrderDirection::Asc) => {
                query = query.order((cn_dsl::total.asc(), cn_dsl::id.asc()))
            }
            ("amount", OrderDirection::Desc) => {
                query = query.order((cn_dsl::total.desc(), cn_dsl::id.desc()))
            }
            ("status", OrderDirection::Asc) => {
                query = query.order((cn_dsl::status.asc(), cn_dsl::id.asc()))
            }
            ("status", OrderDirection::Desc) => {
                query = query.order((cn_dsl::status.desc(), cn_dsl::id.desc()))
            }
            ("created_at", OrderDirection::Asc) => {
                query = query.order((cn_dsl::created_at.asc(), cn_dsl::id.asc()))
            }
            ("created_at", OrderDirection::Desc) => {
                query = query.order((cn_dsl::created_at.desc(), cn_dsl::id.desc()))
            }
            _ => query = query.order((cn_dsl::created_at.desc(), cn_dsl::id.desc())),
        }

        let paginated_query = query.paginate(pagination);

        crate::debug_sql(&paginated_query);

        paginated_query
            .load_and_count_pages(conn)
            .await
            .attach("Error while listing credit notes")
            .into_db_result()
    }

    pub async fn list_by_ids(
        conn: &mut PgConn,
        credit_note_ids: Vec<CreditNoteId>,
    ) -> DbResult<Vec<CreditNoteRow>> {
        use crate::schema::credit_note::dsl as cn_dsl;
        use diesel_async::RunQueryDsl;

        let query = cn_dsl::credit_note
            .filter(cn_dsl::id.eq_any(credit_note_ids))
            .select(CreditNoteRow::as_select());

        crate::debug_sql(&query);

        query
            .load(conn)
            .await
            .attach("Error while listing credit notes by ids")
            .into_db_result()
    }

    pub async fn save_credit_note_documents(
        conn: &mut PgConn,
        id: CreditNoteId,
        tenant_id: TenantId,
        pdf_document_id: StoredDocumentId,
        xml_document_id: Option<StoredDocumentId>,
        einvoicing_status: Option<EInvoicingStatusEnum>,
        einvoicing_report: Option<serde_json::Value>,
    ) -> DbResult<usize> {
        use crate::schema::credit_note::dsl as cn_dsl;
        use diesel_async::RunQueryDsl;

        let now = common_domain::clock::now().naive_utc();

        let query = diesel::update(cn_dsl::credit_note)
            .filter(cn_dsl::id.eq(id))
            .filter(cn_dsl::tenant_id.eq(tenant_id))
            .set((
                cn_dsl::pdf_document_id.eq(pdf_document_id),
                cn_dsl::xml_document_id.eq(xml_document_id),
                cn_dsl::einvoicing_status.eq(einvoicing_status),
                cn_dsl::einvoicing_report.eq(einvoicing_report),
                cn_dsl::updated_at.eq(now),
            ));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while saving credit note documents")
            .into_db_result()
    }

    pub async fn delete_draft(
        conn: &mut PgConn,
        id: CreditNoteId,
        tenant_id: TenantId,
    ) -> DbResult<usize> {
        use crate::schema::credit_note::dsl as cn_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::delete(cn_dsl::credit_note)
            .filter(cn_dsl::id.eq(id))
            .filter(cn_dsl::tenant_id.eq(tenant_id))
            .filter(cn_dsl::status.eq(CreditNoteStatus::Draft));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while deleting draft credit note")
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

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while updating credit note")
            .into_db_result()
    }
}

impl CreditNoteRow {
    /// The lines of the finalized credit notes of each of `invoice_ids`.
    pub async fn list_finalized_lines_by_invoices(
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_ids: &[InvoiceId],
    ) -> DbResult<Vec<(InvoiceId, serde_json::Value)>> {
        use crate::schema::credit_note::dsl as cn;
        use diesel_async::RunQueryDsl;

        if invoice_ids.is_empty() {
            return Ok(Vec::new());
        }
        let query = cn::credit_note
            .filter(cn::tenant_id.eq(tenant_id))
            .filter(cn::invoice_id.eq_any(invoice_ids))
            .filter(cn::status.eq(CreditNoteStatus::Finalized))
            .select((cn::invoice_id, cn::line_items));

        crate::debug_sql(&query);

        query
            .load(conn)
            .await
            .attach("Error while listing the credit note lines of invoices")
            .into_db_result()
    }
}
