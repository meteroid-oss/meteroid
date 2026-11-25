//! A product line's pricing page: its ladder priced in one currency in one option,
//! and the marketing copy each plan carries for it.
use common_domain::ids::{FeatureId, PlanId, ProductFamilyId, ProductId};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};

use super::enums::BillingPeriodEnum;
use super::prices::{FeeStructure, UsageModel};

pub const MAX_COPY_CHARS: usize = 120;
pub const MAX_HIGHLIGHTS: usize = 12;
pub const MAX_HIDDEN_CHARGES: usize = 50;
pub const MAX_ORDERED_CHARGES: usize = 100;

/// One line of a plan's card, stored as `{"feature": id}` or `{"text": "..."}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Highlight {
    /// Rendered with its value in the plan's published version.
    Feature(FeatureId),
    Text(String),
}

/// What a plan shows on a pricing page. Not versioned: editing it never makes a plan version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanPricingPage {
    pub plan_id: PlanId,
    pub tagline: Option<String>,
    /// At most one plan per product line.
    pub popular: bool,
    pub cta_label: Option<String>,
    pub highlights: Vec<Highlight>,
    /// Sold by contacting sales: shown on the page, never offered self-serve.
    pub sales_led: bool,
    /// A sales-led plan's price stays off the page.
    pub hide_price: bool,
    /// Where a sales-led plan's button leads, an absolute http(s) URL.
    pub contact_url: Option<String>,
    /// The order the card reads its charges in, by product (charges are new in each version,
    /// their products stay): the first that can lead is its price. Products not listed follow
    /// as computed (`price_headline::in_order`).
    pub product_order: Vec<ProductId>,
    /// Usage charges left off the card, by product: they read together as "+ usage".
    pub hidden_product_ids: Vec<ProductId>,
}

/// How a charge reads on a card, by what its product bills; in the order a card lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PricingChargeKind {
    /// In the card's amount, or beside it when billed on another period.
    Fixed,
    Seats,
    Usage,
    OneTime,
    CreditPack,
}

/// A charge of a plan as its card's editor lists it: what the price may lead with, what may be
/// left off.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PricingCharge {
    pub product_id: ProductId,
    pub name: String,
    pub kind: PricingChargeKind,
    /// It can be the card's price: the fixed fees together as its base price, a seat, or usage
    /// read per unit (per unit, by tiers, by package); a rate card, rates by dimension and a
    /// capacity never.
    pub can_lead: bool,
    /// Any charge may be left off the card but the one leading it, which reads whatever is hidden.
    pub can_hide: bool,
}

/// What a plan's card can be led by and leave off, and the product line whose page shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanPricingCharges {
    pub product_family_id: ProductFamilyId,
    pub charges: Vec<PricingCharge>,
}

impl PricingCharge {
    pub fn of(product_id: ProductId, name: String, fee: &FeeStructure) -> Self {
        let (kind, can_lead) = match fee {
            FeeStructure::Rate {} | FeeStructure::ExtraRecurring { .. } => {
                (PricingChargeKind::Fixed, true)
            }
            FeeStructure::Slot { .. } => (PricingChargeKind::Seats, true),
            FeeStructure::Usage { model, .. } => (
                PricingChargeKind::Usage,
                matches!(
                    model,
                    UsageModel::PerUnit
                        | UsageModel::Tiered
                        | UsageModel::Volume
                        | UsageModel::Package
                ),
            ),
            // Its fee is in the base price; the level is what it sells.
            FeeStructure::Capacity { .. } => (PricingChargeKind::Usage, false),
            FeeStructure::OneTime {} => (PricingChargeKind::OneTime, false),
            FeeStructure::Credit {} => (PricingChargeKind::CreditPack, false),
        };
        PricingCharge {
            product_id,
            name,
            kind,
            can_lead,
            can_hide: true,
        }
    }
}

/// Which page a visitor asks for; each left out picks the default.
#[derive(Clone, Debug, Default)]
pub struct PricingPageRequest {
    /// A currency the ladder is sold in; none, or one it is not sold in: the default.
    pub currency: Option<String>,
    /// An option alias of the page's switch; none, or one not on it: the first.
    pub option: Option<String>,
    /// A plan's copy as an editor holds it, read instead of the saved one: a preview, unsaved.
    pub draft: Option<PlanPricingPage>,
}

/// An option a visitor switches to: plans name an option alike by its alias.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageOption {
    pub alias: String,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct PricingPage {
    pub product_family_id: ProductFamilyId,
    pub product_family_name: String,
    pub currency: String,
    /// The alias of the option shown; none when no plan of the ladder is sold in the currency.
    pub option: Option<String>,
    /// The options a visitor switches between: those two priced plans of the ladder sell in the
    /// currency at least (when none is shared, every one), hidden ones never; in ladder order.
    pub options: Vec<PageOption>,
    /// The currencies a visitor switches between: those the ladder is sold in.
    pub currencies: Vec<String>,
    pub cards: Vec<PricingCard>,
    /// The plans of the ladder left off the page: not sold in the currency, and not sold by
    /// contacting sales.
    pub not_sold: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct PricingCard {
    pub plan_id: PlanId,
    pub plan_name: String,
    pub tagline: Option<String>,
    pub popular: bool,
    pub cta_label: Option<String>,
    pub free: bool,
    /// None when the plan is not sold in the currency.
    pub price: Option<CardPrice>,
    /// The aliases of the options the plan sells in the currency.
    pub options: Vec<String>,
    pub includes_plan_name: Option<String>,
    pub highlights: Vec<RenderedHighlight>,
    pub trial_days: Option<u32>,
    pub sales_led: bool,
    /// The price is not shown: `price` is none and the card reads "Custom pricing".
    pub hide_price: bool,
    pub contact_url: Option<String>,
    /// The plan does not sell the option shown: its default is priced instead, and this reads
    /// under the price ("Billed monthly", "Annual only").
    pub option_note: Option<String>,
}

#[derive(Clone, Debug)]
pub struct CardPrice {
    pub amount: Decimal,
    /// None for a plan billing only usage or one-time charges.
    pub period: Option<BillingPeriodEnum>,
    pub monthly_equivalent: Option<Decimal>,
    pub saving_percent: Option<u32>,
    /// The amount is the lowest of several levels: "from".
    pub from_lowest: bool,
    /// A charge could not be shown: never read as free.
    pub partial: bool,
    pub extras: Vec<CardExtra>,
    /// The usage levels a visitor picks between, the card's amount and extras reading the lowest.
    pub levels: Option<CardLevels>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardExtraKind {
    /// The fixed fees billed on the card's period, summed: its `CardPrice::amount`.
    Base,
    PerSeat,
    Usage,
    OneTime,
    CreditPack,
    /// A fixed fee billed on its own period, shown beside the card's amount.
    Recurring,
}

/// An item of a card, in the order it reads (`price_headline::in_order`).
#[derive(Clone, Debug)]
pub struct CardExtra {
    pub kind: CardExtraKind,
    /// The charge's name; empty for the base price.
    pub name: String,
    /// The base price; per seat or unit, for one price per unit (`from_lowest`, `included`,
    /// `block_size` read with it); once; a pack's. None for a rate card, rates by dimension,
    /// levels whose overage varies.
    pub amount: Option<Decimal>,
    /// `amount` is the lowest rate of its tiers: "from".
    pub from_lowest: bool,
    /// Units free before `amount` applies ("3 seats included"); a capacity's level.
    pub included: Option<u64>,
    /// `amount` is per this many units: "$30 / 10 units".
    pub block_size: Option<u64>,
    pub unit: Option<String>,
    /// How often it is charged, for a recurring extra.
    pub period: Option<BillingPeriodEnum>,
    /// Its products: the charge's, the base price's fees, the usage a "+ usage" line folds.
    pub product_ids: Vec<ProductId>,
    /// It can be the card's price (`price_headline::in_order`); never leaves the store.
    pub can_lead: bool,
    /// The card's price: the first item that can be, the others reading beside it.
    pub lead: bool,
}

/// A plan's usage-levels charge (several capacity prices in one cell) as its card offers it: the
/// visitor picks a level, and the card's amount, its limit and overage read that level's. Offered
/// for a card with one such charge only; with several, the card reads "from" their lowest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardLevels {
    /// The charge's name.
    pub name: String,
    /// By the usage they include, smallest first.
    pub levels: Vec<CardLevel>,
    /// What the charge counts, as its metric names it: a page reads usage counting money in the
    /// card's currency as an amount, and its overage as a share of it.
    pub metric_name: Option<String>,
    pub unit_singular: Option<String>,
    pub unit_plural: Option<String>,
    pub unit_conversion_factor: Option<i32>,
    /// A last stop past the largest level, sold by contacting sales; none when the ladder sells
    /// nothing so.
    pub more: Option<LevelsMore>,
    /// The charge's product, to tell its extra; never leaves the store.
    pub product_id: Option<ProductId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardLevel {
    /// The usage included each period, in the metric's unit: what a sign-up commits to.
    pub included: u64,
    /// The card's amount with this level: its fixed amount, this level's fee instead of the
    /// lowest.
    pub amount: Decimal,
    /// `amount` per month, for a card billed on a longer period.
    pub monthly_equivalent: Option<Decimal>,
    /// Per unit used above `included`; none when priced in a credit unit or left off the card.
    pub overage_rate: Option<Decimal>,
}

/// Where a card's "More" stop leads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LevelsMore {
    pub contact_url: Option<String>,
    /// The sales-led plan of the ladder it leads to; none when it is the card's own.
    pub plan_name: Option<String>,
}

/// A plan of a ladder as the "More" stop of its levels reads it.
pub(crate) struct LadderSeller<'a> {
    pub name: &'a str,
    pub sales_led: bool,
    pub contact_url: Option<&'a str>,
}

/// Where the levels of `ladder[at]`'s card go past their largest: the plan's own sales contact
/// when sold by contacting sales, else the next sales-led plan up the ladder, else the nearest
/// below; none when the ladder sells nothing by contacting sales.
pub(crate) fn levels_more(ladder: &[LadderSeller<'_>], at: usize) -> Option<LevelsMore> {
    let own = ladder.get(at)?;
    if own.sales_led {
        return Some(LevelsMore {
            contact_url: own.contact_url.map(str::to_owned),
            plan_name: None,
        });
    }
    let seller = ladder[at + 1..]
        .iter()
        .chain(ladder[..at].iter().rev())
        .find(|plan| plan.sales_led)?;
    Some(LevelsMore {
        contact_url: seller.contact_url.map(str::to_owned),
        plan_name: Some(seller.name.to_owned()),
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderedHighlight {
    pub text: String,
    pub feature_id: Option<FeatureId>,
}

/// `amount` at the currency's precision ("149.00") when that loses nothing; a finer unit rate
/// ("0.0015") keeps its digits.
pub fn money(amount: Decimal, precision: Option<u8>) -> Decimal {
    let Some(precision) = precision.map(u32::from) else {
        return amount.normalize();
    };
    let mut rounded = amount.round_dp(precision);
    if rounded != amount {
        return amount.normalize();
    }
    rounded.rescale(precision);
    rounded
}

/// `value` with thousands separators and no trailing zeros: "10,000", "2.5".
pub fn format_quantity(value: Decimal) -> String {
    let normalized = value.normalize();
    let text = normalized.abs().to_string();
    let (int_part, frac_part) = match text.split_once('.') {
        Some((int_part, frac)) => (int_part.to_owned(), Some(frac.to_owned())),
        None => (text.clone(), None),
    };
    let mut grouped = String::with_capacity(int_part.len() + int_part.len() / 3);
    for (index, digit) in int_part.chars().enumerate() {
        if index > 0 && (int_part.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    let sign = if normalized.is_sign_negative() && !normalized.is_zero() {
        "-"
    } else {
        ""
    };
    match frac_part {
        Some(frac) => format!("{sign}{grouped}.{frac}"),
        None => format!("{sign}{grouped}"),
    }
}

/// Per month, for a period longer than a month, at the currency's precision (2 when unknown).
pub fn monthly_equivalent(
    amount: Decimal,
    period: BillingPeriodEnum,
    precision: Option<u8>,
) -> Option<Decimal> {
    let months = period.as_months();
    let precision = u32::from(precision.unwrap_or(2));
    (months > 1).then(|| {
        let mut monthly = (amount / Decimal::from(months)).round_dp(precision);
        monthly.rescale(precision);
        monthly
    })
}

/// Whole percent saved paying `amount` a `period` instead of `monthly` each month, rounded down;
/// none unless something is saved. Both amounts are the ones shown, rounded alike.
pub fn saving_percent(amount: Decimal, period: BillingPeriodEnum, monthly: Decimal) -> Option<u32> {
    let months = period.as_months();
    if months <= 1 || monthly <= Decimal::ZERO {
        return None;
    }
    let full = monthly * Decimal::from(months);
    let saved = ((full - amount) / full * Decimal::from(100)).floor();
    saved.to_u32().filter(|percent| *percent > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn amounts_and_quantities_read_as_a_page_shows_them() {
        assert_eq!(money(dec!(149), Some(2)).to_string(), "149.00");
        assert_eq!(money(dec!(0.0015), Some(2)).to_string(), "0.0015");
        assert_eq!(money(dec!(12.50), None).to_string(), "12.5");
        assert_eq!(format_quantity(dec!(10000)), "10,000");
        assert_eq!(format_quantity(dec!(1234567.50)), "1,234,567.5");
        assert_eq!(format_quantity(dec!(999)), "999");
    }

    #[test]
    fn a_year_paid_up_front_shows_its_month_and_its_saving() {
        assert_eq!(
            monthly_equivalent(dec!(200), BillingPeriodEnum::Annual, Some(2)),
            Some(dec!(16.67))
        );
        assert_eq!(
            monthly_equivalent(dec!(2000), BillingPeriodEnum::Annual, Some(0))
                .map(|d| d.to_string()),
            Some("167".to_string()),
            "a currency without minor units"
        );
        assert_eq!(
            monthly_equivalent(dec!(20), BillingPeriodEnum::Monthly, Some(2)),
            None
        );
        assert_eq!(
            saving_percent(dec!(200), BillingPeriodEnum::Annual, dec!(20)),
            Some(16)
        );
        assert_eq!(
            saving_percent(dec!(240), BillingPeriodEnum::Annual, dec!(20)),
            None
        );
        assert_eq!(
            saving_percent(dec!(300), BillingPeriodEnum::Annual, dec!(20)),
            None
        );
    }

    #[test]
    fn levels_go_past_their_largest_by_contacting_sales_when_the_ladder_sells_so() {
        let seller = |name, sales_led, contact_url| LadderSeller {
            name,
            sales_led,
            contact_url,
        };
        let ladder = [
            seller("Starter", false, None),
            seller("Volume", false, None),
            seller("Scale", false, None),
            seller("Enterprise", true, Some("https://example.com/sales")),
        ];
        assert_eq!(
            levels_more(&ladder, 1),
            Some(LevelsMore {
                contact_url: Some("https://example.com/sales".to_owned()),
                plan_name: Some("Enterprise".to_owned()),
            }),
            "the sales-led plan up the ladder"
        );

        let own = [
            seller("Partners", true, None),
            seller("Volume", true, Some("https://example.com/volume")),
        ];
        assert_eq!(
            levels_more(&own, 1),
            Some(LevelsMore {
                contact_url: Some("https://example.com/volume".to_owned()),
                plan_name: None,
            }),
            "a sales-led plan's own contact first"
        );

        let below = [
            seller("Custom", true, None),
            seller("Volume", false, None),
            seller("Max", false, None),
        ];
        assert_eq!(
            levels_more(&below, 1),
            Some(LevelsMore {
                contact_url: None,
                plan_name: Some("Custom".to_owned()),
            }),
            "else the nearest below, linked or not"
        );

        let self_serve = [seller("Volume", false, None), seller("Max", false, None)];
        assert_eq!(
            levels_more(&self_serve, 0),
            None,
            "nothing sold by contacting sales"
        );
        assert_eq!(levels_more(&self_serve, 5), None);
    }

    #[test]
    fn a_card_leads_with_its_base_price_seats_or_usage_per_unit_and_hides_any_charge() {
        use common_domain::ids::{BaseId, BillableMetricId};
        let metric_id = BillableMetricId::new();
        let read = |fee: FeeStructure| {
            let charge = PricingCharge::of(ProductId::new(), "Charge".to_owned(), &fee);
            (charge.kind, charge.can_lead, charge.can_hide)
        };
        assert_eq!(
            read(FeeStructure::Rate {}),
            (PricingChargeKind::Fixed, true, true),
            "as the base price"
        );
        assert_eq!(
            read(FeeStructure::Usage {
                metric_id,
                model: UsageModel::PerUnit,
            }),
            (PricingChargeKind::Usage, true, true)
        );
        assert_eq!(
            read(FeeStructure::Usage {
                metric_id,
                model: UsageModel::Tiered,
            }),
            (PricingChargeKind::Usage, true, true),
            "from its lowest rate, or after its free tier"
        );
        assert_eq!(
            read(FeeStructure::Usage {
                metric_id,
                model: UsageModel::RateCard,
            }),
            (PricingChargeKind::Usage, false, true),
            "no one unit price to lead with"
        );
        assert_eq!(
            read(FeeStructure::Capacity { metric_id }),
            (PricingChargeKind::Usage, false, true),
            "its fee is in the base price"
        );
        assert_eq!(
            read(FeeStructure::OneTime {}),
            (PricingChargeKind::OneTime, false, true)
        );
        assert_eq!(
            read(FeeStructure::Credit {}),
            (PricingChargeKind::CreditPack, false, true)
        );
    }
}
