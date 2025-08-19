#[derive(Debug, Clone)]
pub struct Address {
    pub country: Option<String>,
    pub postal_code: Option<String>,
    pub line1: Option<String>,
    pub city: Option<String>,
    pub region: Option<String>, // ISO 3166-2
}

pub struct CustomerForTax {
    pub vat_number: Option<String>,
    pub vat_number_format_valid: bool,
    pub custom_tax_rate: Option<rust_decimal::Decimal>,
    pub tax_exempt: bool,
    pub billing_address: Address,
}

pub enum CustomerTax {
    CustomTaxRate(rust_decimal::Decimal),
    ResolvedTaxRate(world_tax::TaxRate),
    Exempt,
    NoTax,
}

pub struct TaxRule {
    pub country: Option<String>,
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

pub struct CustomTax {
    pub reference: String,
    pub name: String,
    pub tax_rules: Vec<TaxRule>,
}

pub struct LineItemForTax {
    pub line_id: String,
    pub amount: u64,
    pub custom_tax: Option<CustomTax>,
}

#[derive(Debug, Clone)]
pub struct LineItemWithTax {
    pub line_id: String,
    pub pre_tax_amount: u64,
    pub tax_details: TaxDetails,
}

#[derive(Debug, Clone)]
pub enum VatExemptionReason {
    TaxExempt,
    ReverseCharge,
    NotRegistered,
    Other(String),
}

#[derive(Debug, Clone)]
pub enum TaxDetails {
    Tax {
        tax_rate: rust_decimal::Decimal,
        tax_reference: String,
        tax_name: String,
        tax_amount: u64,
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
