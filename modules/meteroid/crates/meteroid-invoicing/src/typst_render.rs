use crate::errors::{InvoicingError, InvoicingResult};
use crate::model::*;
use chrono::prelude::*;
use derive_typst_intoval::{IntoDict, IntoValue};
use fluent_static::MessageBundle;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use rusty_money::{FormattableCurrency, iso};
use typst::foundations::{Bytes, Dict, IntoValue};
use typst::layout::PagedDocument;
use typst::text::Font;
use typst_as_lib::TypstEngine;

static INVOICE_CORE: &str = include_str!("../templates/invoice.typ");
static TEMPLATE_CORE: &str = include_str!("../templates/template.typ");
static MAIN_TEMPLATE: &str = include_str!("../templates/main.typ");

static INTER_VARIABLE_FONT: &[u8] = include_bytes!("../assets/fonts/Inter-Variable.ttf");
static WORDMARK_LOGO: &[u8] = include_bytes!("../assets/wordmark.svg");
static LOGO: &[u8] = include_bytes!("../assets/logo.png");

// Define message bundle for localization
#[allow(clippy::all)]
mod l10n {
    use fluent_static::message_bundle;

    #[message_bundle(
        resources = [
            ("l10n/en-US/invoice.ftl", "en-US"),
            ("l10n/fr-FR/invoice.ftl", "fr-FR"),
        ],
        default_language = "en-US")]
    pub struct InvoiceL10n;

    include!(concat!(env!("OUT_DIR"), "/i18n_data.rs"));

    pub fn get_country_local_name<'a>(lang: &str, country_code: &str) -> Option<&'a str> {
        let primary = LOCALES
            .get(lang)
            .and_then(|locale| locale.get(country_code).map(|v| v.as_str()));
        let fallback = || {
            LOCALES
                .get("en-US")
                .and_then(|locale| locale.get(country_code).map(|v| v.as_str()))
        };

        primary.or_else(fallback)
    }
}

#[derive(Debug, Clone, IntoValue, IntoDict)]
pub struct TypstAddress {
    pub line1: String,
    pub line2: Option<String>,
    pub city: Option<String>,
    pub country: Option<String>,
    pub state: Option<String>,
    pub zipcode: Option<String>,
}

impl TypstAddress {
    pub fn from_address(address: &Address, lang: &str) -> Self {
        let country_name = address
            .country
            .as_ref()
            .and_then(|code| l10n::get_country_local_name(lang, code).map(|name| name.to_string()));

        TypstAddress {
            line1: address.line1.clone().unwrap_or_default(),
            line2: address.line2.clone(),
            city: address.city.clone(),
            country: country_name, // Use the resolved name instead of code
            state: address.state.clone(),
            zipcode: address.zip_code.clone(),
        }
    }
}

#[derive(Debug, Clone, IntoValue, IntoDict)]
pub struct TypstOrganization {
    pub name: String,
    pub logo_src: Option<String>,
    pub legal_number: Option<String>,
    pub address: TypstAddress,
    pub email: Option<String>,
    pub tax_id: Option<String>,
    pub footer_info: Option<String>,
    pub footer_legal: Option<String>,
    pub currency_code: String,
    pub exchange_rate: Option<f64>,
    pub accounting_currency_code: Option<String>,
}

impl TypstOrganization {
    pub fn from_org_with_lang(org: &Organization, lang: &str) -> Self {
        let currency_code = org.accounting_currency.code().to_string();

        TypstOrganization {
            name: org.name.clone(),
            logo_src: org.logo_src.clone(),
            legal_number: org.legal_number.clone(),
            address: TypstAddress::from_address(&org.address, lang),
            email: org.email.clone(),
            tax_id: org.tax_id.clone(),
            footer_info: org.footer_info.clone(),
            footer_legal: org.footer_legal.clone(),
            currency_code: currency_code.clone(),
            exchange_rate: org.exchange_rate.and_then(|d| d.to_f64()),
            accounting_currency_code: Some(currency_code),
        }
    }
}

#[derive(Debug, Clone, IntoValue, IntoDict)]
pub struct TypstCustomer {
    pub name: String,
    pub legal_number: Option<String>,
    pub address: TypstAddress,
    pub email: Option<String>,
    pub tax_id: Option<String>,
}

impl TypstCustomer {
    pub fn from_customer_with_lang(customer: &Customer, lang: &str) -> Self {
        TypstCustomer {
            name: customer.name.clone(),
            legal_number: customer.legal_number.clone(),
            address: TypstAddress::from_address(&customer.address, lang),
            email: customer.email.clone(),
            tax_id: customer.tax_id.clone(),
        }
    }
}

#[derive(Debug, Clone, IntoValue, IntoDict)]
pub struct TypstInvoiceLine {
    pub name: String,
    pub description: Option<String>,
    pub quantity: Option<f64>,
    pub unit_price: Option<f64>,
    pub vat_rate: Option<f64>,
    pub subtotal: f64,
    pub start_date: String,
    pub end_date: String,
    pub sub_lines: Vec<TypstInvoiceSubLine>,
}

impl From<&InvoiceLine> for TypstInvoiceLine {
    fn from(line: &InvoiceLine) -> Self {
        let start_date = line.start_date.format("%Y-%m-%d").to_string();
        let end_date = line.end_date.format("%Y-%m-%d").to_string();

        TypstInvoiceLine {
            name: line.name.clone(),
            description: line.description.clone(),
            quantity: line.quantity.and_then(|d| d.to_f64()),
            unit_price: line.unit_price.as_ref().and_then(|d| d.amount().to_f64()),
            vat_rate: line.tax_rate.to_f64().map(|r| r * 100.0),
            subtotal: line.subtotal.amount().to_f64().unwrap_or(0.0),
            start_date,
            end_date,
            sub_lines: {
                let mut sub_lines = Vec::with_capacity(line.sub_lines.len());
                for sub_line in &line.sub_lines {
                    sub_lines.push(TypstInvoiceSubLine::from(sub_line));
                }
                sub_lines
            },
        }
    }
}

#[derive(Debug, Clone, IntoValue, IntoDict)]
pub struct TypstInvoiceSubLine {
    pub name: String,
    pub quantity: f64,
    pub unit_price: f64,
    pub total: f64,
}

impl From<&InvoiceSubLine> for TypstInvoiceSubLine {
    fn from(sub_line: &InvoiceSubLine) -> Self {
        let quantity = sub_line.quantity.to_f64().unwrap_or(0.0);
        let unit_price = sub_line.unit_price.amount().to_f64().unwrap_or(0.0);

        TypstInvoiceSubLine {
            name: sub_line.name.clone(),
            quantity,
            unit_price,
            total: sub_line.total.amount().to_f64().unwrap_or(0.0),
        }
    }
}

#[derive(Debug, Clone, IntoValue, IntoDict)]
pub struct TypstTransaction {
    pub method: String,
    pub date: String,
    pub amount: f64,
}

impl From<&Transaction> for TypstTransaction {
    fn from(transaction: &Transaction) -> Self {
        TypstTransaction {
            method: transaction.method.clone(),
            date: transaction.date.format("%Y-%m-%d").to_string(),
            amount: transaction.amount.amount().to_f64().unwrap_or(0.0),
        }
    }
}

#[derive(Debug, Clone, IntoValue, IntoDict)]
pub struct TypstCoupon {
    pub name: String,
    pub total: f64,
}

impl From<&Coupon> for TypstCoupon {
    fn from(coupon: &Coupon) -> Self {
        TypstCoupon {
            name: coupon.name.clone(),
            total: coupon.total.amount().to_f64().unwrap_or(0.0),
        }
    }
}

#[derive(Debug, Clone, IntoValue, IntoDict)]
pub struct TypstTaxBreakdownItem {
    pub name: String,
    pub rate: f64,
    pub amount: f64,
    pub exemption_type: Option<String>, // Simplified to string for typst
}

impl From<&TaxBreakdownItem> for TypstTaxBreakdownItem {
    fn from(item: &TaxBreakdownItem) -> Self {
        use crate::model::TaxExemptionType;
        let exemption_type = item.exemption_type.as_ref().map(|e| match e {
            TaxExemptionType::ReverseCharge => "reverse_charge".to_string(),
            TaxExemptionType::TaxExempt => "tax_exempt".to_string(),
            TaxExemptionType::NotRegistered => "not_registered".to_string(),
            TaxExemptionType::Other(s) => s.clone(),
        });
        TypstTaxBreakdownItem {
            name: item.name.clone(),
            rate: item.rate.to_f64().unwrap_or(0.0) * 100.0,
            amount: item.amount.amount().to_f64().unwrap_or(0.0),
            exemption_type,
        }
    }
}

#[derive(Debug, Clone, IntoValue, IntoDict)]
pub struct TypstInvoiceContent {
    pub lang: String,
    pub organization: TypstOrganization,
    pub customer: TypstCustomer,
    pub number: String,
    pub issue_date: String,
    pub due_date: String,
    pub subtotal: f64,
    // TODO add discounts
    pub tax_amount: f64,
    pub total_amount: f64,
    pub currency_code: String,
    pub currency_symbol: String,
    pub memo: Option<String>,
    pub payment_term: u32,
    pub lines: Vec<TypstInvoiceLine>,
    pub translations: Dict,
    pub formatted_currency: Dict,
    pub pay_online_url: Option<String>,
    pub footer_custom_message: Option<String>,
    pub payment_status: String,
    pub transactions: Vec<TypstTransaction>,
    pub payment_info: Option<Dict>,
    pub show_payment_status: bool,
    pub show_payment_info: bool,
    pub show_terms: bool,
    pub show_tax_info: bool,
    pub show_legal_info: bool,
    pub whitelabel: bool,
    pub coupons: Vec<TypstCoupon>,
    pub tax_breakdown: Vec<TypstTaxBreakdownItem>,
}

impl From<&Invoice> for TypstInvoiceContent {
    fn from(invoice: &Invoice) -> Self {
        let lang = match invoice.lang.as_str() {
            "fr" | "fr-FR" => "fr-FR",
            _ => "en-US",
        };

        let invoice_l10n = &l10n::InvoiceL10N::get(lang).unwrap_or(l10n::InvoiceL10N::default());

        let mut translations = typst::foundations::dict! {
            "invoice_title" => invoice_l10n.invoice_title().into_value(),
            "invoice_number" => invoice_l10n.invoice_number().into_value(),
            "issue_date" => invoice_l10n.issue_date().into_value(),
            "amount_due" => invoice_l10n.amount_due().into_value(),
            "due_date" => invoice_l10n.due_date().into_value(),
            "bill_from" => invoice_l10n.bill_from().into_value(),
            "bill_to" => invoice_l10n.bill_to().into_value(),
            "invoice_lines" => invoice_l10n.invoice_lines().into_value(),
            "description" => invoice_l10n.description().into_value(),
            "quantity" => invoice_l10n.quantity().into_value(),
            "unit_price" => invoice_l10n.unit_price().into_value(),
            "tax_rate" => invoice_l10n.tax_rate().into_value(),
            "tax" => invoice_l10n.tax().into_value(),
            "amount" => invoice_l10n.amount().into_value(),
            "subtotal" => invoice_l10n.subtotal().into_value(),
            "total_due" => invoice_l10n.total_due().into_value(),
            "legal_info" => invoice_l10n.legal_info().into_value(),
            "vat_exempt_legal" => invoice_l10n.vat_exempt_legal().into_value(),
            "payment_status" => invoice_l10n.payment_status().into_value(),
            "payment_status_paid" => invoice_l10n.payment_status_paid().into_value(),
            "payment_status_partially_paid" => invoice_l10n.payment_status_partially_paid().into_value(),
            "payment_status_unpaid" => invoice_l10n.payment_status_unpaid().into_value(),
            "payment_method" => invoice_l10n.payment_method().into_value(),
            "payment_date" => invoice_l10n.payment_date().into_value(),
            "payment_amount" => invoice_l10n.payment_amount().into_value(),
            "no_transactions" => invoice_l10n.no_transactions().into_value(),
            "payment_info_title" => invoice_l10n.payment_info_title().into_value(),
            "payment_terms_title" => invoice_l10n.payment_terms_title().into_value(),
            "payment_terms_text" => invoice_l10n.payment_terms_text(invoice.metadata.payment_term.to_string()).into_value(),
            "tax_info_title" => invoice_l10n.tax_info_title().into_value(),
            "tax_reverse_charge" => invoice_l10n.tax_reverse_charge().into_value(),
            "pay_online" => invoice_l10n.pay_online().into_value(),
            "vat_id" => invoice_l10n.vat_id().into_value(),
            "tax_breakdown_title" => invoice_l10n.tax_breakdown_title().into_value(),
            "vat_standard" => invoice_l10n.vat_standard().into_value(),
            "vat_reduced" => invoice_l10n.vat_reduced().into_value(),
            "vat_exempt_notice" => invoice_l10n.vat_exempt_notice().into_value(),
            "reverse_charge_notice" => invoice_l10n.reverse_charge_notice().into_value(),
            "intra_eu_notice" => invoice_l10n.intra_eu_notice().into_value(),
            "b2b_notice" => invoice_l10n.b_2_b_notice().into_value(),
            "eu_vat_directive_notice" => invoice_l10n.eu_vat_directive_notice().into_value(),
            "late_payment_interest" => invoice_l10n.late_payment_interest().into_value(),
            "company_registration" => invoice_l10n.company_registration().into_value(),
        };

        if let Some(exchange_rate) = invoice.organization.exchange_rate {
            let date = format_date(lang, &invoice.metadata.issue_date)
                .unwrap_or_else(|_| invoice.metadata.issue_date.format("%Y-%m-%d").to_string());

            let equality = format!(
                "1 {} = {} {}",
                invoice.metadata.currency.code(),
                exchange_rate,
                invoice.organization.accounting_currency.code()
            );

            let amount_converted = format_currency_dec(
                invoice.metadata.total_amount.amount() * exchange_rate,
                &invoice.organization.accounting_currency,
            );

            translations.insert(
                "exchange_rate_info".into(),
                invoice_l10n
                    .exchange_rate_info(&date, &equality, &amount_converted)
                    .into_value(),
            );
        }

        let mut formatted_currency = Dict::new();

        let currency_symbol = invoice.metadata.currency.symbol();
        formatted_currency.insert("symbol".into(), currency_symbol.into_value());

        let formatted_issue_date = format_date(lang, &invoice.metadata.issue_date)
            .unwrap_or_else(|_| invoice.metadata.issue_date.format("%Y-%m-%d").to_string());

        let formatted_due_date = format_date(lang, &invoice.metadata.due_date)
            .unwrap_or_else(|_| invoice.metadata.due_date.format("%Y-%m-%d").to_string());

        let payment_info = if let Some(info) = &invoice.bank_details {
            let mut info_dict = Dict::new();
            for (key, value) in info {
                info_dict.insert(key.clone().into(), value.clone().into_value());
            }
            Some(info_dict)
        } else {
            None
        };

        let mut lines = Vec::with_capacity(invoice.lines.len());
        for line in &invoice.lines {
            lines.push(TypstInvoiceLine::from(line));
        }

        let mut transactions = Vec::with_capacity(invoice.transactions.len());
        for transaction in &invoice.transactions {
            transactions.push(TypstTransaction::from(transaction));
        }

        let mut coupons = Vec::with_capacity(invoice.coupons.len());
        for coupon in &invoice.coupons {
            coupons.push(TypstCoupon::from(coupon));
        }
        let mut tax_breakdown = Vec::with_capacity(invoice.tax_breakdown.len());
        for tax_breakdown_item in &invoice.tax_breakdown {
            tax_breakdown.push(TypstTaxBreakdownItem::from(tax_breakdown_item));
        }

        let currency_code = invoice.metadata.currency.code().to_string();

        let payment_status = invoice
            .payment_status
            .clone()
            .unwrap_or(PaymentStatus::Unpaid)
            .as_template_string();

        let subtotal = invoice.metadata.subtotal.amount().to_f64().unwrap_or(0.0);
        let tax_amount = invoice.metadata.tax_amount.amount().to_f64().unwrap_or(0.0);
        let total_amount = invoice
            .metadata
            .total_amount
            .amount()
            .to_f64()
            .unwrap_or(0.0);

        TypstInvoiceContent {
            lang: invoice.lang.clone(),
            // Use the new methods with language parameter
            organization: TypstOrganization::from_org_with_lang(&invoice.organization, lang),
            customer: TypstCustomer::from_customer_with_lang(&invoice.customer, lang),
            number: invoice.metadata.number.clone(),
            issue_date: formatted_issue_date,
            due_date: formatted_due_date,
            subtotal,
            tax_amount,
            total_amount,
            currency_code,
            currency_symbol: currency_symbol.to_string(),
            memo: invoice.metadata.memo.clone(),
            payment_term: invoice.metadata.payment_term,
            lines,
            translations,
            formatted_currency,
            pay_online_url: invoice.metadata.payment_url.clone(),
            footer_custom_message: None,
            payment_status,
            transactions,
            coupons,
            payment_info,
            // Use unwrap_or directly instead of unwrap_or_else for simple booleans
            show_payment_status: invoice.metadata.flags.show_payment_status.unwrap_or(true),
            show_payment_info: invoice.metadata.flags.show_payment_info.unwrap_or(true),
            show_terms: invoice.metadata.flags.show_terms.unwrap_or(false),
            show_tax_info: invoice.metadata.flags.show_tax_info.unwrap_or(false),
            show_legal_info: invoice.metadata.flags.show_legal_info.unwrap_or(true),
            whitelabel: invoice.metadata.flags.whitelabel.unwrap_or(false),
            tax_breakdown,
        }
    }
}

#[inline]
fn format_date(lang: &str, date: &NaiveDate) -> Result<String, InvoicingError> {
    match lang {
        "fr-FR" => Ok(date.format_localized("%e %B %Y", Locale::fr_FR).to_string()),
        _ => Ok(date.format("%B %e, %Y").to_string()),
    }
}

#[inline]
fn format_currency_dec(amount: Decimal, currency: &iso::Currency) -> String {
    rusty_money::Money::from_decimal(amount, currency).to_string()
}

pub struct TypstInvoiceRenderer {
    engine: TypstEngine,
}

impl TypstInvoiceRenderer {
    pub fn new() -> Result<Self, InvoicingError> {
        let font = Font::new(Bytes::new(INTER_VARIABLE_FONT), 0).ok_or(
            InvoicingError::I18nError("Failed to load Inter variable font".to_string()),
        )?;

        let engine = TypstEngine::builder()
            .with_static_source_file_resolver([
                ("invoice.typ", INVOICE_CORE),
                ("template.typ", TEMPLATE_CORE),
                ("main.typ", MAIN_TEMPLATE),
            ])
            .with_static_file_resolver([("wordmark.svg", WORDMARK_LOGO)])
            .with_static_file_resolver([("logo.png", LOGO)])
            .fonts([font])
            .build();

        Ok(TypstInvoiceRenderer { engine })
    }

    pub fn render_invoice(&self, invoice: &Invoice) -> InvoicingResult<PagedDocument> {
        let invoice_content = TypstInvoiceContent::from(invoice);

        let result = self
            .engine
            .compile_with_input("main.typ", invoice_content.into_dict())
            .output
            .map_err(|e| {
                InvoicingError::InvoiceGenerationError(format!(
                    "Failed to compile Typst document: {:?}",
                    e
                ))
            })?;

        Ok(result)
    }
}
