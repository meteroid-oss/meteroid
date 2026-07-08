//! Tax calculation types and structures.
//!
//! This module contains the core types used for tax calculations across different
//! tax systems including VAT, GST, HST, and other regional tax schemes. It provides
//! the fundamental data structures and enums needed to represent tax scenarios,
//! trade agreements, and calculation rules.

use crate::errors::InputValidationError;
use log::debug;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;
use strum::Display;

/// Represents different types of tax systems used globally.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaxSystemType {
    /// Value Added Tax - Common in EU and many other countries
    Vat,
    /// Goods and Services Tax
    Gst,
    /// Provincial Sales Tax (Canadian)
    Pst,
    /// Harmonized Sales Tax (Canadian)
    Hst,
    /// Quebec Sales Tax
    Qst,
    /// No tax system applicable
    None,
}

/// Defines the type of transaction between parties.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransactionType {
    /// Business to Business transaction
    B2B,
    /// Business to Consumer transaction
    B2C,
}

/// Specifies how tax should be calculated for a given transaction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaxCalculationType {
    /// Use origin tax rate; below threshold
    Origin,
    /// Use destination tax rate; above threshold
    Destination,
    /// Buyer pays tax in their country
    ReverseCharge,
    /// Tax applies but at zero rate
    ZeroRated,
    /// No tax applies
    Exempt,
    /// Tax status unknown
    None,
    /// Calculation depends on threshold
    ThresholdBased,
}

/// Represents different types of taxes that can be applied.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "content")]
#[serde(rename_all = "snake_case")]
pub enum TaxType {
    /// Value Added Tax with specific rate
    VAT(VatRate),
    /// Goods and Services Tax
    GST,
    /// Harmonized Sales Tax
    HST,
    /// Provincial Sales Tax
    PST,
    /// Quebec Sales Tax
    QST,
    /// US State Sales Tax
    StateSalesTax,
}

/// Different rates that can be applied for Value Added Tax.
#[derive(Debug, Clone, Display, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum VatRate {
    /// Standard VAT rate
    Standard,
    /// Reduced VAT rate
    Reduced,
    /// Alternative reduced VAT rate
    ReducedAlt,
    /// Super-reduced VAT rate
    SuperReduced,
    /// Zero-rated goods/services
    Zero,
    /// VAT exempt
    Exempt,
    /// Reverse charge applies
    ReverseCharge,
}

/// Defines the type of trade agreement between regions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TradeAgreementType {
    /// Agreement between multiple countries in a customs union (e.g., EU)
    CustomsUnion,
    /// Agreement between states within a federal system
    FederalState,
}

/// Override options for trade agreement application.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "content")]
pub enum TradeAgreementOverride {
    /// Explicitly use a specific agreement (e.g., "EU", "USMCA")
    UseAgreement(String),
    /// Force no trade agreement to be applied
    NoAgreement,
}

/// Specifies which types of goods/services an agreement applies to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppliesTo {
    /// Whether the agreement applies to physical goods
    pub physical_goods: bool,
    /// Whether the agreement applies to digital goods
    pub digital_goods: bool,
    /// Whether the agreement applies to services
    pub services: bool,
}

/// Represents a trade agreement between regions or states.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeAgreement {
    /// Name of the trade agreement
    pub name: String,
    /// Type of the trade agreement
    pub r#type: TradeAgreementType,
    /// List of member regions/states
    pub members: Vec<String>,
    /// Whether agreement applies by default
    pub default_applicable: bool,
    /// Types of goods/services covered
    pub applies_to: AppliesTo,
    /// Tax rules under this agreement
    pub tax_rules: TaxRules,
}

impl TradeAgreement {
    /// Returns true if this is a federal-level agreement
    pub fn is_federal(&self) -> bool {
        self.r#type == TradeAgreementType::FederalState
    }

    /// Returns true if this is an international agreement
    pub fn is_international(&self) -> bool {
        self.r#type == TradeAgreementType::CustomsUnion
    }
}

/// Configuration for tax calculation rules based on various thresholds and conditions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxRuleConfig {
    /// Default tax calculation type
    pub r#type: TaxCalculationType,
    /// Tax calculation type for amounts below threshold
    pub below_threshold: Option<TaxCalculationType>,
    /// Tax calculation type for amounts above threshold
    pub above_threshold: Option<TaxCalculationType>,
    /// Monetary threshold for standard goods
    pub threshold: Option<u32>,
    /// Tax calculation type for digital products below threshold
    pub below_threshold_digital_products: Option<TaxCalculationType>,
    /// Tax calculation type for digital products above threshold
    pub above_threshold_digital_products: Option<TaxCalculationType>,
    /// Monetary threshold for digital products
    pub threshold_digital_products: Option<u32>,
    /// Whether a resale certificate is required for special treatment
    pub requires_resale_certificate: Option<bool>,
}

impl TaxRuleConfig {
    /// Determines the tax calculation type based on the amount and threshold
    ///
    /// # Arguments
    /// * `amount` - The transaction amount
    /// * `ignore_threshold` - Whether to ignore threshold-based calculations
    pub fn by_threshold(&self, amount: u32, ignore_threshold: bool) -> &TaxCalculationType {
        let has_threshold = self.below_threshold.is_some()
            && self.above_threshold.is_some()
            && self.threshold.is_some();
        if has_threshold {
            let rule_threshold: u32 = self.threshold.unwrap();
            if amount < rule_threshold && !ignore_threshold {
                return self.below_threshold.as_ref().unwrap();
            } else {
                return self.above_threshold.as_ref().unwrap();
            }
        }
        &self.r#type
    }

    /// Determines the tax calculation type for digital products based on amount and threshold
    pub fn by_digital_product_threshold(
        &self,
        amount: u32,
        ignore_threshold: bool,
    ) -> &TaxCalculationType {
        let has_threshold = self.below_threshold_digital_products.is_some()
            && self.above_threshold_digital_products.is_some()
            && self.threshold_digital_products.is_some();
        if has_threshold {
            let rule_threshold: u32 = self.threshold_digital_products.unwrap();
            if amount < rule_threshold && !ignore_threshold {
                return self.below_threshold_digital_products.as_ref().unwrap();
            } else {
                return self.above_threshold_digital_products.as_ref().unwrap();
            }
        }
        &self.r#type
    }

    /// Determines the appropriate tax calculation type based on product type and amount
    pub fn by_threshold_or_digital_product_threshold(
        &self,
        amount: u32,
        is_digital_product_or_service: bool,
        ignore_threshold: bool,
    ) -> &TaxCalculationType {
        if is_digital_product_or_service {
            return self.by_digital_product_threshold(amount, ignore_threshold);
        }
        self.by_threshold(amount, ignore_threshold)
    }

    /// Determines if the transaction qualifies for reseller treatment
    pub fn is_reseller(&self, has_resale_certificate: bool) -> bool {
        if self.requires_resale_certificate.is_some() {
            return self.requires_resale_certificate.unwrap() && has_resale_certificate;
        }
        false
    }
}

/// Collection of tax rules for different transaction scenarios
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxRules {
    /// Rules for internal B2B transactions
    pub internal_b2b: Option<TaxRuleConfig>,
    /// Rules for internal B2C transactions
    pub internal_b2c: Option<TaxRuleConfig>,
    /// Rules for external exports
    pub external_export: TaxRuleConfig,
}

/// Product-specific tax rules configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductRules {
    /// Default tax rule to apply
    pub default: String,
    /// Tax rules for specific products
    pub specific_products: HashMap<String, String>,
}

/// Represents tax information for a state/province
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct State {
    /// Standard tax rate for the state
    pub standard_rate: f64,
    /// Type of tax system used in the state
    #[serde(rename = "type")]
    pub tax_type: TaxSystemType,
}

/// Custom deserializer for handling rate values that might be boolean or numeric
fn deserialize_rate<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum RateValue {
        Number(f64),
        Boolean(bool),
    }

    match Option::<RateValue>::deserialize(deserializer)? {
        Some(RateValue::Number(rate)) => Ok(Some(rate)),
        Some(RateValue::Boolean(false)) => Ok(None),
        Some(RateValue::Boolean(true)) => Ok(None),
        None => Ok(None),
    }
}

/// Represents tax information for a country
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Country {
    /// Type of tax system used in the country
    #[serde(rename = "type")]
    pub tax_type: TaxSystemType,
    /// Currency code for the country
    pub currency: String,
    /// Standard tax rate
    pub standard_rate: f64,
    /// Reduced tax rate if applicable
    #[serde(default, deserialize_with = "deserialize_rate")]
    pub reduced_rate: Option<f64>,
    /// Alternative reduced tax rate if applicable
    #[serde(default, deserialize_with = "deserialize_rate")]
    pub reduced_rate_alt: Option<f64>,
    /// Super-reduced tax rate if applicable
    #[serde(default, deserialize_with = "deserialize_rate")]
    pub super_reduced_rate: Option<f64>,
    /// Parking rate if applicable
    #[serde(default, deserialize_with = "deserialize_rate")]
    pub parking_rate: Option<f64>,
    /// Full name of the VAT system
    pub vat_name: Option<String>,
    /// Abbreviation of the VAT system name
    pub vat_abbr: Option<String>,
    /// Tax information for states/provinces if applicable
    pub states: Option<HashMap<String, State>>,
}

/// Represents a geographical region for tax purposes
///
/// # Examples
///
/// ```
/// # use world_tax::types::Region;
/// // Create a region for France (no sub-region)
/// let france = Region::new("FR".to_string(), None).unwrap();
///
/// // Create a region for California, USA
/// let california = Region::new("US".to_string(), Some("US-CA".to_string())).unwrap();
/// ```
#[derive(Debug, Clone)]
pub struct Region {
    /// ISO 3166-1 alpha-2 country code
    pub country: String,
    /// Optional ISO 3166-2 region code
    pub region: Option<String>,
}

impl Region {
    /// Creates a new Region with validation
    pub fn new(country: String, region: Option<String>) -> Result<Self, InputValidationError> {
        Self::validate(&country, &region)?;
        Ok(Self { country, region })
    }

    /// Validates country and region codes against ISO standards
    fn validate(country: &str, region: &Option<String>) -> Result<(), InputValidationError> {
        let country_info = rust_iso3166::from_alpha2(country)
            .ok_or_else(|| InputValidationError::InvalidCountryCode(country.to_string()))?;

        debug!("Found country: {}", country_info.name);

        if let Some(region_code) = region {
            let _ = country_info
                .subdivisions()
                .ok_or_else(|| InputValidationError::UnexpectedRegionCode(region_code.clone()))?;

            let region_info = rust_iso3166::iso3166_2::from_code(region_code)
                .ok_or_else(|| InputValidationError::InvalidRegionCode(region_code.clone()))?;

            debug!("Found region: {}", region_info.name);
        }

        Ok(())
    }
}

/// Represents a complete tax calculation scenario
#[derive(Debug, Clone)]
pub struct TaxScenario {
    /// Region where the seller is located
    pub source_region: Region,
    /// Region where the buyer is located
    pub destination_region: Region,
    /// Type of transaction (B2B or B2C)
    pub transaction_type: TransactionType,
    /// How the tax should be calculated
    // pub calculation_type: TaxCalculationType,
    /// Optional override for trade agreement application
    pub trade_agreement_override: Option<TradeAgreementOverride>,
    /// Whether the product/service is digital
    pub is_digital_product_or_service: bool,
    /// Whether the buyer has a resale certificate (relevant for B2B in US)
    pub has_resale_certificate: bool,
    /// Whether to ignore thresholds in calculations
    pub ignore_threshold: bool,
    /// Specific VAT rate to apply if applicable
    pub vat_rate: Option<VatRate>,
}

/// Represents a specific tax rate and its characteristics.
#[derive(Debug, Serialize, Deserialize)]
pub struct TaxRate {
    /// The numerical tax rate as a decimal (e.g., 0.20 for 20%)
    pub rate: f64,
    /// The type of tax (VAT, GST, etc.)
    pub tax_type: TaxType,
    /// Whether this tax compounds on top of other taxes
    pub compound: bool,
}
