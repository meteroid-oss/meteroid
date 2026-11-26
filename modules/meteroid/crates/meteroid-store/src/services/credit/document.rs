//! The store's lines as `billing_core::document` reads them, and the figures the core computes
//! written back onto them.
use crate::domain::{
    LineItem, LineItemKind, SubLineAttributes, SubLineItem, TaxDetail, TaxInclusiveQuote,
};
use crate::errors::StoreError;
use crate::services::credit::promotional::LineScope;
use crate::services::credit::scope::line_subscription;
use crate::services::credit::{invalid, precondition};
use billing_core::assembly::tax_inclusive::SubLinePrice;
use billing_core::credit::SettleError;
use billing_core::document as core;
use common_domain::ids::SubscriptionId;
use error_stack::Report;
use rust_decimal::Decimal;

pub(crate) fn settle_error(error: SettleError) -> Report<StoreError> {
    match error {
        SettleError::Overflow => invalid("invoice credit exceeds the supported range"),
        SettleError::UnknownDemand => precondition("a usage charge names no demand of the request"),
        SettleError::Unbalanced => Report::new(StoreError::BillingInvariantViolated(
            "an invoice's credit shares do not add up to its draws (I3, I23)".to_owned(),
        )),
    }
}

fn sub_prices(sub_lines: &[SubLineItem]) -> Vec<SubLinePrice> {
    sub_lines
        .iter()
        .map(|sub_line| {
            let (flat_cap, flat_fee) = match &sub_line.attributes {
                Some(
                    SubLineAttributes::Tiered {
                        flat_cap, flat_fee, ..
                    }
                    | SubLineAttributes::Volume {
                        flat_cap, flat_fee, ..
                    },
                ) => (*flat_cap, *flat_fee),
                Some(SubLineAttributes::Package { .. } | SubLineAttributes::Matrix { .. })
                | None => (None, None),
            };
            SubLinePrice {
                total: sub_line.total,
                unit_price: sub_line.unit_price,
                flat_cap,
                flat_fee,
            }
        })
        .collect()
}

fn write_sub_lines(sub_lines: &mut [SubLineItem], prices: &[SubLinePrice]) {
    for (sub_line, price) in sub_lines.iter_mut().zip(prices) {
        sub_line.total = price.total;
        sub_line.unit_price = price.unit_price;
        if let Some(
            SubLineAttributes::Tiered {
                flat_cap, flat_fee, ..
            }
            | SubLineAttributes::Volume {
                flat_cap, flat_fee, ..
            },
        ) = &mut sub_line.attributes
        {
            *flat_cap = price.flat_cap;
            *flat_fee = price.flat_fee;
        }
    }
}

/// `item` as the core reads it, billing `subscription` (resolved by `line_subscription`).
pub(crate) fn line_of(item: &LineItem, subscription: Option<SubscriptionId>) -> core::Line {
    let kind = if item.kind == LineItemKind::Credit {
        core::LineKind::Credit
    } else if item.metric_id.is_some() && item.kind == LineItemKind::Regular {
        core::LineKind::Usage {
            dated_before: item.usage_dated_before(),
        }
    } else {
        core::LineKind::Fee
    };
    core::Line {
        id: item.local_id.clone(),
        kind,
        subscription,
        product: item.credit_product(),
        figures: core::Figures {
            subtotal: item.amount_subtotal,
            unit_price: item.unit_price,
            sub_lines: sub_prices(&item.sub_lines),
        },
        quote: item
            .tax_inclusive_quote
            .as_ref()
            .map(|quote| core::Figures {
                subtotal: quote.amount_subtotal,
                unit_price: quote.unit_price,
                sub_lines: sub_prices(&quote.sub_lines),
            }),
        taxable: item.taxable_amount,
        tax: item.tax_amount,
        total: item.amount_total,
        tax_rate: item.tax_rate,
        taxes: item
            .tax_details
            .iter()
            .map(|detail| core::LineTax {
                rate: detail.tax_rate,
                name: detail.tax_name.clone(),
                amount: detail.tax_amount,
            })
            .collect(),
        promotional: item.promotional_credit,
        taxed_at_redemption: crate::services::credit::prepaid_purchase_tax::taxed_at_redemption(
            item,
        ),
    }
}

/// `items` as the core reads them, each billing the subscription `scope` resolves.
pub(crate) fn lines_of(items: &[LineItem], scope: LineScope<'_>) -> Vec<core::Line> {
    items
        .iter()
        .map(|item| {
            line_of(
                item,
                line_subscription(item, scope.subscription_id, scope.owners),
            )
        })
        .collect()
}

/// The rates a stored line is taxed by again: its own details, unless its rate was edited away
/// from them; then the edited rate is the single tax, since nothing records how to split it.
pub(crate) fn rates_of(item: &LineItem) -> Vec<core::Rate> {
    let details_match_rate = !item.tax_details.is_empty()
        && item
            .tax_details
            .iter()
            .map(|detail| detail.tax_rate)
            .sum::<Decimal>()
            == item.tax_rate;
    if details_match_rate {
        item.tax_details
            .iter()
            .map(|detail| core::Rate {
                rate: detail.tax_rate,
                name: detail.tax_name.clone(),
            })
            .collect()
    } else if item.tax_rate.is_zero() {
        Vec::new()
    } else {
        vec![core::Rate {
            rate: item.tax_rate,
            name: "Tax".to_string(),
        }]
    }
}

pub(crate) fn rates(items: &[LineItem]) -> core::Rates {
    core::Rates(items.iter().map(rates_of).collect())
}

/// Writes the core's figures of `line` onto `item`; `netted` when the core just netted it, from
/// the figures `item` holds, which become its quote.
pub(crate) fn write_money(item: &mut LineItem, line: &core::Line, netted: bool) {
    if netted && let Some(quote) = &line.quote {
        let mut sub_lines = item.sub_lines.clone();
        write_sub_lines(&mut sub_lines, &quote.sub_lines);
        item.tax_inclusive_quote = Some(TaxInclusiveQuote {
            amount_subtotal: quote.subtotal,
            unit_price: quote.unit_price,
            sub_lines,
        });
    }
    item.amount_subtotal = line.figures.subtotal;
    item.unit_price = line.figures.unit_price;
    write_sub_lines(&mut item.sub_lines, &line.figures.sub_lines);
    item.taxable_amount = line.taxable;
    item.tax_amount = line.tax;
    item.amount_total = line.total;
    item.tax_details = line
        .taxes
        .iter()
        .map(|tax| TaxDetail {
            tax_rate: tax.rate,
            tax_name: tax.name.clone(),
            tax_amount: tax.amount,
        })
        .collect();
    item.promotional_credit = line.promotional;
}
