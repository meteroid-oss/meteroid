use chrono::NaiveDateTime;
use common_domain::ids::{BillableMetricId, CreditUnitId, PriceId, ProductId, TenantId};
use diesel_models::prices::PriceRow;
use error_stack::Report;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::credit_units::PriceCreditUnit;
use super::enums::{BillingPeriodEnum, BillingType, FeeTypeEnum, SubscriptionFeeBillingPeriod};
use super::price_components::{DowngradePolicy, FeeType, UpgradePolicy, UsagePricingModel};
use super::subscription_components::SubscriptionFee;
use crate::errors::StoreError;

// structural billing config stored on Product.fee_structure
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
pub enum FeeStructure {
    Rate {},
    Slot {
        unit_name: String,
        upgrade_policy: UpgradePolicy,
        downgrade_policy: DowngradePolicy,
    },
    Capacity {
        metric_id: BillableMetricId,
    },
    Usage {
        metric_id: BillableMetricId,
        model: UsageModel,
    },
    ExtraRecurring {
        billing_type: BillingType,
    },
    OneTime {},
    Credit {},
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
pub enum UsageModel {
    PerUnit,
    Tiered,
    Volume,
    Package,
    Matrix,
}

impl From<&UsagePricingModel> for UsageModel {
    fn from(model: &UsagePricingModel) -> Self {
        match model {
            UsagePricingModel::PerUnit { .. } => UsageModel::PerUnit,
            UsagePricingModel::Tiered { .. } => UsageModel::Tiered,
            UsagePricingModel::Volume { .. } => UsageModel::Volume,
            UsagePricingModel::Package { .. } => UsageModel::Package,
            UsagePricingModel::Matrix { .. } => UsageModel::Matrix,
        }
    }
}

impl FeeStructure {
    pub fn is_credit(&self) -> bool {
        matches!(self, FeeStructure::Credit {})
    }

    fn kind(&self) -> &'static str {
        match self {
            FeeStructure::Rate {} => "rate",
            FeeStructure::Slot { .. } => "seat",
            FeeStructure::Capacity { .. } => "capacity",
            FeeStructure::Usage { .. } => "usage",
            FeeStructure::ExtraRecurring { .. } => "recurring",
            FeeStructure::OneTime {} => "one-time",
            FeeStructure::Credit {} => "credit pack",
        }
    }
}

// monetary config stored on Price.pricing
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
pub enum Pricing {
    Rate {
        rate: Decimal,
    },
    Slot {
        unit_rate: Decimal,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_slots: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_slots: Option<u32>,
    },
    Capacity {
        rate: Decimal,
        included: u64,
        overage_rate: Decimal,
    },
    Usage(UsagePricingModel),
    ExtraRecurring {
        unit_price: Decimal,
        quantity: u32,
    },
    OneTime {
        unit_price: Decimal,
        quantity: u32,
    },
    Credit {
        unit_price: Decimal,
        quantity: u32,
        credit_amount: Decimal,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expiry: Option<super::price_components::CreditExpiry>,
        /// The unit `credit_amount` is granted in; absent is the price currency. The pack itself
        /// is always paid in money.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        credit_unit_id: Option<CreditUnitId>,
        /// Only on included credit (zero price).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rollover: Option<super::price_components::CreditRollover>,
        /// Billed on the subscription's first invoice only; never rolls over.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        once: bool,
    },
}

impl Pricing {
    fn kind(&self) -> &'static str {
        match self {
            Pricing::Rate { .. } => "rate",
            Pricing::Slot { .. } => "seat",
            Pricing::Capacity { .. } => "capacity",
            Pricing::Usage(_) => "usage",
            Pricing::ExtraRecurring { .. } => "recurring",
            Pricing::OneTime { .. } => "one-time",
            Pricing::Credit { .. } => "credit pack",
        }
    }
}

/// Refuses a credit pack that cannot fund an exact amount: the granted credit must be a whole,
/// non-negative number of minor units of the price currency, with a positive quantity and a
/// usable expiry.
pub fn validate_pricing(pricing: &Pricing, currency: &str) -> Result<(), StoreError> {
    let Pricing::Credit {
        unit_price,
        quantity,
        credit_amount,
        expiry,
        credit_unit_id,
        rollover,
        once,
    } = pricing
    else {
        return Ok(());
    };
    validate_credit_once(*once, rollover.as_ref())?;
    // A unit's own precision is checked once the unit is loaded (`ensure_credit_packs_usable`).
    let unit_precision = credit_unit_id.map(|_| super::credit_units::CREDIT_UNIT_MAX_PRECISION);
    let precision = validate_credit_pack(
        *unit_price,
        *quantity,
        *credit_amount,
        *expiry,
        currency,
        unit_precision,
    )?;
    validate_credit_rollover(*unit_price, *expiry, rollover.as_ref(), precision)
}

/// A one-time grant has no next period to carry its credit into.
pub fn validate_credit_once(
    once: bool,
    rollover: Option<&super::price_components::CreditRollover>,
) -> Result<(), StoreError> {
    if once && rollover.is_some() {
        return Err(StoreError::InvalidArgument(
            "credit granted once cannot roll over".to_owned(),
        ));
    }
    Ok(())
}

/// Longest rollover term, so the rolled credit's expiry always resolves.
const MAX_ROLLOVER_DAYS: u32 = 3_660;
const MAX_ROLLOVER_MONTHS: u32 = 120;

/// Only included credit (a free pack) that expires rolls over, for a term of days or months,
/// capped by a positive amount in whole minor units of the pack's denomination (`precision`).
pub fn validate_credit_rollover(
    unit_price: Decimal,
    expiry: Option<super::price_components::CreditExpiry>,
    rollover: Option<&super::price_components::CreditRollover>,
    precision: u8,
) -> Result<(), StoreError> {
    use super::price_components::CreditExpiry;
    let invalid = |message: &str| Err(StoreError::InvalidArgument(message.to_owned()));
    let Some(rollover) = rollover else {
        return Ok(());
    };
    if !unit_price.is_zero() {
        return invalid("only credit included in the plan (a free pack) rolls over");
    }
    if expiry.is_none() {
        return invalid("credit that never expires cannot roll over");
    }
    match rollover.expiry {
        CreditExpiry::Day(days) if (1..=MAX_ROLLOVER_DAYS).contains(&days) => {}
        CreditExpiry::Month(months) if (1..=MAX_ROLLOVER_MONTHS).contains(&months) => {}
        CreditExpiry::Day(_) | CreditExpiry::Month(_) => {
            return invalid("rolled credit expires 1 to 3660 days or 1 to 120 months later");
        }
        CreditExpiry::BillingPeriod => {
            return invalid("rolled credit expires a number of days or months later");
        }
    }
    if let Some(max) = rollover.max_amount {
        if max <= Decimal::ZERO {
            return invalid("a rollover cap must be positive");
        }
        if max.normalize().scale() > u32::from(precision) {
            return invalid(&decimals_allowed("a rollover cap", precision));
        }
    }
    Ok(())
}

/// A price expressed in a credit unit: only usage, or a capacity fee's overage, at a rate that is
/// not negative. A zero rate leaves credits beyond the balance unbilled, for a plan that pauses
/// usage once the credit runs out.
pub fn validate_price_credit_unit(
    pricing: &Pricing,
    credit_unit: Option<&PriceCreditUnit>,
) -> Result<(), StoreError> {
    let Some(unit) = credit_unit else {
        return Ok(());
    };
    if !matches!(pricing, Pricing::Usage(_) | Pricing::Capacity { .. }) {
        return Err(StoreError::InvalidArgument(
            "only usage and capacity overage can be priced in a credit unit".to_owned(),
        ));
    }
    if unit.rate < Decimal::ZERO {
        return Err(StoreError::InvalidArgument(
            "a credit unit's rate to the price currency must not be negative".to_owned(),
        ));
    }
    Ok(())
}

fn decimals_allowed(what: &str, precision: u8) -> String {
    match precision {
        0 => format!("{what} is a whole number"),
        1 => format!("{what} has at most 1 decimal place"),
        n => format!("{what} has at most {n} decimal places"),
    }
}

/// See [`validate_pricing`]; shared with credit packs set directly on a subscription.
/// `unit_precision` is the precision of the credit unit the pack grants, if any; returns the
/// precision the granted credit was checked against.
pub fn validate_credit_pack(
    unit_price: Decimal,
    quantity: u32,
    credit_amount: Decimal,
    expiry: Option<super::price_components::CreditExpiry>,
    currency: &str,
    unit_precision: Option<u8>,
) -> Result<u8, StoreError> {
    let invalid = |message: &str| Err(StoreError::InvalidArgument(message.to_owned()));
    let Some(currency_precision) =
        crate::constants::Currencies::resolve_currency_precision(currency)
    else {
        return invalid("a credit pack is priced in a supported currency");
    };
    let precision = unit_precision.unwrap_or(currency_precision);
    if quantity == 0 {
        return invalid("a credit pack sells a positive quantity");
    }
    if unit_price.is_sign_negative() || credit_amount.is_sign_negative() {
        return invalid("a credit pack's price and credit cannot be negative");
    }
    if credit_amount.is_zero() {
        return invalid("a credit pack grants a positive amount of credit");
    }
    // Money credit is minted as paid credit worth its price plus bonus credit, never less.
    if unit_precision.is_none() && unit_price > credit_amount {
        return invalid("a credit pack cannot cost more than the credit it grants");
    }
    if credit_amount.normalize().scale() > u32::from(precision) {
        return invalid(&decimals_allowed("a credit pack's credit", precision));
    }
    if matches!(
        expiry,
        Some(super::price_components::CreditExpiry::Day(0))
            | Some(super::price_components::CreditExpiry::Month(0))
    ) {
        return invalid("a credit pack expiry must be at least one day or month");
    }
    Ok(precision)
}

impl Pricing {
    /// Revenue taxonomy, matching `SubscriptionFee::charge_type()`. Used to recover
    /// the charge type of v2 (price-resolved) components for legacy invoice lines.
    pub fn charge_type(&self) -> super::invoice_lines::ChargeType {
        use super::invoice_lines::ChargeType;
        match self {
            Pricing::Capacity { .. } | Pricing::Usage(_) => ChargeType::Usage,
            Pricing::OneTime { .. } | Pricing::Credit { .. } => ChargeType::OneTime,
            Pricing::Rate { .. } | Pricing::Slot { .. } | Pricing::ExtraRecurring { .. } => {
                ChargeType::Recurring
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct Price {
    pub id: PriceId,
    pub product_id: ProductId,
    pub cadence: BillingPeriodEnum,
    pub currency: String,
    pub pricing: Pricing,
    pub tenant_id: TenantId,
    pub created_at: NaiveDateTime,
    pub archived_at: Option<NaiveDateTime>,
    pub catalog: bool,
    pub credit_unit: Option<PriceCreditUnit>,
}

impl TryFrom<PriceRow> for Price {
    type Error = Report<StoreError>;

    fn try_from(row: PriceRow) -> Result<Self, Self::Error> {
        let pricing: Pricing = serde_json::from_value(row.pricing).map_err(|e| {
            Report::new(StoreError::SerdeError(
                "Failed to deserialize Pricing".to_string(),
                e,
            ))
        })?;

        let credit_unit = match (row.credit_unit_id, row.credit_unit_rate) {
            (Some(unit_id), Some(rate)) => Some(PriceCreditUnit { unit_id, rate }),
            (None, None) => None,
            _ => {
                return Err(Report::new(StoreError::InvalidArgument(
                    "a credit-priced price carries both its unit and its rate".to_owned(),
                )));
            }
        };

        Ok(Price {
            id: row.id,
            product_id: row.product_id,
            cadence: row.cadence.into(),
            currency: row.currency,
            pricing,
            tenant_id: row.tenant_id,
            created_at: row.created_at,
            archived_at: row.archived_at,
            catalog: row.catalog,
            credit_unit,
        })
    }
}

// -----------------------------------------------------------------------
// Matrix mutation types
// -----------------------------------------------------------------------

use super::price_components::MatrixDimension;

/// A dimension combination key for matching rows across prices.
#[derive(Clone, Debug)]
pub struct MatrixDimensionKey {
    pub dimension1: MatrixDimension,
    pub dimension2: Option<MatrixDimension>,
}

/// A dimension combination to add, with per-currency prices.
#[derive(Clone, Debug)]
pub struct MatrixRowAdd {
    pub dimension1: MatrixDimension,
    pub dimension2: Option<MatrixDimension>,
    /// Per-unit price keyed by currency code (e.g. "USD" → 0.05).
    /// Currencies not in the map default to Decimal::ZERO.
    pub per_unit_prices: HashMap<String, rust_decimal::Decimal>,
}

/// Update: add/remove dimension combinations across all matrix prices for a product.
#[derive(Clone, Debug)]
pub struct MatrixPriceUpdate {
    /// Rows to add to all matrix prices, each with per-currency prices.
    pub add_rows: Vec<MatrixRowAdd>,
    /// Dimension combinations to remove from all matrix prices.
    pub remove_rows: Vec<MatrixDimensionKey>,
}

#[derive(Clone, Debug)]
pub struct AffectedPlan {
    pub plan_name: String,
    pub versions: Vec<i32>,
}

#[derive(Clone, Debug)]
pub struct MatrixUpdatePreview {
    pub affected_prices_count: usize,
    pub affected_subscriptions_count: usize,
    /// Total rows that will be added across all prices.
    pub rows_to_add: usize,
    /// Total rows that will be removed across all prices.
    pub rows_to_remove: usize,
    pub affected_plans: Vec<AffectedPlan>,
}

/// The credit units a price's terms refer to: the one its usage is priced in, the one its pack
/// grants.
pub fn credit_unit_ids(
    pricing: &Pricing,
    credit_unit: Option<&PriceCreditUnit>,
) -> impl Iterator<Item = CreditUnitId> {
    let granted = match pricing {
        Pricing::Credit { credit_unit_id, .. } => *credit_unit_id,
        _ => None,
    };
    credit_unit
        .map(|unit| unit.unit_id)
        .into_iter()
        .chain(granted)
}

impl Price {
    pub fn credit_unit_ids(&self) -> impl Iterator<Item = CreditUnitId> {
        credit_unit_ids(&self.pricing, self.credit_unit.as_ref())
    }
}

fn loaded_unit(
    units: &super::credit_units::CreditUnitsById,
    unit_id: CreditUnitId,
) -> Result<&super::credit_units::CreditUnit, StoreError> {
    units.get(&unit_id).ok_or_else(|| {
        StoreError::InvalidArgument(format!("credit unit {unit_id} of the price is not loaded"))
    })
}

/// Resolution: Product (FeeStructure) + Price (Pricing, and the credit unit its usage is priced
/// in) → SubscriptionFee. `units` must hold every unit of `credit_unit_ids`.
pub fn resolve_subscription_fee(
    structure: &FeeStructure,
    pricing: &Pricing,
    credit_unit: Option<&PriceCreditUnit>,
    units: &super::credit_units::CreditUnitsById,
    params: Option<&super::subscription_components::ComponentParameters>,
) -> Result<SubscriptionFee, StoreError> {
    let mut fee = resolve_fee_shape(structure, pricing, params)?;
    if let Some(priced_in) = credit_unit {
        let unit = loaded_unit(units, priced_in.unit_id)?;
        fee.price_in_credit_unit(super::credit_units::FeeCreditUnit {
            unit_id: unit.id,
            code: unit.code.as_str().to_owned(),
            precision: unit.precision,
            rate: priced_in.rate,
        })?;
    }
    if let (
        Pricing::Credit {
            credit_unit_id: Some(unit_id),
            ..
        },
        SubscriptionFee::Credit { credit_unit, .. },
    ) = (pricing, &mut fee)
    {
        *credit_unit = Some(loaded_unit(units, *unit_id)?.reference());
    }
    Ok(fee)
}

fn resolve_fee_shape(
    structure: &FeeStructure,
    pricing: &Pricing,
    params: Option<&super::subscription_components::ComponentParameters>,
) -> Result<SubscriptionFee, StoreError> {
    match (structure, pricing) {
        (FeeStructure::Rate {}, Pricing::Rate { rate }) => {
            Ok(SubscriptionFee::Rate { rate: *rate })
        }
        (
            FeeStructure::Slot { unit_name, .. },
            Pricing::Slot {
                unit_rate,
                min_slots,
                max_slots,
            },
        ) => {
            let initial_slots = params
                .and_then(|p| p.initial_slot_count)
                .unwrap_or_else(|| min_slots.unwrap_or(0));
            Ok(SubscriptionFee::Slot {
                unit: unit_name.clone(),
                unit_rate: *unit_rate,
                min_slots: *min_slots,
                max_slots: *max_slots,
                initial_slots,
            })
        }
        (
            FeeStructure::Capacity { metric_id },
            Pricing::Capacity {
                rate,
                included,
                overage_rate,
            },
        ) => Ok(SubscriptionFee::Capacity {
            metric_id: *metric_id,
            rate: *rate,
            included: *included,
            overage_rate: *overage_rate,
            credit_unit: None,
        }),
        (FeeStructure::Usage { metric_id, .. }, Pricing::Usage(model)) => {
            Ok(SubscriptionFee::Usage {
                metric_id: *metric_id,
                model: model.clone(),
                credit_unit: None,
            })
        }
        (
            FeeStructure::ExtraRecurring { billing_type },
            Pricing::ExtraRecurring {
                unit_price,
                quantity,
            },
        ) => Ok(SubscriptionFee::Recurring {
            rate: *unit_price,
            quantity: *quantity,
            billing_type: billing_type.clone(),
        }),
        (
            FeeStructure::OneTime {},
            Pricing::OneTime {
                unit_price,
                quantity,
            },
        ) => Ok(SubscriptionFee::OneTime {
            rate: *unit_price,
            quantity: *quantity,
        }),
        (
            FeeStructure::Credit {},
            Pricing::Credit {
                unit_price,
                quantity,
                credit_amount,
                expiry,
                credit_unit_id: _,
                rollover,
                once,
            },
        ) => Ok(SubscriptionFee::Credit {
            rate: *unit_price,
            quantity: *quantity,
            credit_amount: *credit_amount,
            expiry: *expiry,
            credit_unit: None,
            rollover: *rollover,
            once: *once,
        }),
        _ => Err(StoreError::InvalidArgument(format!(
            "A {} price cannot price a {} fee",
            pricing.kind(),
            structure.kind()
        ))),
    }
}

// Extraction: FeeType → FeeStructure + Pricing (for backfill)
pub fn extract_fee_structure(fee: &FeeType) -> (FeeTypeEnum, FeeStructure) {
    match fee {
        FeeType::Rate { .. } => (FeeTypeEnum::Rate, FeeStructure::Rate {}),
        FeeType::Slot {
            slot_unit_name,
            upgrade_policy,
            downgrade_policy,
            ..
        } => (
            FeeTypeEnum::Slot,
            FeeStructure::Slot {
                unit_name: slot_unit_name.clone(),
                upgrade_policy: upgrade_policy.clone(),
                downgrade_policy: downgrade_policy.clone(),
            },
        ),
        FeeType::Capacity { metric_id, .. } => (
            FeeTypeEnum::Capacity,
            FeeStructure::Capacity {
                metric_id: *metric_id,
            },
        ),
        FeeType::Usage {
            metric_id, pricing, ..
        } => (
            FeeTypeEnum::Usage,
            FeeStructure::Usage {
                metric_id: *metric_id,
                model: UsageModel::from(pricing),
            },
        ),
        FeeType::ExtraRecurring { billing_type, .. } => (
            FeeTypeEnum::ExtraRecurring,
            FeeStructure::ExtraRecurring {
                billing_type: billing_type.clone(),
            },
        ),
        FeeType::OneTime { .. } => (FeeTypeEnum::OneTime, FeeStructure::OneTime {}),
        FeeType::Credit { .. } => (FeeTypeEnum::Credit, FeeStructure::Credit {}),
    }
}

pub fn extract_pricing(fee: &FeeType) -> Vec<(BillingPeriodEnum, Pricing)> {
    match fee {
        FeeType::Rate { rates } => rates
            .iter()
            .map(|tr| (tr.term, Pricing::Rate { rate: tr.price }))
            .collect(),
        FeeType::Slot {
            rates,
            minimum_count,
            quota,
            ..
        } => rates
            .iter()
            .map(|tr| {
                (
                    tr.term,
                    Pricing::Slot {
                        unit_rate: tr.price,
                        min_slots: *minimum_count,
                        max_slots: *quota,
                    },
                )
            })
            .collect(),
        FeeType::Capacity {
            thresholds,
            cadence,
            ..
        } => {
            // Each threshold becomes a separate Price; the subscriber selects
            // the desired tier via `committed_capacity` in ComponentParameters.
            thresholds
                .iter()
                .map(|t| {
                    (
                        *cadence,
                        Pricing::Capacity {
                            rate: t.price,
                            included: t.included_amount,
                            overage_rate: t.per_unit_overage,
                        },
                    )
                })
                .collect()
        }
        FeeType::Usage {
            pricing, cadence, ..
        } => {
            vec![(*cadence, Pricing::Usage(pricing.clone()))]
        }
        FeeType::ExtraRecurring {
            unit_price,
            quantity,
            cadence,
            ..
        } => {
            vec![(
                *cadence,
                Pricing::ExtraRecurring {
                    unit_price: *unit_price,
                    quantity: *quantity,
                },
            )]
        }
        FeeType::OneTime {
            unit_price,
            quantity,
        } => {
            // OneTime doesn't have a billing period, use Monthly as placeholder
            vec![(
                BillingPeriodEnum::Monthly,
                Pricing::OneTime {
                    unit_price: *unit_price,
                    quantity: *quantity,
                },
            )]
        }
        FeeType::Credit {
            unit_price,
            quantity,
            credit_amount,
            expiry,
            once,
        } => {
            // No legacy fee carries a pack; new packs are priced with a real cadence.
            vec![(
                BillingPeriodEnum::Monthly,
                Pricing::Credit {
                    unit_price: *unit_price,
                    quantity: *quantity,
                    credit_amount: *credit_amount,
                    expiry: *expiry,
                    credit_unit_id: None,
                    rollover: None,
                    once: *once,
                },
            )]
        }
    }
}

/// Extracted from Row legacy_fee — value types only, no database IDs.
#[derive(Debug, Clone)]
pub struct LegacyPricingData {
    pub fee_structure: FeeStructure,
    pub pricing_entries: Vec<(BillingPeriodEnum, Pricing)>,
    pub currency: String,
    /// Original FeeType kept for REST API backward compatibility
    pub fee_type: FeeType,
}

pub fn extract_legacy_pricing(
    legacy_fee_json: &serde_json::Value,
    currency: String,
) -> Result<LegacyPricingData, Report<StoreError>> {
    let fee: FeeType = serde_json::from_value(legacy_fee_json.clone()).map_err(|e| {
        Report::new(StoreError::SerdeError(
            "Failed to parse legacy_fee".to_string(),
            e,
        ))
    })?;
    let (_, fee_structure) = extract_fee_structure(&fee);
    let pricing_entries = extract_pricing(&fee);
    Ok(LegacyPricingData {
        fee_structure,
        pricing_entries,
        currency,
        fee_type: fee,
    })
}

pub fn fee_type_billing_period(structure: &FeeStructure) -> Option<SubscriptionFeeBillingPeriod> {
    match structure {
        FeeStructure::OneTime {} => Some(SubscriptionFeeBillingPeriod::OneTime),
        _ => None, // cadence comes from the Price
    }
}

/// Sync fee resolution using pre-loaded prices.
/// For PriceEntry::Existing, looks up in the provided map.
/// For PriceEntry::New, extracts pricing directly from the payload.
pub fn resolve_fee_from_entry(
    fee_structure: &FeeStructure,
    price_entry: &super::price_components::PriceEntry,
    prices_by_id: &HashMap<PriceId, Price>,
    units: &super::credit_units::CreditUnitsById,
) -> Result<(SubscriptionFee, SubscriptionFeeBillingPeriod), StoreError> {
    match price_entry {
        super::price_components::PriceEntry::Existing(price_id) => {
            let price = prices_by_id.get(price_id).ok_or_else(|| {
                StoreError::InvalidArgument(format!("Price {} not found", price_id))
            })?;
            let fee = resolve_subscription_fee(
                fee_structure,
                &price.pricing,
                price.credit_unit.as_ref(),
                units,
                None,
            )?;
            let period = fee_type_billing_period(fee_structure)
                .unwrap_or_else(|| price.cadence.as_subscription_billing_period());
            Ok((fee, period))
        }
        super::price_components::PriceEntry::New(price_input) => {
            let fee = resolve_subscription_fee(
                fee_structure,
                &price_input.pricing,
                price_input.credit_unit.as_ref(),
                units,
                None,
            )?;
            let period = fee_type_billing_period(fee_structure)
                .unwrap_or_else(|| price_input.cadence.as_subscription_billing_period());
            Ok((fee, period))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::price_components::*;
    use common_domain::ids::BaseId;
    use rust_decimal_macros::dec;

    // -----------------------------------------------------------------------
    // resolve_subscription_fee tests
    // -----------------------------------------------------------------------

    fn no_units() -> crate::domain::credit_units::CreditUnitsById {
        crate::domain::credit_units::CreditUnitsById::new()
    }

    #[test]
    fn a_price_of_another_fee_type_is_refused_in_words() {
        let err = resolve_subscription_fee(
            &FeeStructure::Credit {},
            &Pricing::Rate { rate: dec!(10) },
            None,
            &no_units(),
            None,
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            StoreError::InvalidArgument("A rate price cannot price a credit pack fee".into())
                .to_string()
        );
    }

    fn credits_unit() -> crate::domain::credit_units::CreditUnit {
        let now = common_domain::clock::now();
        crate::domain::credit_units::CreditUnit {
            id: CreditUnitId::new(),
            tenant_id: TenantId::new(),
            code: crate::domain::credit_units::CreditUnitCode::new("CREDITS").unwrap(),
            name: "Credit".to_owned(),
            plural_name: "Credits".to_owned(),
            symbol: None,
            precision: 2,
            created_at: now,
            updated_at: now,
            archived_at: None,
            product_id: None,
        }
    }

    #[test]
    fn usage_priced_in_a_credit_unit_resolves_with_the_unit_and_its_rate() {
        let unit = credits_unit();
        let units = crate::domain::credit_units::CreditUnitsById::from([(unit.id, unit.clone())]);
        let structure = FeeStructure::Usage {
            metric_id: BillableMetricId::new(),
            model: UsageModel::PerUnit,
        };
        let pricing = Pricing::Usage(UsagePricingModel::PerUnit { rate: dec!(3) });
        let priced_in = PriceCreditUnit {
            unit_id: unit.id,
            rate: dec!(0.01),
        };
        let fee =
            resolve_subscription_fee(&structure, &pricing, Some(&priced_in), &units, None).unwrap();
        let SubscriptionFee::Usage { credit_unit, .. } = fee else {
            panic!("Expected Usage");
        };
        assert_eq!(
            credit_unit,
            Some(crate::domain::credit_units::FeeCreditUnit {
                unit_id: unit.id,
                code: "CREDITS".to_owned(),
                precision: 2,
                rate: dec!(0.01),
            })
        );

        let capacity = FeeStructure::Capacity {
            metric_id: BillableMetricId::new(),
        };
        let overage = Pricing::Capacity {
            rate: dec!(10),
            included: 100,
            overage_rate: dec!(2),
        };
        let fee =
            resolve_subscription_fee(&capacity, &overage, Some(&priced_in), &units, None).unwrap();
        assert!(matches!(
            fee,
            SubscriptionFee::Capacity { credit_unit: Some(ref priced), .. } if priced.unit_id == unit.id
        ));
    }

    #[test]
    fn a_unit_pack_resolves_granting_its_unit() {
        let unit = credits_unit();
        let units = crate::domain::credit_units::CreditUnitsById::from([(unit.id, unit.clone())]);
        let pricing = Pricing::Credit {
            unit_price: dec!(40),
            quantity: 1,
            credit_amount: dec!(5000),
            expiry: None,
            credit_unit_id: Some(unit.id),
            rollover: None,
            once: false,
        };
        let fee = resolve_subscription_fee(&FeeStructure::Credit {}, &pricing, None, &units, None)
            .unwrap();
        let SubscriptionFee::Credit { credit_unit, .. } = fee else {
            panic!("Expected Credit");
        };
        assert_eq!(credit_unit, Some(unit.reference()));
    }

    /// A fee whose unit was not loaded is refused: resolving it without would bill credits as money.
    #[test]
    fn a_fee_whose_credit_unit_is_not_loaded_is_refused() {
        let unit = credits_unit();
        let usage = FeeStructure::Usage {
            metric_id: BillableMetricId::new(),
            model: UsageModel::PerUnit,
        };
        let priced_in = PriceCreditUnit {
            unit_id: unit.id,
            rate: dec!(0.01),
        };
        let per_unit = Pricing::Usage(UsagePricingModel::PerUnit { rate: dec!(3) });
        assert!(
            resolve_subscription_fee(&usage, &per_unit, Some(&priced_in), &no_units(), None)
                .is_err()
        );
        let pack = Pricing::Credit {
            unit_price: dec!(40),
            quantity: 1,
            credit_amount: dec!(5000),
            expiry: None,
            credit_unit_id: Some(unit.id),
            rollover: None,
            once: false,
        };
        assert!(
            resolve_subscription_fee(&FeeStructure::Credit {}, &pack, None, &no_units(), None)
                .is_err()
        );
        assert_eq!(
            credit_unit_ids(&pack, Some(&priced_in)).collect::<Vec<_>>(),
            vec![unit.id, unit.id]
        );
    }

    #[test]
    fn test_resolve_rate() {
        let structure = FeeStructure::Rate {};
        let pricing = Pricing::Rate { rate: dec!(9.99) };
        let fee = resolve_subscription_fee(&structure, &pricing, None, &no_units(), None).unwrap();
        match fee {
            SubscriptionFee::Rate { rate } => assert_eq!(rate, dec!(9.99)),
            _ => panic!("Expected Rate"),
        }
    }

    #[test]
    fn test_resolve_slot() {
        let structure = FeeStructure::Slot {
            unit_name: "seat".to_string(),
            upgrade_policy: UpgradePolicy::Prorated,
            downgrade_policy: DowngradePolicy::RemoveAtEndOfPeriod,
        };
        let pricing = Pricing::Slot {
            unit_rate: dec!(10.00),
            min_slots: Some(1),
            max_slots: Some(100),
        };
        let fee = resolve_subscription_fee(&structure, &pricing, None, &no_units(), None).unwrap();
        match fee {
            SubscriptionFee::Slot {
                unit,
                unit_rate,
                min_slots,
                max_slots,
                initial_slots,
            } => {
                assert_eq!(unit, "seat");
                assert_eq!(unit_rate, dec!(10.00));
                assert_eq!(min_slots, Some(1));
                assert_eq!(max_slots, Some(100));
                assert_eq!(initial_slots, 1); // defaults to min_slots
            }
            _ => panic!("Expected Slot"),
        }
    }

    #[test]
    fn test_resolve_slot_with_initial_slot_count() {
        let structure = FeeStructure::Slot {
            unit_name: "seat".to_string(),
            upgrade_policy: UpgradePolicy::Prorated,
            downgrade_policy: DowngradePolicy::RemoveAtEndOfPeriod,
        };
        let pricing = Pricing::Slot {
            unit_rate: dec!(10.00),
            min_slots: Some(1),
            max_slots: Some(100),
        };
        let params = crate::domain::subscription_components::ComponentParameters {
            initial_slot_count: Some(5),
            billing_period: None,
            committed_capacity: None,
        };
        let fee = resolve_subscription_fee(&structure, &pricing, None, &no_units(), Some(&params))
            .unwrap();
        match fee {
            SubscriptionFee::Slot { initial_slots, .. } => {
                assert_eq!(initial_slots, 5);
            }
            _ => panic!("Expected Slot"),
        }
    }

    #[test]
    fn test_resolve_capacity() {
        let metric_id = BillableMetricId::default();
        let structure = FeeStructure::Capacity { metric_id };
        let pricing = Pricing::Capacity {
            rate: dec!(50.00),
            included: 1000,
            overage_rate: dec!(0.05),
        };
        let fee = resolve_subscription_fee(&structure, &pricing, None, &no_units(), None).unwrap();
        match fee {
            SubscriptionFee::Capacity {
                metric_id: mid,
                rate,
                included,
                overage_rate,
                ..
            } => {
                assert_eq!(mid, metric_id);
                assert_eq!(rate, dec!(50.00));
                assert_eq!(included, 1000);
                assert_eq!(overage_rate, dec!(0.05));
            }
            _ => panic!("Expected Capacity"),
        }
    }

    #[test]
    fn test_resolve_usage() {
        let metric_id = BillableMetricId::default();
        let structure = FeeStructure::Usage {
            metric_id,
            model: UsageModel::PerUnit,
        };
        let pricing = Pricing::Usage(UsagePricingModel::PerUnit { rate: dec!(0.01) });
        let fee = resolve_subscription_fee(&structure, &pricing, None, &no_units(), None).unwrap();
        match fee {
            SubscriptionFee::Usage {
                metric_id: mid,
                model,
                ..
            } => {
                assert_eq!(mid, metric_id);
                match model {
                    UsagePricingModel::PerUnit { rate } => assert_eq!(rate, dec!(0.01)),
                    _ => panic!("Expected PerUnit"),
                }
            }
            _ => panic!("Expected Usage"),
        }
    }

    #[test]
    fn test_resolve_extra_recurring() {
        let structure = FeeStructure::ExtraRecurring {
            billing_type: BillingType::Advance,
        };
        let pricing = Pricing::ExtraRecurring {
            unit_price: dec!(25.00),
            quantity: 2,
        };
        let fee = resolve_subscription_fee(&structure, &pricing, None, &no_units(), None).unwrap();
        match fee {
            SubscriptionFee::Recurring {
                rate,
                quantity,
                billing_type,
            } => {
                assert_eq!(rate, dec!(25.00));
                assert_eq!(quantity, 2);
                assert!(matches!(billing_type, BillingType::Advance));
            }
            _ => panic!("Expected Recurring"),
        }
    }

    #[test]
    fn test_resolve_one_time() {
        let structure = FeeStructure::OneTime {};
        let pricing = Pricing::OneTime {
            unit_price: dec!(100.00),
            quantity: 1,
        };
        let fee = resolve_subscription_fee(&structure, &pricing, None, &no_units(), None).unwrap();
        match fee {
            SubscriptionFee::OneTime { rate, quantity } => {
                assert_eq!(rate, dec!(100.00));
                assert_eq!(quantity, 1);
            }
            _ => panic!("Expected OneTime"),
        }
    }

    #[test]
    fn test_resolve_mismatch_errors() {
        let structure = FeeStructure::Rate {};
        let pricing = Pricing::Slot {
            unit_rate: dec!(10.0),
            min_slots: None,
            max_slots: None,
        };
        assert!(resolve_subscription_fee(&structure, &pricing, None, &no_units(), None).is_err());
    }

    // -----------------------------------------------------------------------
    // extract_fee_structure tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_extract_fee_structure_rate() {
        let fee = FeeType::Rate {
            rates: vec![TermRate {
                term: BillingPeriodEnum::Monthly,
                price: dec!(10.00),
            }],
        };
        let (ft, fs) = extract_fee_structure(&fee);
        assert_eq!(ft, FeeTypeEnum::Rate);
        assert!(matches!(fs, FeeStructure::Rate {}));
    }

    #[test]
    fn test_extract_fee_structure_slot() {
        let fee = FeeType::Slot {
            rates: vec![TermRate {
                term: BillingPeriodEnum::Monthly,
                price: dec!(5.00),
            }],
            slot_unit_name: "seat".to_string(),
            upgrade_policy: UpgradePolicy::Prorated,
            downgrade_policy: DowngradePolicy::RemoveAtEndOfPeriod,
            minimum_count: Some(1),
            quota: Some(10),
        };
        let (ft, fs) = extract_fee_structure(&fee);
        assert_eq!(ft, FeeTypeEnum::Slot);
        match fs {
            FeeStructure::Slot { unit_name, .. } => {
                assert_eq!(unit_name, "seat");
            }
            _ => panic!("Expected Slot"),
        }
    }

    #[test]
    fn test_extract_fee_structure_capacity() {
        let mid = BillableMetricId::default();
        let fee = FeeType::Capacity {
            metric_id: mid,
            thresholds: vec![CapacityThreshold {
                included_amount: 100,
                price: dec!(50.00),
                per_unit_overage: dec!(0.10),
            }],
            cadence: BillingPeriodEnum::Monthly,
        };
        let (ft, fs) = extract_fee_structure(&fee);
        assert_eq!(ft, FeeTypeEnum::Capacity);
        match fs {
            FeeStructure::Capacity { metric_id } => assert_eq!(metric_id, mid),
            _ => panic!("Expected Capacity"),
        }
    }

    #[test]
    fn test_extract_fee_structure_usage() {
        let mid = BillableMetricId::default();
        let fee = FeeType::Usage {
            metric_id: mid,
            pricing: UsagePricingModel::PerUnit { rate: dec!(0.01) },
            cadence: BillingPeriodEnum::Monthly,
        };
        let (ft, fs) = extract_fee_structure(&fee);
        assert_eq!(ft, FeeTypeEnum::Usage);
        match fs {
            FeeStructure::Usage { metric_id, model } => {
                assert_eq!(metric_id, mid);
                assert!(matches!(model, UsageModel::PerUnit));
            }
            _ => panic!("Expected Usage"),
        }
    }

    #[test]
    fn test_extract_fee_structure_extra_recurring() {
        let fee = FeeType::ExtraRecurring {
            unit_price: dec!(25.00),
            quantity: 1,
            billing_type: BillingType::Advance,
            cadence: BillingPeriodEnum::Monthly,
        };
        let (ft, fs) = extract_fee_structure(&fee);
        assert_eq!(ft, FeeTypeEnum::ExtraRecurring);
        match fs {
            FeeStructure::ExtraRecurring { billing_type } => {
                assert!(matches!(billing_type, BillingType::Advance));
            }
            _ => panic!("Expected ExtraRecurring"),
        }
    }

    #[test]
    fn test_extract_fee_structure_one_time() {
        let fee = FeeType::OneTime {
            unit_price: dec!(100.00),
            quantity: 1,
        };
        let (ft, fs) = extract_fee_structure(&fee);
        assert_eq!(ft, FeeTypeEnum::OneTime);
        assert!(matches!(fs, FeeStructure::OneTime {}));
    }

    // -----------------------------------------------------------------------
    // extract_pricing tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_extract_pricing_rate_multi() {
        let fee = FeeType::Rate {
            rates: vec![
                TermRate {
                    term: BillingPeriodEnum::Monthly,
                    price: dec!(10.00),
                },
                TermRate {
                    term: BillingPeriodEnum::Annual,
                    price: dec!(100.00),
                },
            ],
        };
        let prices = extract_pricing(&fee);
        assert_eq!(prices.len(), 2);
        assert_eq!(prices[0].0, BillingPeriodEnum::Monthly);
        assert_eq!(prices[1].0, BillingPeriodEnum::Annual);
        match &prices[0].1 {
            Pricing::Rate { rate } => assert_eq!(*rate, dec!(10.00)),
            _ => panic!("Expected Rate"),
        }
        match &prices[1].1 {
            Pricing::Rate { rate } => assert_eq!(*rate, dec!(100.00)),
            _ => panic!("Expected Rate"),
        }
    }

    #[test]
    fn test_extract_pricing_slot() {
        let fee = FeeType::Slot {
            rates: vec![TermRate {
                term: BillingPeriodEnum::Monthly,
                price: dec!(5.00),
            }],
            slot_unit_name: "seat".to_string(),
            upgrade_policy: UpgradePolicy::Prorated,
            downgrade_policy: DowngradePolicy::RemoveAtEndOfPeriod,
            minimum_count: Some(1),
            quota: Some(10),
        };
        let prices = extract_pricing(&fee);
        assert_eq!(prices.len(), 1);
        match &prices[0].1 {
            Pricing::Slot {
                unit_rate,
                min_slots,
                max_slots,
            } => {
                assert_eq!(*unit_rate, dec!(5.00));
                assert_eq!(*min_slots, Some(1));
                assert_eq!(*max_slots, Some(10));
            }
            _ => panic!("Expected Slot"),
        }
    }

    #[test]
    fn test_extract_pricing_usage() {
        let mid = BillableMetricId::default();
        let fee = FeeType::Usage {
            metric_id: mid,
            pricing: UsagePricingModel::Tiered {
                tiers: vec![TierRow {
                    first_unit: 0,
                    rate: dec!(0.10),
                    flat_fee: None,
                    flat_cap: None,
                }],
                block_size: None,
            },
            cadence: BillingPeriodEnum::Monthly,
        };
        let prices = extract_pricing(&fee);
        assert_eq!(prices.len(), 1);
        assert_eq!(prices[0].0, BillingPeriodEnum::Monthly);
        match &prices[0].1 {
            Pricing::Usage(UsagePricingModel::Tiered { tiers, .. }) => {
                assert_eq!(tiers.len(), 1);
            }
            _ => panic!("Expected Usage Tiered"),
        }
    }

    #[test]
    fn test_extract_pricing_one_time() {
        let fee = FeeType::OneTime {
            unit_price: dec!(100.00),
            quantity: 1,
        };
        let prices = extract_pricing(&fee);
        assert_eq!(prices.len(), 1);
        match &prices[0].1 {
            Pricing::OneTime {
                unit_price,
                quantity,
            } => {
                assert_eq!(*unit_price, dec!(100.00));
                assert_eq!(*quantity, 1);
            }
            _ => panic!("Expected OneTime"),
        }
    }

    // -----------------------------------------------------------------------
    // Serde round-trip tests for FeeStructure
    // -----------------------------------------------------------------------

    #[test]
    fn test_fee_structure_serde_roundtrip() {
        let structures = vec![
            FeeStructure::Rate {},
            FeeStructure::Slot {
                unit_name: "seat".to_string(),
                upgrade_policy: UpgradePolicy::Prorated,
                downgrade_policy: DowngradePolicy::RemoveAtEndOfPeriod,
            },
            FeeStructure::Capacity {
                metric_id: BillableMetricId::default(),
            },
            FeeStructure::Usage {
                metric_id: BillableMetricId::default(),
                model: UsageModel::PerUnit,
            },
            FeeStructure::Usage {
                metric_id: BillableMetricId::default(),
                model: UsageModel::Matrix,
            },
            FeeStructure::ExtraRecurring {
                billing_type: BillingType::Advance,
            },
            FeeStructure::OneTime {},
        ];

        for structure in structures {
            let json = serde_json::to_value(&structure).unwrap();
            let deser: FeeStructure = serde_json::from_value(json).unwrap();
            // Re-serialize and compare
            let json1 = serde_json::to_string(&structure).unwrap();
            let json2 = serde_json::to_string(&deser).unwrap();
            assert_eq!(json1, json2, "Round-trip failed for {:?}", structure);
        }
    }

    // -----------------------------------------------------------------------
    // Serde round-trip tests for Pricing
    // -----------------------------------------------------------------------

    #[test]
    fn test_pricing_serde_roundtrip() {
        let pricings = vec![
            Pricing::Rate { rate: dec!(9.99) },
            Pricing::Slot {
                unit_rate: dec!(10.00),
                min_slots: Some(1),
                max_slots: None,
            },
            Pricing::Capacity {
                rate: dec!(50.00),
                included: 1000,
                overage_rate: dec!(0.05),
            },
            Pricing::Usage(UsagePricingModel::PerUnit { rate: dec!(0.01) }),
            Pricing::Usage(UsagePricingModel::Tiered {
                tiers: vec![TierRow {
                    first_unit: 0,
                    rate: dec!(0.10),
                    flat_fee: None,
                    flat_cap: None,
                }],
                block_size: Some(100),
            }),
            Pricing::Usage(UsagePricingModel::Matrix {
                rates: vec![MatrixRow {
                    dimension1: MatrixDimension {
                        key: "model".to_string(),
                        value: "gpt-4".to_string(),
                    },
                    dimension2: None,
                    per_unit_price: dec!(0.03),
                }],
            }),
            Pricing::ExtraRecurring {
                unit_price: dec!(25.00),
                quantity: 2,
            },
            Pricing::OneTime {
                unit_price: dec!(100.00),
                quantity: 1,
            },
        ];

        for pricing in pricings {
            let json = serde_json::to_value(&pricing).unwrap();
            let deser: Pricing = serde_json::from_value(json).unwrap();
            let json1 = serde_json::to_string(&pricing).unwrap();
            let json2 = serde_json::to_string(&deser).unwrap();
            assert_eq!(json1, json2, "Round-trip failed for {:?}", pricing);
        }
    }

    // -----------------------------------------------------------------------
    // Matrix-specific extract tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_extract_fee_structure_matrix() {
        let mid = BillableMetricId::default();
        let fee = FeeType::Usage {
            metric_id: mid,
            pricing: UsagePricingModel::Matrix {
                rates: vec![
                    MatrixRow {
                        dimension1: MatrixDimension {
                            key: "model".to_string(),
                            value: "gpt-4".to_string(),
                        },
                        dimension2: None,
                        per_unit_price: dec!(0.03),
                    },
                    MatrixRow {
                        dimension1: MatrixDimension {
                            key: "model".to_string(),
                            value: "gpt-3.5".to_string(),
                        },
                        dimension2: None,
                        per_unit_price: dec!(0.01),
                    },
                ],
            },
            cadence: BillingPeriodEnum::Monthly,
        };

        let (ft, fs) = extract_fee_structure(&fee);
        assert_eq!(ft, FeeTypeEnum::Usage);
        match fs {
            FeeStructure::Usage { metric_id, model } => {
                assert_eq!(metric_id, mid);
                assert!(matches!(model, UsageModel::Matrix));
            }
            _ => panic!("Expected Usage"),
        }
    }

    #[test]
    fn test_extract_pricing_matrix() {
        let mid = BillableMetricId::default();
        let fee = FeeType::Usage {
            metric_id: mid,
            pricing: UsagePricingModel::Matrix {
                rates: vec![MatrixRow {
                    dimension1: MatrixDimension {
                        key: "model".to_string(),
                        value: "gpt-4".to_string(),
                    },
                    dimension2: Some(MatrixDimension {
                        key: "region".to_string(),
                        value: "us".to_string(),
                    }),
                    per_unit_price: dec!(0.05),
                }],
            },
            cadence: BillingPeriodEnum::Monthly,
        };

        let prices = extract_pricing(&fee);
        assert_eq!(prices.len(), 1);
        assert_eq!(prices[0].0, BillingPeriodEnum::Monthly);
        match &prices[0].1 {
            Pricing::Usage(UsagePricingModel::Matrix { rates }) => {
                assert_eq!(rates.len(), 1);
                assert_eq!(rates[0].per_unit_price, dec!(0.05));
                assert_eq!(rates[0].dimension1.key, "model");
                assert_eq!(rates[0].dimension1.value, "gpt-4");
                let d2 = rates[0].dimension2.as_ref().unwrap();
                assert_eq!(d2.key, "region");
                assert_eq!(d2.value, "us");
            }
            _ => panic!("Expected Matrix"),
        }
    }

    #[test]
    fn test_resolve_matrix_usage() {
        let mid = BillableMetricId::default();
        let structure = FeeStructure::Usage {
            metric_id: mid,
            model: UsageModel::Matrix,
        };
        let pricing = Pricing::Usage(UsagePricingModel::Matrix {
            rates: vec![MatrixRow {
                dimension1: MatrixDimension {
                    key: "model".to_string(),
                    value: "gpt-4".to_string(),
                },
                dimension2: None,
                per_unit_price: dec!(0.03),
            }],
        });
        let fee = resolve_subscription_fee(&structure, &pricing, None, &no_units(), None).unwrap();
        match fee {
            SubscriptionFee::Usage {
                metric_id,
                model: UsagePricingModel::Matrix { rates },
                ..
            } => {
                assert_eq!(metric_id, mid);
                assert_eq!(rates.len(), 1);
                assert_eq!(rates[0].per_unit_price, dec!(0.03));
            }
            _ => panic!("Expected Usage/Matrix"),
        }
    }

    #[test]
    fn test_serde_backward_compat_old_matrix_with_fields() {
        // Old JSONB with dimension_keys/dimension_values should still deserialize
        let old_json = serde_json::json!({
            "type": "Matrix",
            "dimension_keys": ["model"],
            "dimension_values": {"model": ["gpt-4", "gpt-3.5"]}
        });
        let model: UsageModel = serde_json::from_value(old_json).unwrap();
        assert!(matches!(model, UsageModel::Matrix));
    }

    #[test]
    fn test_serde_backward_compat_old_slot_with_fields() {
        // Old JSONB with min_slots/max_slots should still deserialize
        let old_json = serde_json::json!({
            "type": "Slot",
            "unit_name": "seat",
            "min_slots": 1,
            "max_slots": 100,
            "upgrade_policy": "Prorated",
            "downgrade_policy": "RemoveAtEndOfPeriod"
        });
        let fs: FeeStructure = serde_json::from_value(old_json).unwrap();
        match fs {
            FeeStructure::Slot { unit_name, .. } => assert_eq!(unit_name, "seat"),
            _ => panic!("Expected Slot"),
        }
    }

    fn included_pack(expiry: Option<CreditExpiry>, rollover: CreditRollover) -> Pricing {
        Pricing::Credit {
            unit_price: dec!(0),
            quantity: 1,
            credit_amount: dec!(500),
            expiry,
            credit_unit_id: None,
            rollover: Some(rollover),
            once: false,
        }
    }

    /// $500 for $550 sells $50 of bonus credit; $600 for $550 would mint paid credit above face.
    #[test]
    fn a_money_pack_never_costs_more_than_its_credit() {
        let pack = |unit_price, credit_unit_id| Pricing::Credit {
            unit_price,
            quantity: 1,
            credit_amount: dec!(550),
            expiry: Some(CreditExpiry::Month(12)),
            credit_unit_id,
            rollover: None,
            once: false,
        };
        assert!(validate_pricing(&pack(dec!(500), None), "USD").is_ok());
        assert!(validate_pricing(&pack(dec!(550), None), "USD").is_ok());
        assert!(validate_pricing(&pack(dec!(600), None), "USD").is_err());
        let unit = Some(common_domain::ids::CreditUnitId::new());
        assert!(
            validate_pricing(&pack(dec!(600), unit), "USD").is_ok(),
            "credit units are not money"
        );
    }

    #[test]
    fn only_expiring_included_credit_rolls_over_for_a_term_under_a_positive_cap() {
        let rollover = CreditRollover {
            max_amount: Some(dec!(200)),
            expiry: CreditExpiry::Month(1),
        };
        let monthly = Some(CreditExpiry::BillingPeriod);
        assert!(validate_pricing(&included_pack(monthly, rollover), "EUR").is_ok());
        let uncapped = CreditRollover {
            max_amount: None,
            ..rollover
        };
        assert!(validate_pricing(&included_pack(monthly, uncapped), "EUR").is_ok());

        let paid = Pricing::Credit {
            unit_price: dec!(10),
            quantity: 1,
            credit_amount: dec!(500),
            expiry: monthly,
            credit_unit_id: None,
            rollover: Some(rollover),
            once: false,
        };
        assert!(
            validate_pricing(&paid, "EUR").is_err(),
            "paid packs do not roll over"
        );
        assert!(
            validate_pricing(&included_pack(None, rollover), "EUR").is_err(),
            "credit that never expires has nothing to roll"
        );
        let once = Pricing::Credit {
            unit_price: dec!(0),
            quantity: 1,
            credit_amount: dec!(500),
            expiry: monthly,
            credit_unit_id: None,
            rollover: Some(rollover),
            once: true,
        };
        assert!(
            validate_pricing(&once, "EUR").is_err(),
            "credit granted once does not roll over"
        );
        for refused in [
            CreditRollover {
                expiry: CreditExpiry::BillingPeriod,
                ..rollover
            },
            CreditRollover {
                expiry: CreditExpiry::Day(0),
                ..rollover
            },
            CreditRollover {
                expiry: CreditExpiry::Month(121),
                ..rollover
            },
            CreditRollover {
                max_amount: Some(dec!(0)),
                ..rollover
            },
            CreditRollover {
                max_amount: Some(dec!(0.001)),
                ..rollover
            },
        ] {
            assert!(
                validate_pricing(&included_pack(monthly, refused), "EUR").is_err(),
                "{refused:?}"
            );
        }
    }
}
