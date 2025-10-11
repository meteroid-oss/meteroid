use crate::StoreResult;
use crate::constants::{Currencies, Currency};
use crate::domain::{
    ComponentPeriods, CouponLineItem, Customer, Invoice, InvoicingEntity, LineItem,
    SubscriptionDetails, SubscriptionFeeInterface, TaxBreakdownItem, TaxResolverEnum,
};
use chrono::NaiveDate;
use itertools::Itertools;
use std::cmp::min;

use crate::errors::StoreError;
use crate::repositories::accounting::AccountingInterface;
use crate::services::Services;
use crate::services::invoice_lines::discount::calculate_coupons_discount;
use crate::store::PgConn;
use crate::utils::periods::calculate_component_period_for_invoice_date;
use common_utils::integers::ToNonNegativeU64;
use error_stack::{Report, ResultExt};
use meteroid_tax::{ManualTaxEngine, MeteroidTaxEngine, TaxDetails, TaxEngine};

impl Services {}

#[derive(Debug)]
pub struct ComputedInvoiceContent {
    pub invoice_lines: Vec<LineItem>,
    pub subtotal: i64, // before discounts, coupons, credits, taxes
    pub applied_coupons: Vec<CouponLineItem>,
    pub discount: i64,
    pub tax_breakdown: Vec<TaxBreakdownItem>,
    pub applied_credits: i64,

    pub total: i64,
    pub tax_amount: i64,
    pub amount_due: i64,
    //
    pub subtotal_recurring: i64,
}

impl Services {
    pub async fn compute_invoice(
        &self,
        conn: &mut PgConn,
        invoice_date: &NaiveDate,
        subscription_details: &SubscriptionDetails,
        prepaid_amount: Option<u64>,
        invoice: Option<&Invoice>, // for refresh purposes
    ) -> StoreResult<ComputedInvoiceContent> {
        let is_usage_based_line = |line: &LineItem| {
            line.metric_id.is_some()
                && (line.sub_component_id.is_some() || line.sub_add_on_id.is_some())
        };

        // do not recompute if invoice has no usage-based lines
        if let Some(invoice) = invoice
            && !invoice.line_items.iter().any(is_usage_based_line)
        {
            return Ok(ComputedInvoiceContent {
                invoice_lines: invoice.line_items.clone(),
                subtotal: invoice.subtotal,
                applied_coupons: invoice.coupons.clone(),
                discount: invoice.discount,
                tax_breakdown: invoice.tax_breakdown.clone(),
                applied_credits: invoice.applied_credits,
                total: invoice.total,
                amount_due: invoice.amount_due,
                subtotal_recurring: invoice.subtotal_recurring,
                tax_amount: invoice.tax_amount,
            });
        }

        let billing_start_date = subscription_details
            .subscription
            .billing_start_date
            // TODO should we return empty ?
            .ok_or(Report::new(StoreError::BillingError))
            .attach("No billing_start_date is present")?;

        let currency = Currencies::resolve_currency(&subscription_details.subscription.currency)
            .ok_or(Report::new(StoreError::ValueNotFound(format!(
                "Currency {} not found",
                subscription_details.subscription.currency
            ))))?;

        let cycle_index = subscription_details.subscription.cycle_index.unwrap_or(0);

        let invoice_date = *invoice_date;

        let price_components_lines = self
            .process_fee_records(
                conn,
                subscription_details,
                &subscription_details.price_components,
                invoice_date,
                billing_start_date,
                cycle_index,
                currency,
            )
            .await?;

        let add_ons_lines = self
            .process_fee_records(
                conn,
                subscription_details,
                &subscription_details.add_ons,
                invoice_date,
                billing_start_date,
                cycle_index,
                currency,
            )
            .await?;

        let invoice_lines = price_components_lines
            .into_iter()
            .chain(add_ons_lines)
            .collect_vec();

        // refresh only usage-based line items if invoice is set
        let invoice_lines = if let Some(invoice) = invoice {
            invoice
                .line_items
                .iter()
                .map(|invoice_line| {
                    if is_usage_based_line(invoice_line)
                        && let Some(computed_line) = invoice_lines.iter().find(|line| {
                            line.metric_id == invoice_line.metric_id
                                && (line.sub_component_id == invoice_line.sub_component_id
                                    || line.sub_add_on_id == invoice_line.sub_add_on_id)
                                && line.group_by_dimensions == invoice_line.group_by_dimensions
                        })
                    {
                        return LineItem {
                            amount_subtotal: computed_line.amount_subtotal,
                            quantity: computed_line.quantity,
                            unit_price: computed_line.unit_price,
                            sub_lines: computed_line.sub_lines.clone(),
                            ..invoice_line.clone()
                        };
                    }
                    invoice_line.clone()
                })
                .collect()
        } else {
            invoice_lines
        };

        let subtotal = invoice_lines
            .iter()
            .fold(0, |acc, x| acc + x.amount_subtotal);

        let coupons_discount = calculate_coupons_discount(
            subtotal,
            &subscription_details.subscription.currency,
            &subscription_details.applied_coupons,
        );

        let discount_total = coupons_discount.discount_subunit.to_non_negative_u64(); // TODO we need to define the rules for negatives, same below with taxes & subtotal
        let invoice_lines = super::discount::distribute_discount(invoice_lines, discount_total);

        // we add taxes
        let (invoice_lines, breakdown) = self
            .process_invoice_lines_taxes(
                conn,
                invoice_lines,
                &subscription_details.invoicing_entity,
                &subscription_details.customer,
                subscription_details.subscription.currency.clone(),
                &invoice_date,
            )
            .await?;

        let subtotal = invoice_lines
            .iter()
            .fold(0, |acc, x| acc + x.amount_subtotal)
            .to_non_negative_u64();

        let subtotal_with_discounts = subtotal - discount_total;
        let tax_amount = invoice_lines
            .iter()
            .fold(0, |acc, x| acc + x.tax_amount)
            .to_non_negative_u64();

        let total = subtotal_with_discounts + tax_amount;
        let applied_credits = min(
            total,
            subscription_details
                .customer
                .balance_value_cents
                .to_non_negative_u64(),
        );
        let already_paid = prepaid_amount.unwrap_or(0);
        let amount_due = total - already_paid - applied_credits;
        let subtotal_recurring = invoice_lines
            .iter()
            .filter(|x| x.metric_id.is_none())
            .fold(0, |acc, x| acc + x.amount_subtotal)
            .to_non_negative_u64();

        Ok(ComputedInvoiceContent {
            invoice_lines,
            subtotal: subtotal as i64,
            applied_coupons: coupons_discount.applied_coupons,
            discount: discount_total as i64,
            tax_breakdown: breakdown,
            applied_credits: applied_credits as i64,
            total: total as i64,
            amount_due: amount_due as i64,
            subtotal_recurring: subtotal_recurring as i64,
            tax_amount: tax_amount as i64,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn compute_oneoff_invoice(
        &self,
        conn: &mut PgConn,
        invoice_date: &NaiveDate,
        invoice_lines: Vec<LineItem>,
        invoicing_entity: &InvoicingEntity,
        customer: &Customer,
        currency: String,
        discount: Option<u64>,
        prepaid_amount: Option<u64>,
    ) -> StoreResult<ComputedInvoiceContent> {
        let discount_total = discount.unwrap_or(0);
        let invoice_lines = super::discount::distribute_discount(invoice_lines, discount_total);

        // we add taxes
        let (line_items, breakdown) = self
            .process_invoice_lines_taxes(
                conn,
                invoice_lines,
                invoicing_entity,
                customer,
                currency,
                invoice_date,
            )
            .await?;

        let subtotal = line_items
            .iter()
            .fold(0, |acc, x| acc + x.amount_subtotal)
            .to_non_negative_u64();

        let subtotal_with_discounts = subtotal - discount_total;
        let tax_amount = line_items
            .iter()
            .fold(0, |acc, x| acc + x.tax_amount)
            .to_non_negative_u64();

        let total = subtotal_with_discounts + tax_amount;
        let applied_credits = min(total, customer.balance_value_cents.to_non_negative_u64());
        let already_paid = prepaid_amount.unwrap_or(0);
        let amount_due = total - already_paid - applied_credits;
        let subtotal_recurring = line_items
            .iter()
            .filter(|x| x.metric_id.is_none())
            .fold(0, |acc, x| acc + x.amount_subtotal)
            .to_non_negative_u64();

        Ok(ComputedInvoiceContent {
            invoice_lines: line_items,
            subtotal: subtotal as i64,
            applied_coupons: Vec::new(),
            discount: discount_total as i64,
            tax_breakdown: breakdown,
            applied_credits: applied_credits as i64,
            total: total as i64,
            amount_due: amount_due as i64,
            subtotal_recurring: subtotal_recurring as i64,
            tax_amount: tax_amount as i64,
        })
    }

    async fn process_invoice_lines_taxes(
        &self,
        conn: &mut PgConn,
        invoice_lines: Vec<LineItem>,
        invoicing_entity: &InvoicingEntity,
        customer: &Customer,
        currency: String,
        invoice_date: &NaiveDate,
    ) -> StoreResult<(Vec<LineItem>, Vec<TaxBreakdownItem>)> {
        let customer_address = match &customer.billing_address {
            Some(address) => address.clone(),
            None => return Ok((invoice_lines.clone(), Vec::new())),
        };

        let tax_engine: Box<dyn TaxEngine + Send + Sync> = match invoicing_entity.tax_resolver {
            TaxResolverEnum::None => {
                return Ok((invoice_lines.clone(), Vec::new()));
            }
            TaxResolverEnum::Manual => Box::new(ManualTaxEngine {}),
            TaxResolverEnum::MeteroidEuVat => Box::new(MeteroidTaxEngine {}),
        };

        let customer = meteroid_tax::CustomerForTax {
            vat_number: customer.vat_number.clone(),
            vat_number_format_valid: customer.vat_number_format_valid,
            custom_tax_rate: customer.custom_tax_rate,
            tax_exempt: customer.is_tax_exempt,
            billing_address: customer_address.into(),
        };

        // we retrieve the custom tax rates for each line item
        let product_ids = invoice_lines
            .iter()
            .filter_map(|line| line.product_id)
            .collect::<Vec<_>>();

        let product_taxes = self
            .store
            .list_product_tax_configuration_by_product_ids_and_invoicing_entity_id(
                conn,
                invoicing_entity.tenant_id,
                product_ids,
                invoicing_entity.id,
            )
            .await?;

        let invoice_lines_for_tax: Vec<meteroid_tax::LineItemForTax> = invoice_lines
            .iter()
            .filter_map(|line| {
                if line.taxable_amount > 0 {
                    let total = line.taxable_amount.to_non_negative_u64();
                    let custom_tax = line
                        .product_id
                        .and_then(|p| product_taxes.iter().find(|tax| tax.product_id == p))
                        .and_then(|p| p.custom_tax.as_ref());

                    Some(meteroid_tax::LineItemForTax {
                        line_id: line.local_id.to_string(),
                        amount: total,
                        custom_tax: custom_tax.map(|t| t.clone().into()),
                    })
                } else {
                    // If the line amount is zero (or refund), we skip it
                    // TODO : consider whether we want to allow tax credits within an invoice, or if we restrict to using a separate credit note (prob cleaner)
                    None
                }
            })
            .collect();

        let res = tax_engine
            .calculate_line_items_tax(
                currency,
                customer,
                invoicing_entity.address().into(),
                invoice_lines_for_tax,
                *invoice_date,
            )
            .await
            .change_context(StoreError::TaxError)?;

        let mut updated_invoice_lines = invoice_lines.clone();

        for line in &mut updated_invoice_lines {
            // we get the matching taxed line
            if let Some(taxed_line) = res.line_items.iter().find(|l| l.line_id == line.local_id) {
                // we update the line with the tax details

                if let TaxDetails::Tax {
                    tax_rate,
                    tax_amount,
                    ..
                } = taxed_line.tax_details.clone()
                {
                    line.tax_amount = tax_amount as i64;
                    line.tax_rate = tax_rate;
                } else {
                    line.tax_amount = 0;
                    line.tax_rate = rust_decimal::Decimal::ZERO;
                }
                line.taxable_amount = taxed_line.pre_tax_amount as i64;
            } else {
                // no tax details found
                line.tax_rate = rust_decimal::Decimal::ZERO;
            }
        }

        let breakdown = res
            .breakdown
            .into_iter()
            .map(std::convert::Into::into)
            .collect::<Vec<_>>();

        Ok((updated_invoice_lines, breakdown))
    }

    #[allow(clippy::too_many_arguments)]
    async fn process_fee_records<T: SubscriptionFeeInterface>(
        &self,
        conn: &mut PgConn,
        subscription_details: &SubscriptionDetails,
        fee_records: &[T],
        invoice_date: NaiveDate,
        billing_start_or_resume_date: NaiveDate,
        cycle_index: u32,
        currency: &Currency,
    ) -> StoreResult<Vec<LineItem>> {
        let component_groups = fee_records
            .iter()
            .into_group_map_by(|c| c.period_ref().clone());

        // TODO case when invoiced early via threshold (that's for usage-based only)
        // can be quite easy => we need some last_invoice_threshold date in the subscription, to reduce the usage periods if that date is within the period

        let component_period_components: Vec<(ComponentPeriods, Vec<&T>)> = component_groups
            .into_iter()
            .filter_map(|(billing_period, components)| {
                // we calculate the periods range, for each billing_period. There can be advance, arrears, or both
                let period = calculate_component_period_for_invoice_date(
                    invoice_date,
                    &subscription_details.subscription.period,
                    &billing_period,
                    billing_start_or_resume_date,
                    cycle_index,
                    u32::from(subscription_details.subscription.billing_day_anchor),
                    subscription_details
                        .subscription
                        .current_period_end
                        .is_none()
                        && !subscription_details.subscription.pending_checkout,
                );

                // if period is None - the components are not relevant for this invoice
                period.map(|period| (period, components))
            })
            .collect();

        // we can now compute all the components for each period
        let mut invoice_lines = Vec::new();
        for (period, components) in component_period_components {
            for component in components {
                let lines = self
                    .compute_component(
                        conn,
                        subscription_details,
                        component,
                        period.clone(),
                        &invoice_date,
                        currency.precision,
                    )
                    .await?;

                invoice_lines.extend(lines);
            }
        }

        Ok(invoice_lines)
    }
}
