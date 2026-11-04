use crate::domain::custom_property_definitions::CustomPropertyMode;
use crate::domain::einvoicing::EInvoicingOutcome;
use crate::domain::entity_activity::Actor;
use crate::domain::enums::{CustomPropertyEntityType, InvoiceType, StoredObjectKindEnum};
use crate::domain::outbox_event::{InvoicePdfGeneratedEvent, OutboxEvent};
use crate::domain::pgmq::{
    PennylaneSyncCreditNote, PennylaneSyncInvoice, PennylaneSyncRequestEvent, PgmqQueue,
};
use crate::domain::stored_object::StoredObjectRef;
use crate::domain::{
    ConnectorProviderEnum, CursorPaginatedVec, CursorPaginationRequest, DetailedInvoice,
    EmissionProjectionContext, Invoice, InvoiceNew, InvoiceWithCustomer, LineItem, PaginatedVec,
    PaginationRequest, TaxBreakdownItem, TaxDetail,
};
use crate::errors::StoreError;
use crate::repositories::connectors::ConnectorsInterface;
use crate::repositories::credit_notes::{
    CreateCreditNoteTxParams, CreditType, create_credit_note_tx,
};
use crate::repositories::pgmq::PgmqInterface;
use crate::services::invoices::bill::CONSOLIDATION_MIN_GRACE_HOURS;
use crate::store::Store;
use crate::{StoreResult, domain};
use common_domain::ids::{
    AliasOr, BaseId, ConnectorId, CreditNoteId, CustomerId, EventId, InvoiceId, InvoicingEntityId,
    SubscriptionId, TenantId,
};
use diesel_models::PgConn;
use diesel_models::credit_notes::CreditNoteRow;
use diesel_models::customers::CustomerRow;
use diesel_models::enums::{MrrMovementType, SubscriptionEventType, SubscriptionStatusEnum};
use diesel_models::extend::connection_metadata;
use diesel_models::invoices::{InvoiceRow, InvoiceRowNew};
use diesel_models::stored_object::StoredObjectRowNew;
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::{Report, bail};
use itertools::Itertools;
use meteroid_tax::{LineToTax, TaxToApply, apply_taxes};
use ordermap::OrderMap;
use rust_decimal::Decimal;
use scoped_futures::ScopedFutureExt;
use std::collections::HashMap;
use tracing_log::log;
use uuid::Uuid;

/// Shared body for the paginated invoice listings. `issued_only` excludes Draft/Closed in SQL so
/// pagination is correct; see [`InvoiceInterface::list_issued_invoices`].
#[allow(clippy::too_many_arguments)]
async fn list_invoices_paginated(
    store: &Store,
    tenant_id: TenantId,
    customer_id: Option<CustomerId>,
    subscription_id: Option<SubscriptionId>,
    status: Option<domain::enums::InvoiceStatusEnum>,
    payment_status: Option<domain::enums::InvoicePaymentStatus>,
    einvoicing_status: Option<domain::einvoicing::EInvoicingStatus>,
    overdue: bool,
    issued_only: bool,
    query: Option<String>,
    filters: Vec<domain::filters::Filter>,
    order_by: Option<String>,
    pagination: PaginationRequest,
) -> StoreResult<PaginatedVec<InvoiceWithCustomer>> {
    let mut conn = store.get_conn().await?;

    let cp_filters = if filters.is_empty() {
        vec![]
    } else {
        let definitions = store
            .internal
            .custom_property_definitions
            .load_active(
                &mut conn,
                tenant_id,
                domain::enums::CustomPropertyEntityType::Invoice,
            )
            .await?;
        crate::repositories::filter_resolution::resolve_list_filters(
            crate::repositories::filter_resolution::ListEntity::Invoice,
            definitions.as_slice(),
            filters,
        )?
    };

    let rows = InvoiceRow::list(
        &mut conn,
        tenant_id,
        customer_id,
        subscription_id,
        status.map(Into::into),
        payment_status.map(Into::into),
        einvoicing_status.map(Into::into),
        overdue.then(|| chrono::Utc::now().naive_utc()),
        issued_only,
        query,
        cp_filters,
        order_by.as_deref(),
        pagination.into(),
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;

    Ok(PaginatedVec {
        items: rows
            .items
            .into_iter()
            .map(std::convert::TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()?,
        total_pages: rows.total_pages,
        total_results: rows.total_results,
    })
}

#[async_trait::async_trait]
pub trait InvoiceInterface {
    async fn get_detailed_invoice_by_id(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<DetailedInvoice>;

    async fn get_detailed_invoice_by_id_with_conn(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<DetailedInvoice>;

    async fn get_invoice_by_id(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<Invoice>;

    /// Per-subscription inputs for projecting a draft's emission/due dates, keyed by subscription
    /// id. Subscriptions not returned (or invoices without one) fall back to a today projection.
    async fn subscriptions_emission_projection_context(
        &self,
        subscription_ids: &[SubscriptionId],
    ) -> StoreResult<HashMap<SubscriptionId, EmissionProjectionContext>>;

    /// Projects a single draft's emission/due dates in place (see
    /// [`Invoice::project_emission_dates`]), resolving its subscription's context. Returns whether
    /// it actually projected. No-op (false) on non-draft, manual, backdated, or merged-child
    /// invoices — a merged child is finalized by its consolidated parent, not on its own schedule.
    async fn project_draft_emission_dates(&self, invoice: &mut Invoice) -> StoreResult<bool>;

    /// Merges a custom-property delta onto a draft invoice (validated against the tenant's
    /// `INVOICE` property definitions). Fails if the invoice is not an editable draft.
    async fn patch_invoice_custom_properties(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        custom_properties: serde_json::Value,
    ) -> StoreResult<Invoice>;

    async fn find_child_invoice_id(
        &self,
        tenant_id: TenantId,
        parent_invoice_id: InvoiceId,
    ) -> StoreResult<Option<InvoiceId>>;

    /// Batch form of [`Self::find_child_invoice_id`], keyed by parent invoice id. Parents
    /// without a correction are absent from the map.
    async fn find_child_invoice_ids(
        &self,
        tenant_id: TenantId,
        parent_invoice_ids: &[InvoiceId],
    ) -> StoreResult<HashMap<InvoiceId, InvoiceId>>;

    /// Lists the per-subscription child invoices merged into the given consolidated parent.
    async fn list_consolidated_children(
        &self,
        tenant_id: TenantId,
        parent_invoice_id: InvoiceId,
    ) -> StoreResult<Vec<Invoice>>;

    /// `overdue` keeps the invoices still owed past their due date, as receivables count them.
    #[allow(clippy::too_many_arguments)]
    async fn list_invoices(
        &self,
        tenant_id: TenantId,
        customer_id: Option<CustomerId>,
        subscription_id: Option<SubscriptionId>,
        status: Option<domain::enums::InvoiceStatusEnum>,
        payment_status: Option<domain::enums::InvoicePaymentStatus>,
        einvoicing_status: Option<domain::einvoicing::EInvoicingStatus>,
        overdue: bool,
        query: Option<String>,
        filters: Vec<domain::filters::Filter>,
        order_by: Option<String>,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<InvoiceWithCustomer>>;

    /// Like [`InvoiceInterface::list_invoices`] but returns only *issued* invoices — Draft and
    /// Closed are excluded in SQL (via `finalized_at`), so pagination stays correct. Use for
    /// customer-facing listings where non-issued invoices must never surface.
    #[allow(clippy::too_many_arguments)]
    async fn list_issued_invoices(
        &self,
        tenant_id: TenantId,
        customer_id: Option<CustomerId>,
        subscription_id: Option<SubscriptionId>,
        status: Option<domain::enums::InvoiceStatusEnum>,
        payment_status: Option<domain::enums::InvoicePaymentStatus>,
        einvoicing_status: Option<domain::einvoicing::EInvoicingStatus>,
        query: Option<String>,
        order_by: Option<String>,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<InvoiceWithCustomer>>;

    async fn list_full_invoices(
        &self,
        tenant_id: TenantId,
        customer_id: Option<AliasOr<CustomerId>>,
        subscription_id: Option<SubscriptionId>,
        statuses: Option<Vec<domain::enums::InvoiceStatusEnum>>,
        einvoicing_status: Option<domain::einvoicing::EInvoicingStatus>,
        query: Option<String>,
        order_by: Option<String>,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<(InvoiceWithCustomer, Vec<domain::PaymentTransaction>)>>;

    async fn insert_invoice(&self, invoice: InvoiceNew) -> StoreResult<Invoice>;

    async fn insert_invoice_batch(&self, invoice: Vec<InvoiceNew>) -> StoreResult<Vec<Invoice>>;

    async fn list_refreshable_draft_invoices(
        &self,
        stale_after_hours: i32,
        horizon_days: i64,
        pagination: CursorPaginationRequest,
    ) -> StoreResult<CursorPaginatedVec<Invoice>>;

    async fn list_invoices_by_ids(&self, ids: Vec<InvoiceId>) -> StoreResult<Vec<Invoice>>;

    async fn list_detailed_invoices_by_ids(
        &self,
        ids: Vec<InvoiceId>,
    ) -> StoreResult<Vec<DetailedInvoice>>;

    /// A cursor-paginated page of the tax report's invoices, as domain `Invoice`s in
    /// `(invoice_date, id)` order. Backs the streaming export worker and the tax reports;
    /// the caller derives the next cursor from the last returned invoice's `(invoice_date, id)`.
    async fn list_invoices_for_tax_report_page(
        &self,
        tenant_id: TenantId,
        period_start: chrono::NaiveDate,
        period_end: chrono::NaiveDate,
        invoicing_entity_id: Option<InvoicingEntityId>,
        after: Option<(chrono::NaiveDate, InvoiceId)>,
        limit: i64,
    ) -> StoreResult<Vec<Invoice>>;

    async fn save_invoice_documents(
        &self,
        id: InvoiceId,
        tenant_id: TenantId,
        customer_id: CustomerId,
        pdf: StoredObjectRef,
        xml: Option<StoredObjectRef>,
        einvoicing: Option<EInvoicingOutcome>,
    ) -> StoreResult<()>;

    async fn sync_invoices_to_pennylane(
        &self,
        ids: Vec<InvoiceId>,
        tenant_id: TenantId,
    ) -> StoreResult<()>;

    /// Sibling of [`InvoiceInterface::sync_invoices_to_pennylane`]; the worker imports the
    /// credited invoice first, so both live on the same interface.
    async fn sync_credit_notes_to_pennylane(
        &self,
        ids: Vec<CreditNoteId>,
        tenant_id: TenantId,
    ) -> StoreResult<()>;

    async fn patch_invoice_conn_meta(
        &self,
        invoice_id: InvoiceId,
        connector_id: ConnectorId,
        provider: ConnectorProviderEnum,
        external_id: &str,
        external_company_id: &str,
    ) -> StoreResult<()>;

    async fn patch_credit_note_conn_meta(
        &self,
        credit_note_id: CreditNoteId,
        connector_id: ConnectorId,
        provider: ConnectorProviderEnum,
        external_id: &str,
        external_company_id: &str,
    ) -> StoreResult<()>;

    async fn delete_invoice(
        &self,
        actor: Actor,
        id: InvoiceId,
        tenant_id: TenantId,
    ) -> StoreResult<()>;

    async fn void_invoice(
        &self,
        actor: Actor,
        id: InvoiceId,
        tenant_id: TenantId,
    ) -> StoreResult<DetailedInvoice>;

    /// `void_invoice` at the business moment `now`: credit given back lapses if due by then.
    async fn void_invoice_at(
        &self,
        actor: Actor,
        id: InvoiceId,
        tenant_id: TenantId,
        now: chrono::DateTime<chrono::Utc>,
    ) -> StoreResult<DetailedInvoice>;

    async fn mark_invoice_as_uncollectible(
        &self,
        id: InvoiceId,
        tenant_id: TenantId,
    ) -> StoreResult<DetailedInvoice>;
}

#[async_trait::async_trait]
impl InvoiceInterface for Store {
    async fn get_detailed_invoice_by_id(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<DetailedInvoice> {
        let mut conn = self.get_conn().await?;

        self.get_detailed_invoice_by_id_with_conn(&mut conn, tenant_id, invoice_id)
            .await
    }

    async fn get_detailed_invoice_by_id_with_conn(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<DetailedInvoice> {
        InvoiceRow::find_detailed_by_id(conn, tenant_id, invoice_id)
            .await
            .map_err(Into::into)
            .and_then(std::convert::TryInto::try_into)
    }

    async fn get_invoice_by_id(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<Invoice> {
        let mut conn = self.get_conn().await?;

        InvoiceRow::find_by_id(&mut conn, tenant_id, invoice_id)
            .await
            .map_err(Into::into)
            .and_then(std::convert::TryInto::try_into)
    }

    async fn subscriptions_emission_projection_context(
        &self,
        subscription_ids: &[SubscriptionId],
    ) -> StoreResult<HashMap<SubscriptionId, EmissionProjectionContext>> {
        if subscription_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let mut conn = self.get_conn().await?;

        let rows =
            InvoiceRow::get_subscriptions_projection_context(&mut conn, subscription_ids).await?;

        Ok(rows
            .into_iter()
            .map(
                |(id, backdated, auto_advance, grace_period_hours, consolidate, terminated_at)| {
                    // Consolidating entities floor the effective grace, so mirror that here or the
                    // projected issue date can land a day before the real finalization.
                    let grace_period_hours = if consolidate {
                        grace_period_hours.max(CONSOLIDATION_MIN_GRACE_HOURS)
                    } else {
                        grace_period_hours
                    };
                    (
                        id,
                        EmissionProjectionContext {
                            backdated,
                            auto_advance,
                            grace_period_hours,
                            terminated_at,
                        },
                    )
                },
            )
            .collect())
    }

    async fn project_draft_emission_dates(&self, invoice: &mut Invoice) -> StoreResult<bool> {
        // A merged child is finalized by its consolidated parent, not on its own grace schedule,
        // so its own dates are never projected.
        if invoice.consolidated_into_invoice_id.is_some() {
            return Ok(false);
        }
        if invoice.can_edit() && !invoice.manual {
            let ctx = match invoice.subscription_id {
                Some(id) => self
                    .subscriptions_emission_projection_context(&[id])
                    .await?
                    .get(&id)
                    .copied(),
                None => None,
            };
            Ok(invoice.project_emission_dates_with(ctx))
        } else {
            Ok(false)
        }
    }

    async fn patch_invoice_custom_properties(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        custom_properties: serde_json::Value,
    ) -> StoreResult<Invoice> {
        let mut conn = self.get_conn().await?;

        let existing = InvoiceRow::find_by_id(&mut conn, tenant_id, invoice_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let normalized = self
            .internal
            .validate_and_normalize_custom_properties(
                &mut conn,
                tenant_id,
                crate::domain::enums::CustomPropertyEntityType::Invoice,
                &custom_properties,
                &existing.custom_properties,
                crate::domain::custom_property_definitions::CustomPropertyMode::Merge,
            )
            .await?;

        InvoiceRow::update_custom_properties(&mut conn, tenant_id, invoice_id, normalized)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .ok_or_else(|| StoreError::InvalidArgument("Invoice not found".to_string()).into())
            .and_then(std::convert::TryInto::try_into)
    }

    async fn find_child_invoice_id(
        &self,
        tenant_id: TenantId,
        parent_invoice_id: InvoiceId,
    ) -> StoreResult<Option<InvoiceId>> {
        let mut conn = self.get_conn().await?;

        InvoiceRow::find_child_id_by_parent(&mut conn, tenant_id, parent_invoice_id)
            .await
            .map_err(Into::into)
    }

    async fn find_child_invoice_ids(
        &self,
        tenant_id: TenantId,
        parent_invoice_ids: &[InvoiceId],
    ) -> StoreResult<HashMap<InvoiceId, InvoiceId>> {
        let mut conn = self.get_conn().await?;

        Ok(
            InvoiceRow::find_child_ids_by_parents(&mut conn, tenant_id, parent_invoice_ids)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .collect(),
        )
    }

    async fn list_consolidated_children(
        &self,
        tenant_id: TenantId,
        parent_invoice_id: InvoiceId,
    ) -> StoreResult<Vec<Invoice>> {
        let mut conn = self.get_conn().await?;

        InvoiceRow::list_consolidated_children(&mut conn, tenant_id, parent_invoice_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(TryInto::try_into)
            .collect()
    }

    async fn list_invoices(
        &self,
        tenant_id: TenantId,
        customer_id: Option<CustomerId>,
        subscription_id: Option<SubscriptionId>,
        status: Option<domain::enums::InvoiceStatusEnum>,
        payment_status: Option<domain::enums::InvoicePaymentStatus>,
        einvoicing_status: Option<domain::einvoicing::EInvoicingStatus>,
        overdue: bool,
        query: Option<String>,
        filters: Vec<domain::filters::Filter>,
        order_by: Option<String>,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<InvoiceWithCustomer>> {
        list_invoices_paginated(
            self,
            tenant_id,
            customer_id,
            subscription_id,
            status,
            payment_status,
            einvoicing_status,
            overdue,
            false,
            query,
            filters,
            order_by,
            pagination,
        )
        .await
    }

    async fn list_issued_invoices(
        &self,
        tenant_id: TenantId,
        customer_id: Option<CustomerId>,
        subscription_id: Option<SubscriptionId>,
        status: Option<domain::enums::InvoiceStatusEnum>,
        payment_status: Option<domain::enums::InvoicePaymentStatus>,
        einvoicing_status: Option<domain::einvoicing::EInvoicingStatus>,
        query: Option<String>,
        order_by: Option<String>,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<InvoiceWithCustomer>> {
        list_invoices_paginated(
            self,
            tenant_id,
            customer_id,
            subscription_id,
            status,
            payment_status,
            einvoicing_status,
            false,
            true,
            query,
            vec![],
            order_by,
            pagination,
        )
        .await
    }

    async fn list_full_invoices(
        &self,
        tenant_id: TenantId,
        customer_id: Option<AliasOr<CustomerId>>,
        subscription_id: Option<SubscriptionId>,
        statuses: Option<Vec<domain::enums::InvoiceStatusEnum>>,
        einvoicing_status: Option<domain::einvoicing::EInvoicingStatus>,
        query: Option<String>,
        order_by: Option<String>,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<(InvoiceWithCustomer, Vec<domain::PaymentTransaction>)>> {
        let mut conn = self.get_conn().await?;

        let rows = InvoiceRow::list_with_transactions(
            &mut conn,
            tenant_id,
            customer_id,
            subscription_id,
            statuses.map(|s| s.into_iter().map(Into::into).collect()),
            einvoicing_status.map(Into::into),
            query,
            order_by.as_deref(),
            pagination.into(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let items: Vec<(InvoiceWithCustomer, Vec<domain::PaymentTransaction>)> = rows
            .items
            .into_iter()
            .map(|(invoice_row, txs)| {
                let invoice: InvoiceWithCustomer = invoice_row.try_into()?;
                let transactions: Vec<domain::PaymentTransaction> =
                    txs.into_iter().map(Into::into).collect();
                Ok((invoice, transactions))
            })
            .collect::<Result<Vec<_>, Report<StoreError>>>()?;

        Ok(PaginatedVec {
            total_results: rows.total_results,
            total_pages: rows.total_pages,
            items,
        })
    }

    async fn insert_invoice(&self, invoice: InvoiceNew) -> StoreResult<Invoice> {
        self.transaction(|conn| {
            async move {
                let lock = CustomerRow::lock(conn, invoice.tenant_id, invoice.customer_id).await?;
                diesel_models::query::credit_inputs::bump_customer(conn, &lock).await?;
                insert_invoice_tx(self, conn, invoice).await
            }
            .scope_boxed()
        })
        .await
    }

    async fn insert_invoice_batch(&self, invoice: Vec<InvoiceNew>) -> StoreResult<Vec<Invoice>> {
        self.transaction(|conn| {
            async move {
                let by_tenant: std::collections::BTreeMap<_, Vec<CustomerId>> = invoice
                    .iter()
                    .map(|invoice| (invoice.tenant_id.as_uuid(), invoice.customer_id))
                    .into_group_map()
                    .into_iter()
                    .collect();
                let mut locks = Vec::new();
                for (tenant_id, customers) in by_tenant {
                    let tenant_id = TenantId::from(tenant_id);
                    locks.extend(CustomerRow::lock_many(conn, tenant_id, &customers).await?);
                }
                diesel_models::query::credit_inputs::bump_customers(
                    conn,
                    &locks.iter().collect::<Vec<_>>(),
                )
                .await?;
                insert_invoice_batch_tx(self, conn, invoice).await
            }
            .scope_boxed()
        })
        .await
    }

    async fn list_refreshable_draft_invoices(
        &self,
        stale_after_hours: i32,
        horizon_days: i64,
        pagination: CursorPaginationRequest,
    ) -> StoreResult<CursorPaginatedVec<Invoice>> {
        let mut conn = self.get_conn().await?;

        let invoices = InvoiceRow::list_refreshable_drafts(
            &mut conn,
            stale_after_hours,
            horizon_days,
            pagination.into(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let res: CursorPaginatedVec<Invoice> = CursorPaginatedVec {
            items: invoices
                .items
                .into_iter()
                .map(std::convert::TryInto::try_into)
                .collect::<Result<Vec<_>, _>>()?,
            next_cursor: invoices.next_cursor,
        };

        Ok(res)
    }

    async fn list_invoices_by_ids(&self, ids: Vec<InvoiceId>) -> StoreResult<Vec<Invoice>> {
        let mut conn = self.get_conn().await?;

        let invoices = InvoiceRow::list_by_ids(&mut conn, ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        invoices
            .into_iter()
            .map(std::convert::TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()
    }

    async fn list_detailed_invoices_by_ids(
        &self,
        ids: Vec<InvoiceId>,
    ) -> StoreResult<Vec<DetailedInvoice>> {
        let mut conn = self.get_conn().await?;

        let invoices = InvoiceRow::list_detailed_by_ids(&mut conn, ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        invoices
            .into_iter()
            .map(std::convert::TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()
    }

    async fn list_invoices_for_tax_report_page(
        &self,
        tenant_id: TenantId,
        period_start: chrono::NaiveDate,
        period_end: chrono::NaiveDate,
        invoicing_entity_id: Option<InvoicingEntityId>,
        after: Option<(chrono::NaiveDate, InvoiceId)>,
        limit: i64,
    ) -> StoreResult<Vec<Invoice>> {
        let mut conn = self.get_conn().await?;

        let rows = InvoiceRow::list_for_tax_report_page(
            &mut conn,
            tenant_id,
            period_start,
            period_end,
            invoicing_entity_id,
            after,
            limit,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        rows.into_iter()
            .map(std::convert::TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()
    }

    async fn save_invoice_documents(
        &self,
        id: InvoiceId,
        tenant_id: TenantId,
        customer_id: CustomerId,
        pdf: StoredObjectRef,
        xml: Option<StoredObjectRef>,
        einvoicing: Option<EInvoicingOutcome>,
    ) -> StoreResult<()> {
        self.transaction(|conn| {
            async move {
                let pdf_id = pdf.id;
                let xml_id = xml.as_ref().map(|o| o.id);
                let (status, report) = match &einvoicing {
                    Some(outcome) => (
                        Some(outcome.status.into()),
                        Some(serde_json::to_value(&outcome.report).map_err(|e| {
                            StoreError::SerdeError(
                                "Failed to serialize einvoicing_report".to_string(),
                                e,
                            )
                        })?),
                    ),
                    None => (None, None),
                };

                InvoiceRow::save_invoice_documents(
                    conn, id, tenant_id, pdf_id, xml_id, status, report,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                // Record the object-store documents in the same transaction as the reference.
                let mut docs = vec![StoredObjectRowNew {
                    id: pdf.id,
                    tenant_id,
                    kind: StoredObjectKindEnum::InvoicePdf.into(),
                    key: pdf.key,
                }];
                if let Some(xml) = xml {
                    docs.push(StoredObjectRowNew {
                        id: xml.id,
                        tenant_id,
                        kind: StoredObjectKindEnum::InvoiceXml.into(),
                        key: xml.key,
                    });
                }
                StoredObjectRowNew::insert_batch(conn, &docs)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                let event = OutboxEvent::invoice_pdf_generated(InvoicePdfGeneratedEvent {
                    id: EventId::new(),
                    invoice_id: id,
                    tenant_id,
                    customer_id,
                    pdf_id,
                    xml_id,
                    einvoicing_status: einvoicing.as_ref().map(|o| o.status),
                    einvoicing_report: einvoicing.map(|o| o.report),
                });
                self.internal
                    .publish_outbox_events_topic_tx(conn, vec![event])
                    .await?;

                Ok(())
            }
            .scope_boxed()
        })
        .await?;

        Ok(())
    }

    async fn sync_invoices_to_pennylane(
        &self,
        ids: Vec<InvoiceId>,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        let connector = self.get_pennylane_connector(tenant_id).await?;

        if connector.is_none() {
            bail!(StoreError::InvalidArgument(
                "No Pennylane connector found".to_string()
            ));
        }

        let mut conn = self.get_conn().await?;

        let invoices = InvoiceRow::list_by_ids(&mut conn, ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        self.pgmq_send_batch(
            PgmqQueue::PennylaneSync,
            invoices
                .into_iter()
                .map(|invoice| {
                    PennylaneSyncRequestEvent::Invoice(Box::new(PennylaneSyncInvoice {
                        id: invoice.id,
                        tenant_id,
                    }))
                    .try_into()
                })
                .collect::<Result<Vec<_>, _>>()?,
        )
        .await
    }

    async fn sync_credit_notes_to_pennylane(
        &self,
        ids: Vec<CreditNoteId>,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        let connector = self.get_pennylane_connector(tenant_id).await?;

        if connector.is_none() {
            bail!(StoreError::InvalidArgument(
                "No Pennylane connector found".to_string()
            ));
        }

        let mut conn = self.get_conn().await?;

        let credit_notes = CreditNoteRow::list_by_ids(&mut conn, ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        self.pgmq_send_batch(
            PgmqQueue::PennylaneSync,
            credit_notes
                .into_iter()
                .filter(|credit_note| credit_note.tenant_id == tenant_id)
                .map(|credit_note| {
                    PennylaneSyncRequestEvent::CreditNote(Box::new(PennylaneSyncCreditNote {
                        id: credit_note.id,
                        tenant_id,
                    }))
                    .try_into()
                })
                .collect::<Result<Vec<_>, _>>()?,
        )
        .await
    }

    async fn patch_invoice_conn_meta(
        &self,
        invoice_id: InvoiceId,
        connector_id: ConnectorId,
        provider: ConnectorProviderEnum,
        external_id: &str,
        external_company_id: &str,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        InvoiceRow::upsert_conn_meta(
            &mut conn,
            provider.into(),
            invoice_id,
            connector_id,
            external_id,
            external_company_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)
    }

    async fn patch_credit_note_conn_meta(
        &self,
        credit_note_id: CreditNoteId,
        connector_id: ConnectorId,
        provider: ConnectorProviderEnum,
        external_id: &str,
        external_company_id: &str,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        let provider: diesel_models::enums::ConnectorProviderEnum = provider.into();

        connection_metadata::upsert(
            &mut conn,
            "credit_note",
            provider.as_meta_key(),
            credit_note_id.as_uuid(),
            connector_id,
            external_id,
            external_company_id,
        )
        .await
        .map(|_| ())
        .map_err(Into::<Report<StoreError>>::into)
    }

    async fn delete_invoice(
        &self,
        actor: Actor,
        id: InvoiceId,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        self.transaction(|conn| {
            let actor = &actor;
            async move {
                let lock = InvoiceRow::lock_customer_of(conn, tenant_id, id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                diesel_models::query::credit_inputs::bump_customer(conn, &lock).await?;
                // Deleting a consolidated child would sever its parent's audit trail.
                let invoice: Invoice = InvoiceRow::find_by_id(conn, tenant_id, id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)
                    .and_then(TryInto::try_into)?;
                invoice.ensure_not_consolidated_child("delete")?;

                // The SQL delete filters on Draft, so anything else would report success
                // while deleting nothing — and emit an event for a still-live invoice.
                if invoice.status != domain::enums::InvoiceStatusEnum::Draft {
                    bail!(StoreError::InvalidArgument(
                        "Only draft invoices can be deleted".to_string()
                    ));
                }

                // Snapshot before the row is gone — the event payload is the only record left.
                let event = domain::outbox_event::OutboxEvent::invoice_deleted((&invoice).into());

                InvoiceRow::delete(conn, id, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                self.internal
                    .record_outbox_batch_tx(conn, tenant_id, actor, vec![event])
                    .await
            }
            .scope_boxed()
        })
        .await
    }

    async fn void_invoice(
        &self,
        actor: Actor,
        id: InvoiceId,
        tenant_id: TenantId,
    ) -> StoreResult<DetailedInvoice> {
        self.void_invoice_at(actor, id, tenant_id, chrono::Utc::now())
            .await
    }

    async fn void_invoice_at(
        &self,
        actor: Actor,
        id: InvoiceId,
        tenant_id: TenantId,
        now: chrono::DateTime<chrono::Utc>,
    ) -> StoreResult<DetailedInvoice> {
        // Usage that credit given back already expired paid (step 4b) is read before the locks.
        let mut passes = crate::services::served_usage::UsagePasses::default();
        loop {
            let usage = passes.next_pass()?;
            let voided = self.transaction(|conn| {
            let actor = &actor;
            let usage = &usage;
            async move {
                // 1. Lock and validate the invoice; the lock serializes the credit it returns.
                let locked = InvoiceRow::select_for_update_by_id(conn, tenant_id, id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                let lock = &locked.lock;
                diesel_models::query::credit_inputs::bump_customer(conn, lock).await?;
                let invoice: Invoice = locked.invoice.try_into()?;

                // A consolidated child is superseded by its parent and never finalized on its own;
                // it must never be voided independently.
                invoice.ensure_not_consolidated_child("void")?;

                // 2. Validate invoice can be voided (must be finalized and not paid)
                if invoice.status != domain::enums::InvoiceStatusEnum::Finalized {
                    bail!(StoreError::InvalidArgument(
                        "Only finalized invoices can be voided".to_string()
                    ));
                }

                if matches!(
                    invoice.payment_status,
                    domain::enums::InvoicePaymentStatus::Paid
                        | domain::enums::InvoicePaymentStatus::PartiallyPaid
                ) {
                    bail!(StoreError::InvalidArgument(
                        "Paid or partially paid invoices cannot be voided".to_string()
                    ));
                }

                // 2b. A consolidated parent activates its member subscriptions only when it is
                // paid (see `activate_subscription_on_invoice_paid`). Voiding it never pays, so any
                // member still awaiting that payment (TrialExpired / PendingCharge) would be
                // stranded with no remaining activation path — its own FinalizeInvoice event has
                // already no-op'd via `is_consolidated_child()`. Block the void in that case.
                if invoice.subscription_id.is_none() {
                    let children = InvoiceRow::list_consolidated_children(conn, tenant_id, id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    let member_sub_ids: Vec<_> =
                        children.iter().filter_map(|c| c.subscription_id).collect();
                    if !member_sub_ids.is_empty() {
                        let members = SubscriptionRow::list_by_ids(conn, &member_sub_ids)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                        let has_pending_activation = members.iter().any(|s| {
                            matches!(
                                s.subscription.status,
                                SubscriptionStatusEnum::TrialExpired
                                    | SubscriptionStatusEnum::PendingCharge
                            )
                        });
                        if has_pending_activation {
                            bail!(StoreError::InvalidArgument(
                                "Cannot void this consolidated invoice while some of its \
                                 subscriptions are awaiting this payment to activate"
                                    .to_string()
                            ));
                        }
                    }
                }

                let voided_coupons = invoice.coupons.clone();
                let voided_currency = invoice.currency.clone();

                // 3. The void gives back the credit the invoice drew; its note then converts only
                // collected cash, so the drawn credit is never returned twice. Every grant is
                // locked first, in the order the expiry sweep takes them (see step 4b).
                let customer_id = invoice.customer_id;
                // Unit wallets first, as finalization draws them before the money wallet.
                let money =
                    crate::repositories::credit::customer_credit::normalized_currency(&invoice.currency);
                let mut denominations =
                    diesel_models::customer_credit::CustomerCreditEntryRow::list_denominations_by_invoice(
                        conn, tenant_id, id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                denominations.retain(|code| *code != money);
                denominations.push(money);
                for denomination in &denominations {
                    crate::repositories::credit::customer_credit::lock_grants_tx(
                        conn,
                        lock,
                        denomination,
                    )
                    .await?;
                }
                crate::repositories::credit_notes::restore_invoice_credit_tx(
                    conn,
                    &*self.hooks.credit, lock, &invoice, now,
                )
                    .await?;
                crate::services::credit::follow_credit_given_back_tx(conn, &*self.hooks.credit, lock, id).await?;
                let note = create_credit_note_tx(
                    self,
                    conn,
                    tenant_id,
                    actor,
                    CreateCreditNoteTxParams {
                        invoice,
                        line_items: None, // Credit all line items
                        status: domain::enums::CreditNoteStatus::Draft,
                        finalized_at: None,
                        reason: Some("Invoice voided".to_string()),
                        memo: None,
                        credit_type: CreditType::CreditToBalance,
                        custom_properties: None,
                        from_proration: false,
                        purchased_credit_removed: false,
                        applied_credits_restored_externally: true,
                        voids_invoice: true,
                    },
                )
                .await?;
                crate::repositories::credit_notes::finalize_void_credit_note_tx(
                    self, conn, tenant_id, actor, note.id,
                )
                .await?;
                crate::repositories::credit_notes::take_back_invoice_granted_credit_tx(
                    conn,
                    &*self.hooks.credit, lock, id, note.id, now,
                )
                .await?;

                // 4. Void the invoice
                InvoiceRow::void(conn, id, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                crate::services::invoices::release_applied_coupons(
                    conn,
                    lock,
                    &voided_currency,
                    &voided_coupons,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                // 4b. Value given back to credit already due to lapse lapses now, in every
                // denomination. In a savepoint each: a failure leaves it to the expiry sweep, it
                // never blocks the void.
                for denomination in &denominations {
                    let lapsed = self
                        .transaction_with(conn, |conn| {
                            async move {
                                crate::services::credit::expiry_settlement::lapse_due_expiries_tx(
                                    self,
                                    &**usage,
                                    conn,
                                    tenant_id,
                                    customer_id,
                                    denomination,
                                    now,
                                )
                                .await
                            }
                            .scope_boxed()
                        })
                        .await;
                    if let Err(error) = lapsed {
                        log::error!(
                            "expired {denomination} credit given back by voided invoice {id} is left to the sweep: {error:?}"
                        );
                    }
                }
                self.internal
                    .end_reminders_best_effort(conn, tenant_id, id, chrono::Utc::now().naive_utc())
                    .await;

                // 5. Get the voided invoice
                let voided_invoice: DetailedInvoice =
                    InvoiceRow::find_detailed_by_id(conn, tenant_id, id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .try_into()?;

                // 6. Emit outbox event for voided invoice
                self.internal
                    .record_outbox_batch_tx(
                        conn,
                        tenant_id,
                        actor,
                        vec![domain::outbox_event::OutboxEvent::invoice_voided(
                            (&voided_invoice.invoice).into(),
                        )],
                    )
                    .await?;

                usage.ensure_served()?;
                Ok(voided_invoice)
            }
            .scope_boxed()
        })
        .await;
            if passes.served(self.usage_client.as_ref(), &usage).await? {
                return voided;
            }
        }
    }

    async fn mark_invoice_as_uncollectible(
        &self,
        id: InvoiceId,
        tenant_id: TenantId,
    ) -> StoreResult<DetailedInvoice> {
        let mut conn = self.get_conn().await?;

        // A consolidated child is never finalized on its own, so it can't legitimately become
        // uncollectible; guard explicitly rather than relying on the SQL status filter silently
        // no-op'ing.
        let invoice: Invoice = InvoiceRow::find_by_id(&mut conn, tenant_id, id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .and_then(TryInto::try_into)?;
        invoice.ensure_not_consolidated_child("mark as uncollectible")?;

        InvoiceRow::mark_as_uncollectible(&mut conn, id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        self.internal
            .end_reminders_best_effort(&mut conn, tenant_id, id, chrono::Utc::now().naive_utc())
            .await;

        InvoiceRow::find_detailed_by_id(&mut conn, tenant_id, id)
            .await
            .map_err(Into::into)
            .and_then(TryInto::try_into)
    }
}

/*

TODO special cases :
- cancellation/all invoice => all mrr logs after that should be cancelled, unless reactivation
- cancellation : recalculate the mrr delta (as the one in the event was calculated before the invoice was created)
- consolidation if multiple events in the same day. Ex: new business + expansion = new business, or cancellation + reactivation => nothing
 */
async fn process_mrr(inserted: &Invoice, conn: &mut PgConn) -> StoreResult<()> {
    log::debug!("Processing MRR logs for invoice {}", inserted.id);
    // Only a period's recurring invoice books its subscription events. An adjustment invoice
    // prorates an in-place change, which books its own movement; dated on the creation (or
    // cancellation) day, it would book that event a second time.
    if inserted.invoice_type == InvoiceType::Recurring {
        // A consolidated invoice has no subscription_id; its members already logged their MRR.
        let subscription_id = match inserted.subscription_id {
            Some(id) => id,
            None => return Ok(()),
        };

        // A regenerated period's movements were booked by the invoice it replaces.
        if InvoiceRow::period_billed_by_another_recurring_invoice(
            conn,
            inserted.tenant_id,
            subscription_id,
            inserted.billing_period_start,
            inserted.id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        {
            return Ok(());
        }

        let subscription_events = diesel_models::subscription_events::SubscriptionEventRow::fetch_by_subscription_id_and_date(
            conn,
            subscription_id,
            inserted.billing_period_start,
        )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let mut mrr_logs = vec![];
        let mut booked = vec![];

        for event in subscription_events {
            // Booked already, by this path or by the termination's churn booking.
            if event.bi_mrr_movement_log_id.is_some() {
                continue;
            }
            let mrr_delta = match event.mrr_delta {
                None | Some(0) => continue,
                Some(c) => c,
            };

            let movement_type = match event.event_type {
                // TODO
                // SubscriptionEventType::Created => continue,
                SubscriptionEventType::Created => MrrMovementType::NewBusiness, // TODO
                SubscriptionEventType::Activated => MrrMovementType::NewBusiness,
                SubscriptionEventType::Switch => {
                    if mrr_delta > 0 {
                        MrrMovementType::Expansion
                    } else {
                        MrrMovementType::Contraction
                    }
                }
                SubscriptionEventType::Cancelled => MrrMovementType::Churn,
                // Ended events carry mrr_delta = None (the churn is booked directly in
                // churn_ended_subscription), so they are skipped above; this arm only
                // keeps the match exhaustive.
                SubscriptionEventType::Ended => MrrMovementType::Churn,
                SubscriptionEventType::Reactivated => MrrMovementType::Reactivation,
                SubscriptionEventType::Updated => {
                    if mrr_delta > 0 {
                        MrrMovementType::Expansion
                    } else {
                        MrrMovementType::Contraction
                    }
                }
            };

            // TODO proper description from event_type + details
            let description = match event.event_type {
                SubscriptionEventType::Created => "Subscription created",
                SubscriptionEventType::Activated => "Subscription activated",
                SubscriptionEventType::Switch => "Switched plan",
                SubscriptionEventType::Cancelled => "Subscription cancelled",
                SubscriptionEventType::Ended => "Subscription ended",
                SubscriptionEventType::Reactivated => "Subscription reactivated",
                SubscriptionEventType::Updated => "Subscription updated",
            };

            if let Some(plan_version_id) = inserted.plan_version_id {
                let new_log = diesel_models::bi::BiMrrMovementLogRowNew {
                    id: Uuid::now_v7(),
                    description: description.to_string(),
                    movement_type,
                    net_mrr_change: mrr_delta,
                    currency: inserted.currency.clone(),
                    // Book against the service period, matching the event fetch above.
                    applies_to: inserted.billing_period_start,
                    invoice_id: Some(inserted.id),
                    credit_note_id: None,
                    plan_version_id,
                    tenant_id: inserted.tenant_id,
                    subscription_id,
                    customer_id: inserted.customer_id,
                };

                booked.push((event.id, new_log.id));
                mrr_logs.push(new_log);
            }
        }

        let mrr_delta_cents: i64 = mrr_logs.iter().map(|l| l.net_mrr_change).sum();

        // Insert MRR movement logs. This log is the transactional source the
        // analytics projector reads into ClickHouse `fact_mrr_movement`; the
        // legacy Postgres rollups (bi_delta_mrr_daily) are gone — the analytics
        // plane derives every MRR aggregate from these movements.
        diesel_models::bi::BiMrrMovementLogRow::insert_movement_log_batch(conn, mrr_logs)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        for (event_id, log_id) in booked {
            diesel_models::subscription_events::SubscriptionEventRow::update_mrr_movement_log_id(
                conn, event_id, log_id,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        }

        SubscriptionRow::update_subscription_mrr_delta(conn, subscription_id, mrr_delta_cents)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
    }
    Ok(())
}

pub async fn insert_invoice_tx(
    store: &Store,
    tx: &mut PgConn,
    invoice: InvoiceNew,
) -> StoreResult<Invoice> {
    insert_invoice_batch_tx(store, tx, vec![invoice])
        .await?
        .pop()
        .ok_or(StoreError::InsertError.into())
}

async fn insert_invoice_batch_tx(
    store: &Store,
    tx: &mut PgConn,
    invoice: Vec<InvoiceNew>,
) -> StoreResult<Vec<Invoice>> {
    // Check if any customers are archived before creating invoices
    use diesel_models::customers::CustomerRow;
    use itertools::Itertools;

    let customer_ids: Vec<CustomerId> =
        invoice.iter().map(|inv| inv.customer_id).unique().collect();

    if !customer_ids.is_empty() {
        let tenant_id = invoice.first().ok_or(StoreError::InsertError)?.tenant_id;

        if let Some((id, name)) =
            CustomerRow::find_archived_customer_in_batch(tx, tenant_id, customer_ids)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
        {
            return Err(StoreError::InvalidArgument(format!(
                "Cannot create invoice for archived customer: {} ({})",
                name, id
            ))
            .into());
        }
    }

    let mut insertable_invoice: Vec<InvoiceRowNew> = invoice
        .into_iter()
        .map(std::convert::TryInto::try_into)
        .collect::<Result<_, _>>()?;

    // Validate any user-supplied custom properties against the tenant's invoice definitions.
    // A null bag (system-triggered creates) is a no-op and falls back to an empty object.
    for row in insertable_invoice.iter_mut() {
        let normalized = store
            .internal
            .validate_and_normalize_custom_properties(
                tx,
                row.tenant_id,
                CustomPropertyEntityType::Invoice,
                &row.custom_properties,
                &serde_json::json!({}),
                CustomPropertyMode::Replace,
            )
            .await?;
        row.custom_properties = normalized;
    }

    let inserted: Vec<Invoice> = InvoiceRow::insert_invoice_batch(tx, insertable_invoice)
        .await
        .map_err(Into::<Report<StoreError>>::into)
        .and_then(|v| {
            v.into_iter()
                .map(TryInto::try_into)
                .collect::<Result<Vec<_>, Report<StoreError>>>()
        })?;

    // TODO batch
    for inv in &inserted {
        process_mrr(inv, tx).await?; // TODO
        let final_invoice: Invoice = InvoiceRow::find_by_id(tx, inv.tenant_id, inv.id)
            .await
            .map_err(Into::into)
            .and_then(std::convert::TryInto::try_into)?;

        // Invoice creation here is always system-triggered (billing pipeline,
        // subscription activation, slot transactions). User-initiated invoice
        // creation goes through a different path with a typed Actor.
        store
            .internal
            .record_outbox_batch_tx(
                tx,
                final_invoice.tenant_id,
                &Actor::System,
                vec![OutboxEvent::invoice_created((&final_invoice).into())],
            )
            .await?;
    }

    Ok(inserted)
}

/// Recomputes line tax outside the tax engine, for the paths that already know their rates:
/// the draft editor and manual invoices. Rate resolution stays the engine's job — this only
/// applies what a line already carries, through the same applier the engine uses, so an edit
/// can't drift from the figures the engine produced.
///
/// A line's `tax_details` are authoritative when they agree with its `tax_rate`; a rate the
/// operator changed by hand replaces them with a single tax at that rate.
pub fn apply_tax_rates_to_lines(lines: &mut [LineItem]) -> Vec<TaxBreakdownItem> {
    apply_tax_rates_to_priced_lines(lines, false).0
}

/// [`apply_tax_rates_to_lines`] for lines that may be priced tax-included.
///
/// When `tax_inclusive`, the lines come in carrying the figures they were quoted in (a
/// stored line goes back to them through `LineItem::into_tax_inclusive_quote`) and leave
/// net, exactly as they do from the invoice pipeline: the tax is carved out of each line
/// and the line is rewritten around what is left. Taxing the stored net lines on top
/// instead would land a cent away from the quoted price as soon as the rounding disagrees
/// (8.32 + 20% is 9.98, not the 9.99 it was netted from).
///
/// Also returns how much of the lines' discount turned out to be tax, which the
/// invoice-level discount has to shrink by. Always zero for tax-exclusive lines.
pub fn apply_tax_rates_to_priced_lines(
    lines: &mut [LineItem],
    tax_inclusive: bool,
) -> (Vec<TaxBreakdownItem>, i64) {
    let to_apply: Vec<LineToTax> = lines
        .iter()
        .map(|line| LineToTax {
            taxable_amount: line.taxable_amount,
            tax_inclusive,
            taxes: taxes_of(line)
                .iter()
                .map(|detail| TaxToApply {
                    rate: detail.tax_rate,
                    key: format!("{}:{}", detail.tax_name, detail.tax_rate),
                    // Stored lines never carry a compound tax: the engine cannot resolve one
                    // (it never passes a region), so nothing has been billed that way.
                    compound: false,
                })
                .collect(),
        })
        .collect();

    let mut discount_tax = 0;
    for ((line, taxes), amounts) in lines
        .iter_mut()
        .zip(to_apply.iter())
        .zip(apply_taxes(&to_apply))
    {
        line.tax_details = taxes
            .taxes
            .iter()
            .zip(amounts.iter().copied())
            .map(|(tax, tax_amount)| TaxDetail {
                tax_rate: tax.rate,
                tax_name: tax_name_of(&tax.key),
                tax_amount,
            })
            .collect();
        line.tax_amount = amounts.iter().sum();
        let priced_taxable = line.taxable_amount;
        line.taxable_amount = taxes.net_amount(&amounts);
        line.amount_total = line.taxable_amount + line.tax_amount;
        if tax_inclusive {
            discount_tax +=
                crate::services::invoice_lines::tax_inclusive::net_line(line, priced_taxable);
        }
    }

    (compute_tax_breakdown(lines), discount_tax)
}

/// The taxes a line carries: its own details, unless the rate was edited away from them —
/// then the edited rate is the single tax, since nothing records how to split it.
fn taxes_of(line: &LineItem) -> Vec<TaxDetail> {
    let details_match_rate = !line.tax_details.is_empty()
        && line
            .tax_details
            .iter()
            .map(|detail| detail.tax_rate)
            .sum::<Decimal>()
            == line.tax_rate;

    if details_match_rate {
        line.tax_details.clone()
    } else if line.tax_rate.is_zero() {
        Vec::new()
    } else {
        vec![TaxDetail {
            tax_rate: line.tax_rate,
            tax_name: "Tax".to_string(),
            tax_amount: 0,
        }]
    }
}

fn tax_name_of(key: &str) -> String {
    key.rsplit_once(':')
        .map(|(name, _)| name.to_string())
        .unwrap_or_else(|| key.to_string())
}

/// One row per distinct tax, so a stacked set (GST + QST) reports as the two taxes actually
/// levied. A line with no details reports under its own rate.
pub fn compute_tax_breakdown(lines: &[LineItem]) -> Vec<TaxBreakdownItem> {
    let mut tax_groups: OrderMap<(Decimal, String, bool), (i64, i64)> = OrderMap::new();

    for line in lines {
        // A charge promotional credit or a coupon took whole keeps its rates and its exemption:
        // its row stays, at nothing, so the category it is stated in has its row (BR-S-01).
        let taken_whole = line.amount_subtotal > 0 || !line.tax_details.is_empty();
        if line.tax_amount == 0 && line.taxable_amount == 0 && !taken_whole {
            continue;
        }

        if line.tax_details.is_empty() {
            let entry = tax_groups
                .entry((
                    line.tax_rate,
                    "Tax".to_string(),
                    line.states_prepaid_credit(),
                ))
                .or_insert((0, 0));
            entry.0 += line.taxable_amount;
            entry.1 += line.tax_amount;
        } else {
            for detail in &line.tax_details {
                let entry = tax_groups
                    .entry((detail.tax_rate, detail.tax_name.clone(), false))
                    .or_insert((0, 0));
                entry.0 += line.taxable_amount;
                entry.1 += detail.tax_amount;
            }
        }
    }

    tax_groups
        .into_iter()
        .map(
            |((tax_rate, name, prepaid_credit), (taxable_amount, tax_amount))| TaxBreakdownItem {
                taxable_amount,
                tax_amount,
                tax_rate,
                name,
                exemption_type: None,
                exemption_reason: None,
                tax_reference: None,
                overridden: false,
                // Rate rows carry no exemption, so no place-of-supply basis either.
                supply_basis: None,
                // Rebuilt from line details, which carry no tax kind of their own —
                // see `carry_over_tax_findings` for how a rebuild gets it back.
                tax_kind: None,
                prepaid_credit,
            },
        )
        .collect()
}

/// Re-attaches what the tax engine established to a breakdown rebuilt from line
/// details, taking it from the breakdown the rebuild derives from.
///
/// A rebuild recomputes the amounts by rate, but the engine's findings live only on
/// the breakdown, never on the lines: the exemption with its merchant mention and
/// place-of-supply basis (customer-level, so uniform across the document), and what
/// levy each rate is. Dropping them makes a credit note, or a draft after an edit,
/// state the same tax differently from the document it derives from — "VAT" under
/// an invoice that said "USt.", the directive citation under one that cited the
/// merchant's own article.
///
/// A rate row takes its kind only from a source row of the same name and rate: a
/// rate edited away from what the engine resolved is no longer the engine's, and
/// keeps no kind (its label falls back to the stored name).
pub fn carry_over_tax_findings(source: &[TaxBreakdownItem], rebuilt: &mut [TaxBreakdownItem]) {
    // An exemption states a rate of nothing: a rated row at nothing (a charge promotional credit
    // took whole) keeps its rate, never an exemption (BR-E-05).
    if let Some(exempt) = source.iter().find(|item| item.exemption_type.is_some()) {
        for item in rebuilt
            .iter_mut()
            .filter(|item| item.tax_amount == 0 && item.tax_rate.is_zero())
        {
            item.exemption_type = exempt.exemption_type.clone();
            item.exemption_reason = exempt.exemption_reason.clone();
            item.supply_basis = exempt.supply_basis;
        }
    }

    for item in rebuilt.iter_mut() {
        item.tax_kind = source
            .iter()
            .find(|s| s.name == item.name && s.tax_rate == item.tax_rate)
            .and_then(|s| s.tax_kind);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn line(local_id: &str, taxable_amount: i64, tax_rate: Decimal) -> LineItem {
        LineItem {
            local_id: local_id.to_string(),
            name: local_id.to_string(),
            charge_type: None,
            amount_subtotal: taxable_amount,
            tax_rate,
            taxable_amount,
            tax_amount: 0,
            amount_total: taxable_amount,
            tax_details: vec![],
            quantity: Some(dec!(1)),
            unit_price: None,
            start_date: chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            end_date: chrono::NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
            sub_lines: vec![],
            is_prorated: false,
            price_component_id: None,
            sub_component_id: None,
            sub_add_on_id: None,
            product_id: None,
            metric_id: None,
            description: None,
            group_by_dimensions: None,
            kind: crate::domain::LineItemKind::Regular,
            credit_grant_amount: None,
            credit_grant_expiry: None,
            credit_grant_included: false,
            taxed_at_purchase: false,
            tax_inclusive_quote: None,
            credit_unit: None,
            credit_grant_unit: None,
            credit_grant_rollover: None,
            promotional_credit: None,
            subscription_id: None,
            usage_ends_at: None,
        }
    }

    #[test]
    fn re_taxing_a_tax_inclusive_line_lands_back_on_the_quoted_price() {
        // 9.99 quoted at 20% is stored as 8.32 net + 1.67. Taxing the stored net on top
        // gives 8.32 + 1.66 = 9.98 — a cent off what the customer was quoted.
        let mut on_top = vec![line("drift", 832, dec!(0.2))];
        apply_tax_rates_to_lines(&mut on_top);
        assert_eq!(on_top[0].amount_total, 998);

        // Re-taxed from its quoted figures, it comes back to 9.99, and net again.
        let mut stored = line("quoted", 832, dec!(0.2));
        stored.tax_amount = 167;
        stored.amount_total = 999;
        stored.tax_inclusive_quote = Some(crate::domain::TaxInclusiveQuote {
            amount_subtotal: 999,
            unit_price: stored.unit_price,
            sub_lines: vec![],
        });
        let mut lines = vec![stored.into_tax_inclusive_quote()];

        let (breakdown, discount_tax) = apply_tax_rates_to_priced_lines(&mut lines, true);
        assert_eq!(discount_tax, 0);
        assert_eq!(lines[0].amount_total, 999);
        assert_eq!(lines[0].amount_subtotal, 832);
        assert_eq!(lines[0].taxable_amount, 832);
        assert_eq!(lines[0].tax_amount, 167);
        assert_eq!(breakdown[0].taxable_amount, 832);
        assert_eq!(crate::domain::quoted_discount(&lines), 0);
    }

    #[test]
    fn two_taxes_on_a_line_keep_their_own_rows_and_rounding() {
        // 10.50 with a 5% and a 7% customer rate: 0.525 and 0.735 round to 0.53 + 0.74.
        // The flattened 12% gives 1.26 in a single row.
        let mut lines = vec![line("multi", 1050, dec!(0.12))];
        lines[0].tax_details = vec![
            TaxDetail {
                tax_rate: dec!(0.05),
                tax_name: "State".to_string(),
                tax_amount: 53,
            },
            TaxDetail {
                tax_rate: dec!(0.07),
                tax_name: "City".to_string(),
                tax_amount: 74,
            },
        ];

        let breakdown = apply_tax_rates_to_lines(&mut lines);

        assert_eq!(lines[0].tax_amount, 127);
        assert_eq!(lines[0].tax_details[0].tax_amount, 53);
        assert_eq!(lines[0].tax_details[1].tax_amount, 74);
        assert_eq!(breakdown.len(), 2, "State and City keep their own rows");
    }

    #[test]
    fn recomputed_tax_is_rounded_once_per_rate() {
        // 15.12 + 10.17 at 20% is 5.058: per line it would be 3.02 + 2.03 = 5.05.
        let mut lines = vec![line("a", 1512, dec!(0.2)), line("b", 1017, dec!(0.2))];

        let breakdown = apply_tax_rates_to_lines(&mut lines);

        assert_eq!(lines[0].tax_amount, 303);
        assert_eq!(lines[1].tax_amount, 203);
        assert_eq!(lines[0].amount_total, 1815);
        assert_eq!(breakdown.len(), 1);
        assert_eq!(breakdown[0].tax_amount, 506);
        assert_eq!(breakdown[0].taxable_amount, 2529);
    }

    #[test]
    fn each_rate_is_rounded_over_its_own_base() {
        let mut lines = vec![
            line("standard", 1512, dec!(0.2)),
            line("standard2", 1017, dec!(0.2)),
            line("reduced", 1017, dec!(0.055)),
        ];

        let breakdown = apply_tax_rates_to_lines(&mut lines);

        assert_eq!(lines[2].tax_amount, 56, "10.17 at 5.5% is 0.55935");
        let standard = breakdown
            .iter()
            .find(|item| item.tax_rate == dec!(0.2))
            .unwrap();
        assert_eq!(standard.tax_amount, 506);
        assert_eq!(standard.taxable_amount, 2529);
    }

    #[test]
    fn a_single_tax_detail_follows_the_allocated_amount() {
        let mut lines = vec![line("a", 1512, dec!(0.2)), line("b", 1017, dec!(0.2))];
        for item in lines.iter_mut() {
            item.tax_details = vec![TaxDetail {
                tax_rate: dec!(0.2),
                tax_name: "VAT".to_string(),
                tax_amount: 0,
            }];
        }

        let breakdown = apply_tax_rates_to_lines(&mut lines);

        assert_eq!(lines[0].tax_details[0].tax_amount, 303);
        assert_eq!(lines[1].tax_details[0].tax_amount, 203);
        assert_eq!(breakdown[0].name, "VAT");
    }

    #[test]
    fn an_operator_edited_rate_replaces_the_stored_details() {
        // The details no longer add up to the line's rate, so the typed rate wins.
        let mut lines = vec![line("edited", 10000, dec!(0.1))];
        lines[0].tax_details = vec![TaxDetail {
            tax_rate: dec!(0.2),
            tax_name: "VAT".to_string(),
            tax_amount: 2000,
        }];

        apply_tax_rates_to_lines(&mut lines);

        assert_eq!(lines[0].tax_amount, 1000);
        assert_eq!(lines[0].tax_details.len(), 1);
        assert_eq!(lines[0].tax_details[0].tax_rate, dec!(0.1));
    }

    #[test]
    fn an_untaxed_line_carries_no_details() {
        let mut lines = vec![line("free", 10000, dec!(0))];

        let breakdown = apply_tax_rates_to_lines(&mut lines);

        assert_eq!(lines[0].tax_amount, 0);
        assert!(lines[0].tax_details.is_empty());
        assert_eq!(breakdown.len(), 1, "the zero-rated base still reports");
    }

    fn engine_row(name: &str, tax_rate: Decimal, tax_amount: i64) -> TaxBreakdownItem {
        TaxBreakdownItem {
            taxable_amount: 10000,
            tax_amount,
            tax_rate,
            name: name.to_string(),
            exemption_type: None,
            exemption_reason: None,
            tax_reference: None,
            overridden: false,
            supply_basis: None,
            tax_kind: None,
            prepaid_credit: false,
        }
    }

    /// A rebuild from lines (credit note, draft edit) must word the tax as the
    /// document it derives from did: the engine's kind comes back onto the row of
    /// the same name and rate, so a German credit note keeps "USt." rather than
    /// falling back to the engine's English name.
    #[test]
    fn a_rebuilt_breakdown_keeps_the_engines_tax_kind() {
        use crate::domain::invoices::TaxKind;

        let mut source = engine_row("VAT", dec!(0.2), 2000);
        source.tax_kind = Some(TaxKind::Vat);

        let mut lines = vec![line("a", 10000, dec!(0.2))];
        lines[0].tax_details = vec![TaxDetail {
            tax_rate: dec!(0.2),
            tax_name: "VAT".to_string(),
            tax_amount: 0,
        }];
        let mut rebuilt = apply_tax_rates_to_lines(&mut lines);
        assert_eq!(rebuilt[0].tax_kind, None, "the lines alone cannot know it");

        carry_over_tax_findings(&[source], &mut rebuilt);
        assert_eq!(rebuilt[0].tax_kind, Some(TaxKind::Vat));
    }

    /// An operator-edited rate is no longer what the engine resolved, so it must not
    /// inherit the engine's kind — its label falls back to the stored name.
    #[test]
    fn an_edited_rate_does_not_inherit_the_engines_tax_kind() {
        use crate::domain::invoices::TaxKind;

        let mut source = engine_row("VAT", dec!(0.2), 2000);
        source.tax_kind = Some(TaxKind::Vat);
        let mut rebuilt = vec![engine_row("Tax", dec!(0.1), 1000)];

        carry_over_tax_findings(&[source], &mut rebuilt);
        assert_eq!(rebuilt[0].tax_kind, None);
    }

    /// The exemption travels whole: type, the merchant's own mention (which the PDF
    /// and BT-120 print in place of the statutory wording), and the basis the
    /// citation is chosen by. Carrying only the type made a credit note cite the
    /// directive under an invoice that cited the merchant's article.
    #[test]
    fn a_rebuilt_breakdown_keeps_the_whole_exemption() {
        use crate::domain::invoices::{SupplyBasis, TaxExemptionType};

        let mut source = engine_row("Exempt", Decimal::ZERO, 0);
        source.exemption_type = Some(TaxExemptionType::ReverseCharge);
        source.exemption_reason = Some("Autoliquidation - art. 283-2 du CGI".to_string());
        source.supply_basis = Some(SupplyBasis::Business);
        let mut rebuilt = vec![engine_row("Tax", Decimal::ZERO, 0)];

        carry_over_tax_findings(&[source], &mut rebuilt);
        assert_eq!(
            rebuilt[0].exemption_type,
            Some(TaxExemptionType::ReverseCharge)
        );
        assert_eq!(
            rebuilt[0].exemption_reason.as_deref(),
            Some("Autoliquidation - art. 283-2 du CGI")
        );
        assert_eq!(rebuilt[0].supply_basis, Some(SupplyBasis::Business));
    }
}
