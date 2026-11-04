use crate::errors::{InvoicingError, InvoicingResult};
use crate::format::{self, Price};
use crate::glyphs::FontCoverage;
use crate::model::{
    Address, Coupon, Customer, Invoice, InvoiceLine, InvoiceSubLine, Organization, PaymentStatus,
    TaxBreakdownItem, TaxKind, Transaction,
};
use crate::typst_value::typst_dict_struct;
use chrono::prelude::*;
use fluent_static::MessageBundle;
use meteroid_money::{FormattableCurrency, iso};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use typst::foundations::{Bytes, Decimal as TypstDecimal, Dict, IntoValue};
use typst::layout::PagedDocument;
use typst::text::Font;
use typst_as_lib::TypstEngine;

static INVOICE_CORE: &str = include_str!("../templates/invoice.typ");
static TEMPLATE_CORE: &str = include_str!("../templates/template.typ");
static MAIN_TEMPLATE: &str = include_str!("../templates/main.typ");

static INTER_VARIABLE_FONT: &[u8] = include_bytes!("../assets/fonts/Inter-Variable.ttf");
static NOTO_SANS_HEBREW: &[u8] = include_bytes!("../assets/fonts/NotoSansHebrew-Regular.ttf");
static NOTO_SANS_ARABIC: &[u8] = include_bytes!("../assets/fonts/NotoSansArabic-Regular.ttf");
static NOTO_SANS_DEVANAGARI: &[u8] =
    include_bytes!("../assets/fonts/NotoSansDevanagari-Regular.ttf");
static NOTO_SANS_BENGALI: &[u8] = include_bytes!("../assets/fonts/NotoSansBengali-Regular.ttf");
static NOTO_SANS_THAI: &[u8] = include_bytes!("../assets/fonts/NotoSansThai-Regular.ttf");
static NOTO_SANS_TAMIL: &[u8] = include_bytes!("../assets/fonts/NotoSansTamil-Regular.ttf");
static NOTO_SANS_CJK: &[u8] = include_bytes!("../assets/fonts/NotoSansCJKsc-Regular.otf");
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
            ("l10n/de-DE/invoice.ftl", "de-DE"),
            ("l10n/es-ES/invoice.ftl", "es-ES"),
            ("l10n/it-IT/invoice.ftl", "it-IT"),
            ("l10n/nl-NL/invoice.ftl", "nl-NL"),
            ("l10n/pt-PT/invoice.ftl", "pt-PT"),
            ("l10n/pt-BR/invoice.ftl", "pt-BR"),
        ],
        default_language = "en-US")]
    pub struct InvoiceL10n;

    include!(concat!(env!("OUT_DIR"), "/i18n_data.rs"));

    pub fn get_country_local_name<'a>(lang: &str, country_code: &str) -> Option<&'a str> {
        let primary = LOCALES
            .get(lang)
            .and_then(|locale| locale.get(country_code).map(std::string::String::as_str));
        let fallback = || {
            LOCALES
                .get("en-US")
                .and_then(|locale| locale.get(country_code).map(std::string::String::as_str))
        };

        primary.or_else(fallback)
    }
}

typst_dict_struct! {
    #[derive(Debug, Clone)]
    pub struct TypstAddress {
        pub line1: String,
        pub line2: Option<String>,
        pub city: Option<String>,
        pub country: Option<String>,
        pub state: Option<String>,
        pub zipcode: Option<String>,
    }
}

impl TypstAddress {
    pub fn from_address(address: &Address, lang: &str) -> Self {
        let country_name = address
            .country
            .as_ref()
            .map(|code| l10n::get_country_local_name(lang, &code.code).unwrap_or(&code.name))
            .map(std::string::ToString::to_string);

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

typst_dict_struct! {
    #[derive(Debug, Clone)]
    pub struct TypstOrganization {
        pub name: String,
        pub logo_src: Option<Bytes>,
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
}

impl TypstOrganization {
    pub fn from_org_with_lang(org: Organization, lang: &str) -> Self {
        let currency_code = org.accounting_currency.code().to_string();
        TypstOrganization {
            name: org.name,
            logo_src: org.logo_src.map(Bytes::new),
            legal_number: org.legal_number,
            address: TypstAddress::from_address(&org.address, lang),
            email: org.email,
            tax_id: org.tax_id,
            footer_info: org.footer_info,
            footer_legal: org.footer_legal,
            currency_code: currency_code.clone(),
            exchange_rate: org.exchange_rate.and_then(|d| d.to_f64()),
            accounting_currency_code: Some(currency_code),
        }
    }
}

typst_dict_struct! {
    #[derive(Debug, Clone)]
    pub struct TypstCustomer {
        pub name: String,
        pub legal_number: Option<String>,
        pub address: TypstAddress,
        pub email: Option<String>,
        pub tax_id: Option<String>,
    }
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

typst_dict_struct! {
    #[derive(Debug, Clone)]
    pub struct TypstInvoiceLine {
        pub name: String,
        pub description: Option<String>,
        pub quantity: Option<f64>,
        pub unit_price: Option<TypstDecimal>,
        pub vat_rate: Option<f64>,
        pub subtotal: TypstDecimal,
        pub start_date: String,
        pub end_date: String,
        pub is_prorated: bool,
        pub sub_lines: Vec<TypstInvoiceSubLine>,
        pub group_label: Option<String>,
        pub credit: Option<TypstLineCredit>,
    }
}

typst_dict_struct! {
    #[derive(Debug, Clone)]
    pub struct TypstLineCredit {
        pub consumed: String,
        pub covered: String,
    }
}

impl TryFrom<&InvoiceLine> for TypstInvoiceLine {
    type Error = InvoicingError;

    fn try_from(line: &InvoiceLine) -> InvoicingResult<Self> {
        let start_date = line.start_date.format("%Y-%m-%d").to_string();
        let end_date = line.end_date.format("%Y-%m-%d").to_string();

        // A tax-inclusive document shows the line as the customer was quoted it; the net
        // figures stay on the model for the e-invoice.
        let (subtotal, unit_price, model_sub_lines) = match &line.quoted {
            Some(quoted) => (&quoted.subtotal, &quoted.unit_price, &quoted.sub_lines),
            None => (&line.subtotal, &line.unit_price, &line.sub_lines),
        };

        let mut sub_lines = Vec::with_capacity(model_sub_lines.len());
        for sub_line in model_sub_lines {
            sub_lines.push(TypstInvoiceSubLine::try_from(sub_line)?);
        }

        Ok(TypstInvoiceLine {
            name: line.name.clone(),
            description: line.description.clone(),
            quantity: line.quantity.and_then(|d| d.to_f64()),
            unit_price: unit_price.as_ref().map(format::unit_price).transpose()?,
            vat_rate: (line.tax_rate * Decimal::from(100)).to_f64(),
            subtotal: format::money(subtotal)?,
            start_date,
            end_date,
            is_prorated: line.is_prorated,
            group_label: line.group_label.clone(),
            sub_lines,
            credit: line.credit.as_ref().map(|credit| TypstLineCredit {
                consumed: credit.quantity(credit.consumed),
                covered: credit.quantity(credit.covered),
            }),
        })
    }
}

typst_dict_struct! {
    #[derive(Debug, Clone)]
    pub struct TypstInvoiceSubLine {
        pub name: String,
        pub quantity: f64,
        pub unit_price: TypstDecimal,
        pub total: TypstDecimal,
    }
}

impl TryFrom<&InvoiceSubLine> for TypstInvoiceSubLine {
    type Error = InvoicingError;

    fn try_from(sub_line: &InvoiceSubLine) -> InvoicingResult<Self> {
        Ok(TypstInvoiceSubLine {
            name: sub_line.name.clone(),
            quantity: sub_line.quantity.to_f64().unwrap_or(0.0),
            unit_price: format::unit_price(&sub_line.unit_price)?,
            total: format::money(&sub_line.total)?,
        })
    }
}

typst_dict_struct! {
    #[derive(Debug, Clone)]
    pub struct TypstTransaction {
        pub method: String,
        pub date: String,
        pub amount: TypstDecimal,
    }
}

impl TryFrom<&Transaction> for TypstTransaction {
    type Error = InvoicingError;

    fn try_from(transaction: &Transaction) -> InvoicingResult<Self> {
        Ok(TypstTransaction {
            method: transaction.method.clone(),
            date: transaction.date.format("%Y-%m-%d").to_string(),
            amount: format::money(&transaction.amount)?,
        })
    }
}

typst_dict_struct! {
    #[derive(Debug, Clone)]
    pub struct TypstCoupon {
        pub name: String,
        pub total: TypstDecimal,
    }
}

impl TryFrom<&Coupon> for TypstCoupon {
    type Error = InvoicingError;

    fn try_from(coupon: &Coupon) -> InvoicingResult<Self> {
        Ok(TypstCoupon {
            name: coupon.name.clone(),
            total: format::money(coupon.quoted_total.as_ref().unwrap_or(&coupon.total))?,
        })
    }
}

typst_dict_struct! {
    #[derive(Debug, Clone)]
    pub struct TypstTaxBreakdownItem {
        pub name: String,
        pub rate: f64,
        pub amount: TypstDecimal,
        pub exemption_type: Option<String>, // Simplified to string for typst
        /// The merchant's own free-text legal mention for an exemption row. On the page
        /// it outranks the derived statutory wording — the precedence the EN 16931
        /// mapping already gives BT-120 — so the PDF and the embedded XML never state
        /// two different tax positions on one document.
        pub exemption_reason: Option<String>,
    }
}

impl TypstTaxBreakdownItem {
    /// `name` is the row label the caller worded for the document's language (see
    /// [`tax_row_name`]) — not `item.name`, which is the engine's English. The rest
    /// is a structural copy.
    fn from_item(item: &TaxBreakdownItem, name: String) -> InvoicingResult<Self> {
        use crate::model::TaxExemptionType;
        let exemption_type = item.exemption_type.as_ref().map(|e| match e {
            TaxExemptionType::ReverseCharge => "reverse_charge".to_string(),
            TaxExemptionType::TaxExempt => "tax_exempt".to_string(),
            TaxExemptionType::NotRegistered => "undetermined".to_string(),
            TaxExemptionType::Export => "export".to_string(),
            TaxExemptionType::NoVatTerritory => "no_vat_territory".to_string(),
        });
        Ok(TypstTaxBreakdownItem {
            name,
            rate: (item.rate * Decimal::from(100)).to_f64().unwrap_or(0.0),
            amount: format::money(&item.amount)?,
            exemption_type,
            exemption_reason: item.exemption_reason.clone(),
        })
    }
}

/// Words a breakdown row's label in the document's language.
///
/// The engine records *what* a resolved rate is ([`TaxKind`]); the renderer owns the
/// wording, because "VAT" is a word of the presentation language ("TVA", "USt.",
/// "btw"), not a name. A row without a kind keeps its stored name verbatim: merchant
/// custom taxes are named by the merchant, and rows persisted before the kind was
/// recorded must keep rendering the label they were issued with rather than have one
/// guessed retroactively.
///
/// `vat_name` comes resolved through the legal corpus rather than straight from the
/// bundle: it is the one row label that is jurisdiction-sensitive on top of
/// language-sensitive (the shared `de` bundle says "USt.", a Swiss seller's document
/// must say "MWST" — same mechanism as the MWST-Nr. registration label).
fn tax_row_name(item: &TaxBreakdownItem, l10n: &l10n::InvoiceL10N, vat_name: &str) -> String {
    match item.tax_kind {
        None => item.name.clone(),
        Some(TaxKind::Vat) => vat_name.to_string(),
        Some(TaxKind::Gst) => l10n.tax_name_gst().to_string(),
        Some(TaxKind::Hst) => l10n.tax_name_hst().to_string(),
        Some(TaxKind::Pst) => l10n.tax_name_pst().to_string(),
        Some(TaxKind::Qst) => l10n.tax_name_qst().to_string(),
        Some(TaxKind::SalesTax) => l10n.tax_name_sales_tax().to_string(),
    }
}

typst_dict_struct! {
    #[derive(Debug, Clone)]
    pub struct TypstAnnex {
        pub title: String,
        pub description: Option<String>,
        pub columns: Vec<String>,
        pub groups: Vec<TypstAnnexGroup>,
    }
}

fn typst_annex(annex: &crate::model::Annex, lang: &str, currency: &iso::Currency) -> TypstAnnex {
    TypstAnnex {
        title: annex.title.clone(),
        description: annex.description.clone(),
        columns: annex.table.columns.clone(),
        groups: annex
            .table
            .groups
            .iter()
            .map(|g| typst_annex_group(g, lang, currency))
            .collect(),
    }
}

typst_dict_struct! {
    #[derive(Debug, Clone)]
    pub struct TypstAnnexGroup {
        pub heading: Option<String>,
        pub rows: Vec<TypstAnnexRow>,
    }
}

fn typst_annex_group(
    group: &crate::model::AnnexGroup,
    lang: &str,
    currency: &iso::Currency,
) -> TypstAnnexGroup {
    TypstAnnexGroup {
        heading: group.heading.clone(),
        rows: group
            .rows
            .iter()
            .map(|r| typst_annex_row(r, lang, currency))
            .collect(),
    }
}

typst_dict_struct! {
    #[derive(Debug, Clone)]
    pub struct TypstAnnexRow {
        pub cells: Vec<String>,
        pub emphasis: bool,
    }
}

fn typst_annex_row(
    row: &crate::model::AnnexRow,
    lang: &str,
    currency: &iso::Currency,
) -> TypstAnnexRow {
    TypstAnnexRow {
        cells: row
            .cells
            .iter()
            .map(|c| format_annex_cell(c, lang, currency))
            .collect(),
        emphasis: row.emphasis,
    }
}

/// Formats a typed annex cell with the invoice's language and currency, so annex money and dates
/// match the rest of the document (currency symbol/separators, localized month names).
fn format_annex_cell(
    cell: &crate::model::AnnexCell,
    lang: &str,
    currency: &iso::Currency,
) -> String {
    use crate::model::AnnexCell;
    match cell {
        AnnexCell::Empty => String::new(),
        AnnexCell::Text(s) => s.clone(),
        AnnexCell::Number(d) => format_plain_number(*d),
        AnnexCell::Money(minor) => {
            let scale = Decimal::from(10u64.pow(currency.exponent()));
            format_currency_dec(minor / scale, currency)
        }
        AnnexCell::Date { ts, fmt } => format_annex_date(ts, fmt, lang),
        AnnexCell::DateRange { min, max, fmt } => {
            let lo = format_annex_date(min, fmt, lang);
            let hi = format_annex_date(max, fmt, lang);
            if lo == hi { lo } else { format!("{lo} - {hi}") }
        }
    }
}

fn format_plain_number(d: Decimal) -> String {
    if d.fract() == Decimal::ZERO {
        d.trunc().to_string()
    } else {
        d.round_dp(2).to_string()
    }
}

fn format_annex_date(ts: &NaiveDateTime, fmt: &str, lang: &str) -> String {
    // Annex date presets are date-only; format the date part (NaiveDate supports localization).
    crate::format::pattern_date(lang, &ts.date(), fmt)
}

typst_dict_struct! {
    #[derive(Debug, Clone)]
    pub struct TypstInvoiceContent {
        pub lang: String,
        pub organization: TypstOrganization,
        pub customer: TypstCustomer,
        pub number: String,
        pub issue_date: String,
        pub due_date: String,
        pub subtotal: TypstDecimal,
        // TODO add discounts
        pub tax_amount: TypstDecimal,
        pub total_amount: TypstDecimal,
        pub prepaid_amount: TypstDecimal,
        pub amount_due: TypstDecimal,
        pub currency_code: String,
        pub currency_symbol: String,
        pub memo: Option<String>,
        pub payment_term: u32,
        pub lines: Vec<TypstInvoiceLine>,
        pub translations: Dict,
        pub pay_online_url: Option<String>,
        pub payment_status: String,
        pub transactions: Vec<TypstTransaction>,
        pub payment_info: Option<Dict>,
        pub show_payment_status: bool,
        pub show_payment_info: bool,
        pub show_terms: bool,
        pub show_tax_info: bool,
        pub show_legal_info: bool,
        pub show_footer_custom_info: bool,
        pub whitelabel: bool,
        pub coupons: Vec<TypstCoupon>,
        pub tax_breakdown: Vec<TypstTaxBreakdownItem>,
        pub discount: TypstDecimal,
        pub promotional_credit: TypstDecimal,
        pub purchase_order: Option<String>,
        pub parent_invoice_number: Option<String>,
        pub parent_invoice_date: Option<String>,
        pub annexes: Vec<TypstAnnex>,
        /// Prices were quoted tax-included: lines, subtotal, discount, promotional credit and
        /// coupons above are the quoted figures, and `total_excl_tax` carries the net total
        /// the tax was carved out of.
        pub tax_inclusive: bool,
        pub total_excl_tax: Option<TypstDecimal>,
        pub prepaid_credit_purchase: bool,
    }
}

/// A credit-purchase line the platform writes. Its stored name is English; the document words it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreditPurchaseName {
    WalletTopUp,
    AutomaticTopUp,
    Purchase,
}

/// What a credit-purchase line funds, for its description.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreditPurchaseDetail<'a> {
    /// Currency credit into a wallet, with no expiry.
    CurrencyWallet { currency: &'a str },
    /// Currency credit lapsing some months after payment.
    ValidMonths(u32),
    /// Credits of a unit, named by its plural.
    Units {
        credits: Decimal,
        unit: &'a str,
        valid_months: Option<u32>,
    },
}

pub fn credit_purchase_name(lang: &str, name: CreditPurchaseName) -> String {
    let lang = crate::lang::template_lang(lang);
    let l10n = l10n::InvoiceL10N::get(lang).unwrap_or(l10n::InvoiceL10N::default());
    match name {
        CreditPurchaseName::WalletTopUp => l10n.credit_purchase_wallet_top_up(),
        CreditPurchaseName::AutomaticTopUp => l10n.credit_purchase_automatic_top_up(),
        CreditPurchaseName::Purchase => l10n.credit_purchase(),
    }
    .to_string()
}

/// Without Fluent's bidi isolation marks: the text is also the e-invoice line's (BT-127).
pub fn credit_purchase_description(lang: &str, detail: CreditPurchaseDetail<'_>) -> String {
    let lang = crate::lang::template_lang(lang);
    let l10n = l10n::InvoiceL10N::get(lang).unwrap_or(l10n::InvoiceL10N::default());
    let text = match detail {
        CreditPurchaseDetail::CurrencyWallet { currency } => {
            l10n.credit_purchase_currency_wallet(currency).to_string()
        }
        CreditPurchaseDetail::ValidMonths(months) => l10n
            .credit_purchase_valid_months(months.to_string())
            .to_string(),
        CreditPurchaseDetail::Units {
            credits,
            unit,
            valid_months,
        } => {
            let credits = format::grouped_number(lang, credits);
            match valid_months {
                Some(months) => l10n
                    .credit_purchase_units_valid_months(credits, unit, months.to_string())
                    .to_string(),
                None => format!("{credits} {unit}"),
            }
        }
    };
    text.replace(['\u{2068}', '\u{2069}'], "")
}

/// The statutory mentions for `invoice`, in its own presentation language.
///
/// Extracted from the renderer so the EN 16931 mapping resolves BT-120 from the *same*
/// decision the PDF prints. In Factur-X the XML is the authoritative half of the
/// document, so the two must not state the tax position differently.
pub(crate) fn resolve_legal_mentions(invoice: &Invoice) -> crate::legal::LegalMentions {
    let lang = crate::lang::template_lang(&invoice.lang);
    let invoice_l10n = &l10n::InvoiceL10N::get(lang).unwrap_or(l10n::InvoiceL10N::default());
    let seller_jur = common_domain::country::TaxJurisdiction::of(
        invoice
            .organization
            .address
            .country
            .as_ref()
            .map(|c| c.code.as_str()),
    );

    // The basis the tax engine priced on, where the breakdown records it. Falling
    // back to the customer snapshot is for documents written before it did — never
    // a preference, because the snapshot is editable and the engine's finding is not.
    let customer_kind = invoice
        .tax_breakdown
        .iter()
        .find_map(|item| item.supply_basis)
        .map_or_else(
            || crate::legal::CustomerKind::from_tax_id(invoice.customer.tax_id.as_deref()),
            |basis| match basis {
                crate::model::SupplyBasis::Business => crate::legal::CustomerKind::Business,
                crate::model::SupplyBasis::Consumer => crate::legal::CustomerKind::Consumer,
            },
        );

    crate::legal::LegalMentions::resolve(
        seller_jur,
        customer_kind,
        crate::legal::LegalCandidates {
            national: crate::legal::NationalCandidates {
                france: crate::legal::NationalCitations {
                    export_notice_b2b: Some(invoice_l10n.export_notice_fr().to_string()),
                    export_notice_b2c: Some(invoice_l10n.export_notice_fr_b_2_c().to_string()),
                    ..Default::default()
                },
                belgium: crate::legal::NationalCitations {
                    export_notice_b2b: Some(invoice_l10n.export_notice_be().to_string()),
                    export_notice_b2c: Some(invoice_l10n.export_notice_be_b_2_c().to_string()),
                    ..Default::default()
                },
                // Italy mandates "operazione non soggetta" with the national article
                // (DPR 633/1972 art. 21 comma 6-bis). No settled B2C citation, so that
                // case falls through to the directive wording rather than inventing one.
                italy: crate::legal::NationalCitations {
                    export_notice_b2b: Some(invoice_l10n.export_notice_it().to_string()),
                    // Art. 21 comma 6-bis DPR 633/1972 mandates the annotation
                    // "operazione non soggetta" as the statement itself.
                    vat_not_applicable: Some(invoice_l10n.vat_not_applicable_it().to_string()),
                    ..Default::default()
                },
                // Swiss place-of-supply (art. 8 al. 1 MWSTG/LTVA) is customer-agnostic —
                // the "Wohnort" limb is the private-person case — so the same citation
                // serves both. The registration number is an MWST-Nr., and the statutory
                // category is "not taxable", neither of which the language bundle knows.
                switzerland: crate::legal::NationalCitations {
                    export_notice_b2b: Some(invoice_l10n.export_notice_ch().to_string()),
                    export_notice_b2c: Some(invoice_l10n.export_notice_ch().to_string()),
                    seller_tax_label: Some(invoice_l10n.seller_vat_id_ch().to_string()),
                    vat_not_applicable: Some(invoice_l10n.vat_not_applicable_ch().to_string()),
                    vat_name: Some(invoice_l10n.tax_name_vat_ch().to_string()),
                    ..Default::default()
                },
                // Austria contributes no citation, only its UID-Nummer label.
                austria: crate::legal::NationalCitations {
                    seller_tax_label: Some(invoice_l10n.seller_vat_id_at().to_string()),
                    ..Default::default()
                },
                // Portugal: CIVA art. 36 n.º 5 e) requires the reason tax was not applied
                // on the face of the invoice, so the national article is cited for both
                // supply bases. The registration number is the NIF (art. 36 n.º 5 a)).
                portugal: crate::legal::NationalCitations {
                    export_notice_b2b: Some(invoice_l10n.export_notice_pt().to_string()),
                    export_notice_b2c: Some(invoice_l10n.export_notice_pt_b_2_c().to_string()),
                    seller_tax_label: Some(invoice_l10n.seller_vat_id_pt().to_string()),
                    ..Default::default()
                },
                // Brazil operates no VAT, so it contributes no citation — only the
                // CNPJ, which is the registration number a Brazilian document names.
                brazil: crate::legal::NationalCitations {
                    seller_tax_label: Some(invoice_l10n.seller_tax_id_br().to_string()),
                    ..Default::default()
                },
            },
            vat_not_applicable: invoice_l10n.vat_not_applicable().to_string(),
            tax_not_applicable: invoice_l10n.tax_not_applicable().to_string(),
            vat_exempt_headline: invoice_l10n.vat_exempt_headline().to_string(),
            tax_exempt_headline: invoice_l10n.tax_exempt_headline().to_string(),
            export_notice_generic: invoice_l10n.export_notice_generic().to_string(),
            export_notice_generic_b2c: invoice_l10n.export_notice_generic_b_2_c().to_string(),
            export_notice_non_eu: invoice_l10n.export_notice_non_eu().to_string(),
            export_notice_no_vat: invoice_l10n.export_notice_no_vat().to_string(),
            no_vat_territory_notice: invoice_l10n.no_vat_territory_notice().to_string(),
            vat_exempt_notice: invoice_l10n.vat_exempt_notice().to_string(),
            tax_exempt_notice_generic: invoice_l10n.tax_exempt_notice_generic().to_string(),
            seller_vat_id: invoice_l10n.seller_vat_id().to_string(),
            seller_tax_id: invoice_l10n.seller_tax_id().to_string(),
            customer_vat_id: invoice_l10n.customer_vat_id().to_string(),
            customer_tax_id: invoice_l10n.customer_tax_id().to_string(),
            vat_name: invoice_l10n.tax_name_vat().to_string(),
            tax_included_text: invoice_l10n
                .tax_included_text(invoice.metadata.currency.code())
                .to_string(),
            tax_included_generic: invoice_l10n
                .tax_included_generic(invoice.metadata.currency.code())
                .to_string(),
            reverse_charge_notice: invoice_l10n.reverse_charge_notice().to_string(),
            reverse_charge_headline: invoice_l10n.tax_reverse_charge().to_string(),
            reverse_charge_headline_generic: invoice_l10n.tax_reverse_charge_generic().to_string(),
            reverse_charge_notice_generic: invoice_l10n.reverse_charge_notice_generic().to_string(),
        },
    )
}

impl TryFrom<&Invoice> for TypstInvoiceContent {
    type Error = InvoicingError;

    fn try_from(invoice: &Invoice) -> InvoicingResult<Self> {
        let lang = crate::lang::template_lang(&invoice.lang);

        let invoice_l10n = &l10n::InvoiceL10N::get(lang).unwrap_or(l10n::InvoiceL10N::default());

        let legal = resolve_legal_mentions(invoice);
        // Taken before the translations dict moves `legal` field-by-field.
        let vat_row_name = legal.vat_name_resolved.clone();

        let mut translations = typst::foundations::dict! {
            "invoice_title" => invoice_l10n.invoice_title().into_value(),
            "invoice_number" => invoice_l10n.invoice_number().into_value(),
            "issue_date" => invoice_l10n.issue_date().into_value(),
            "amount_due" => invoice_l10n.amount_due().into_value(),
            "nothing_to_pay" => invoice_l10n.nothing_to_pay().into_value(),
            "due_date" => invoice_l10n.due_date().into_value(),
            "due_label" => invoice_l10n.due_label().into_value(),
            "bill_from" => invoice_l10n.bill_from().into_value(),
            "bill_to" => invoice_l10n.bill_to().into_value(),
            "invoice_lines" => invoice_l10n.invoice_lines().into_value(),
            "description" => invoice_l10n.description().into_value(),
            "quantity" => invoice_l10n.quantity().into_value(),
            "unit_price" => invoice_l10n.unit_price().into_value(),
            "prorated" => invoice_l10n.prorated().into_value(),
            "credits_used" => invoice_l10n.credits_used().into_value(),
            "credits_covered" => invoice_l10n.credits_covered().into_value(),
            "tax_rate" => invoice_l10n.tax_rate().into_value(),
            "tax" => invoice_l10n.tax().into_value(),
            "amount" => invoice_l10n.amount().into_value(),
            "subtotal" => invoice_l10n.subtotal().into_value(),
            "total_due" => invoice_l10n.total_due().into_value(),
            "total" => invoice_l10n.total().into_value(),
            "credits_applied" => invoice_l10n.credits_applied().into_value(),
            "prepaid_credit_tax_note" => invoice_l10n.prepaid_credit_tax_note().into_value(),
            "legal_info" => invoice_l10n.legal_info().into_value(),
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
            "tax_reverse_charge" => legal.reverse_charge_headline_resolved.into_value(),
            "pay_online" => invoice_l10n.pay_online().into_value(),
            "seller_vat_id" => invoice_l10n.seller_vat_id().into_value(),
            "customer_vat_id" => invoice_l10n.customer_vat_id().into_value(),
            "vat_exempt_notice" => invoice_l10n.vat_exempt_notice().into_value(),
            "vat_not_applicable" => invoice_l10n.vat_not_applicable().into_value(),
            "vat_not_applicable_legal" => legal.vat_not_applicable_legal.into_value(),
            "vat_exempt_legal" => legal.vat_exempt_legal.into_value(),
            "export_notice_resolved" => legal.export_notice_resolved.into_value(),
            "no_vat_territory_notice_resolved" => legal.no_vat_territory_notice_resolved.into_value(),
            "vat_exempt_notice_resolved" => legal.vat_exempt_notice_resolved.into_value(),
            "seller_tax_label" => legal.seller_tax_label.into_value(),
            "customer_tax_label" => legal.customer_tax_label.into_value(),
            "tax_included_resolved" => legal.tax_included_resolved.into_value(),
            "reverse_charge_notice" => legal.reverse_charge_notice_resolved.into_value(),
            "company_registration" => invoice_l10n.company_registration().into_value(),
            "customer_registration" => invoice_l10n.customer_registration().into_value(),
            "discount" => invoice_l10n.discount().into_value(),
            "promotional_credit" => invoice_l10n.promotional_credit().into_value(),
            "purchase_order" => invoice_l10n.purchase_order().into_value(),
            "tax_included_text" => invoice_l10n.tax_included_text(invoice.metadata.currency.code()).into_value(),
            "no_tax_applied" => invoice_l10n.no_tax_applied().into_value(),
            "total_excl_tax" => invoice_l10n.total_excl_tax().into_value(),
            "tax_row_included" => invoice_l10n.tax_row_included().into_value()
        };

        if let (Some(parent_number), Some(parent_date)) = (
            &invoice.metadata.parent_invoice_number,
            &invoice.metadata.parent_invoice_date,
        ) {
            let formatted_parent_date = format_date(lang, parent_date)
                .unwrap_or_else(|_| parent_date.format("%Y-%m-%d").to_string());
            translations.insert(
                "cancels_and_replaces".into(),
                invoice_l10n
                    .cancels_and_replaces(parent_number, &formatted_parent_date)
                    .into_value(),
            );
        }

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

        let currency_symbol = invoice.metadata.currency.symbol();

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
            lines.push(TypstInvoiceLine::try_from(line)?);
        }

        let mut transactions = Vec::with_capacity(invoice.transactions.len());
        for transaction in &invoice.transactions {
            transactions.push(TypstTransaction::try_from(transaction)?);
        }

        let mut coupons = Vec::with_capacity(invoice.coupons.len());
        for coupon in &invoice.coupons {
            coupons.push(TypstCoupon::try_from(coupon)?);
        }
        let mut tax_breakdown = Vec::with_capacity(invoice.tax_breakdown.len());
        for tax_breakdown_item in &invoice.tax_breakdown {
            let name = tax_row_name(tax_breakdown_item, invoice_l10n, &vat_row_name);
            tax_breakdown.push(TypstTaxBreakdownItem::from_item(tax_breakdown_item, name)?);
        }

        let currency_code = invoice.metadata.currency.code().to_string();

        let payment_status = invoice
            .payment_status
            .clone()
            .unwrap_or(PaymentStatus::Unpaid)
            .as_template_string();

        let tax_amount = format::money(&invoice.metadata.tax_amount)?;
        let total_amount = format::money(&invoice.metadata.total_amount)?;
        let prepaid = (*invoice.metadata.prepaid_amount.amount()).clamp(
            Decimal::ZERO,
            (*invoice.metadata.total_amount.amount()).max(Decimal::ZERO),
        );
        let prepaid_amount =
            format::money(&Price::from_decimal(prepaid, invoice.metadata.currency))?;
        let amount_due = format::money(&Price::from_decimal(
            *invoice.metadata.total_amount.amount() - prepaid,
            invoice.metadata.currency,
        ))?;

        // On a tax-inclusive document the summary is in quoted figures too: the subtotal
        // is what the lines show, the discount and promotional credit are what separate it
        // from the total, and the net total gets its own row above the tax carved out of it.
        let (subtotal, discount, promotional_credit, total_excl_tax) = if invoice.tax_inclusive {
            let quoted_subtotal = invoice.lines.iter().fold(Decimal::ZERO, |sum, line| {
                sum + line
                    .quoted
                    .as_ref()
                    .map_or(&line.subtotal, |q| &q.subtotal)
                    .amount()
            });
            let currency = invoice.metadata.currency;
            let quoted_promotional_credit = *invoice
                .metadata
                .quoted_promotional_credit
                .as_ref()
                .unwrap_or(&invoice.metadata.promotional_credit)
                .amount();
            let quoted_discount = (quoted_subtotal
                - invoice.metadata.total_amount.amount()
                - quoted_promotional_credit)
                .max(Decimal::ZERO);
            let total_excl_tax =
                *invoice.metadata.total_amount.amount() - invoice.metadata.tax_amount.amount();
            (
                format::money(&Price::from_decimal(quoted_subtotal, currency))?,
                format::money(&Price::from_decimal(quoted_discount, currency))?,
                format::money(&Price::from_decimal(quoted_promotional_credit, currency))?,
                Some(format::money(&Price::from_decimal(
                    total_excl_tax,
                    currency,
                ))?),
            )
        } else {
            (
                format::money(&invoice.metadata.subtotal)?,
                format::money(&invoice.metadata.discount)?,
                format::money(&invoice.metadata.promotional_credit)?,
                None,
            )
        };

        Ok(TypstInvoiceContent {
            // Normalized, so the template's own language branches (amount layout) can
            // never disagree with the message bundle we actually resolved.
            lang: lang.to_string(),
            // Use the new methods with language parameter
            organization: TypstOrganization::from_org_with_lang(invoice.organization.clone(), lang),
            customer: TypstCustomer::from_customer_with_lang(&invoice.customer, lang),
            number: invoice.metadata.number.clone(),
            issue_date: formatted_issue_date,
            due_date: formatted_due_date,
            subtotal,
            tax_amount,
            total_amount,
            prepaid_amount,
            amount_due,
            currency_code,
            currency_symbol: currency_symbol.to_string(),
            memo: invoice.metadata.memo.clone(),
            payment_term: invoice.metadata.payment_term,
            lines,
            translations,
            pay_online_url: invoice.metadata.payment_url.clone(),
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
            show_footer_custom_info: invoice
                .metadata
                .flags
                .show_footer_custom_info
                .unwrap_or(true),
            whitelabel: invoice.metadata.flags.whitelabel.unwrap_or(false),
            tax_breakdown,
            discount,
            promotional_credit,
            purchase_order: invoice.metadata.purchase_order.clone(),
            parent_invoice_number: invoice.metadata.parent_invoice_number.clone(),
            parent_invoice_date: invoice.metadata.parent_invoice_date.map(|d| {
                format_date(lang, &d).unwrap_or_else(|_| d.format("%Y-%m-%d").to_string())
            }),
            annexes: invoice
                .annexes
                .iter()
                .map(|a| typst_annex(a, lang, invoice.metadata.currency))
                .collect(),
            tax_inclusive: invoice.tax_inclusive,
            total_excl_tax,
            prepaid_credit_purchase: invoice.prepaid_credit_purchase,
        })
    }
}

#[inline]
fn format_date(lang: &str, date: &NaiveDate) -> Result<String, InvoicingError> {
    Ok(crate::format::long_date(lang, date))
}

#[inline]
fn format_currency_dec(amount: Decimal, currency: &iso::Currency) -> String {
    meteroid_money::Money::from_decimal(amount, currency).to_string()
}

pub(crate) fn load_fonts() -> Result<Vec<Font>, InvoicingError> {
    fn load(data: &'static [u8], name: &str) -> Result<Font, InvoicingError> {
        Font::new(Bytes::new(data), 0)
            .ok_or_else(|| InvoicingError::I18nError(format!("Failed to load {name} font")))
    }

    Ok(vec![
        load(INTER_VARIABLE_FONT, "Inter")?,
        load(NOTO_SANS_HEBREW, "Noto Sans Hebrew")?,
        load(NOTO_SANS_ARABIC, "Noto Sans Arabic")?,
        load(NOTO_SANS_DEVANAGARI, "Noto Sans Devanagari")?,
        load(NOTO_SANS_BENGALI, "Noto Sans Bengali")?,
        load(NOTO_SANS_THAI, "Noto Sans Thai")?,
        load(NOTO_SANS_TAMIL, "Noto Sans Tamil")?,
        load(NOTO_SANS_CJK, "Noto Sans CJK")?,
    ])
}

/// The structured e-invoice to embed in the PDF.
///
/// `relationship` is legally load-bearing rather than descriptive: it states
/// whether the XML *is* the invoice or merely accompanies one, and published
/// guidance does not agree on it. `alternative` is what ZUGFeRD's own sources
/// and the Factur-X reference implementation use for the profiles that carry
/// lines, which is the only case we produce.
pub struct EInvoiceAttachment {
    pub filename: String,
    pub xml: Vec<u8>,
    pub description: String,
}

impl EInvoiceAttachment {
    pub(crate) fn to_value(&self) -> typst::foundations::Value {
        let mut dict = Dict::new();
        dict.insert("filename".into(), self.filename.clone().into_value());
        dict.insert("data".into(), Bytes::new(self.xml.clone()).into_value());
        dict.insert("relationship".into(), "alternative".into_value());
        dict.insert("description".into(), self.description.clone().into_value());
        dict.into_value()
    }
}

pub struct TypstInvoiceRenderer {
    engine: TypstEngine,
    coverage: FontCoverage,
}

impl TypstInvoiceRenderer {
    pub fn new() -> Result<Self, InvoicingError> {
        let fonts = load_fonts()?;
        let coverage = FontCoverage::new(&fonts);

        let engine = TypstEngine::builder()
            .with_static_source_file_resolver([
                ("invoice.typ", INVOICE_CORE),
                ("template.typ", TEMPLATE_CORE),
                ("main.typ", MAIN_TEMPLATE),
            ])
            .with_static_file_resolver([("wordmark.svg", WORDMARK_LOGO)])
            .with_static_file_resolver([("logo.png", LOGO)])
            .fonts(fonts)
            .build();

        Ok(TypstInvoiceRenderer { engine, coverage })
    }

    pub fn render_invoice(&self, invoice: &Invoice) -> InvoicingResult<PagedDocument> {
        self.render_invoice_with_attachment(invoice, None)
    }

    /// Renders the invoice, optionally embedding the structured e-invoice as an
    /// associated file — which is what makes the PDF a Factur-X / ZUGFeRD
    /// document rather than a PDF that happens to have an XML next to it.
    pub fn render_invoice_with_attachment(
        &self,
        invoice: &Invoice,
        attachment: Option<&EInvoiceAttachment>,
    ) -> InvoicingResult<PagedDocument> {
        let mut invoice_content = TypstInvoiceContent::try_from(invoice)?;
        // Sanitizing would drop an uncovered symbol (₭, ៛, ...) and leave bare amounts.
        if !self.coverage.renders(&invoice_content.currency_symbol) {
            tracing::warn!(
                currency = %invoice_content.currency_code,
                symbol = %invoice_content.currency_symbol,
                "no bundled font covers the currency symbol; falling back to the ISO code"
            );
            invoice_content.currency_symbol = invoice_content.currency_code.clone();
        }

        // Sanitization is for text the fonts must render; the payload is bytes and
        // the filename is fixed by the specification, so both go in after it.
        let mut inputs = self.coverage.sanitize_dict(invoice_content.into_dict());
        if let Some(attachment) = attachment {
            inputs.insert("einvoice".into(), attachment.to_value());
        }

        let result = self
            .engine
            .compile_with_input("main.typ", inputs)
            .output
            .map_err(|e| {
                InvoicingError::InvoiceGenerationError(format!(
                    "Failed to compile Typst document: {e:?}"
                ))
            })?;

        Ok(result)
    }
}

/// Flattens every glyph run of a rendered document back into plain text.
///
/// Both PDF and SVG output encode glyphs as paths, so this is the only way to assert
/// on — or debug — what actually reaches the page.
pub fn document_text(document: &PagedDocument) -> String {
    fn walk(frame: &typst::layout::Frame, out: &mut String) {
        for (_, item) in frame.items() {
            match item {
                typst::layout::FrameItem::Text(text) => out.push_str(&text.text),
                typst::layout::FrameItem::Group(group) => walk(&group.frame, out),
                _ => {}
            }
        }
    }

    let mut out = String::new();
    for page in &document.pages {
        walk(&page.frame, &mut out);
        out.push('\n');
    }
    out
}
