//! The EN 16931 code lists this crate needs, and the mapping from our own
//! vocabulary onto them.

use crate::model::TaxExemptionType;
use rust_decimal::Decimal;

/// UNTDID 1001 — document type code (BT-3).
pub const TYPE_COMMERCIAL_INVOICE: &str = "380";
pub const TYPE_CREDIT_NOTE: &str = "381";
pub const TYPE_CORRECTED_INVOICE: &str = "384";

/// UN/ECE Recommendation 20 — "one", the unit for a countable item (BT-130).
///
/// Nothing in our catalogue carries a unit of measure, and a subscription line
/// bills a count of things (seats, requests, one plan). `C62` is the code for
/// exactly that, and BR-23 requires *some* code, so this is the default rather
/// than a guess made per line.
pub const UNIT_ONE: &str = "C62";

/// UNCL 5189 (BT-140) "Special rebate": a reduction the seller grants, as promotional credit is.
pub const ALLOWANCE_SPECIAL_REBATE: &str = "100";

/// UNTDID 4461 — credit transfer (BT-81).
pub const PAYMENT_MEANS_CREDIT_TRANSFER: &str = "30";
/// UNTDID 4461 — clearing between partners: a credit set off against what is owed.
pub const PAYMENT_MEANS_CLEARING: &str = "97";
/// UNTDID 4461 — mutually defined: a refund to whatever method the invoice was paid by.
pub const PAYMENT_MEANS_MUTUALLY_DEFINED: &str = "ZZZ";

/// BT-23 — the Peppol billing process, which XRechnung inherits (`PEPPOL-EN16931-R001`).
pub const PEPPOL_BILLING_PROCESS: &str = "urn:fdc:peppol.eu:2017:poacc:billing:01:1.0";

/// CEF EAS — an email address as an electronic address (BT-34 / BT-49).
pub const EAS_EMAIL: &str = "EM";

/// UNTDID 5305 — VAT category code (BT-118 / BT-151).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VatCategory {
    /// Standard rate.
    Standard,
    /// Zero rated.
    Zero,
    /// Exempt from VAT.
    Exempt,
    /// VAT reverse charge.
    ReverseCharge,
    /// Services outside scope of tax.
    ///
    /// UNCL 5305 `G` ("free export item, tax not charged") is deliberately absent: it
    /// is the art. 146 *goods* regime, and BR-G-02 requires a seller VAT identifier,
    /// which a non-VAT seller does not have. A service whose place of supply lies
    /// outside the seller's territory is outside the scope of that territory's VAT,
    /// which is `O`. Removing the variant makes it unrepresentable rather than merely
    /// unused.
    OutsideScope,
}

impl VatCategory {
    pub fn code(self) -> &'static str {
        match self {
            VatCategory::Standard => "S",
            VatCategory::Zero => "Z",
            VatCategory::Exempt => "E",
            VatCategory::ReverseCharge => "AE",
            VatCategory::OutsideScope => "O",
        }
    }

    /// CEF VATEX code (BT-121) justifying the exemption.
    ///
    /// `None` for the categories that need no justification: standard and zero rated
    /// are both "subject to VAT". `None` too for every category when the seller is
    /// outside the EU — VATEX is the CEF catalogue of *EU-law* bases, and BR-E-10,
    /// BR-AE-10 and BR-O-10 are each satisfied by the BT-120 text alone.
    ///
    /// `Exempt` carries no code even for an EU seller: there is no generic "exempt"
    /// VATEX. VATEX-EU-132 asserts the public-interest exemptions of art. 132
    /// specifically — hospitals, education, postal services — which nothing in a
    /// customer-level exempt flag establishes. The resolved BT-120 wording is the
    /// justification instead.
    pub fn vatex_code(self, seller_in_eu: bool) -> Option<&'static str> {
        if !seller_in_eu {
            return None;
        }
        match self {
            VatCategory::ReverseCharge => Some("VATEX-EU-AE"),
            VatCategory::OutsideScope => Some("VATEX-EU-O"),
            VatCategory::Standard | VatCategory::Zero | VatCategory::Exempt => None,
        }
    }

    /// The wording used when the merchant configured no exemption mention of
    /// their own. BT-120 or BT-121 is mandatory for every non-standard category.
    pub fn default_reason(self) -> Option<&'static str> {
        match self {
            VatCategory::Standard | VatCategory::Zero => None,
            VatCategory::Exempt => Some("Exempt from VAT"),
            VatCategory::ReverseCharge => Some("Reverse charge"),
            // The BR-O-10 standard text verbatim.
            VatCategory::OutsideScope => Some("Not subject to VAT"),
        }
    }

    /// Whether the category is one where the rate is meaningful.
    pub fn is_taxed(self) -> bool {
        matches!(self, VatCategory::Standard)
    }
}

/// The category a tax-breakdown row belongs to.
///
/// A zero rate with no exemption is `Z`, not `S`: a `S` row at 0 % contradicts
/// BR-S-08, and an invoice that carries no VAT because none applies is a
/// different statement than one taxed at zero.
/// Why a tax position cannot be expressed in EN 16931's vocabulary at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CategoryError {
    /// The engine recorded that no tax applied without establishing why. Every
    /// UNCL 5305 code is a positive legal assertion — `Z` a zero-rated taxable
    /// supply, `E` an exemption, `O` "not subject to VAT" — and an undetermined
    /// position supports none of them. The document is refused rather than
    /// misdescribed; the PDF invoice is still issued.
    UndeterminedTaxPosition,
}

impl std::fmt::Display for CategoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CategoryError::UndeterminedTaxPosition => f.write_str(
                "the tax engine did not establish why no tax applied, and EN 16931 has \
                 no code for an undetermined position",
            ),
        }
    }
}

/// The category a tax-breakdown row belongs to.
///
/// A cross-border *service* is never an exportation: `G` is the art. 146 goods regime.
/// A service whose place of supply lies outside the seller's territory (art. 44 B2B,
/// arts 58/59 B2C) is outside the scope of that territory's VAT — `O`. B2B and B2C
/// differ in the article BT-120 cites, not in the category, so `SupplyBasis` is
/// deliberately not a parameter.
pub fn category_of(
    exemption: Option<&TaxExemptionType>,
    rate: Decimal,
) -> Result<VatCategory, CategoryError> {
    match exemption {
        // Intra-EU B2B service, arts 44 + 196: the customer accounts for the VAT.
        Some(TaxExemptionType::ReverseCharge) => Ok(VatCategory::ReverseCharge),
        // Outside the seller's VAT territory — B2B or B2C, EU or non-EU seller.
        Some(TaxExemptionType::Export) => Ok(VatCategory::OutsideScope),
        // A part of the seller's territory where VAT does not apply (CGI art. 294-1):
        // not subject to VAT, the same category as an out-of-territory supply.
        Some(TaxExemptionType::NoVatTerritory) => Ok(VatCategory::OutsideScope),
        Some(TaxExemptionType::TaxExempt) => Ok(VatCategory::Exempt),
        Some(TaxExemptionType::NotRegistered) => Err(CategoryError::UndeterminedTaxPosition),
        None if rate.is_zero() => Ok(VatCategory::Zero),
        None => Ok(VatCategory::Standard),
    }
}
