use crate::errors::IntoDbResult;
use crate::invoices::{
    ConsolidatedMemberLinesRow, DetailedInvoiceRow, InvoiceCollections, InvoiceLockRow, InvoiceRow,
    InvoiceRowLinesPatch, InvoiceRowNew, InvoiceWithCustomerRow, UnsettledInvoiceRow,
};

use crate::lock::CustomerLock;
use crate::{DbResult, PgConn};

use crate::enums::{
    ConnectorProviderEnum, EInvoicingStatusEnum, InvoicePaymentStatus, InvoiceStatusEnum,
};
use crate::extend::connection_metadata;
use crate::extend::cursor_pagination::{
    CursorPaginate, CursorPaginatedVec, CursorPaginationRequest,
};
use crate::extend::filtering::{ListFilter, render_list_filter};
use crate::extend::order::{OrderByParam, OrderDirection};
use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};
use crate::payments::PaymentTransactionRow;
use chrono::NaiveDateTime;
use common_domain::ids::{
    AliasOr, BaseId, ConnectorId, CustomerId, InvoiceId, InvoicingEntityId, ProductId,
    StoredDocumentId, SubscriptionId, TenantId,
};
use diesel::{
    BelongingToDsl, BoolExpressionMethods, JoinOnDsl, NullableExpressionMethods,
    PgTextExpressionMethods, SelectableHelper,
};
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl};
use error_stack::ResultExt;

impl InvoiceRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<InvoiceRow> {
        use crate::schema::invoice::dsl::invoice;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(invoice).values((
            self,
            crate::schema::invoice::created_at.eq(common_domain::clock::now()),
        ));

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while inserting invoice")
            .into_db_result()
    }
}

impl InvoiceRow {
    /// Locks the invoice's customer, then the invoice (`crate::lock`).
    pub async fn select_for_update_by_id(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_invoice_id: InvoiceId,
    ) -> DbResult<InvoiceLockRow> {
        use crate::customers::CustomerRow;
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        let invoice_customer_id: common_domain::ids::CustomerId = i_dsl::invoice
            .select(i_dsl::customer_id)
            .filter(i_dsl::tenant_id.eq(param_tenant_id))
            .filter(i_dsl::id.eq(param_invoice_id))
            .first(conn)
            .await
            .attach("Error while fetching invoice customer_id")
            .into_db_result()?;
        let lock = CustomerRow::lock(conn, param_tenant_id, invoice_customer_id).await?;
        let invoice = Self::lock_by_id(conn, &lock, param_invoice_id).await?;
        Ok(InvoiceLockRow { lock, invoice })
    }

    /// Locks the invoice's customer only (`crate::lock`).
    pub async fn lock_customer_of(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_invoice_id: InvoiceId,
    ) -> DbResult<CustomerLock> {
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        let customer: common_domain::ids::CustomerId = i_dsl::invoice
            .select(i_dsl::customer_id)
            .filter(i_dsl::tenant_id.eq(param_tenant_id))
            .filter(i_dsl::id.eq(param_invoice_id))
            .first(conn)
            .await
            .attach("Error while fetching invoice customer_id")
            .into_db_result()?;
        crate::customers::CustomerRow::lock(conn, param_tenant_id, customer).await
    }

    /// FOR UPDATE, not NO KEY: drafts are deleted.
    pub async fn lock_by_id(
        conn: &mut PgConn,
        lock: &CustomerLock,
        param_invoice_id: InvoiceId,
    ) -> DbResult<InvoiceRow> {
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        let invoice_query = i_dsl::invoice
            .filter(i_dsl::tenant_id.eq(lock.tenant_id()))
            .filter(i_dsl::id.eq(param_invoice_id))
            .for_update();

        crate::debug_sql(&invoice_query);

        let invoice: InvoiceRow = invoice_query
            .first(conn)
            .await
            .attach("Error while locking invoice by id")
            .into_db_result()?;
        lock.covers(conn, invoice.tenant_id, invoice.customer_id)?;
        Ok(invoice)
    }

    /// Get the backdate_invoices flag from the subscription (if any)
    pub async fn get_subscription_backdate_flag(
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
    ) -> DbResult<bool> {
        use crate::schema::subscription::dsl as s_dsl;
        use diesel_async::RunQueryDsl;

        let query = s_dsl::subscription
            .select(s_dsl::backdate_invoices)
            .filter(s_dsl::id.eq(subscription_id));

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while fetching subscription backdate flag")
            .into_db_result()
    }

    /// `(subscription_id, backdate_invoices, auto_advance_invoices, grace_period_hours,
    /// consolidate_recurring_invoices)` per subscription; grace resolved via the customer's
    /// invoicing entity.
    pub async fn get_subscriptions_projection_context(
        conn: &mut PgConn,
        subscription_ids: &[SubscriptionId],
    ) -> DbResult<Vec<(SubscriptionId, bool, bool, i32, bool, Option<NaiveDateTime>)>> {
        use crate::schema::customer::dsl as c_dsl;
        use crate::schema::invoicing_entity::dsl as ie_dsl;
        use crate::schema::subscription::dsl as s_dsl;
        use diesel_async::RunQueryDsl;

        let query = s_dsl::subscription
            .inner_join(c_dsl::customer.on(s_dsl::customer_id.eq(c_dsl::id)))
            .inner_join(ie_dsl::invoicing_entity.on(c_dsl::invoicing_entity_id.eq(ie_dsl::id)))
            .filter(s_dsl::id.eq_any(subscription_ids))
            .select((
                s_dsl::id,
                s_dsl::backdate_invoices,
                s_dsl::auto_advance_invoices,
                ie_dsl::grace_period_hours,
                ie_dsl::consolidate_recurring_invoices,
                s_dsl::terminated_at,
            ));

        crate::debug_sql(&query);

        query
            .load(conn)
            .await
            .attach("Error while fetching subscriptions projection context")
            .into_db_result()
    }

    pub async fn find_child_id_by_parent(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_parent_invoice_id: InvoiceId,
    ) -> DbResult<Option<InvoiceId>> {
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        i_dsl::invoice
            .filter(i_dsl::tenant_id.eq(param_tenant_id))
            .filter(i_dsl::parent_invoice_id.eq(param_parent_invoice_id))
            .select(i_dsl::id)
            .first::<InvoiceId>(conn)
            .await
            .optional()
            .attach("Error while finding child invoice by parent id")
            .into_db_result()
    }

    /// Batch form of [`Self::find_child_id_by_parent`], returning `(parent_id, child_id)` pairs
    /// for the parents that have a correction. Keeps list endpoints at one query per page.
    pub async fn find_child_ids_by_parents(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_parent_invoice_ids: &[InvoiceId],
    ) -> DbResult<Vec<(InvoiceId, InvoiceId)>> {
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        if param_parent_invoice_ids.is_empty() {
            return Ok(vec![]);
        }

        i_dsl::invoice
            .filter(i_dsl::tenant_id.eq(param_tenant_id))
            .filter(i_dsl::parent_invoice_id.eq_any(param_parent_invoice_ids))
            .select((i_dsl::parent_invoice_id.assume_not_null(), i_dsl::id))
            .load::<(InvoiceId, InvoiceId)>(conn)
            .await
            .attach("Error while finding child invoices by parent ids")
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

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while finding invoice by id")
            .into_db_result()
    }

    /// Sets only the `custom_properties` bag on an invoice. Allowed at any status: custom
    /// properties are external workflow metadata (e.g. tax-filing / accounting-export status) that
    /// legitimately changes after finalization, unlike the invoice's financial content.
    /// Returns `None` only when the invoice doesn't exist.
    pub async fn update_custom_properties(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_invoice_id: InvoiceId,
        custom_properties: serde_json::Value,
    ) -> DbResult<Option<InvoiceRow>> {
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(i_dsl::invoice)
            .filter(i_dsl::tenant_id.eq(param_tenant_id))
            .filter(i_dsl::id.eq(param_invoice_id))
            .set(i_dsl::custom_properties.eq(custom_properties));

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .optional()
            .attach("Error while updating invoice custom_properties")
            .into_db_result()
    }

    /// Stamps (or clears, with `None`) the collection decision; the only writer of the column.
    pub async fn set_auto_charge_requested(
        conn: &mut PgConn,
        param_invoice_id: InvoiceId,
        param_tenant_id: TenantId,
        requested_at: Option<NaiveDateTime>,
    ) -> DbResult<usize> {
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(i_dsl::invoice)
            .filter(i_dsl::tenant_id.eq(param_tenant_id))
            .filter(i_dsl::id.eq(param_invoice_id))
            .set(i_dsl::auto_charge_requested_at.eq(requested_at));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while setting invoice auto_charge_requested_at")
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

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while finding invoice by id")
            .into_db_result()
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn list(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_customer_id: Option<CustomerId>,
        param_subscription_id: Option<SubscriptionId>,
        param_status: Option<InvoiceStatusEnum>,
        param_payment_status: Option<InvoicePaymentStatus>,
        param_einvoicing_status: Option<EInvoicingStatusEnum>,
        param_overdue_at: Option<NaiveDateTime>,
        param_issued_only: bool,
        param_query: Option<String>,
        filters: Vec<ListFilter>,
        order_by: Option<&str>,
        pagination: PaginationRequest,
    ) -> DbResult<PaginatedVec<InvoiceWithCustomerRow>> {
        use crate::schema::customer::dsl as c_dsl;
        use crate::schema::invoice::dsl as i_dsl;

        let mut query = i_dsl::invoice
            .inner_join(c_dsl::customer.on(i_dsl::customer_id.eq(c_dsl::id)))
            .filter(i_dsl::tenant_id.eq(param_tenant_id))
            .select(InvoiceWithCustomerRow::as_select())
            .into_boxed();

        // Hide consolidated children from customer/global listings (the parent represents them).
        // A subscription-scoped listing keeps them, so a subscription still shows its own
        // contribution (linked to the consolidated parent it was merged into).
        if param_subscription_id.is_none() {
            query = query.filter(i_dsl::consolidated_into_invoice_id.is_null());
        }

        if let Some(param_customer_id) = param_customer_id {
            query = query.filter(i_dsl::customer_id.eq(param_customer_id));
        }

        if let Some(param_subscription_id) = param_subscription_id {
            let parents =
                Self::consolidated_parent_ids(conn, param_tenant_id, param_subscription_id).await?;
            query = query.filter(
                i_dsl::subscription_id
                    .eq(param_subscription_id)
                    .or(i_dsl::id.eq_any(parents)),
            );
        }

        if let Some(param_status) = param_status {
            query = query.filter(i_dsl::status.eq(param_status));
        }

        if let Some(param_payment_status) = param_payment_status {
            query = query.filter(i_dsl::payment_status.eq(param_payment_status));
        }

        if let Some(param_einvoicing_status) = param_einvoicing_status {
            query = query.filter(i_dsl::einvoicing_status.eq(param_einvoicing_status));
        }

        // Overdue as the receivables count it: finalized, still owed, due before the day of `at`.
        if let Some(at) = param_overdue_at {
            query = query
                .filter(i_dsl::status.eq(InvoiceStatusEnum::Finalized))
                .filter(i_dsl::payment_status.eq_any([
                    InvoicePaymentStatus::Unpaid,
                    InvoicePaymentStatus::PartiallyPaid,
                    InvoicePaymentStatus::Errored,
                ]))
                .filter(i_dsl::amount_due.gt(0))
                .filter(i_dsl::voided_at.is_null())
                .filter(i_dsl::marked_as_uncollectible_at.is_null())
                .filter(i_dsl::consolidated_into_invoice_id.is_null())
                .filter(i_dsl::due_at.lt(at.date().and_time(chrono::NaiveTime::MIN)));
        }

        // "Issued" = anything a customer should see: everything except a not-yet-finalized Draft or
        // a terminal Closed (a period that settled with nothing to bill). Both are the only statuses
        // that lack `finalized_at`, so gate on that instead of enumerating statuses — this keeps
        // pagination correct (the filter runs in SQL, not after the page is fetched) and excludes
        // any future non-issued state automatically.
        if param_issued_only {
            query = query.filter(i_dsl::finalized_at.is_not_null());
        }

        if let Some(param_query) = param_query
            && !param_query.trim().is_empty()
        {
            let pattern = format!("%{param_query}%");
            query = query.filter(
                c_dsl::name
                    .ilike(pattern.clone())
                    .or(i_dsl::invoice_number.ilike(pattern)),
            );
        }

        for f in filters {
            query = query.filter(render_list_filter(f, "invoice.custom_properties"));
        }

        let order = OrderByParam::parse(order_by, "billing_period_start.desc");

        match (order.column.as_str(), order.direction) {
            ("created_at", OrderDirection::Asc) => {
                query = query.order((i_dsl::created_at.asc(), i_dsl::id.asc()))
            }
            ("created_at", OrderDirection::Desc) => {
                query = query.order((i_dsl::created_at.desc(), i_dsl::id.desc()))
            }
            ("invoice_number", OrderDirection::Asc) => {
                query = query.order((i_dsl::invoice_number.asc(), i_dsl::id.asc()))
            }
            ("invoice_number", OrderDirection::Desc) => {
                query = query.order((i_dsl::invoice_number.desc(), i_dsl::id.desc()))
            }
            ("customer_name", OrderDirection::Asc) => {
                query = query.order((c_dsl::name.asc(), i_dsl::id.asc()))
            }
            ("customer_name", OrderDirection::Desc) => {
                query = query.order((c_dsl::name.desc(), i_dsl::id.desc()))
            }
            ("amount", OrderDirection::Asc) => {
                query = query.order((i_dsl::total.asc(), i_dsl::id.asc()))
            }
            ("amount", OrderDirection::Desc) => {
                query = query.order((i_dsl::total.desc(), i_dsl::id.desc()))
            }
            ("invoice_date", OrderDirection::Asc) => {
                query = query.order((i_dsl::invoice_date.asc(), i_dsl::id.asc()))
            }
            ("invoice_date", OrderDirection::Desc) => {
                query = query.order((i_dsl::invoice_date.desc(), i_dsl::id.desc()))
            }
            ("billing_period_start", OrderDirection::Asc) => {
                query = query.order((i_dsl::billing_period_start.asc(), i_dsl::id.asc()))
            }
            ("billing_period_start", OrderDirection::Desc) => {
                query = query.order((i_dsl::billing_period_start.desc(), i_dsl::id.desc()))
            }
            ("status", OrderDirection::Asc) => {
                query = query.order((i_dsl::status.asc(), i_dsl::id.asc()))
            }
            ("status", OrderDirection::Desc) => {
                query = query.order((i_dsl::status.desc(), i_dsl::id.desc()))
            }
            ("payment_status", OrderDirection::Asc) => {
                query = query.order((i_dsl::payment_status.asc(), i_dsl::id.asc()))
            }
            ("payment_status", OrderDirection::Desc) => {
                query = query.order((i_dsl::payment_status.desc(), i_dsl::id.desc()))
            }
            _ => query = query.order((i_dsl::billing_period_start.desc(), i_dsl::id.desc())),
        }

        let paginated_query = query.paginate(pagination);

        crate::debug_sql(&paginated_query);

        paginated_query
            .load_and_count_pages(conn)
            .await
            .attach("Error while fetching invoices")
            .into_db_result()
    }

    /// The issued supplies whose tax point (`invoice_date`) falls within `[from, to]`: finalized
    /// or uncollectible, consolidated children left to their parent, in `(invoice_date, id)`
    /// order. A page resumes after `after`, the previous page's last `(invoice_date, id)`.
    pub async fn list_for_tax_report_page(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        from: chrono::NaiveDate,
        to: chrono::NaiveDate,
        param_invoicing_entity_id: Option<InvoicingEntityId>,
        after: Option<(chrono::NaiveDate, InvoiceId)>,
        limit: i64,
    ) -> DbResult<Vec<InvoiceRow>> {
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        let mut query = i_dsl::invoice
            .filter(i_dsl::tenant_id.eq(param_tenant_id))
            .filter(i_dsl::consolidated_into_invoice_id.is_null())
            .filter(i_dsl::status.eq_any(vec![
                InvoiceStatusEnum::Finalized,
                InvoiceStatusEnum::Uncollectible,
            ]))
            .filter(i_dsl::invoice_date.ge(from))
            .filter(i_dsl::invoice_date.le(to))
            .select(InvoiceRow::as_select())
            .into_boxed();

        if let Some(entity_id) = param_invoicing_entity_id {
            query = query.filter(i_dsl::invoicing_entity_id.eq(entity_id));
        }

        // Keyset cursor: strictly after (date, id) under the (invoice_date, id) order.
        if let Some((after_date, after_id)) = after {
            query = query.filter(
                i_dsl::invoice_date.gt(after_date).or(i_dsl::invoice_date
                    .eq(after_date)
                    .and(i_dsl::id.gt(after_id))),
            );
        }

        query
            .order((i_dsl::invoice_date.asc(), i_dsl::id.asc()))
            .limit(limit)
            .load(conn)
            .await
            .attach("Error while fetching a tax report page")
            .into_db_result()
    }

    pub async fn list_with_transactions(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_customer_id: Option<AliasOr<CustomerId>>,
        param_subscription_id: Option<SubscriptionId>,
        param_statuses: Option<Vec<InvoiceStatusEnum>>,
        param_einvoicing_status: Option<EInvoicingStatusEnum>,
        param_query: Option<String>,
        order_by: Option<&str>,
        pagination: PaginationRequest,
    ) -> DbResult<PaginatedVec<(InvoiceWithCustomerRow, Vec<PaymentTransactionRow>)>> {
        use crate::schema::customer::dsl as c_dsl;
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        let mut query = i_dsl::invoice
            .inner_join(c_dsl::customer.on(i_dsl::customer_id.eq(c_dsl::id)))
            .filter(i_dsl::tenant_id.eq(param_tenant_id))
            .select(InvoiceWithCustomerRow::as_select())
            .into_boxed();

        // Hide consolidated children from customer/global listings (the parent represents them);
        // keep them in a subscription-scoped listing so the subscription shows its contribution.
        if param_subscription_id.is_none() {
            query = query.filter(i_dsl::consolidated_into_invoice_id.is_null());
        }

        if let Some(param_customer_id) = param_customer_id {
            match param_customer_id {
                AliasOr::Id(id) => {
                    query = query.filter(i_dsl::customer_id.eq(id));
                }
                AliasOr::Alias(alias) => {
                    query = query.filter(c_dsl::alias.eq(alias));
                }
            };
        }

        if let Some(param_subscription_id) = param_subscription_id {
            let parents =
                Self::consolidated_parent_ids(conn, param_tenant_id, param_subscription_id).await?;
            query = query.filter(
                i_dsl::subscription_id
                    .eq(param_subscription_id)
                    .or(i_dsl::id.eq_any(parents)),
            );
        }

        if let Some(param_statuses) = param_statuses {
            query = query.filter(i_dsl::status.eq_any(param_statuses));
        }

        if let Some(param_einvoicing_status) = param_einvoicing_status {
            query = query.filter(i_dsl::einvoicing_status.eq(param_einvoicing_status));
        }

        if let Some(param_query) = param_query
            && !param_query.trim().is_empty()
        {
            let pattern = format!("%{param_query}%");
            query = query.filter(
                c_dsl::name
                    .ilike(pattern.clone())
                    .or(i_dsl::invoice_number.ilike(pattern)),
            );
        }

        let order = OrderByParam::parse(order_by, "billing_period_start.desc");

        match (order.column.as_str(), order.direction) {
            ("created_at", OrderDirection::Asc) => {
                query = query.order((i_dsl::created_at.asc(), i_dsl::id.asc()))
            }
            ("created_at", OrderDirection::Desc) => {
                query = query.order((i_dsl::created_at.desc(), i_dsl::id.desc()))
            }
            ("invoice_number", OrderDirection::Asc) => {
                query = query.order((i_dsl::invoice_number.asc(), i_dsl::id.asc()))
            }
            ("invoice_number", OrderDirection::Desc) => {
                query = query.order((i_dsl::invoice_number.desc(), i_dsl::id.desc()))
            }
            ("customer_name", OrderDirection::Asc) => {
                query = query.order((c_dsl::name.asc(), i_dsl::id.asc()))
            }
            ("customer_name", OrderDirection::Desc) => {
                query = query.order((c_dsl::name.desc(), i_dsl::id.desc()))
            }
            ("amount", OrderDirection::Asc) => {
                query = query.order((i_dsl::total.asc(), i_dsl::id.asc()))
            }
            ("amount", OrderDirection::Desc) => {
                query = query.order((i_dsl::total.desc(), i_dsl::id.desc()))
            }
            ("invoice_date", OrderDirection::Asc) => {
                query = query.order((i_dsl::invoice_date.asc(), i_dsl::id.asc()))
            }
            ("invoice_date", OrderDirection::Desc) => {
                query = query.order((i_dsl::invoice_date.desc(), i_dsl::id.desc()))
            }
            ("billing_period_start", OrderDirection::Asc) => {
                query = query.order((i_dsl::billing_period_start.asc(), i_dsl::id.asc()))
            }
            ("billing_period_start", OrderDirection::Desc) => {
                query = query.order((i_dsl::billing_period_start.desc(), i_dsl::id.desc()))
            }
            ("status", OrderDirection::Asc) => {
                query = query.order((i_dsl::status.asc(), i_dsl::id.asc()))
            }
            ("status", OrderDirection::Desc) => {
                query = query.order((i_dsl::status.desc(), i_dsl::id.desc()))
            }
            ("payment_status", OrderDirection::Asc) => {
                query = query.order((i_dsl::payment_status.asc(), i_dsl::id.asc()))
            }
            ("payment_status", OrderDirection::Desc) => {
                query = query.order((i_dsl::payment_status.desc(), i_dsl::id.desc()))
            }
            _ => query = query.order((i_dsl::billing_period_start.desc(), i_dsl::id.desc())),
        }

        let paginated_query = query.paginate(pagination);

        crate::debug_sql(&paginated_query);

        let paginated_rows: PaginatedVec<InvoiceWithCustomerRow> = paginated_query
            .load_and_count_pages(conn)
            .await
            .attach("Error while fetching invoices")
            .into_db_result()?;

        // Fetch transactions using belonging_to
        let invoices: Vec<&InvoiceRow> = paginated_rows
            .items
            .iter()
            .map(|row| &row.invoice)
            .collect();

        let transactions: Vec<PaymentTransactionRow> =
            PaymentTransactionRow::belonging_to(&invoices)
                .select(PaymentTransactionRow::as_select())
                .load(conn)
                .await
                .attach("Error while listing payment transactions for invoices")
                .into_db_result()?;

        // Group transactions by invoice
        let items: Vec<(InvoiceWithCustomerRow, Vec<PaymentTransactionRow>)> = paginated_rows
            .items
            .into_iter()
            .map(|invoice_row| {
                let txs: Vec<PaymentTransactionRow> = transactions
                    .iter()
                    .filter(|tx| tx.invoice_id == Some(invoice_row.invoice.id))
                    .cloned()
                    .collect();
                (invoice_row, txs)
            })
            .collect();

        Ok(PaginatedVec {
            total_results: paginated_rows.total_results,
            total_pages: paginated_rows.total_pages,
            items,
        })
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

        crate::debug_sql(&query);

        query
            .load(conn)
            .await
            .attach("Error while fetching invoices by ids")
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

        crate::debug_sql(&query);

        query
            .load(conn)
            .await
            .attach("Error while finding invoice by id")
            .into_db_result()
    }

    pub async fn insert_invoice_batch(
        conn: &mut PgConn,
        invoices: Vec<InvoiceRowNew>,
    ) -> DbResult<Vec<InvoiceRow>> {
        use crate::schema::invoice::dsl::invoice;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(invoice).values(
            invoices
                .iter()
                .map(|row| {
                    (
                        row,
                        crate::schema::invoice::created_at.eq(common_domain::clock::now()),
                    )
                })
                .collect::<Vec<_>>(),
        );

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while inserting invoice")
            .into_db_result()
    }

    pub async fn finalize(
        conn: &mut PgConn,
        id: InvoiceId,
        tenant_id: TenantId,
        new_invoice_number: String,
        payment_reference: String,
        coupons: serde_json::Value,
        finalized_at: NaiveDateTime,
        invoice_date: chrono::NaiveDate,
        due_at: Option<NaiveDateTime>,
    ) -> DbResult<usize> {
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        let now = common_domain::clock::now().naive_utc();

        let query = diesel::update(i_dsl::invoice)
            .filter(i_dsl::id.eq(id))
            .filter(i_dsl::tenant_id.eq(tenant_id))
            .filter(i_dsl::status.ne_all(vec![
                InvoiceStatusEnum::Finalized,
                InvoiceStatusEnum::Void,
                InvoiceStatusEnum::Closed,
            ]))
            .set((
                i_dsl::status.eq(InvoiceStatusEnum::Finalized),
                i_dsl::updated_at.eq(now),
                i_dsl::data_updated_at.eq(now),
                i_dsl::finalized_at.eq(finalized_at),
                i_dsl::invoice_date.eq(invoice_date),
                i_dsl::due_at.eq(due_at),
                i_dsl::invoice_number.eq(new_invoice_number),
                i_dsl::reference.eq(payment_reference),
                i_dsl::coupons.eq(coupons),
            ));

        crate::debug_sql(&query);

        let updated = query
            .execute(conn)
            .await
            .attach("Error while finalizing invoice")
            .into_db_result()?;

        // Same lost-update guard as `close`: callers hold FOR UPDATE and have checked `can_edit`,
        // so a zero-row update means the invoice reached a terminal state under us. Left unchecked
        // it still bumps the invoicing entity's number counter and emits `invoice.finalized` for an
        // invoice that was never finalized.
        if updated != 1 {
            return Err(crate::errors::DatabaseError::ValidationError(format!(
                "finalize() updated {updated} rows for invoice {id}, expected exactly 1"
            ))
            .into());
        }

        Ok(updated)
    }

    /// What the invoice still asks for after the credits and credit notes applied to it — the
    /// figure `amount_due` is measured against. The issued `total` never moves, so comparing
    /// against it would read a fully credited, fully unpaid invoice as partially paid.
    pub async fn collectible_total(
        conn: &mut PgConn,
        id: InvoiceId,
        tenant_id: TenantId,
    ) -> DbResult<i64> {
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        let (total, applied_credits): (i64, i64) = i_dsl::invoice
            .filter(i_dsl::id.eq(id))
            .filter(i_dsl::tenant_id.eq(tenant_id))
            .select((i_dsl::total, i_dsl::applied_credits))
            .first(conn)
            .await
            .attach("Error while loading invoice total")
            .into_db_result()?;

        let cancelled =
            crate::credit_notes::CreditNoteRow::sum_finalized_invoice_reducing(conn, id, tenant_id)
                .await?;
        Ok((total - applied_credits - cancelled).max(0))
    }

    /// Records when a dispute first withdrew money still lost on the invoice's payments, or
    /// clears it once none is; the earliest stamp is kept while some stays disputed.
    pub async fn sync_payment_disputed_at(
        conn: &mut PgConn,
        id: InvoiceId,
        tenant_id: TenantId,
        disputed: bool,
        at: NaiveDateTime,
    ) -> DbResult<()> {
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        let target = i_dsl::invoice
            .filter(i_dsl::id.eq(id))
            .filter(i_dsl::tenant_id.eq(tenant_id));
        let query = if disputed {
            diesel::update(target.filter(i_dsl::payment_disputed_at.is_null()))
                .set(i_dsl::payment_disputed_at.eq(Some(at)))
                .execute(conn)
                .await
        } else {
            diesel::update(target.filter(i_dsl::payment_disputed_at.is_not_null()))
                .set(i_dsl::payment_disputed_at.eq(None::<NaiveDateTime>))
                .execute(conn)
                .await
        };
        query
            .map(|_| ())
            .attach("Error while recording an invoice's disputed payment")
            .into_db_result()
    }

    pub async fn apply_payment_status(
        conn: &mut PgConn,
        id: InvoiceId,
        tenant_id: TenantId,
        payment_status: InvoicePaymentStatus,
        tx_at: Option<NaiveDateTime>,
    ) -> DbResult<InvoiceRow> {
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        let now = common_domain::clock::now().naive_utc();
        let paid_at = if payment_status == InvoicePaymentStatus::Paid {
            Some(tx_at.unwrap_or(now))
        } else {
            None
        };

        let query = diesel::update(i_dsl::invoice)
            .filter(i_dsl::id.eq(id))
            .filter(i_dsl::tenant_id.eq(tenant_id))
            .set((
                i_dsl::payment_status.eq(payment_status),
                i_dsl::paid_at.eq(paid_at),
                i_dsl::updated_at.eq(now),
            ));

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while applying payment status to invoice")
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

        let now = common_domain::clock::now().naive_utc();

        let query = diesel::update(i_dsl::invoice)
            .filter(i_dsl::id.eq(id))
            .filter(i_dsl::tenant_id.eq(tenant_id))
            .set((
                i_dsl::updated_at.eq(now),
                i_dsl::amount_due.eq(i_dsl::amount_due - transaction_amount),
            ));

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while applying transaction to invoice")
            .into_db_result()
    }

    /// What the invoice asks and what its collections did for it. See [`InvoiceCollections`].
    pub async fn collections(
        conn: &mut PgConn,
        id: InvoiceId,
        tenant_id: TenantId,
    ) -> DbResult<InvoiceCollections> {
        use crate::schema::invoice::dsl as i_dsl;
        use crate::schema::payment_transaction::dsl as pt_dsl;
        use diesel_async::RunQueryDsl;

        let settled: Vec<(i64, i64, i64)> = pt_dsl::payment_transaction
            .filter(pt_dsl::invoice_id.eq(id))
            .filter(pt_dsl::tenant_id.eq(tenant_id))
            // `Refunded` belongs here too: a payment fully given back voluntarily still
            // collected the money once, and its refund rows are what take it away below.
            .filter(pt_dsl::status.eq_any([
                crate::enums::PaymentStatusEnum::Settled,
                crate::enums::PaymentStatusEnum::Refunded,
            ]))
            .filter(pt_dsl::payment_type.eq_any(crate::enums::PaymentTypeEnum::COLLECTIONS))
            .select((
                pt_dsl::amount,
                pt_dsl::amount_reversed,
                pt_dsl::amount_disputed,
            ))
            .load(conn)
            .await
            .attach("Error while summing settled payments")
            .into_db_result()?;
        // A refund drawn against an avoir is netted because that avoir also shrank what is owed.
        let refunded_against_notes =
            crate::payments::PaymentTransactionRow::sum_credit_note_backed_refunds_by_invoice(
                conn, id, tenant_id,
            )
            .await?;
        let excess_refunded =
            crate::payments::PaymentTransactionRow::sum_live_refund_excess_by_invoice(
                conn, id, tenant_id,
            )
            .await?;
        let refunded_without_notes =
            crate::payments::PaymentTransactionRow::sum_live_refunds_without_note_by_invoice(
                conn, id, tenant_id,
            )
            .await?;
        // Finalized DebtCancellation and Refund credit notes reduce what is owed; keep them
        // netted out here or a later settlement silently re-bills the cancelled debt.
        let cancelled_sum =
            crate::credit_notes::CreditNoteRow::sum_finalized_invoice_reducing(conn, id, tenant_id)
                .await?;
        let refundable =
            crate::credit_notes::CreditNoteRow::sum_finalized_refundable(conn, id, tenant_id)
                .await?;
        let (total, applied_credits): (i64, i64) = i_dsl::invoice
            .filter(i_dsl::id.eq(id))
            .filter(i_dsl::tenant_id.eq(tenant_id))
            .select((i_dsl::total, i_dsl::applied_credits))
            .first(conn)
            .await
            .attach("Error while loading invoice total")
            .into_db_result()?;

        let (mut gross, mut failed, mut disputed) = (0_i64, 0_i64, 0_i64);
        for (amount, reversed, in_dispute) in settled {
            gross += amount;
            failed += reversed - in_dispute;
            disputed += in_dispute;
        }
        let owed = total - applied_credits - cancelled_sum;
        let gross = gross - refunded_against_notes;
        Ok(InvoiceCollections {
            owed,
            gross,
            failed,
            disputed,
            excess_refunded,
            refunded: (refunded_without_notes - excess_refunded).max(0),
            owed_back: (refundable - refunded_against_notes).max(0),
        })
    }

    /// What the invoice's collections keep above what it asks, net of cancelled debt. Refund notes
    /// and their refunds cancel out; a refund with no note, from the moment it is issued, gives
    /// back the excess first.
    pub async fn overpaid_by_collections(
        conn: &mut PgConn,
        id: InvoiceId,
        tenant_id: TenantId,
    ) -> DbResult<i64> {
        use crate::schema::invoice::dsl as i_dsl;
        use crate::schema::payment_transaction::dsl as pt_dsl;
        use diesel_async::RunQueryDsl;

        let collections: Vec<(i64, i64)> = pt_dsl::payment_transaction
            .filter(pt_dsl::invoice_id.eq(id))
            .filter(pt_dsl::tenant_id.eq(tenant_id))
            .filter(pt_dsl::status.eq_any([
                crate::enums::PaymentStatusEnum::Settled,
                crate::enums::PaymentStatusEnum::Refunded,
            ]))
            .filter(pt_dsl::payment_type.eq_any(crate::enums::PaymentTypeEnum::COLLECTIONS))
            .select((pt_dsl::amount, pt_dsl::amount_reversed))
            .load(conn)
            .await
            .attach("Error while summing collections")
            .into_db_result()?;
        let refunded_without_notes =
            crate::payments::PaymentTransactionRow::sum_live_refunds_without_note_by_invoice(
                conn, id, tenant_id,
            )
            .await?;
        let cancelled =
            crate::credit_notes::CreditNoteRow::sum_finalized_debt_cancelled(conn, id, tenant_id)
                .await?;
        let (total, applied_credits): (i64, i64) = i_dsl::invoice
            .filter(i_dsl::id.eq(id))
            .filter(i_dsl::tenant_id.eq(tenant_id))
            .select((i_dsl::total, i_dsl::applied_credits))
            .first(conn)
            .await
            .attach("Error while loading invoice total")
            .into_db_result()?;
        let due = total - applied_credits - cancelled;
        let collected: i64 = collections
            .into_iter()
            .map(|(amount, reversed)| amount - reversed)
            .sum();
        Ok((collected - refunded_without_notes - due).max(0))
    }

    /// Idempotently recompute `amount_due` from [`InvoiceCollections::amount_due`]: safe under
    /// queue redelivery and never negative. A failed collection reopens the invoice; money lost
    /// to a dispute and a voluntary refund leave it as it stands. The caller must have locked the
    /// invoice row and persisted the transaction state.
    pub async fn recompute_amount_due_from_settled_payments(
        conn: &mut PgConn,
        id: InvoiceId,
        tenant_id: TenantId,
    ) -> DbResult<InvoiceRow> {
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        let new_amount_due = Self::collections(conn, id, tenant_id).await?.amount_due();
        let now = common_domain::clock::now().naive_utc();

        let query = diesel::update(i_dsl::invoice)
            .filter(i_dsl::id.eq(id))
            .filter(i_dsl::tenant_id.eq(tenant_id))
            .set((
                i_dsl::updated_at.eq(now),
                i_dsl::amount_due.eq(new_amount_due),
            ));

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while recomputing invoice amount due")
            .into_db_result()
    }

    pub async fn save_invoice_documents(
        conn: &mut PgConn,
        id: InvoiceId,
        tenant_id: TenantId,
        pdf_document_id: StoredDocumentId,
        xml_document_id: Option<StoredDocumentId>,
        einvoicing_status: Option<EInvoicingStatusEnum>,
        einvoicing_report: Option<serde_json::Value>,
    ) -> DbResult<usize> {
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(i_dsl::invoice)
            .filter(i_dsl::id.eq(id))
            .filter(i_dsl::tenant_id.eq(tenant_id))
            .set((
                i_dsl::pdf_document_id.eq(pdf_document_id),
                i_dsl::xml_document_id.eq(xml_document_id),
                i_dsl::einvoicing_status.eq(einvoicing_status),
                i_dsl::einvoicing_report.eq(einvoicing_report),
            ));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while saving invoice documents")
            .into_db_result()
    }

    /// Subscription-period drafts whose stored lines may have drifted from current usage,
    /// for the background refresh sweep.
    ///
    /// Staleness is measured from `data_updated_at` (the last refresh), not from
    /// `invoice_date`: gating on the invoice date makes every draft older than the interval
    /// match forever, so the sweep would never converge.
    ///
    /// `horizon_days` bounds how long a draft stays worth recomputing, measured from
    /// `billing_period_start`. Period and grace are two
    /// different axes: the billing period selects events by `timestamp` (the only time bound the
    /// metering query carries), while grace is a wall-clock allowance for those events to be
    /// *ingested* before the invoice finalizes. This horizon sits on the grace axis — it is the
    /// ingestion allowance for a draft that never got one, since `auto_advance = false` schedules
    /// no finalization and would otherwise be recomputed hourly forever.
    ///
    /// Being on that axis, it must never cut in before grace itself: while an entity's grace
    /// window is open the system is still explicitly waiting for late usage on that period, so a
    /// draft stays eligible for the longer of the two. Past both, refreshing bets on late events
    /// still arriving; nothing structurally stops one (a backdated event is always in-window), but
    /// re-querying metering hourly forever on that chance is not worth it.
    ///
    /// This bounds the background sweep only: the operator's refresh action has no horizon, and
    /// every finalization path recomputes from current usage, so what is billed never depends on
    /// it — only what a draft displays beforehand.
    ///
    /// Excluded here rather than in the caller, so a large backlog of ineligible drafts can
    /// never crowd out a page:
    /// - non-`Draft` statuses — `can_edit()` is `Draft`-only, so refreshing one errors;
    /// - non-`Recurring` or subscription-less invoices — `compute_invoice` reuses their stored
    ///   lines by design, so a refresh can only cost a round trip;
    /// - consolidated children — the parent is the canonical document;
    /// - manually edited drafts — a refresh regenerates the lines the operator took ownership of;
    /// - drafts with a payment in flight — moving `amount_due` under a live intent desyncs the
    ///   invoice from the money the provider is already capturing.
    pub async fn list_refreshable_drafts(
        conn: &mut PgConn,
        stale_after_hours: i32,
        horizon_days: i64,
        pagination: CursorPaginationRequest,
    ) -> DbResult<CursorPaginatedVec<InvoiceRow>> {
        let query = refreshable_drafts_filter(
            stale_after_hours,
            horizon_days,
            common_domain::clock::now().naive_utc(),
        )
        .select(InvoiceRow::as_select())
        .cursor_paginate(pagination, "id");

        crate::debug_sql(&query);

        query
            .load_and_get_next_cursor(conn, |a| a.id.as_uuid())
            .await
            .attach("Error while paginating refreshable draft invoices")
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

    pub async fn delete(conn: &mut PgConn, id: InvoiceId, tenant_id: TenantId) -> DbResult<()> {
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::delete(i_dsl::invoice)
            .filter(i_dsl::id.eq(id))
            .filter(i_dsl::tenant_id.eq(tenant_id))
            .filter(i_dsl::status.eq(InvoiceStatusEnum::Draft));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .map(|_| ())
            .attach("Error while deleting invoice")
            .into_db_result()
    }

    pub async fn void(conn: &mut PgConn, id: InvoiceId, tenant_id: TenantId) -> DbResult<()> {
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(i_dsl::invoice)
            .filter(i_dsl::id.eq(id))
            .filter(i_dsl::tenant_id.eq(tenant_id))
            .filter(i_dsl::status.eq(InvoiceStatusEnum::Finalized))
            .set((
                i_dsl::status.eq(InvoiceStatusEnum::Void),
                i_dsl::updated_at.eq(common_domain::clock::now().naive_utc()),
                i_dsl::voided_at.eq(common_domain::clock::now().naive_utc()),
            ));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .map(|_| ())
            .attach("Error while voiding invoice")
            .into_db_result()
    }

    /// Close a draft that settled with nothing to bill (no line items). A closed invoice is
    /// terminal and never receives a sequential number, so it consumes no numbering slot (no gap).
    ///
    /// The `status = Draft` guard scopes the update to a still-editable invoice. Callers hold
    /// `FOR UPDATE` and have checked `can_edit`, so exactly one row must change; updating zero
    /// means a lost update (the invoice reached a terminal state under us), which we surface as an
    /// error that rolls back the settlement rather than swallow.
    pub async fn close(conn: &mut PgConn, id: InvoiceId, tenant_id: TenantId) -> DbResult<()> {
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(i_dsl::invoice)
            .filter(i_dsl::id.eq(id))
            .filter(i_dsl::tenant_id.eq(tenant_id))
            .filter(i_dsl::status.eq(InvoiceStatusEnum::Draft))
            .set((
                i_dsl::status.eq(InvoiceStatusEnum::Closed),
                i_dsl::updated_at.eq(common_domain::clock::now().naive_utc()),
            ));

        crate::debug_sql(&query);

        let updated = query
            .execute(conn)
            .await
            .attach("Error while closing invoice")
            .into_db_result()?;

        if updated != 1 {
            return Err(crate::errors::DatabaseError::ValidationError(format!(
                "close() updated {updated} rows for invoice {id}, expected exactly 1"
            ))
            .into());
        }

        Ok(())
    }

    pub async fn mark_as_uncollectible(
        conn: &mut PgConn,
        id: InvoiceId,
        tenant_id: TenantId,
    ) -> DbResult<()> {
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(i_dsl::invoice)
            .filter(i_dsl::id.eq(id))
            .filter(i_dsl::tenant_id.eq(tenant_id))
            .filter(i_dsl::status.eq(InvoiceStatusEnum::Finalized))
            .set((
                i_dsl::status.eq(InvoiceStatusEnum::Uncollectible),
                i_dsl::updated_at.eq(common_domain::clock::now().naive_utc()),
                i_dsl::marked_as_uncollectible_at.eq(common_domain::clock::now().naive_utc()),
            ));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .map(|_| ())
            .attach("Error while marking invoice as uncollectible")
            .into_db_result()
    }

    pub async fn find_last_by_subscription_id(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_subscription_id: SubscriptionId,
    ) -> DbResult<Option<InvoiceRow>> {
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        let query = i_dsl::invoice
            .filter(i_dsl::tenant_id.eq(param_tenant_id))
            .filter(i_dsl::subscription_id.eq(param_subscription_id))
            .filter(i_dsl::status.eq(InvoiceStatusEnum::Finalized))
            .order(i_dsl::billing_period_start.desc())
            .limit(1)
            .select(InvoiceRow::as_select());

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding last invoice by subscription id")
            .into_db_result()
    }

    /// The consolidated invoices the subscription's drafts were merged into: they bill it, though
    /// they carry no subscription of their own.
    pub async fn consolidated_parent_ids(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_subscription_id: SubscriptionId,
    ) -> DbResult<Vec<InvoiceId>> {
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        let query = i_dsl::invoice
            .filter(i_dsl::tenant_id.eq(param_tenant_id))
            .filter(i_dsl::subscription_id.eq(param_subscription_id))
            .filter(i_dsl::consolidated_into_invoice_id.is_not_null())
            .group_by(i_dsl::consolidated_into_invoice_id)
            .select(i_dsl::consolidated_into_invoice_id.assume_not_null());

        crate::debug_sql(&query);

        query
            .load(conn)
            .await
            .attach("Error while listing the consolidated invoices of a subscription")
            .into_db_result()
    }

    /// The issued invoices billing the subscription (its own, and the consolidated ones in
    /// `parent_ids`) not yet paid in full, voided or written off, oldest due first.
    pub async fn list_unsettled_by_subscription(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_subscription_id: SubscriptionId,
        parent_ids: &[InvoiceId],
    ) -> DbResult<Vec<UnsettledInvoiceRow>> {
        use crate::schema::invoice::dsl as i_dsl;
        use diesel::PgSortExpressionMethods;
        use diesel_async::RunQueryDsl;

        let query = i_dsl::invoice
            .filter(i_dsl::tenant_id.eq(param_tenant_id))
            .filter(
                i_dsl::subscription_id
                    .eq(param_subscription_id)
                    .or(i_dsl::id.eq_any(parent_ids)),
            )
            .filter(i_dsl::status.eq(InvoiceStatusEnum::Finalized))
            .filter(i_dsl::payment_status.ne(InvoicePaymentStatus::Paid))
            .filter(i_dsl::voided_at.is_null())
            .filter(i_dsl::marked_as_uncollectible_at.is_null())
            .filter(i_dsl::consolidated_into_invoice_id.is_null())
            .order((
                i_dsl::due_at.asc().nulls_last(),
                i_dsl::invoice_date.asc(),
                i_dsl::id.asc(),
            ))
            .select(UnsettledInvoiceRow::as_select());

        crate::debug_sql(&query);

        query
            .load(conn)
            .await
            .attach("Error while listing the unsettled invoices of a subscription")
            .into_db_result()
    }

    /// The lines the subscription contributed to each of `parent_ids`.
    pub async fn list_member_lines(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_subscription_id: SubscriptionId,
        parent_ids: &[InvoiceId],
    ) -> DbResult<Vec<ConsolidatedMemberLinesRow>> {
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        if parent_ids.is_empty() {
            return Ok(Vec::new());
        }
        let query = i_dsl::invoice
            .filter(i_dsl::tenant_id.eq(param_tenant_id))
            .filter(i_dsl::subscription_id.eq(param_subscription_id))
            .filter(i_dsl::consolidated_into_invoice_id.eq_any(parent_ids))
            .select(ConsolidatedMemberLinesRow::as_select());

        crate::debug_sql(&query);

        query
            .load(conn)
            .await
            .attach("Error while listing the consolidated lines of a subscription")
            .into_db_result()
    }

    /// Whether an invoice of the subscription other than `except` bills a credit line of one of its
    /// one-time ("once") credit components or add-ons of `product_id`. A void invoice (or a draft
    /// merged into a voided consolidated invoice) counts only while the credit its line granted
    /// was not taken back whole: credit that lapsed or was spent before the void is never granted
    /// twice.
    pub async fn subscription_billed_once_credit(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_subscription_id: SubscriptionId,
        param_product_id: ProductId,
        except: Option<InvoiceId>,
    ) -> DbResult<bool> {
        use diesel::sql_types;
        use diesel_async::RunQueryDsl;

        #[derive(diesel::QueryableByName)]
        struct Billed {
            #[diesel(sql_type = sql_types::Bool)]
            billed: bool,
        }

        diesel::sql_query(
            r#"
            WITH once_credit AS (
                SELECT sc.id::text AS id, 'sub_component_id' AS line_key
                FROM subscription_component sc
                WHERE sc.subscription_id = $2
                  AND sc.product_id = $3
                  AND sc.legacy_fee @> '{"Credit": {"once": true}}'
                UNION ALL
                SELECT sa.id::text, 'sub_add_on_id'
                FROM subscription_add_on sa
                WHERE sa.subscription_id = $2
                  AND sa.product_id = $3
                  AND sa.legacy_fee @> '{"Credit": {"once": true}}'
            ), included AS (
                SELECT g.id, g.invoice_id, g.line_local_id, g.amount
                FROM customer_credit_grant g
                WHERE g.tenant_id = $1
                  AND g.subscription_id = $2
                  AND g.source_kind = 'INCLUDED_CREDIT'
                  AND g.line_local_id IS NOT NULL
            ), reversed AS (
                SELECT e.grant_id, -SUM(e.amount) AS amount
                FROM customer_credit_entry e
                JOIN included ON included.id = e.grant_id
                WHERE e.kind = 'REVERSAL'
                GROUP BY e.grant_id
            ), kept AS (
                SELECT included.invoice_id, included.line_local_id
                FROM included
                LEFT JOIN reversed ON reversed.grant_id = included.id
                WHERE included.amount > COALESCE(reversed.amount, 0)
            )
            SELECT EXISTS (
                SELECT 1
                FROM invoice i
                JOIN once_credit oc
                  ON i.line_items @> jsonb_build_array(
                         jsonb_build_object('kind', 'Credit', oc.line_key, oc.id))
                LEFT JOIN invoice parent ON parent.id = i.consolidated_into_invoice_id
                LEFT JOIN kept
                  ON kept.invoice_id IN (i.id, parent.id)
                 AND i.line_items @> jsonb_build_array(
                         jsonb_build_object('kind', 'Credit', oc.line_key, oc.id,
                                            'local_id', kept.line_local_id))
                WHERE i.tenant_id = $1
                  AND i.subscription_id = $2
                  AND ($4::uuid IS NULL OR i.id <> $4)
                  AND ((i.status <> 'VOID' AND (parent.id IS NULL OR parent.status <> 'VOID'))
                       OR kept.invoice_id IS NOT NULL)
            ) AS billed
            "#,
        )
        .bind::<sql_types::Uuid, _>(param_tenant_id)
        .bind::<sql_types::Uuid, _>(param_subscription_id)
        .bind::<sql_types::Uuid, _>(param_product_id)
        .bind::<sql_types::Nullable<sql_types::Uuid>, _>(except)
        .get_result::<Billed>(conn)
        .await
        .map(|row| row.billed)
        .attach("Error while looking up a billed one-time credit")
        .into_db_result()
    }

    /// Whether a void invoice of the subscription carried a one-time ("once") credit line of
    /// `product_id`: [`Self::subscription_billed_once_credit`] then decides if it still counts.
    pub async fn subscription_voided_once_credit(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_subscription_id: SubscriptionId,
        param_product_id: ProductId,
    ) -> DbResult<bool> {
        use diesel::sql_types;
        use diesel_async::RunQueryDsl;

        #[derive(diesel::QueryableByName)]
        struct Voided {
            #[diesel(sql_type = sql_types::Bool)]
            voided: bool,
        }

        diesel::sql_query(
            r#"
            WITH once_credit AS (
                SELECT sc.id::text AS id, 'sub_component_id' AS line_key
                FROM subscription_component sc
                WHERE sc.subscription_id = $2
                  AND sc.product_id = $3
                  AND sc.legacy_fee @> '{"Credit": {"once": true}}'
                UNION ALL
                SELECT sa.id::text, 'sub_add_on_id'
                FROM subscription_add_on sa
                WHERE sa.subscription_id = $2
                  AND sa.product_id = $3
                  AND sa.legacy_fee @> '{"Credit": {"once": true}}'
            )
            SELECT EXISTS (
                SELECT 1
                FROM invoice i
                JOIN once_credit oc
                  ON i.line_items @> jsonb_build_array(
                         jsonb_build_object('kind', 'Credit', oc.line_key, oc.id))
                WHERE i.tenant_id = $1
                  AND i.subscription_id = $2
                  AND i.status = 'VOID'
            ) AS voided
            "#,
        )
        .bind::<sql_types::Uuid, _>(param_tenant_id)
        .bind::<sql_types::Uuid, _>(param_subscription_id)
        .bind::<sql_types::Uuid, _>(param_product_id)
        .get_result::<Voided>(conn)
        .await
        .map(|row| row.voided)
        .attach("Error while looking up a voided one-time credit")
        .into_db_result()
    }

    /// Locks (FOR UPDATE, ordered by id to avoid deadlocks) the draft recurring invoices that
    /// can merge into one invoice for a customer, matching the static merge key. The
    /// payment-method part of the key is applied by the caller.
    pub async fn find_consolidation_candidates(
        conn: &mut PgConn,
        lock: &CustomerLock,
        param_invoice_date: chrono::NaiveDate,
        param_currency: &str,
        param_auto_advance: bool,
        param_invoicing_entity_id: InvoicingEntityId,
        param_net_terms: i32,
    ) -> DbResult<Vec<InvoiceRow>> {
        use crate::enums::InvoiceType;
        use crate::query::payment_transactions::LIVE_PAYMENT_STATUSES;
        use crate::schema::invoice::dsl as i_dsl;
        use crate::schema::payment_transaction::dsl as pt_dsl;
        use diesel_async::RunQueryDsl;

        lock.on(conn);
        let (param_tenant_id, param_customer_id) = (lock.tenant_id(), lock.customer_id());
        let query = i_dsl::invoice
            .select(InvoiceRow::as_select())
            .filter(i_dsl::tenant_id.eq(param_tenant_id))
            .filter(i_dsl::customer_id.eq(param_customer_id))
            .filter(i_dsl::billing_period_start.eq(param_invoice_date))
            .filter(i_dsl::currency.eq(param_currency.to_string()))
            .filter(i_dsl::auto_advance.eq(param_auto_advance))
            .filter(i_dsl::invoicing_entity_id.eq(param_invoicing_entity_id))
            // net_terms drives the due date, which the parent inherits from the trigger; only
            // merge subscriptions that share it so the consolidated due date is well-defined.
            .filter(i_dsl::net_terms.eq(param_net_terms))
            .filter(i_dsl::invoice_type.eq(InvoiceType::Recurring))
            .filter(i_dsl::status.eq(InvoiceStatusEnum::Draft))
            .filter(i_dsl::manual.eq(false))
            .filter(i_dsl::subscription_id.is_not_null())
            .filter(i_dsl::parent_invoice_id.is_null())
            .filter(i_dsl::consolidated_into_invoice_id.is_null())
            // A draft with a live payment attached is owned by a payment flow (checkout
            // acceptance, card 3DS). Merging it would bill the same period twice — once via
            // the consolidated parent, once when the in-flight charge settles — and strand
            // the settlement, which cannot finalize a consolidated child.
            .filter(diesel::dsl::not(diesel::dsl::exists(
                pt_dsl::payment_transaction
                    .filter(pt_dsl::invoice_id.eq(i_dsl::id.nullable()))
                    .filter(pt_dsl::tenant_id.eq(param_tenant_id))
                    .filter(pt_dsl::status.eq_any(LIVE_PAYMENT_STATUSES)),
            )))
            .order_by(i_dsl::id.asc())
            .for_update();

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while finding consolidation candidates")
            .into_db_result()
    }

    /// Marks a set of draft invoices as consolidated into a parent invoice. They keep their
    /// Draft status (for MRR/idempotency) but are no longer finalized or charged on their own.
    pub async fn mark_consolidated_into(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        child_ids: &[InvoiceId],
        param_parent_invoice_id: InvoiceId,
    ) -> DbResult<usize> {
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        let now = common_domain::clock::now().naive_utc();

        // Credits are recomputed once on the consolidated parent and deducted there; the child
        // never finalizes, so clear its draft-time applied_credits (and re-derive amount_due) to
        // avoid leaving stale financials that any aggregate over child rows would double-count.
        let query = diesel::update(i_dsl::invoice)
            .filter(i_dsl::tenant_id.eq(param_tenant_id))
            .filter(i_dsl::id.eq_any(child_ids))
            .set((
                i_dsl::consolidated_into_invoice_id.eq(Some(param_parent_invoice_id)),
                i_dsl::applied_credits.eq(0i64),
                i_dsl::amount_due.eq(i_dsl::total),
                i_dsl::updated_at.eq(now),
            ));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while marking invoices as consolidated")
            .into_db_result()
    }

    /// Lists the per-subscription child invoices that were consolidated into the given parent.
    pub async fn list_consolidated_children(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_parent_invoice_id: InvoiceId,
    ) -> DbResult<Vec<InvoiceRow>> {
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        let query = i_dsl::invoice
            .filter(i_dsl::tenant_id.eq(param_tenant_id))
            .filter(i_dsl::consolidated_into_invoice_id.eq(param_parent_invoice_id))
            .order_by(i_dsl::id.asc())
            .select(InvoiceRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing consolidated children")
            .into_db_result()
    }

    /// Transaction-scoped advisory lock, used to serialize consolidation per
    /// (tenant, customer, invoice_date) so concurrent FinalizeInvoice events elect one leader.
    pub async fn advisory_xact_lock(conn: &mut PgConn, key: i64) -> DbResult<()> {
        use diesel::sql_types::BigInt;
        use diesel_async::RunQueryDsl;

        diesel::sql_query("SELECT pg_advisory_xact_lock($1)")
            .bind::<BigInt, _>(key)
            .execute(conn)
            .await
            .attach("Error acquiring consolidation advisory lock")
            .into_db_result()?;
        Ok(())
    }

    /// The recurring invoice that handled the subscription's period starting on that date, a voided
    /// one included (only an explicit regeneration bills a voided period again); a live invoice
    /// comes first.
    pub async fn find_existing_recurring_invoice(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_subscription_id: SubscriptionId,
        param_period_start: chrono::NaiveDate,
    ) -> DbResult<Option<InvoiceRow>> {
        use crate::enums::InvoiceType;
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        let query = i_dsl::invoice
            .filter(i_dsl::tenant_id.eq(param_tenant_id))
            .filter(i_dsl::subscription_id.eq(param_subscription_id))
            .filter(i_dsl::billing_period_start.eq(param_period_start))
            .filter(i_dsl::invoice_type.eq(InvoiceType::Recurring))
            .order((
                i_dsl::status.eq(InvoiceStatusEnum::Void).asc(),
                i_dsl::created_at.desc(),
            ))
            .select(InvoiceRow::as_select());

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding existing recurring invoice")
            .into_db_result()
    }

    /// The subscription's final invoice not voided: it has at most one.
    pub async fn find_final_invoice(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_subscription_id: SubscriptionId,
    ) -> DbResult<Option<InvoiceRow>> {
        use crate::enums::InvoiceType;
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        i_dsl::invoice
            .filter(i_dsl::tenant_id.eq(param_tenant_id))
            .filter(i_dsl::subscription_id.eq(param_subscription_id))
            .filter(i_dsl::invoice_type.eq(InvoiceType::Final))
            .filter(i_dsl::status.ne(InvoiceStatusEnum::Void))
            .select(InvoiceRow::as_select())
            .first(conn)
            .await
            .optional()
            .attach("Error while finding a subscription's final invoice")
            .into_db_result()
    }

    /// Whether a final invoice of the subscription ever billed its last window, voided ones
    /// included: that window is then billed again only by regenerating the final invoice.
    pub async fn final_invoice_owns_last_window(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_subscription_id: SubscriptionId,
    ) -> DbResult<bool> {
        use crate::enums::InvoiceType;
        use crate::schema::invoice::dsl as i_dsl;
        use diesel::dsl::exists;
        use diesel_async::RunQueryDsl;

        diesel::select(exists(
            i_dsl::invoice
                .filter(i_dsl::tenant_id.eq(param_tenant_id))
                .filter(i_dsl::subscription_id.eq(param_subscription_id))
                .filter(i_dsl::invoice_type.eq(InvoiceType::Final)),
        ))
        .get_result(conn)
        .await
        .attach("Error while checking for a subscription's final invoice")
        .into_db_result()
    }

    /// Whether the subscription changed within the period starting on `period_start` after its
    /// invoice was issued at `since`: an adjustment invoice, a component, add-on or seat change
    /// effective in the period, or a plan switch or update applying to it.
    pub async fn subscription_changed_within_period(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_subscription_id: SubscriptionId,
        period_start: chrono::NaiveDate,
        since: NaiveDateTime,
    ) -> DbResult<bool> {
        use diesel::sql_types::{Bool, Date, Timestamp, Uuid};
        use diesel_async::RunQueryDsl;

        #[derive(diesel::QueryableByName)]
        struct Changed {
            #[diesel(sql_type = Bool)]
            changed: bool,
        }

        diesel::sql_query(
            r#"SELECT (
                EXISTS (SELECT 1 FROM invoice
                    WHERE tenant_id = $1 AND subscription_id = $2
                      AND invoice_type = 'ADJUSTMENT' AND status <> 'VOID'
                      AND billing_period_start >= $3)
                OR EXISTS (SELECT 1 FROM subscription_component
                    WHERE subscription_id = $2 AND (effective_from > $3 OR effective_to > $3))
                OR EXISTS (SELECT 1 FROM subscription_add_on
                    WHERE subscription_id = $2
                      AND (effective_from > $3 OR effective_to > $3 OR created_at > $4))
                OR EXISTS (SELECT 1 FROM slot_transaction
                    WHERE subscription_id = $2
                      AND (effective_at > $3::timestamp OR transaction_at > $4))
                OR EXISTS (SELECT 1 FROM subscription_event
                    WHERE subscription_id = $2 AND event_type IN ('SWITCH', 'UPDATED')
                      AND (applies_to > $3 OR (applies_to = $3 AND created_at > $4)))
            ) AS changed"#,
        )
        .bind::<Uuid, _>(param_tenant_id)
        .bind::<Uuid, _>(param_subscription_id)
        .bind::<Date, _>(period_start)
        .bind::<Timestamp, _>(since)
        .get_result::<Changed>(conn)
        .await
        .map(|row| row.changed)
        .attach("Error while checking the subscription's changes within the period")
        .into_db_result()
    }

    /// Whether another recurring invoice of the subscription billed the period starting on that
    /// date (a voided one included).
    pub async fn period_billed_by_another_recurring_invoice(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_subscription_id: SubscriptionId,
        param_period_start: chrono::NaiveDate,
        param_except: InvoiceId,
    ) -> DbResult<bool> {
        use crate::enums::InvoiceType;
        use crate::schema::invoice::dsl as i_dsl;
        use diesel::dsl::exists;
        use diesel_async::RunQueryDsl;

        diesel::select(exists(
            i_dsl::invoice
                .filter(i_dsl::tenant_id.eq(param_tenant_id))
                .filter(i_dsl::subscription_id.eq(param_subscription_id))
                .filter(i_dsl::billing_period_start.eq(param_period_start))
                .filter(i_dsl::invoice_type.eq(InvoiceType::Recurring))
                .filter(i_dsl::id.ne(param_except)),
        ))
        .get_result(conn)
        .await
        .attach("Error while checking the period's other recurring invoices")
        .into_db_result()
    }

    /// The subscription's invoices not issued yet that started by `at`: its own drafts, unless
    /// merged into a consolidated invoice already issued.
    pub async fn list_unissued_started_by(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_subscription_id: SubscriptionId,
        at: chrono::NaiveDate,
    ) -> DbResult<Vec<InvoiceRow>> {
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        let drafts: Vec<InvoiceRow> = i_dsl::invoice
            .filter(i_dsl::tenant_id.eq(param_tenant_id))
            .filter(i_dsl::subscription_id.eq(param_subscription_id))
            .filter(i_dsl::status.eq(InvoiceStatusEnum::Draft))
            .filter(i_dsl::billing_period_start.le(at))
            .select(InvoiceRow::as_select())
            .load(conn)
            .await
            .attach("Error while listing unissued invoices")
            .into_db_result()?;
        let parents: Vec<InvoiceId> = drafts
            .iter()
            .filter_map(|draft| draft.consolidated_into_invoice_id)
            .collect();
        let issued_parents: Vec<InvoiceId> = if parents.is_empty() {
            vec![]
        } else {
            i_dsl::invoice
                .filter(i_dsl::tenant_id.eq(param_tenant_id))
                .filter(i_dsl::id.eq_any(&parents))
                .filter(i_dsl::status.ne(InvoiceStatusEnum::Draft))
                .select(i_dsl::id)
                .load(conn)
                .await
                .attach("Error while reading consolidated invoices")
                .into_db_result()?
        };
        Ok(drafts
            .into_iter()
            .filter(|draft| {
                draft
                    .consolidated_into_invoice_id
                    .is_none_or(|parent| !issued_parents.contains(&parent))
            })
            .collect())
    }

    /// The two earliest period starts of the subscription's recurring invoices on or after `from`,
    /// and the latest before `before` (any status: a voided or closed renewal still opened its period).
    pub async fn recurring_period_starts_around(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_subscription_id: SubscriptionId,
        from: chrono::NaiveDate,
        before: chrono::NaiveDate,
    ) -> DbResult<(Vec<chrono::NaiveDate>, Option<chrono::NaiveDate>)> {
        use crate::enums::InvoiceType;
        use crate::schema::invoice::dsl as i_dsl;
        use diesel::dsl::max;
        use diesel_async::RunQueryDsl;

        let first: Vec<chrono::NaiveDate> = i_dsl::invoice
            .filter(i_dsl::tenant_id.eq(param_tenant_id))
            .filter(i_dsl::subscription_id.eq(param_subscription_id))
            .filter(i_dsl::invoice_type.eq(InvoiceType::Recurring))
            .filter(i_dsl::billing_period_start.ge(from))
            .select(i_dsl::billing_period_start)
            .distinct()
            .order(i_dsl::billing_period_start.asc())
            .limit(2)
            .load(conn)
            .await
            .attach("Error while reading the first renewals of a cadence")
            .into_db_result()?;
        let previous: Option<chrono::NaiveDate> = i_dsl::invoice
            .filter(i_dsl::tenant_id.eq(param_tenant_id))
            .filter(i_dsl::subscription_id.eq(param_subscription_id))
            .filter(i_dsl::invoice_type.eq(InvoiceType::Recurring))
            .filter(i_dsl::billing_period_start.lt(before))
            .select(max(i_dsl::billing_period_start))
            .first(conn)
            .await
            .attach("Error while reading the previous renewal")
            .into_db_result()?;
        Ok((first, previous))
    }

    /// The line items of the subscription's recurring invoices for the period starting `on`.
    pub async fn recurring_line_items_on(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_subscription_id: SubscriptionId,
        on: chrono::NaiveDate,
    ) -> DbResult<Vec<serde_json::Value>> {
        use crate::enums::InvoiceType;
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        i_dsl::invoice
            .filter(i_dsl::tenant_id.eq(param_tenant_id))
            .filter(i_dsl::subscription_id.eq(param_subscription_id))
            .filter(i_dsl::invoice_type.eq(InvoiceType::Recurring))
            .filter(i_dsl::billing_period_start.eq(on))
            .select(i_dsl::line_items)
            .load(conn)
            .await
            .attach("Error while reading a renewal's lines")
            .into_db_result()
    }

    /// The subscription's invoices dated in `[period_start, period_end)` that bill or will bill
    /// (drafts, issued, uncollectible; never void), excluding `except`. A draft merged into a
    /// consolidated invoice is replaced by that invoice once it is issued; it carries no
    /// subscription, so the caller keeps the subscription's lines only.
    pub async fn list_period_spend_invoices(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_subscription_id: SubscriptionId,
        period_start: chrono::NaiveDate,
        period_end: chrono::NaiveDate,
        except: Option<InvoiceId>,
    ) -> DbResult<Vec<InvoiceRow>> {
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        let billing = [
            InvoiceStatusEnum::Draft,
            InvoiceStatusEnum::Finalized,
            InvoiceStatusEnum::Uncollectible,
        ];
        let rows: Vec<InvoiceRow> = i_dsl::invoice
            .filter(i_dsl::tenant_id.eq(param_tenant_id))
            .filter(i_dsl::subscription_id.eq(param_subscription_id))
            .filter(i_dsl::billing_period_start.ge(period_start))
            .filter(i_dsl::billing_period_start.lt(period_end))
            .filter(i_dsl::status.eq_any(billing))
            .select(InvoiceRow::as_select())
            .load(conn)
            .await
            .attach("Error while listing a period's invoices")
            .into_db_result()?;
        let parents: Vec<InvoiceId> = rows
            .iter()
            .filter_map(|row| row.consolidated_into_invoice_id)
            .collect();
        let issued_parents: Vec<InvoiceRow> = if parents.is_empty() {
            vec![]
        } else {
            i_dsl::invoice
                .filter(i_dsl::tenant_id.eq(param_tenant_id))
                .filter(i_dsl::id.eq_any(&parents))
                .filter(i_dsl::status.eq_any([
                    InvoiceStatusEnum::Finalized,
                    InvoiceStatusEnum::Uncollectible,
                ]))
                .select(InvoiceRow::as_select())
                .load(conn)
                .await
                .attach("Error while loading a period's consolidated invoices")
                .into_db_result()?
        };
        let mut invoices: Vec<InvoiceRow> = rows
            .into_iter()
            .filter(|row| Some(row.id) != except)
            .filter(|row| {
                row.consolidated_into_invoice_id
                    .is_none_or(|parent| !issued_parents.iter().any(|p| p.id == parent))
            })
            .collect();
        invoices.extend(issued_parents.into_iter().filter(|p| Some(p.id) != except));
        Ok(invoices)
    }

    /// The Recurring invoice that opened the subscription's billing: its cycle-0 one from its
    /// billing start on (a free trial's come before), else the first predating the cycle column.
    /// None when no such invoice was written. No correction.
    pub async fn find_opening_recurring_invoice(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_subscription_id: SubscriptionId,
    ) -> DbResult<Option<InvoiceRow>> {
        use crate::enums::InvoiceType;
        use crate::schema::invoice::dsl as i_dsl;
        use crate::schema::subscription::dsl as s_dsl;
        use diesel_async::RunQueryDsl;

        let billing_start: Option<chrono::NaiveDate> = s_dsl::subscription
            .filter(s_dsl::tenant_id.eq(param_tenant_id))
            .filter(s_dsl::id.eq(param_subscription_id))
            .select(s_dsl::billing_start_date)
            .first(conn)
            .await
            .optional()
            .attach("Error while reading a subscription's billing start")
            .into_db_result()?
            .flatten();
        let mut query = i_dsl::invoice
            .filter(i_dsl::tenant_id.eq(param_tenant_id))
            .filter(i_dsl::subscription_id.eq(param_subscription_id))
            .filter(i_dsl::invoice_type.eq(InvoiceType::Recurring))
            .filter(i_dsl::parent_invoice_id.is_null())
            .filter(i_dsl::cycle_index.eq(0).or(i_dsl::cycle_index.is_null()))
            .into_boxed();
        if let Some(billing_start) = billing_start {
            query = query.filter(i_dsl::billing_period_start.ge(billing_start));
        }
        query
            .order((
                i_dsl::cycle_index.is_null().asc(),
                i_dsl::billing_period_start.asc(),
                i_dsl::status.eq(InvoiceStatusEnum::Void).asc(),
                i_dsl::created_at.asc(),
            ))
            .select(InvoiceRow::as_select())
            .first(conn)
            .await
            .optional()
            .attach("Error while finding a subscription's opening invoice")
            .into_db_result()
    }

    /// Finalized invoices that advance-billed any part of the current period and
    /// could therefore be credited by a mid-period amendment: the period's
    /// `Recurring` invoice (`billing_period_start == period_start`) plus any `Adjustment`
    /// invoices issued by earlier immediate amendments within the same period
    /// (`period_start <= billing_period_start < period_end`).
    ///
    /// The upper bound is exclusive so the *next* period's recurring invoice
    /// (whose `billing_period_start == period_end`) is never picked up.
    pub async fn list_creditable_period_invoices(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_subscription_id: SubscriptionId,
        period_start: chrono::NaiveDate,
        period_end: chrono::NaiveDate,
    ) -> DbResult<Vec<InvoiceRow>> {
        use crate::enums::InvoiceType;
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        let query = i_dsl::invoice
            .filter(i_dsl::tenant_id.eq(param_tenant_id))
            .filter(i_dsl::subscription_id.eq(param_subscription_id))
            .filter(i_dsl::billing_period_start.ge(period_start))
            .filter(i_dsl::billing_period_start.lt(period_end))
            .filter(i_dsl::status.eq(InvoiceStatusEnum::Finalized))
            // One-off invoices of the subscription bill its seats and add-ons mid-period.
            .filter(
                i_dsl::invoice_type
                    .eq(InvoiceType::Recurring)
                    .or(i_dsl::invoice_type.eq(InvoiceType::Adjustment))
                    .or(i_dsl::invoice_type.eq(InvoiceType::OneOff)),
            )
            .select(InvoiceRow::as_select());

        crate::debug_sql(&query);

        let mut invoices: Vec<InvoiceRow> = query
            .load(conn)
            .await
            .attach("Error while listing creditable period invoices")
            .into_db_result()?;

        // A period draft merged into a consolidated invoice is never issued: its parent, which
        // carries no subscription, billed the period's lines.
        let parents: Vec<InvoiceId> = i_dsl::invoice
            .filter(i_dsl::tenant_id.eq(param_tenant_id))
            .filter(i_dsl::subscription_id.eq(param_subscription_id))
            .filter(i_dsl::billing_period_start.ge(period_start))
            .filter(i_dsl::billing_period_start.lt(period_end))
            .filter(i_dsl::consolidated_into_invoice_id.is_not_null())
            .select(i_dsl::consolidated_into_invoice_id.assume_not_null())
            .distinct()
            .load(conn)
            .await
            .attach("Error while listing consolidated parents of a period")
            .into_db_result()?;
        if !parents.is_empty() {
            invoices.extend(
                i_dsl::invoice
                    .filter(i_dsl::tenant_id.eq(param_tenant_id))
                    .filter(i_dsl::id.eq_any(&parents))
                    .filter(i_dsl::status.eq(InvoiceStatusEnum::Finalized))
                    .select(InvoiceRow::as_select())
                    .load(conn)
                    .await
                    .attach("Error while loading consolidated parents of a period")
                    .into_db_result()?,
            );
        }
        Ok(invoices)
    }
}

impl InvoiceRow {
    /// Overwrites the frozen annex snapshot of a draft. Status-filtered: an issued document's
    /// appendix must never move.
    pub async fn update_annexes(
        conn: &mut PgConn,
        id: InvoiceId,
        tenant_id: TenantId,
        annexes: serde_json::Value,
    ) -> DbResult<usize> {
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        diesel::update(i_dsl::invoice)
            .filter(
                i_dsl::id
                    .eq(id)
                    .and(i_dsl::tenant_id.eq(tenant_id))
                    .and(i_dsl::status.eq(InvoiceStatusEnum::Draft)),
            )
            .set(i_dsl::annexes.eq(annexes))
            .execute(conn)
            .await
            .attach("Error while updating invoice annexes")
            .into_db_result()
    }
}

impl crate::invoices::InvoiceRowIdentityPatch {
    /// Freezes the resolved customer/seller identity at finalization. Deliberately unfiltered on
    /// status: the caller runs inside the finalize transaction, on the locked row.
    pub async fn update_identity(
        &self,
        conn: &mut PgConn,
        id: InvoiceId,
        tenant_id: TenantId,
    ) -> DbResult<usize> {
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(i_dsl::invoice)
            .filter(i_dsl::id.eq(id).and(i_dsl::tenant_id.eq(tenant_id)))
            .set(self);

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while freezing invoice identity")
            .into_db_result()
    }
}

impl InvoiceRowLinesPatch {
    /// Stamps `data_updated_at` alongside the lines: it is what the background sweep measures
    /// staleness from, and what the invoice view shows as "last refreshed". Without it the
    /// column only ever moves at finalization, so a refreshed draft still reads as stale.
    pub async fn update_lines(
        &self,
        id: InvoiceId,
        tenant_id: TenantId,
        conn: &mut PgConn,
    ) -> DbResult<usize> {
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        let now = common_domain::clock::now().naive_utc();

        let query = diesel::update(i_dsl::invoice)
            .filter(i_dsl::id.eq(id).and(i_dsl::tenant_id.eq(tenant_id)))
            .set((self, i_dsl::data_updated_at.eq(now)));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while updating invoice lines")
            .into_db_result()
    }
}

/// Eligibility predicate for [`InvoiceRow::list_refreshable_drafts`], split out so the SQL it
/// produces can be pinned by a test without a connection.
///
/// `now` is bound as a cutoff rather than compared against `CURRENT_TIMESTAMP`: `data_updated_at`
/// is a naive timestamp, so a timestamptz comparison would resolve through the session TimeZone.
/// Passing the same UTC clock `update_lines` stamps with keeps both sides in one frame.
fn refreshable_drafts_filter(
    stale_after_hours: i32,
    horizon_days: i64,
    now: NaiveDateTime,
) -> crate::schema::invoice::BoxedQuery<'static, diesel::pg::Pg> {
    use crate::schema::invoice::dsl as i_dsl;
    use crate::schema::payment_transaction::dsl as pt_dsl;

    // Uncorrelated: a correlated EXISTS cannot name an outer column here, since the subquery's
    // only query source is `payment_transaction`. NULL `invoice_id`s are excluded because
    // `<> ALL` over a set containing NULL is never true.
    let invoices_with_payment_in_flight = pt_dsl::payment_transaction
        .filter(pt_dsl::invoice_id.is_not_null())
        .filter(pt_dsl::payment_type.eq_any(crate::enums::PaymentTypeEnum::COLLECTIONS))
        .filter(pt_dsl::status.eq_any([
            crate::enums::PaymentStatusEnum::Pending,
            crate::enums::PaymentStatusEnum::Ready,
        ]))
        .select(pt_dsl::invoice_id);

    i_dsl::invoice
        .filter(i_dsl::status.eq(InvoiceStatusEnum::Draft))
        .filter(i_dsl::invoice_type.eq_any([
            crate::enums::InvoiceType::Recurring,
            crate::enums::InvoiceType::Final,
        ]))
        .filter(i_dsl::subscription_id.is_not_null())
        .filter(i_dsl::consolidated_into_invoice_id.is_null())
        .filter(i_dsl::manually_edited_at.is_null())
        .filter(i_dsl::id.nullable().ne_all(invoices_with_payment_in_flight))
        .filter(
            i_dsl::data_updated_at.is_null().or(i_dsl::data_updated_at
                .assume_not_null()
                .lt(now - chrono::Duration::hours(stale_after_hours.into()))),
        )
        // Both bounds key on `billing_period_start`, the period this invoice settles and the same
        // anchor `schedule_invoice_finalization` gives the grace deadline. `invoice_date` became
        // the emission date at finalization, so it no longer describes the period being waited on.
        .filter(
            i_dsl::billing_period_start
                .gt((now - chrono::Duration::days(horizon_days)).date())
                // Scalar subquery rather than a join, so the query source stays `invoice` alone.
                // `invoicing_entity_id` is NOT NULL, but COALESCE keeps a 0-hour grace explicit.
                .or(diesel::dsl::sql::<diesel::sql_types::Bool>(
                    "\"invoice\".\"billing_period_start\" + (COALESCE((SELECT \"grace_period_hours\" \
                     FROM \"invoicing_entity\" WHERE \"invoicing_entity\".\"id\" = \"invoice\".\"invoicing_entity_id\"), 0) \
                     * INTERVAL '1 hour') > ",
                )
                .bind::<diesel::sql_types::Timestamp, _>(now)),
        )
        .into_boxed()
}

impl crate::invoices::InvoiceRowPatch {
    pub async fn update(
        &self,
        id: InvoiceId,
        tenant_id: TenantId,
        conn: &mut PgConn,
    ) -> DbResult<usize> {
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(i_dsl::invoice)
            .filter(i_dsl::id.eq(id).and(i_dsl::tenant_id.eq(tenant_id)))
            .filter(i_dsl::status.eq(crate::enums::InvoiceStatusEnum::Draft))
            .set(self);

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while updating draft invoice")
            .into_db_result()
    }
}

#[cfg(test)]
mod refreshable_drafts_tests {
    use diesel::debug_query;

    fn where_clause() -> String {
        let now = chrono::NaiveDate::from_ymd_opt(2026, 8, 25)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap();
        let sql = debug_query::<diesel::pg::Pg, _>(&super::refreshable_drafts_filter(1, 7, now))
            .to_string();
        let (_, predicate) = sql.split_once("WHERE ").expect("query has a WHERE clause");
        predicate.to_string()
    }

    /// The predicate this replaced gated on the invoice's own date, so every draft older than
    /// the interval matched forever and the sweep never converged. Staleness must be measured
    /// from the last refresh.
    /// Staleness gates on `data_updated_at`; `billing_period_start` carries the separate horizon
    /// bound, so both columns appear — what matters is which sits next to `data_updated_at`.
    #[test]
    fn staleness_is_measured_from_the_last_refresh() {
        let sql = where_clause();
        assert!(
            sql.contains(
                r#"("invoice"."data_updated_at" IS NULL) OR ("invoice"."data_updated_at" <"#
            ),
            "staleness must gate on the last refresh: {sql}"
        );
    }

    /// A draft nobody is waiting on any more should stop being recomputed rather than
    /// re-querying metering hourly forever.
    /// `invoice_date` is the emission date once finalized, so it cannot anchor a bound about the
    /// period being waited on — both time bounds key on the period, as the grace deadline does.
    #[test]
    fn time_bounds_key_on_the_period_not_the_emission_date() {
        let sql = where_clause();
        assert!(
            !sql.contains(r#""invoice"."invoice_date""#),
            "time bounds must not key on the emission date: {sql}"
        );
    }

    #[test]
    fn drafts_past_the_horizon_are_dropped() {
        let sql = where_clause();
        assert!(
            sql.contains(r#""invoice"."billing_period_start" > "#),
            "missing horizon bound on billing_period_start: {sql}"
        );
    }

    /// The horizon is an ingestion allowance, the same axis grace sits on, so it must not cut in
    /// while the entity's own grace window is still open and late usage is still expected.
    #[test]
    fn an_open_grace_window_keeps_a_draft_eligible_past_the_horizon() {
        let sql = where_clause();
        assert!(
            sql.contains("grace_period_hours") && sql.contains("INTERVAL '1 hour'"),
            "horizon must not cut in before grace: {sql}"
        );
    }

    /// A `CURRENT_TIMESTAMP` comparison against this naive column would resolve through the
    /// session TimeZone; the cutoff is bound instead.
    #[test]
    fn the_cutoff_is_bound_not_taken_from_the_session_clock() {
        let sql = where_clause();
        assert!(!sql.contains("CURRENT_TIMESTAMP"), "{sql}");
    }

    #[test]
    fn only_untouched_recurring_subscription_drafts_are_eligible() {
        let sql = where_clause();
        for clause in [
            "\"status\" =",
            "\"invoice_type\" =",
            "\"subscription_id\" IS NOT NULL",
            "\"consolidated_into_invoice_id\" IS NULL",
            "\"manually_edited_at\" IS NULL",
        ] {
            assert!(sql.contains(clause), "missing {clause} in: {sql}");
        }
    }

    /// Refreshing under a live intent moves `amount_due` away from the amount the provider is
    /// already capturing, which `FinalizeAfterPayment` asserts on.
    #[test]
    fn drafts_with_a_payment_in_flight_are_excluded() {
        let sql = where_clause();
        assert!(sql.contains("!= ALL"), "{sql}");
        assert!(sql.contains("payment_transaction"), "{sql}");
    }
}
