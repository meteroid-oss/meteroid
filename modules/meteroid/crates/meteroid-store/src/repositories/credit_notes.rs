use crate::domain::invoice_lines::LineItem;
use crate::domain::invoices::TaxBreakdownItem;
use crate::domain::{
    CreditNote, CreditNoteNew, Invoice, InvoicePaymentStatus, InvoiceStatusEnum,
    outbox_event::OutboxEvent,
};
use crate::errors::StoreError;
use crate::repositories::customer_balance::CustomerBalance;
use crate::store::Store;
use crate::StoreResult;
use chrono::NaiveDateTime;
use common_domain::ids::{CreditNoteId, CustomerId, InvoiceId, StoredDocumentId, TenantId};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::credit_notes::CreditNoteRow;
use diesel_models::invoices::InvoiceRow;
use diesel_models::invoicing_entities::InvoicingEntityRow;
use diesel_models::PgConn;
use error_stack::{Report, bail};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use std::collections::HashMap;

/// How the credit note amount should be applied
#[derive(Debug, Clone)]
pub enum CreditType {
    /// Credit is added to customer's balance for future invoices
    CreditToBalance,
    /// Credit triggers a refund to the customer
    Refund,
}

/// Parameters for creating a credit note via the public API
#[derive(Debug, Clone)]
pub struct CreateCreditNoteParams {
    pub invoice_id: InvoiceId,
    /// Local IDs of line items to credit. Empty means credit all line items.
    pub line_item_ids: Vec<String>,
    pub reason: Option<String>,
    pub memo: Option<String>,
    pub credit_type: CreditType,
}

/// Internal parameters for creating a credit note within a transaction.
/// Used by both `create_credit_note` and `void_invoice`.
#[derive(Debug, Clone)]
pub(crate) struct CreateCreditNoteTxParams {
    /// The invoice to create the credit note for (must be already loaded)
    pub invoice: Invoice,
    /// Line items to credit. If None, credits all invoice line items.
    pub line_item_ids: Option<Vec<String>>,
    /// Initial status of the credit note
    pub status: crate::domain::enums::CreditNoteStatus,
    /// When the credit note was finalized (only set if status is Finalized)
    pub finalized_at: Option<NaiveDateTime>,
    /// Reason for the credit note
    pub reason: Option<String>,
    /// Optional memo
    pub memo: Option<String>,
    /// How to apply the credit
    pub credit_type: CreditType,
}

#[async_trait::async_trait]
pub trait CreditNoteInterface {
    async fn insert_credit_note(&self, credit_note: CreditNoteNew) -> StoreResult<CreditNote>;

    async fn create_credit_note(
        &self,
        tenant_id: TenantId,
        params: CreateCreditNoteParams,
    ) -> StoreResult<CreditNote>;

    async fn list_credit_notes(
        &self,
        tenant_id: TenantId,
        customer_id: Option<CustomerId>,
        invoice_id: Option<InvoiceId>,
        status: Option<crate::domain::enums::CreditNoteStatus>,
        search: Option<String>,
        order_by: crate::domain::OrderByRequest,
        pagination: crate::domain::PaginationRequest,
    ) -> StoreResult<crate::domain::PaginatedVec<CreditNote>>;

    async fn get_credit_note_by_id(
        &self,
        tenant_id: TenantId,
        credit_note_id: CreditNoteId,
    ) -> StoreResult<CreditNote>;

    async fn list_credit_notes_by_invoice_id(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<Vec<CreditNote>>;

    async fn list_credit_notes_by_customer_id(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> StoreResult<Vec<CreditNote>>;

    async fn finalize_credit_note(
        &self,
        tenant_id: TenantId,
        credit_note_id: CreditNoteId,
    ) -> StoreResult<CreditNote>;

    async fn void_credit_note(
        &self,
        tenant_id: TenantId,
        credit_note_id: CreditNoteId,
    ) -> StoreResult<CreditNote>;

    async fn list_credit_notes_by_ids(
        &self,
        credit_note_ids: Vec<CreditNoteId>,
    ) -> StoreResult<Vec<CreditNote>>;

    async fn save_credit_note_pdf_document(
        &self,
        credit_note_id: CreditNoteId,
        tenant_id: TenantId,
        pdf_document_id: StoredDocumentId,
    ) -> StoreResult<()>;
}

// ============================================================================
// Internal helper functions for credit note creation
// ============================================================================

/// Creates negated line items for a credit note.
/// The amounts are negated to represent credits.
fn negate_line_items(line_items: &[LineItem]) -> Vec<LineItem> {
    line_items
        .iter()
        .map(|item| LineItem {
            local_id: item.local_id.clone(),
            name: item.name.clone(),
            // Negate monetary amounts to represent the credit
            amount_subtotal: -item.amount_subtotal,
            taxable_amount: -item.taxable_amount,
            tax_amount: -item.tax_amount,
            amount_total: -item.amount_total,
            // Keep these as-is - they describe the original charge
            tax_rate: item.tax_rate,
            tax_details: item.tax_details.clone(),
            quantity: item.quantity,
            unit_price: item.unit_price,
            start_date: item.start_date,
            end_date: item.end_date,
            sub_lines: item.sub_lines.clone(),
            is_prorated: item.is_prorated,
            price_component_id: item.price_component_id,
            sub_component_id: item.sub_component_id,
            sub_add_on_id: item.sub_add_on_id,
            product_id: item.product_id,
            metric_id: item.metric_id,
            description: item.description.clone(),
            group_by_dimensions: item.group_by_dimensions.clone(),
        })
        .collect()
}

/// Computes tax breakdown from line items using their tax_details.
/// Groups by (tax_rate, tax_name) to preserve individual tax information.
/// Returns unsigned amounts (the credit note context implies the direction).
fn compute_tax_breakdown(line_items: &[LineItem]) -> Vec<TaxBreakdownItem> {
    // Group by (tax_rate, tax_name) to preserve detailed breakdown
    let mut tax_groups: HashMap<(Decimal, String), u64> = HashMap::new();
    // Track taxable amounts separately per tax rate (since multiple taxes can apply to same base)
    let mut taxable_by_rate: HashMap<Decimal, u64> = HashMap::new();

    for item in line_items {
        let taxable = item.taxable_amount.abs() as u64;

        if item.tax_details.is_empty() {
            // No detailed breakdown available, use combined rate
            if item.tax_amount != 0 || taxable > 0 {
                let key = (item.tax_rate, "Tax".to_string());
                *tax_groups.entry(key).or_insert(0) += item.tax_amount.abs() as u64;
                *taxable_by_rate.entry(item.tax_rate).or_insert(0) += taxable;
            }
        } else {
            // Use detailed breakdown - each tax_detail has its own rate and amount
            for detail in &item.tax_details {
                let key = (detail.tax_rate, detail.tax_name.clone());
                *tax_groups.entry(key).or_insert(0) += detail.tax_amount.abs() as u64;
                // Taxable amount is shared across all taxes on this line item
                // We track it per rate to avoid double-counting when multiple taxes share a rate
            }
            // Add taxable amount once per unique rate on this item
            let mut seen_rates: HashMap<Decimal, bool> = HashMap::new();
            for detail in &item.tax_details {
                if !seen_rates.contains_key(&detail.tax_rate) {
                    *taxable_by_rate.entry(detail.tax_rate).or_insert(0) += taxable;
                    seen_rates.insert(detail.tax_rate, true);
                }
            }
        }
    }

    tax_groups
        .into_iter()
        .map(|((tax_rate, name), tax_amount)| {
            let taxable_amount = taxable_by_rate.get(&tax_rate).copied().unwrap_or(0);
            TaxBreakdownItem {
                taxable_amount,
                tax_amount,
                tax_rate,
                name,
                exemption_type: None, // TODO: preserve exemption info if needed
            }
        })
        .collect()
}

/// Internal function to create a credit note within an existing transaction.
/// This is the shared implementation used by both `create_credit_note` and `void_invoice`.
pub(crate) async fn create_credit_note_tx(
    store: &Store,
    conn: &mut PgConn,
    tenant_id: TenantId,
    params: CreateCreditNoteTxParams,
) -> StoreResult<CreditNote> {
    let invoice = params.invoice;

    // 1. Lock the invoice row to prevent race conditions when creating credit notes
    // This ensures that concurrent credit note creations for the same invoice are serialized
    let _invoice_lock = InvoiceRow::select_for_update_by_id(conn, tenant_id, invoice.id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    // Get existing credit notes for this invoice to check for already credited lines
    let existing_credit_notes = CreditNoteRow::list_by_invoice_id(conn, tenant_id, invoice.id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    // Collect all line item IDs that have already been credited (excluding voided credit notes)
    let already_credited_line_ids: std::collections::HashSet<String> = existing_credit_notes
        .iter()
        .filter(|cn| cn.status != diesel_models::enums::CreditNoteStatus::Voided)
        .flat_map(|cn| {
            // Parse line_items JSON to extract local_ids
            let line_items: Vec<LineItem> = serde_json::from_value(cn.line_items.clone())
                .unwrap_or_default();
            line_items.into_iter().map(|item| item.local_id)
        })
        .collect();

    // 2. Filter line items to credit
    let line_items_to_credit: Vec<LineItem> = match params.line_item_ids {
        None => invoice.line_items.clone(),
        Some(ids) if ids.is_empty() => invoice.line_items.clone(),
        Some(ids) => invoice
            .line_items
            .iter()
            .filter(|item| ids.contains(&item.local_id))
            .cloned()
            .collect(),
    };

    if line_items_to_credit.is_empty() {
        bail!(StoreError::InvalidArgument(
            "No line items to credit".to_string()
        ));
    }

    // 3. Check for duplicate line items (already credited)
    let requested_line_ids: Vec<&str> = line_items_to_credit
        .iter()
        .map(|item| item.local_id.as_str())
        .collect();

    let duplicate_ids: Vec<&str> = requested_line_ids
        .iter()
        .filter(|id| already_credited_line_ids.contains(**id))
        .copied()
        .collect();

    if !duplicate_ids.is_empty() {
        bail!(StoreError::InvalidArgument(format!(
            "Line items already credited: {}",
            duplicate_ids.join(", ")
        )));
    }

    // 4. Create negated line items for the credit note
    let negated_line_items = negate_line_items(&line_items_to_credit);

    // 5. Calculate totals from negated line items (will be negative)
    let subtotal: i64 = negated_line_items
        .iter()
        .map(|item| item.amount_subtotal)
        .sum();
    let tax_amount: i64 = negated_line_items.iter().map(|item| item.tax_amount).sum();
    let total: i64 = negated_line_items
        .iter()
        .map(|item| item.amount_total)
        .sum();

    // 6. Compute tax breakdown from original line items (unsigned amounts)
    let tax_breakdown = compute_tax_breakdown(&line_items_to_credit);

    // 7. Get credit note number - only assign real number when finalizing
    let (credit_note_number, credit_note_number_value) =
        if params.status == crate::domain::enums::CreditNoteStatus::Finalized {
            // Lock invoicing entity for sequential numbering
            let invoicing_entity = InvoicingEntityRow::select_for_update_by_id_and_tenant(
                conn,
                invoice.invoicing_entity_id,
                tenant_id,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

            let number_value = invoicing_entity.next_credit_note_number;
            (format!("CN-{:06}", number_value), Some(number_value))
        } else {
            // Draft credit notes use placeholder
            ("draft".to_string(), None)
        };

    // 8. Determine credit/refund amounts based on credit type
    // These are positive values representing how much is credited/refunded
    let credit_total = total.unsigned_abs() as i64;

    let (credited_amount_cents, refunded_amount_cents) = match params.credit_type {
        CreditType::CreditToBalance => (credit_total, 0),
        CreditType::Refund => {
            // For refunds, we need to handle applied credits from the original invoice.
            // If the invoice was partially paid with customer balance (applied_credits),
            // we should restore that portion to the balance and only refund what was actually paid.
            //
            // Calculate proportionally based on how much of the invoice we're crediting:
            // - If crediting full invoice: use full applied_credits
            // - If partial: prorate the applied_credits
            let invoice_total = invoice.total;

            if invoice_total > 0 && invoice.applied_credits > 0 {
                // Calculate the proportion of the invoice being credited
                let credit_proportion =
                    Decimal::from(credit_total) / Decimal::from(invoice_total);

                // Prorate the applied_credits for this credit note
                let applied_credits_portion = (Decimal::from(invoice.applied_credits)
                    * credit_proportion)
                    .round()
                    .to_i64()
                    .unwrap_or(0);

                // The rest should be refunded
                let refund_portion = credit_total - applied_credits_portion;

                (applied_credits_portion, refund_portion)
            } else {
                // No applied credits, full refund
                (0, credit_total)
            }
        }
    };

    // 9. Build the credit note
    let credit_note_new = CreditNoteNew {
        credit_note_number: credit_note_number.clone(),
        status: params.status.clone(),
        tenant_id,
        customer_id: invoice.customer_id,
        invoice_id: invoice.id,
        plan_version_id: invoice.plan_version_id,
        subscription_id: invoice.subscription_id,
        currency: invoice.currency.clone(),
        subtotal,
        tax_amount,
        total,
        refunded_amount_cents,
        credited_amount_cents,
        line_items: negated_line_items,
        tax_breakdown,
        reason: params.reason,
        memo: params.memo,
        customer_details: invoice.customer_details.clone(),
        seller_details: invoice.seller_details.clone(),
        invoicing_entity_id: invoice.invoicing_entity_id,
        finalized_at: params.finalized_at,
    };

    // 10. Insert the credit note
    let insertable: diesel_models::credit_notes::CreditNoteRowNew = credit_note_new.try_into()?;
    let inserted_credit_note = insertable
        .insert(conn)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    // 11. Convert to domain model
    let credit_note: CreditNote = inserted_credit_note.try_into()?;

    // 12. If credit note is created as Finalized, update customer balance immediately
    if params.status == crate::domain::enums::CreditNoteStatus::Finalized
        && credit_note.credited_amount_cents > 0
    {
        CustomerBalance::update(
            conn,
            credit_note.customer_id,
            tenant_id,
            credit_note.credited_amount_cents,
            None, // No invoice_id for credit note balance updates
        )
        .await?;
    }

    // 13. Emit outbox events
    let mut events = vec![OutboxEvent::credit_note_created((&credit_note).into())];
    // If created as finalized, also emit the finalized event
    if params.status == crate::domain::enums::CreditNoteStatus::Finalized {
        events.push(OutboxEvent::credit_note_finalized((&credit_note).into()));
    }
    store
        .internal
        .insert_outbox_events_tx(conn, events)
        .await?;

    // 14. Update the credit note number in the invoicing entity (only when finalizing)
    if let Some(number_value) = credit_note_number_value {
        InvoicingEntityRow::update_credit_note_number(
            conn,
            invoice.invoicing_entity_id,
            tenant_id,
            number_value,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    }

    Ok(credit_note)
}

#[async_trait::async_trait]
impl CreditNoteInterface for Store {
    async fn insert_credit_note(&self, credit_note: CreditNoteNew) -> StoreResult<CreditNote> {
        let mut conn = self.get_conn().await?;

        let insertable: diesel_models::credit_notes::CreditNoteRowNew =
            credit_note.try_into()?;

        insertable.insert( &mut conn)
            .await
            .map_err(Into::into)
            .and_then(std::convert::TryInto::try_into)
    }

    async fn create_credit_note(
        &self,
        tenant_id: TenantId,
        params: CreateCreditNoteParams,
    ) -> StoreResult<CreditNote> {
        self.transaction(|conn| {
            async move {
                // 1. Get and validate the invoice
                let detailed_invoice =
                    InvoiceRow::find_detailed_by_id(conn, tenant_id, params.invoice_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                let invoice: Invoice = detailed_invoice.invoice.try_into()?;

                // 2. Validate invoice is finalized
                if invoice.status != InvoiceStatusEnum::Finalized {
                    bail!(StoreError::InvalidArgument(
                        "Credit notes can only be created for finalized invoices".to_string()
                    ));
                }

                // 3. Validate payment status for refund type
                if matches!(params.credit_type, CreditType::Refund)
                    && !matches!(
                        invoice.payment_status,
                        InvoicePaymentStatus::Paid | InvoicePaymentStatus::PartiallyPaid
                    )
                {
                    bail!(StoreError::InvalidArgument(
                        "Refund-type credit notes can only be created for paid or partially paid invoices".to_string()
                    ));
                }

                // 4. Create credit note using shared implementation
                let line_item_ids = if params.line_item_ids.is_empty() {
                    None
                } else {
                    Some(params.line_item_ids)
                };

                create_credit_note_tx(
                    self,
                    conn,
                    tenant_id,
                    CreateCreditNoteTxParams {
                        invoice,
                        line_item_ids,
                        status: crate::domain::enums::CreditNoteStatus::Draft,
                        finalized_at: None,
                        reason: params.reason,
                        memo: params.memo,
                        credit_type: params.credit_type,
                    },
                )
                .await
            }
            .scope_boxed()
        })
        .await
    }

    async fn list_credit_notes(
        &self,
        tenant_id: TenantId,
        customer_id: Option<CustomerId>,
        invoice_id: Option<InvoiceId>,
        status: Option<crate::domain::enums::CreditNoteStatus>,
        search: Option<String>,
        order_by: crate::domain::OrderByRequest,
        pagination: crate::domain::PaginationRequest,
    ) -> StoreResult<crate::domain::PaginatedVec<CreditNote>> {
        let mut conn = self.get_conn().await?;

        let rows = CreditNoteRow::list(
            &mut conn,
            tenant_id,
            customer_id,
            invoice_id,
            status.map(Into::into),
            search,
            order_by.into(),
            pagination.into(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let items: Vec<CreditNote> = rows
            .items
            .into_iter()
            .map(std::convert::TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(crate::domain::PaginatedVec {
            items,
            total_pages: rows.total_pages,
            total_results: rows.total_results,
        })
    }

    async fn get_credit_note_by_id(
        &self,
        tenant_id: TenantId,
        credit_note_id: CreditNoteId,
    ) -> StoreResult<CreditNote> {
        let mut conn = self.get_conn().await?;

        CreditNoteRow::find_by_id(&mut conn, tenant_id, credit_note_id)
            .await
            .map_err(Into::into)
            .and_then(std::convert::TryInto::try_into)
    }

    async fn list_credit_notes_by_invoice_id(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<Vec<CreditNote>> {
        let mut conn = self.get_conn().await?;

        let rows = CreditNoteRow::list_by_invoice_id(&mut conn, tenant_id, invoice_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        rows.into_iter()
            .map(std::convert::TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()
    }

    async fn list_credit_notes_by_customer_id(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> StoreResult<Vec<CreditNote>> {
        let mut conn = self.get_conn().await?;

        let rows = CreditNoteRow::list_by_customer_id(&mut conn, tenant_id, customer_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        rows.into_iter()
            .map(std::convert::TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()
    }

    async fn finalize_credit_note(
        &self,
        tenant_id: TenantId,
        credit_note_id: CreditNoteId,
    ) -> StoreResult<CreditNote> {
        self.transaction(|conn| {
            async move {
                // 1. Get the credit note
                let credit_note_row =
                    CreditNoteRow::find_by_id(conn, tenant_id, credit_note_id).await?;

                // 2. Validate it's a draft
                if credit_note_row.status != diesel_models::enums::CreditNoteStatus::Draft {
                    bail!(StoreError::InvalidArgument(
                        "Only draft credit notes can be finalized".to_string()
                    ));
                }

                // 3. Lock invoicing entity and get next credit note number
                let invoicing_entity = InvoicingEntityRow::select_for_update_by_id_and_tenant(
                    conn,
                    credit_note_row.invoicing_entity_id,
                    tenant_id,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                let credit_note_number_value = invoicing_entity.next_credit_note_number;
                let credit_note_number = format!("CN-{:06}", credit_note_number_value);

                // 4. Finalize the credit note with the assigned number
                CreditNoteRow::finalize_with_number(
                    conn,
                    credit_note_id,
                    tenant_id,
                    &credit_note_number,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                // 5. Update invoicing entity's next credit note number
                InvoicingEntityRow::update_credit_note_number(
                    conn,
                    credit_note_row.invoicing_entity_id,
                    tenant_id,
                    credit_note_number_value,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                // 6. Update customer balance if there are credited amounts
                if credit_note_row.credited_amount_cents > 0 {
                    CustomerBalance::update(
                        conn,
                        credit_note_row.customer_id,
                        tenant_id,
                        credit_note_row.credited_amount_cents,
                        None, // No invoice_id for credit note balance updates
                    )
                    .await?;
                }

                // 7. Get the finalized credit note
                let credit_note: CreditNote = CreditNoteRow::find_by_id(conn, tenant_id, credit_note_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    .try_into()?;

                // 8. Emit outbox event for finalized credit note
                self.internal
                    .insert_outbox_events_tx(
                        conn,
                        vec![OutboxEvent::credit_note_finalized((&credit_note).into())],
                    )
                    .await?;

                Ok(credit_note)
            }
            .scope_boxed()
        })
        .await
    }

    async fn void_credit_note(
        &self,
        tenant_id: TenantId,
        credit_note_id: CreditNoteId,
    ) -> StoreResult<CreditNote> {
        self.transaction(|conn| {
            async move {
                // 1. Get the credit note to check if balance needs to be reversed
                let credit_note_row =
                    CreditNoteRow::find_by_id(conn, tenant_id, credit_note_id).await?;

                // 2. Validate it's finalized (only finalized credit notes can be voided)
                if credit_note_row.status != diesel_models::enums::CreditNoteStatus::Finalized {
                    bail!(StoreError::InvalidArgument(
                        "Only finalized credit notes can be voided".to_string()
                    ));
                }

                // 3. Void the credit note
                CreditNoteRow::void(conn, credit_note_id, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                // 4. Reverse customer balance if there were credited amounts
                if credit_note_row.credited_amount_cents > 0 {
                    CustomerBalance::update(
                        conn,
                        credit_note_row.customer_id,
                        tenant_id,
                        -credit_note_row.credited_amount_cents, // Negative to reverse
                        None,
                    )
                    .await?;
                }

                // 5. Get the voided credit note
                let credit_note: CreditNote = CreditNoteRow::find_by_id(conn, tenant_id, credit_note_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    .try_into()?;

                // 6. Emit outbox event for voided credit note
                self.internal
                    .insert_outbox_events_tx(
                        conn,
                        vec![OutboxEvent::credit_note_voided((&credit_note).into())],
                    )
                    .await?;

                Ok(credit_note)
            }
            .scope_boxed()
        })
        .await
    }

    async fn list_credit_notes_by_ids(
        &self,
        credit_note_ids: Vec<CreditNoteId>,
    ) -> StoreResult<Vec<CreditNote>> {
        let mut conn = self.get_conn().await?;

        let rows = CreditNoteRow::list_by_ids(&mut conn, credit_note_ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        rows.into_iter()
            .map(std::convert::TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()
    }

    async fn save_credit_note_pdf_document(
        &self,
        credit_note_id: CreditNoteId,
        tenant_id: TenantId,
        pdf_document_id: StoredDocumentId,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        CreditNoteRow::update_pdf_document_id(&mut conn, credit_note_id, tenant_id, pdf_document_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(())
    }
}
