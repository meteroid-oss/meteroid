//! What a quote adds up to, for display: its amounts by cadence and the first invoice of the
//! subscription it converts to.

use common_domain::ids::TenantId;
use common_utils::decimals::ToSubunit;
use error_stack::Report;
use rust_decimal::Decimal;

use crate::StoreResult;
use crate::constants::Currencies;
use crate::domain::quote_document::QuoteDocument;
use crate::domain::quote_minimums::{QuoteMinimum, QuoteMinimumNew};
use crate::domain::quotes::{
    DetailedQuote, Quote, QuoteAddOn, QuoteCoupon, QuotePriceComponent, QuoteRecurringTotal,
    QuoteTotals, UnsavedQuote,
};
use crate::domain::{SubscriptionFee, SubscriptionFeeBillingPeriod};
use crate::errors::StoreError;
use crate::repositories::customers::CustomersInterface;
use crate::repositories::invoicing_entities::InvoicingEntityInterface;
use crate::repositories::plans::PlansInterface;
use crate::services::ServicesEdge;
use crate::services::quotes::build_subscription_from_quote;
use crate::services::subscriptions::utils::{fee_period_total_cents, scale_fee};

impl ServicesEdge {
    pub async fn quote_totals(
        &self,
        tenant_id: TenantId,
        quote: &DetailedQuote,
    ) -> StoreResult<QuoteTotals> {
        let mut conn = self.store.get_conn().await?;
        // The subscription bills in its plan's currency.
        let currency = self
            .store
            .get_plan_by_version_id_with_conn(&mut conn, quote.quote.plan_version_id, tenant_id)
            .await?
            .version
            .ok_or_else(|| {
                Report::new(StoreError::ValueNotFound(
                    "Plan version not found".to_string(),
                ))
            })?
            .currency;
        let precision = Currencies::resolve_currency_precision(&currency).ok_or_else(|| {
            Report::new(StoreError::InvalidArgument(format!(
                "Unknown currency {currency}"
            )))
        })?;
        let (recurring, has_usage) = recurring_totals(&totals_lines(quote), precision);

        let first_invoice_subtotal = match build_subscription_from_quote(quote) {
            Ok(conversion) => self
                .services
                .preview_subscription_from_quote(&mut conn, &conversion, tenant_id)
                .await
                .map(|(content, _)| content.total - content.tax_amount),
            Err(error) => Err(error),
        };
        let first_invoice_subtotal = match first_invoice_subtotal {
            Ok(cents) => Some(major(cents, precision)),
            Err(error) => {
                log::warn!(
                    "no first invoice preview for quote {:?}: {error:?}",
                    quote.quote.id
                );
                None
            }
        };

        Ok(QuoteTotals {
            currency,
            recurring: recurring
                .into_iter()
                .map(|(period, amount, list_amount)| QuoteRecurringTotal {
                    period,
                    amount: major(amount, precision),
                    list_amount: major(list_amount, precision),
                })
                .collect(),
            has_usage,
            first_invoice_subtotal,
        })
    }

    /// The totals of an editor's quote, as they would read once it is stored. Writes nothing.
    pub async fn unsaved_quote_totals(
        &self,
        tenant_id: TenantId,
        unsaved: UnsavedQuote,
    ) -> StoreResult<QuoteTotals> {
        let (customer, invoicing_entity) = {
            let mut conn = self.store.get_conn().await?;
            let customer = self
                .store
                .find_customer_by_id_with_conn(&mut conn, unsaved.quote.customer_id, tenant_id)
                .await?;
            let invoicing_entity = self
                .store
                .get_invoicing_entity_with_conn(
                    &mut conn,
                    tenant_id,
                    Some(customer.invoicing_entity_id),
                )
                .await?;
            (customer, invoicing_entity)
        };
        let quote_id = unsaved.quote.id;
        let quote = DetailedQuote {
            quote: Quote::unsaved(unsaved.quote),
            customer,
            invoicing_entity,
            components: unsaved
                .components
                .into_iter()
                .map(QuotePriceComponent::unsaved)
                .collect(),
            add_ons: unsaved
                .add_ons
                .into_iter()
                .map(QuoteAddOn::unsaved)
                .collect(),
            coupons: unsaved
                .coupons
                .into_iter()
                .map(QuoteCoupon::unsaved)
                .collect(),
            // Coupons are applied by id: the totals never read their details.
            coupon_details: vec![],
            signatures: vec![],
            // Entitlements carry no price: the totals never read them.
            entitlements: vec![],
            minimum: unsaved.minimum.map(|(amount_subunit, scope)| {
                QuoteMinimum::from(QuoteMinimumNew {
                    quote_id,
                    amount_subunit,
                    scope,
                })
            }),
        };
        self.quote_totals(tenant_id, &quote).await
    }
}

/// What a published document adds up to by cadence, and whether a line bills usage: computed
/// from the document alone, so it reads the same as when it was published.
pub fn document_totals(document: &QuoteDocument) -> StoreResult<(Vec<QuoteRecurringTotal>, bool)> {
    let precision =
        Currencies::resolve_currency_precision(&document.currency).ok_or_else(|| {
            Report::new(StoreError::InvalidArgument(format!(
                "Unknown currency {}",
                document.currency
            )))
        })?;
    let lines: Vec<TotalsLine<'_>> = document
        .lines
        .iter()
        .map(|line| TotalsLine {
            period: line.period,
            fee: &line.fee,
            list_fee: line.list_fee.as_ref(),
            quantity: line.quantity.unwrap_or(1),
        })
        .collect();
    let (recurring, has_usage) = recurring_totals(&lines, precision);
    Ok((
        recurring
            .into_iter()
            .map(|(period, amount, list_amount)| QuoteRecurringTotal {
                period,
                amount: major(amount, precision),
                list_amount: major(list_amount, precision),
            })
            .collect(),
        has_usage,
    ))
}

fn major(subunits: i64, precision: u8) -> Decimal {
    Decimal::new(subunits, u32::from(precision))
}

struct TotalsLine<'a> {
    period: SubscriptionFeeBillingPeriod,
    fee: &'a SubscriptionFee,
    list_fee: Option<&'a SubscriptionFee>,
    quantity: i32,
}

fn totals_lines(quote: &DetailedQuote) -> Vec<TotalsLine<'_>> {
    quote
        .components
        .iter()
        .map(|c| TotalsLine {
            period: c.period,
            fee: &c.fee,
            list_fee: c.list_fee.as_ref(),
            quantity: 1,
        })
        .chain(quote.add_ons.iter().map(|a| TotalsLine {
            period: a.period,
            fee: &a.fee,
            list_fee: a.list_fee.as_ref(),
            quantity: a.quantity,
        }))
        .collect()
}

/// What one billing of the fee charges up front, in subunits, and whether it is charged once.
/// `None` for a fee billed on consumption only.
fn billed(fee: &SubscriptionFee, quantity: i32, precision: u8) -> Option<(i64, bool)> {
    let rounded = |amount: Decimal| amount.to_subunit_opt(precision).unwrap_or(0);
    match scale_fee(fee, quantity) {
        SubscriptionFee::Usage { .. } => None,
        SubscriptionFee::OneTime {
            rate,
            quantity,
            model,
        } => Some((
            crate::domain::price_components::fixed_quantity_minor(
                rate,
                Decimal::from(quantity),
                model.as_ref(),
                precision,
            ),
            true,
        )),
        SubscriptionFee::Credit {
            rate,
            quantity,
            once: true,
            ..
        } => Some((rounded(rate * Decimal::from(quantity)), true)),
        // Rounded once on the seats' total, as the invoice prices it.
        SubscriptionFee::Slot {
            model,
            initial_slots,
            ..
        } => Some((
            model
                .price(Decimal::from(initial_slots), precision)
                .ok()
                .map(|line| line.total)
                .unwrap_or(0),
            false,
        )),
        recurring => Some((fee_period_total_cents(&recurring, precision), false)),
    }
}

fn bills_usage(fee: &SubscriptionFee) -> bool {
    match fee {
        SubscriptionFee::Usage { .. } => true,
        SubscriptionFee::Capacity { overage_rate, .. } => *overage_rate > Decimal::ZERO,
        _ => false,
    }
}

/// `(period, amount, list amount)` in subunits by cadence, shortest first and one-time charges
/// last, and whether some line bills consumption.
fn recurring_totals(
    lines: &[TotalsLine<'_>],
    precision: u8,
) -> (Vec<(SubscriptionFeeBillingPeriod, i64, i64)>, bool) {
    let mut totals: Vec<(SubscriptionFeeBillingPeriod, i64, i64)> = vec![];
    let mut has_usage = false;
    for line in lines {
        has_usage |= bills_usage(line.fee);
        let Some((amount, once)) = billed(line.fee, line.quantity, precision) else {
            continue;
        };
        let list = line
            .list_fee
            .and_then(|fee| billed(fee, line.quantity, precision))
            .map_or(amount, |(list, _)| list);
        let period = if once {
            SubscriptionFeeBillingPeriod::OneTime
        } else {
            line.period
        };
        match totals.iter_mut().find(|(p, ..)| *p == period) {
            Some((_, total, total_list)) => {
                *total += amount;
                *total_list += list;
            }
            None => totals.push((period, amount, list)),
        }
    }
    totals.sort_by_key(|(period, ..)| period.as_months());
    (totals, has_usage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::enums::BillingType;
    use common_domain::ids::{BaseId, BillableMetricId};
    use rust_decimal_macros::dec;

    fn line<'a>(
        period: SubscriptionFeeBillingPeriod,
        fee: &'a SubscriptionFee,
        list_fee: Option<&'a SubscriptionFee>,
        quantity: i32,
    ) -> TotalsLine<'a> {
        TotalsLine {
            period,
            fee,
            list_fee,
            quantity,
        }
    }

    #[test]
    fn a_discounted_line_keeps_its_list_amount() {
        let fee = SubscriptionFee::Rate { rate: dec!(80) };
        let list = SubscriptionFee::Rate { rate: dec!(100) };
        let (totals, has_usage) = recurring_totals(
            &[line(
                SubscriptionFeeBillingPeriod::Monthly,
                &fee,
                Some(&list),
                1,
            )],
            2,
        );
        assert_eq!(
            totals,
            vec![(SubscriptionFeeBillingPeriod::Monthly, 8000, 10000)]
        );
        assert!(!has_usage);
    }

    #[test]
    fn slots_bill_their_seat_count() {
        let fee = SubscriptionFee::Slot {
            unit: "seat".to_string(),
            model: crate::domain::price_components::RatingModel::PerUnit { rate: dec!(12.50) },
            min_slots: None,
            max_slots: None,
            initial_slots: 5,
        };
        let (totals, _) = recurring_totals(
            &[line(SubscriptionFeeBillingPeriod::Monthly, &fee, None, 1)],
            2,
        );
        assert_eq!(
            totals,
            vec![(SubscriptionFeeBillingPeriod::Monthly, 6250, 6250)]
        );
    }

    #[test]
    fn an_add_on_bills_its_quantity() {
        let fee = SubscriptionFee::Rate { rate: dec!(15) };
        let list = SubscriptionFee::Rate { rate: dec!(20) };
        let (totals, _) = recurring_totals(
            &[line(
                SubscriptionFeeBillingPeriod::Monthly,
                &fee,
                Some(&list),
                3,
            )],
            2,
        );
        assert_eq!(
            totals,
            vec![(SubscriptionFeeBillingPeriod::Monthly, 4500, 6000)]
        );
    }

    #[test]
    fn usage_is_left_out_and_flagged() {
        let usage = SubscriptionFee::Usage {
            metric_id: BillableMetricId::new(),
            model: crate::domain::UsagePricingModel::PerUnit { rate: dec!(0.01) },
            credit_unit: None,
        };
        let rate = SubscriptionFee::Rate { rate: dec!(10) };
        let (totals, has_usage) = recurring_totals(
            &[
                line(SubscriptionFeeBillingPeriod::Monthly, &usage, None, 1),
                line(SubscriptionFeeBillingPeriod::Monthly, &rate, None, 1),
            ],
            2,
        );
        assert_eq!(
            totals,
            vec![(SubscriptionFeeBillingPeriod::Monthly, 1000, 1000)]
        );
        assert!(has_usage);
    }

    #[test]
    fn each_cadence_has_its_total_and_one_time_charges_come_last() {
        let annual = SubscriptionFee::Rate { rate: dec!(1200) };
        let setup = SubscriptionFee::OneTime {
            rate: dec!(500),
            quantity: 2,
            model: None,
        };
        let monthly = SubscriptionFee::Recurring {
            rate: dec!(30),
            quantity: 2,
            billing_type: BillingType::Advance,
            model: None,
        };
        let more_monthly = SubscriptionFee::Rate { rate: dec!(40) };
        let (totals, _) = recurring_totals(
            &[
                line(SubscriptionFeeBillingPeriod::Annual, &annual, None, 1),
                line(SubscriptionFeeBillingPeriod::OneTime, &setup, None, 1),
                line(SubscriptionFeeBillingPeriod::Monthly, &monthly, None, 1),
                line(
                    SubscriptionFeeBillingPeriod::Monthly,
                    &more_monthly,
                    None,
                    1,
                ),
            ],
            2,
        );
        assert_eq!(
            totals,
            vec![
                (SubscriptionFeeBillingPeriod::Monthly, 10000, 10000),
                (SubscriptionFeeBillingPeriod::Annual, 120000, 120000),
                (SubscriptionFeeBillingPeriod::OneTime, 100000, 100000),
            ]
        );
    }

    #[test]
    fn seats_round_once_on_their_total() {
        let fee = SubscriptionFee::Slot {
            unit: "seat".to_string(),
            model: crate::domain::price_components::RatingModel::PerUnit { rate: dec!(0.125) },
            min_slots: None,
            max_slots: None,
            initial_slots: 1000,
        };
        let (totals, _) = recurring_totals(
            &[line(SubscriptionFeeBillingPeriod::Monthly, &fee, None, 1)],
            2,
        );
        assert_eq!(
            totals,
            vec![(SubscriptionFeeBillingPeriod::Monthly, 12500, 12500)]
        );
    }
}
