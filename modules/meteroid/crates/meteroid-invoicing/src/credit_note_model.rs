use chrono::NaiveDate;
use common_domain::country::CountryCode;
use rust_decimal::Decimal;
use rusty_money::iso;
use rusty_money::iso::Currency;

#[derive(Debug, Clone)]
pub enum CreditType {
    CreditToBalance,
    Refund,
}

impl CreditType {
    pub(crate) fn as_template_string(&self) -> String {
        match self {
            CreditType::CreditToBalance => "credit_to_balance".to_string(),
            CreditType::Refund => "refund".to_string(),
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
    pub subtotal: Price,
    pub tax_amount: Price,
    pub total_amount: Price,
    pub currency: &'static iso::Currency,
    pub reason: Option<String>,
    pub memo: Option<String>,
    pub credit_type: CreditType,
    pub refunded_amount: Price,
    pub credited_amount: Price,
    pub flags: Flags,
}

#[derive(Default)]
pub struct Flags {
    pub show_tax_info: Option<bool>,
    pub show_legal_info: Option<bool>,
    pub show_footer_custom_info: Option<bool>,
    pub whitelabel: Option<bool>,
}

type Price = rusty_money::Money<'static, Currency>;

#[derive(Default, Clone)]
pub struct Address {
    pub line1: Option<String>,
    pub line2: Option<String>,
    pub city: Option<String>,
    pub country: Option<CountryCode>,
    pub state: Option<String>,
    pub zip_code: Option<String>,
}

#[derive(Clone)]
pub struct Organization {
    pub logo_src: Option<Vec<u8>>,
    pub name: String,
    pub legal_number: Option<String>,
    pub address: Address,
    pub email: Option<String>,
    pub tax_id: Option<String>,
    pub footer_info: Option<String>,
    pub footer_legal: Option<String>,
    pub accounting_currency: iso::Currency,
    pub exchange_rate: Option<Decimal>,
}

pub struct Customer {
    pub name: String,
    pub legal_number: Option<String>,
    pub address: Address,
    pub email: Option<String>,
    pub tax_id: Option<String>,
}

pub struct CreditNoteLine {
    pub name: String,
    pub description: Option<String>,
    pub subtotal: Price,
    pub quantity: Option<Decimal>,
    pub unit_price: Option<Price>,
    pub tax_rate: Decimal,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub sub_lines: Vec<CreditNoteSubLine>,
}

pub struct CreditNoteSubLine {
    pub name: String,
    pub total: Price,
    pub quantity: Decimal,
    pub unit_price: Price,
}

pub enum TaxExemptionType {
    ReverseCharge,
    TaxExempt,
    NotRegistered,
}

pub struct TaxBreakdownItem {
    pub name: String,
    pub rate: Decimal,
    pub amount: Price,
    pub exemption_type: Option<TaxExemptionType>,
}
