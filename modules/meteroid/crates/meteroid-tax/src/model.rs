use common_domain::country::CountryCode;

#[derive(Debug, Clone)]
pub struct Address {
    pub country: Option<CountryCode>,
    pub postal_code: Option<String>,
    pub line1: Option<String>,
    pub city: Option<String>,
    pub region: Option<String>, // ISO 3166-2
}

#[derive(Debug, Clone)]
pub struct CustomerCustomTaxRate {
    pub tax_code: String,
    pub name: String,
    pub rate: rust_decimal::Decimal,
}

#[derive(Debug, Clone)]
pub struct CustomerForTax {
    pub vat_number: Option<String>,
    pub vat_number_format_valid: bool,
    /// Definitive VIES answer for `vat_number` if any (`Some(true)` = registered).
    /// Pending/unavailable/never-checked numbers are `None`.
    pub vat_number_vies_valid: Option<bool>,
    /// Invoicing-entity policy: apply reverse charge (B2B) only once VIES has
    /// confirmed the number. Off = fail-open on format validity alone.
    pub require_vies_valid_for_reverse_charge: bool,
    pub custom_tax_rates: Vec<CustomerCustomTaxRate>,
    pub tax_exempt: bool,
    pub billing_address: Address,
}

#[derive(Debug)]
pub enum CustomerTax {
    CustomTaxRate(rust_decimal::Decimal),
    CustomTaxRates(Vec<CustomerCustomTaxRate>),
    ResolvedTaxRate(world_tax::TaxRate),
    ResolvedMultipleTaxRates(Vec<world_tax::TaxRate>),
    Exempt,
    NoTax,
}

impl Clone for CustomerTax {
    fn clone(&self) -> Self {
        match self {
            CustomerTax::CustomTaxRate(rate) => CustomerTax::CustomTaxRate(*rate),
            CustomerTax::CustomTaxRates(rates) => CustomerTax::CustomTaxRates(rates.clone()),
            CustomerTax::ResolvedTaxRate(tax_rate) => {
                CustomerTax::ResolvedTaxRate(world_tax::TaxRate {
                    rate: tax_rate.rate,
                    tax_type: tax_rate.tax_type.clone(),
                    compound: tax_rate.compound,
                })
            }
            CustomerTax::ResolvedMultipleTaxRates(rates) => CustomerTax::ResolvedMultipleTaxRates(
                rates
                    .iter()
                    .map(|tax_rate| world_tax::TaxRate {
                        rate: tax_rate.rate,
                        tax_type: tax_rate.tax_type.clone(),
                        compound: tax_rate.compound,
                    })
                    .collect(),
            ),
            CustomerTax::Exempt => CustomerTax::Exempt,
            CustomerTax::NoTax => CustomerTax::NoTax,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TaxRule {
    pub country: Option<CountryCode>,
    pub region: Option<String>,
    pub rate: rust_decimal::Decimal,
}

pub struct TaxEntry {
    pub reference: String,
    pub name: String,
    pub rate: rust_decimal::Decimal,
    pub taxable_amount: u64,
    pub tax_amount: u64,
    pub is_exempt: bool,
}

pub struct TaxRateEntry {
    pub reference: String,
    pub name: String,
    pub rate: rust_decimal::Decimal,
}

#[derive(Debug, Clone)]
pub struct CustomTax {
    pub reference: String,
    pub name: String,
    pub tax_rules: Vec<TaxRule>,
}

pub struct LineItemForTax {
    pub line_id: String,
    pub amount: u64,
    pub custom_taxes: Vec<CustomTax>,
    /// Resolved provider-agnostic tax category key (product's category, else the
    /// invoicing entity default). Engines may price on it; None if unclassified.
    pub tax_category: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LineItemWithTax {
    pub line_id: String,
    pub pre_tax_amount: u64,
    pub tax_details: TaxDetails,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum VatExemptionReason {
    TaxExempt,
    ReverseCharge,
    NotRegistered,
}

#[derive(Debug, Clone)]
pub struct TaxItem {
    pub tax_rate: rust_decimal::Decimal,
    pub tax_reference: String,
    pub tax_name: String,
    pub tax_amount: u64,
}

#[derive(Debug, Clone)]
pub enum TaxDetails {
    Tax {
        tax_rate: rust_decimal::Decimal,
        tax_reference: String,
        tax_name: String,
        tax_amount: u64,
    },
    MultipleTaxes {
        taxes: Vec<TaxItem>,
        total_tax_amount: u64,
    },
    Exempt(VatExemptionReason),
}

pub struct TaxBreakdownItem {
    pub taxable_amount: u64,
    pub details: TaxDetails,
}

pub struct CalculationResult {
    pub tax_amount: u64,
    pub total_amount_after_tax: u64,
    pub breakdown: Vec<TaxBreakdownItem>,
    pub line_items: Vec<LineItemWithTax>,
}

pub enum VatNumberExternalValidationResult {
    Valid,
    Invalid,
    ServiceUnavailable,
}

/// Raw fields captured from a definitive VIES answer, kept as audit evidence
/// for reverse-charge decisions (and to surface the registered identity).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ViesCheckData {
    pub request_date: Option<String>,
    /// Consultation number; VIES only issues one for qualified (requester-
    /// identified) checks, so this is usually absent.
    pub request_identifier: Option<String>,
    pub name: Option<String>,
    pub address: Option<String>,
}

/// Outcome of a VIES call: the tri-state result plus, on a definitive answer,
/// the evidence VIES returned alongside it.
pub struct ViesValidation {
    pub result: VatNumberExternalValidationResult,
    pub check: Option<ViesCheckData>,
}

impl From<VatNumberExternalValidationResult> for ViesValidation {
    fn from(result: VatNumberExternalValidationResult) -> Self {
        Self {
            result,
            check: None,
        }
    }
}
