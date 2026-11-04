use error_stack::Report;
use std::collections::HashMap;
use uuid::Uuid;
// TODO duplicate as well
use super::enums::{BillingPeriodEnum, BillingType, FeeTypeEnum, SubscriptionFeeBillingPeriod};

use crate::domain::credit_units::CreditUnitsById;
use crate::domain::prices::{self, FeeStructure, LegacyPricingData, Pricing};
use crate::domain::{Price, Product, SubscriptionFee};
use crate::errors::{StoreError, StoreErrorReport};
use crate::json_value_serde;
use common_domain::ids::{
    BaseId, BillableMetricId, PlanVersionId, PriceComponentId, PriceId, ProductId,
};
use diesel_models::price_components::{PriceComponentRow, PriceComponentRowNew};
use golden::golden;
use serde::{Deserialize, Serialize};

// ── Write-side types ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProductRef {
    Existing(ProductId),
    New {
        name: String,
        fee_type: FeeTypeEnum,
        fee_structure: FeeStructure,
    },
}

impl ProductRef {
    pub fn existing_product_id(&self) -> Option<ProductId> {
        match self {
            ProductRef::Existing(id) => Some(*id),
            ProductRef::New { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PriceEntry {
    Existing(PriceId),
    New(PriceInput),
}

impl PriceEntry {
    pub fn existing_price_id(&self) -> Option<PriceId> {
        match self {
            PriceEntry::Existing(id) => Some(*id),
            PriceEntry::New(_) => None,
        }
    }

    /// The credit units of a new price; an existing price's come with it.
    pub fn new_credit_unit_ids(&self) -> Vec<common_domain::ids::CreditUnitId> {
        match self {
            PriceEntry::Existing(_) => Vec::new(),
            PriceEntry::New(input) => {
                prices::credit_unit_ids(&input.pricing, input.credit_unit.as_ref()).collect()
            }
        }
    }
}

/// Per-cadence pricing input for creating a price with associated prices.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceInput {
    pub cadence: BillingPeriodEnum,
    pub currency: String,
    pub pricing: Pricing,
    /// The credit unit `pricing`'s usage amounts are expressed in, with its money rate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_unit: Option<crate::domain::credit_units::PriceCreditUnit>,
}

/// High-level component definition used by `insert_plan` and the seeder.
#[derive(Debug, Clone)]
pub struct PriceComponentNewInternal {
    pub name: String,
    pub product_ref: ProductRef,
    pub prices: Vec<PriceEntry>,
}

/// Low-level row builder for inserting a PriceComponentRow.
#[derive(Debug, Clone)]
pub struct PriceComponentNew {
    pub name: String,
    pub product_id: Option<ProductId>,
    pub plan_version_id: PlanVersionId,
}

impl TryInto<PriceComponentRowNew> for PriceComponentNew {
    type Error = StoreErrorReport;

    fn try_into(self) -> Result<PriceComponentRowNew, Self::Error> {
        Ok(PriceComponentRowNew {
            id: PriceComponentId::new(),
            plan_version_id: self.plan_version_id,
            name: self.name,
            legacy_fee: None,
            product_id: self.product_id,
            billable_metric_id: None,
        })
    }
}

// ── Read-side types ─────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PriceComponent {
    pub id: PriceComponentId,
    pub name: String,
    pub product_id: Option<ProductId>,
    pub prices: Vec<Price>,
    /// V1 pricing data extracted from Row legacy_fee. No database IDs.
    /// None for v2 components.
    pub legacy_pricing: Option<LegacyPricingData>,
}

impl TryInto<PriceComponent> for PriceComponentRow {
    type Error = Report<StoreError>;

    fn try_into(self) -> Result<PriceComponent, Self::Error> {
        Ok(PriceComponent {
            id: self.id,
            name: self.name,
            product_id: self.product_id,
            prices: Vec::new(),
            legacy_pricing: None,
        })
    }
}

/// Result of resolving a price component's fee (v1 or v2 path).
#[derive(Debug, Clone)]
pub struct ResolvedFee {
    pub period: SubscriptionFeeBillingPeriod,
    pub fee: SubscriptionFee,
    pub price_id: Option<PriceId>,
}

impl PriceComponent {
    /// Whether the component sells credit for a price, as opposed to including it at no charge.
    pub fn sells_credit(&self) -> bool {
        let paid = |pricing: &Pricing| matches!(pricing, Pricing::Credit { unit_price, .. } if *unit_price > rust_decimal::Decimal::ZERO);
        self.prices.iter().any(|price| paid(&price.pricing))
            || self.legacy_pricing.as_ref().is_some_and(|legacy| {
                legacy
                    .pricing_entries
                    .iter()
                    .any(|(_, pricing)| paid(pricing))
            })
    }

    pub fn credit_unit_ids(&self) -> impl Iterator<Item = common_domain::ids::CreditUnitId> + '_ {
        self.prices.iter().flat_map(Price::credit_unit_ids)
    }

    /// Centralized resolution — all components must have product + prices (v2).
    /// v1 components get synthesized prices at the repository/context layer.
    pub fn resolve_subscription_fee(
        &self,
        products: &HashMap<ProductId, Product>,
        units: &CreditUnitsById,
        params: Option<&ComponentParameters>,
    ) -> Result<ResolvedFee, StoreError> {
        let product_id = self.product_id.ok_or_else(|| {
            StoreError::InvalidArgument(format!("PriceComponent {} has no product_id", self.id))
        })?;

        let product = products.get(&product_id).ok_or_else(|| {
            StoreError::InvalidArgument(format!(
                "Product {} not found for component {}",
                product_id, self.id
            ))
        })?;

        let fee_structure = &product.fee_structure;

        if self.prices.is_empty() {
            return Err(StoreError::InvalidArgument(format!(
                "PriceComponent {} has no prices",
                self.id
            )));
        }

        let price = self.select_price(params)?;
        let fee = prices::resolve_subscription_fee(
            fee_structure,
            &price.pricing,
            price.credit_unit.as_ref(),
            units,
            params,
        )?;
        let period = prices::fee_type_billing_period(fee_structure)
            .unwrap_or_else(|| price.cadence.as_subscription_billing_period());

        Ok(ResolvedFee {
            period,
            fee,
            price_id: Some(price.id),
        })
    }

    /// Resolve the fee for a component, handling both v2 (product + prices) and v1 (legacy) paths.
    pub fn resolve_fee(
        &self,
        products: &HashMap<ProductId, Product>,
        units: &CreditUnitsById,
        params: Option<&ComponentParameters>,
    ) -> Result<ResolvedFee, StoreError> {
        if self.product_id.is_some() && !self.prices.is_empty() {
            self.resolve_subscription_fee(products, units, params)
        } else if let Some(legacy) = &self.legacy_pricing {
            resolve_legacy_subscription_fee(legacy, params)
        } else {
            Err(StoreError::InvalidArgument(format!(
                "Component {} has no pricing data",
                self.name
            )))
        }
    }

    fn select_price(&self, params: Option<&ComponentParameters>) -> Result<&Price, StoreError> {
        if self.prices.len() == 1 {
            return Ok(&self.prices[0]);
        }

        let mut candidates: Vec<&Price> = self.prices.iter().collect();

        if let Some(p) = params {
            // Filter by billing period if specified
            if let Some(bp) = &p.billing_period {
                let target = bp.as_subscription_billing_period();
                candidates.retain(|pr| pr.cadence.as_subscription_billing_period() == target);
            }

            // Filter by committed capacity (matches Pricing::Capacity { included })
            if let Some(cap) = p.committed_capacity {
                candidates.retain(|pr| match &pr.pricing {
                    super::prices::Pricing::Capacity { included, .. } => *included == cap,
                    _ => true, // non-capacity prices are unaffected
                });
            }
        }

        match candidates.len() {
            1 => Ok(candidates[0]),
            0 => Err(StoreError::InvalidArgument(format!(
                "No matching price found for component {}",
                self.id
            ))),
            _ => Err(StoreError::InvalidArgument(format!(
                "Multiple prices match for component {} — provide billing_period or committed_capacity to disambiguate",
                self.id
            ))),
        }
    }
}

pub use super::subscription_components::ComponentParameters;

pub fn resolve_legacy_subscription_fee(
    legacy: &LegacyPricingData,
    params: Option<&ComponentParameters>,
) -> Result<ResolvedFee, StoreError> {
    let (cadence, pricing) = select_legacy_pricing_entry(&legacy.pricing_entries, params)?;
    // Legacy components are never priced in, nor grant, a credit unit.
    let fee = prices::resolve_subscription_fee(
        &legacy.fee_structure,
        pricing,
        None,
        &CreditUnitsById::new(),
        params,
    )?;
    let period = prices::fee_type_billing_period(&legacy.fee_structure)
        .unwrap_or_else(|| cadence.as_subscription_billing_period());
    Ok(ResolvedFee {
        period,
        fee,
        price_id: None,
    })
}

fn select_legacy_pricing_entry<'a>(
    entries: &'a [(BillingPeriodEnum, Pricing)],
    params: Option<&ComponentParameters>,
) -> Result<&'a (BillingPeriodEnum, Pricing), StoreError> {
    if entries.len() == 1 {
        return Ok(&entries[0]);
    }
    if let Some(p) = params
        && let Some(bp) = &p.billing_period
    {
        return entries.iter().find(|(c, _)| c == bp).ok_or_else(|| {
            StoreError::InvalidArgument(format!("No pricing entry for billing period {:?}", bp))
        });
    }
    Err(StoreError::InvalidArgument(
        "Multiple pricing entries but no billing_period specified".into(),
    ))
}

//
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum UsagePricingModel {
    PerUnit {
        rate: rust_decimal::Decimal,
    },
    Tiered {
        tiers: Vec<TierRow>,
        block_size: Option<u64>,
    },
    Volume {
        tiers: Vec<TierRow>,
        block_size: Option<u64>,
    },
    Package {
        block_size: u64,
        rate: rust_decimal::Decimal,
    },
    Matrix {
        rates: Vec<MatrixRow>,
    },
}

impl UsagePricingModel {
    /// Decimal places to round this model's billable quantity to, at currency `precision`.
    ///
    /// Linear models (per-unit, graduated tiers) use the price-derived `quantity_scale` of
    /// their largest rate; step models (package, volume) only strip float noise, since a
    /// coarser scale would round genuine fractional usage across a `ceil`/tier boundary and
    /// drop a whole block of revenue. Matrix is priced per cell at the call site.
    pub fn billable_quantity_scale(&self, precision: u8) -> u32 {
        use common_utils::decimals::{MAX_QUANTITY_SCALE, quantity_scale};
        match self {
            UsagePricingModel::PerUnit { rate } => quantity_scale(*rate, precision),
            UsagePricingModel::Tiered { tiers, .. } => tiers
                .iter()
                .map(|t| t.rate)
                .max()
                .map_or(MAX_QUANTITY_SCALE, |rate| quantity_scale(rate, precision)),
            UsagePricingModel::Package { .. }
            | UsagePricingModel::Volume { .. }
            | UsagePricingModel::Matrix { .. } => MAX_QUANTITY_SCALE,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MatrixRow {
    pub dimension1: MatrixDimension,
    pub dimension2: Option<MatrixDimension>,
    pub per_unit_price: rust_decimal::Decimal,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MatrixDimension {
    pub key: String,
    pub value: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TierRow {
    pub first_unit: u64,
    // last unit is implicit.
    pub rate: rust_decimal::Decimal,
    pub flat_fee: Option<rust_decimal::Decimal>,
    pub flat_cap: Option<rust_decimal::Decimal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FeeType {
    Rate {
        rates: Vec<TermRate>,
    },
    Slot {
        rates: Vec<TermRate>,
        slot_unit_name: String,
        upgrade_policy: UpgradePolicy,
        downgrade_policy: DowngradePolicy,
        minimum_count: Option<u32>,
        quota: Option<u32>,
    },
    Capacity {
        metric_id: BillableMetricId,
        thresholds: Vec<CapacityThreshold>,
        #[serde(default)]
        cadence: BillingPeriodEnum,
    },
    Usage {
        metric_id: BillableMetricId,
        pricing: UsagePricingModel,
        #[serde(default)]
        cadence: BillingPeriodEnum,
    },
    ExtraRecurring {
        unit_price: rust_decimal::Decimal,
        quantity: u32,
        billing_type: BillingType,
        cadence: BillingPeriodEnum,
    },
    OneTime {
        unit_price: rust_decimal::Decimal,
        quantity: u32,
    },
    /// A credit pack: charges `unit_price` × `quantity` in advance each period and, once paid,
    /// funds `credit_amount` × `quantity` of invoice credit in the plan currency. A zero price
    /// makes it credit the plan includes, funded when the period is invoiced.
    Credit {
        unit_price: rust_decimal::Decimal,
        quantity: u32,
        credit_amount: rust_decimal::Decimal,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expiry: Option<CreditExpiry>,
        /// Billed on the subscription's first invoice only, never again.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        once: bool,
    },
}

json_value_serde!(FeeType);

/// Relative expiry of credit a pack or purchase funds. A plan is a template, so every purchase
/// expires the span after its own payment rather than at one shared instant.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "unit", content = "value", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CreditExpiry {
    Day(u32),
    Month(u32),
    /// The exclusive end of the invoiced component period, not a duration after payment.
    BillingPeriod,
}

impl CreditExpiry {
    /// Relative expiry from `at`; `None` on overflow or when an invoice period is required.
    pub fn resolve(
        &self,
        at: chrono::DateTime<chrono::Utc>,
    ) -> Option<chrono::DateTime<chrono::Utc>> {
        match self {
            CreditExpiry::Day(days) => {
                at.checked_add_signed(chrono::Duration::try_days(i64::from(*days))?)
            }
            CreditExpiry::Month(months) => at.checked_add_months(chrono::Months::new(*months)),
            CreditExpiry::BillingPeriod => None,
        }
    }

    pub fn resolve_for_period(
        &self,
        at: chrono::DateTime<chrono::Utc>,
        period_end: chrono::NaiveDate,
    ) -> Option<chrono::DateTime<chrono::Utc>> {
        match self {
            Self::BillingPeriod => Some(period_end.and_time(chrono::NaiveTime::MIN).and_utc()),
            _ => self.resolve(at),
        }
    }
}

/// Plan-included credit left unspent at expiry, carried once into a new grant that burns before
/// the next period's credit.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreditRollover {
    /// Cap per rolled grant, in the pack's denomination (major units); None carries all that is left.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_amount: Option<rust_decimal::Decimal>,
    /// Term of the rolled credit from the original expiry; `Day` or `Month` only.
    pub expiry: CreditExpiry,
}

impl FeeType {
    pub fn metric_id(&self) -> Option<BillableMetricId> {
        match self {
            FeeType::Capacity { metric_id, .. } => Some(*metric_id),
            FeeType::Usage { metric_id, .. } => Some(*metric_id),
            _ => None,
        }
    }

    /// Revenue taxonomy, matching `SubscriptionFee::charge_type()`. Used to recover
    /// the charge type of v1 plan components for legacy invoice lines.
    pub fn charge_type(&self) -> super::invoice_lines::ChargeType {
        use super::invoice_lines::ChargeType;
        match self {
            FeeType::Capacity { .. } | FeeType::Usage { .. } => ChargeType::Usage,
            FeeType::OneTime { .. } | FeeType::Credit { .. } => ChargeType::OneTime,
            FeeType::Rate { .. } | FeeType::Slot { .. } | FeeType::ExtraRecurring { .. } => {
                ChargeType::Recurring
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TermRate {
    pub term: BillingPeriodEnum,
    pub price: rust_decimal::Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapacityThreshold {
    pub included_amount: u64,
    pub price: rust_decimal::Decimal,
    pub per_unit_overage: rust_decimal::Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UpgradePolicy {
    Prorated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DowngradePolicy {
    RemoveAtEndOfPeriod,
}

golden!(FeeType, {
    "rate" => FeeType::Rate {
        rates: vec![TermRate {
            term: BillingPeriodEnum::Monthly,
            price: rust_decimal::Decimal::new(100, 2),
        }],
    },
    "slot" => FeeType::Slot {
        rates: vec![TermRate {
            term: BillingPeriodEnum::Monthly,
            price: rust_decimal::Decimal::new(100, 2),
        }],
        slot_unit_name: "slot".to_string(),
        upgrade_policy: UpgradePolicy::Prorated,
        downgrade_policy: DowngradePolicy::RemoveAtEndOfPeriod,
        minimum_count: Some(1),
        quota: Some(10),
    },
    "capacity" => FeeType::Capacity {
        metric_id: Uuid::nil().into(),
        thresholds: vec![CapacityThreshold {
            included_amount: 100,
            price: rust_decimal::Decimal::new(100, 2),
            per_unit_overage: rust_decimal::Decimal::new(10, 2),
        }],
        cadence: BillingPeriodEnum::Monthly,
    },
    "usage_per_unit" => FeeType::Usage {
        metric_id: Uuid::nil().into(),
        pricing: UsagePricingModel::PerUnit {
            rate: rust_decimal::Decimal::new(100, 2),
        },
        cadence: BillingPeriodEnum::Monthly,
    },
    "usage_tiered" => FeeType::Usage {
        metric_id: Uuid::nil().into(),
        pricing: UsagePricingModel::Tiered {
            tiers: vec![TierRow {
                first_unit: 1,
                rate: rust_decimal::Decimal::new(100, 2),
                flat_fee: None,
                flat_cap: None,
            }],
            block_size: Some(10),
        },
        cadence: BillingPeriodEnum::Monthly,
    },
    "usage_volume" => FeeType::Usage {
        metric_id: Uuid::nil().into(),
        pricing: UsagePricingModel::Volume {
            tiers: vec![TierRow {
                first_unit: 1,
                rate: rust_decimal::Decimal::new(100, 2),
                flat_fee: None,
                flat_cap: None,
            }],
            block_size: Some(10),
        },
        cadence: BillingPeriodEnum::Monthly,
    },
    "usage_package" => FeeType::Usage {
        metric_id: Uuid::nil().into(),
        pricing: UsagePricingModel::Package {
            block_size: 10,
            rate: rust_decimal::Decimal::new(100, 2),
        },
        cadence: BillingPeriodEnum::Monthly,
    },
    "usage_matrix" => FeeType::Usage {
        metric_id: Uuid::nil().into(),
        pricing: UsagePricingModel::Matrix {
            rates: vec![MatrixRow {
                dimension1: MatrixDimension {
                    key: "key".to_string(),
                    value: "value".to_string(),
                },
                dimension2: None,
                per_unit_price: rust_decimal::Decimal::new(100, 2),
            }],
        },
        cadence: BillingPeriodEnum::Monthly,
    },
    "extra_recurring" => FeeType::ExtraRecurring {
        unit_price: rust_decimal::Decimal::new(100, 2),
        quantity: 1,
        billing_type: BillingType::Advance,
        cadence: BillingPeriodEnum::Monthly,
    },
    "one_time" => FeeType::OneTime {
        unit_price: rust_decimal::Decimal::new(100, 2),
        quantity: 1,
    },

});

#[cfg(test)]
mod billable_quantity_scale_tests {
    use super::*;
    use common_utils::decimals::{MAX_QUANTITY_SCALE, quantity_scale};
    use rust_decimal_macros::dec;

    fn tier(first_unit: u64, rate: rust_decimal::Decimal) -> TierRow {
        TierRow {
            first_unit,
            rate,
            flat_fee: None,
            flat_cap: None,
        }
    }

    #[test]
    fn per_unit_uses_the_price_derived_scale() {
        assert_eq!(
            UsagePricingModel::PerUnit { rate: dec!(0.02) }.billable_quantity_scale(2),
            1
        );
    }

    #[test]
    fn tiered_uses_the_largest_tier_rate() {
        let tiers = vec![tier(0, dec!(10)), tier(100, dec!(0.01))];
        assert_eq!(
            UsagePricingModel::Tiered {
                tiers,
                block_size: None,
            }
            .billable_quantity_scale(2),
            quantity_scale(dec!(10), 2)
        );
    }

    #[test]
    fn step_models_only_strip_noise() {
        assert_eq!(
            UsagePricingModel::Package {
                block_size: 1000,
                rate: dec!(5),
            }
            .billable_quantity_scale(2),
            MAX_QUANTITY_SCALE
        );
        assert_eq!(
            UsagePricingModel::Volume {
                tiers: vec![tier(0, dec!(10)), tier(100, dec!(1))],
                block_size: None,
            }
            .billable_quantity_scale(2),
            MAX_QUANTITY_SCALE
        );
    }
}
