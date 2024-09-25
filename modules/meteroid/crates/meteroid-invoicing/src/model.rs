use rust_decimal::Decimal;
use rusty_money::iso;

pub struct Invoice {
    pub lang: String,
    pub organization: Organization,
    pub customer: Customer,
    pub metadata: InvoiceMetadata,
    pub lines: Vec<InvoiceLine>,
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
    pub logo_url: Option<String>,
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
    pub subtotal: i64,
    pub tax_amount: i64,
    pub tax_rate: i32,
    pub total_amount: i64,
    pub currency: iso::Currency,
    pub due_date: chrono::NaiveDate,
    pub memo: Option<String>,
}

pub struct InvoiceLine {
    pub name: String,
    pub description: Option<String>,
    pub subtotal: i64,
    pub total: i64,
    pub quantity: Option<Decimal>,
    pub unit_price: Option<Decimal>,
    pub vat_rate: Option<Decimal>,
    pub start_date: chrono::NaiveDate,
    pub end_date: chrono::NaiveDate,
    pub sub_lines: Vec<InvoiceSubLine>,
}

pub struct InvoiceSubLine {
    pub name: String,
    pub total: i64,
    pub quantity: Decimal,
    pub unit_price: Decimal,
    // pub attributes: Option<SubLineAttributes>,
}
