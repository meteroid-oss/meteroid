use chrono::{NaiveDate, NaiveDateTime};
use common_domain::country::CountryCode;
use meteroid_money::iso;
use meteroid_money::iso::Currency;
use rust_decimal::Decimal;
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
    /// Structured counterpart of `bank_details`, for the e-invoice payment instructions
    /// (BG-16). `bank_details` stays the display form and is free-text by design.
    pub payment_means: Option<PaymentMeans>,
    pub tax_breakdown: Vec<TaxBreakdownItem>,
    /// Appendices rendered after the invoice body, each on its own page(s).
    /// Typically an event-level detail table backing an aggregated usage line.
    pub annexes: Vec<Annex>,
    /// The prices billed were quoted tax-included. Every amount on the model is net
    /// regardless (the structured e-invoice depends on that); a tax-inclusive document
    /// shows the customer the quoted figures each line carries in `InvoiceLine::quoted`,
    /// with the tax listed as already included rather than added on.
    pub tax_inclusive: bool,
    /// The invoice sells currency credit, on which tax is charged when the credit is used.
    pub prepaid_credit_purchase: bool,
}

/// An appendix rendered after the invoice body. The content is a pre-resolved
/// snapshot (already projected/transformed/grouped) — the renderer only lays it
/// out, it performs no aggregation or querying.
pub struct Annex {
    pub title: String,
    /// Optional free-text introduction shown under the title.
    pub description: Option<String>,
    pub table: AnnexTable,
}

pub struct AnnexTable {
    /// Column headers, in display order.
    pub columns: Vec<String>,
    /// Row groups. A flat (ungrouped) table is a single group with no heading.
    /// Grouping is purely for display — rows are listed individually, never aggregated.
    pub groups: Vec<AnnexGroup>,
}

pub struct AnnexGroup {
    /// Heading spanning the full width above the group's rows (e.g. the client name).
    pub heading: Option<String>,
    pub rows: Vec<AnnexRow>,
}

pub struct AnnexRow {
    /// One typed cell per column, aligned with `AnnexTable::columns`. Formatting (currency,
    /// localized dates) is applied by the renderer using the invoice's language and currency.
    pub cells: Vec<AnnexCell>,
    /// Subtotal / grand-total rows are rendered emphasized (bold + filled).
    pub emphasis: bool,
}

/// A typed annex cell. `Money` is in minor currency units; dates carry their strftime pattern.
#[derive(Debug, Clone)]
pub enum AnnexCell {
    Empty,
    Text(String),
    Number(Decimal),
    Money(Decimal),
    Date {
        ts: NaiveDateTime,
        fmt: String,
    },
    DateRange {
        min: NaiveDateTime,
        max: NaiveDateTime,
        fmt: String,
    },
}

/// Whether the tax engine priced the supply as one to a taxable person, as recorded
/// on the breakdown. The place-of-supply article a document cites must match the basis
/// the tax was computed on, so this is read rather than re-derived from the customer
/// snapshot — which an operator can edit without the engine re-running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupplyBasis {
    Business,
    Consumer,
}

/// Credit-transfer instructions, structured. Only the account identifier is
/// mandatory once a transfer is announced (BR-61).
#[derive(Debug, Clone)]
pub struct PaymentMeans {
    /// Account identifier — IBAN where the account is one, else the local number (BT-84).
    pub account_identifier: String,
    /// True when `account_identifier` is an IBAN, which decides the scheme on the wire.
    pub is_iban: bool,
    pub bic: Option<String>,
    pub account_name: Option<String>,
    /// Remittance information the payer must quote (BT-83).
    pub reference: Option<String>,
}

pub enum TaxExemptionType {
    ReverseCharge,
    TaxExempt,
    NotRegistered,
    Export,
    /// Taxable in a part of the seller's own VAT territory where VAT does not apply
    /// (Guyane, Mayotte: CGI art. 294-1). Out of scope like an export, but it cites
    /// art. 294-1 — never a place-of-supply article, since the supply stays inside.
    NoVatTerritory,
}

/// What levy an engine-resolved breakdown row is. The renderer words it in the
/// document's language ("TVA", "USt.", "btw") instead of printing the stored
/// `name`, which is the engine's English. `None` — merchant-authored custom taxes
/// and rows persisted before the kind was recorded — keeps `name` verbatim: the
/// merchant chose that name, and an already-issued document must keep the label it
/// was issued with rather than have one guessed retroactively.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaxKind {
    Vat,
    Gst,
    Hst,
    Pst,
    Qst,
    SalesTax,
}

pub struct TaxBreakdownItem {
    /// Stored row label; the rendered label when `tax_kind` is `None`.
    pub name: String,
    /// See [`TaxKind`]. Decides the rendered row label; `name` is the fallback.
    pub tax_kind: Option<TaxKind>,
    pub rate: Decimal,
    pub amount: Price,
    /// Base the rate applied to (BT-116). Carried for the e-invoice VAT breakdown;
    /// the rendered document only shows `amount`.
    pub taxable_amount: Price,
    pub exemption_type: Option<TaxExemptionType>,
    /// Free-text legal mention justifying the exemption (BT-120). Required by
    /// EN 16931 for every category other than standard-rated.
    pub exemption_reason: Option<String>,
    /// Basis the engine priced this on; `None` on documents predating the record.
    pub supply_basis: Option<SupplyBasis>,
    /// The row of the currency credit sold on the document (see `InvoiceLine::prepaid_credit`).
    pub prepaid_credit: bool,
}

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
    /// BT-41 / BT-42 — who a buyer calls about the invoice.
    pub contact_name: Option<String>,
    pub contact_phone: Option<String>,
    pub tax_id: Option<String>,
    pub footer_info: Option<String>,
    pub footer_legal: Option<String>,
    pub accounting_currency: iso::Currency,
    pub exchange_rate: Option<Decimal>,
}

pub struct Customer {
    pub name: String,
    pub legal_number: Option<String>,
    /// BT-10 — the reference the buyer routes the invoice by (a Leitweg-ID for German
    /// public bodies).
    pub buyer_reference: Option<String>,
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
    pub discount: Price,
    /// Promotional credit drawn before tax, net like `discount`; it is already out of each
    /// line's `taxable_amount`.
    pub promotional_credit: Price,
    /// On a tax-inclusive invoice, `promotional_credit` in the tax-included figures the
    /// lines were quoted in.
    pub quoted_promotional_credit: Option<Price>,
    /// Already settled at issuance — customer credit applied to this invoice (BT-113).
    /// The amount due (BT-115) is `total_amount - prepaid_amount`.
    pub prepaid_amount: Price,
    pub currency: &'static iso::Currency,
    pub due_date: chrono::NaiveDate,
    pub memo: Option<String>,
    pub payment_url: Option<String>,
    pub flags: Flags,
    pub purchase_order: Option<String>,
    pub parent_invoice_number: Option<String>,
    pub parent_invoice_date: Option<chrono::NaiveDate>,
}

#[derive(Default)]
pub struct Flags {
    pub show_payment_status: Option<bool>,
    pub show_payment_info: Option<bool>,
    pub show_terms: Option<bool>,
    pub show_tax_info: Option<bool>,
    pub show_legal_info: Option<bool>,
    pub show_footer_custom_info: Option<bool>,
    pub whitelabel: Option<bool>,
}

type Price = meteroid_money::Money<'static, Currency>;

pub struct InvoiceLine {
    pub name: String,
    pub description: Option<String>,
    pub subtotal: Price,
    /// The base tax applied to: `subtotal` less the share of any discount or credit
    /// allocated to this line. Equals `subtotal` when nothing was allocated.
    pub taxable_amount: Price,
    pub quantity: Option<Decimal>,
    pub unit_price: Option<Price>,
    pub tax_rate: Decimal,
    pub start_date: chrono::NaiveDate,
    pub end_date: chrono::NaiveDate,
    pub is_prorated: bool,
    pub sub_lines: Vec<InvoiceSubLine>,
    /// Optional grouping heading (e.g. plan name) used to group lines on the rendered invoice,
    /// typically set for consolidated invoices that merge several subscriptions.
    pub group_label: Option<String>,
    /// The tax-included figures this line was quoted in, when it was billed from
    /// tax-inclusive prices and bore tax. Display only: `subtotal`, `unit_price` and
    /// `sub_lines` stay net for the e-invoice.
    pub quoted: Option<QuotedLine>,
    /// Usage priced in a credit unit: the money fields only carry what credits did not cover.
    pub credit: Option<LineCredit>,
    /// Of `subtotal - taxable_amount`, what promotional credit took off before tax (net); the
    /// rest is discount.
    pub promotional_credit: Option<Price>,
    /// Currency credit sold, taxed when it is used: stated under its own breakdown row.
    pub prepaid_credit: bool,
}

pub struct LineCredit {
    /// The unit's display names; its code is internal and never printed.
    pub name: String,
    pub plural_name: String,
    /// Scaled to the unit's precision, which is how they print.
    pub consumed: Decimal,
    pub covered: Decimal,
}

impl LineCredit {
    /// `amount` of the unit, by its name: singular for exactly one.
    pub fn quantity(&self, amount: Decimal) -> String {
        let name = if amount.abs() == Decimal::ONE {
            &self.name
        } else {
            &self.plural_name
        };
        format!("{amount} {name}")
    }
}

pub struct QuotedLine {
    pub subtotal: Price,
    pub unit_price: Option<Price>,
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
    /// What the coupon took off the quoted, tax-included prices, when `total` is only the
    /// net share of it.
    pub quoted_total: Option<Price>,
}
