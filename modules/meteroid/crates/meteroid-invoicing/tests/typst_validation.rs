use chrono::NaiveDate;
use common_domain::country::CountryCode;
use meteroid_invoicing::credit_note_model::{
    self as cn, CreditNote, CreditNoteMetadata, CreditType,
};
use meteroid_invoicing::credit_note_render::{TypstCreditNoteContent, TypstCreditNoteRenderer};
use meteroid_invoicing::model::{
    Coupon, Flags, PaymentStatus, SupplyBasis, TaxBreakdownItem, TaxExemptionType, TaxKind,
};
use meteroid_invoicing::pdf::{CreditNotePdfGenerator, PdfGenerator, TypstCreditNotePdfGenerator};
use meteroid_invoicing::typst_render::{TypstInvoiceContent, TypstInvoiceRenderer};
use meteroid_invoicing::{
    model::{
        Address, Customer, Invoice, InvoiceLine, InvoiceMetadata, InvoiceSubLine, LineCredit,
        Organization, Transaction,
    },
    pdf::TypstPdfGenerator,
    typst_render::document_text,
};
use meteroid_money::{Money, iso};
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::str::FromStr;
use typst::layout::{Frame, FrameItem, PagedDocument};

#[tokio::test]
async fn test_typst_invoice_generation_validates_template() {
    let generator = TypstPdfGenerator::new().expect("Failed to create TypstPdfGenerator");
    let invoice = create_minimal_invoice();

    let result = generator.generate_pdf(&invoice, None).await;
    assert!(result.is_ok(), "Failed to generate PDF: {:?}", result.err());

    let pdf_data = result.unwrap();
    assert!(!pdf_data.is_empty(), "Generated PDF should not be empty");
    assert!(
        pdf_data.len() > 1000,
        "Generated PDF seems too small, likely an error"
    );
    assert!(
        &pdf_data[0..4] == b"%PDF",
        "Output should be a valid PDF file"
    );
}

#[tokio::test]
async fn test_typst_invoice_with_full_data() {
    let generator = TypstPdfGenerator::new().expect("Failed to create TypstPdfGenerator");
    let invoice = create_full_invoice();

    let result = generator.generate_pdf(&invoice, None).await;
    assert!(
        result.is_ok(),
        "Failed to generate PDF with full data: {:?}",
        result.err()
    );

    let pdf_data = result.unwrap();

    // let output_path = Path::new("benchmark_invoice.pdf");
    // std::fs::write(output_path, &pdf_data).unwrap();

    assert!(!pdf_data.is_empty(), "Generated PDF should not be empty");
    assert!(pdf_data.len() > 1000, "Generated PDF seems too small");
    assert!(
        &pdf_data[0..4] == b"%PDF",
        "Output should be a valid PDF file"
    );
}

#[tokio::test]
async fn test_typst_invoice_with_multiple_languages() {
    let generator = TypstPdfGenerator::new().expect("Failed to create TypstPdfGenerator");

    // Test English invoice
    let mut invoice_en = create_minimal_invoice();
    invoice_en.lang = "en-US".to_string();
    let result_en = generator.generate_pdf(&invoice_en, None).await;
    assert!(
        result_en.is_ok(),
        "Failed to generate English PDF: {:?}",
        result_en.err()
    );

    // Test French invoice
    let mut invoice_fr = create_minimal_invoice();
    invoice_fr.lang = "fr-FR".to_string();
    let result_fr = generator.generate_pdf(&invoice_fr, None).await;
    assert!(
        result_fr.is_ok(),
        "Failed to generate French PDF: {:?}",
        result_fr.err()
    );
}

/// Customer-supplied text used to abort the whole compilation whenever it carried a character
/// none of the bundled fonts covers (the word separator middle dot, here).
#[tokio::test]
async fn test_typst_invoice_with_characters_no_bundled_font_covers() {
    let generator = TypstPdfGenerator::new().expect("Failed to create TypstPdfGenerator");
    let mut invoice = create_minimal_invoice();
    invoice.customer.name = "Acme\u{2E31}Corp".to_string();
    invoice.lines[0].name = "API calls\u{2E31}EU\u{13000}".to_string();
    invoice.metadata.memo = Some("Merci\u{2E31}beaucoup".to_string());

    let result = generator.generate_pdf(&invoice, None).await;
    assert!(
        result.is_ok(),
        "Failed to generate PDF with uncovered characters: {:?}",
        result.err()
    );

    let renderer = TypstInvoiceRenderer::new().expect("Failed to create TypstInvoiceRenderer");
    let text = rendered_text(&renderer.render_invoice(&invoice).unwrap());

    assert!(text.contains("Acme\u{00B7}Corp"), "separator lost: {text}");
    assert!(
        text.contains("API calls\u{00B7}EU"),
        "separator lost: {text}"
    );
    assert!(
        !text.contains('\u{13000}'),
        "unmappable character reached the page: {text}"
    );
}

/// The Khmer riel sign is outside every bundled font, so amounts fall back to the ISO code
/// rather than losing their currency marker.
#[tokio::test]
async fn test_typst_invoice_with_a_currency_symbol_no_bundled_font_covers() {
    let khr = iso::find("KHR").unwrap();
    let mut invoice = create_minimal_invoice();
    invoice.metadata.currency = khr;
    invoice.metadata.subtotal = Money::from_major(100, khr);
    invoice.metadata.tax_amount = Money::from_major(20, khr);
    invoice.metadata.total_amount = Money::from_major(120, khr);
    invoice.metadata.discount = Money::from_major(0, khr);

    let renderer = TypstInvoiceRenderer::new().expect("Failed to create TypstInvoiceRenderer");
    let text = rendered_text(&renderer.render_invoice(&invoice).unwrap());

    assert!(text.contains("KHR"), "currency marker lost: {text}");
}

#[test]
fn test_sub_cent_unit_prices_are_not_rounded_to_the_currency_exponent() {
    let content = TypstInvoiceContent::try_from(&create_sub_cent_invoice()).unwrap();

    let metered = &content.lines[0];
    assert_eq!(
        metered.unit_price.map(|d| d.to_string()).as_deref(),
        Some("0.005")
    );
    assert_eq!(metered.subtotal.to_string(), "0.04");
    assert_eq!(metered.sub_lines[0].unit_price.to_string(), "0.005");
    assert_eq!(metered.sub_lines[0].total.to_string(), "0.04");

    let requests = &content.lines[1];
    assert_eq!(
        requests.unit_price.map(|d| d.to_string()).as_deref(),
        Some("0.0025")
    );
    assert_eq!(requests.subtotal.to_string(), "10.00");
}

/// The content struct is one step short of the page: this guards the template itself, which is
/// where the amounts used to be rounded before being printed.
#[test]
fn test_sub_cent_unit_prices_reach_the_laid_out_page() {
    let renderer = TypstInvoiceRenderer::new().expect("Failed to create TypstInvoiceRenderer");
    let document = renderer
        .render_invoice(&create_sub_cent_invoice())
        .expect("Failed to render invoice");

    let text = rendered_text(&document);

    assert!(text.contains("0.005"), "unit price rounded away: {text}");
    assert!(text.contains("0.0025"), "unit price rounded away: {text}");
    assert!(
        !text.contains("0.01"),
        "sub-cent price rendered as the currency minimum: {text}"
    );
    assert!(
        text.contains("10.00"),
        "amount not padded to the currency exponent: {text}"
    );
    assert!(
        text.contains("\u{00A0}0.0025"),
        "amount and currency symbol must stay on one line: {text}"
    );
}

/// Concatenates the text of a laid-out document, in frame order.
fn rendered_text(document: &PagedDocument) -> String {
    fn walk(frame: &Frame, out: &mut String) {
        for (_, item) in frame.items() {
            match item {
                FrameItem::Text(text) => out.push_str(&text.text),
                FrameItem::Group(group) => walk(&group.frame, out),
                _ => {}
            }
        }
    }

    let mut out = String::new();
    for page in &document.pages {
        walk(&page.frame, &mut out);
    }
    out
}

/// A usage invoice whose quantities, unit prices and totals multiply out, so the rendered
/// figures can be read as a coherent invoice: 8 x 0.005 + 4000 x 0.0025 = 10.04.
fn create_sub_cent_invoice() -> Invoice {
    let eur = iso::find("EUR").unwrap();
    let dec = |value: &str| Decimal::from_str(value).unwrap();
    let money = |value: &str| Money::from_decimal(dec(value), eur);

    let mut invoice = create_minimal_invoice();
    invoice.metadata.subtotal = money("10.04");
    invoice.metadata.tax_amount = money("0");
    invoice.metadata.total_amount = money("10.04");
    invoice.tax_breakdown = vec![];

    let template = invoice.lines.remove(0);
    invoice.lines = vec![
        InvoiceLine {
            name: "Metered events".to_string(),
            subtotal: money("0.04"),
            taxable_amount: money("0.04"),
            quantity: Some(dec("8")),
            unit_price: Some(money("0.005")),
            tax_rate: Decimal::ZERO,
            sub_lines: vec![InvoiceSubLine {
                name: "Ingested events".to_string(),
                total: money("0.04"),
                quantity: dec("8"),
                unit_price: money("0.005"),
            }],
            ..template
        },
        InvoiceLine {
            name: "API requests".to_string(),
            description: None,
            subtotal: money("10.00"),
            taxable_amount: money("10.00"),
            quantity: Some(dec("4000")),
            unit_price: Some(money("0.0025")),
            tax_rate: Decimal::ZERO,
            start_date: invoice.metadata.issue_date,
            end_date: invoice.metadata.issue_date,
            is_prorated: false,
            sub_lines: vec![],
            group_label: None,
            quoted: None,
            credit: None,
            promotional_credit: None,
            prepaid_credit: false,
        },
    ];

    invoice
}

#[test]
fn test_invoice_amounts_are_padded_to_the_currency_exponent() {
    let content = TypstInvoiceContent::try_from(&create_minimal_invoice()).unwrap();

    assert_eq!(content.subtotal.to_string(), "100.00");
    assert_eq!(content.tax_amount.to_string(), "20.00");
    assert_eq!(content.total_amount.to_string(), "120.00");
    assert_eq!(content.discount.to_string(), "0.00");
    assert_eq!(
        content.lines[0]
            .unit_price
            .map(|d| d.to_string())
            .as_deref(),
        Some("100.00")
    );
    assert_eq!(content.tax_breakdown[0].amount.to_string(), "20.00");
}

#[test]
fn test_credit_note_amounts_are_padded_to_the_currency_exponent() {
    let content = TypstCreditNoteContent::try_from(&create_minimal_credit_note()).unwrap();

    assert_eq!(content.subtotal.to_string(), "100.00");
    assert_eq!(content.total_amount.to_string(), "120.00");
    assert_eq!(content.credited_amount.to_string(), "120.00");
    assert_eq!(
        content.lines[0]
            .unit_price
            .map(|d| d.to_string())
            .as_deref(),
        Some("100.00")
    );
}

#[tokio::test]
async fn test_typst_invoice_with_sub_cent_unit_price_renders() {
    let generator = TypstPdfGenerator::new().expect("Failed to create TypstPdfGenerator");

    let result = generator
        .generate_pdf(&create_sub_cent_invoice(), None)
        .await;
    assert!(result.is_ok(), "Failed to generate PDF: {:?}", result.err());
}

/// Renders and flattens the page text, so a test can assert on what actually
/// reaches the page rather than just "a PDF came out".
fn render_text(invoice: &Invoice) -> String {
    let renderer = TypstInvoiceRenderer::new().expect("Failed to create TypstInvoiceRenderer");
    let document = renderer
        .render_invoice(invoice)
        .expect("Failed to render invoice");
    document_text(&document)
}

/// Same, for credit notes — so a test can assert on the statutory text a credit note
/// actually prints, not just that a PDF came out.
fn render_credit_note_text(credit_note: &CreditNote) -> String {
    let renderer =
        TypstCreditNoteRenderer::new().expect("Failed to create TypstCreditNoteRenderer");
    let document = renderer
        .render_credit_note(credit_note)
        .expect("Failed to render credit note");
    document_text(&document)
}

/// A tax-inclusive invoice shows the customer the price they were quoted — on the line,
/// in the subtotal and as the total — and lists the tax as already included in it, with
/// the net total on its own row. The model underneath stays net.
#[test]
fn a_tax_inclusive_invoice_shows_quoted_prices_and_marks_the_tax_as_included() {
    let eur = iso::find("EUR").unwrap();
    let money = |s: &str| Money::from_decimal(Decimal::from_str(s).unwrap(), eur);

    let mut invoice = create_minimal_invoice();
    invoice.tax_inclusive = true;
    // 120.00 quoted: the engine carved 20.00 out and stored 100.00 net.
    invoice.lines[0].quoted = Some(meteroid_invoicing::model::QuotedLine {
        subtotal: money("120"),
        unit_price: Some(money("120")),
        sub_lines: vec![],
    });

    let text = render_text(&invoice);
    assert!(
        text.contains("Total excluding tax"),
        "net total row: {text}"
    );
    assert!(
        text.contains("(included)"),
        "tax row marked as included: {text}"
    );
    let quoted = text.matches("120.00").count();
    let net = text.matches("100.00").count();
    assert!(
        quoted >= 3,
        "line, subtotal and total show the quoted 120.00: {text}"
    );
    assert_eq!(
        net, 1,
        "the net 100.00 appears once, as the total excluding tax: {text}"
    );

    // Nothing changes on a tax-exclusive document.
    let exclusive = render_text(&create_minimal_invoice());
    assert!(!exclusive.contains("Total excluding tax"));
    assert!(!exclusive.contains("(included)"));
}

/// Promotional credit comes off before tax, between the subtotal and the tax rows.
#[test]
fn a_promotional_credit_is_listed_before_the_tax() {
    let eur = iso::find("EUR").unwrap();
    let money = |s: &str| Money::from_decimal(Decimal::from_str(s).unwrap(), eur);

    let mut invoice = create_minimal_invoice();
    // 100.00 less 25.00 of promotional credit, taxed at 20% on the remaining 75.00.
    invoice.lines[0].taxable_amount = money("75");
    invoice.metadata.promotional_credit = money("25");
    invoice.metadata.tax_amount = money("15");
    invoice.metadata.total_amount = money("90");
    invoice.tax_breakdown[0].amount = money("15");
    invoice.tax_breakdown[0].taxable_amount = money("75");

    let text = render_text(&invoice);
    let credit = text
        .find("Promotional credit")
        .unwrap_or_else(|| panic!("promotional credit row: {text}"));
    let tax = text
        .rfind("VAT 20")
        .unwrap_or_else(|| panic!("tax row: {text}"));
    assert!(credit < tax, "the credit comes before the tax: {text}");
    assert!(text.contains("25.00"), "credit amount: {text}");
    assert!(!text.contains("Discount"), "no discount row: {text}");
}

/// On a tax-inclusive invoice the promotional credit is shown in the quoted figures, and
/// is not mistaken for a discount in what separates the quoted subtotal from the total.
#[test]
fn a_tax_inclusive_promotional_credit_is_shown_as_quoted() {
    let eur = iso::find("EUR").unwrap();
    let money = |s: &str| Money::from_decimal(Decimal::from_str(s).unwrap(), eur);

    let mut invoice = create_minimal_invoice();
    invoice.tax_inclusive = true;
    // 120.00 quoted less 30.00 of promotional credit leaves 90.00, of which 15.00 is tax.
    invoice.lines[0].quoted = Some(meteroid_invoicing::model::QuotedLine {
        subtotal: money("120"),
        unit_price: Some(money("120")),
        sub_lines: vec![],
    });
    invoice.lines[0].taxable_amount = money("75");
    invoice.metadata.promotional_credit = money("25");
    invoice.metadata.quoted_promotional_credit = Some(money("30"));
    invoice.metadata.tax_amount = money("15");
    invoice.metadata.total_amount = money("90");
    invoice.tax_breakdown[0].amount = money("15");
    invoice.tax_breakdown[0].taxable_amount = money("75");

    let text = render_text(&invoice);
    assert!(text.contains("Promotional credit"), "credit row: {text}");
    assert!(text.contains("30.00"), "quoted credit amount: {text}");
    assert!(
        !text.contains("25.00"),
        "the net share is not shown: {text}"
    );
    assert!(!text.contains("Discount"), "no discount row: {text}");
    assert!(text.contains("75.00"), "total excluding tax: {text}");
}

#[test]
fn usage_priced_in_a_credit_unit_prints_the_units_name_at_its_precision() {
    let mut invoice = create_minimal_invoice();
    invoice.lines[0].credit = Some(LineCredit {
        name: "Halyard credit".to_owned(),
        plural_name: "Halyard credits".to_owned(),
        consumed: Decimal::new(48_000, 2),
        covered: Decimal::new(100, 2),
    });
    let text = render_text(&invoice)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(text.contains("480.00 Halyard credits"), "{text}");
    assert!(text.contains("1.00 Halyard credit "), "{text}");
}

#[test]
fn an_invoice_selling_currency_credit_says_its_tax_is_charged_on_use() {
    let mut invoice = create_minimal_invoice();
    invoice.prepaid_credit_purchase = true;
    // Wrapped lines lose the space at the break, so compare without whitespace.
    let squeeze = |text: String| text.split_whitespace().collect::<String>();
    let note = squeeze("Tax on prepaid credit is charged when the credit is used.".to_owned());
    let text = squeeze(render_text(&invoice));
    assert!(text.contains(&note), "prepaid credit note: {text}");

    invoice.metadata.flags.show_tax_info = Some(false);
    let text = squeeze(render_text(&invoice));
    assert!(text.contains(&note), "note without the tax section: {text}");
    assert!(!squeeze(render_text(&create_minimal_invoice())).contains(&note));
}

#[test]
fn requested_language_selects_the_message_bundle() {
    let mut invoice = create_minimal_invoice();
    for (tag, title) in [
        ("fr-FR", "Facture"),
        ("de-DE", "Rechnung"),
        ("es-ES", "Factura"),
        ("it-IT", "Fattura"),
        ("nl-NL", "Factuur"),
    ] {
        invoice.lang = tag.to_string();
        let rendered = render_text(&invoice);
        assert!(rendered.contains(title), "{tag} should render {title}");
    }

    // Bare subtag is accepted too.
    invoice.lang = "fr".to_string();
    assert!(render_text(&invoice).contains("Facture"));
}

/// A Belgian or Swiss seller issues in a neighbouring language, so the regional tag
/// must reach that language's bundle rather than falling back to English.
#[test]
fn regional_tags_render_in_their_language() {
    let mut invoice = create_minimal_invoice();
    for (tag, title) in [
        ("nl-BE", "Factuur"),
        ("de-CH", "Rechnung"),
        ("it-CH", "Fattura"),
    ] {
        invoice.lang = tag.to_string();
        let rendered = render_text(&invoice);
        assert!(rendered.contains(title), "{tag} should render {title}");
        assert!(!rendered.contains("Invoice n"), "{tag} fell back to en-US");
    }
}

/// The seller's jurisdiction picks the statute, the document language words it: a
/// Belgian seller invoicing in Dutch must cite the Belgian VAT code, in Dutch.
///
/// The export branch prints two slots — a headline and a citation under it — so both
/// must rest on the same statutory basis. Asserting on each of them separately is what
/// catches a regime whose two strings drift onto contradictory articles.
#[test]
fn seller_jurisdiction_selects_the_national_citation() {
    let mut invoice = create_minimal_invoice();
    invoice.organization.address.country = CountryCode::parse_as_opt("BE");
    invoice.tax_breakdown[0].exemption_type = Some(TaxExemptionType::Export);
    // A customer tax ID is what makes this a B2B supply, and the place-of-supply
    // article the corpus cites turns on exactly that.
    invoice.customer.tax_id = Some("NL123456789B01".to_string());

    invoice.lang = "nl-NL".to_string();
    let dutch = render_text(&invoice);
    // The headline states the position; the citation under it carries the article.
    assert!(dutch.contains("Btw niet van toepassing"), "{dutch}");
    assert!(
        // Asserted in halves, so the separator convention can change without
        // this test needing to know which one is in force.
        dutch.contains("Plaats van de dienst buiten België")
            && dutch.contains("belastingplichtige afnemer")
            && dutch.contains("art. 21, § 2, W.Btw"),
        "{dutch}"
    );

    invoice.lang = "fr-BE".to_string();
    let walloon = render_text(&invoice);
    assert!(
        walloon.contains("art. 21, § 2, du Code de la TVA"),
        "{walloon}"
    );
    assert!(walloon.contains("Prestation de services localisée hors de Belgique"));

    // Switzerland cites its own act, and nothing EU-directive-worded leaks in.
    invoice.organization.address.country = CountryCode::parse_as_opt("CH");
    invoice.lang = "de-CH".to_string();
    let swiss = render_text(&invoice);
    // Headline in the bundle's own German spelling, Swiss statute in the citation.
    assert!(swiss.contains("Nicht steuerbar"), "{swiss}");
    assert!(
        swiss.contains("Ort der Dienstleistung im Ausland"),
        "{swiss}"
    );
    assert!(swiss.contains("Art. 8 Abs. 1 MWSTG"), "{swiss}");
    assert!(!swiss.contains("2006/112"));

    // Art. 21 MWSTG (supplies excluded without input-tax credit) and art. 23 (goods
    // export) are the wrong basis for a cross-border service and must never reappear.
    assert!(
        !swiss.contains("Art. 21 MWSTG") && !swiss.contains("Art. 23"),
        "{swiss}"
    );
}

/// A private customer is a different place-of-supply case, so dropping the customer's
/// tax ID must change the article — not silently keep citing the business one.
#[test]
fn a_consumer_supply_cites_the_consumer_article() {
    let mut invoice = create_minimal_invoice();
    invoice.organization.address.country = CountryCode::parse_as_opt("FR");
    invoice.tax_breakdown[0].exemption_type = Some(TaxExemptionType::Export);
    invoice.lang = "fr-FR".to_string();

    invoice.customer.tax_id = Some("DE123456789".to_string());
    let business = render_text(&invoice);
    assert!(business.contains("Art. 259-1° du CGI"), "{business}");

    invoice.customer.tax_id = None;
    let consumer = render_text(&invoice);
    assert!(consumer.contains("Art. 259 B du CGI"), "{consumer}");
    assert!(!consumer.contains("259-1°"));
}

/// The reverse-charge citation used to be read straight from the bundle, so every
/// seller got "Directive 2006/112/EC" — including sellers with no connection to the EU.
#[test]
fn a_non_eu_seller_is_never_cited_the_eu_directive() {
    let mut invoice = create_minimal_invoice();
    invoice.tax_breakdown[0].exemption_type = Some(TaxExemptionType::ReverseCharge);
    invoice.customer.tax_id = Some("AE123456789".to_string());
    invoice.organization.tax_id = Some("SA123456789".to_string());

    for country in ["SA", "CH", "US", "GB"] {
        invoice.organization.address.country = CountryCode::parse_as_opt(country);
        let rendered = render_text(&invoice);
        assert!(
            !rendered.contains("2006/112"),
            "{country} seller was cited the EU VAT Directive: {rendered}"
        );
    }

    // An EU seller still gets it, or the guard would be vacuous.
    invoice.organization.address.country = CountryCode::parse_as_opt("DE");
    assert!(render_text(&invoice).contains("2006/112"));

    // Credit notes carry the same reverse-charge citation, and its dict key must match
    // the template: a mismatch silently drops the mandatory Art. 44/196 mention.
    let mut cn = create_minimal_credit_note();
    cn.tax_breakdown[0].exemption_type = Some(cn::TaxExemptionType::ReverseCharge);
    cn.customer.tax_id = Some("DE123456789".to_string());
    cn.organization.tax_id = Some("FR123456789".to_string());
    cn.organization.address.country = CountryCode::parse_as_opt("FR");
    let rendered = render_credit_note_text(&cn);
    assert!(
        rendered.contains("2006/112"),
        "EU credit note lost its reverse-charge citation"
    );
    // "2006/112" alone cannot tell the two apart — both strings carry it. The document
    // type is only visible in who is said to owe the tax, which is the whole point of
    // having a separate credit-note key.
    assert!(
        rendered.contains("by the customer on the original invoice"),
        "credit note did not use its own reverse-charge wording: {rendered}"
    );
    assert!(
        !rendered.contains("by the customer (Article 196"),
        "invoice reverse-charge wording leaked onto a credit note: {rendered}"
    );
    cn.organization.address.country = CountryCode::parse_as_opt("CH");
    assert!(!render_credit_note_text(&cn).contains("2006/112"));
}

/// Spanish and Italian documents had no legal-block coverage at all, and each has a
/// mandated term that a plain translation gets wrong: Spain distinguishes *no sujeta*
/// (outside the scope) from *exenta* (exempt), and Italy mandates the bare annotation
/// "inversione contabile" plus its own national article.
#[test]
fn spanish_and_italian_legal_blocks_use_the_mandated_terms() {
    let mut invoice = create_minimal_invoice();
    invoice.tax_breakdown[0].exemption_type = Some(TaxExemptionType::Export);
    invoice.customer.tax_id = Some("US-EIN-123456789".to_string());

    invoice.organization.address.country = CountryCode::parse_as_opt("ES");
    invoice.lang = "es-ES".to_string();
    let spanish = render_text(&invoice);
    // A place-of-supply case is "no sujeta", never "exenta".
    assert!(spanish.contains("empresario o profesional"), "{spanish}");
    assert!(!spanish.contains("exenta"), "{spanish}");

    invoice.organization.address.country = CountryCode::parse_as_opt("IT");
    invoice.lang = "it-IT".to_string();
    let italian = render_text(&invoice);
    // Italy carries its own regime: the national article, not a directive-only cite.
    assert!(italian.contains("7-ter"), "{italian}");
    assert!(italian.contains("non soggetta"), "{italian}");

    // The reverse-charge annotation Italy mandates is the bare phrase.
    invoice.tax_breakdown[0].exemption_type = Some(TaxExemptionType::ReverseCharge);
    let italian = render_text(&invoice);
    assert!(italian.contains("Inversione contabile"), "{italian}");

    // Spain's is likewise a fixed form (RD 1619/2012 art. 6.1.m).
    invoice.organization.address.country = CountryCode::parse_as_opt("ES");
    invoice.lang = "es-ES".to_string();
    let spanish = render_text(&invoice);
    assert!(spanish.contains("Inversión del sujeto pasivo"), "{spanish}");
}

/// An exemption and an out-of-scope supply are different legal categories, and both
/// branches used to print the same headline — so a Spanish *operación exenta* was
/// labelled *no sujeta*, and a Swiss art. 21 *ausgenommen* supply was labelled with the
/// art. 8 *nicht steuerbar* headline that exists to say the opposite.
#[test]
fn an_exempt_supply_is_not_labelled_as_out_of_scope() {
    let mut invoice = create_minimal_invoice();
    invoice.organization.address.country = CountryCode::parse_as_opt("ES");
    invoice.lang = "es-ES".to_string();

    invoice.tax_breakdown[0].exemption_type = Some(TaxExemptionType::Export);
    let out_of_scope = render_text(&invoice);
    assert!(out_of_scope.contains("IVA no aplicable"), "{out_of_scope}");

    invoice.tax_breakdown[0].exemption_type = Some(TaxExemptionType::TaxExempt);
    let exempt = render_text(&invoice);
    assert!(exempt.contains("exenta"), "{exempt}");
    assert!(
        !exempt.contains("IVA no aplicable"),
        "an exempt supply took the out-of-scope headline: {exempt}"
    );

    // Switzerland overrides the out-of-scope headline but not the exemption one, so the
    // two must still differ there.
    invoice.organization.address.country = CountryCode::parse_as_opt("CH");
    invoice.lang = "de-CH".to_string();
    invoice.tax_breakdown[0].exemption_type = Some(TaxExemptionType::Export);
    let ch_scope = render_text(&invoice);
    invoice.tax_breakdown[0].exemption_type = Some(TaxExemptionType::TaxExempt);
    let ch_exempt = render_text(&invoice);
    assert!(ch_scope.contains("Nicht steuerbar"), "{ch_scope}");
    assert!(ch_exempt.contains("Steuerfrei"), "{ch_exempt}");
    assert!(
        !ch_exempt.contains("Nicht steuerbar"),
        "Swiss exempt supply took the art. 8 out-of-scope headline: {ch_exempt}"
    );
}

/// The engine's finding, not the document's snapshot, decides the place-of-supply
/// article. These diverge in production: the engine classifies on the live customer
/// record, while an operator can edit the invoice's own copy of the VAT number without
/// the tax being recomputed (the update path carries the exemption over deliberately).
#[test]
fn the_recorded_supply_basis_beats_the_customer_snapshot() {
    let mut invoice = create_minimal_invoice();
    invoice.organization.address.country = CountryCode::parse_as_opt("FR");
    invoice.tax_breakdown[0].exemption_type = Some(TaxExemptionType::Export);

    // Priced as a supply to a taxable person, but the snapshot's VAT number was cleared.
    invoice.tax_breakdown[0].supply_basis = Some(SupplyBasis::Business);
    invoice.customer.tax_id = None;
    let rendered = render_text(&invoice);
    assert!(
        rendered.contains("259-1\u{b0}"),
        "a B2B-priced supply must cite the business article: {rendered}"
    );
    assert!(
        !rendered.contains("non-taxable customer"),
        "document asserted the customer is a non-taxable person against the engine's \
         own finding: {rendered}"
    );

    // And the reverse: priced as B2C, with a tax ID typed onto the invoice afterwards.
    invoice.tax_breakdown[0].supply_basis = Some(SupplyBasis::Consumer);
    invoice.customer.tax_id = Some("US-EIN-123456789".to_string());
    let rendered = render_text(&invoice);
    assert!(
        rendered.contains("Article 259 B"),
        "a B2C-priced supply must cite the consumer article: {rendered}"
    );

    // With nothing recorded, the snapshot is still the only signal available.
    invoice.tax_breakdown[0].supply_basis = None;
    assert!(render_text(&invoice).contains("259-1\u{b0}"));
}

/// Territories folded into the seller's own VAT area are not exports, and the mention
/// must not tell a Monegasque or Manx customer their supply left the country.
#[test]
fn a_supply_inside_the_sellers_vat_territory_is_not_foreign() {
    use common_domain::country::same_vat_territory;
    for (seller, customer) in [("FR", "MC"), ("GB", "IM"), ("CH", "LI"), ("FR", "GP")] {
        assert!(
            same_vat_territory(seller, customer),
            "{seller} -> {customer} should stay inside one VAT territory"
        );
    }
    for (seller, customer) in [("FR", "DE"), ("FR", "CH"), ("CH", "AT")] {
        assert!(
            !same_vat_territory(seller, customer),
            "{seller} -> {customer}"
        );
    }
}

/// A seller outside the EU VAT area must never print the EU VAT Directive on an export,
/// on either document type. The French overseas departments are the trap: French VAT,
/// but third territories towards the EU, and the French export citation embeds the
/// directive — so they must resolve to the neutral non-EU wording, not the French regime.
#[test]
fn a_non_eu_export_never_cites_the_directive() {
    let mut invoice = create_minimal_invoice();
    invoice.tax_breakdown[0].exemption_type = Some(TaxExemptionType::Export);
    invoice.customer.tax_id = Some("DE123456789".to_string());

    for country in ["RE", "GP", "MQ", "GF", "YT", "CH", "GB", "US"] {
        invoice.organization.address.country = CountryCode::parse_as_opt(country);
        for lang in ["en-US", "fr-FR", "de-DE"] {
            invoice.lang = lang.to_string();
            let rendered = render_text(&invoice);
            assert!(
                !rendered.contains("2006/112"),
                "{country} seller ({lang}) cited the EU VAT Directive on an export: {rendered}"
            );
        }
    }
}

/// An unattributed line prints "no tax applied" and no statutory citation — the
/// fall-through is deliberate (see `TaxExemptionType::NotRegistered`), so pin it.
#[test]
fn an_undetermined_line_prints_no_citation() {
    let mut invoice = create_minimal_invoice();
    invoice.organization.address.country = CountryCode::parse_as_opt("FR");
    invoice.tax_breakdown[0].exemption_type = Some(TaxExemptionType::NotRegistered);
    let rendered = render_text(&invoice);
    assert!(!rendered.contains("2006/112"), "{rendered}");
    assert!(!rendered.contains("259"), "{rendered}");
}

#[test]
fn unsupported_language_falls_back_to_english() {
    let mut invoice = create_minimal_invoice();
    invoice.lang = "pl-PL".to_string();

    let rendered = render_text(&invoice);
    assert!(rendered.contains("Invoice"), "expected the en-US bundle");
    assert!(!rendered.contains("Facture"));
}

/// Swiss Standard German has no "ß" — it is always "ss". The `de` bundle is shared with
/// Germany and Austria, where "ß" is correct, so the two are kept apart by which strings
/// a Swiss seller can reach rather than by forking a `de-CH` bundle. That is a
/// convention, and conventions decay: this renders every branch a Swiss (and
/// Liechtenstein) seller can reach and asserts none of it carries a "ß".
///
/// If this fails, either the offending string is not actually Swiss-reachable and the
/// fix is to spell it "ss", or the shared bundle has stopped being viable for
/// Switzerland and a `de-CH` bundle is the answer.
#[test]
fn no_swiss_reachable_german_string_uses_the_eszett() {
    let mut invoice = create_minimal_invoice();
    invoice.lang = "de-CH".to_string();

    // Liechtenstein bills under the Swiss regime, so it reaches the same strings.
    for country in ["CH", "LI"] {
        invoice.organization.address.country = CountryCode::parse_as_opt(country);
        invoice.organization.tax_id = Some("CHE-123.456.789 MWST".to_string());

        // Every tax-treatment branch the template can take. Built fresh each pass
        // because `TaxExemptionType` is not `Clone`.
        for exemption in [
            "none",
            "export",
            "reverse_charge",
            "tax_exempt",
            "undetermined",
        ] {
            for customer_tax_id in [None, Some("DE123456789".to_string())] {
                invoice.tax_breakdown[0].exemption_type = match exemption {
                    "export" => Some(TaxExemptionType::Export),
                    "reverse_charge" => Some(TaxExemptionType::ReverseCharge),
                    "tax_exempt" => Some(TaxExemptionType::TaxExempt),
                    "undetermined" => Some(TaxExemptionType::NotRegistered),
                    _ => None,
                };
                invoice.customer.tax_id = customer_tax_id.clone();

                let rendered = render_text(&invoice);
                assert!(
                    !rendered.contains('\u{00df}'),
                    "{country} seller, {exemption} branch, customer tax id \
                     {customer_tax_id:?}: Swiss document carries a German ß:\n{rendered}"
                );
            }
        }
    }

    // The guard only means something if the bundle really does use ß elsewhere — a
    // German seller on the same bundle must still get the correct German spelling.
    invoice.organization.address.country = CountryCode::parse_as_opt("DE");
    invoice.lang = "de-DE".to_string();
    invoice.tax_breakdown[0].exemption_type = Some(TaxExemptionType::Export);
    invoice.customer.tax_id = Some("US-EIN-123456789".to_string());
    let german = render_text(&invoice);
    assert!(
        german.contains('\u{00df}'),
        "the ß guard is vacuous — the German bundle no longer uses ß at all:\n{german}"
    );
}

/// Portuguese ships as two bundles, and the vocabulary that separates them is on the
/// document's face: a Brazilian tag must not render European Portuguese.
#[test]
fn portuguese_renders_in_the_regions_own_variant() {
    let mut invoice = create_minimal_invoice();
    invoice.lang = "pt-PT".to_string();
    let european = render_text(&invoice);
    assert!(european.contains("Fatura n.º"), "{european}");
    // The rate column is "Taxa" in Portugal and "Alíquota" in Brazil.
    assert!(european.contains("Taxa"), "{european}");
    assert!(!european.contains("Alíquota"), "{european}");
    assert!(!european.contains("Invoice"), "pt-PT fell back to en-US");

    invoice.lang = "pt-BR".to_string();
    let brazilian = render_text(&invoice);
    assert!(brazilian.contains("Fatura nº"), "{brazilian}");
    assert!(brazilian.contains("Alíquota"), "{brazilian}");
    assert!(
        !brazilian.contains("Taxa"),
        "pt-BR rendered European Portuguese"
    );

    // Angola and the bare tag are European Portuguese; nothing falls back to English.
    for tag in ["pt", "pt-AO"] {
        invoice.lang = tag.to_string();
        let rendered = render_text(&invoice);
        assert!(rendered.contains("Fatura n.º"), "{tag}: {rendered}");
    }
}

/// A Portuguese seller must cite the CIVA article for the basis the tax was computed
/// on, under the mandated NIF label, in whichever language the document is issued.
#[test]
fn a_portuguese_seller_cites_the_civa_for_each_supply_basis() {
    let mut invoice = create_minimal_invoice();
    invoice.organization.address.country = CountryCode::parse_as_opt("PT");
    invoice.organization.tax_id = Some("PT123456789".to_string());
    invoice.tax_breakdown[0].exemption_type = Some(TaxExemptionType::Export);

    invoice.lang = "pt-PT".to_string();
    invoice.customer.tax_id = Some("US-EIN-123456789".to_string());
    let business = render_text(&invoice);
    assert!(business.contains("IVA não aplicável"), "{business}");
    // The literal M40 wording from the AT's exemption-reason table, which is what a
    // Portuguese accountant scans a document for.
    assert!(
        business.contains("Artigo 6.º n.º 6 alínea a) do CIVA, a contrário"),
        "{business}"
    );
    assert!(business.contains("NIF do emitente"), "{business}");

    invoice.customer.tax_id = None;
    let consumer = render_text(&invoice);
    assert!(consumer.contains("n.º 9, alínea h), do CIVA"), "{consumer}");
    assert!(!consumer.contains("a contrário"), "{consumer}");

    // The statute follows the seller, the wording follows the language.
    invoice.lang = "en-US".to_string();
    let english = render_text(&invoice);
    assert!(
        english.contains("Article 6(9)(h) of the Portuguese VAT Code"),
        "{english}"
    );
    assert!(english.contains("Seller VAT ID (NIF)"), "{english}");

    // The reverse-charge mention CIVA art. 36 n.º 13 prescribes is the exact
    // expression, hyphen included.
    invoice.lang = "pt-PT".to_string();
    invoice.customer.tax_id = Some("ES-B12345678".to_string());
    invoice.tax_breakdown[0].exemption_type = Some(TaxExemptionType::ReverseCharge);
    let reverse = render_text(&invoice);
    assert!(reverse.contains("IVA - autoliquidação"), "{reverse}");
    assert!(reverse.contains("2006/112"), "{reverse}");
}

/// A Brazilian seller prints the CNPJ label and generic tax wording, and no VAT
/// vocabulary or citation anywhere — the regime exists for the label alone.
#[test]
fn a_brazilian_seller_prints_the_cnpj_and_no_vat_wording() {
    let mut invoice = create_minimal_invoice();
    invoice.organization.address.country = CountryCode::parse_as_opt("BR");
    invoice.organization.tax_id = Some("12.345.678/0001-95".to_string());
    invoice.customer.tax_id = Some("US-EIN-123456789".to_string());
    invoice.tax_breakdown[0].exemption_type = Some(TaxExemptionType::Export);

    invoice.lang = "pt-BR".to_string();
    let rendered = render_text(&invoice);
    assert!(rendered.contains("CNPJ do emitente"), "{rendered}");
    assert!(rendered.contains("Imposto não aplicável"), "{rendered}");
    assert!(!rendered.contains("IVA"), "{rendered}");
    assert!(!rendered.contains("CIVA"), "{rendered}");
    assert!(!rendered.contains("2006/112"), "{rendered}");

    // The fixture's tax line is itself named "VAT 20%", so the guard is on the
    // corpus's own strings: the label and the headline.
    invoice.lang = "en-US".to_string();
    let english = render_text(&invoice);
    assert!(english.contains("Seller CNPJ"), "{english}");
    assert!(!english.contains("VAT ID"), "{english}");
    assert!(!english.contains("VAT not applicable"), "{english}");
}

#[test]
fn blank_legal_footer_drops_the_whole_section() {
    let mut invoice = create_minimal_invoice();
    invoice.metadata.flags.show_legal_info = Some(true);

    // A cleared form field can reach the renderer as "" rather than NULL; the
    // heading must not survive on its own.
    invoice.organization.footer_legal = Some(String::new());
    assert!(
        !render_text(&invoice).contains("Legal"),
        "empty legal footer must not print its heading"
    );

    invoice.organization.footer_legal = Some("SARL au capital de 10 000 EUR".to_string());
    let filled = render_text(&invoice);
    assert!(filled.contains("Legal"));
    assert!(filled.contains("SARL au capital"));
}

/// The engine records *what* a resolved rate is; the document words it. A German
/// invoice used to label its rate "VAT" — the engine's hardcoded English — instead
/// of "USt.", because the row label was the one line of the tax block no message
/// bundle could reach.
#[test]
fn an_engine_resolved_rate_is_worded_in_the_document_language() {
    let mut invoice = create_minimal_invoice();
    invoice.tax_breakdown[0].name = "VAT".to_string();
    invoice.tax_breakdown[0].tax_kind = Some(TaxKind::Vat);
    invoice.tax_breakdown[0].rate = Decimal::from_str("0.2").unwrap();

    invoice.lang = "de-DE".to_string();
    let german = render_text(&invoice);
    assert!(german.contains("USt. 20"), "{german}");
    assert!(!german.contains("VAT 20"), "{german}");

    invoice.lang = "fr-FR".to_string();
    let french = render_text(&invoice);
    assert!(french.contains("TVA 20"), "{french}");

    invoice.lang = "nl-NL".to_string();
    let dutch = render_text(&invoice);
    assert!(dutch.contains("btw 20"), "{dutch}");

    // The credit-note renderer words the same row through its own bundle.
    let mut credit_note = create_minimal_credit_note();
    credit_note.lang = "de-DE".to_string();
    credit_note.tax_breakdown[0].name = "VAT".to_string();
    credit_note.tax_breakdown[0].tax_kind = Some(TaxKind::Vat);
    credit_note.tax_breakdown[0].rate = Decimal::from_str("0.2").unwrap();
    let rendered = render_credit_note_text(&credit_note);
    assert!(rendered.contains("USt. 20"), "{rendered}");
    assert!(!rendered.contains("VAT 20"), "{rendered}");
}

/// The VAT row label is jurisdiction-sensitive on top of language-sensitive: the
/// shared `de` bundle writes "USt.", Swiss usage writes "MWST", and which applies
/// follows the seller — the same mechanism as the MWST-Nr. registration label.
#[test]
fn a_swiss_sellers_vat_row_is_labelled_mwst_in_german() {
    let mut invoice = create_minimal_invoice();
    invoice.organization.address.country = CountryCode::parse_as_opt("CH");
    invoice.lang = "de-DE".to_string();
    invoice.tax_breakdown[0].name = "VAT".to_string();
    invoice.tax_breakdown[0].tax_kind = Some(TaxKind::Vat);
    invoice.tax_breakdown[0].rate = Decimal::from_str("0.081").unwrap();

    let swiss = render_text(&invoice);
    assert!(swiss.contains("MWST"), "{swiss}");
    assert!(!swiss.contains("USt."), "{swiss}");

    // French-speaking Switzerland writes TVA — the override happens to coincide
    // with the generic term, so the French page must not change.
    invoice.lang = "fr-FR".to_string();
    let romandy = render_text(&invoice);
    assert!(romandy.contains("TVA"), "{romandy}");

    // A German seller keeps the bundle's own term.
    invoice.organization.address.country = CountryCode::parse_as_opt("DE");
    invoice.lang = "de-DE".to_string();
    let german = render_text(&invoice);
    assert!(german.contains("USt."), "{german}");
    assert!(!german.contains("MWST"), "{german}");
}

/// A row without a recorded kind keeps its stored label verbatim: merchant custom
/// taxes are named by the merchant, and a document persisted before the kind
/// existed must keep rendering the label it was issued with — re-wording a
/// finalized snapshot retroactively would be a guess.
#[test]
fn a_row_without_a_kind_keeps_its_stored_name() {
    let mut invoice = create_minimal_invoice();
    invoice.lang = "de-DE".to_string();
    // The fixture's row is named "VAT 20%" and carries no kind, like every row
    // written before the kind was recorded.
    let rendered = render_text(&invoice);
    assert!(rendered.contains("VAT 20%"), "{rendered}");
    assert!(!rendered.contains("USt."), "{rendered}");
}

/// The merchant's own exemption mention outranks the derived statutory wording —
/// the precedence the EN 16931 mapping already gives BT-120 — so the PDF and the
/// embedded XML state one tax position per document. It used to not render at all:
/// the merchant configured a mention the paying human never saw, while the XML
/// half of the same Factur-X document carried it.
#[test]
fn the_merchants_own_mention_replaces_the_statutory_wording() {
    let mut invoice = create_minimal_invoice();
    invoice.organization.address.country = CountryCode::parse_as_opt("FR");
    invoice.organization.tax_id = Some("FR12345678901".to_string());
    invoice.customer.tax_id = Some("DE123456789".to_string());
    invoice.tax_breakdown[0].exemption_type = Some(TaxExemptionType::ReverseCharge);
    invoice.tax_breakdown[0].exemption_reason =
        Some("Autoliquidation - article 283-2 du CGI".to_string());

    let rendered = render_text(&invoice);
    assert!(rendered.contains("article 283-2 du CGI"), "{rendered}");
    // One position per document: the corpus citation must not stack under it.
    assert!(!rendered.contains("2006/112"), "{rendered}");

    // Without a merchant mention, the derived statutory wording still renders.
    invoice.tax_breakdown[0].exemption_reason = None;
    let statutory = render_text(&invoice);
    assert!(statutory.contains("2006/112"), "{statutory}");
}

/// The credit-note template holds the same precedence, so the two document types
/// cannot drift apart on whose mention states the exemption.
#[test]
fn a_credit_note_carries_the_merchants_own_mention_too() {
    let mut credit_note = create_minimal_credit_note();
    credit_note.organization.address.country = CountryCode::parse_as_opt("FR");
    credit_note.customer.tax_id = Some("DE123456789".to_string());
    credit_note.tax_breakdown[0].exemption_type = Some(TaxExemptionType::ReverseCharge);
    credit_note.tax_breakdown[0].exemption_reason =
        Some("Autoliquidation - article 283-2 du CGI".to_string());

    let rendered = render_credit_note_text(&credit_note);
    assert!(rendered.contains("article 283-2 du CGI"), "{rendered}");
    assert!(!rendered.contains("2006/112"), "{rendered}");
}

/// A supply taxable in Guyane or Mayotte states that VAT does not apply and cites
/// CGI art. 294-1 — not the export's place-of-supply article (the supply never left
/// France's VAT territory), and not "No tax applied", which states no position.
#[test]
fn a_supply_in_guyane_cites_art_294_on_both_documents() {
    let mut invoice = create_minimal_invoice();
    invoice.lang = "fr-FR".to_string();
    invoice.organization.address.country = CountryCode::parse_as_opt("FR");
    invoice.tax_breakdown[0].exemption_type = Some(TaxExemptionType::NoVatTerritory);

    let rendered = render_text(&invoice);
    assert!(rendered.contains("TVA non applicable"), "{rendered}");
    assert!(rendered.contains("294-1"), "{rendered}");
    assert!(!rendered.contains("259-1"), "{rendered}");

    let mut credit_note = create_minimal_credit_note();
    credit_note.lang = "fr-FR".to_string();
    credit_note.organization.address.country = CountryCode::parse_as_opt("FR");
    credit_note.tax_breakdown[0].exemption_type = Some(TaxExemptionType::NoVatTerritory);

    let rendered = render_credit_note_text(&credit_note);
    assert!(rendered.contains("TVA non applicable"), "{rendered}");
    assert!(rendered.contains("294-1"), "{rendered}");
    assert!(!rendered.contains("259-1"), "{rendered}");
}

/// Body markers: every page of a document must carry at least one of these. A page
/// with none of them holds nothing but the running page footer — a blank page to the
/// reader, even though it is not blank to the layout engine.
const BODY_MARKERS: [&str; 6] = [
    "Issue Date",
    "Service line",
    "Total Due",
    "Tax Information",
    "Payment Terms",
    "Legal Information",
];

fn invoice_with_lines(count: usize) -> Invoice {
    let mut invoice = create_minimal_invoice();
    let template = invoice.lines.remove(0);
    invoice.lines = (0..count)
        .map(|i| InvoiceLine {
            name: format!("Service line {i}"),
            description: template.description.clone(),
            subtotal: template.subtotal,
            taxable_amount: template.taxable_amount,
            quantity: template.quantity,
            unit_price: template.unit_price,
            tax_rate: template.tax_rate,
            start_date: template.start_date,
            end_date: template.end_date,
            is_prorated: template.is_prorated,
            sub_lines: vec![],
            group_label: None,
            quoted: None,
            credit: None,
            promotional_credit: None,
            prepaid_credit: false,
        })
        .collect();
    invoice
}

/// Sweeps the line count across the first two page boundaries, for each state the
/// trailing footer sections can be in. The spacing around those sections is emitted
/// whether or not they render, so an unlucky line count used to spill a page
/// carrying only the page footer. Two boundaries is enough — later ones repeat the
/// same structure, and every extra count is a full template compile.
#[test]
fn no_page_carries_only_the_running_footer() {
    let footer_states = [
        (None, None, "absent"),
        (Some(""), Some(""), "blank"),
        (
            Some("SARL au capital de 10 000 EUR"),
            Some("Thank you"),
            "filled",
        ),
    ];

    // Font loading dominates renderer construction, so build it once.
    let renderer = TypstInvoiceRenderer::new().unwrap();

    for count in 1..24 {
        for (legal, info, label) in footer_states {
            let mut invoice = invoice_with_lines(count);
            invoice.metadata.flags.show_legal_info = Some(true);
            invoice.metadata.flags.show_footer_custom_info = Some(true);
            invoice.metadata.flags.show_terms = Some(true);
            invoice.organization.footer_legal = legal.map(str::to_string);
            invoice.organization.footer_info = info.map(str::to_string);

            let document = renderer.render_invoice(&invoice).unwrap();

            for (page, text) in document_text(&document).split('\n').enumerate() {
                if text.trim().is_empty() {
                    continue;
                }
                assert!(
                    BODY_MARKERS.iter().any(|marker| text.contains(marker)),
                    "page {page} holds only the running footer \
                     (lines={count}, footers={label}): {text:?}"
                );
            }
        }
    }
}

fn create_minimal_invoice() -> Invoice {
    let issue_date = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
    let due_date = NaiveDate::from_ymd_opt(2025, 1, 15).unwrap();
    let start_date = NaiveDate::from_ymd_opt(2024, 12, 1).unwrap();
    let end_date = NaiveDate::from_ymd_opt(2024, 12, 31).unwrap();

    let eur = iso::find("EUR").unwrap();

    Invoice {
        lang: "en-US".to_string(),
        organization: Organization {
            name: "Test Company".to_string(),
            logo_src: None,
            legal_number: None,
            address: Address {
                line1: Some("123 Test St".to_string()),
                line2: None,
                city: Some("Test City".to_string()),
                country: CountryCode::parse_as_opt("US"),
                state: None,
                zip_code: Some("12345".to_string()),
            },
            email: Some("test@company.com".to_string()),
            contact_name: None,
            contact_phone: None,
            tax_id: None,
            footer_info: None,
            footer_legal: None,
            accounting_currency: *eur,
            exchange_rate: None,
        },
        customer: Customer {
            name: "Test Customer".to_string(),
            legal_number: None,
            buyer_reference: None,
            address: Address {
                line1: Some("456 Customer Ave".to_string()),
                line2: None,
                city: Some("Customer City".to_string()),
                country: CountryCode::parse_as_opt("US"),
                state: None,
                zip_code: Some("54321".to_string()),
            },
            email: Some("customer@test.com".to_string()),
            tax_id: None,
        },
        metadata: InvoiceMetadata {
            number: "TEST-001".to_string(),
            issue_date,
            payment_term: 14,
            subtotal: Money::from_major(100, eur),
            tax_amount: Money::from_major(20, eur),
            total_amount: Money::from_major(120, eur),
            currency: eur,
            due_date,
            memo: None,
            payment_url: None,
            flags: Flags {
                show_payment_status: Some(true),
                show_payment_info: Some(false),
                show_terms: Some(false),
                show_tax_info: Some(true),
                show_legal_info: Some(false),
                show_footer_custom_info: Some(false),
                whitelabel: Some(false),
            },
            discount: Money::from_major(0, eur),
            promotional_credit: Money::from_major(0, eur),
            quoted_promotional_credit: None,
            prepaid_amount: Money::from_major(0, eur),
            purchase_order: Some("some order".to_string()),
            parent_invoice_number: None,
            parent_invoice_date: None,
        },
        lines: vec![InvoiceLine {
            name: "Test Service".to_string(),
            description: Some("Basic test service".to_string()),
            subtotal: Money::from_major(100, eur),
            taxable_amount: Money::from_major(100, eur),
            quantity: Some(Decimal::from_str("1.0").unwrap()),
            unit_price: Some(Money::from_major(100, eur)),
            tax_rate: Decimal::from_str("20.0").unwrap(),
            start_date,
            end_date,
            is_prorated: false,
            sub_lines: vec![],
            group_label: None,
            quoted: None,
            credit: None,
            promotional_credit: None,
            prepaid_credit: false,
        }],
        coupons: vec![],
        payment_status: Some(PaymentStatus::Unpaid),
        transactions: vec![],
        bank_details: None,
        payment_means: None,
        tax_breakdown: vec![TaxBreakdownItem {
            name: "VAT 20%".to_string(),
            tax_kind: None,
            rate: Decimal::from_str("20.0").unwrap(),
            amount: Money::from_major(20, eur),
            taxable_amount: Money::from_major(20, eur),
            exemption_reason: None,
            exemption_type: None,
            supply_basis: None,
            prepaid_credit: false,
        }],
        annexes: vec![],
        tax_inclusive: false,
        prepaid_credit_purchase: false,
    }
}

fn create_full_invoice() -> Invoice {
    let issue_date = NaiveDate::from_ymd_opt(2025, 4, 1).unwrap();
    let due_date = NaiveDate::from_ymd_opt(2025, 4, 15).unwrap();
    let start_date = NaiveDate::from_ymd_opt(2025, 3, 1).unwrap();
    let end_date = NaiveDate::from_ymd_opt(2025, 3, 31).unwrap();
    let payment_date = NaiveDate::from_ymd_opt(2025, 3, 15).unwrap();

    let eur = iso::find("EUR").unwrap();

    let mut payment_info = HashMap::new();
    payment_info.insert("Bank Name".to_string(), "Test Bank".to_string());
    payment_info.insert(
        "IBAN".to_string(),
        "DE89 3704 0044 0532 0130 00".to_string(),
    );

    Invoice {
        lang: "en-US".to_string(),
        organization: Organization {
            name: "Full Test Corp".to_string(),
            logo_src: None,
            legal_number: Some("123456789".to_string()),
            address: Address {
                line1: Some("789 Corporate Blvd".to_string()),
                line2: Some("Suite 100".to_string()),
                city: Some("Business City".to_string()),
                country: CountryCode::parse_as_opt("US"),
                state: Some("CA".to_string()),
                zip_code: Some("90210".to_string()),
            },
            email: Some("billing@fulltestcorp.com".to_string()),
            contact_name: None,
            contact_phone: None,
            tax_id: Some("US123456789".to_string()),
            footer_info: Some("Payment terms: Net 14 days".to_string()),
            footer_legal: Some("All prices exclude VAT unless specified.".to_string()),
            accounting_currency: *eur,
            exchange_rate: Some(Decimal::from_str("1.08").unwrap()),
        },
        customer: Customer {
            name: "Premium Customer Ltd لد語தא어简".to_string(),
            legal_number: Some("987654321".to_string()),
            buyer_reference: None,
            address: Address {
                line1: Some("321 Premium Plaza".to_string()),
                line2: Some("Floor 25".to_string()),
                city: Some("Metro City".to_string()),
                country: CountryCode::parse_as_opt("US"),
                state: None,
                zip_code: Some("SW1A 1AA".to_string()),
            },
            email: Some("accounts@premiumcustomer.com".to_string()),
            tax_id: Some("GB987654321".to_string()),
        },
        metadata: InvoiceMetadata {
            number: "INV-2025-FULL-001".to_string(),
            issue_date,
            payment_term: 14,
            subtotal: Money::from_major(1000, eur),
            tax_amount: Money::from_major(200, eur),
            total_amount: Money::from_major(1175, eur),
            currency: eur,
            due_date,
            memo: Some("Thank you for your business!".to_string()),
            payment_url: Some("https://pay.example.com/invoice/full-001".to_string()),
            flags: Flags {
                show_payment_status: Some(true),
                show_payment_info: Some(true),
                show_terms: Some(true),
                show_tax_info: Some(true),
                show_legal_info: Some(true),
                show_footer_custom_info: Some(true),
                whitelabel: Some(false),
            },
            discount: Money::from_major(0, eur),
            promotional_credit: Money::from_major(0, eur),
            quoted_promotional_credit: None,
            prepaid_amount: Money::from_major(0, eur),
            purchase_order: Some("some order".to_string()),
            parent_invoice_number: None,
            parent_invoice_date: None,
        },
        lines: vec![
            InvoiceLine {
                name: "Professional Services".to_string(),
                description: Some("Consulting and development services".to_string()),
                subtotal: Money::from_major(800, eur),
                taxable_amount: Money::from_major(800, eur),
                quantity: Some(Decimal::from_str("40.0").unwrap()),
                unit_price: Some(Money::from_major(20, eur)),
                tax_rate: Decimal::from_str("20.0").unwrap(),
                start_date,
                end_date,
                is_prorated: false,
                sub_lines: vec![
                    InvoiceSubLine {
                        name: "Architecture Design".to_string(),
                        total: Money::from_major(300, eur),
                        quantity: Decimal::from_str("15.0").unwrap(),
                        unit_price: Money::from_major(20, eur),
                    },
                    InvoiceSubLine {
                        name: "Implementation".to_string(),
                        total: Money::from_major(500, eur),
                        quantity: Decimal::from_str("25.0").unwrap(),
                        unit_price: Money::from_major(20, eur),
                    },
                ],
                group_label: None,
                quoted: None,
                credit: None,
                promotional_credit: None,
                prepaid_credit: false,
            },
            InvoiceLine {
                name: "Support Package".to_string(),
                description: Some("Monthly support and maintenance".to_string()),
                subtotal: Money::from_major(200, eur),
                taxable_amount: Money::from_major(200, eur),
                quantity: Some(Decimal::from_str("1.0").unwrap()),
                unit_price: Some(Money::from_major(200, eur)),
                tax_rate: Decimal::from_str("20.0").unwrap(),
                start_date,
                end_date,
                is_prorated: false,
                sub_lines: vec![],
                group_label: None,
                quoted: None,
                credit: None,
                promotional_credit: None,
                prepaid_credit: false,
            },
        ],
        coupons: vec![Coupon {
            name: "Early Payment Discount".to_string(),
            total: Money::from_major(25, eur),
            quoted_total: None,
        }],
        payment_status: Some(PaymentStatus::PartiallyPaid),
        transactions: vec![Transaction {
            method: "Wire Transfer".to_string(),
            date: payment_date,
            amount: Money::from_major(600, eur),
        }],
        bank_details: Some(payment_info),
        payment_means: None,
        tax_breakdown: vec![TaxBreakdownItem {
            name: "VAT 20%".to_string(),
            tax_kind: None,
            rate: Decimal::from_str("20.0").unwrap(),
            amount: Money::from_major(200, eur),
            taxable_amount: Money::from_major(200, eur),
            exemption_reason: None,
            exemption_type: None,
            supply_basis: None,
            prepaid_credit: false,
        }],
        annexes: vec![sample_event_annex()],
        tax_inclusive: false,
        prepaid_credit_purchase: false,
    }
}

/// A grouped event-detail annex mirroring the `count(project_id)` use case:
/// individual events listed (not aggregated), grouped by client.
fn sample_event_annex() -> meteroid_invoicing::model::Annex {
    use meteroid_invoicing::model::{Annex, AnnexCell, AnnexGroup, AnnexRow, AnnexTable};

    let row = |cells: &[&str]| AnnexRow {
        cells: cells
            .iter()
            .map(|s| AnnexCell::Text(s.to_string()))
            .collect(),
        emphasis: false,
    };

    Annex {
        title: "Detail of billed events".to_string(),
        description: Some(
            "Individual events backing the aggregated usage line, grouped by client.".to_string(),
        ),
        table: AnnexTable {
            columns: ["Project ID", "Period", "RFP ID", "Consultant", "Mission"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            groups: vec![
                AnnexGroup {
                    heading: Some("Acme Corp — France".to_string()),
                    rows: vec![
                        row(&[
                            "PRJ-001",
                            "2025/01",
                            "RFP-42",
                            "Jane Doe",
                            "Cloud migration",
                        ]),
                        row(&[
                            "PRJ-001",
                            "2025/02",
                            "RFP-42",
                            "Jane Doe",
                            "Cloud migration",
                        ]),
                        row(&["PRJ-007", "2025/03", "RFP-88", "John Roe", "Data platform"]),
                    ],
                },
                AnnexGroup {
                    heading: Some("Globex — Germany".to_string()),
                    rows: vec![
                        row(&["PRJ-031", "2025/01", "RFP-15", "Erika M.", "Security audit"]),
                        row(&["PRJ-031", "2025/03", "RFP-15", "Erika M.", "Security audit"]),
                    ],
                },
            ],
        },
    }
}

// Credit Note Tests

#[tokio::test]
async fn test_typst_credit_note_generation_validates_template() {
    let generator =
        TypstCreditNotePdfGenerator::new().expect("Failed to create TypstCreditNotePdfGenerator");
    let credit_note = create_minimal_credit_note();

    let result = generator.generate_credit_note_pdf(&credit_note, None).await;
    assert!(
        result.is_ok(),
        "Failed to generate credit note PDF: {:?}",
        result.err()
    );

    let pdf_data = result.unwrap();
    assert!(
        !pdf_data.is_empty(),
        "Generated credit note PDF should not be empty"
    );
    assert!(
        pdf_data.len() > 1000,
        "Generated credit note PDF seems too small, likely an error"
    );
    assert!(
        &pdf_data[0..4] == b"%PDF",
        "Output should be a valid PDF file"
    );
}

#[tokio::test]
async fn test_typst_credit_note_with_full_data() {
    let generator =
        TypstCreditNotePdfGenerator::new().expect("Failed to create TypstCreditNotePdfGenerator");
    let credit_note = create_full_credit_note();

    let result = generator.generate_credit_note_pdf(&credit_note, None).await;
    assert!(
        result.is_ok(),
        "Failed to generate credit note PDF with full data: {:?}",
        result.err()
    );

    let pdf_data = result.unwrap();
    assert!(
        !pdf_data.is_empty(),
        "Generated credit note PDF should not be empty"
    );
    assert!(
        pdf_data.len() > 1000,
        "Generated credit note PDF seems too small"
    );
    assert!(
        &pdf_data[0..4] == b"%PDF",
        "Output should be a valid PDF file"
    );

    // let output_path = Path::new("benchmark_invoice.pdf");
    // std::fs::write(output_path, pdf_data).unwrap();
    // println!(
    //     "\nExample invoice saved at: {:?}",
    //     output_path.canonicalize().unwrap()
    // );
}

#[tokio::test]
async fn test_typst_credit_note_with_multiple_languages() {
    let generator =
        TypstCreditNotePdfGenerator::new().expect("Failed to create TypstCreditNotePdfGenerator");

    // Test English credit note
    let mut credit_note_en = create_minimal_credit_note();
    credit_note_en.lang = "en-US".to_string();
    let result_en = generator
        .generate_credit_note_pdf(&credit_note_en, None)
        .await;
    assert!(
        result_en.is_ok(),
        "Failed to generate English credit note PDF: {:?}",
        result_en.err()
    );

    // Test French credit note
    let mut credit_note_fr = create_minimal_credit_note();
    credit_note_fr.lang = "fr-FR".to_string();
    let result_fr = generator
        .generate_credit_note_pdf(&credit_note_fr, None)
        .await;
    assert!(
        result_fr.is_ok(),
        "Failed to generate French credit note PDF: {:?}",
        result_fr.err()
    );
}

#[tokio::test]
async fn test_typst_credit_note_debt_cancellation_type() {
    let generator =
        TypstCreditNotePdfGenerator::new().expect("Failed to create TypstCreditNotePdfGenerator");

    let mut credit_note = create_minimal_credit_note();
    credit_note.metadata.credit_type = CreditType::DebtCancellation;
    credit_note.metadata.refunded_amount = Money::from_major(0, iso::find("EUR").unwrap());
    credit_note.metadata.credited_amount = Money::from_major(0, iso::find("EUR").unwrap());

    let result = generator.generate_credit_note_pdf(&credit_note, None).await;
    assert!(
        result.is_ok(),
        "Failed to generate debt-cancellation credit note PDF: {:?}",
        result.err()
    );

    let pdf_data = result.unwrap();
    assert!(
        &pdf_data[0..4] == b"%PDF",
        "Output should be a valid PDF file"
    );

    let mut credit_note_fr = create_minimal_credit_note();
    credit_note_fr.lang = "fr-FR".to_string();
    credit_note_fr.metadata.credit_type = CreditType::DebtCancellation;
    let result_fr = generator
        .generate_credit_note_pdf(&credit_note_fr, None)
        .await;
    assert!(
        result_fr.is_ok(),
        "Failed to generate French debt-cancellation credit note PDF: {:?}",
        result_fr.err()
    );
}

#[tokio::test]
async fn test_typst_credit_note_refund_type() {
    let generator =
        TypstCreditNotePdfGenerator::new().expect("Failed to create TypstCreditNotePdfGenerator");

    let mut credit_note = create_minimal_credit_note();
    credit_note.metadata.credit_type = CreditType::Refund;
    credit_note.metadata.refunded_amount = Money::from_major(120, iso::find("EUR").unwrap());
    credit_note.metadata.credited_amount = Money::from_major(0, iso::find("EUR").unwrap());

    let result = generator.generate_credit_note_pdf(&credit_note, None).await;
    assert!(
        result.is_ok(),
        "Failed to generate refund credit note PDF: {:?}",
        result.err()
    );

    let pdf_data = result.unwrap();
    assert!(
        &pdf_data[0..4] == b"%PDF",
        "Output should be a valid PDF file"
    );
}

fn create_minimal_credit_note() -> CreditNote {
    let issue_date = NaiveDate::from_ymd_opt(2025, 1, 15).unwrap();
    let start_date = NaiveDate::from_ymd_opt(2024, 12, 1).unwrap();
    let end_date = NaiveDate::from_ymd_opt(2024, 12, 31).unwrap();

    let eur = iso::find("EUR").unwrap();

    CreditNote {
        lang: "en-US".to_string(),
        organization: cn::Organization {
            name: "Test Company".to_string(),
            logo_src: None,
            legal_number: None,
            address: cn::Address {
                line1: Some("123 Test St".to_string()),
                line2: None,
                city: Some("Test City".to_string()),
                country: CountryCode::parse_as_opt("US"),
                state: None,
                zip_code: Some("12345".to_string()),
            },
            email: Some("test@company.com".to_string()),
            contact_name: None,
            contact_phone: None,
            tax_id: None,
            footer_info: None,
            footer_legal: None,
            accounting_currency: *eur,
            exchange_rate: None,
        },
        customer: cn::Customer {
            name: "Test Customer".to_string(),
            legal_number: None,
            buyer_reference: None,
            address: cn::Address {
                line1: Some("456 Customer Ave".to_string()),
                line2: None,
                city: Some("Customer City".to_string()),
                country: CountryCode::parse_as_opt("US"),
                state: None,
                zip_code: Some("54321".to_string()),
            },
            email: Some("customer@test.com".to_string()),
            tax_id: None,
        },
        metadata: CreditNoteMetadata {
            number: "CN-001".to_string(),
            issue_date,
            related_invoice_number: "INV-001".to_string(),
            related_invoice_date: issue_date,
            subtotal: Money::from_major(100, eur),
            tax_amount: Money::from_major(20, eur),
            total_amount: Money::from_major(120, eur),
            currency: eur,
            reason: Some("Billing error".to_string()),
            memo: None,
            credit_type: CreditType::CreditToBalance,
            refunded_amount: Money::from_major(0, eur),
            credited_amount: Money::from_major(120, eur),
            debt_cancelled_amount: Money::from_major(0, eur),
            promotional_credit_restored: Money::from_major(0, eur),
            promotional_credit_restored_estimated: false,
            flags: cn::Flags {
                show_tax_info: Some(true),
                show_legal_info: Some(false),
                show_footer_custom_info: Some(false),
                whitelabel: Some(false),
            },
        },
        lines: vec![cn::CreditNoteLine {
            name: "Test Service".to_string(),
            description: Some("Credited service".to_string()),
            subtotal: Money::from_major(100, eur),
            taxable_amount: Money::from_major(100, eur),
            quantity: Some(Decimal::from_str("1.0").unwrap()),
            unit_price: Some(Money::from_major(100, eur)),
            tax_rate: Decimal::from_str("20.0").unwrap(),
            start_date,
            end_date,
            is_prorated: false,
            sub_lines: vec![],
            promotional_credit: None,
        }],
        tax_breakdown: vec![cn::TaxBreakdownItem {
            prepaid_credit: false,
            name: "VAT 20%".to_string(),
            tax_kind: None,
            rate: Decimal::from_str("20.0").unwrap(),
            amount: Money::from_major(20, eur),
            taxable_amount: Money::from_major(20, eur),
            exemption_reason: None,
            exemption_type: None,
            supply_basis: None,
        }],
    }
}

fn create_full_credit_note() -> CreditNote {
    let issue_date = NaiveDate::from_ymd_opt(2025, 4, 15).unwrap();
    let start_date = NaiveDate::from_ymd_opt(2025, 3, 1).unwrap();
    let end_date = NaiveDate::from_ymd_opt(2025, 3, 31).unwrap();

    let eur = iso::find("EUR").unwrap();

    CreditNote {
        lang: "en-US".to_string(),
        organization: cn::Organization {
            name: "Full Test Corp".to_string(),
            logo_src: None,
            legal_number: Some("123456789".to_string()),
            address: cn::Address {
                line1: Some("789 Corporate Blvd".to_string()),
                line2: Some("Suite 100".to_string()),
                city: Some("Business City".to_string()),
                country: CountryCode::parse_as_opt("US"),
                state: Some("CA".to_string()),
                zip_code: Some("90210".to_string()),
            },
            email: Some("billing@fulltestcorp.com".to_string()),
            contact_name: None,
            contact_phone: None,
            tax_id: Some("US123456789".to_string()),
            footer_info: Some("Credit notes are non-transferable.".to_string()),
            footer_legal: Some("This credit note is valid for 12 months.".to_string()),
            accounting_currency: *eur,
            exchange_rate: Some(Decimal::from_str("1.08").unwrap()),
        },
        customer: cn::Customer {
            name: "Premium Customer Ltd".to_string(),
            legal_number: Some("987654321".to_string()),
            buyer_reference: None,
            address: cn::Address {
                line1: Some("321 Premium Plaza".to_string()),
                line2: Some("Floor 25".to_string()),
                city: Some("Metro City".to_string()),
                country: CountryCode::parse_as_opt("GB"),
                state: None,
                zip_code: Some("SW1A 1AA".to_string()),
            },
            email: Some("accounts@premiumcustomer.com".to_string()),
            tax_id: Some("GB987654321".to_string()),
        },
        metadata: CreditNoteMetadata {
            number: "CN-2025-FULL-001".to_string(),
            issue_date,
            related_invoice_number: "INV-2025-FULL-001".to_string(),
            related_invoice_date: issue_date,
            subtotal: Money::from_major(500, eur),
            tax_amount: Money::from_major(100, eur),
            total_amount: Money::from_major(600, eur),
            currency: eur,
            reason: Some("Partial refund for unused services".to_string()),
            memo: Some("We apologize for any inconvenience.".to_string()),
            credit_type: CreditType::Refund,
            refunded_amount: Money::from_major(600, eur),
            credited_amount: Money::from_major(0, eur),
            debt_cancelled_amount: Money::from_major(0, eur),
            promotional_credit_restored: Money::from_major(0, eur),
            promotional_credit_restored_estimated: false,
            flags: cn::Flags {
                show_tax_info: Some(true),
                show_legal_info: Some(true),
                show_footer_custom_info: Some(true),
                whitelabel: Some(false),
            },
        },
        lines: vec![
            cn::CreditNoteLine {
                name: "Professional Services".to_string(),
                description: Some("Unused consulting hours".to_string()),
                subtotal: Money::from_major(400, eur),
                taxable_amount: Money::from_major(400, eur),
                quantity: Some(Decimal::from_str("20.0").unwrap()),
                unit_price: Some(Money::from_major(20, eur)),
                tax_rate: Decimal::from_str("20.0").unwrap(),
                start_date,
                end_date,
                is_prorated: false,
                sub_lines: vec![
                    cn::CreditNoteSubLine {
                        name: "Architecture Design".to_string(),
                        total: Money::from_major(200, eur),
                        quantity: Decimal::from_str("10.0").unwrap(),
                        unit_price: Money::from_major(20, eur),
                    },
                    cn::CreditNoteSubLine {
                        name: "Implementation".to_string(),
                        total: Money::from_major(200, eur),
                        quantity: Decimal::from_str("10.0").unwrap(),
                        unit_price: Money::from_major(20, eur),
                    },
                ],
                promotional_credit: None,
            },
            cn::CreditNoteLine {
                name: "Support Package".to_string(),
                description: Some("Unused support credit".to_string()),
                subtotal: Money::from_major(100, eur),
                taxable_amount: Money::from_major(100, eur),
                quantity: Some(Decimal::from_str("1.0").unwrap()),
                unit_price: Some(Money::from_major(100, eur)),
                tax_rate: Decimal::from_str("20.0").unwrap(),
                start_date,
                end_date,
                is_prorated: false,
                sub_lines: vec![],
                promotional_credit: None,
            },
        ],
        tax_breakdown: vec![cn::TaxBreakdownItem {
            prepaid_credit: false,
            name: "VAT 20%".to_string(),
            tax_kind: None,
            rate: Decimal::from_str("20.0").unwrap(),
            amount: Money::from_major(100, eur),
            taxable_amount: Money::from_major(100, eur),
            exemption_reason: None,
            exemption_type: None,
            supply_basis: None,
        }],
    }
}

/// Both parties' registration numbers are mandatory mentions on a French
/// invoice, so neither may depend on unrelated optional content.
#[test]
fn both_parties_registration_numbers_reach_the_pdf() {
    let mut invoice = create_minimal_invoice();
    invoice.organization.legal_number = Some("889 123 456 00015".to_string());
    invoice.customer.legal_number = Some("552 100 554 00023".to_string());
    // The section is on, but with no free-text blurb — which used to be what
    // silently suppressed the seller's number.
    invoice.metadata.flags.show_legal_info = Some(true);
    invoice.organization.footer_legal = None;

    let text = render_text(&invoice);
    assert!(
        text.contains("889 123 456 00015"),
        "seller registration number missing: {text}"
    );
    assert!(
        text.contains("552 100 554 00023"),
        "buyer registration number missing: {text}"
    );
}

/// The note of QA M8 (CN-000001): a 50.00 line that took 5.00 of promotional credit before
/// 20% tax, stated negative as the store writes it: 50.00 - 5.00 = 45.00 taxed 9.00, 54.00.
fn promotional_credit_note() -> CreditNote {
    let eur = iso::find("EUR").unwrap();
    let money = |s: &str| Money::from_decimal(Decimal::from_str(s).unwrap(), eur);
    let mut note = create_minimal_credit_note();
    note.lang = "fr-FR".to_string();
    note.metadata.subtotal = money("-50");
    note.metadata.tax_amount = money("-9");
    note.metadata.total_amount = money("-54");
    note.metadata.credit_type = CreditType::Refund;
    note.metadata.refunded_amount = money("39");
    note.metadata.credited_amount = money("15");
    note.metadata.promotional_credit_restored = money("5");
    let line = &mut note.lines[0];
    line.name = "Onboarding workshop".to_string();
    line.subtotal = money("-50");
    line.taxable_amount = money("-45");
    line.unit_price = Some(money("50"));
    line.tax_rate = Decimal::from_str("0.20").unwrap();
    line.promotional_credit = Some(money("-5"));
    note.tax_breakdown[0].tax_kind = Some(TaxKind::Vat);
    note.tax_breakdown[0].rate = Decimal::from_str("0.20").unwrap();
    note.tax_breakdown[0].amount = money("-9");
    note.tax_breakdown[0].taxable_amount = money("-45");
    note
}

fn summary_foots(content: &TypstCreditNoteContent) {
    let value = |d: &typst::foundations::Decimal| {
        Decimal::from_str(&d.to_string().replace('\u{2212}', "-")).unwrap()
    };
    assert_eq!(
        value(&content.subtotal)
            + value(&content.discount)
            + value(&content.promotional_credit)
            + value(&content.tax_amount),
        value(&content.total_amount),
        "subtotal, reductions and tax add up to the total"
    );
}

#[test]
fn a_credit_note_reversing_promotional_credit_foots() {
    let note = promotional_credit_note();
    let content = TypstCreditNoteContent::try_from(&note).unwrap();
    assert_eq!(content.subtotal.to_string(), "\u{2212}50.00");
    assert_eq!(content.discount.to_string(), "0.00");
    assert_eq!(content.promotional_credit.to_string(), "5.00");
    assert_eq!(content.tax_amount.to_string(), "\u{2212}9.00");
    assert_eq!(content.total_amount.to_string(), "\u{2212}54.00");
    assert_eq!(content.promotional_credit_restored.to_string(), "5.00");
    summary_foots(&content);

    let text = render_credit_note_text(&note);
    let subtotal = text.find("Sous-total").expect("subtotal row");
    let credit = text
        .find("Crédit promotionnel\n")
        .or_else(|| text.find("Crédit promotionnel+"))
        .or_else(|| text.rfind("Crédit promotionnel"))
        .unwrap();
    let tax = text
        .rfind("TVA 20")
        .unwrap_or_else(|| panic!("tax row: {text}"));
    assert!(
        subtotal < credit && credit < tax,
        "between subtotal and tax: {text}"
    );
    assert!(
        text.contains("+5.00"),
        "the reversed credit adds back: {text}"
    );
    assert!(
        text.contains("Crédit promotionnel restitué"),
        "restored credit: {text}"
    );
    assert!(
        text.contains("Montant à rembourser"),
        "issued before any refund is paid: {text}"
    );
    assert!(!text.contains("Montant remboursé"), "{text}");
    assert!(text.contains("Remboursement"), "the note's type: {text}");
    assert!(text.contains("39.00"), "{text}");
    assert!(text.contains("Montant crédité"), "{text}");
    assert!(text.contains("15.00"), "{text}");
    assert!(!text.contains("Remise"), "no discount row: {text}");
}

/// A coupon took 20.00 off the 100.00 line: the note reverses it as a discount, not as
/// promotional credit, and states the debt it cancelled.
#[test]
fn a_debt_cancellation_of_a_discounted_line_states_the_discount_and_the_debt() {
    let eur = iso::find("EUR").unwrap();
    let money = |s: &str| Money::from_decimal(Decimal::from_str(s).unwrap(), eur);
    let mut note = promotional_credit_note();
    note.lang = "en-US".to_string();
    note.metadata.subtotal = money("-100");
    note.metadata.tax_amount = money("-16");
    note.metadata.total_amount = money("-96");
    note.metadata.credit_type = CreditType::DebtCancellation;
    note.metadata.refunded_amount = money("0");
    note.metadata.credited_amount = money("0");
    note.metadata.promotional_credit_restored = money("0");
    note.metadata.debt_cancelled_amount = money("96");
    note.lines[0].subtotal = money("-100");
    note.lines[0].taxable_amount = money("-80");
    note.lines[0].unit_price = Some(money("100"));
    note.lines[0].promotional_credit = None;
    note.tax_breakdown[0].amount = money("-16");
    note.tax_breakdown[0].taxable_amount = money("-80");

    let content = TypstCreditNoteContent::try_from(&note).unwrap();
    assert_eq!(content.discount.to_string(), "20.00");
    assert_eq!(content.promotional_credit.to_string(), "0.00");
    summary_foots(&content);

    let text = render_credit_note_text(&note);
    assert!(text.contains("Discount"), "discount row: {text}");
    assert!(text.contains("+€\u{a0}20.00"), "{text}");
    assert!(!text.contains("Promotional credit"), "{text}");
    assert!(text.contains("Debt Cancelled"), "{text}");
    assert!(text.contains("96.00"), "{text}");
    assert!(!text.contains("Amount to Refund"), "{text}");
}

/// A coupon and promotional credit on one line: each reduction gets its own row.
#[test]
fn a_note_splits_the_discount_from_the_promotional_credit() {
    let eur = iso::find("EUR").unwrap();
    let money = |s: &str| Money::from_decimal(Decimal::from_str(s).unwrap(), eur);
    let mut note = promotional_credit_note();
    note.metadata.subtotal = money("-100");
    note.metadata.tax_amount = money("-15");
    note.metadata.total_amount = money("-90");
    note.lines[0].subtotal = money("-100");
    note.lines[0].taxable_amount = money("-75");
    note.lines[0].promotional_credit = Some(money("-5"));

    let content = TypstCreditNoteContent::try_from(&note).unwrap();
    assert_eq!(content.discount.to_string(), "20.00");
    assert_eq!(content.promotional_credit.to_string(), "5.00");
    summary_foots(&content);

    let text = render_credit_note_text(&note);
    assert!(text.contains("Remise"), "{text}");
    assert!(text.contains("+20.00"), "{text}");
    assert!(text.contains("+5.00"), "{text}");
}

/// Credit applied at issuance covered the whole invoice: nothing is due, so the PDF neither
/// states an amount due by a date nor offers to pay online.
#[test]
fn an_invoice_with_nothing_left_to_pay_says_so_and_offers_no_payment() {
    let eur = iso::find("EUR").unwrap();
    let mut invoice = create_minimal_invoice();
    invoice.metadata.prepaid_amount = Money::from_major(120, eur);
    invoice.metadata.payment_url = Some("https://pay.example.com/i/1".to_string());

    let text = render_text(&invoice);
    assert!(text.contains("Nothing to pay"), "{text}");
    assert!(!text.contains("Pay online"), "{text}");
    assert!(!text.contains(" due "), "no due date is stated: {text}");

    invoice.lang = "fr-FR".to_string();
    let text = render_text(&invoice);
    assert!(text.contains("Rien à payer"), "{text}");
    assert!(!text.contains("Payer en ligne"), "{text}");

    let mut owed = create_minimal_invoice();
    owed.metadata.payment_url = Some("https://pay.example.com/i/1".to_string());
    let text = render_text(&owed);
    assert!(text.contains("Pay online"), "{text}");
    assert!(!text.contains("Nothing to pay"), "{text}");
}

#[test]
fn credit_purchase_lines_are_worded_in_the_documents_language() {
    use meteroid_invoicing::typst_render::{
        CreditPurchaseDetail, CreditPurchaseName, credit_purchase_description, credit_purchase_name,
    };
    assert_eq!(
        credit_purchase_name("fr-FR", CreditPurchaseName::WalletTopUp),
        "Recharge du portefeuille"
    );
    assert_eq!(
        credit_purchase_name("en-US", CreditPurchaseName::AutomaticTopUp),
        "Automatic top-up"
    );
    assert_eq!(
        credit_purchase_description(
            "fr-FR",
            CreditPurchaseDetail::CurrencyWallet { currency: "EUR" }
        ),
        "Crédit prépayé ajouté au portefeuille EUR"
    );
    assert_eq!(
        credit_purchase_description(
            "fr-FR",
            CreditPurchaseDetail::Units {
                credits: Decimal::from(1500),
                unit: "requêtes rapides",
                valid_months: Some(12),
            }
        ),
        "1\u{a0}500 requêtes rapides, valables 12 mois après paiement"
    );
    assert_eq!(
        credit_purchase_description(
            "en-US",
            CreditPurchaseDetail::Units {
                credits: Decimal::from(1500),
                unit: "fast requests",
                valid_months: None,
            }
        ),
        "1,500 fast requests"
    );
    assert_eq!(
        credit_purchase_description("de-DE", CreditPurchaseDetail::ValidMonths(6)),
        "Guthaben gültig für 6 Monate nach Zahlung"
    );
}

/// A draft's restored promotional credit is an estimate, and is worded as one.
#[test]
fn a_draft_note_words_its_promotional_credit_as_to_be_restored() {
    let mut note = promotional_credit_note();
    note.metadata.promotional_credit_restored_estimated = true;
    let text = render_credit_note_text(&note);
    assert!(text.contains("Crédit promotionnel à restituer"), "{text}");
    assert!(!text.contains("Crédit promotionnel restitué"), "{text}");
}
