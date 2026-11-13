use crate::StoreResult;
use crate::domain::{DetailedInvoice, InlineCustomer, Invoice, LineItem, UpdateInvoiceParams};
use crate::errors::StoreError;
use crate::repositories::InvoiceInterface;
use crate::repositories::invoices::{apply_tax_rates_to_priced_lines, carry_over_tax_findings};
use crate::services::Services;
use crate::utils::local_id::{IdType, LocalId};
use chrono::{NaiveDateTime, NaiveTime};
use common_domain::ids::{InvoiceId, TenantId};
use common_utils::decimals::ToSubunit;
use diesel_models::PgConn;
use diesel_models::customers::CustomerRow;
use diesel_models::invoices::{InvoiceRow, InvoiceRowPatch};
use diesel_models::invoicing_entities::InvoicingEntityRow;
use diesel_models::payments::PaymentTransactionRow;
use error_stack::bail;
use rust_decimal::Decimal;
use scoped_futures::ScopedFutureExt;
use std::collections::HashMap;

#[allow(clippy::large_enum_variant)]
pub enum CustomerDetailsUpdate {
    RefreshFromCustomer,
    InlineEdit {
        name: Option<String>,
        billing_address: Option<crate::domain::Address>,
        vat_number: Option<String>,
    },
}

impl Services {
    pub async fn update_draft_invoice(
        &self,
        id: InvoiceId,
        tenant_id: TenantId,
        params: UpdateInvoiceParams,
    ) -> StoreResult<DetailedInvoice> {
        self.store
            .transaction(|conn| {
                self.update_draft_invoice_tx(conn, id, tenant_id, params)
                    .scope_boxed()
            })
            .await
    }

    pub async fn preview_draft_invoice_update(
        &self,
        id: InvoiceId,
        tenant_id: TenantId,
        params: UpdateInvoiceParams,
    ) -> StoreResult<Invoice> {
        let invoice = self.store.get_invoice_by_id(tenant_id, id).await?;

        if !invoice.can_edit() {
            bail!(StoreError::InvalidArgument(
                "Can only preview edits for draft invoices".into(),
            ));
        }
        invoice.ensure_not_consolidated_child("edit")?;

        let mut conn = self.store.get_conn().await?;
        let (mut updated_invoice, _) = self
            .prepare_invoice_update(&mut conn, invoice, tenant_id, params)
            .await?;
        // Release this connection before the projection takes another from the pool.
        drop(conn);

        // Project like the read paths do, so the preview's dates match what GetInvoice shows.
        self.store
            .project_draft_emission_dates(&mut updated_invoice)
            .await?;

        Ok(updated_invoice)
    }

    async fn update_draft_invoice_tx(
        &self,
        conn: &mut PgConn,
        id: InvoiceId,
        tenant_id: TenantId,
        params: UpdateInvoiceParams,
    ) -> StoreResult<DetailedInvoice> {
        let invoice_lock = InvoiceRow::select_for_update_by_id(conn, tenant_id, id).await?;
        diesel_models::query::credit_inputs::bump_customer(conn, &invoice_lock.lock).await?;
        let invoice: Invoice = invoice_lock.invoice.try_into()?;

        if !invoice.can_edit() {
            bail!(StoreError::InvalidArgument(
                "Can only edit draft invoices".into(),
            ));
        }
        invoice.ensure_not_consolidated_child("edit")?;

        // Editing amounts under a live charge would desync the invoice from the money the
        // provider is already moving: on settlement the invoice lands PartiallyPaid or
        // overpaid depending on which way the edit went.
        if PaymentTransactionRow::exists_live_for_invoice(conn, id, tenant_id).await? {
            bail!(StoreError::InvalidArgument(
                "Cannot edit an invoice with a payment in progress".into(),
            ));
        }

        let existing_custom_properties = invoice.custom_properties.clone();
        let provided_custom_properties = params.custom_properties.clone();

        // Line, discount or annex edits mean the operator took ownership of the draft's billing
        // content: stamp it so a manual finalize keeps these lines instead of settling with a
        // refresh, and keeps the appendix as stored.
        // Metadata-only edits (memo, PO, due date, customer details) don't freeze anything.
        let billing_content_edited =
            params.line_items.is_some() || params.discount.is_some() || params.annexes.is_some();

        let annexes_edited = params.annexes.is_some();

        let (mut invoice, customer_details_overridden_at) = self
            .prepare_invoice_update(conn, invoice, tenant_id, params)
            .await?;

        if let Some(provided) = provided_custom_properties {
            invoice.custom_properties = self
                .store
                .internal
                .validate_and_normalize_custom_properties(
                    conn,
                    tenant_id,
                    crate::domain::enums::CustomPropertyEntityType::Invoice,
                    &provided,
                    &existing_custom_properties,
                    crate::domain::custom_property_definitions::CustomPropertyMode::Merge,
                )
                .await?;
        }

        let patch = InvoiceRowPatch {
            line_items: serde_json::to_value(&invoice.line_items).map_err(|e| {
                StoreError::SerdeError("Failed to serialize line_items".to_string(), e)
            })?,
            customer_details: serde_json::to_value(&invoice.customer_details).map_err(|e| {
                StoreError::SerdeError("Failed to serialize customer_details".to_string(), e)
            })?,
            seller_details: serde_json::to_value(&invoice.seller_details).map_err(|e| {
                StoreError::SerdeError("Failed to serialize seller_details".to_string(), e)
            })?,
            discount: invoice.discount,
            memo: invoice.memo.clone(),
            reference: invoice.reference.clone(),
            purchase_order: invoice.purchase_order.clone(),
            due_at: invoice.due_at,
            invoicing_entity_id: invoice.invoicing_entity_id,
            subtotal: invoice.subtotal,
            subtotal_recurring: invoice.subtotal_recurring,
            tax_amount: invoice.tax_amount,
            total: invoice.total,
            amount_due: invoice.amount_due,
            promotional_credits: invoice.promotional_credits,
            tax_breakdown: serde_json::to_value(&invoice.tax_breakdown).map_err(|e| {
                StoreError::SerdeError("Failed to serialize tax_breakdown".to_string(), e)
            })?,
            custom_properties: invoice.custom_properties.clone(),
            annexes: annexes_edited
                .then(|| {
                    serde_json::to_value(&invoice.annexes).map_err(|e| {
                        StoreError::SerdeError("Failed to serialize annexes".to_string(), e)
                    })
                })
                .transpose()?,
            manually_edited_at: billing_content_edited
                .then(|| common_domain::clock::now().naive_utc()),
            customer_details_overridden_at,
        };

        patch.update(id, tenant_id, conn).await?;

        let detailed_invoice = InvoiceRow::find_detailed_by_id(conn, tenant_id, id)
            .await
            .map_err(Into::into)
            .and_then(std::convert::TryInto::try_into)?;

        Ok(detailed_invoice)
    }

    /// Shared logic for update and preview. Also returns the override marker to persist: `None`
    /// leaves it untouched, `Some(None)` clears it, `Some(Some(ts))` stamps it.
    async fn prepare_invoice_update(
        &self,
        conn: &mut PgConn,
        mut invoice: Invoice,
        tenant_id: TenantId,
        params: UpdateInvoiceParams,
    ) -> StoreResult<(Invoice, Option<Option<NaiveDateTime>>)> {
        // Credit-priced lines only estimate what the unit wallets cover until finalization draws
        // them. An edited draft finalizes from its stored lines and is never recomputed, so a
        // wallet that moved since would block it for good, or bill other than what was previewed.
        if (params.line_items.is_some() || params.discount.is_some())
            && invoice
                .line_items
                .iter()
                .any(|line| line.credit_unit.is_some())
        {
            bail!(StoreError::InvalidArgument(
                "A draft billing usage priced in credits cannot have its lines or discount edited: \
                 what the credit balance covers is only settled when it is finalized. Change the \
                 subscription instead, or issue a credit note once it is finalized"
                    .into(),
            ));
        }

        let currency = meteroid_money::iso::find(&invoice.currency)
            .ok_or_else(|| StoreError::InvalidArgument("Invalid currency".into()))?;

        // One load on the caller's connection, reused for the overlay, the inline-edit base and
        // the divergence check. Taking a second pool connection here would risk starving the pool
        // while this transaction holds the invoice under SELECT ... FOR UPDATE.
        let live_customer: Option<InlineCustomer> =
            if invoice.tracks_live_customer() || params.customer_details.is_some() {
                let customer: crate::domain::Customer =
                    CustomerRow::find_by_id(conn, &invoice.customer_id, &tenant_id)
                        .await
                        .map_err(Into::<error_stack::Report<StoreError>>::into)
                        .and_then(std::convert::TryInto::try_into)?;
                Some(customer.into())
            } else {
                None
            };

        let customer_details_overridden_at = match params.customer_details {
            None => {
                // Keep the stored cache in step with what this draft renders, so a save can't
                // persist a snapshot older than the preview the operator just approved.
                if let Some(live) = live_customer.clone() {
                    invoice.overlay_customer_details(live);
                }
                None
            }
            Some(CustomerDetailsUpdate::RefreshFromCustomer) => {
                if let Some(live) = live_customer.clone() {
                    invoice.customer_details = live;
                }
                Some(None)
            }
            Some(CustomerDetailsUpdate::InlineEdit {
                name,
                billing_address,
                vat_number,
            }) => {
                // Base the edit on what this invoice would render *now*, not on the stored
                // snapshot: on a tracking draft the stored copy may be stale, and freezing it
                // would silently pin an outdated language or email alongside the edited name.
                let mut details = match (invoice.tracks_live_customer(), live_customer.clone()) {
                    (true, Some(live)) => live,
                    _ => invoice.customer_details.clone(),
                };

                if let Some(n) = name {
                    details.name = n;
                }
                details.billing_address = billing_address;
                details.vat_number = vat_number;
                details.snapshot_at = common_domain::clock::now().naive_utc();
                invoice.customer_details = details;

                // An override exists only where the result actually diverges from the customer
                // record; echoing back the displayed values must leave the draft tracking.
                match &live_customer {
                    Some(live) if invoice.customer_details.matches_record(live) => Some(None),
                    _ => Some(Some(common_domain::clock::now().naive_utc())),
                }
            }
        };

        if let Some(new_invoicing_entity_id) = params.invoicing_entity_id {
            // On the caller's connection, for the same reason as the customer load above.
            let invoicing_entity = InvoicingEntityRow::get_invoicing_entity_by_id_and_tenant(
                conn,
                new_invoicing_entity_id,
                tenant_id,
            )
            .await
            .map_err(Into::<error_stack::Report<StoreError>>::into)?;

            invoice.seller_details = crate::domain::InvoicingEntity::from(invoicing_entity).into();
            invoice.invoicing_entity_id = new_invoicing_entity_id;
        }

        let mut lines = if let Some(new_line_items) = params.line_items {
            // Build a map of existing line items by local_id for quick lookup
            let existing_lines_map: HashMap<String, LineItem> = invoice
                .line_items
                .iter()
                .map(|line| (line.local_id.clone(), line.clone()))
                .collect();

            let mut lines = vec![];
            for line_params in &new_line_items {
                let local_id = if let Some(ref existing_id) = line_params.id {
                    existing_id.clone()
                } else {
                    LocalId::generate_for(IdType::Other)
                };

                let existing_line = line_params
                    .id
                    .as_ref()
                    .and_then(|id| existing_lines_map.get(id));

                // Recompute each incoming subline total from qty * unit_price using the
                // invoice currency precision — backend is authoritative.
                let mut incoming_sublines = line_params.sub_lines.clone();
                for sl in &mut incoming_sublines {
                    sl.total = (sl.quantity * sl.unit_price)
                        .to_subunit_opt(currency.exponent as u8)
                        .unwrap_or(0);
                }

                let has_sublines = !incoming_sublines.is_empty();

                let (quantity, unit_price, amount_subtotal, sub_lines) = if has_sublines {
                    let subtotal = incoming_sublines.iter().map(|s| s.total).sum();
                    (None, None, subtotal, incoming_sublines)
                } else {
                    let q = line_params.quantity.unwrap_or(rust_decimal::Decimal::ZERO);
                    let p = line_params
                        .unit_price
                        .unwrap_or(rust_decimal::Decimal::ZERO);
                    let amount = (q * p).to_subunit_opt(currency.exponent as u8).unwrap_or(0);
                    (line_params.quantity, line_params.unit_price, amount, vec![])
                };

                let item = LineItem {
                    local_id,
                    name: line_params.name.clone(),
                    charge_type: existing_line.and_then(|l| l.charge_type),
                    tax_rate: line_params.tax_rate,
                    tax_details: vec![],
                    amount_subtotal,
                    taxable_amount: amount_subtotal,
                    tax_amount: 0,
                    amount_total: 0,
                    quantity,
                    unit_price,
                    start_date: line_params.start_date,
                    end_date: line_params.end_date,
                    sub_lines,
                    is_prorated: existing_line.map(|l| l.is_prorated).unwrap_or(false),
                    price_component_id: existing_line.and_then(|l| l.price_component_id),
                    sub_component_id: existing_line.and_then(|l| l.sub_component_id),
                    sub_add_on_id: existing_line.and_then(|l| l.sub_add_on_id),
                    product_id: existing_line.and_then(|l| l.product_id),
                    metric_id: existing_line.and_then(|l| l.metric_id),
                    description: line_params.description.clone(),
                    group_by_dimensions: existing_line.and_then(|l| l.group_by_dimensions.clone()),
                    kind: existing_line
                        .map(|l| l.kind)
                        .unwrap_or(crate::domain::LineItemKind::Regular),
                    // The credit a line sells follows an edit of its charge pro rata: a pack edited
                    // from $500 to $1 must not keep funding $500.
                    credit_grant_amount: existing_line
                        .and_then(|l| Some((l.credit_grant_amount?, l.amount_subtotal)))
                        .map(|(grant, old_amount)| {
                            scale_credit_grant(grant, old_amount, amount_subtotal)
                        })
                        .transpose()?,
                    credit_grant_expiry: existing_line.and_then(|l| l.credit_grant_expiry),
                    credit_grant_included: existing_line.is_some_and(|l| l.credit_grant_included),
                    taxed_at_purchase: existing_line.is_some_and(|l| l.taxed_at_purchase),
                    tax_inclusive_quote: None,
                    credit_unit: None,
                    // An edited pack keeps funding the unit it sold.
                    credit_grant_unit: existing_line.and_then(|l| l.credit_grant_unit.clone()),
                    credit_grant_rollover: existing_line.and_then(|l| l.credit_grant_rollover),
                    promotional_credit: None,
                    // An edit never changes which subscription the line bills.
                    subscription_id: existing_line.and_then(|l| l.subscription_id),
                    usage_ends_at: None,
                };

                lines.push(item);
            }
            lines
        } else if invoice.tax_inclusive {
            // Stored lines are net; they are re-taxed below from the figures they were
            // quoted in, the same figures an edited line comes in with.
            invoice
                .line_items
                .iter()
                .cloned()
                .map(LineItem::into_tax_inclusive_quote)
                .collect()
        } else {
            invoice.line_items.clone()
        };
        // Promotional credit is estimated again below, on the edited lines.
        crate::services::credit::promotional::restore_before_tax(&mut lines);

        let discount = if let Some(discount_str) = params.discount {
            let decimal = discount_str.parse::<Decimal>().map_err(|e| {
                StoreError::InvalidArgument(format!("Invalid discount value: {}", e))
            })?;
            let money = meteroid_money::Money::from_decimal(decimal, currency);
            money
                .amount()
                .to_subunit_opt(currency.exponent as u8)
                .ok_or_else(|| {
                    StoreError::InvalidArgument("Decimal to subunit conversion failed".into())
                })?
        } else if invoice.tax_inclusive {
            crate::domain::quoted_discount(&invoice.line_items)
        } else {
            invoice.discount
        };

        if discount > 0 {
            lines = crate::services::invoice_lines::discount::distribute_discount(
                lines,
                discount as u64,
            );
        }

        // On a tax-inclusive invoice the editor works in quoted figures throughout: line
        // prices and the discount come in tax-included, and the lines leave net.
        let (mut tax_breakdown, discount_tax) =
            apply_tax_rates_to_priced_lines(&mut lines, invoice.tax_inclusive);
        let discount = discount - discount_tax;
        // This path recomputes amounts by rate, not the tax engine, so what the engine
        // found is unchanged: carry it over from the invoice's existing breakdown.
        carry_over_tax_findings(&invoice.tax_breakdown, &mut tax_breakdown);

        let mut subtotal: i64 = lines.iter().map(|line| line.amount_subtotal).sum();
        let mut total_tax_amount: i64 = lines.iter().map(|line| line.tax_amount).sum();
        let mut total =
            crate::services::invoice_lines::invoice_lines::totals_of(&lines, discount).total;

        let currency_code = invoice.currency.clone();
        let scope = crate::services::credit::value_application::EstimateContext {
            tenant_id,
            customer_id: invoice.customer_id,
            currency: &currency_code,
            invoice_id: Some(invoice.id),
            subscription_id: invoice.subscription_id,
            periods: crate::services::credit::value_application::billed_periods(
                invoice.subscription_id,
                &lines,
            ),
            at: common_domain::clock::now(),
            book: None,
        };
        let mut promotional_credits = 0;
        let mut repriced_recurring = 0;
        self.reprice_promotional_estimate(
            conn,
            &scope,
            &mut crate::services::credit::promotional::TaxedDocument {
                lines: &mut lines,
                subtotal: &mut subtotal,
                subtotal_recurring: &mut repriced_recurring,
                discount,
                tax_inclusive: invoice.tax_inclusive,
                tax_amount: &mut total_tax_amount,
                tax_breakdown: &mut tax_breakdown,
                promotional_credits: &mut promotional_credits,
                total: &mut total,
            },
        )
        .await?;

        // Paid and credited so far, both carried by the stored amount due.
        let settled = invoice.total - invoice.amount_due;
        let totals = crate::services::invoice_lines::invoice_lines::totals_of(&lines, discount);

        invoice.line_items = lines;
        invoice.subtotal = totals.subtotal;
        invoice.subtotal_recurring = totals.subtotal_recurring;
        invoice.tax_amount = totals.tax;
        invoice.total = totals.total;
        invoice.amount_due = totals.amount_due(settled, 0);
        invoice.discount = totals.discount;
        invoice.promotional_credits = totals.promotional;
        invoice.tax_breakdown = tax_breakdown;

        if let Some(annexes) = params.annexes {
            // Only a correction carries a hand-editable appendix. Every other draft re-derives it
            // from the event stream on the next refresh or at finalization, which would silently
            // overwrite the edit.
            if !invoice.is_correction() {
                bail!(StoreError::InvalidArgument(
                    "Annexes can only be edited on a correction invoice".into(),
                ));
            }
            // The renderer lays each annex out as a fixed `columns.len()` grid and flattens every
            // row into it, so a row of the wrong width silently shifts every cell after it. Every
            // derived row — data and subtotals alike — is exactly that wide; hand-edited ones must
            // be too.
            for annex in &annexes {
                let width = annex.table.columns.len();
                if let Some(row) = annex
                    .table
                    .groups
                    .iter()
                    .flat_map(|group| &group.rows)
                    .find(|row| row.cells.len() != width)
                {
                    bail!(StoreError::InvalidArgument(format!(
                        "Annex '{}' has a row of {} cells but {} columns",
                        annex.title,
                        row.cells.len(),
                        width
                    )));
                }
            }

            invoice.annexes = annexes;
        }

        if let Some(memo) = params.memo {
            invoice.memo = memo;
        }
        if let Some(reference) = params.reference {
            invoice.reference = reference;
        }
        if let Some(purchase_order) = params.purchase_order {
            invoice.purchase_order = purchase_order;
        }
        if let Some(due_date) = params.due_date {
            invoice.due_at = due_date.map(|d| d.and_time(NaiveTime::MIN));
        }

        Ok((invoice, customer_details_overridden_at))
    }
}

/// Scales the credit a line sells to its edited charge, rounding toward zero.
fn scale_credit_grant(grant: i64, old_amount: i64, new_amount: i64) -> StoreResult<i64> {
    if old_amount == new_amount {
        return Ok(grant);
    }
    if grant <= 0 || old_amount <= 0 || new_amount <= 0 {
        return Ok(0);
    }
    let scaled = i128::from(grant) * i128::from(new_amount) / i128::from(old_amount);
    i64::try_from(scaled).map_err(|_| {
        error_stack::Report::new(StoreError::InvalidArgument(
            "the credit this line sells exceeds the supported amount".to_owned(),
        ))
    })
}

#[cfg(test)]
mod credit_grant_tests {
    use super::scale_credit_grant;

    #[test]
    fn an_edited_charge_scales_the_credit_it_sells() {
        assert_eq!(scale_credit_grant(50_000, 50_000, 50_000).unwrap(), 50_000);
        assert_eq!(scale_credit_grant(60_000, 50_000, 100).unwrap(), 120);
        assert_eq!(scale_credit_grant(1_000, 3, 1).unwrap(), 333);
        assert_eq!(scale_credit_grant(1_000, 1_000, 0).unwrap(), 0);
        assert!(scale_credit_grant(i64::MAX, 1, 2).is_err());
    }
}
