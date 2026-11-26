//! A charge as v2 speaks it (ADR-0013 §5): a product billed by a quantity, on a timing, at prices
//! per option, currency and cadence — never an engine fee type. [`to_domain`] maps a charge onto
//! the charge's `FeeStructure` and its prices' `Pricing`, refusing what the domain cannot
//! express; [`render`] reads a stored charge back as its canonical composition. Plans, add-ons
//! and (next) subscription extras share both.
use crate::v2::bounds;
use crate::v2::problem::Problem;
use common_domain::ids::{
    BillableMetricId, CreditUnitId, ManagedQuantityId, ProductId, RateCardId,
};
use meteroid_store::domain::credit_units::PriceCreditUnit as DomainCreditUnit;
use meteroid_store::domain::enums::{BillingPeriodEnum, BillingType};
use meteroid_store::domain::price_components::{
    CreditExpiry as DomainExpiry, CreditRollover as DomainRollover, DimensionRate as DomainRate,
    DimensionRates as DomainRates, DowngradePolicy, PriceInput, RatingModel, TierRow,
    UpgradePolicy, UsagePricingModel,
};
use meteroid_store::domain::prices::{FeeStructure, Pricing, UsageModel};
use meteroid_store::domain::rate_cards::RateCardUnmatched;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

/// The option of a price sold in every option of the version.
pub const EVERY_OPTION: &str = "*";

fn every_option() -> String {
    EVERY_OPTION.to_owned()
}

// ── Model ───────────────────────────────────────────────────────────

/// How often a price bills: `monthly`, `quarterly`, `semiannual` (every 6 months) or `annual`.
/// Usage and one-time prices read their usage or fee whatever it is.
#[derive(
    Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum Cadence {
    #[default]
    Monthly,
    Quarterly,
    Semiannual,
    Annual,
}

impl Cadence {
    /// As the API writes it.
    pub fn as_str(self) -> &'static str {
        match self {
            Cadence::Monthly => "monthly",
            Cadence::Quarterly => "quarterly",
            Cadence::Semiannual => "semiannual",
            Cadence::Annual => "annual",
        }
    }
}

impl From<Cadence> for BillingPeriodEnum {
    fn from(cadence: Cadence) -> Self {
        match cadence {
            Cadence::Monthly => BillingPeriodEnum::Monthly,
            Cadence::Quarterly => BillingPeriodEnum::Quarterly,
            Cadence::Semiannual => BillingPeriodEnum::Semiannual,
            Cadence::Annual => BillingPeriodEnum::Annual,
        }
    }
}

impl From<BillingPeriodEnum> for Cadence {
    fn from(period: BillingPeriodEnum) -> Self {
        match period {
            BillingPeriodEnum::Monthly => Cadence::Monthly,
            BillingPeriodEnum::Quarterly => Cadence::Quarterly,
            BillingPeriodEnum::Semiannual => Cadence::Semiannual,
            BillingPeriodEnum::Annual => Cadence::Annual,
        }
    }
}

/// Whether a charge bills each period (`recurring`) or once, on the first invoice (`one_time`).
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Recurrence {
    /// Billed each period.
    Recurring,
    /// Billed once, on the first invoice.
    OneTime,
}

/// When in its period a charge bills: `advance`, at the start of the period it pays for;
/// `arrears`, at the end, for what it used.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Billing {
    /// At the start of the period it pays for.
    Advance,
    /// At the end of the period, for what it used.
    Arrears,
}

/// When a charge bills. Left out of a request, a metered charge bills recurring in arrears
/// (in advance when priced in levels), any other recurring in advance.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Timing {
    /// Each period, or once.
    pub recurrence: Recurrence,
    /// At the start of the period, or at its end. A one-time charge, seats and usage levels
    /// bill in advance; other usage in arrears.
    pub billing: Billing,
}

impl Timing {
    const RECURRING_ADVANCE: Timing = Timing {
        recurrence: Recurrence::Recurring,
        billing: Billing::Advance,
    };
    const RECURRING_ARREARS: Timing = Timing {
        recurrence: Recurrence::Recurring,
        billing: Billing::Arrears,
    };
    const ONE_TIME: Timing = Timing {
        recurrence: Recurrence::OneTime,
        billing: Billing::Advance,
    };
}

/// Sugar for a common composition: it fills `quantity` and `timing`, and refuses an explicit one
/// contradicting it (`preset_conflict`). Responses name the preset a charge reads as, a hint
/// only. `flat_fee`: a fixed recurring fee, one unit billed in advance; `seats`: per seat of a
/// managed quantity, in advance; `usage`: metered usage, in arrears; `usage_levels`: usage sold
/// in levels a buyer picks, in advance; `one_time`: a fee billed once; `credit_pack`: a pack of
/// credit, its price buying `grants`.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    /// A fixed recurring fee: one unit, billed in advance.
    FlatFee,
    /// Per seat of a managed quantity, billed in advance.
    Seats,
    /// Metered usage, billed in arrears.
    Usage,
    /// Usage sold in levels a buyer picks (each a fee including some usage, with an overage
    /// rate), billed in advance.
    UsageLevels,
    /// A fee billed once.
    OneTime,
    /// A pack of credit: its price buys `grants` of credit.
    CreditPack,
}

/// A count set on the plan: the same for every subscription.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FixedQuantity {
    /// A whole number of units, 1 or more.
    #[schema(value_type = String, format = "decimal", example = "1")]
    pub value: Decimal,
}

/// A count each subscription holds and changes (seats), shared by the charges naming it.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ManagedQuantityRef {
    /// The managed quantity, by code or id.
    pub quantity: String,
    /// Refused: a subscription sets its starting count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial: Option<u32>,
    /// The fewest units a subscription holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<u32>,
    /// The most units a subscription holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<u32>,
}

/// Usage measured by a metric.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MeteredQuantity {
    /// The metric, by code or id.
    pub metric: String,
}

/// How many units a charge bills, by `type`: `fixed`, a count set on the plan; `managed`, a
/// count each subscription holds and changes (seats); `metered`, the usage a metric measures.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Quantity {
    Fixed(FixedQuantity),
    Managed(ManagedQuantityRef),
    Metered(MeteredQuantity),
}

/// A tier of a graduated or volume price: it runs from `first_unit` up to the next tier's.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Tier {
    /// The first unit the tier prices; the first tier starts at 0, the next ones strictly above.
    pub first_unit: u64,
    /// Per unit in the tier, a decimal string in the currency's major unit.
    #[schema(value_type = String, format = "decimal")]
    pub unit_price: Decimal,
    /// A fee for reaching the tier, added to its units' amount.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub flat_fee: Option<Decimal>,
    /// The most the tier bills: its units' amount and `flat_fee` together, capped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub flat_cap: Option<Decimal>,
}

/// Every unit at one price.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PerUnitModel {
    /// Per unit, a decimal string in the currency's major unit. Negative only on metered usage
    /// priced in money: a reversal, billed as a negative line.
    #[schema(value_type = String, format = "decimal", example = "0.002")]
    pub unit_price: Decimal,
}

/// Each unit priced by the tier it falls in.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GraduatedModel {
    /// 1 to 100 tiers, the first starting at unit 0, ascending.
    pub tiers: Vec<Tier>,
}

/// Every unit priced by the tier the total reaches.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct VolumeModel {
    /// 1 to 100 tiers, the first starting at unit 0, ascending.
    pub tiers: Vec<Tier>,
}

/// Units sold by packages, a started one billed whole.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PackageModel {
    /// Units a package holds, 1 or more.
    pub package_size: u64,
    /// Per package, a decimal string in the currency's major unit.
    #[schema(value_type = String, format = "decimal")]
    pub package_price: Decimal,
}

/// One amount, whatever the quantity (which is 1).
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlatModel {
    /// A decimal string in the currency's major unit.
    #[schema(value_type = String, format = "decimal", example = "49")]
    pub amount: Decimal,
}

/// What happens to usage no rate matches: `hold` (the default), the invoice waits until the
/// usage is rated; `ignore`, the usage is left unbilled.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Unmatched {
    /// The invoice waits until the usage is rated.
    #[default]
    Hold,
    /// The usage is left unbilled.
    Ignore,
}

/// A rate of a dimension rates price: the usage whose dimensions match `key`, at `unit_price`.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DimensionRate {
    /// A pattern per dimension, in the order of the price's `dimensions`: a value, a prefix
    /// ending in `*`, or `*`. Two rates never share a key.
    pub key: Vec<String>,
    /// Per `unit_size` units, a decimal string in the currency's major unit.
    #[schema(value_type = String, format = "decimal")]
    pub unit_price: Decimal,
    /// A name for the rate, kept with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Usage rated by its dimensions at rates the price holds, the most specific key winning.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DimensionRatesModel {
    /// The usage event properties the rates are keyed by, in key order: 1 to 4.
    pub dimensions: Vec<String>,
    /// Rates are per this many units (1000000: per 1M tokens).
    #[serde(default = "one")]
    #[schema(value_type = String, format = "decimal")]
    pub unit_size: Decimal,
    /// What happens to usage no rate matches; `hold` when left out.
    #[serde(default)]
    pub on_unmatched: Unmatched,
    /// 1,000 rates at most.
    pub rates: Vec<DimensionRate>,
}

fn one() -> Decimal {
    Decimal::ONE
}

/// Usage rated by a rate card's dated rates.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RateCardModel {
    /// The rate card, by code or id.
    pub rate_card: String,
    /// Scales every rate of the card (1.2: 20% over the card).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub multiplier: Option<Decimal>,
}

/// A level of a price sold in levels: a fee each period including some usage, with an overage
/// rate beyond it.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Level {
    /// The usage the level includes each period.
    pub included: u64,
    /// The level's fee each period, billed in advance; a decimal string in the currency's major
    /// unit.
    #[schema(value_type = String, format = "decimal")]
    pub price: Decimal,
    /// Per unit used beyond `included`, billed in arrears; a decimal string in the currency's
    /// major unit.
    #[schema(value_type = String, format = "decimal")]
    pub overage_unit_price: Decimal,
}

/// Levels a buyer picks between, fewest units first.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LevelsModel {
    /// 1 to 100 levels, `included` strictly ascending.
    pub levels: Vec<Level>,
}

/// How a price turns the quantity into an amount, by `type`: `per_unit`, every unit at one
/// price; `graduated`, each unit at the tier it falls in; `volume`, every unit at the tier the
/// total reaches; `package`, by packages, a started one billed whole; `flat`, one amount;
/// `dimension_rates`, usage at rates by its dimensions; `rate_card`, usage at a rate card's
/// dated rates; `levels`, levels a buyer picks. `flat` prices a quantity of 1 only;
/// `dimension_rates`, `rate_card` and `levels` metered usage only.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PriceModel {
    PerUnit(PerUnitModel),
    Graduated(GraduatedModel),
    Volume(VolumeModel),
    Package(PackageModel),
    Flat(FlatModel),
    DimensionRates(DimensionRatesModel),
    RateCard(RateCardModel),
    Levels(LevelsModel),
}

impl PriceModel {
    fn name(&self) -> &'static str {
        match self {
            PriceModel::PerUnit(_) => "per_unit",
            PriceModel::Graduated(_) => "graduated",
            PriceModel::Volume(_) => "volume",
            PriceModel::Package(_) => "package",
            PriceModel::Flat(_) => "flat",
            PriceModel::DimensionRates(_) => "dimension_rates",
            PriceModel::RateCard(_) => "rate_card",
            PriceModel::Levels(_) => "levels",
        }
    }
}

/// A usage or overage price expressed in a credit unit: its amounts are credits, each worth
/// `rate` of the price currency when billed.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PriceCreditUnit {
    /// The credit unit the price's amounts are in.
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub unit: CreditUnitId,
    /// What one credit is worth when billed, a decimal string in the price currency's major
    /// unit; not negative.
    #[schema(value_type = String, format = "decimal")]
    pub rate: Decimal,
}

/// What an expiry counts in: `days`, `months`, or `billing_period` (at the end of the period
/// invoiced, without a `value`).
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExpiryUnit {
    Days,
    Months,
    /// At the end of the period invoiced; no `value`.
    BillingPeriod,
}

/// When granted credit expires: so many days or months after it is granted, or at the end of
/// the period invoiced.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Expiry {
    /// Days, months, or the billing period.
    pub unit: ExpiryUnit,
    /// How many days or months; required with them, refused with `billing_period`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<u32>,
}

/// Included credit left at expiry, carried once into a new grant.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Rollover {
    /// Cap per rolled grant, positive, in the grant's currency or credit unit; none carries all
    /// that is left.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub max_amount: Option<Decimal>,
    /// When the rolled-over credit expires.
    pub expires_after: Expiry,
}

/// The credit one unit of a credit pack grants once paid.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CreditGrant {
    /// Positive, a decimal string in the price's currency (major unit), or in `credit_unit`
    /// when set.
    #[schema(value_type = String, format = "decimal")]
    pub amount: Decimal,
    /// The credit unit granted; none: credit in the price's currency.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(serialize_with = "common_domain::ids::string_serde_opt::serialize")]
    pub credit_unit: Option<CreditUnitId>,
    /// When the granted credit expires; none: never.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_after: Option<Expiry>,
    /// What is left at expiry carried into a new grant; none: nothing carries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollover: Option<Rollover>,
}

/// A price of a charge in an option, a currency and a cadence.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ChargePrice {
    /// The option's alias, or `*` for every option of the version. An add-on's are `*`.
    #[serde(default = "every_option")]
    pub option: String,
    /// ISO 4217, read upper-case. One price per option, currency and cadence; one cadence per
    /// option and currency.
    pub currency: String,
    /// Monthly when left out.
    #[serde(default)]
    pub cadence: Cadence,
    /// How the price turns the charge's quantity into an amount.
    pub model: PriceModel,
    /// The credit unit the price's amounts are in; none: money.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_unit: Option<PriceCreditUnit>,
    /// A credit pack's grant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grants: Option<CreditGrant>,
}

/// A product billed by a quantity, on a timing, at its prices. Responses always carry the
/// canonical composition (`quantity`, `timing`, `prices`) with `preset` as a hint; a request
/// may send a `preset` instead of `quantity` and `timing`. A charge without a quantity, a preset
/// or a price includes its product for free.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
// Unknown fields are refused by the body extractor, not here: a subscription's charge flattens
// it, and a schema denying them would refuse that charge's own fields.
pub struct Charge {
    /// The product, by code or id; responses name it by code.
    pub product: String,
    /// Unique within the version; the product's name when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The common composition the charge reads as; on a request, it fills `quantity` and
    /// `timing`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<Preset>,
    /// How many units it bills; none on a charge that includes its product for free.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    /// Usage free each period before a per-unit price applies (metered quantities only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub included: Option<Decimal>,
    /// When it bills; left out of a request, as the quantity implies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timing: Option<Timing>,
    /// One per option, currency and cadence, 100 at most; ordered by option position, currency,
    /// cadence. Empty: the product is included for free.
    #[serde(default)]
    pub prices: Vec<ChargePrice>,
}

// ── To the domain ───────────────────────────────────────────────────

/// What a charge's references resolved to.
#[derive(Debug, Clone)]
pub struct ChargeRefs {
    pub product_id: ProductId,
    pub product_name: String,
    pub metric_id: Option<BillableMetricId>,
    /// The managed quantity and its name.
    pub quantity: Option<(ManagedQuantityId, String)>,
    /// By the reference a price sent.
    pub rate_cards: HashMap<String, RateCardId>,
}

/// A charge as the domain holds it: its structure (none: an included product) and its prices,
/// each with the option it is in (`*`: every one).
#[derive(Debug, Clone)]
pub struct DomainCharge {
    pub name: String,
    pub product_id: ProductId,
    pub structure: Option<FeeStructure>,
    pub prices: Vec<(PriceInput, String)>,
}

fn whole(value: Decimal, path: &str, what: &str) -> Result<u64, Problem> {
    let normalized = value.normalize();
    if normalized.is_sign_negative() || normalized.scale() > 0 {
        return Err(Problem::field(
            path,
            "invalid",
            format!("{what} is a whole number of units"),
        ));
    }
    u64::try_from(normalized.mantissa())
        .map_err(|_| Problem::field(path, "invalid", format!("{what} is too large")))
}

/// The quantity and timing a charge bills on, its preset applied.
fn shape(charge: &Charge, path: &str) -> Result<(Option<Quantity>, Timing), Problem> {
    let conflict = |field: &str, message: String| {
        Problem::field(format!("{path}/{field}"), "preset_conflict", message)
    };
    let levels = charge
        .prices
        .iter()
        .any(|price| matches!(price.model, PriceModel::Levels(_)));
    let one = || {
        Quantity::Fixed(FixedQuantity {
            value: Decimal::ONE,
        })
    };
    let Some(preset) = charge.preset else {
        let timing = charge.timing.unwrap_or(match &charge.quantity {
            Some(Quantity::Metered(_)) if !levels => Timing::RECURRING_ARREARS,
            _ => Timing::RECURRING_ADVANCE,
        });
        return Ok((charge.quantity.clone(), timing));
    };
    let (quantity, timing) = match preset {
        Preset::FlatFee => match &charge.quantity {
            None | Some(Quantity::Fixed(_)) => (
                charge.quantity.clone().unwrap_or_else(one),
                Timing::RECURRING_ADVANCE,
            ),
            Some(_) => {
                return Err(conflict(
                    "quantity",
                    "a flat fee has a fixed quantity".to_owned(),
                ));
            }
        },
        Preset::OneTime => match &charge.quantity {
            None | Some(Quantity::Fixed(_)) => (
                charge.quantity.clone().unwrap_or_else(one),
                Timing::ONE_TIME,
            ),
            Some(_) => {
                return Err(conflict(
                    "quantity",
                    "a one-time fee has a fixed quantity".to_owned(),
                ));
            }
        },
        Preset::CreditPack => match &charge.quantity {
            None | Some(Quantity::Fixed(_)) => (
                charge.quantity.clone().unwrap_or_else(one),
                match charge.timing {
                    Some(timing) if timing.recurrence == Recurrence::OneTime => Timing::ONE_TIME,
                    _ => Timing::RECURRING_ADVANCE,
                },
            ),
            Some(_) => {
                return Err(conflict(
                    "quantity",
                    "a credit pack has a fixed quantity of packs".to_owned(),
                ));
            }
        },
        Preset::Seats => match &charge.quantity {
            Some(quantity @ Quantity::Managed(_)) => (quantity.clone(), Timing::RECURRING_ADVANCE),
            _ => {
                return Err(conflict(
                    "quantity",
                    "seats count a managed quantity: send {\"type\": \"managed\", \"quantity\": <code>}"
                        .to_owned(),
                ));
            }
        },
        Preset::Usage | Preset::UsageLevels => match &charge.quantity {
            Some(quantity @ Quantity::Metered(_)) => (
                quantity.clone(),
                if preset == Preset::UsageLevels {
                    Timing::RECURRING_ADVANCE
                } else {
                    Timing::RECURRING_ARREARS
                },
            ),
            _ => {
                return Err(conflict(
                    "quantity",
                    "usage is metered: send {\"type\": \"metered\", \"metric\": <code>}".to_owned(),
                ));
            }
        },
    };
    if let Some(asked) = charge.timing
        && asked != timing
    {
        return Err(conflict(
            "timing",
            format!("the {preset:?} preset bills {timing:?}, not {asked:?}"),
        ));
    }
    if preset == Preset::UsageLevels && !levels {
        return Err(conflict(
            "prices",
            "usage levels are priced with the levels model".to_owned(),
        ));
    }
    if preset == Preset::CreditPack && charge.prices.iter().any(|p| p.grants.is_none()) {
        return Err(conflict(
            "prices",
            "a credit pack's prices each say what they grant".to_owned(),
        ));
    }
    Ok((Some(quantity), timing))
}

/// The most prices a charge holds (each an option, a currency and a cadence).
pub const MAX_PRICES: usize = 100;
/// The most tiers, levels and rates by dimension one price holds.
pub const MAX_TIERS: usize = 100;
pub const MAX_LEVELS: usize = 100;
pub const MAX_DIMENSION_RATES: usize = 1000;

/// What every price written checks, whatever the charge and wherever it is written (a plan
/// draft, an add-on, a subscription's own or overridden charge), each refusal at its pointer
/// (`path` is the price's): a currency Meteroid knows (ISO 4217), no negative amount (a price,
/// a rate, a tier's amounts, a level's fee and overage, a credit unit's rate, a grant) but a
/// per-unit rate on usage (`metered`: a reversal, R-142), a positive rate card multiplier and
/// unit size, and lists within their bounds.
/// The schema of amounts by currency (ISO 4217 code), each a decimal string in the currency's
/// major unit; `patch`: the map and each amount may be `null` (a merge patch removing it).
pub fn decimal_map(
    description: &str,
    patch: bool,
) -> utoipa::openapi::RefOr<utoipa::openapi::Schema> {
    use utoipa::openapi::schema::{ObjectBuilder, SchemaFormat, SchemaType, Type};
    let kind = |kind: Type| match patch {
        true => SchemaType::from_iter([kind, Type::Null]),
        false => SchemaType::Type(kind),
    };
    ObjectBuilder::new()
        .schema_type(kind(Type::Object))
        .description(Some(description))
        .additional_properties(Some(
            ObjectBuilder::new()
                .schema_type(kind(Type::String))
                .format(Some(SchemaFormat::Custom("decimal".to_owned()))),
        ))
        .into()
}

pub fn check_price(price: &ChargePrice, metered: bool, path: &str) -> Result<(), Problem> {
    let currency = price.currency.trim().to_ascii_uppercase();
    if meteroid_store::constants::Currencies::resolve_currency(&currency).is_none() {
        return Err(Problem::field(
            format!("{path}/currency"),
            "invalid",
            format!(
                "'{}' is not a currency Meteroid supports: an ISO 4217 code (EUR, USD…)",
                price.currency.trim()
            ),
        ));
    }
    let model = format!("{path}/model");
    let mut amounts: Vec<(String, Decimal)> = Vec::new();
    match &price.model {
        // A per-unit rate on usage may be negative: a reversal (R-142). On seats or a fixed
        // quantity the pricer would bill a negative amount at 0.
        // In money only: a reversal is not decided for a price drawing credits.
        PriceModel::PerUnit(_) if metered && price.credit_unit.is_none() => {}
        PriceModel::PerUnit(m) => amounts.push((format!("{model}/unit_price"), m.unit_price)),
        PriceModel::Graduated(GraduatedModel { tiers })
        | PriceModel::Volume(VolumeModel { tiers }) => {
            bounds::count(tiers.len(), MAX_TIERS, &format!("{model}/tiers"), "tiers")?;
            for (index, tier) in tiers.iter().enumerate() {
                let at = |field: &str| format!("{model}/tiers/{index}/{field}");
                amounts.push((at("unit_price"), tier.unit_price));
                amounts.extend(tier.flat_fee.map(|fee| (at("flat_fee"), fee)));
                amounts.extend(tier.flat_cap.map(|cap| (at("flat_cap"), cap)));
            }
        }
        PriceModel::Package(m) => amounts.push((format!("{model}/package_price"), m.package_price)),
        PriceModel::Flat(m) => amounts.push((format!("{model}/amount"), m.amount)),
        PriceModel::DimensionRates(m) => {
            bounds::count(
                m.rates.len(),
                MAX_DIMENSION_RATES,
                &format!("{model}/rates"),
                "rates",
            )?;
            if m.unit_size <= Decimal::ZERO {
                return Err(Problem::field(
                    format!("{model}/unit_size"),
                    "invalid",
                    "rates are per a positive number of units",
                ));
            }
            for (index, rate) in m.rates.iter().enumerate() {
                amounts.push((format!("{model}/rates/{index}/unit_price"), rate.unit_price));
            }
        }
        PriceModel::RateCard(m) => {
            if m.multiplier
                .is_some_and(|multiplier| multiplier <= Decimal::ZERO)
            {
                return Err(Problem::field(
                    format!("{model}/multiplier"),
                    "invalid",
                    "a rate card's multiplier is positive",
                ));
            }
        }
        PriceModel::Levels(m) => {
            bounds::count(
                m.levels.len(),
                MAX_LEVELS,
                &format!("{model}/levels"),
                "levels",
            )?;
            for (index, level) in m.levels.iter().enumerate() {
                let at = |field: &str| format!("{model}/levels/{index}/{field}");
                amounts.push((at("price"), level.price));
                amounts.push((at("overage_unit_price"), level.overage_unit_price));
            }
        }
    }
    if let Some(unit) = &price.credit_unit {
        amounts.push((format!("{path}/credit_unit/rate"), unit.rate));
    }
    if let Some(grant) = &price.grants {
        if grant.amount <= Decimal::ZERO {
            return Err(Problem::field(
                format!("{path}/grants/amount"),
                "invalid",
                "a credit pack grants a positive amount of credit",
            ));
        }
        if let Some(max) = grant
            .rollover
            .as_ref()
            .and_then(|rollover| rollover.max_amount)
            && max <= Decimal::ZERO
        {
            return Err(Problem::field(
                format!("{path}/grants/rollover/max_amount"),
                "invalid",
                "a rollover cap is positive",
            ));
        }
    }
    if let Some((at, _)) = amounts
        .into_iter()
        .find(|(_, amount)| *amount < Decimal::ZERO)
    {
        return Err(Problem::field(at, "invalid", "a price is not negative"));
    }
    Ok(())
}

/// Refuses tiers the engine would misprice: none, a first tier starting past unit 0, or starts
/// not strictly ascending (a negative amount is refused by [`check_price`]). `path` is the
/// price's JSON pointer.
fn check_tiers(tiers: &[Tier], path: &str) -> Result<(), Problem> {
    let at = |index: usize, field: &str| format!("{path}/model/tiers/{index}/{field}");
    if tiers.is_empty() {
        return Err(Problem::field(
            format!("{path}/model/tiers"),
            "required",
            "one tier at least",
        ));
    }
    for (index, tier) in tiers.iter().enumerate() {
        if index == 0 && tier.first_unit != 0 {
            return Err(Problem::field(
                at(0, "first_unit"),
                "invalid",
                "the first tier starts at unit 0",
            ));
        }
        if index > 0 && tier.first_unit <= tiers[index - 1].first_unit {
            return Err(Problem::field(
                at(index, "first_unit"),
                "invalid",
                "tiers start on strictly ascending units",
            ));
        }
    }
    Ok(())
}

/// Refuses a package of no unit, which the store refuses too.
fn check_package(size: u64, path: &str) -> Result<(), Problem> {
    if size == 0 {
        return Err(Problem::field(
            format!("{path}/model/package_size"),
            "invalid",
            "a package holds one unit at least",
        ));
    }
    Ok(())
}

/// Refuses levels not strictly ascending by what they include: they read back in that order, so
/// another order (or two levels including the same) would not read as written.
fn check_levels(levels: &[Level], path: &str) -> Result<(), Problem> {
    if levels.is_empty() {
        return Err(Problem::field(
            format!("{path}/model/levels"),
            "required",
            "one level at least",
        ));
    }
    for (index, pair) in levels.windows(2).enumerate() {
        if pair[1].included <= pair[0].included {
            return Err(Problem::field(
                format!("{path}/model/levels/{}/included", index + 1),
                "invalid",
                "levels include strictly ascending usage, fewest units first",
            ));
        }
    }
    Ok(())
}

/// Refuses two prices of a metered charge in one cell (option, currency, cadence): a cell's
/// prices read back as one price's levels.
fn check_one_price_per_cell(charge: &Charge, path: &str) -> Result<(), Problem> {
    let mut seen: Vec<(String, String, Cadence)> = Vec::with_capacity(charge.prices.len());
    for (index, price) in charge.prices.iter().enumerate() {
        let cell = (
            price.option.trim().to_owned(),
            price.currency.trim().to_ascii_uppercase(),
            price.cadence,
        );
        if seen.contains(&cell) {
            return Err(Problem::field(
                format!("{path}/prices/{index}"),
                "duplicate",
                "one price per option, currency and cadence: list a cell's levels in one price",
            ));
        }
        seen.push(cell);
    }
    Ok(())
}

fn tier_rows(tiers: &[Tier]) -> Vec<TierRow> {
    tiers
        .iter()
        .map(|tier| TierRow {
            first_unit: tier.first_unit,
            rate: tier.unit_price,
            flat_fee: tier.flat_fee,
            flat_cap: tier.flat_cap,
        })
        .collect()
}

fn api_tiers(rows: &[TierRow]) -> Vec<Tier> {
    rows.iter()
        .map(|row| Tier {
            first_unit: row.first_unit,
            unit_price: row.rate.normalize(),
            flat_fee: row.flat_fee.map(|fee| fee.normalize()),
            flat_cap: row.flat_cap.map(|cap| cap.normalize()),
        })
        .collect()
}

/// A count's model: per unit, graduated, volume or package.
fn rating_model(model: &PriceModel, path: &str, what: &str) -> Result<RatingModel, Problem> {
    match model {
        PriceModel::PerUnit(m) => Ok(RatingModel::PerUnit { rate: m.unit_price }),
        PriceModel::Graduated(m) => {
            check_tiers(&m.tiers, path)?;
            Ok(RatingModel::Tiered {
                tiers: tier_rows(&m.tiers),
            })
        }
        PriceModel::Volume(m) => {
            check_tiers(&m.tiers, path)?;
            Ok(RatingModel::Volume {
                tiers: tier_rows(&m.tiers),
            })
        }
        PriceModel::Package(m) => {
            check_package(m.package_size, path)?;
            Ok(RatingModel::Package {
                block_size: m.package_size,
                rate: m.package_price,
            })
        }
        other => Err(Problem::not_expressible(
            format!("{path}/model"),
            format!(
                "{what} is priced per unit, graduated, volume or package, not {}",
                other.name()
            ),
        )),
    }
}

fn api_rating_model(model: &RatingModel) -> PriceModel {
    match model {
        RatingModel::PerUnit { rate } => PriceModel::PerUnit(PerUnitModel {
            unit_price: rate.normalize(),
        }),
        RatingModel::Tiered { tiers } => PriceModel::Graduated(GraduatedModel {
            tiers: api_tiers(tiers),
        }),
        RatingModel::Volume { tiers } => PriceModel::Volume(VolumeModel {
            tiers: api_tiers(tiers),
        }),
        RatingModel::Package { block_size, rate } => PriceModel::Package(PackageModel {
            package_size: *block_size,
            package_price: rate.normalize(),
        }),
    }
}

fn usage_model(
    model: &PriceModel,
    refs: &ChargeRefs,
    path: &str,
) -> Result<UsagePricingModel, Problem> {
    match model {
        PriceModel::PerUnit(m) => Ok(UsagePricingModel::PerUnit { rate: m.unit_price }),
        PriceModel::Graduated(m) => {
            check_tiers(&m.tiers, path)?;
            Ok(UsagePricingModel::Tiered {
                tiers: tier_rows(&m.tiers),
                block_size: None,
            })
        }
        PriceModel::Volume(m) => {
            check_tiers(&m.tiers, path)?;
            Ok(UsagePricingModel::Volume {
                tiers: tier_rows(&m.tiers),
                block_size: None,
            })
        }
        PriceModel::Package(m) => {
            check_package(m.package_size, path)?;
            Ok(UsagePricingModel::Package {
                block_size: m.package_size,
                rate: m.package_price,
            })
        }
        PriceModel::DimensionRates(m) => Ok(UsagePricingModel::DimensionRates(DomainRates {
            dimensions: m.dimensions.clone(),
            rates: m
                .rates
                .iter()
                .map(|rate| DomainRate {
                    key: rate.key.clone(),
                    rate: rate.unit_price,
                    label: rate.label.clone(),
                })
                .collect(),
            unit_size: m.unit_size,
            on_unmatched: match m.on_unmatched {
                Unmatched::Hold => RateCardUnmatched::Hold,
                Unmatched::Ignore => RateCardUnmatched::Ignore,
            },
        })),
        PriceModel::RateCard(m) => Ok(UsagePricingModel::RateCard {
            rate_card_id: *refs.rate_cards.get(&m.rate_card).ok_or_else(|| {
                Problem::field(
                    format!("{path}/model/rate_card"),
                    "not_found",
                    format!("rate card {} not found", m.rate_card),
                )
            })?,
            multiplier: m.multiplier,
        }),
        PriceModel::Flat(_) | PriceModel::Levels(_) => Err(Problem::not_expressible(
            format!("{path}/model"),
            format!(
                "metered usage is not priced {}: use levels for usage sold in levels, \
                 per_unit with `included` for an allowance",
                model.name()
            ),
        )),
    }
}

fn domain_expiry(expiry: &Expiry, path: &str) -> Result<DomainExpiry, Problem> {
    match (expiry.unit, expiry.value) {
        (ExpiryUnit::Days, Some(days)) => Ok(DomainExpiry::Day(days)),
        (ExpiryUnit::Months, Some(months)) => Ok(DomainExpiry::Month(months)),
        (ExpiryUnit::BillingPeriod, None) => Ok(DomainExpiry::BillingPeriod),
        (ExpiryUnit::BillingPeriod, Some(_)) => Err(Problem::field(
            path,
            "invalid",
            "an expiry at the end of the billing period has no value",
        )),
        (_, None) => Err(Problem::field(
            path,
            "required",
            "an expiry in days or months has a value",
        )),
    }
}

fn api_expiry(expiry: DomainExpiry) -> Expiry {
    match expiry {
        DomainExpiry::Day(days) => Expiry {
            unit: ExpiryUnit::Days,
            value: Some(days),
        },
        DomainExpiry::Month(months) => Expiry {
            unit: ExpiryUnit::Months,
            value: Some(months),
        },
        DomainExpiry::BillingPeriod => Expiry {
            unit: ExpiryUnit::BillingPeriod,
            value: None,
        },
    }
}

/// Seats counting `quantity_id`. When they change, how they are billed is the managed quantity's
/// (`on_increase`, `on_decrease`), never the structure's: its `upgrade_policy` and
/// `downgrade_policy` are not read by the engine and have one value each, written as the stored
/// shape requires.
fn seats_structure(unit_name: String, quantity_id: ManagedQuantityId) -> FeeStructure {
    FeeStructure::Slot {
        unit_name,
        quantity_id: Some(quantity_id),
        upgrade_policy: UpgradePolicy::Prorated,
        downgrade_policy: DowngradePolicy::RemoveAtEndOfPeriod,
    }
}

/// The charge as the domain holds it; `path` is the charge's JSON pointer.
///
/// Reading the result back with [`render`] returns the charge sent, but for these
/// normalizations: decimals read normalized (`"490.00"` → `"490"`), currencies upper case, the
/// name and the quantity and timing a preset fills are written out, and the preset read is the
/// hint of the composition. Where two compositions bill the same and the domain cannot tell them
/// apart, one reading is canonical:
/// - usage sold in levels whose every cell is one level at no fee, all including the same usage,
///   reads as that allowance (`included` with a `per_unit` price, preset `usage`); it bills the
///   same.
///
/// What would read otherwise is refused: levels not strictly ascending, tiers not strictly
/// ascending or not starting at unit 0, two prices of a metered charge in one cell. Every price
/// is checked first ([`check_price`]): its currency, no negative amount, its lists' bounds.
pub fn to_domain(charge: &Charge, refs: &ChargeRefs, path: &str) -> Result<DomainCharge, Problem> {
    let name = charge
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(&refs.product_name)
        .to_owned();
    if let Some(name) = charge.name.as_deref() {
        bounds::name(name.trim(), &format!("{path}/name"))?;
    }
    let price_path = |index: usize| format!("{path}/prices/{index}");
    bounds::count(
        charge.prices.len(),
        MAX_PRICES,
        &format!("{path}/prices"),
        "prices",
    )?;
    let (quantity, timing) = shape(charge, path)?;
    let metered = matches!(quantity, Some(Quantity::Metered(_)));
    for (index, price) in charge.prices.iter().enumerate() {
        check_price(price, metered, &price_path(index))?;
    }
    let fixed_count = |value: &Decimal| -> Result<u32, Problem> {
        let count = whole(
            *value,
            &format!("{path}/quantity/value"),
            "a fixed quantity",
        )?;
        if count == 0 {
            return Err(Problem::field(
                format!("{path}/quantity/value"),
                "invalid",
                "a fixed quantity is 1 or more",
            ));
        }
        u32::try_from(count).map_err(|_| {
            Problem::field(
                format!("{path}/quantity/value"),
                "invalid",
                "a fixed quantity is too large",
            )
        })
    };
    let input = |price: &ChargePrice, pricing: Pricing| -> Result<(PriceInput, String), Problem> {
        let currency = price.currency.trim().to_ascii_uppercase();
        Ok((
            PriceInput {
                cadence: price.cadence.into(),
                currency,
                pricing,
                credit_unit: price.credit_unit.as_ref().map(|unit| DomainCreditUnit {
                    unit_id: unit.unit,
                    rate: unit.rate,
                }),
            },
            price.option.trim().to_owned(),
        ))
    };
    if charge.included.is_some() && !matches!(quantity, Some(Quantity::Metered(_))) {
        return Err(Problem::not_expressible(
            format!("{path}/included"),
            "only metered usage has an included allowance",
        ));
    }
    let granting = charge.prices.iter().any(|price| price.grants.is_some());
    if granting && !matches!(quantity, Some(Quantity::Fixed(_))) {
        return Err(Problem::not_expressible(
            format!("{path}/prices"),
            "a credit pack sells a fixed quantity of packs",
        ));
    }

    let (structure, prices) = match &quantity {
        None => {
            if !charge.prices.is_empty() {
                return Err(Problem::field(
                    format!("{path}/quantity"),
                    "required",
                    "a priced charge says what it bills: a quantity or a preset",
                ));
            }
            (None, Vec::new())
        }
        Some(Quantity::Fixed(fixed)) => {
            let count = fixed_count(&fixed.value)?;
            if granting {
                if timing.billing != Billing::Advance {
                    return Err(Problem::not_expressible(
                        format!("{path}/timing"),
                        "a credit pack is billed in advance",
                    ));
                }
                let once = timing.recurrence == Recurrence::OneTime;
                let prices = charge
                    .prices
                    .iter()
                    .enumerate()
                    .map(|(index, price)| {
                        let (PriceModel::Flat(flat), Some(grant)) = (&price.model, &price.grants)
                        else {
                            return Err(Problem::not_expressible(
                                format!("{}/model", price_path(index)),
                                "a credit pack has a flat price and a grant",
                            ));
                        };
                        let grant_path = format!("{}/grants", price_path(index));
                        input(
                            price,
                            Pricing::Credit {
                                unit_price: flat.amount,
                                quantity: count,
                                credit_amount: grant.amount,
                                expiry: grant
                                    .expires_after
                                    .as_ref()
                                    .map(|e| {
                                        domain_expiry(e, &format!("{grant_path}/expires_after"))
                                    })
                                    .transpose()?,
                                credit_unit_id: grant.credit_unit,
                                rollover: grant
                                    .rollover
                                    .as_ref()
                                    .map(|r| {
                                        Ok::<_, Problem>(DomainRollover {
                                            max_amount: r.max_amount,
                                            expiry: domain_expiry(
                                                &r.expires_after,
                                                &format!("{grant_path}/rollover/expires_after"),
                                            )?,
                                        })
                                    })
                                    .transpose()?,
                                once,
                            },
                        )
                    })
                    .collect::<Result<Vec<_>, Problem>>()?;
                (Some(FeeStructure::Credit {}), prices)
            } else {
                let flat_rate = count == 1
                    && timing == Timing::RECURRING_ADVANCE
                    && charge
                        .prices
                        .iter()
                        .all(|price| matches!(price.model, PriceModel::Flat(_)));
                if timing.recurrence == Recurrence::OneTime && timing.billing == Billing::Arrears {
                    return Err(Problem::not_expressible(
                        format!("{path}/timing"),
                        "a one-time charge is billed in advance",
                    ));
                }
                let mut prices = Vec::with_capacity(charge.prices.len());
                for (index, price) in charge.prices.iter().enumerate() {
                    let at = price_path(index);
                    let (unit_price, model) = match &price.model {
                        PriceModel::Flat(flat) if count == 1 => (flat.amount, None),
                        PriceModel::Flat(_) => {
                            return Err(Problem::not_expressible(
                                format!("{at}/model"),
                                "a flat price bills one unit: use per_unit for a fixed quantity above 1",
                            ));
                        }
                        // On one unit a bare unit price reads as flat: a per-unit price says so
                        // with its model, billing the same, so it reads back per unit.
                        PriceModel::PerUnit(m) if count == 1 => (
                            m.unit_price,
                            Some(RatingModel::PerUnit { rate: m.unit_price }),
                        ),
                        PriceModel::PerUnit(m) => (m.unit_price, None),
                        other => (
                            Decimal::ZERO,
                            Some(rating_model(other, &at, "a fixed quantity")?),
                        ),
                    };
                    let pricing = if flat_rate {
                        Pricing::Rate { rate: unit_price }
                    } else if timing.recurrence == Recurrence::OneTime {
                        Pricing::OneTime {
                            unit_price,
                            quantity: count,
                            model,
                        }
                    } else {
                        Pricing::ExtraRecurring {
                            unit_price,
                            quantity: count,
                            model,
                        }
                    };
                    prices.push(input(price, pricing)?);
                }
                let structure = if flat_rate {
                    FeeStructure::Rate {}
                } else if timing.recurrence == Recurrence::OneTime {
                    FeeStructure::OneTime {}
                } else {
                    FeeStructure::ExtraRecurring {
                        billing_type: match timing.billing {
                            Billing::Advance => BillingType::Advance,
                            Billing::Arrears => BillingType::Arrears,
                        },
                    }
                };
                (Some(structure), prices)
            }
        }
        Some(Quantity::Managed(managed)) => {
            if managed.initial.is_some() {
                return Err(Problem::not_expressible(
                    format!("{path}/quantity/initial"),
                    "a plan does not set the starting count: each subscription does",
                ));
            }
            if timing != Timing::RECURRING_ADVANCE {
                return Err(Problem::not_expressible(
                    format!("{path}/timing"),
                    "seats recur and are billed in advance",
                ));
            }
            let Some((quantity_id, quantity_name)) = refs.quantity.clone() else {
                return Err(Problem::field(
                    format!("{path}/quantity/quantity"),
                    "not_found",
                    format!("managed quantity {} not found", managed.quantity),
                ));
            };
            let prices = charge
                .prices
                .iter()
                .enumerate()
                .map(|(index, price)| {
                    let model = rating_model(&price.model, &price_path(index), "a seat")?;
                    input(
                        price,
                        Pricing::Slot {
                            model,
                            min_slots: managed.min,
                            max_slots: managed.max,
                        },
                    )
                })
                .collect::<Result<Vec<_>, Problem>>()?;
            (Some(seats_structure(quantity_name, quantity_id)), prices)
        }
        Some(Quantity::Metered(metered)) => {
            let Some(metric_id) = refs.metric_id else {
                return Err(Problem::field(
                    format!("{path}/quantity/metric"),
                    "not_found",
                    format!("metric {} not found", metered.metric),
                ));
            };
            if timing.recurrence != Recurrence::Recurring {
                return Err(Problem::not_expressible(
                    format!("{path}/timing"),
                    "usage recurs each period",
                ));
            }
            let levels = charge
                .prices
                .iter()
                .any(|price| matches!(price.model, PriceModel::Levels(_)));
            if levels {
                if charge.included.is_some() {
                    return Err(Problem::not_expressible(
                        format!("{path}/included"),
                        "each level says what it includes",
                    ));
                }
                if timing.billing != Billing::Advance {
                    return Err(Problem::not_expressible(
                        format!("{path}/timing"),
                        "usage levels are billed in advance, their overage in arrears",
                    ));
                }
                check_one_price_per_cell(charge, path)?;
                let mut prices = Vec::new();
                for (index, price) in charge.prices.iter().enumerate() {
                    let PriceModel::Levels(model) = &price.model else {
                        return Err(Problem::not_expressible(
                            format!("{}/model", price_path(index)),
                            "a charge sold in levels is priced in levels in every cell",
                        ));
                    };
                    check_levels(&model.levels, &price_path(index))?;
                    for level in &model.levels {
                        prices.push(input(
                            price,
                            Pricing::Capacity {
                                rate: level.price,
                                included: level.included,
                                overage_rate: level.overage_unit_price,
                            },
                        )?);
                    }
                }
                (Some(FeeStructure::Capacity { metric_id }), prices)
            } else if let Some(included) = charge.included {
                let included = whole(included, &format!("{path}/included"), "an allowance")?;
                if timing.billing != Billing::Arrears {
                    return Err(Problem::not_expressible(
                        format!("{path}/timing"),
                        "usage beyond an allowance is billed in arrears",
                    ));
                }
                check_one_price_per_cell(charge, path)?;
                let prices = charge
                    .prices
                    .iter()
                    .enumerate()
                    .map(|(index, price)| {
                        let PriceModel::PerUnit(model) = &price.model else {
                            return Err(Problem::not_expressible(
                                format!("{}/model", price_path(index)),
                                "usage beyond an allowance is priced per unit",
                            ));
                        };
                        input(
                            price,
                            Pricing::Capacity {
                                rate: Decimal::ZERO,
                                included,
                                overage_rate: model.unit_price,
                            },
                        )
                    })
                    .collect::<Result<Vec<_>, Problem>>()?;
                (Some(FeeStructure::Capacity { metric_id }), prices)
            } else {
                if timing.billing != Billing::Arrears {
                    return Err(Problem::not_expressible(
                        format!("{path}/timing"),
                        "usage is billed in arrears; levels are billed in advance",
                    ));
                }
                let models = charge
                    .prices
                    .iter()
                    .enumerate()
                    .map(|(index, price)| usage_model(&price.model, refs, &price_path(index)))
                    .collect::<Result<Vec<_>, Problem>>()?;
                let hint = models
                    .first()
                    .map(UsageModel::from)
                    .unwrap_or(UsageModel::PerUnit);
                let prices = charge
                    .prices
                    .iter()
                    .zip(models)
                    .map(|(price, model)| input(price, Pricing::Usage(model)))
                    .collect::<Result<Vec<_>, Problem>>()?;
                (
                    Some(FeeStructure::Usage {
                        metric_id,
                        model: hint,
                    }),
                    prices,
                )
            }
        }
    };
    if granting && !matches!(structure, Some(FeeStructure::Credit {})) {
        return Err(Problem::not_expressible(
            format!("{path}/prices"),
            "only a credit pack grants credit",
        ));
    }
    Ok(DomainCharge {
        name,
        product_id: refs.product_id,
        structure,
        prices,
    })
}

// ── From the domain ─────────────────────────────────────────────────

/// The options of the version a stored price is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptionSet {
    Every,
    /// By position in the version's options.
    Positions(Vec<usize>),
}

#[derive(Debug, Clone)]
pub struct StoredPrice {
    pub cadence: BillingPeriodEnum,
    pub currency: String,
    pub pricing: Pricing,
    pub credit_unit: Option<DomainCreditUnit>,
    pub options: OptionSet,
}

#[derive(Debug, Clone)]
pub struct StoredCharge {
    pub name: String,
    pub product_id: ProductId,
    pub structure: Option<FeeStructure>,
    pub prices: Vec<StoredPrice>,
}

/// The codes a rendered charge names its references by; an id when no code is known.
pub trait Names {
    fn product(&self, id: ProductId) -> String;
    fn metric(&self, id: BillableMetricId) -> String;
    fn quantity(&self, id: ManagedQuantityId) -> String;
    fn rate_card(&self, id: RateCardId) -> String;
}

fn unreadable(path: &str, message: impl Into<String>) -> Problem {
    Problem::not_expressible(
        path,
        format!("{}; read this charge with API v1", message.into()),
    )
}

fn api_usage_model(
    model: &UsagePricingModel,
    names: &impl Names,
    path: &str,
) -> Result<PriceModel, Problem> {
    Ok(match model {
        UsagePricingModel::PerUnit { rate } => PriceModel::PerUnit(PerUnitModel {
            unit_price: rate.normalize(),
        }),
        UsagePricingModel::Tiered { tiers, block_size } if block_size.unwrap_or(1) <= 1 => {
            PriceModel::Graduated(GraduatedModel {
                tiers: api_tiers(tiers),
            })
        }
        UsagePricingModel::Volume { tiers, block_size } if block_size.unwrap_or(1) <= 1 => {
            PriceModel::Volume(VolumeModel {
                tiers: api_tiers(tiers),
            })
        }
        UsagePricingModel::Tiered { .. } | UsagePricingModel::Volume { .. } => {
            return Err(unreadable(path, "tiers priced by blocks"));
        }
        UsagePricingModel::Package { block_size, rate } => PriceModel::Package(PackageModel {
            package_size: *block_size,
            package_price: rate.normalize(),
        }),
        UsagePricingModel::DimensionRates(rates) => {
            PriceModel::DimensionRates(DimensionRatesModel {
                dimensions: rates.dimensions.clone(),
                unit_size: rates.unit_size.normalize(),
                on_unmatched: match rates.on_unmatched {
                    RateCardUnmatched::Hold => Unmatched::Hold,
                    RateCardUnmatched::Ignore => Unmatched::Ignore,
                },
                rates: rates
                    .rates
                    .iter()
                    .map(|rate| DimensionRate {
                        key: rate.key.clone(),
                        unit_price: rate.rate.normalize(),
                        label: rate.label.clone(),
                    })
                    .collect(),
            })
        }
        UsagePricingModel::RateCard {
            rate_card_id,
            multiplier,
        } => PriceModel::RateCard(RateCardModel {
            rate_card: names.rate_card(*rate_card_id),
            multiplier: multiplier.map(|m| m.normalize()),
        }),
    })
}

/// The one value every price agrees on, else unreadable.
fn agreed<T: PartialEq + Clone>(
    values: impl IntoIterator<Item = T>,
    path: &str,
    what: &str,
) -> Result<Option<T>, Problem> {
    let mut agreed: Option<T> = None;
    for value in values {
        match &agreed {
            None => agreed = Some(value),
            Some(first) if *first == value => {}
            Some(_) => return Err(unreadable(path, format!("its prices disagree on {what}"))),
        }
    }
    Ok(agreed)
}

fn api_credit_unit(unit: &Option<DomainCreditUnit>) -> Option<PriceCreditUnit> {
    unit.as_ref().map(|unit| PriceCreditUnit {
        unit: unit.unit_id,
        rate: unit.rate.normalize(),
    })
}

/// A price as it reads in a group of options, before it is spread over them.
struct ApiPrice {
    options: OptionSet,
    currency: String,
    cadence: Cadence,
    model: PriceModel,
    credit_unit: Option<PriceCreditUnit>,
    grants: Option<CreditGrant>,
}

/// The charge as its canonical composition. `aliases` are the version's options by position
/// (empty for an add-on); `path` locates a charge that cannot be read.
pub fn render(
    charge: &StoredCharge,
    aliases: &[String],
    names: &impl Names,
    path: &str,
) -> Result<Charge, Problem> {
    let product = names.product(charge.product_id);
    let base = |prices: Vec<ApiPrice>| -> Vec<ChargePrice> { spread(prices, aliases) };
    let api = |price: &StoredPrice, model: PriceModel, grants: Option<CreditGrant>| ApiPrice {
        options: price.options.clone(),
        currency: price.currency.clone(),
        cadence: price.cadence.into(),
        model,
        credit_unit: api_credit_unit(&price.credit_unit),
        grants,
    };
    let mismatch = || unreadable(path, "a price does not fit how the charge bills");
    let fixed = |value: u32| {
        Some(Quantity::Fixed(FixedQuantity {
            value: Decimal::from(value),
        }))
    };
    let mut included = None;
    let (preset, quantity, timing, prices) = match &charge.structure {
        None => (None, None, None, Vec::new()),
        Some(FeeStructure::Rate {}) => {
            let prices = charge
                .prices
                .iter()
                .map(|price| match &price.pricing {
                    Pricing::Rate { rate } => Ok(api(
                        price,
                        PriceModel::Flat(FlatModel {
                            amount: rate.normalize(),
                        }),
                        None,
                    )),
                    _ => Err(mismatch()),
                })
                .collect::<Result<Vec<_>, Problem>>()?;
            (
                Some(Preset::FlatFee),
                fixed(1),
                Some(Timing::RECURRING_ADVANCE),
                prices,
            )
        }
        Some(structure @ (FeeStructure::ExtraRecurring { .. } | FeeStructure::OneTime {})) => {
            let parts = charge
                .prices
                .iter()
                .map(|price| match &price.pricing {
                    Pricing::ExtraRecurring {
                        unit_price,
                        quantity,
                        model,
                    }
                    | Pricing::OneTime {
                        unit_price,
                        quantity,
                        model,
                    } => Ok((price, *unit_price, *quantity, model.clone())),
                    _ => Err(mismatch()),
                })
                .collect::<Result<Vec<_>, Problem>>()?;
            let count = agreed(parts.iter().map(|p| p.2), path, "the quantity")?.unwrap_or(1);
            let prices = parts
                .into_iter()
                .map(|(price, unit_price, quantity, model)| {
                    let model = match model {
                        Some(model) => api_rating_model(&model),
                        None if quantity == 1 => PriceModel::Flat(FlatModel {
                            amount: unit_price.normalize(),
                        }),
                        None => PriceModel::PerUnit(PerUnitModel {
                            unit_price: unit_price.normalize(),
                        }),
                    };
                    api(price, model, None)
                })
                .collect();
            // The preset of the composition, as `shape` takes it: a fixed quantity billed in
            // advance each period is a flat fee whatever its count and model.
            let (preset, timing) = match structure {
                FeeStructure::OneTime {} => (Some(Preset::OneTime), Timing::ONE_TIME),
                FeeStructure::ExtraRecurring {
                    billing_type: BillingType::Arrears,
                } => (None, Timing::RECURRING_ARREARS),
                _ => (Some(Preset::FlatFee), Timing::RECURRING_ADVANCE),
            };
            (preset, fixed(count), Some(timing), prices)
        }
        Some(FeeStructure::Credit {}) => {
            let parts = charge
                .prices
                .iter()
                .map(|price| match &price.pricing {
                    Pricing::Credit {
                        unit_price,
                        quantity,
                        credit_amount,
                        expiry,
                        credit_unit_id,
                        rollover,
                        once,
                    } => Ok((
                        api(
                            price,
                            PriceModel::Flat(FlatModel {
                                amount: unit_price.normalize(),
                            }),
                            Some(CreditGrant {
                                amount: credit_amount.normalize(),
                                credit_unit: *credit_unit_id,
                                expires_after: expiry.map(api_expiry),
                                rollover: rollover.map(|r| Rollover {
                                    max_amount: r.max_amount.map(|m| m.normalize()),
                                    expires_after: api_expiry(r.expiry),
                                }),
                            }),
                        ),
                        *quantity,
                        *once,
                    )),
                    _ => Err(mismatch()),
                })
                .collect::<Result<Vec<_>, Problem>>()?;
            let count = agreed(parts.iter().map(|p| p.1), path, "the quantity")?.unwrap_or(1);
            let once =
                agreed(parts.iter().map(|p| p.2), path, "being billed once")?.unwrap_or(false);
            (
                Some(Preset::CreditPack),
                fixed(count),
                Some(if once {
                    Timing::ONE_TIME
                } else {
                    Timing::RECURRING_ADVANCE
                }),
                parts.into_iter().map(|p| p.0).collect(),
            )
        }
        Some(FeeStructure::Slot { quantity_id, .. }) => {
            let parts = charge
                .prices
                .iter()
                .map(|price| match &price.pricing {
                    Pricing::Slot {
                        model,
                        min_slots,
                        max_slots,
                    } => Ok((
                        api(price, api_rating_model(model), None),
                        (*min_slots, *max_slots),
                    )),
                    _ => Err(mismatch()),
                })
                .collect::<Result<Vec<_>, Problem>>()?;
            let (min, max) =
                agreed(parts.iter().map(|p| p.1), path, "the seat limits")?.unwrap_or((None, None));
            let Some(quantity_id) = quantity_id else {
                return Err(unreadable(path, "its seats name no managed quantity"));
            };
            (
                Some(Preset::Seats),
                Some(Quantity::Managed(ManagedQuantityRef {
                    quantity: names.quantity(*quantity_id),
                    initial: None,
                    min,
                    max,
                })),
                Some(Timing::RECURRING_ADVANCE),
                parts.into_iter().map(|p| p.0).collect(),
            )
        }
        Some(FeeStructure::Usage { metric_id, .. }) => {
            let prices = charge
                .prices
                .iter()
                .enumerate()
                .map(|(index, price)| match &price.pricing {
                    Pricing::Usage(model) => Ok(api(
                        price,
                        api_usage_model(model, names, &format!("{path}/prices/{index}"))?,
                        None,
                    )),
                    _ => Err(mismatch()),
                })
                .collect::<Result<Vec<_>, Problem>>()?;
            (
                Some(Preset::Usage),
                Some(Quantity::Metered(MeteredQuantity {
                    metric: names.metric(*metric_id),
                })),
                Some(Timing::RECURRING_ARREARS),
                prices,
            )
        }
        Some(FeeStructure::Capacity { metric_id }) => {
            let levels = charge
                .prices
                .iter()
                .map(|price| match &price.pricing {
                    Pricing::Capacity {
                        rate,
                        included,
                        overage_rate,
                    } => Ok((price, *rate, *included, *overage_rate)),
                    _ => Err(mismatch()),
                })
                .collect::<Result<Vec<_>, Problem>>()?;
            // The prices of one (options, currency, cadence) are the levels of one cell.
            let mut groups: Vec<(
                OptionSet,
                String,
                BillingPeriodEnum,
                Vec<Level>,
                &StoredPrice,
            )> = Vec::new();
            for (price, rate, included, overage) in &levels {
                let level = Level {
                    included: *included,
                    price: rate.normalize(),
                    overage_unit_price: overage.normalize(),
                };
                match groups
                    .iter_mut()
                    .find(|(options, currency, cadence, _, _)| {
                        *options == price.options
                            && *currency == price.currency
                            && *cadence == price.cadence
                    }) {
                    Some(group) => group.3.push(level),
                    None => groups.push((
                        price.options.clone(),
                        price.currency.clone(),
                        price.cadence,
                        vec![level],
                        *price,
                    )),
                }
            }
            let allowance = agreed(levels.iter().map(|l| l.2), path, "the allowance")
                .ok()
                .flatten();
            let as_allowance = allowance.is_some()
                && groups.iter().all(|group| group.3.len() == 1)
                && levels.iter().all(|l| l.1.is_zero());
            let metered = Some(Quantity::Metered(MeteredQuantity {
                metric: names.metric(*metric_id),
            }));
            if as_allowance {
                included = allowance.map(Decimal::from);
                let prices = groups
                    .into_iter()
                    .map(|(_, _, _, levels, price)| {
                        api(
                            price,
                            PriceModel::PerUnit(PerUnitModel {
                                unit_price: levels[0].overage_unit_price,
                            }),
                            None,
                        )
                    })
                    .collect();
                (
                    Some(Preset::Usage),
                    metered,
                    Some(Timing::RECURRING_ARREARS),
                    prices,
                )
            } else {
                let prices = groups
                    .into_iter()
                    .map(|(_, _, _, mut levels, price)| {
                        levels.sort_by_key(|level| level.included);
                        api(price, PriceModel::Levels(LevelsModel { levels }), None)
                    })
                    .collect();
                (
                    Some(Preset::UsageLevels),
                    metered,
                    Some(Timing::RECURRING_ADVANCE),
                    prices,
                )
            }
        }
    };
    Ok(Charge {
        product,
        name: Some(charge.name.clone()),
        preset,
        quantity,
        included,
        timing,
        prices: base(prices),
    })
}

/// Each price once per option it is in (`*` for every option), ordered by option position,
/// currency and cadence.
fn spread(prices: Vec<ApiPrice>, aliases: &[String]) -> Vec<ChargePrice> {
    let mut spread: Vec<(usize, ChargePrice)> = Vec::new();
    for price in prices {
        let positions: Vec<Option<usize>> = match &price.options {
            OptionSet::Every => vec![None],
            OptionSet::Positions(positions) => positions.iter().copied().map(Some).collect(),
        };
        for position in positions {
            let (rank, option) = match position {
                None => (0, EVERY_OPTION.to_owned()),
                Some(position) => (
                    position + 1,
                    aliases
                        .get(position)
                        .cloned()
                        .unwrap_or_else(|| EVERY_OPTION.to_owned()),
                ),
            };
            spread.push((
                rank,
                ChargePrice {
                    option,
                    currency: price.currency.clone(),
                    cadence: price.cadence,
                    model: price.model.clone(),
                    credit_unit: price.credit_unit.clone(),
                    grants: price.grants.clone(),
                },
            ));
        }
    }
    spread.sort_by(|(a_rank, a), (b_rank, b)| {
        (a_rank, &a.currency, a.cadence).cmp(&(b_rank, &b.currency, b.cadence))
    });
    spread.into_iter().map(|(_, price)| price).collect()
}

/// The option set of a price linked in `linked` of the version's `options` (by position).
pub fn option_set(linked: &[usize], options: usize) -> OptionSet {
    let mut positions: Vec<usize> = linked.to_vec();
    positions.sort_unstable();
    positions.dedup();
    if options > 0 && positions.len() == options {
        OptionSet::Every
    } else {
        OptionSet::Positions(positions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common_domain::ids::BaseId;

    struct Ids;
    impl Names for Ids {
        fn product(&self, _: ProductId) -> String {
            "api".to_owned()
        }
        fn metric(&self, _: BillableMetricId) -> String {
            "calls".to_owned()
        }
        fn quantity(&self, _: ManagedQuantityId) -> String {
            "seats".to_owned()
        }
        fn rate_card(&self, _: RateCardId) -> String {
            "llm".to_owned()
        }
    }

    fn refs() -> ChargeRefs {
        ChargeRefs {
            product_id: ProductId::new(),
            product_name: "API".to_owned(),
            metric_id: Some(BillableMetricId::new()),
            quantity: Some((ManagedQuantityId::new(), "Seats".to_owned())),
            rate_cards: HashMap::new(),
        }
    }

    fn charge(json: serde_json::Value) -> Charge {
        serde_json::from_value(json).unwrap()
    }

    fn stored(domain: &DomainCharge) -> StoredCharge {
        StoredCharge {
            name: domain.name.clone(),
            product_id: domain.product_id,
            structure: domain.structure.clone(),
            prices: domain
                .prices
                .iter()
                .map(|(input, option)| StoredPrice {
                    cadence: input.cadence,
                    currency: input.currency.clone(),
                    pricing: input.pricing.clone(),
                    credit_unit: input.credit_unit.clone(),
                    options: if option == EVERY_OPTION {
                        OptionSet::Every
                    } else {
                        OptionSet::Positions(vec![if option == "monthly" { 0 } else { 1 }])
                    },
                })
                .collect(),
        }
    }

    fn round_trip(json: serde_json::Value) -> Charge {
        let refs = refs();
        let domain = to_domain(&charge(json), &refs, "/charges/0").unwrap();
        render(
            &stored(&domain),
            &["monthly".to_owned(), "annual".to_owned()],
            &Ids,
            "/charges/0",
        )
        .unwrap()
    }

    #[test]
    fn a_flat_fee_preset_reads_back_as_its_composition() {
        let read = round_trip(serde_json::json!({
            "product": "platform",
            "preset": "flat_fee",
            "prices": [
                {"option": "annual", "currency": "eur", "cadence": "annual", "model": {"type": "flat", "amount": "490.00"}},
                {"option": "monthly", "currency": "EUR", "model": {"type": "flat", "amount": "49"}}
            ]
        }));
        assert_eq!(read.preset, Some(Preset::FlatFee));
        assert_eq!(
            read.quantity,
            Some(Quantity::Fixed(FixedQuantity {
                value: Decimal::ONE
            }))
        );
        assert_eq!(read.timing, Some(Timing::RECURRING_ADVANCE));
        assert_eq!(read.prices[0].option, "monthly");
        assert_eq!(read.prices[1].option, "annual");
        assert_eq!(
            read.prices[1].model,
            PriceModel::Flat(FlatModel {
                amount: Decimal::from(490)
            })
        );
        assert_eq!(read.prices[1].currency, "EUR");
    }

    #[test]
    fn seats_count_their_managed_quantity() {
        let read = round_trip(serde_json::json!({
            "product": "seats",
            "preset": "seats",
            "quantity": {"type": "managed", "quantity": "seats", "min": 1},
            "prices": [{"currency": "EUR", "model": {"type": "per_unit", "unit_price": "10"}}]
        }));
        assert_eq!(read.preset, Some(Preset::Seats));
        let Some(Quantity::Managed(managed)) = read.quantity else {
            panic!("seats are managed");
        };
        assert_eq!((managed.quantity.as_str(), managed.min), ("seats", Some(1)));
        assert_eq!(read.prices[0].option, "*");
    }

    #[test]
    fn usage_with_an_allowance_and_usage_levels_read_apart() {
        let allowance = round_trip(serde_json::json!({
            "product": "api",
            "quantity": {"type": "metered", "metric": "calls"},
            "included": "1000",
            "prices": [{"currency": "EUR", "model": {"type": "per_unit", "unit_price": "0.002"}}]
        }));
        assert_eq!(allowance.included, Some(Decimal::from(1000)));
        assert_eq!(allowance.timing, Some(Timing::RECURRING_ARREARS));
        assert_eq!(allowance.preset, Some(Preset::Usage));

        let levels = round_trip(serde_json::json!({
            "product": "api",
            "preset": "usage_levels",
            "quantity": {"type": "metered", "metric": "calls"},
            "prices": [{"currency": "EUR", "model": {"type": "levels", "levels": [
                {"included": 1000, "price": "19", "overage_unit_price": "0.02"},
                {"included": 10000, "price": "99", "overage_unit_price": "0.01"}
            ]}}]
        }));
        assert_eq!(levels.preset, Some(Preset::UsageLevels));
        assert_eq!(levels.included, None);
        let PriceModel::Levels(model) = &levels.prices[0].model else {
            panic!("levels read as levels");
        };
        assert_eq!(model.levels[0].included, 1000);
        assert_eq!(model.levels.len(), 2);
    }

    #[test]
    fn what_the_domain_cannot_express_is_refused() {
        let refs = refs();
        let flat_seats = charge(serde_json::json!({
            "product": "seats",
            "quantity": {"type": "managed", "quantity": "seats"},
            "prices": [{"currency": "EUR", "model": {"type": "flat", "amount": "10"}}]
        }));
        let problem = to_domain(&flat_seats, &refs, "/charges/1").unwrap_err();
        assert_eq!(
            problem.code,
            crate::v2::problem::ProblemCode::NotExpressible
        );
        assert_eq!(problem.errors[0].path, "/charges/1/prices/0/model");

        let arrears_seats = charge(serde_json::json!({
            "product": "seats",
            "quantity": {"type": "managed", "quantity": "seats"},
            "timing": {"recurrence": "recurring", "billing": "arrears"},
            "prices": [{"currency": "EUR", "model": {"type": "per_unit", "unit_price": "10"}}]
        }));
        assert!(to_domain(&arrears_seats, &refs, "/charges/1").is_err());

        let preset_conflict = charge(serde_json::json!({
            "product": "seats",
            "preset": "usage",
            "quantity": {"type": "fixed", "value": "1"},
            "prices": []
        }));
        let problem = to_domain(&preset_conflict, &refs, "/charges/2").unwrap_err();
        assert_eq!(problem.errors[0].code, "preset_conflict");
    }

    #[test]
    fn a_unit_price_on_one_unit_reads_back_per_unit() {
        let read = round_trip(serde_json::json!({
            "product": "support",
            "quantity": {"type": "fixed", "value": "1"},
            "prices": [
                {"currency": "EUR", "model": {"type": "per_unit", "unit_price": "30"}},
                {"currency": "USD", "model": {"type": "flat", "amount": "35"}}
            ]
        }));
        assert_eq!(read.preset, Some(Preset::FlatFee));
        assert_eq!(
            read.prices[0].model,
            PriceModel::PerUnit(PerUnitModel {
                unit_price: Decimal::from(30)
            })
        );
        assert_eq!(
            read.prices[1].model,
            PriceModel::Flat(FlatModel {
                amount: Decimal::from(35)
            })
        );

        let one_time = round_trip(serde_json::json!({
            "product": "setup",
            "preset": "one_time",
            "prices": [{"currency": "EUR", "model": {"type": "per_unit", "unit_price": "100"}}]
        }));
        assert!(matches!(one_time.prices[0].model, PriceModel::PerUnit(_)));
    }

    #[test]
    fn a_free_level_in_every_cell_reads_as_its_allowance() {
        let read = round_trip(serde_json::json!({
            "product": "api",
            "preset": "usage_levels",
            "quantity": {"type": "metered", "metric": "calls"},
            "prices": [{"currency": "EUR", "model": {"type": "levels", "levels": [
                {"included": 1000, "price": "0", "overage_unit_price": "0.02"}
            ]}}]
        }));
        assert_eq!(read.preset, Some(Preset::Usage));
        assert_eq!(read.included, Some(Decimal::from(1000)));
        assert_eq!(read.timing, Some(Timing::RECURRING_ARREARS));
        // The canonical reading reads back as itself.
        let again = round_trip(serde_json::to_value(&read).unwrap());
        assert_eq!(again, read);
    }

    #[test]
    fn what_would_not_read_as_written_is_refused_at_its_pointer() {
        let refs = refs();
        let unordered_tiers = charge(serde_json::json!({
            "product": "api",
            "quantity": {"type": "metered", "metric": "calls"},
            "prices": [{"currency": "EUR", "model": {"type": "graduated", "tiers": [
                {"first_unit": 0, "unit_price": "0.02"},
                {"first_unit": 1000, "unit_price": "0.01"},
                {"first_unit": 1000, "unit_price": "0.005"}
            ]}}]
        }));
        let problem = to_domain(&unordered_tiers, &refs, "/charges/0").unwrap_err();
        assert_eq!(
            problem.errors[0].path,
            "/charges/0/prices/0/model/tiers/2/first_unit"
        );

        let late_first_tier = charge(serde_json::json!({
            "product": "seats",
            "preset": "seats",
            "quantity": {"type": "managed", "quantity": "seats"},
            "prices": [{"currency": "EUR", "model": {"type": "volume", "tiers": [
                {"first_unit": 1, "unit_price": "10"}
            ]}}]
        }));
        let problem = to_domain(&late_first_tier, &refs, "/charges/1").unwrap_err();
        assert_eq!(
            problem.errors[0].path,
            "/charges/1/prices/0/model/tiers/0/first_unit"
        );

        let unordered_levels = charge(serde_json::json!({
            "product": "api",
            "preset": "usage_levels",
            "quantity": {"type": "metered", "metric": "calls"},
            "prices": [{"currency": "EUR", "model": {"type": "levels", "levels": [
                {"included": 10000, "price": "99", "overage_unit_price": "0.01"},
                {"included": 1000, "price": "19", "overage_unit_price": "0.02"}
            ]}}]
        }));
        let problem = to_domain(&unordered_levels, &refs, "/charges/2").unwrap_err();
        assert_eq!(
            problem.errors[0].path,
            "/charges/2/prices/0/model/levels/1/included"
        );

        let one_cell_twice = charge(serde_json::json!({
            "product": "api",
            "quantity": {"type": "metered", "metric": "calls"},
            "included": "1000",
            "prices": [
                {"currency": "EUR", "model": {"type": "per_unit", "unit_price": "0.02"}},
                {"currency": "eur", "model": {"type": "per_unit", "unit_price": "0.01"}}
            ]
        }));
        let problem = to_domain(&one_cell_twice, &refs, "/charges/3").unwrap_err();
        assert_eq!(problem.errors[0].path, "/charges/3/prices/1");

        let empty_package = charge(serde_json::json!({
            "product": "api",
            "quantity": {"type": "metered", "metric": "calls"},
            "prices": [{"currency": "EUR", "model": {"type": "package", "package_size": 0, "package_price": "5"}}]
        }));
        let problem = to_domain(&empty_package, &refs, "/charges/4").unwrap_err();
        assert_eq!(
            problem.errors[0].path,
            "/charges/4/prices/0/model/package_size"
        );
    }

    #[test]
    fn a_price_in_every_option_reads_as_a_star() {
        assert_eq!(option_set(&[1, 0], 2), OptionSet::Every);
        assert_eq!(option_set(&[1], 2), OptionSet::Positions(vec![1]));
    }
}
