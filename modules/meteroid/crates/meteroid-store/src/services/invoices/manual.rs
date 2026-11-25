//! A one-off invoice written by hand (the dashboard's, API v2's): its lines priced as typed, the
//! discount spread over them, taxed by their rates, its totals; a draft, issued with the
//! finalization every draft goes through.
use crate::StoreResult;
use crate::domain::invoice_lines::SubLineItem;
use crate::domain::{
    Address, ChargeType, InlineCustomer, InvoiceAnnex, InvoiceNew, InvoicingEntity, LineItem,
};
use crate::errors::StoreError;
use crate::repositories::customers::CustomersInterfaceAuto;
use crate::repositories::invoices::apply_tax_rates_to_priced_lines;
use crate::repositories::invoicing_entities::InvoicingEntityInterfaceAuto;
use crate::services::Services;
use crate::utils::local_id::{IdType, LocalId};
use chrono::{NaiveDate, NaiveTime};
use common_domain::ids::{CustomerId, TenantId};
use common_utils::decimals::ToSubunit;
use error_stack::Report;
use rust_decimal::Decimal;

/// A line typed on a one-off invoice: `quantity × unit_price`, or the sum of its sub-lines.
#[derive(Debug, Clone)]
pub struct ManualInvoiceLine {
    pub name: String,
    pub description: Option<String>,
    /// None for a line priced by its sub-lines.
    pub quantity: Option<Decimal>,
    pub unit_price: Option<Decimal>,
    /// As a fraction: 0.2 for 20%.
    pub tax_rate: Decimal,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    /// Each priced `quantity × unit_price`, whatever total it comes with.
    pub sub_lines: Vec<SubLineItem>,
}

/// The customer as the invoice is addressed to it, in place of the customer's own details.
#[derive(Debug, Clone)]
pub struct ManualInvoiceCustomer {
    pub name: String,
    pub billing_address: Option<Address>,
    pub vat_number: Option<String>,
    pub email: Option<String>,
}

/// A one-off invoice to write as a draft.
#[derive(Debug, Clone)]
pub struct ManualInvoice {
    pub customer_id: CustomerId,
    /// ISO 4217.
    pub currency: String,
    pub invoice_date: NaiveDate,
    pub due_date: Option<NaiveDate>,
    pub lines: Vec<ManualInvoiceLine>,
    /// In minor units, off the lines' positive subtotals, before tax.
    pub discount: Option<i64>,
    /// The typed prices include tax: it is carved out of them.
    pub tax_inclusive: bool,
    pub reference: Option<String>,
    pub purchase_order: Option<String>,
    pub memo: Option<String>,
    pub customer_details: Option<ManualInvoiceCustomer>,
    pub annexes: Vec<InvoiceAnnex>,
    /// Null: not managed.
    pub custom_properties: serde_json::Value,
}

fn invalid(message: impl Into<String>) -> Report<StoreError> {
    Report::new(StoreError::InvalidArgument(message.into()))
}

impl Services {
    /// The draft `invoice` is, priced as written, ready to insert, with the invoicing entity it is
    /// issued by: the customer's.
    pub(in crate::services) async fn manual_invoice_new(
        &self,
        tenant_id: TenantId,
        invoice: ManualInvoice,
    ) -> StoreResult<(InvoiceNew, InvoicingEntity)> {
        let currency = meteroid_money::iso::find(&invoice.currency)
            .ok_or_else(|| invalid("Invalid currency"))?;
        let precision = currency.exponent as u8;

        let customer = self
            .store
            .find_customer_by_id(invoice.customer_id, tenant_id)
            .await?;

        // The lines priced before the discount; the backend is authoritative on every total.
        let mut lines = Vec::with_capacity(invoice.lines.len());
        for line in invoice.lines {
            let mut sub_lines = line.sub_lines;
            for sub_line in &mut sub_lines {
                sub_line.total = (sub_line.quantity * sub_line.unit_price)
                    .to_subunit_opt(precision)
                    .unwrap_or(0);
            }
            let amount_subtotal = if !sub_lines.is_empty() {
                sub_lines.iter().map(|sub_line| sub_line.total).sum()
            } else if let (Some(quantity), Some(price)) = (line.quantity, line.unit_price) {
                (quantity * price).to_subunit_opt(precision).unwrap_or(0)
            } else {
                0
            };
            lines.push(LineItem {
                local_id: LocalId::generate_for(IdType::Other),
                name: line.name,
                // Manual one-off invoice line with no subscription component.
                charge_type: Some(ChargeType::OneTime),
                tax_rate: line.tax_rate,
                tax_details: vec![], // Will be calculated after discount
                amount_subtotal,
                taxable_amount: amount_subtotal, // Will be updated by distribute_discount
                tax_amount: 0,                   // Will be calculated after discount
                amount_total: 0,                 // Will be calculated after discount
                quantity: line.quantity,
                unit_price: line.unit_price,
                start_date: line.start_date,
                end_date: line.end_date,
                sub_lines,
                is_prorated: false,
                price_component_id: None,
                sub_component_id: None,
                sub_add_on_id: None,
                product_id: None,
                metric_id: None,
                description: line.description,
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
                usage_breakdown: None,
            });
        }

        // A discount comes off the lines' positive subtotals, never more and never below zero (I25).
        let discount = invoice.discount;
        let discountable: i64 = lines.iter().map(|line| line.amount_subtotal.max(0)).sum();
        if discount.is_some_and(|discount| discount < 0 || discount > discountable) {
            return Err(invalid(
                "The discount must be between zero and the invoice's subtotal",
            ));
        }
        if let Some(discount_amount) = discount {
            lines = crate::services::invoice_lines::discount::distribute_discount(
                lines,
                discount_amount as u64,
            );
        }

        // Tax after discount, rounded once per rate over the whole base like the tax engine.
        // When the operator entered tax-included prices, the tax is carved out of them and the
        // lines come back net, so the total stays the figure that was typed.
        let tax_inclusive = invoice.tax_inclusive;
        let (tax_breakdown, discount_tax) =
            apply_tax_rates_to_priced_lines(&mut lines, tax_inclusive);

        let subtotal: i64 = lines.iter().map(|line| line.amount_subtotal).sum();
        let total_tax_amount: i64 = lines.iter().map(|line| line.tax_amount).sum();
        let total_taxable: i64 = lines.iter().map(|line| line.taxable_amount).sum();
        let total = total_taxable + total_tax_amount;
        // A manual draft has collected nothing and drawn no credit yet (R-087).
        let amount_due = billing_core::amount_due(total, 0, 0);

        let invoicing_entity = self
            .store
            .get_invoicing_entity(tenant_id, Some(customer.invoicing_entity_id))
            .await?;

        // The details typed in, else the customer's.
        let customer_details = match invoice.customer_details {
            Some(details) => InlineCustomer {
                id: invoice.customer_id,
                name: details.name,
                billing_address: details.billing_address,
                snapshot_at: common_clock::now().naive_utc(),
                vat_number: details.vat_number,
                legal_number: customer.legal_number.clone(),
                buyer_reference: customer.buyer_reference.clone(),
                email: details.email,
                alias: customer.alias,
                preferred_locales: customer.preferred_locales.clone(),
            },
            None => customer.into(),
        };

        let invoice_new = InvoiceNew {
            status: crate::domain::InvoiceStatusEnum::Draft,
            tenant_id,
            customer_id: invoice.customer_id,
            subscription_id: None,
            plan_name: None,
            plan_version_id: None,
            invoice_type: crate::domain::InvoiceType::OneOff,
            // Manual one-off: not a subscription period, so no cycle to pin.
            cycle_index: None,
            currency: invoice.currency,
            line_items: lines,
            coupons: vec![],
            data_updated_at: None,
            invoice_date: invoice.invoice_date,
            billing_period_start: invoice.invoice_date,
            tax_inclusive,
            finalized_at: None,
            total,
            amount_due,
            applied_credits: 0, // manual invoices don't use customer credits ?
            promotional_credits: 0,
            subtotal,
            subtotal_recurring: 0, // no recurring items in manual invoices
            tax_amount: total_tax_amount,
            net_terms: invoicing_entity.net_terms, // todo derive from due_date?
            discount: discount.unwrap_or(0) - discount_tax,
            invoice_number: "draft".to_string(),
            reference: invoice.reference,
            purchase_order: invoice.purchase_order,
            memo: invoice.memo,
            due_at: invoice.due_date.map(|d| d.and_time(NaiveTime::MIN)),
            customer_details,
            seller_details: invoicing_entity.clone().into(),
            auto_advance: false,
            payment_status: crate::domain::InvoicePaymentStatus::Unpaid,
            tax_breakdown,
            manual: true,
            invoicing_entity_id: invoicing_entity.id,
            parent_invoice_id: None,
            consolidated_into_invoice_id: None,
            annexes: invoice.annexes,
            custom_properties: invoice.custom_properties,
        };
        Ok((invoice_new, invoicing_entity))
    }
}
