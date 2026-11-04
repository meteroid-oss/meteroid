use crate::credit_note_model::{
    Address, CreditNote, CreditNoteLine, CreditNoteSubLine, Customer, Organization,
    TaxBreakdownItem, TaxExemptionType, TaxKind,
};
use crate::errors::{InvoicingError, InvoicingResult};
use crate::format;
use crate::glyphs::FontCoverage;
use crate::typst_value::typst_dict_struct;
use fluent_static::MessageBundle;
use meteroid_money::{FormattableCurrency, iso};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use typst::foundations::{Bytes, Decimal as TypstDecimal, Dict, IntoValue};
use typst::layout::PagedDocument;
use typst_as_lib::TypstEngine;

static CREDIT_NOTE_CORE: &str = include_str!("../templates/credit_note.typ");
static TEMPLATE_CORE: &str = include_str!("../templates/template.typ");
static CREDIT_NOTE_MAIN_TEMPLATE: &str = include_str!("../templates/credit_note_main.typ");

static WORDMARK_LOGO: &[u8] = include_bytes!("../assets/wordmark.svg");
static LOGO: &[u8] = include_bytes!("../assets/logo.png");

// Use same localization as invoices
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
    pub struct CreditNoteL10n;

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
            country: country_name,
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
    pub struct TypstCreditNoteLine {
        pub name: String,
        pub description: Option<String>,
        pub quantity: Option<f64>,
        pub unit_price: Option<TypstDecimal>,
        pub vat_rate: Option<f64>,
        pub subtotal: TypstDecimal,
        pub start_date: String,
        pub end_date: String,
        pub is_prorated: bool,
        pub sub_lines: Vec<TypstCreditNoteSubLine>,
    }
}

impl TryFrom<&CreditNoteLine> for TypstCreditNoteLine {
    type Error = InvoicingError;

    fn try_from(line: &CreditNoteLine) -> InvoicingResult<Self> {
        let start_date = line.start_date.format("%Y-%m-%d").to_string();
        let end_date = line.end_date.format("%Y-%m-%d").to_string();

        let mut sub_lines = Vec::with_capacity(line.sub_lines.len());
        for sub_line in &line.sub_lines {
            sub_lines.push(TypstCreditNoteSubLine::try_from(sub_line)?);
        }

        Ok(TypstCreditNoteLine {
            name: line.name.clone(),
            description: line.description.clone(),
            quantity: line.quantity.and_then(|d| d.to_f64()),
            unit_price: line
                .unit_price
                .as_ref()
                .map(format::unit_price)
                .transpose()?,
            vat_rate: (line.tax_rate * Decimal::from(100)).to_f64(),
            subtotal: format::money(&line.subtotal)?,
            start_date,
            end_date,
            is_prorated: line.is_prorated,
            sub_lines,
        })
    }
}

typst_dict_struct! {
    #[derive(Debug, Clone)]
    pub struct TypstCreditNoteSubLine {
        pub name: String,
        pub quantity: f64,
        pub unit_price: TypstDecimal,
        pub total: TypstDecimal,
    }
}

impl TryFrom<&CreditNoteSubLine> for TypstCreditNoteSubLine {
    type Error = InvoicingError;

    fn try_from(sub_line: &CreditNoteSubLine) -> InvoicingResult<Self> {
        Ok(TypstCreditNoteSubLine {
            name: sub_line.name.clone(),
            quantity: sub_line.quantity.to_f64().unwrap_or(0.0),
            unit_price: format::unit_price(&sub_line.unit_price)?,
            total: format::money(&sub_line.total)?,
        })
    }
}

typst_dict_struct! {
    #[derive(Debug, Clone)]
    pub struct TypstTaxBreakdownItem {
        pub name: String,
        pub rate: f64,
        pub amount: TypstDecimal,
        pub exemption_type: Option<String>,
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

/// Words a breakdown row's label in the document's language — the credit-note twin
/// of `typst_render::tax_row_name`, duplicated because each renderer owns its own
/// generated bundle type. A row without a kind keeps its stored name verbatim
/// (merchant custom taxes, and rows persisted before the kind was recorded);
/// `vat_name` comes resolved through the legal corpus so the Swiss "MWST" override
/// follows the seller's jurisdiction.
fn tax_row_name(item: &TaxBreakdownItem, l10n: &l10n::CreditNoteL10N, vat_name: &str) -> String {
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
    pub struct TypstCreditNoteContent {
        pub lang: String,
        pub organization: TypstOrganization,
        pub customer: TypstCustomer,
        pub number: String,
        pub related_invoice_number: String,
        pub related_invoice_date: String,
        pub issue_date: String,
        pub subtotal: TypstDecimal,
        pub tax_amount: TypstDecimal,
        pub total_amount: TypstDecimal,
        pub currency_code: String,
        pub currency_symbol: String,
        pub reason: Option<String>,
        pub memo: Option<String>,
        pub credit_type: String,
        pub refunded_amount: TypstDecimal,
        pub credited_amount: TypstDecimal,
        pub debt_cancelled_amount: TypstDecimal,
        pub promotional_credit_restored: TypstDecimal,
        /// What takes the subtotal to the taxed base, in the note's own sign (positive on a note
        /// stated negative): coupons and discounts, then the promotional credit, as the lines'
        /// allowances in the e-invoice. Subtotal + discount + promotional credit + tax = total.
        pub discount: TypstDecimal,
        pub promotional_credit: TypstDecimal,
        pub lines: Vec<TypstCreditNoteLine>,
        pub tax_breakdown: Vec<TypstTaxBreakdownItem>,
        pub translations: Dict,
        pub show_tax_info: bool,
        pub show_legal_info: bool,
        pub show_footer_custom_info: bool,
        pub whitelabel: bool,
    }
}

/// The statutory mentions for `credit_note`, in its own presentation language. See
/// [`crate::typst_render::resolve_legal_mentions`] — same reason: the EN 16931 mapping
/// must derive BT-120 from the decision the document itself prints.
pub(crate) fn resolve_legal_mentions(credit_note: &CreditNote) -> crate::legal::LegalMentions {
    let lang = crate::lang::template_lang(&credit_note.lang);
    let credit_note_l10n =
        &l10n::CreditNoteL10N::get(lang).unwrap_or(l10n::CreditNoteL10N::default());
    let seller_jur = common_domain::country::TaxJurisdiction::of(
        credit_note
            .organization
            .address
            .country
            .as_ref()
            .map(|c| c.code.as_str()),
    );

    // The basis the tax engine priced on, where the breakdown records it. Falling
    // back to the customer snapshot is for documents written before it did — never
    // a preference, because the snapshot is editable and the engine's finding is not.
    let customer_kind = credit_note
        .tax_breakdown
        .iter()
        .find_map(|item| item.supply_basis)
        .map_or_else(
            || crate::legal::CustomerKind::from_tax_id(credit_note.customer.tax_id.as_deref()),
            |basis| match basis {
                crate::credit_note_model::SupplyBasis::Business => {
                    crate::legal::CustomerKind::Business
                }
                crate::credit_note_model::SupplyBasis::Consumer => {
                    crate::legal::CustomerKind::Consumer
                }
            },
        );

    crate::legal::LegalMentions::resolve(
        seller_jur,
        customer_kind,
        crate::legal::LegalCandidates {
            national: crate::legal::NationalCandidates {
                france: crate::legal::NationalCitations {
                    export_notice_b2b: Some(credit_note_l10n.export_notice_fr().to_string()),
                    export_notice_b2c: Some(credit_note_l10n.export_notice_fr_b_2_c().to_string()),
                    ..Default::default()
                },
                belgium: crate::legal::NationalCitations {
                    export_notice_b2b: Some(credit_note_l10n.export_notice_be().to_string()),
                    export_notice_b2c: Some(credit_note_l10n.export_notice_be_b_2_c().to_string()),
                    ..Default::default()
                },
                // Italy mandates "operazione non soggetta" with the national article
                // (DPR 633/1972 art. 21 comma 6-bis). No settled B2C citation, so that
                // case falls through to the directive wording rather than inventing one.
                italy: crate::legal::NationalCitations {
                    export_notice_b2b: Some(credit_note_l10n.export_notice_it().to_string()),
                    // Art. 21 comma 6-bis DPR 633/1972 mandates the annotation
                    // "operazione non soggetta" as the statement itself.
                    vat_not_applicable: Some(credit_note_l10n.vat_not_applicable_it().to_string()),
                    ..Default::default()
                },
                // Swiss place-of-supply (art. 8 al. 1 MWSTG/LTVA) is customer-agnostic —
                // the "Wohnort" limb is the private-person case — so the same citation
                // serves both. The registration number is an MWST-Nr., and the statutory
                // category is "not taxable", neither of which the language bundle knows.
                switzerland: crate::legal::NationalCitations {
                    export_notice_b2b: Some(credit_note_l10n.export_notice_ch().to_string()),
                    export_notice_b2c: Some(credit_note_l10n.export_notice_ch().to_string()),
                    seller_tax_label: Some(credit_note_l10n.seller_vat_id_ch().to_string()),
                    vat_not_applicable: Some(credit_note_l10n.vat_not_applicable_ch().to_string()),
                    vat_name: Some(credit_note_l10n.tax_name_vat_ch().to_string()),
                    ..Default::default()
                },
                // Austria contributes no citation, only its UID-Nummer label.
                austria: crate::legal::NationalCitations {
                    seller_tax_label: Some(credit_note_l10n.seller_vat_id_at().to_string()),
                    ..Default::default()
                },
                // Portugal: CIVA art. 36 n.º 5 e) requires the reason tax was not applied
                // on the face of the invoice, so the national article is cited for both
                // supply bases. The registration number is the NIF (art. 36 n.º 5 a)).
                portugal: crate::legal::NationalCitations {
                    export_notice_b2b: Some(credit_note_l10n.export_notice_pt().to_string()),
                    export_notice_b2c: Some(credit_note_l10n.export_notice_pt_b_2_c().to_string()),
                    seller_tax_label: Some(credit_note_l10n.seller_vat_id_pt().to_string()),
                    ..Default::default()
                },
                // Brazil operates no VAT, so it contributes no citation — only the
                // CNPJ, which is the registration number a Brazilian document names.
                brazil: crate::legal::NationalCitations {
                    seller_tax_label: Some(credit_note_l10n.seller_tax_id_br().to_string()),
                    ..Default::default()
                },
            },
            vat_not_applicable: credit_note_l10n.vat_not_applicable().to_string(),
            tax_not_applicable: credit_note_l10n.tax_not_applicable().to_string(),
            vat_exempt_headline: credit_note_l10n.vat_exempt_headline().to_string(),
            tax_exempt_headline: credit_note_l10n.tax_exempt_headline().to_string(),
            export_notice_generic: credit_note_l10n.export_notice_generic().to_string(),
            export_notice_generic_b2c: credit_note_l10n.export_notice_generic_b_2_c().to_string(),
            export_notice_non_eu: credit_note_l10n.export_notice_non_eu().to_string(),
            export_notice_no_vat: credit_note_l10n.export_notice_no_vat().to_string(),
            no_vat_territory_notice: credit_note_l10n.no_vat_territory_notice().to_string(),
            vat_exempt_notice: credit_note_l10n.vat_exempt_notice().to_string(),
            tax_exempt_notice_generic: credit_note_l10n.tax_exempt_notice_generic().to_string(),
            seller_vat_id: credit_note_l10n.seller_vat_id().to_string(),
            seller_tax_id: credit_note_l10n.seller_tax_id().to_string(),
            customer_vat_id: credit_note_l10n.customer_vat_id().to_string(),
            vat_name: credit_note_l10n.tax_name_vat().to_string(),
            customer_tax_id: credit_note_l10n.customer_tax_id().to_string(),
            tax_included_text: credit_note_l10n
                .tax_included_text(credit_note.metadata.currency.code())
                .to_string(),
            tax_included_generic: credit_note_l10n
                .tax_included_generic(credit_note.metadata.currency.code())
                .to_string(),
            reverse_charge_notice: credit_note_l10n
                .credit_note_reverse_charge_notice()
                .to_string(),
            reverse_charge_headline: credit_note_l10n.tax_reverse_charge().to_string(),
            reverse_charge_headline_generic: credit_note_l10n
                .tax_reverse_charge_generic()
                .to_string(),
            reverse_charge_notice_generic: credit_note_l10n
                .credit_note_reverse_charge_notice_generic()
                .to_string(),
        },
    )
}

impl TryFrom<&CreditNote> for TypstCreditNoteContent {
    type Error = InvoicingError;

    fn try_from(credit_note: &CreditNote) -> InvoicingResult<Self> {
        let lang = crate::lang::template_lang(&credit_note.lang);

        let credit_note_l10n =
            &l10n::CreditNoteL10N::get(lang).unwrap_or(l10n::CreditNoteL10N::default());

        let legal = resolve_legal_mentions(credit_note);
        // Taken before the translations dict moves `legal` field-by-field.
        let vat_row_name = legal.vat_name_resolved.clone();

        let mut translations = typst::foundations::dict! {
            // Credit note specific translations
            "credit_note_title" => credit_note_l10n.credit_note_title().into_value(),
            "credit_note_number" => credit_note_l10n.credit_note_number().into_value(),
            "related_invoice" => credit_note_l10n.related_invoice().into_value(),
            "related_invoice_value" => {
                let formatted_related_date = format_date(lang, &credit_note.metadata.related_invoice_date)
                    .unwrap_or_else(|_| credit_note.metadata.related_invoice_date.format("%Y-%m-%d").to_string());
                credit_note_l10n.related_invoice_value(&credit_note.metadata.related_invoice_number, &formatted_related_date).into_value()
            },
            "credit_to" => credit_note_l10n.credit_to().into_value(),
            "reason" => credit_note_l10n.reason().into_value(),
            "refund" => credit_note_l10n.refund().into_value(),
            "credit_to_balance" => credit_note_l10n.credit_to_balance().into_value(),
            "debt_cancellation" => credit_note_l10n.debt_cancellation().into_value(),
            // Issued before any refund is paid out, and never re-rendered after.
            "amount_to_refund" => credit_note_l10n.amount_to_refund().into_value(),
            "credited_amount" => credit_note_l10n.credited_amount().into_value(),
            "debt_cancelled_amount" => credit_note_l10n.debt_cancelled_amount().into_value(),
            "promotional_credit_restored" => if credit_note.metadata.promotional_credit_restored_estimated {
                credit_note_l10n.promotional_credit_to_restore().into_value()
            } else {
                credit_note_l10n.promotional_credit_restored().into_value()
            },
            "discount" => credit_note_l10n.discount().into_value(),
            "promotional_credit" => credit_note_l10n.promotional_credit().into_value(),
            "total_credit" => credit_note_l10n.total_credit().into_value(),
            // Shared translations
            "issue_date" => credit_note_l10n.issue_date().into_value(),
            "description" => credit_note_l10n.description().into_value(),
            "quantity" => credit_note_l10n.quantity().into_value(),
            "unit_price" => credit_note_l10n.unit_price().into_value(),
            "prorated" => credit_note_l10n.prorated().into_value(),
            "tax_rate" => credit_note_l10n.tax_rate().into_value(),
            "amount" => credit_note_l10n.amount().into_value(),
            "subtotal" => credit_note_l10n.subtotal().into_value(),
            "legal_info" => credit_note_l10n.legal_info().into_value(),
            "seller_vat_id" => credit_note_l10n.seller_vat_id().into_value(),
            "customer_vat_id" => credit_note_l10n.customer_vat_id().into_value(),
            "tax_info_title" => credit_note_l10n.tax_info_title().into_value(),
            "tax_reverse_charge" => legal.reverse_charge_headline_resolved.into_value(),
            "vat_exempt_notice" => credit_note_l10n.vat_exempt_notice().into_value(),
            "vat_not_applicable" => credit_note_l10n.vat_not_applicable().into_value(),
            "vat_not_applicable_legal" => legal.vat_not_applicable_legal.into_value(),
            "vat_exempt_legal" => legal.vat_exempt_legal.into_value(),
            "export_notice_resolved" => legal.export_notice_resolved.into_value(),
            "no_vat_territory_notice_resolved" => legal.no_vat_territory_notice_resolved.into_value(),
            "vat_exempt_notice_resolved" => legal.vat_exempt_notice_resolved.into_value(),
            "seller_tax_label" => legal.seller_tax_label.into_value(),
            "customer_tax_label" => legal.customer_tax_label.into_value(),
            "tax_included_resolved" => legal.tax_included_resolved.into_value(),
            "no_tax_applied" => credit_note_l10n.no_tax_applied().into_value(),
            "tax_included_text" => credit_note_l10n.tax_included_text(credit_note.metadata.currency.code()).into_value(),
            "company_registration" => credit_note_l10n.company_registration().into_value(),
            "customer_registration" => credit_note_l10n.customer_registration().into_value(),
            "reverse_charge_notice" => legal.reverse_charge_notice_resolved.into_value(),
        };

        if let Some(exchange_rate) = credit_note.organization.exchange_rate {
            let date = format_date(lang, &credit_note.metadata.issue_date).unwrap_or_else(|_| {
                credit_note
                    .metadata
                    .issue_date
                    .format("%Y-%m-%d")
                    .to_string()
            });

            let equality = format!(
                "1 {} = {} {}",
                credit_note.metadata.currency.code(),
                exchange_rate,
                credit_note.organization.accounting_currency.code()
            );

            let amount_converted = format_currency_dec(
                credit_note.metadata.total_amount.amount() * exchange_rate,
                &credit_note.organization.accounting_currency,
            );

            translations.insert(
                "exchange_rate_info".into(),
                credit_note_l10n
                    .exchange_rate_info(&date, &equality, &amount_converted)
                    .into_value(),
            );
        }

        let currency_symbol = credit_note.metadata.currency.symbol();

        let formatted_issue_date = format_date(lang, &credit_note.metadata.issue_date)
            .unwrap_or_else(|_| {
                credit_note
                    .metadata
                    .issue_date
                    .format("%Y-%m-%d")
                    .to_string()
            });

        let mut lines = Vec::with_capacity(credit_note.lines.len());
        for line in &credit_note.lines {
            lines.push(TypstCreditNoteLine::try_from(line)?);
        }

        let mut tax_breakdown = Vec::with_capacity(credit_note.tax_breakdown.len());
        for tax_breakdown_item in &credit_note.tax_breakdown {
            let name = tax_row_name(tax_breakdown_item, credit_note_l10n, &vat_row_name);
            tax_breakdown.push(TypstTaxBreakdownItem::from_item(tax_breakdown_item, name)?);
        }

        let currency_code = credit_note.metadata.currency.code().to_string();

        let subtotal = format::money(&credit_note.metadata.subtotal)?;
        let tax_amount = format::money(&credit_note.metadata.tax_amount)?;
        let total_amount = format::money(&credit_note.metadata.total_amount)?;
        let refunded_amount = format::money(&credit_note.metadata.refunded_amount)?;
        let credited_amount = format::money(&credit_note.metadata.credited_amount)?;
        let debt_cancelled_amount = format::money(&credit_note.metadata.debt_cancelled_amount)?;
        let promotional_credit_restored =
            format::money(&credit_note.metadata.promotional_credit_restored)?;

        let (discount, promotional_credit) = summary_reductions(credit_note);
        let discount = format::money(&Price::from_decimal(
            discount,
            credit_note.metadata.currency,
        ))?;
        let promotional_credit = format::money(&Price::from_decimal(
            promotional_credit,
            credit_note.metadata.currency,
        ))?;

        Ok(TypstCreditNoteContent {
            // Normalized, so the template's own language branches (amount layout) can
            // never disagree with the message bundle we actually resolved.
            lang: lang.to_string(),
            organization: TypstOrganization::from_org_with_lang(
                credit_note.organization.clone(),
                lang,
            ),
            customer: TypstCustomer::from_customer_with_lang(&credit_note.customer, lang),
            number: credit_note.metadata.number.clone(),
            related_invoice_number: credit_note.metadata.related_invoice_number.clone(),
            related_invoice_date: format_date(lang, &credit_note.metadata.related_invoice_date)
                .unwrap_or_else(|_| {
                    credit_note
                        .metadata
                        .related_invoice_date
                        .format("%Y-%m-%d")
                        .to_string()
                }),
            issue_date: formatted_issue_date,
            subtotal,
            tax_amount,
            total_amount,
            currency_code,
            currency_symbol: currency_symbol.to_string(),
            reason: credit_note.metadata.reason.clone(),
            memo: credit_note.metadata.memo.clone(),
            credit_type: credit_note.metadata.credit_type.as_template_string(),
            refunded_amount,
            credited_amount,
            debt_cancelled_amount,
            promotional_credit_restored,
            discount,
            promotional_credit,
            lines,
            tax_breakdown,
            translations,
            show_tax_info: credit_note.metadata.flags.show_tax_info.unwrap_or(true),
            show_legal_info: credit_note.metadata.flags.show_legal_info.unwrap_or(true),
            show_footer_custom_info: credit_note
                .metadata
                .flags
                .show_footer_custom_info
                .unwrap_or(true),
            whitelabel: credit_note.metadata.flags.whitelabel.unwrap_or(false),
        })
    }
}

type Price = meteroid_money::Money<'static, iso::Currency>;

/// The discount and the promotional credit between the subtotal and the tax, in the note's sign:
/// the lines' reductions, split as their e-invoice allowances are. The store sums the header
/// from the same lines, so subtotal + discount + promotional credit + tax = total.
fn summary_reductions(credit_note: &CreditNote) -> (Decimal, Decimal) {
    let (reduction, promotional) = credit_note.lines.iter().fold(
        (Decimal::ZERO, Decimal::ZERO),
        |(reduction, promotional), line| {
            let line_reduction = line.taxable_amount.amount() - line.subtotal.amount();
            // Within the line's reduction, so neither row turns negative.
            let line_promotional = (-line
                .promotional_credit
                .as_ref()
                .map_or(Decimal::ZERO, |c| *c.amount()))
            .clamp(
                line_reduction.min(Decimal::ZERO),
                line_reduction.max(Decimal::ZERO),
            );
            (reduction + line_reduction, promotional + line_promotional)
        },
    );
    (reduction - promotional, promotional)
}

#[inline]
fn format_date(lang: &str, date: &chrono::NaiveDate) -> Result<String, InvoicingError> {
    Ok(crate::format::long_date(lang, date))
}

#[inline]
fn format_currency_dec(amount: Decimal, currency: &iso::Currency) -> String {
    meteroid_money::Money::from_decimal(amount, currency).to_string()
}

pub struct TypstCreditNoteRenderer {
    engine: TypstEngine,
    coverage: FontCoverage,
}

impl TypstCreditNoteRenderer {
    pub fn new() -> Result<Self, InvoicingError> {
        let fonts = crate::typst_render::load_fonts()?;
        let coverage = FontCoverage::new(&fonts);

        let engine = TypstEngine::builder()
            .with_static_source_file_resolver([
                ("credit_note.typ", CREDIT_NOTE_CORE),
                ("template.typ", TEMPLATE_CORE),
                ("credit_note_main.typ", CREDIT_NOTE_MAIN_TEMPLATE),
            ])
            .with_static_file_resolver([("wordmark.svg", WORDMARK_LOGO)])
            .with_static_file_resolver([("logo.png", LOGO)])
            .fonts(fonts)
            .build();

        Ok(TypstCreditNoteRenderer { engine, coverage })
    }

    pub fn render_credit_note(&self, credit_note: &CreditNote) -> InvoicingResult<PagedDocument> {
        self.render_credit_note_with_attachment(credit_note, None)
    }

    /// Renders the credit note, optionally embedding the structured e-invoice as
    /// an associated file. See `TypstInvoiceRenderer::render_invoice_with_attachment`.
    pub fn render_credit_note_with_attachment(
        &self,
        credit_note: &CreditNote,
        attachment: Option<&crate::typst_render::EInvoiceAttachment>,
    ) -> InvoicingResult<PagedDocument> {
        let mut credit_note_content = TypstCreditNoteContent::try_from(credit_note)?;
        // Sanitizing would drop an uncovered symbol (₭, ៛, ...) and leave bare amounts.
        if !self.coverage.renders(&credit_note_content.currency_symbol) {
            tracing::warn!(
                currency = %credit_note_content.currency_code,
                symbol = %credit_note_content.currency_symbol,
                "no bundled font covers the currency symbol; falling back to the ISO code"
            );
            credit_note_content.currency_symbol = credit_note_content.currency_code.clone();
        }

        let mut inputs = self.coverage.sanitize_dict(credit_note_content.into_dict());
        if let Some(attachment) = attachment {
            inputs.insert("einvoice".into(), attachment.to_value());
        }

        let result = self
            .engine
            .compile_with_input("credit_note_main.typ", inputs)
            .output
            .map_err(|e| {
                InvoicingError::InvoiceGenerationError(format!(
                    "Failed to compile Typst credit note document: {e:?}"
                ))
            })?;

        Ok(result)
    }
}
