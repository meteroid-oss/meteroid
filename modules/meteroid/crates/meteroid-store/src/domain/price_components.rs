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
    BaseId, BillableMetricId, PlanOptionId, PlanVersionId, PriceComponentId, PriceId, ProductId,
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
    /// A new product of the credit fee structure.
    pub fn creates_credit_product(&self) -> bool {
        matches!(self, ProductRef::New { fee_structure, .. } if fee_structure.is_credit())
    }

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
    /// Every price linked in any option of the version, once each.
    pub prices: Vec<Price>,
    /// Which option's cell each price is in (`price_cells`); a price in several options is shared.
    pub option_prices: Vec<OptionPrice>,
    /// V1 pricing data extracted from Row legacy_fee. No database IDs.
    /// None for v2 components.
    pub legacy_pricing: Option<LegacyPricingData>,
}

/// A price in the cell of one option.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OptionPrice {
    pub option_id: PlanOptionId,
    pub price_id: PriceId,
}

impl TryInto<PriceComponent> for PriceComponentRow {
    type Error = Report<StoreError>;

    fn try_into(self) -> Result<PriceComponent, Self::Error> {
        Ok(PriceComponent {
            id: self.id,
            name: self.name,
            product_id: self.product_id,
            prices: Vec::new(),
            option_prices: Vec::new(),
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
    /// A product the plan includes with no price: it brings its features and bills nothing, so
    /// a subscription holds no fee for it.
    /// Keeps its prices in `currency`; refused when it has prices and none in it, so a component
    /// is never taken for an included product only for lack of a price in the currency.
    pub fn keep_currency(&mut self, currency: &str) -> Result<(), StoreError> {
        if self.prices.is_empty() {
            return Ok(());
        }
        self.prices
            .retain(|price| price.currency.eq_ignore_ascii_case(currency));
        let prices = &self.prices;
        self.option_prices
            .retain(|link| prices.iter().any(|price| price.id == link.price_id));
        if self.prices.is_empty() {
            return Err(StoreError::InvalidArgument(format!(
                "{} has no price in {currency}",
                self.name
            )));
        }
        Ok(())
    }

    /// Attaches the links of the component among `links`, and their prices once each.
    pub fn attach_links(
        &mut self,
        links: &[diesel_models::plan_component_prices::PlanComponentPriceRow],
        prices_by_id: &HashMap<PriceId, Price>,
    ) {
        for link in links
            .iter()
            .filter(|link| link.plan_component_id == self.id)
        {
            let Some(price) = prices_by_id.get(&link.price_id) else {
                continue;
            };
            if !self.prices.iter().any(|p| p.id == price.id) {
                self.prices.push(price.clone());
            }
            self.option_prices.push(OptionPrice {
                option_id: link.option_id,
                price_id: link.price_id,
            });
        }
    }

    pub fn is_included_only(&self) -> bool {
        self.product_id.is_some() && self.prices.is_empty() && self.legacy_pricing.is_none()
    }

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
    /// Rated by its dimensions at rates the price holds; reads a matrix stored before them.
    #[serde(alias = "Matrix")]
    DimensionRates(DimensionRates),
    /// Rated by a rate card's entries in force each day, each rate scaled by `multiplier`.
    RateCard {
        rate_card_id: common_domain::ids::RateCardId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        multiplier: Option<rust_decimal::Decimal>,
    },
}

impl UsagePricingModel {
    /// Whether some usage costs anything. A rate card's rates live on the card, so it is taken
    /// to charge unless scaled to zero.
    pub fn charges(&self) -> bool {
        let positive = |rate: &rust_decimal::Decimal| *rate > rust_decimal::Decimal::ZERO;
        match self {
            UsagePricingModel::PerUnit { rate } | UsagePricingModel::Package { rate, .. } => {
                positive(rate)
            }
            UsagePricingModel::Tiered { tiers, .. } | UsagePricingModel::Volume { tiers, .. } => {
                tiers.iter().any(|tier| {
                    positive(&tier.rate) || tier.flat_fee.as_ref().is_some_and(positive)
                })
            }
            UsagePricingModel::DimensionRates(rates) => {
                rates.rates.iter().any(|rate| positive(&rate.rate))
            }
            UsagePricingModel::RateCard { multiplier, .. } => {
                multiplier.is_none_or(|multiplier| !multiplier.is_zero())
            }
        }
    }

    /// Decimal places to round this model's billable quantity to, at currency `precision`.
    ///
    /// Linear models (per-unit, graduated tiers) use the price-derived `quantity_scale` of
    /// their largest rate; step models (package, volume) only strip float noise, since a
    /// coarser scale would round genuine fractional usage across a `ceil`/tier boundary and
    /// drop a whole block of revenue. Rates by dimension are priced per key.
    pub fn billable_quantity_scale(&self, precision: u8) -> u32 {
        billing_core::pricing::UsageModel::from(self).billable_quantity_scale(precision)
    }
}

/// Usage rated by its dimensions at rates a price holds: each rate's key is matched and the most
/// specific one wins, as a rate card's entries are (R-131), but the rates are not dated: other
/// rates are another price (R-136).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(from = "StoredDimensionRates")]
pub struct DimensionRates {
    /// The dimensions keys name, in the order that decides between keys equally specific on an
    /// earlier one.
    pub dimensions: Vec<String>,
    pub rates: Vec<DimensionRate>,
    /// Rates are per this many units of the metric (1000000: per 1M tokens).
    pub unit_size: rust_decimal::Decimal,
    /// Usage no key matches holds the invoice, or is left unbilled.
    pub on_unmatched: super::rate_cards::RateCardUnmatched,
}

/// The rate of the usage `key` matches.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DimensionRate {
    /// A pattern per dimension: a value, a prefix ending in `*`, or `*`.
    pub key: Vec<String>,
    pub rate: rust_decimal::Decimal,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl DimensionRates {
    /// The rates as billing reads a rate card's entries: in force from always, unscaled.
    pub fn card_rates(&self) -> billing_core::pricing::CardRates {
        use billing_core::rate_card::{Entry, Pattern};
        billing_core::pricing::CardRates {
            dimensions: self.dimensions.clone(),
            entries: self
                .rates
                .iter()
                .map(|rate| Entry {
                    key: rate.key.iter().map(|p| Pattern::parse(p)).collect(),
                    effective_from: chrono::NaiveDate::MIN,
                    rate: Some(rate.rate),
                })
                .collect(),
            unit_size: self.unit_size,
            multiplier: rust_decimal::Decimal::ONE,
            ignore_unrated: self.on_unmatched == super::rate_cards::RateCardUnmatched::Ignore,
            on: chrono::NaiveDate::MIN,
        }
    }

    /// Refuses rates billing could not resolve: no dimension, a key of another length, two
    /// rates of one key, a negative rate or unit size.
    pub fn validate(&self) -> Result<(), StoreError> {
        let invalid = |message: &str| Err(StoreError::InvalidArgument(message.to_owned()));
        let named = |dimension: &String| !dimension.trim().is_empty();
        if self.dimensions.is_empty()
            || self.dimensions.len() > 4
            || !self.dimensions.iter().all(named)
        {
            return invalid("rates by dimension name one to four dimensions");
        }
        let mut seen = std::collections::HashSet::new();
        if !self
            .dimensions
            .iter()
            .all(|dimension| seen.insert(dimension))
        {
            return invalid("a dimension is named once");
        }
        if self.unit_size <= rust_decimal::Decimal::ZERO {
            return invalid("rates are per a positive number of units");
        }
        let mut keys = std::collections::HashSet::new();
        for rate in &self.rates {
            if rate.key.len() != self.dimensions.len() {
                return invalid("each rate's key has a value per dimension");
            }
            if rate.key.iter().any(|pattern| pattern.trim().is_empty()) {
                return invalid("a key's value is a value, a prefix ending in `*`, or `*`");
            }
            if rate.rate < rust_decimal::Decimal::ZERO {
                return invalid("a rate is not negative");
            }
            if !keys.insert(&rate.key) {
                return invalid("two rates have the same key");
            }
        }
        Ok(())
    }

    /// [`Self::from_matrix`] for cells given as input: each a value of one or two dimensions
    /// and its per-unit price.
    pub fn from_matrix_cells(
        cells: impl IntoIterator<
            Item = (
                MatrixDimension,
                Option<MatrixDimension>,
                rust_decimal::Decimal,
            ),
        >,
    ) -> DimensionRates {
        DimensionRates::from_matrix(
            cells
                .into_iter()
                .map(|(dimension1, dimension2, per_unit_price)| StoredMatrixRow {
                    dimension1,
                    dimension2,
                    per_unit_price,
                })
                .collect(),
        )
    }

    /// The matrix cells these rates are, the inverse of [`Self::from_matrix_cells`]: per unit, one
    /// or two dimensions, each key of exact values, `*` only second for a cell of one dimension.
    /// None for rates a matrix cannot hold. Labels and `on_unmatched` are not a matrix's.
    pub fn as_matrix_cells(
        &self,
    ) -> Option<
        Vec<(
            MatrixDimension,
            Option<MatrixDimension>,
            rust_decimal::Decimal,
        )>,
    > {
        // No dimension: a matrix stored without cells. Dimensions without rates: no matrix.
        let shaped = match self.dimensions.len() {
            0 => self.rates.is_empty(),
            1 | 2 => !self.rates.is_empty(),
            _ => false,
        };
        if !shaped || self.unit_size != rust_decimal::Decimal::ONE {
            return None;
        }
        let exact = |pattern: &String| !pattern.ends_with('*');
        let cell = |index: usize, value: &String| MatrixDimension {
            key: self.dimensions[index].clone(),
            value: value.clone(),
        };
        self.rates
            .iter()
            .map(|rate| {
                if rate.key.len() != self.dimensions.len() {
                    return None;
                }
                match rate.key.as_slice() {
                    [first] if exact(first) => Some((cell(0, first), None, rate.rate)),
                    [first, second] if exact(first) && second == "*" => {
                        Some((cell(0, first), None, rate.rate))
                    }
                    [first, second] if exact(first) && exact(second) => {
                        Some((cell(0, first), Some(cell(1, second)), rate.rate))
                    }
                    _ => None,
                }
            })
            .collect()
    }

    /// Rates of a matrix stored before rates by dimension: a key per cell, `*` for a dimension
    /// it leaves out, and usage no cell matches left unbilled as the matrix left it.
    fn from_matrix(rows: Vec<StoredMatrixRow>) -> DimensionRates {
        let cells = |row: &StoredMatrixRow| {
            std::iter::once(&row.dimension1)
                .chain(row.dimension2.as_ref())
                .cloned()
                .collect::<Vec<MatrixDimension>>()
        };
        let mut dimensions: Vec<String> = Vec::new();
        for row in &rows {
            for cell in cells(row) {
                if !dimensions.contains(&cell.key) {
                    dimensions.push(cell.key);
                }
            }
        }
        let rates = rows
            .iter()
            .map(|row| {
                let cells = cells(row);
                DimensionRate {
                    key: dimensions
                        .iter()
                        .map(|dimension| {
                            cells
                                .iter()
                                .find(|cell| &cell.key == dimension)
                                .map_or_else(|| "*".to_owned(), |cell| cell.value.clone())
                        })
                        .collect(),
                    rate: row.per_unit_price,
                    label: None,
                }
            })
            .collect();
        DimensionRates {
            dimensions,
            rates,
            unit_size: rust_decimal::Decimal::ONE,
            on_unmatched: super::rate_cards::RateCardUnmatched::Ignore,
        }
    }
}

/// Rates by dimension as stored: today's, or a matrix's cells.
#[derive(Deserialize)]
#[serde(untagged)]
enum StoredDimensionRates {
    Rates {
        dimensions: Vec<String>,
        rates: Vec<DimensionRate>,
        #[serde(default = "one")]
        unit_size: rust_decimal::Decimal,
        #[serde(default)]
        on_unmatched: super::rate_cards::RateCardUnmatched,
    },
    Matrix {
        rates: Vec<StoredMatrixRow>,
    },
}

fn one() -> rust_decimal::Decimal {
    rust_decimal::Decimal::ONE
}

impl From<StoredDimensionRates> for DimensionRates {
    fn from(stored: StoredDimensionRates) -> Self {
        match stored {
            StoredDimensionRates::Rates {
                dimensions,
                rates,
                unit_size,
                on_unmatched,
            } => DimensionRates {
                dimensions,
                rates,
                unit_size,
                on_unmatched,
            },
            StoredDimensionRates::Matrix { rates } => DimensionRates::from_matrix(rates),
        }
    }
}

/// A cell of a matrix price: the usage carrying `dimension1` (and `dimension2`).
#[derive(Deserialize)]
struct StoredMatrixRow {
    dimension1: MatrixDimension,
    dimension2: Option<MatrixDimension>,
    per_unit_price: rust_decimal::Decimal,
}

/// How a quantity that is not metered (seats, a count set on the plan) becomes an amount.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum RatingModel {
    PerUnit {
        rate: rust_decimal::Decimal,
    },
    Tiered {
        tiers: Vec<TierRow>,
    },
    Volume {
        tiers: Vec<TierRow>,
    },
    Package {
        block_size: u64,
        rate: rust_decimal::Decimal,
    },
}

impl RatingModel {
    /// The price of each unit, when every unit costs the same.
    pub fn per_unit_rate(&self) -> Option<rust_decimal::Decimal> {
        match self {
            RatingModel::PerUnit { rate } => Some(*rate),
            _ => None,
        }
    }

    pub fn usage_model(&self) -> billing_core::pricing::UsageModel {
        let tiers = |tiers: &[TierRow]| tiers.iter().map(Into::into).collect();
        match self {
            RatingModel::PerUnit { rate } => {
                billing_core::pricing::UsageModel::PerUnit { rate: *rate }
            }
            RatingModel::Tiered { tiers: rows } => billing_core::pricing::UsageModel::Tiered {
                tiers: tiers(rows),
                block_size: None,
            },
            RatingModel::Volume { tiers: rows } => billing_core::pricing::UsageModel::Volume {
                tiers: tiers(rows),
                block_size: None,
            },
            RatingModel::Package { block_size, rate } => {
                billing_core::pricing::UsageModel::Package {
                    block_size: *block_size,
                    rate: *rate,
                }
            }
        }
    }

    /// What `quantity` units cost, a subline per tier or package reached.
    pub fn price(
        &self,
        quantity: rust_decimal::Decimal,
        precision: u8,
    ) -> Result<billing_core::pricing::PricedLine, billing_core::pricing::PricingError> {
        billing_core::pricing::quantity_line(&self.usage_model(), quantity, precision)
    }

    /// A fixed fee's line for `quantity` units, its `share` of a period. A whole period shows how
    /// the model priced the units: volume its one rate for all of them, tiers and packages a
    /// subline per tier or package reached, adding up to the line. A part of a period shows the
    /// rate its total works out to over the units, as a prorated per-unit line does: the
    /// period's sublines no longer add up to it, and a credit by sub-line would give back more.
    pub fn fixed_line(
        &self,
        quantity: rust_decimal::Decimal,
        share: Option<billing_core::proration::DayFraction>,
        precision: u8,
    ) -> Result<billing_core::pricing::PricedLine, billing_core::pricing::PricingError> {
        self.instances_line(quantity, rust_decimal::Decimal::ONE, share, precision)
    }

    /// [`Self::fixed_line`] for `instances` of a fee, each pricing its `quantity` units on its
    /// own: the line shows all the units, each subline its tier's units over the instances.
    fn instances_line(
        &self,
        quantity: rust_decimal::Decimal,
        instances: rust_decimal::Decimal,
        share: Option<billing_core::proration::DayFraction>,
        precision: u8,
    ) -> Result<billing_core::pricing::PricedLine, billing_core::pricing::PricingError> {
        use billing_core::pricing::{PricedLine, PricingError, simple_prorated};
        use common_utils::decimals::ToSubunit;
        let units = quantity * instances;
        if let Some(rate) = self.per_unit_rate() {
            return simple_prorated(&rate, &units, share, precision);
        }
        let priced = self.price(quantity, precision)?;
        let scaled = (instances != rust_decimal::Decimal::ONE).then(|| self.scaled(instances));
        let exact = scaled.as_ref().unwrap_or(self).exact_amount(quantity);
        let prorated = share.is_some_and(|share| !share.is_whole());
        let total = share
            .map_or(exact, |share| share.prorate_decimal(exact))
            .to_subunit_opt(precision)
            .ok_or(PricingError::OutOfRange)?
            .max(0);
        let line = PricedLine {
            quantity: Some(units),
            unit_price: None,
            total,
            is_prorated: prorated,
            sublines: Vec::new(),
            dimensions: None,
        };
        let worked_out = PricedLine {
            unit_price: (!units.is_zero()).then(|| {
                (rust_decimal::Decimal::new(total, u32::from(precision)) / units)
                    .round_dp_with_strategy(8, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
            }),
            ..line.clone()
        };
        if prorated {
            return Ok(worked_out);
        }
        let parts = self.breakdown(priced.sublines, instances);
        if matches!(self, RatingModel::Volume { .. })
            && let [(volume, amount)] = parts.as_slice()
            && volume.quantity * volume.unit_price == *amount
        {
            return Ok(PricedLine {
                unit_price: Some(volume.unit_price),
                ..line
            });
        }
        let shown = PricedLine::from_exact_sublines(parts, precision)?;
        Ok(if shown.total == total {
            PricedLine {
                sublines: shown.sublines,
                ..line
            }
        } else {
            worked_out
        })
    }

    /// What `sublines` priced for one instance show for `instances`, each at its exact amount
    /// and named for the units it prices: "First 3 included", "Next 2", "All 15", "Packs of 10".
    /// A tier's flat fee is a subline of its own, so each one's quantity times its unit price is
    /// its amount; a capped tier's cap is its amount.
    fn breakdown(
        &self,
        sublines: Vec<billing_core::pricing::SubLine>,
        instances: rust_decimal::Decimal,
    ) -> Vec<(billing_core::pricing::SubLine, rust_decimal::Decimal)> {
        use billing_core::pricing::{SubLine, SubLineAttributes};
        let mut parts = Vec::with_capacity(sublines.len());
        for (index, subline) in sublines.into_iter().enumerate() {
            let (flat_fee, flat_cap) = match &subline.attributes {
                Some(
                    SubLineAttributes::Tiered {
                        flat_fee, flat_cap, ..
                    }
                    | SubLineAttributes::Volume {
                        flat_fee, flat_cap, ..
                    },
                ) => (flat_fee.unwrap_or_default(), *flat_cap),
                _ => (rust_decimal::Decimal::ZERO, None),
            };
            let count = subline.quantity.normalize();
            let name = match self {
                RatingModel::Package { block_size, .. } => format!("Packs of {block_size}"),
                RatingModel::Volume { .. } => format!("All {count}"),
                _ if index == 0 => format!("First {count}"),
                _ => format!("Next {count}"),
            };
            let variable = subline.quantity * subline.unit_price;
            let uncapped = variable + flat_fee;
            let quantity = subline.quantity * instances;
            if let Some(cap) = flat_cap.filter(|cap| uncapped > *cap) {
                parts.push((
                    SubLine {
                        name: format!("{name}, capped"),
                        quantity,
                        ..subline
                    },
                    cap * instances,
                ));
                continue;
            }
            let name = if subline.unit_price.is_zero() && flat_fee.is_zero() {
                format!("{name} included")
            } else {
                name
            };
            parts.push((
                SubLine {
                    name,
                    quantity,
                    ..subline
                },
                variable * instances,
            ));
            if !flat_fee.is_zero() {
                parts.push((
                    SubLine {
                        name: "Flat fee".to_owned(),
                        total: 0,
                        quantity: instances,
                        unit_price: flat_fee,
                        attributes: None,
                    },
                    flat_fee * instances,
                ));
            }
        }
        parts
    }

    /// What `count` units cost a period, in minor units: per unit, the rounded rate times the
    /// count, as recurring totals and MRR have always counted seats.
    pub fn period_total_minor(&self, count: u64, precision: u8) -> i64 {
        use common_utils::decimals::ToSubunit;
        match self {
            RatingModel::PerUnit { rate } => i64::try_from(count)
                .unwrap_or(i64::MAX)
                .saturating_mul(rate.to_subunit_opt(precision).unwrap_or(0)),
            model => model
                .price(rust_decimal::Decimal::from(count), precision)
                .ok()
                .map(|line| line.total)
                .unwrap_or(0),
        }
    }

    /// The unrounded amount `quantity` units cost.
    pub fn exact_amount(&self, quantity: rust_decimal::Decimal) -> rust_decimal::Decimal {
        billing_core::pricing::usage_amount(&self.usage_model(), quantity).unwrap_or_default()
    }

    /// Every amount times `factor`: `n` instances of a fee priced through this model cost `n`
    /// times what one costs, at the same tier bounds and package size.
    pub fn scaled(&self, factor: rust_decimal::Decimal) -> RatingModel {
        let mut model = self.clone();
        match &mut model {
            RatingModel::PerUnit { rate } | RatingModel::Package { rate, .. } => *rate *= factor,
            RatingModel::Tiered { tiers } | RatingModel::Volume { tiers } => {
                for tier in tiers {
                    tier.rate *= factor;
                    tier.flat_fee = tier.flat_fee.map(|fee| fee * factor);
                    tier.flat_cap = tier.flat_cap.map(|cap| cap * factor);
                }
            }
        }
        model
    }

    /// The rate a price list shows: the unit rate, or the first tier's or package's.
    pub fn headline_rate(&self) -> rust_decimal::Decimal {
        match self {
            RatingModel::PerUnit { rate } | RatingModel::Package { rate, .. } => *rate,
            RatingModel::Tiered { tiers } | RatingModel::Volume { tiers } => tiers
                .iter()
                .min_by_key(|tier| tier.first_unit)
                .map(|tier| tier.rate)
                .unwrap_or_default(),
        }
    }

    /// Whether some quantity costs anything.
    pub fn charges(&self) -> bool {
        let positive = |rate: &rust_decimal::Decimal| *rate > rust_decimal::Decimal::ZERO;
        match self {
            RatingModel::PerUnit { rate } | RatingModel::Package { rate, .. } => positive(rate),
            RatingModel::Tiered { tiers } | RatingModel::Volume { tiers } => tiers
                .iter()
                .any(|tier| positive(&tier.rate) || tier.flat_fee.as_ref().is_some_and(positive)),
        }
    }

    /// The same pricing as a usage model, to show or edit it as one.
    pub fn as_usage_model(&self) -> UsagePricingModel {
        match self.clone() {
            RatingModel::PerUnit { rate } => UsagePricingModel::PerUnit { rate },
            RatingModel::Tiered { tiers } => UsagePricingModel::Tiered {
                tiers,
                block_size: None,
            },
            RatingModel::Volume { tiers } => UsagePricingModel::Volume {
                tiers,
                block_size: None,
            },
            RatingModel::Package { block_size, rate } => {
                UsagePricingModel::Package { block_size, rate }
            }
        }
    }

    /// A usage model priced over one quantity; rates by dimension or a rate card price
    /// dimensions, none of which a count has.
    pub fn from_usage_model(model: UsagePricingModel) -> Result<RatingModel, StoreError> {
        Ok(match model {
            UsagePricingModel::PerUnit { rate } => RatingModel::PerUnit { rate },
            UsagePricingModel::Tiered {
                block_size: Some(2..),
                ..
            }
            | UsagePricingModel::Volume {
                block_size: Some(2..),
                ..
            } => {
                return Err(StoreError::InvalidArgument(
                    "tiers price units one by one: use a package for blocks".to_owned(),
                ));
            }
            UsagePricingModel::Tiered { tiers, .. } => RatingModel::Tiered { tiers },
            UsagePricingModel::Volume { tiers, .. } => RatingModel::Volume { tiers },
            UsagePricingModel::Package { block_size, rate } => {
                RatingModel::Package { block_size, rate }
            }
            UsagePricingModel::DimensionRates(_) | UsagePricingModel::RateCard { .. } => {
                return Err(StoreError::InvalidArgument(
                    "a count has no dimensions to price by".to_owned(),
                ));
            }
        })
    }

    /// Reads a model, or the bare rate a seat price stored before models (`unit_rate`).
    pub fn rate_or_model<'de, D>(deserializer: D) -> Result<RatingModel, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Stored {
            Rate(rust_decimal::Decimal),
            Model(RatingModel),
        }
        Ok(match Stored::deserialize(deserializer)? {
            Stored::Rate(rate) => RatingModel::PerUnit { rate },
            Stored::Model(model) => model,
        })
    }
}

/// What `quantity` units of an extra recurring or one-time fee cost, unrounded: through `model`
/// when the price has one, else `unit_price` × `quantity`. Every reader of these fees goes here.
pub fn fixed_quantity_amount(
    unit_price: rust_decimal::Decimal,
    quantity: rust_decimal::Decimal,
    model: Option<&RatingModel>,
) -> rust_decimal::Decimal {
    match model {
        Some(model) => model.exact_amount(quantity),
        None => unit_price * quantity,
    }
}

/// [`fixed_quantity_amount`] in minor units, rounded once.
pub fn fixed_quantity_minor(
    unit_price: rust_decimal::Decimal,
    quantity: rust_decimal::Decimal,
    model: Option<&RatingModel>,
    precision: u8,
) -> i64 {
    use common_utils::decimals::ToSubunit;
    fixed_quantity_amount(unit_price, quantity, model)
        .to_subunit_opt(precision)
        .unwrap_or(0)
}

/// Whether a plan version's charges bill nothing, in any option and currency: none has a
/// positive price, so a subscriber needs no payment method. Included products charge nothing.
pub fn charges_nothing(components: &[PriceComponent]) -> bool {
    components.iter().all(|component| {
        !component.prices.iter().any(|price| price.pricing.charges())
            && component.legacy_pricing.as_ref().is_none_or(|legacy| {
                !legacy
                    .pricing_entries
                    .iter()
                    .any(|(_, pricing)| pricing.charges())
            })
    })
}

/// Whether some of `quantity` units of an extra recurring or one-time fee costs anything.
pub fn fixed_quantity_charges(
    unit_price: rust_decimal::Decimal,
    quantity: u32,
    model: Option<&RatingModel>,
) -> bool {
    quantity > 0
        && model.map_or(
            unit_price > rust_decimal::Decimal::ZERO,
            RatingModel::charges,
        )
}

/// The line of `instances` of an extra recurring or one-time fee of `quantity` units, its `share`
/// of a period, rounded once. A model prices each instance's quantity; the line shows all the
/// units and what the model shows of them (see [`RatingModel::fixed_line`]).
pub fn fixed_quantity_line(
    unit_price: rust_decimal::Decimal,
    quantity: rust_decimal::Decimal,
    instances: rust_decimal::Decimal,
    model: Option<&RatingModel>,
    share: Option<billing_core::proration::DayFraction>,
    precision: u8,
) -> Result<billing_core::pricing::PricedLine, billing_core::pricing::PricingError> {
    match model {
        Some(model) => model.instances_line(quantity, instances, share, precision),
        None => billing_core::pricing::simple_prorated(
            &unit_price,
            &(quantity * instances),
            share,
            precision,
        ),
    }
}

/// A dimension and its value: a matrix cell's, or a key the price editor's bulk edit names.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MatrixDimension {
    pub key: String,
    pub value: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
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
    "usage_dimension_rates" => FeeType::Usage {
        metric_id: Uuid::nil().into(),
        pricing: UsagePricingModel::DimensionRates(DimensionRates {
            dimensions: vec!["model".to_string(), "token_type".to_string()],
            rates: vec![DimensionRate {
                key: vec!["gpt-*".to_string(), "*".to_string()],
                rate: rust_decimal::Decimal::new(250, 2),
                label: Some("GPT".to_string()),
            }],
            unit_size: rust_decimal::Decimal::new(1_000_000, 0),
            on_unmatched: super::rate_cards::RateCardUnmatched::Hold,
        }),
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
mod rating_model_tests {
    use super::*;
    use crate::domain::SubscriptionFee;
    use crate::domain::prices::Pricing;
    use rust_decimal_macros::dec;

    #[test]
    fn a_seat_price_stored_with_a_rate_reads_as_per_unit() {
        let price: Pricing =
            serde_json::from_str(r#"{"type":"Slot","unit_rate":"12.50","min_slots":1}"#).unwrap();
        assert!(matches!(
            price,
            Pricing::Slot { model: RatingModel::PerUnit { rate }, .. } if rate == dec!(12.50)
        ));
        let fee: SubscriptionFee = serde_json::from_str(
            r#"{"Slot":{"unit":"seat","unit_rate":"3","min_slots":null,"max_slots":null,"initial_slots":2}}"#,
        )
        .unwrap();
        assert!(matches!(
            fee,
            SubscriptionFee::Slot { model: RatingModel::PerUnit { rate }, .. } if rate == dec!(3)
        ));
    }

    #[test]
    fn a_tiered_seat_price_round_trips() {
        let price = Pricing::Slot {
            model: RatingModel::Tiered {
                tiers: vec![TierRow {
                    first_unit: 0,
                    rate: dec!(10),
                    flat_fee: None,
                    flat_cap: None,
                }],
            },
            min_slots: None,
            max_slots: None,
        };
        let stored = serde_json::to_value(&price).unwrap();
        let read: Pricing = serde_json::from_value(stored).unwrap();
        assert!(matches!(
            read,
            Pricing::Slot {
                model: RatingModel::Tiered { .. },
                ..
            }
        ));
    }

    #[test]
    fn a_count_is_priced_through_its_tiers() {
        let tier = |first_unit, rate| TierRow {
            first_unit,
            rate,
            flat_fee: None,
            flat_cap: None,
        };
        let model = RatingModel::Tiered {
            tiers: vec![tier(0, dec!(10)), tier(10, dec!(8))],
        };
        assert_eq!(model.period_total_minor(15, 2), 14_000);
        assert_eq!(
            RatingModel::PerUnit { rate: dec!(10) }.period_total_minor(15, 2),
            15_000
        );
        assert!(
            RatingModel::from_usage_model(UsagePricingModel::DimensionRates(DimensionRates {
                dimensions: vec!["model".to_owned()],
                rates: vec![],
                unit_size: dec!(1),
                on_unmatched: Default::default(),
            }))
            .is_err()
        );
    }

    fn tier(first_unit: u64, rate: rust_decimal::Decimal) -> TierRow {
        TierRow {
            first_unit,
            rate,
            flat_fee: None,
            flat_cap: None,
        }
    }

    #[test]
    fn a_fixed_quantity_without_a_model_is_its_unit_price_times_its_quantity() {
        assert_eq!(fixed_quantity_amount(dec!(12.5), dec!(3), None), dec!(37.5));
        assert_eq!(fixed_quantity_minor(dec!(0.125), dec!(3), None, 2), 38);
        assert!(fixed_quantity_charges(dec!(1), 1, None));
        assert!(!fixed_quantity_charges(dec!(1), 0, None));
        assert!(!fixed_quantity_charges(dec!(0), 3, None));
    }

    #[test]
    fn a_fixed_quantity_with_a_model_ignores_its_unit_price() {
        let tiers = vec![tier(0, dec!(10)), tier(10, dec!(8))];
        let amount = |model: RatingModel| fixed_quantity_amount(dec!(99), dec!(15), Some(&model));
        // Graduated: 10 × 10 + 5 × 8.
        assert_eq!(
            amount(RatingModel::Tiered {
                tiers: tiers.clone()
            }),
            dec!(140)
        );
        // Volume: all 15 at the tier they land in.
        assert_eq!(amount(RatingModel::Volume { tiers }), dec!(120));
        // Package: 2 packs of 10.
        assert_eq!(
            amount(RatingModel::Package {
                block_size: 10,
                rate: dec!(30)
            }),
            dec!(60)
        );
        assert_eq!(amount(RatingModel::PerUnit { rate: dec!(2) }), dec!(30));
        assert!(fixed_quantity_charges(
            dec!(0),
            1,
            Some(&RatingModel::PerUnit { rate: dec!(2) })
        ));
        assert!(!fixed_quantity_charges(
            dec!(5),
            1,
            Some(&RatingModel::PerUnit { rate: dec!(0) })
        ));
    }

    /// Each subline's name, quantity, unit price and total.
    fn shown(
        line: &billing_core::pricing::PricedLine,
    ) -> Vec<(&str, rust_decimal::Decimal, rust_decimal::Decimal, i64)> {
        line.sublines
            .iter()
            .map(|s| (s.name.as_str(), s.quantity, s.unit_price, s.total))
            .collect()
    }

    #[test]
    fn a_modelled_line_shows_its_tiers_over_a_period_and_its_worked_out_rate_over_a_part() {
        let model = RatingModel::Tiered {
            tiers: vec![tier(0, dec!(10)), tier(10, dec!(8))],
        };
        let line = fixed_quantity_line(dec!(0), dec!(15), dec!(1), Some(&model), None, 2).unwrap();
        assert_eq!(line.total, 14_000);
        assert_eq!(line.quantity, Some(dec!(15)));
        assert_eq!(line.unit_price, None);
        assert_eq!(
            shown(&line),
            vec![
                ("First 10", dec!(10), dec!(10), 10_000),
                ("Next 5", dec!(5), dec!(8), 4_000)
            ]
        );

        let half = billing_core::proration::DayFraction::new(1, 2);
        let prorated =
            fixed_quantity_line(dec!(0), dec!(15), dec!(1), Some(&model), Some(half), 2).unwrap();
        assert_eq!(prorated.total, 7_000);
        assert!(prorated.is_prorated);
        assert_eq!(prorated.unit_price, Some(dec!(4.66666667)));
        assert!(prorated.sublines.is_empty());

        let plain = fixed_quantity_line(dec!(12.5), dec!(3), dec!(1), None, None, 2).unwrap();
        assert_eq!((plain.total, plain.unit_price), (3_750, Some(dec!(12.5))));
    }

    /// "3 included, then 10.00": five seats show the three free apart from the two billed.
    #[test]
    fn seats_past_an_included_tier_show_both() {
        let model = RatingModel::Tiered {
            tiers: vec![tier(0, dec!(0)), tier(3, dec!(10))],
        };
        let line = model.fixed_line(dec!(5), None, 2).unwrap();
        assert_eq!(
            (line.total, line.quantity, line.unit_price),
            (2_000, Some(dec!(5)), None)
        );
        assert_eq!(
            shown(&line),
            vec![
                ("First 3 included", dec!(3), dec!(0), 0),
                ("Next 2", dec!(2), dec!(10), 2_000)
            ]
        );
        // Within the included tier, the line shows it alone.
        let free = model.fixed_line(dec!(2), None, 2).unwrap();
        assert_eq!(free.total, 0);
        assert_eq!(
            shown(&free),
            vec![("First 2 included", dec!(2), dec!(0), 0)]
        );
    }

    #[test]
    fn a_volume_line_shows_its_one_rate_and_a_package_line_its_packs() {
        let volume = RatingModel::Volume {
            tiers: vec![tier(0, dec!(10)), tier(10, dec!(8))],
        };
        let line = volume.fixed_line(dec!(15), None, 2).unwrap();
        assert_eq!(
            (line.total, line.quantity, line.unit_price),
            (12_000, Some(dec!(15)), Some(dec!(8)))
        );
        assert!(line.sublines.is_empty());
        let half = billing_core::proration::DayFraction::new(1, 2);
        let prorated = volume.fixed_line(dec!(15), Some(half), 2).unwrap();
        assert_eq!(
            (prorated.total, prorated.unit_price),
            (6_000, Some(dec!(4)))
        );

        let package = RatingModel::Package {
            block_size: 10,
            rate: dec!(30),
        };
        let line = package.fixed_line(dec!(12), None, 2).unwrap();
        assert_eq!(
            (line.total, line.quantity, line.unit_price),
            (6_000, Some(dec!(12)), None)
        );
        assert_eq!(
            shown(&line),
            vec![("Packs of 10", dec!(2), dec!(30), 6_000)]
        );
    }

    /// A flat fee is a subline of its own, so each subline is its quantity times its unit price;
    /// a capped tier bills its cap.
    #[test]
    fn flat_fees_and_caps_show_apart() {
        let with_fee = RatingModel::Tiered {
            tiers: vec![
                tier(0, dec!(0)),
                TierRow {
                    flat_fee: Some(dec!(20)),
                    ..tier(3, dec!(10))
                },
            ],
        };
        let line = with_fee.fixed_line(dec!(5), None, 2).unwrap();
        assert_eq!(line.total, 4_000);
        assert_eq!(
            shown(&line),
            vec![
                ("First 3 included", dec!(3), dec!(0), 0),
                ("Next 2", dec!(2), dec!(10), 2_000),
                ("Flat fee", dec!(1), dec!(20), 2_000)
            ]
        );

        let volume_fee = RatingModel::Volume {
            tiers: vec![TierRow {
                flat_fee: Some(dec!(5)),
                ..tier(0, dec!(10))
            }],
        };
        let line = volume_fee.fixed_line(dec!(3), None, 2).unwrap();
        assert_eq!((line.total, line.unit_price), (3_500, None));
        assert_eq!(
            shown(&line),
            vec![
                ("All 3", dec!(3), dec!(10), 3_000),
                ("Flat fee", dec!(1), dec!(5), 500)
            ]
        );

        let capped = RatingModel::Tiered {
            tiers: vec![TierRow {
                flat_cap: Some(dec!(25)),
                ..tier(0, dec!(10))
            }],
        };
        let line = capped.fixed_line(dec!(5), None, 2).unwrap();
        assert_eq!(line.total, 2_500);
        assert_eq!(
            shown(&line),
            vec![("First 5, capped", dec!(5), dec!(10), 2_500)]
        );
    }

    /// Sublines settle their rounding on the largest, so they add up to the line.
    #[test]
    fn a_modelled_lines_sublines_add_up_to_it() {
        let model = RatingModel::Tiered {
            tiers: vec![
                tier(0, dec!(0.333)),
                tier(1, dec!(0.333)),
                tier(3, dec!(0.333)),
            ],
        };
        let line = model.fixed_line(dec!(6), None, 2).unwrap();
        assert_eq!(line.total, 200);
        assert_eq!(line.sublines.iter().map(|s| s.total).sum::<i64>(), 200);
    }

    #[test]
    fn instances_of_a_modelled_fee_each_cost_what_the_model_prices() {
        let model = RatingModel::Tiered {
            tiers: vec![tier(0, dec!(10)), tier(10, dec!(8))],
        };
        // Twice 140.00, not 30 units through the tiers (260.00).
        let line = fixed_quantity_line(dec!(0), dec!(15), dec!(2), Some(&model), None, 2).unwrap();
        assert_eq!(line.total, 28_000);
        assert_eq!(line.quantity, Some(dec!(30)));
        assert_eq!(line.unit_price, None);
        // Each tier's units over both instances, at its rate.
        assert_eq!(
            shown(&line),
            vec![
                ("First 10", dec!(20), dec!(10), 20_000),
                ("Next 5", dec!(10), dec!(8), 8_000)
            ]
        );
        let per_unit = RatingModel::PerUnit { rate: dec!(2.5) };
        let line =
            fixed_quantity_line(dec!(0), dec!(3), dec!(2), Some(&per_unit), None, 2).unwrap();
        assert_eq!(
            (line.total, line.quantity, line.unit_price),
            (1_500, Some(dec!(6)), Some(dec!(2.5)))
        );
        assert_eq!(
            model.scaled(dec!(2)).exact_amount(dec!(15)),
            dec!(2) * model.exact_amount(dec!(15))
        );
        let package = RatingModel::Package {
            block_size: 10,
            rate: dec!(30),
        };
        assert_eq!(package.scaled(dec!(3)).exact_amount(dec!(25)), dec!(270));
    }

    #[test]
    fn a_fixed_quantity_price_stored_without_a_model_reads_unchanged() {
        let price: Pricing =
            serde_json::from_str(r#"{"type":"ExtraRecurring","unit_price":"5","quantity":2}"#)
                .unwrap();
        assert!(matches!(
            price,
            Pricing::ExtraRecurring { unit_price, quantity: 2, model: None } if unit_price == dec!(5)
        ));
        let fee: SubscriptionFee =
            serde_json::from_str(r#"{"OneTime":{"rate":"7","quantity":3}}"#).unwrap();
        assert!(matches!(fee, SubscriptionFee::OneTime { model: None, .. }));
        // An absent model is not written, so rows stored before models read back the same.
        let stored = serde_json::to_value(&price).unwrap();
        assert!(stored.get("model").is_none());

        let modelled = Pricing::OneTime {
            unit_price: dec!(10),
            quantity: 4,
            model: Some(RatingModel::Package {
                block_size: 3,
                rate: dec!(25),
            }),
        };
        let read: Pricing =
            serde_json::from_value(serde_json::to_value(&modelled).unwrap()).unwrap();
        assert!(matches!(
            read,
            Pricing::OneTime {
                model: Some(RatingModel::Package { block_size: 3, .. }),
                ..
            }
        ));
    }
}

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

#[cfg(test)]
mod charges_nothing_tests {
    use super::*;
    use common_domain::ids::{RateCardId, TenantId};
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;

    fn price(currency: &str, pricing: Pricing) -> Price {
        Price {
            id: PriceId::new(),
            product_id: ProductId::default(),
            cadence: BillingPeriodEnum::Monthly,
            currency: currency.to_owned(),
            pricing,
            tenant_id: TenantId::default(),
            created_at: chrono::NaiveDateTime::default(),
            archived_at: None,
            catalog: true,
            credit_unit: None,
        }
    }

    fn component(prices: Vec<Price>) -> PriceComponent {
        PriceComponent {
            id: PriceComponentId::new(),
            name: "charge".to_owned(),
            product_id: Some(ProductId::default()),
            option_prices: vec![],
            prices,
            legacy_pricing: None,
        }
    }

    fn charges(pricing: Pricing) -> bool {
        !charges_nothing(&[component(vec![price("USD", pricing)])])
    }

    fn usage(model: UsagePricingModel) -> Pricing {
        Pricing::Usage(model)
    }

    fn tiers(rate: Decimal, flat_fee: Option<Decimal>) -> Vec<TierRow> {
        vec![TierRow {
            first_unit: 0,
            rate,
            flat_fee,
            flat_cap: None,
        }]
    }

    #[test]
    fn included_products_and_zero_prices_charge_nothing() {
        assert!(charges_nothing(&[]));
        assert!(charges_nothing(&[component(vec![])]), "an included product");
        let zero = vec![
            price("USD", Pricing::Rate { rate: dec!(0) }),
            price(
                "USD",
                Pricing::Slot {
                    model: RatingModel::PerUnit { rate: dec!(0) },
                    min_slots: None,
                    max_slots: None,
                },
            ),
            price("USD", usage(UsagePricingModel::PerUnit { rate: dec!(0) })),
            price(
                "EUR",
                usage(UsagePricingModel::Tiered {
                    tiers: tiers(dec!(0), None),
                    block_size: None,
                }),
            ),
            price(
                "USD",
                Pricing::OneTime {
                    unit_price: dec!(0),
                    quantity: 1,
                    model: None,
                },
            ),
            price(
                "USD",
                Pricing::Credit {
                    unit_price: dec!(0),
                    quantity: 1,
                    credit_amount: dec!(100),
                    expiry: None,
                    credit_unit_id: None,
                    rollover: None,
                    once: false,
                },
            ),
            price(
                "USD",
                usage(UsagePricingModel::RateCard {
                    rate_card_id: RateCardId::new(),
                    multiplier: Some(dec!(0)),
                }),
            ),
        ];
        assert!(charges_nothing(&[component(zero), component(vec![])]));
    }

    #[test]
    fn any_positive_price_in_any_cell_charges() {
        assert!(charges(Pricing::Rate { rate: dec!(10) }));
        assert!(charges(Pricing::Slot {
            model: RatingModel::PerUnit { rate: dec!(5) },
            min_slots: None,
            max_slots: None,
        }));
        assert!(charges(Pricing::Capacity {
            rate: dec!(0),
            included: 1_000,
            overage_rate: dec!(0.01),
        }));
        assert!(charges(usage(UsagePricingModel::PerUnit {
            rate: dec!(0.001)
        })));
        assert!(
            charges(usage(UsagePricingModel::Volume {
                tiers: tiers(dec!(0), Some(dec!(20))),
                block_size: None,
            })),
            "a tier's flat fee"
        );
        assert!(charges(Pricing::OneTime {
            unit_price: dec!(99),
            quantity: 1,
            model: None,
        }));
        assert!(
            charges(usage(UsagePricingModel::RateCard {
                rate_card_id: RateCardId::new(),
                multiplier: None,
            })),
            "a rate card's rates are not read here"
        );
        // Free in dollars, paid in euros: not free.
        assert!(!charges_nothing(&[component(vec![
            price("USD", Pricing::Rate { rate: dec!(0) }),
            price("EUR", Pricing::Rate { rate: dec!(9) }),
        ])]));
    }
}

#[cfg(test)]
mod dimension_rates_tests {
    use super::*;
    use crate::domain::prices::Pricing;
    use crate::domain::rate_cards::RateCardUnmatched;
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;

    /// A matrix price stored before rates by dimension reads as its rates: a key per cell, `*`
    /// where a cell leaves a dimension out, unmatched usage left unbilled as the matrix did.
    /// Rules: R-136
    #[test]
    fn a_stored_matrix_reads_as_rates_by_dimension() {
        let stored = serde_json::json!({
            "type": "Usage",
            "Matrix": {
                "rates": [
                    {
                        "dimension1": {"key": "region", "value": "eu"},
                        "dimension2": {"key": "tier", "value": "gold"},
                        "per_unit_price": "0.20"
                    },
                    {
                        "dimension1": {"key": "region", "value": "us"},
                        "dimension2": null,
                        "per_unit_price": "0.10"
                    }
                ]
            }
        });
        let Pricing::Usage(UsagePricingModel::DimensionRates(rates)) =
            serde_json::from_value(stored).unwrap()
        else {
            panic!("a matrix reads as rates by dimension");
        };
        assert_eq!(rates.dimensions, vec!["region", "tier"]);
        let keys: Vec<(Vec<String>, Decimal)> = rates
            .rates
            .iter()
            .map(|rate| (rate.key.clone(), rate.rate))
            .collect();
        assert_eq!(
            keys,
            vec![
                (vec!["eu".to_owned(), "gold".to_owned()], dec!(0.20)),
                (vec!["us".to_owned(), "*".to_owned()], dec!(0.10)),
            ]
        );
        assert_eq!(rates.unit_size, dec!(1));
        assert_eq!(rates.on_unmatched, RateCardUnmatched::Ignore);

        // Written back, it is today's shape, and reads the same.
        let written = serde_json::to_value(Pricing::Usage(UsagePricingModel::DimensionRates(
            rates.clone(),
        )))
        .unwrap();
        assert!(written.get("DimensionRates").is_some());
        let Pricing::Usage(UsagePricingModel::DimensionRates(read)) =
            serde_json::from_value(written).unwrap()
        else {
            panic!("rates by dimension round-trip");
        };
        assert_eq!(read, rates);
    }

    /// A component and a subscription's fee carry the model as well: a matrix in either reads.
    #[test]
    fn a_matrix_in_a_fee_reads_as_rates_by_dimension() {
        let fee: FeeType = serde_json::from_str(
            r#"{"Usage":{"metric_id":"00000000-0000-0000-0000-000000000000","pricing":{"Matrix":{"rates":[{"dimension1":{"key":"model","value":"a"},"dimension2":null,"per_unit_price":"1"}]}}}}"#,
        )
        .unwrap();
        assert!(matches!(
            fee,
            FeeType::Usage {
                pricing: UsagePricingModel::DimensionRates(DimensionRates { ref dimensions, .. }),
                ..
            } if dimensions == &vec!["model".to_owned()]
        ));
        let fee: SubscriptionFee = serde_json::from_str(
            r#"{"Usage":{"metric_id":"00000000-0000-0000-0000-000000000000","model":{"Matrix":{"rates":[]}}}}"#,
        )
        .unwrap();
        assert!(matches!(
            fee,
            SubscriptionFee::Usage {
                model: UsagePricingModel::DimensionRates(_),
                ..
            }
        ));
    }

    fn rates(keys: &[&[&str]]) -> DimensionRates {
        DimensionRates {
            dimensions: vec!["model".to_owned(), "token_type".to_owned()],
            rates: keys
                .iter()
                .map(|key| DimensionRate {
                    key: key.iter().map(|p| (*p).to_owned()).collect(),
                    rate: dec!(1),
                    label: None,
                })
                .collect(),
            unit_size: dec!(1000000),
            on_unmatched: RateCardUnmatched::Hold,
        }
    }

    #[test]
    fn rates_billing_cannot_resolve_are_refused() {
        assert!(
            rates(&[&["gpt-*", "*"], &["*", "input"]])
                .validate()
                .is_ok()
        );
        assert!(
            rates(&[&["a", "*"], &["a", "*"]]).validate().is_err(),
            "two rates of one key"
        );
        assert!(
            rates(&[&["a"]]).validate().is_err(),
            "a key without a value per dimension"
        );
        assert!(
            DimensionRates {
                unit_size: dec!(0),
                ..rates(&[])
            }
            .validate()
            .is_err()
        );
        assert!(
            DimensionRates {
                dimensions: vec![],
                ..rates(&[])
            }
            .validate()
            .is_err()
        );
    }

    /// Rates a matrix can hold are its cells and read back as the same rates; any other is none.
    #[test]
    fn rates_a_matrix_can_hold_are_its_cells() {
        fn per_unit(keys: &[&[&str]]) -> DimensionRates {
            DimensionRates {
                unit_size: dec!(1),
                ..rates(keys)
            }
        }
        let shown = |rates: &DimensionRates| {
            rates.as_matrix_cells().map(|cells| {
                cells
                    .into_iter()
                    .map(|(first, second, rate)| {
                        (
                            (first.key, first.value),
                            second.map(|second| (second.key, second.value)),
                            rate,
                        )
                    })
                    .collect::<Vec<_>>()
            })
        };
        let pair = |key: &str, value: &str| (key.to_owned(), value.to_owned());

        let held = per_unit(&[&["gpt-4", "input"], &["gpt-4", "*"]]);
        assert_eq!(
            shown(&held),
            Some(vec![
                (
                    pair("model", "gpt-4"),
                    Some(pair("token_type", "input")),
                    dec!(1)
                ),
                (pair("model", "gpt-4"), None, dec!(1)),
            ])
        );
        let read = DimensionRates::from_matrix_cells(held.as_matrix_cells().unwrap());
        assert_eq!(
            (read.dimensions, read.rates),
            (held.dimensions.clone(), held.rates.clone())
        );

        let one_dimension = DimensionRates {
            dimensions: vec!["model".to_owned()],
            rates: vec![DimensionRate {
                key: vec!["gpt-4".to_owned()],
                rate: dec!(2),
                label: Some("GPT-4".to_owned()),
            }],
            ..per_unit(&[])
        };
        assert_eq!(
            shown(&one_dimension),
            Some(vec![(pair("model", "gpt-4"), None, dec!(2))])
        );
        // A matrix stored without cells.
        let empty = DimensionRates {
            dimensions: vec![],
            ..per_unit(&[])
        };
        assert_eq!(shown(&empty), Some(vec![]));

        assert_eq!(shown(&rates(&[&["gpt-4", "input"]])), None, "per 1M units");
        assert_eq!(shown(&per_unit(&[&["gpt-*", "input"]])), None, "a prefix");
        assert_eq!(shown(&per_unit(&[&["gpt-4", "in*"]])), None, "a prefix");
        assert_eq!(shown(&per_unit(&[&["*", "input"]])), None, "`*` first");
        assert_eq!(shown(&per_unit(&[])), None, "dimensions without rates");
        let three = DimensionRates {
            dimensions: vec![
                "model".to_owned(),
                "token_type".to_owned(),
                "region".to_owned(),
            ],
            rates: vec![DimensionRate {
                key: vec!["gpt-4".to_owned(), "input".to_owned(), "eu".to_owned()],
                rate: dec!(1),
                label: None,
            }],
            ..per_unit(&[])
        };
        assert_eq!(shown(&three), None, "three dimensions");
    }

    #[test]
    fn rates_charge_when_one_is_positive() {
        let free = DimensionRates {
            rates: vec![DimensionRate {
                key: vec!["*".to_owned(), "*".to_owned()],
                rate: dec!(0),
                label: None,
            }],
            ..rates(&[])
        };
        assert!(!UsagePricingModel::DimensionRates(free).charges());
        assert!(UsagePricingModel::DimensionRates(rates(&[&["a", "*"]])).charges());
    }
}
