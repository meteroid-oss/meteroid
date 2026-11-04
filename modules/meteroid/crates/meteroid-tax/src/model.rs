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
pub struct CustomerTaxRate {
    pub tax_code: String,
    pub name: String,
    pub rate: rust_decimal::Decimal,
}

/// Tri-state party tax status (W6). `ReverseCharge` is an explicit merchant
/// choice, additive to the VIES-derived reverse charge the engine also computes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CustomerTaxStatus {
    #[default]
    Taxable,
    Exempt,
    ReverseCharge,
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
    pub custom_tax_rates: Vec<CustomerTaxRate>,
    pub tax_status: CustomerTaxStatus,
    /// Free-text legal mention surfaced on exempt/reverse-charge invoices.
    pub exemption_reason: Option<String>,
    pub billing_address: Address,
    /// Distinct ship-to address when the customer set one that differs from
    /// billing; `None` otherwise. External engines that are destination-based
    /// (e.g. US sales tax on physical goods) should resolve ship-to as
    /// `shipping_address` falling back to `billing_address`. The built-in
    /// engines are place-of-supply and use `billing_address` only.
    pub shipping_address: Option<Address>,
}

/// Whether the supply was priced as one to a taxable person. This is the *engine's*
/// finding, not a re-reading of the document: the place-of-supply article a document
/// cites has to match the basis the tax was computed on, and the two diverge as soon
/// as anything downstream looks at a different field (an operator-edited invoice
/// snapshot, a number that failed format validation, a pending VIES check).
///
/// Recorded alongside the exemption so renderers can cite the right article instead of
/// re-deriving one. When an explicit customer type lands on the customer record, only
/// the engine's `is_b2b` below changes and every consumer of this follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SupplyBasis {
    Business,
    Consumer,
}

#[derive(Debug)]
pub enum CustomerTax {
    TaxRates(Vec<CustomerTaxRate>),
    ResolvedTaxRate(world_tax::TaxRate),
    ResolvedMultipleTaxRates(Vec<world_tax::TaxRate>),
    Exempt,
    /// Zero-rated cross-border supply: no VAT applies and the document must carry the
    /// "VAT not applicable" export mention for the basis the supply was priced on.
    Export(SupplyBasis),
    ReverseCharge,
    /// See [`VatExemptionReason::NoVatTerritory`].
    NoVatTerritory,
    NoTax,
}

impl Clone for CustomerTax {
    fn clone(&self) -> Self {
        match self {
            CustomerTax::TaxRates(rates) => CustomerTax::TaxRates(rates.clone()),
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
            CustomerTax::Export(basis) => CustomerTax::Export(*basis),
            CustomerTax::ReverseCharge => CustomerTax::ReverseCharge,
            CustomerTax::NoVatTerritory => CustomerTax::NoVatTerritory,
            CustomerTax::NoTax => CustomerTax::NoTax,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TaxRateRule {
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
pub struct TaxRate {
    pub reference: String,
    pub name: String,
    pub tax_rules: Vec<TaxRateRule>,
}

pub struct LineItemForTax {
    pub line_id: String,
    /// Signed subunit amount. Credit/proration lines are negative and reduce tax
    /// symmetrically (W4); the currency's precision governs the subunit scale.
    pub amount: i64,
    pub custom_taxes: Vec<TaxRate>,
    /// Resolved provider-agnostic tax category key (product's category, else the
    /// invoicing entity default). Engines may price on it; None if unclassified.
    /// The `nontaxable` key is special-cased to exempt; all others are standard-rated.
    pub tax_category: Option<String>,
    /// `amount` already contains the tax: the engine carves the tax out of it instead of
    /// adding it on top, and reports what is left as the line's `pre_tax_amount`. A line
    /// that resolves to no tax (exempt, reverse charge, export) keeps its whole amount —
    /// the price the customer was quoted is what they pay.
    pub tax_inclusive: bool,
}

/// Key of the built-in tax category that never yields tax, seeded in `tax_category`.
pub const NONTAXABLE_CATEGORY_KEY: &str = "nontaxable";

#[derive(Debug, Clone)]
pub struct LineItemWithTax {
    pub line_id: String,
    /// The line's net base. For a tax-inclusive line this is its amount less the tax
    /// carved out of it, so `pre_tax_amount + tax` is always the amount that came in.
    pub pre_tax_amount: i64,
    /// What this line's resolved taxes come to per unit of net — compounding folded in,
    /// and before any rounding. A tax-inclusive price of this line carries exactly this
    /// much tax, so anything that has to divide such a price back down (MRR on a
    /// tax-inclusive subscription) reads the rate here instead of re-deriving an
    /// approximate one from the rounded amounts. Zero for an exempt line.
    pub effective_rate: rust_decimal::Decimal,
    pub tax_details: TaxDetails,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum VatExemptionReason {
    TaxExempt,
    ReverseCharge,
    NotRegistered,
    /// Zero-rated cross-border supply to a customer outside the seller's VAT
    /// territory (e.g. an EU seller invoicing outside the EU). No VAT applies and the
    /// document must carry the corresponding legal mention, whose place-of-supply
    /// article depends on the basis the supply was priced on.
    Export(SupplyBasis),
    /// A supply taxable in a part of the seller's own VAT territory where VAT does not
    /// apply at all: Guyane and Mayotte, CGI art. 294-1 ("la taxe sur la valeur
    /// ajoutée n'est provisoirement pas applicable"). Not an export — the supply never
    /// leaves the territory — and not undetermined either: the law states that no VAT
    /// applies, and the document must say so and cite it.
    NoVatTerritory,
}

/// What levy an engine-resolved rate is, as a fact rather than a display string.
///
/// `tax_name` stays alongside it as the engine's own (English) label: it is the
/// grouping key, the persisted fallback for rows written before the kind was
/// recorded, and what merchant-authored custom taxes carry (those are named by the
/// merchant and have no kind). The renderer words the kind in the document's
/// language — "VAT" is not a name but a word of the presentation language ("TVA",
/// "USt.", "btw"), and the engine has no l10n to say it in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaxKind {
    Vat,
    /// Goods and services tax (Canada federal, Australia, New Zealand, Singapore, India).
    Gst,
    /// Harmonized sales tax (Canadian provinces that merged PST into GST).
    Hst,
    /// Provincial sales tax (Canada).
    Pst,
    /// Quebec sales tax.
    Qst,
    /// US state sales tax.
    SalesTax,
}

#[derive(Debug, Clone)]
pub struct TaxItem {
    pub tax_rate: rust_decimal::Decimal,
    pub tax_reference: String,
    pub tax_name: String,
    /// `None` for merchant-authored custom taxes, whose name is the merchant's own.
    pub tax_kind: Option<TaxKind>,
    pub tax_amount: i64,
}

#[derive(Debug, Clone)]
pub enum TaxDetails {
    Tax {
        tax_rate: rust_decimal::Decimal,
        tax_reference: String,
        tax_name: String,
        /// See [`TaxItem::tax_kind`].
        tax_kind: Option<TaxKind>,
        tax_amount: i64,
    },
    MultipleTaxes {
        taxes: Vec<TaxItem>,
        total_tax_amount: i64,
    },
    Exempt(VatExemptionReason),
}

pub struct TaxBreakdownItem {
    pub taxable_amount: i64,
    pub details: TaxDetails,
}

pub struct CalculationResult {
    pub tax_amount: i64,
    pub total_amount_after_tax: i64,
    pub breakdown: Vec<TaxBreakdownItem>,
    pub line_items: Vec<LineItemWithTax>,
    /// Customer's free-text exemption mention, surfaced onto exempt/reverse-charge
    /// breakdown items (legally required on EU exempt invoices). `None` when the
    /// customer provided none.
    pub exemption_reason: Option<String>,
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
