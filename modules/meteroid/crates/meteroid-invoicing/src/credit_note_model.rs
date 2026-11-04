use chrono::NaiveDate;
use meteroid_money::iso;
use meteroid_money::iso::Currency;
use rust_decimal::Decimal;

#[derive(Debug, Clone, Copy)]
pub enum CreditType {
    CreditToBalance,
    Refund,
    DebtCancellation,
}

impl CreditType {
    pub(crate) fn as_template_string(&self) -> String {
        match self {
            CreditType::CreditToBalance => "credit_to_balance".to_string(),
            CreditType::Refund => "refund".to_string(),
            CreditType::DebtCancellation => "debt_cancellation".to_string(),
        }
    }
}

pub struct CreditNote {
    pub lang: String,
    pub organization: Organization,
    pub customer: Customer,
    pub metadata: CreditNoteMetadata,
    pub lines: Vec<CreditNoteLine>,
    pub tax_breakdown: Vec<TaxBreakdownItem>,
}

pub struct CreditNoteMetadata {
    pub number: String,
    pub issue_date: NaiveDate,
    pub related_invoice_number: String,
    pub related_invoice_date: NaiveDate,
    pub subtotal: Price,
    pub tax_amount: Price,
    pub total_amount: Price,
    pub currency: &'static iso::Currency,
    pub reason: Option<String>,
    pub memo: Option<String>,
    pub credit_type: CreditType,
    pub refunded_amount: Price,
    pub credited_amount: Price,
    pub debt_cancelled_amount: Price,
    /// Promotional credit the note gives back to the customer's grants, positive. On a
    /// tax-inclusive invoice it is the tax-included value drawn, above the lines' net share.
    pub promotional_credit_restored: Price,
    /// A draft's restored figure is the lines' estimate; an issued note's is the ledger's.
    pub promotional_credit_restored_estimated: bool,
    pub flags: Flags,
}

#[derive(Default)]
pub struct Flags {
    pub show_tax_info: Option<bool>,
    pub show_legal_info: Option<bool>,
    pub show_footer_custom_info: Option<bool>,
    pub whitelabel: Option<bool>,
}

type Price = meteroid_money::Money<'static, Currency>;

/// The parties, their addresses and the VAT breakdown are the same things on a
/// credit note as on an invoice — and EN 16931 gives them the same business
/// terms. Shared rather than duplicated, so one mapping serves both.
pub use crate::model::{
    Address, Customer, Organization, SupplyBasis, TaxBreakdownItem, TaxExemptionType, TaxKind,
};

pub struct CreditNoteLine {
    pub name: String,
    pub description: Option<String>,
    pub subtotal: Price,
    /// The base tax applied to; equals `subtotal` when nothing was allocated.
    pub taxable_amount: Price,
    pub quantity: Option<Decimal>,
    pub unit_price: Option<Price>,
    pub tax_rate: Decimal,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub is_prorated: bool,
    pub sub_lines: Vec<CreditNoteSubLine>,
    /// Of `subtotal - taxable_amount`, the promotional credit the note gives back (stored
    /// negative, like the line); the rest is discount.
    pub promotional_credit: Option<Price>,
}

pub struct CreditNoteSubLine {
    pub name: String,
    pub total: Price,
    pub quantity: Decimal,
    pub unit_price: Price,
}
