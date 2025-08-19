use chrono::NaiveDate;
use rust_decimal::Decimal;
use rusty_money::iso;
use rusty_money::iso::Currency;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum PaymentStatus {
    Paid,
    PartiallyPaid,
    Unpaid,
}

impl PaymentStatus {
    pub(crate) fn as_template_string(&self) -> String {
        match self {
            PaymentStatus::Paid => "paid".to_string(),
            PaymentStatus::PartiallyPaid => "partially_paid".to_string(),
            PaymentStatus::Unpaid => "unpaid".to_string(),
        }
    }
}

pub struct Invoice {
    pub lang: String,
    pub organization: Organization,
    pub customer: Customer,
    pub metadata: InvoiceMetadata,
    pub lines: Vec<InvoiceLine>,
    pub coupons: Vec<Coupon>,
    pub payment_status: Option<PaymentStatus>, // "paid", "partially_paid", or "unpaid"
    pub transactions: Vec<Transaction>,
    pub bank_details: Option<HashMap<String, String>>, // TODO here we need the BankAccount format, and we mak to kv in the Typst types
    pub tax_breakdown: Vec<TaxBreakdownItem>,
}

pub enum TaxExemptionType {
    ReverseCharge,
    TaxExempt,
    NotRegistered,
    Other(String),
}

pub struct TaxBreakdownItem {
    pub name: String,
    pub rate: Decimal,
    pub amount: Price,
    pub exemption_type: Option<TaxExemptionType>,
}

#[derive(Default)]
pub struct Address {
    pub line1: Option<String>,
    pub line2: Option<String>,
    pub city: Option<String>,
    pub country: Option<String>,
    pub state: Option<String>,
    pub zip_code: Option<String>,
}

pub struct Organization {
    pub logo_src: Option<String>,
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

pub struct InvoiceMetadata {
    pub number: String,
    pub issue_date: chrono::NaiveDate,
    pub payment_term: u32,
    pub subtotal: Price,
    pub tax_amount: Price,
    pub total_amount: Price,
    pub currency: &'static iso::Currency,
    pub due_date: chrono::NaiveDate,
    pub memo: Option<String>,
    pub payment_url: Option<String>,
    pub flags: Flags,
}

#[derive(Default)]
pub struct Flags {
    pub show_payment_status: Option<bool>,
    pub show_payment_info: Option<bool>,
    pub show_terms: Option<bool>,
    pub show_tax_info: Option<bool>,
    pub show_legal_info: Option<bool>,
    pub whitelabel: Option<bool>,
}

type Price = rusty_money::Money<'static, Currency>;

pub struct InvoiceLine {
    pub name: String,
    pub description: Option<String>,
    pub subtotal: Price,
    pub quantity: Option<Decimal>,
    pub unit_price: Option<Price>,
    pub tax_rate: Decimal,
    pub start_date: chrono::NaiveDate,
    pub end_date: chrono::NaiveDate,
    pub sub_lines: Vec<InvoiceSubLine>,
}

pub struct InvoiceSubLine {
    pub name: String,
    pub total: Price,
    pub quantity: Decimal,
    pub unit_price: Price,
    // pub attributes: Option<SubLineAttributes>,
}

pub struct Transaction {
    /// ex: "Card •••• 7726" or "Bank Transfer"
    pub method: String,
    pub date: NaiveDate,
    pub amount: Price,
}

pub struct Coupon {
    pub name: String,
    pub total: Price,
}
