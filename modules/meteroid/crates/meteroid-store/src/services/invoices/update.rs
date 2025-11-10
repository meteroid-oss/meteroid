use crate::StoreResult;
use crate::domain::{DetailedInvoice, Invoice, LineItem, TaxBreakdownItem, UpdateInvoiceParams};
use crate::errors::StoreError;
use crate::repositories::invoices::compute_tax_breakdown;
use crate::repositories::invoicing_entities::InvoicingEntityInterface;
use crate::repositories::{CustomersInterface, InvoiceInterface};
use crate::services::Services;
use crate::utils::local_id::{IdType, LocalId};
use chrono::NaiveTime;
use common_domain::ids::{InvoiceId, TenantId};
use common_utils::decimals::ToSubunit;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::PgConn;
use diesel_models::invoices::{InvoiceRow, InvoiceRowPatch};
use error_stack::bail;
use rust_decimal::Decimal;
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

        // Apply the same update logic but in-memory only
        let updated_invoice = self
            .prepare_invoice_update(invoice, tenant_id, params)
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
        let invoice: Invoice = invoice_lock.invoice.try_into()?;

        if !invoice.can_edit() {
            bail!(StoreError::InvalidArgument(
                "Can only edit draft invoices".into(),
            ));
        }

        let invoice = self
            .prepare_invoice_update(invoice, tenant_id, params)
            .await?;

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
            tax_breakdown: serde_json::to_value(&invoice.tax_breakdown).map_err(|e| {
                StoreError::SerdeError("Failed to serialize tax_breakdown".to_string(), e)
            })?,
        };

        patch.update(id, tenant_id, conn).await?;

        let detailed_invoice = InvoiceRow::find_detailed_by_id(conn, tenant_id, id)
            .await
            .map_err(Into::into)
            .and_then(std::convert::TryInto::try_into)?;

        Ok(detailed_invoice)
    }

    /// shared logic for update and preview
    async fn prepare_invoice_update(
        &self,
        mut invoice: Invoice,
        tenant_id: TenantId,
        params: UpdateInvoiceParams,
    ) -> StoreResult<Invoice> {
        let currency = rusty_money::iso::find(&invoice.currency)
            .ok_or_else(|| StoreError::InvalidArgument("Invalid currency".into()))?;

        if let Some(customer_update) = params.customer_details {
            invoice.customer_details = match customer_update {
                CustomerDetailsUpdate::RefreshFromCustomer => {
                    let customer = self
                        .store
                        .find_customer_by_id(invoice.customer_id, tenant_id)
                        .await?;
                    customer.into()
                }
                CustomerDetailsUpdate::InlineEdit {
                    name,
                    billing_address,
                    vat_number,
                } => {
                    let mut details = invoice.customer_details.clone();
                    if let Some(n) = name {
                        details.name = n;
                    }
                    details.billing_address = billing_address;
                    details.vat_number = vat_number;
                    details.snapshot_at = chrono::Utc::now().naive_utc();
                    details
                }
            };
        }

        if let Some(new_invoicing_entity_id) = params.invoicing_entity_id {
            let invoicing_entity = self
                .store
                .get_invoicing_entity(tenant_id, Some(new_invoicing_entity_id))
                .await?;
            invoice.seller_details = invoicing_entity.into();
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

                // Check if line has sublines (either from params or existing)
                let has_sublines = !line_params.sub_lines.is_empty()
                    || existing_line
                        .map(|line| !line.sub_lines.is_empty())
                        .unwrap_or(false);

                // For lines with sublines, preserve the original values
                // Otherwise, use the updated values from params
                let (quantity, unit_price, amount_subtotal) = if has_sublines {
                    if let Some(existing) = existing_line {
                        (
                            existing.quantity,
                            existing.unit_price,
                            existing.amount_subtotal,
                        )
                    } else {
                        // Calculate from sublines passed in params
                        let subtotal = line_params.sub_lines.iter().map(|s| s.total).sum();
                        (None, None, subtotal)
                    }
                } else {
                    let q = line_params.quantity.unwrap_or(rust_decimal::Decimal::ZERO);
                    let p = line_params
                        .unit_price
                        .unwrap_or(rust_decimal::Decimal::ZERO);
                    let amount = (q * p).to_subunit_opt(currency.exponent as u8).unwrap_or(0);
                    (line_params.quantity, line_params.unit_price, amount)
                };

                let item = LineItem {
                    local_id,
                    name: line_params.name.clone(),
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
                    sub_lines: line_params.sub_lines.clone(),
                    is_prorated: existing_line.map(|l| l.is_prorated).unwrap_or(false),
                    price_component_id: existing_line.and_then(|l| l.price_component_id),
                    sub_component_id: existing_line.and_then(|l| l.sub_component_id),
                    sub_add_on_id: existing_line.and_then(|l| l.sub_add_on_id),
                    product_id: existing_line.and_then(|l| l.product_id),
                    metric_id: existing_line.and_then(|l| l.metric_id),
                    description: line_params.description.clone(),
                    group_by_dimensions: existing_line.and_then(|l| l.group_by_dimensions.clone()),
                };

                lines.push(item);
            }
            lines
        } else {
            invoice.line_items.clone()
        };

        let discount = if let Some(discount_str) = params.discount {
            let decimal = discount_str.parse::<Decimal>().map_err(|e| {
                StoreError::InvalidArgument(format!("Invalid discount value: {}", e))
            })?;
            let money = rusty_money::Money::from_decimal(decimal, currency);
            money
                .amount()
                .to_subunit_opt(currency.exponent as u8)
                .ok_or_else(|| {
                    StoreError::InvalidArgument("Decimal to subunit conversion failed".into())
                })?
        } else {
            invoice.discount
        };

        if discount > 0 {
            lines = crate::services::invoice_lines::discount::distribute_discount(
                lines,
                discount as u64,
            );
        }

        let tax_breakdown = apply_tax_rates_to_lines(&mut lines, currency);

        let subtotal: i64 = lines.iter().map(|line| line.amount_subtotal).sum();
        let total_tax_amount: i64 = lines.iter().map(|line| line.tax_amount).sum();
        let total_taxable: i64 = lines.iter().map(|line| line.taxable_amount).sum();
        let total = total_taxable + total_tax_amount;

        let already_paid = invoice.total - invoice.amount_due;
        let amount_due = total - already_paid;

        let subtotal_recurring = lines
            .iter()
            .filter(|x| x.metric_id.is_none())
            .fold(0, |acc, x| acc + x.amount_subtotal);

        invoice.line_items = lines;
        invoice.subtotal = subtotal;
        invoice.subtotal_recurring = subtotal_recurring;
        invoice.tax_amount = total_tax_amount;
        invoice.total = total;
        invoice.amount_due = amount_due;
        invoice.discount = discount;
        invoice.tax_breakdown = tax_breakdown;

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

        Ok(invoice)
    }
}

fn apply_tax_rates_to_lines(
    lines: &mut [LineItem],
    currency: &rusty_money::iso::Currency,
) -> Vec<TaxBreakdownItem> {
    // Calculate tax amounts for each line based on tax_rate
    for line in lines.iter_mut() {
        let taxable_amount_decimal =
            *rusty_money::Money::from_minor(line.taxable_amount, currency).amount();

        let tax_amount_decimal = taxable_amount_decimal * line.tax_rate;
        line.tax_amount = tax_amount_decimal
            .to_subunit_opt(currency.exponent as u8)
            .unwrap_or(0);
        line.amount_total = line.taxable_amount + line.tax_amount;
    }

    compute_tax_breakdown(lines)
}
