use crate::domain::coupons::CouponDiscount;
use common_domain::ids::{
    AppliedCouponId, BillableMetricId, CouponId, CreditUnitId, PriceComponentId, ProductId,
    SubscriptionAddOnId, SubscriptionId, SubscriptionPriceComponentId,
};
use rust_decimal::Decimal;
use rust_decimal::prelude::Zero;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(PartialEq, Eq, Debug, Deserialize, Serialize, Clone)]
pub struct CouponLineItem {
    pub coupon_id: CouponId,
    pub applied_coupon_id: AppliedCouponId,
    pub name: String,
    pub code: String,
    pub value: i64,
    /// What the coupon took off the quoted, tax-included price, when `value` is only the
    /// net share of it (see `TaxInclusiveQuote`). This is what the customer used up of the
    /// coupon; `None` when the two are the same.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quoted_value: Option<i64>,
    pub discount: CouponDiscount,
}

impl CouponLineItem {
    /// The amount to draw down from the coupon: what it took off the price the customer saw.
    pub fn consumed_value(&self) -> i64 {
        self.quoted_value.unwrap_or(self.value)
    }
}

#[derive(PartialEq, Debug, Deserialize, Serialize, Eq, Clone)]
pub struct TaxDetail {
    pub tax_rate: Decimal,
    pub tax_name: String,
    pub tax_amount: i64,
}

/// Revenue taxonomy of an invoice line, resolved from the originating
/// `SubscriptionFee` variant at construction (the authoritative discriminator).
/// Persisted in the `invoice.line_items` jsonb; legacy lines lack it (→ `None`).
#[derive(PartialEq, Debug, Deserialize, Serialize, Eq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum ChargeType {
    Usage,
    Recurring,
    OneTime,
}

#[derive(PartialEq, Debug, Deserialize, Serialize, Eq, Clone)]
pub struct LineItem {
    // TODO id: LocalItemId & serde(alias = "local_id")
    pub local_id: String,
    pub name: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charge_type: Option<ChargeType>,

    #[serde(alias = "subtotal")]
    pub amount_subtotal: i64, // quantity * unit_price, before discounts and tax. Displayed on invoice
    #[serde(default = "Decimal::zero")]
    pub tax_rate: Decimal, // Displayed on invoice (computed from tax_details)
    #[serde(default)]
    pub taxable_amount: i64, // amount_subtotal - any discount or credit applied. Not displayed
    #[serde(default)]
    pub tax_amount: i64, // Not displayed (computed from tax_details)
    #[serde(alias = "total")]
    pub amount_total: i64, // taxable_amount + tax_amount. Not displayed

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tax_details: Vec<TaxDetail>,

    pub quantity: Option<Decimal>,
    pub unit_price: Option<Decimal>, // stored as-is in JSONB, no enforced scale

    pub start_date: chrono::NaiveDate,
    pub end_date: chrono::NaiveDate, // TODO check incl/excl

    pub sub_lines: Vec<SubLineItem>,

    pub is_prorated: bool,
    // todo remove?
    pub price_component_id: Option<PriceComponentId>,
    pub sub_component_id: Option<SubscriptionPriceComponentId>,
    pub sub_add_on_id: Option<SubscriptionAddOnId>,
    pub product_id: Option<ProductId>,
    pub metric_id: Option<BillableMetricId>,

    pub description: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_by_dimensions: Option<HashMap<String, String>>,

    /// Marker for synthesized lines that aren't tied to a price_component, used for
    /// shortfall (plan minimum) and similar invoice-level adjustments. Defaults to
    /// `Regular` for existing stored invoices.
    #[serde(default)]
    pub kind: LineItemKind,

    /// Credit a `Credit` line funds, in minor units of the invoice currency: on payment, or when
    /// its period opens for included credit. Frozen apart from the charge, so a bonus pack or a
    /// later catalog change never alters what was sold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_grant_amount: Option<i64>,

    /// When the funded credit lapses; absent means never.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_grant_expiry: Option<crate::domain::price_components::CreditExpiry>,

    /// Credit included in a plan at no charge, funded as promotional credit.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub credit_grant_included: bool,

    /// Credit taxed as an ordinary sale on the invoice selling it, not when used: a credit unit,
    /// which is no money prepayment. Credit notes reversing the line keep that tax.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub taxed_at_purchase: bool,

    /// The line as it was priced, before the tax was carved out of it. Only set on a line
    /// billed from tax-inclusive prices that actually bore tax; see [`TaxInclusiveQuote`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tax_inclusive_quote: Option<TaxInclusiveQuote>,

    /// Usage priced in a credit unit. The money fields then carry only the overage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_unit: Option<LineCreditUnit>,

    /// The credit unit a `Credit` line funds; absent funds the invoice currency.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_grant_unit: Option<crate::domain::credit_units::CreditUnitRef>,

    /// Rollover of the included credit this line funds, frozen in the grant's minor units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_grant_rollover: Option<crate::domain::credit::GrantRollover>,

    /// Promotional credit drawn against this line before tax, already taken off its taxable
    /// amount; in the figures the line was quoted in (tax included on a tax-inclusive line).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub promotional_credit: Option<i64>,

    /// The subscription the line bills; absent on lines stored before it was recorded, and on
    /// lines of no subscription. Read through `services::credit::scope::line_subscription`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription_id: Option<SubscriptionId>,

    /// When its usage was read up to, when not `end_date`'s midnight: an immediate cancellation
    /// bills its last day's usage up to the cancellation instant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_ends_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// The credit side of a line priced in a credit unit. Covered credits were paid by the unit
/// wallet (untaxed); `consumed - covered` is billed in money at `rate` on the line itself.
#[derive(PartialEq, Debug, Deserialize, Serialize, Eq, Clone)]
pub struct LineCreditUnit {
    pub unit_id: CreditUnitId,
    pub code: String,
    pub precision: u8,
    /// Invoice-currency major units per credit, as `overage_rate` resolves it from the customer's
    /// agreed price, the price's rate and the unit's list price.
    pub rate: Decimal,
    /// The rate the price itself sets, which `rate` is resolved from on every recompute. None on
    /// lines priced before it was kept, where `rate` still is that rate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_rate: Option<Decimal>,
    /// Credits per usage unit.
    pub unit_price: Option<Decimal>,
    /// Tier or matrix breakdown, in credit minor units.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sub_lines: Vec<SubLineItem>,
    /// Credit minor units the usage priced.
    pub consumed: i64,
    /// Of which paid by the unit's grants (draws and settlements).
    pub covered: i64,
}

/// What a tax-inclusive line looked like when it was priced: the quoted, tax-included
/// figures its net `amount_subtotal`, `unit_price` and `sub_lines` were derived from.
///
/// A line is stored net so that everything reading an invoice — credit notes, reports,
/// e-invoices, the draft editor — keeps a single meaning for every amount. But netting is
/// lossy (9.99 quoted at 20% is 8.325 a unit), so a recompute that starts again from stored
/// lines needs the quoted figures back rather than a reconstruction of them. They also let
/// a document show the customer the price they were actually quoted.
#[derive(PartialEq, Debug, Deserialize, Serialize, Eq, Clone)]
pub struct TaxInclusiveQuote {
    pub amount_subtotal: i64,
    pub unit_price: Option<Decimal>,
    pub sub_lines: Vec<SubLineItem>,
}

/// The discount of a stored invoice in the figures it was quoted in: what was taken off the
/// tax-included prices, where `invoice.discount` is only the net share of it.
///
/// Recovered exactly rather than grossed back up: a line's total *is* its quoted taxable
/// amount (net and tax add back to it to the unit), so what separates it from the quoted
/// subtotal is the quoted discount.
pub fn quoted_discount(lines: &[LineItem]) -> i64 {
    lines
        .iter()
        .map(|line| {
            let quoted_subtotal = line
                .tax_inclusive_quote
                .as_ref()
                .map_or(line.amount_subtotal, |quote| quote.amount_subtotal);
            quoted_subtotal - line.amount_total - line.promotional_credit.unwrap_or(0)
        })
        .sum::<i64>()
        .max(0)
}

impl LineItem {
    /// The end of the dates its usage was consumed in: only credit effective before it pays it.
    pub fn usage_dated_before(&self) -> chrono::DateTime<chrono::Utc> {
        self.usage_ends_at
            .unwrap_or_else(|| self.end_date.and_time(chrono::NaiveTime::MIN).and_utc())
    }

    /// The product credit restricted to products answers to: none for a minimum shortfall, whose
    /// product only carries its tax settings, so no product-restricted credit pays it.
    pub fn credit_product(&self) -> Option<common_domain::ids::ProductId> {
        self.product_id
            .filter(|_| self.kind != LineItemKind::MinimumShortfall)
    }

    /// Credit taxed as an ordinary sale when sold; included credit never is.
    pub fn credit_taxed_at_purchase(&self) -> bool {
        self.taxed_at_purchase && !self.credit_grant_included
    }

    /// A credit line whose tax is due when the credit is used: currency credit is a prepayment.
    pub fn credit_taxed_at_redemption(&self) -> bool {
        self.kind == LineItemKind::Credit && !self.credit_taxed_at_purchase()
    }

    /// Currency credit sold or credited on a document, which the tax pass leaves out: its own row
    /// states it in the breakdown, apart from the supplies.
    pub fn states_prepaid_credit(&self) -> bool {
        self.credit_taxed_at_redemption()
            && self.taxable_amount != 0
            && self.tax_amount == 0
            && self.tax_details.is_empty()
    }

    /// A paid purchase of currency credit: a prepayment, taxed when the credit is used.
    pub fn is_prepaid_credit_purchase(&self) -> bool {
        self.amount_subtotal > 0 && self.credit_taxed_at_redemption()
    }

    /// The system purchase this line is, when the platform wrote it (never a plan's pack).
    pub fn credit_purchase_name(&self) -> Option<CreditPurchaseLineName> {
        if self.price_component_id.is_some()
            || self.sub_component_id.is_some()
            || self.sub_add_on_id.is_some()
        {
            return None;
        }
        let name = CreditPurchaseLineName::from_stored_name(&self.name)?;
        // A purchase of the retired balance was written as a Regular line.
        match self.kind {
            LineItemKind::Credit => Some(name),
            LineItemKind::Regular if name == CreditPurchaseLineName::Purchase => Some(name),
            _ => None,
        }
    }

    /// The document figure of the line's promotional credit: its net share of the line's
    /// reduction on a tax-inclusive line that bore tax, its quoted share otherwise.
    pub fn net_promotional_credit(&self) -> i64 {
        let quoted = self.promotional_credit.unwrap_or(0);
        let Some(quote) = self.tax_inclusive_quote.as_ref().filter(|_| quoted != 0) else {
            return quoted;
        };
        let reduced_quoted = quote.amount_subtotal - self.amount_total;
        let reduced_net = self.amount_subtotal - self.taxable_amount;
        if reduced_quoted <= 0 {
            return quoted;
        }
        meteroid_tax::round_to_minor(
            Decimal::from(reduced_net) * Decimal::from(quoted) / Decimal::from(reduced_quoted),
        )
    }

    /// The document figure of a credit-note line's promotional credit, in the note's sign,
    /// `source` being the invoice line it credits. The note's reduction (subtotal less taxed base)
    /// splits as the source line's does: its discount share is rounded and the promotional credit
    /// takes the rest, so the two always add up to the reduction and neither turns negative. The
    /// line's `promotional_credit` stays what goes back to the grants, which may round apart.
    pub fn note_net_promotional_credit(&self, source: Option<&LineItem>) -> i64 {
        let reduction = self.amount_subtotal - self.taxable_amount;
        let within = |share: i64| {
            if reduction <= 0 {
                share.clamp(reduction, 0)
            } else {
                share.clamp(0, reduction)
            }
        };
        let Some(source) = source else {
            return within(self.promotional_credit.unwrap_or(0));
        };
        let source_reduction = source.amount_subtotal - source.taxable_amount;
        let source_promotional = source.net_promotional_credit();
        if source_reduction == 0 || source_promotional == 0 {
            return 0;
        }
        let discount = meteroid_tax::round_to_minor(
            Decimal::from(reduction) * Decimal::from(source_reduction - source_promotional)
                / Decimal::from(source_reduction),
        );
        within(reduction - discount)
    }

    /// Puts a stored line back into the figures it was priced in, ready to go through
    /// discount and tax again. A no-op for a line that was never netted.
    ///
    /// A line without its quote — an operator wrote it, or rewrote its price, in the draft
    /// editor, which works in net amounts — is grossed up by its own rate instead.
    pub fn into_tax_inclusive_quote(mut self) -> LineItem {
        if let Some(quote) = self.tax_inclusive_quote.take() {
            self.amount_subtotal = quote.amount_subtotal;
            self.unit_price = quote.unit_price;
            self.sub_lines = quote.sub_lines;
        } else if !self.tax_rate.is_zero() {
            let factor = Decimal::ONE + self.tax_rate;
            let gross_up = |amount: i64| {
                use rust_decimal::prelude::ToPrimitive;
                (Decimal::from(amount) * factor)
                    .round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
                    .to_i64()
                    .unwrap_or(amount)
            };
            self.amount_subtotal = gross_up(self.amount_subtotal);
            self.unit_price = self.unit_price.map(|price| price * factor);
            for sub_line in &mut self.sub_lines {
                sub_line.total = gross_up(sub_line.total);
                sub_line.unit_price *= factor;
            }
        }
        self.taxable_amount = self.amount_subtotal;
        self.amount_total = self.amount_subtotal;
        self.tax_amount = 0;
        self.promotional_credit = None;
        self
    }
}

/// What an invoice's minimum-commitment shortfall lines bill, before discounts and tax.
pub fn minimum_shortfall(lines: &[LineItem]) -> i64 {
    lines
        .iter()
        .filter(|line| line.kind == LineItemKind::MinimumShortfall)
        .map(|line| line.amount_subtotal)
        .sum()
}

/// The name of a credit-purchase line the platform writes. The stored English name is the key the
/// documents localize from, so it never changes: documents already issued carry it.
#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum CreditPurchaseLineName {
    WalletTopUp,
    AutomaticTopUp,
    Purchase,
}

impl CreditPurchaseLineName {
    pub const fn stored_name(self) -> &'static str {
        match self {
            Self::WalletTopUp => "Wallet top-up",
            Self::AutomaticTopUp => "Automatic top-up",
            Self::Purchase => "Purchase credits",
        }
    }

    pub fn from_stored_name(name: &str) -> Option<Self> {
        [Self::WalletTopUp, Self::AutomaticTopUp, Self::Purchase]
            .into_iter()
            .find(|candidate| candidate.stored_name() == name)
    }
}

#[derive(PartialEq, Eq, Debug, Default, Deserialize, Serialize, Clone, Copy)]
pub enum LineItemKind {
    #[default]
    Regular,
    /// Synthesized fill-up line bringing a billing-period invoice up to the subscription's
    /// minimum commitment floor.
    MinimumShortfall,
    /// A credit purchase: paying the invoice funds `credit_grant_amount` into the customer's
    /// invoice credit.
    Credit,
}

#[derive(PartialEq, Debug, Deserialize, Serialize, Eq, Clone)]
pub struct SubLineItem {
    pub local_id: String,
    pub name: String,
    pub total: i64,
    pub quantity: Decimal,
    pub unit_price: Decimal, // stored as-is in JSONB, no enforced scale
    pub attributes: Option<SubLineAttributes>,
}

#[derive(PartialEq, Debug, Deserialize, Serialize, Eq, Clone)]
pub enum SubLineAttributes {
    Package {
        raw_usage: Decimal,
    },
    Tiered {
        first_unit: u64,
        last_unit: Option<u64>,
        flat_cap: Option<Decimal>,
        flat_fee: Option<Decimal>,
    },
    Volume {
        first_unit: u64,
        last_unit: Option<u64>,
        flat_cap: Option<Decimal>,
        flat_fee: Option<Decimal>,
    },
    Matrix {
        dimension1_key: String,
        dimension1_value: String,
        dimension2_key: Option<String>,
        dimension2_value: Option<String>,
    },
}

#[cfg(test)]
mod credit_unit_serde_tests {
    use super::*;

    fn stored_line() -> serde_json::Value {
        serde_json::json!({
            "local_id": "l1",
            "name": "API calls",
            "charge_type": "usage",
            "amount_subtotal": 1200,
            "tax_rate": "0",
            "taxable_amount": 1200,
            "tax_amount": 0,
            "amount_total": 1200,
            "quantity": "12",
            "unit_price": "1",
            "start_date": "2026-01-01",
            "end_date": "2026-02-01",
            "sub_lines": [],
            "is_prorated": false,
            "price_component_id": null,
            "sub_component_id": null,
            "sub_add_on_id": null,
            "product_id": null,
            "metric_id": null,
            "description": null,
            "kind": "Regular"
        })
    }

    #[test]
    fn a_stored_money_line_reads_without_credit_fields_and_writes_back_unchanged() {
        let line: LineItem = serde_json::from_value(stored_line()).unwrap();
        assert_eq!(line.credit_unit, None);
        assert_eq!(line.credit_grant_unit, None);
        assert_eq!(line.credit_grant_rollover, None);
        assert_eq!(serde_json::to_value(&line).unwrap(), stored_line());
    }

    #[test]
    fn a_credit_block_round_trips() {
        let mut stored = stored_line();
        stored["credit_unit"] = serde_json::json!({
            "unit_id": "00000000-0000-0000-0000-000000000001",
            "code": "CREDITS",
            "precision": 2,
            "rate": "0.01",
            "unit_price": "3",
            "consumed": 360000,
            "covered": 300000
        });
        stored["credit_grant_rollover"] =
            serde_json::json!({"max_minor": 20000, "expiry": {"unit": "MONTH", "value": 1}});
        let line: LineItem = serde_json::from_value(stored.clone()).unwrap();
        let block = line.credit_unit.clone().unwrap();
        assert_eq!(
            (block.consumed, block.covered, block.precision),
            (360000, 300000, 2)
        );
        assert_eq!(line.credit_grant_rollover.unwrap().max_minor, Some(20000));
        assert_eq!(serde_json::to_value(&line).unwrap(), stored);
    }
}

#[cfg(test)]
mod minimum_shortfall_tests {
    use super::*;

    fn line(local_id: &str, kind: &str, subtotal: i64, taxable: i64) -> LineItem {
        serde_json::from_value(serde_json::json!({
            "local_id": local_id,
            "name": local_id,
            "amount_subtotal": subtotal,
            "taxable_amount": taxable,
            "amount_total": taxable,
            "quantity": null,
            "unit_price": null,
            "start_date": "2026-10-01",
            "end_date": "2026-11-01",
            "sub_lines": [],
            "is_prorated": false,
            "kind": kind,
        }))
        .unwrap()
    }

    #[test]
    fn only_shortfall_lines_count_whatever_fees_and_coupons_bill() {
        // Meridian with a $500 platform fee and a $500 coupon: $1,240 usage + $500 fee + $3,260
        // shortfall = $5,000 subtotal, less the coupon = $4,500 before tax.
        let lines = [
            line("usage", "Regular", 124_000, 111_600),
            line("fee", "Regular", 50_000, 45_000),
            line("shortfall", "MinimumShortfall", 326_000, 293_400),
        ];
        assert_eq!(minimum_shortfall(&lines), 326_000);
        let taxable: i64 = lines.iter().map(|l| l.taxable_amount).sum();
        assert_eq!(taxable, 450_000);
        assert_eq!(minimum_shortfall(&lines[..2]), 0);
    }
}
