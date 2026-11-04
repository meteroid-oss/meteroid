//! What EN 16931 actually demands of a Meteroid invoice.
//!
//! These tests are the gap report: each one builds an invoice the way the
//! billing engine builds one and asserts the standard accepts it. A new
//! mandatory term shows up here as a named rule, not as a rejected document at
//! a counterparty weeks later.

use chrono::NaiveDate;
use common_domain::country::CountryCode;
use meteroid_invoicing::einvoicing::{EInvoiceFormat, check, generate, missing_requirements};
use meteroid_invoicing::model::{
    Address, Customer, Flags, Invoice, InvoiceLine, InvoiceMetadata, Organization, PaymentMeans,
    TaxBreakdownItem, TaxExemptionType,
};
use meteroid_money::{Money, iso};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;

fn eur() -> &'static iso::Currency {
    iso::find("EUR").unwrap()
}

fn rate(percent: i64) -> Decimal {
    Decimal::from_f64(percent as f64 / 100.0).unwrap()
}

/// A domestic French SaaS invoice: one subscription line, standard-rated.
fn domestic_invoice() -> Invoice {
    let eur = eur();
    let start = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
    let end = NaiveDate::from_ymd_opt(2026, 3, 31).unwrap();

    Invoice {
        lang: "fr-FR".to_string(),
        organization: Organization {
            logo_src: None,
            name: "Meteroid SAS".to_string(),
            legal_number: Some("889 123 456 00015".to_string()),
            address: Address {
                line1: Some("12 rue de la Paix".to_string()),
                line2: None,
                city: Some("Paris".to_string()),
                country: CountryCode::parse_as_opt("FR"),
                state: None,
                zip_code: Some("75002".to_string()),
            },
            email: Some("billing@meteroid.com".to_string()),
            contact_name: None,
            contact_phone: None,
            tax_id: Some("FR12889123456".to_string()),
            footer_info: None,
            footer_legal: None,
            accounting_currency: *eur,
            exchange_rate: None,
        },
        customer: Customer {
            name: "Client SARL".to_string(),
            legal_number: Some("552 100 554 00023".to_string()),
            buyer_reference: None,
            address: Address {
                line1: Some("3 avenue des Champs".to_string()),
                line2: None,
                city: Some("Lyon".to_string()),
                country: CountryCode::parse_as_opt("FR"),
                state: None,
                zip_code: Some("69001".to_string()),
            },
            email: Some("compta@client.fr".to_string()),
            tax_id: Some("FR40552100554".to_string()),
        },
        metadata: InvoiceMetadata {
            number: "INV-2026-0042".to_string(),
            issue_date: NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
            payment_term: 30,
            subtotal: Money::from_major(1000, eur),
            tax_amount: Money::from_major(200, eur),
            total_amount: Money::from_major(1200, eur),
            discount: Money::from_major(0, eur),
            promotional_credit: Money::from_major(0, eur),
            quoted_promotional_credit: None,
            prepaid_amount: Money::from_major(0, eur),
            currency: eur,
            due_date: NaiveDate::from_ymd_opt(2026, 5, 1).unwrap(),
            memo: Some("Merci de votre confiance".to_string()),
            payment_url: None,
            flags: Flags::default(),
            purchase_order: Some("PO-99".to_string()),
            parent_invoice_number: None,
            parent_invoice_date: None,
        },
        lines: vec![InvoiceLine {
            name: "Scale plan".to_string(),
            description: Some("Monthly subscription".to_string()),
            subtotal: Money::from_major(1000, eur),
            taxable_amount: Money::from_major(1000, eur),
            quantity: Some(Decimal::from(10)),
            unit_price: Some(Money::from_major(100, eur)),
            tax_rate: rate(20),
            start_date: start,
            end_date: end,
            is_prorated: false,
            sub_lines: vec![],
            group_label: None,
            quoted: None,
            credit: None,
            promotional_credit: None,
            prepaid_credit: false,
        }],
        coupons: vec![],
        payment_status: None,
        transactions: vec![],
        bank_details: None,
        payment_means: Some(PaymentMeans {
            account_identifier: "FR7630006000011234567890189".to_string(),
            is_iban: true,
            bic: Some("AGRIFRPP".to_string()),
            account_name: Some("Meteroid SAS".to_string()),
            reference: Some("INV-2026-0042".to_string()),
        }),
        tax_breakdown: vec![TaxBreakdownItem {
            name: "TVA 20%".to_string(),
            tax_kind: None,
            rate: rate(20),
            amount: Money::from_major(200, eur),
            taxable_amount: Money::from_major(1000, eur),
            exemption_type: None,
            supply_basis: None,
            exemption_reason: None,
            prepaid_credit: false,
        }],
        annexes: vec![],
        tax_inclusive: false,
        prepaid_credit_purchase: false,
    }
}

/// The same invoice sold cross-border to a VAT-registered EU business: no VAT,
/// and the reverse-charge mention the buyer accounts against.
fn reverse_charge_invoice() -> Invoice {
    let eur = eur();
    let mut invoice = domestic_invoice();

    invoice.customer.name = "Kunde GmbH".to_string();
    invoice.customer.address.city = Some("Berlin".to_string());
    invoice.customer.address.zip_code = Some("10115".to_string());
    invoice.customer.address.country = CountryCode::parse_as_opt("DE");
    invoice.customer.tax_id = Some("DE123456789".to_string());

    invoice.lines[0].tax_rate = Decimal::ZERO;
    invoice.metadata.tax_amount = Money::from_major(0, eur);
    invoice.metadata.total_amount = Money::from_major(1000, eur);
    invoice.tax_breakdown = vec![TaxBreakdownItem {
        name: "Reverse charge".to_string(),
        tax_kind: None,
        rate: Decimal::ZERO,
        amount: Money::from_major(0, eur),
        taxable_amount: Money::from_major(1000, eur),
        exemption_type: Some(TaxExemptionType::ReverseCharge),
        supply_basis: None,
        exemption_reason: Some("Autoliquidation — article 283-2 du CGI".to_string()),
        prepaid_credit: false,
    }];
    invoice
}

/// BT-120 is the exemption reason a tax authority reads, and under Factur-X the XML is
/// the authoritative half of the document. It must therefore state the same position
/// the PDF prints, in the same language — not a hardcoded English category label.
#[test]
fn the_exemption_reason_states_what_the_document_states() {
    let mut invoice = reverse_charge_invoice();
    // No merchant override: the document's own statutory wording must carry BT-120.
    invoice.tax_breakdown[0].exemption_reason = None;

    invoice.lang = "fr-FR".to_string();
    let french = generate(&invoice, EInvoiceFormat::FacturX).expect("a document");
    assert!(
        french.xml.contains("Autoliquidation"),
        "BT-120 did not carry the French mandated mention:\n{}",
        french.xml
    );
    assert!(
        !french.xml.contains("Reverse charge"),
        "BT-120 fell back to the hardcoded English default on a French document:\n{}",
        french.xml
    );

    // Same invoice, German: the reason follows the document's language.
    invoice.lang = "de-DE".to_string();
    let german = generate(&invoice, EInvoiceFormat::FacturX).expect("a document");
    assert!(
        german
            .xml
            .contains("Steuerschuldnerschaft des Leistungsempf"),
        "BT-120 did not carry the German mandated mention:\n{}",
        german.xml
    );

    // A merchant who configured their own mention still wins over both.
    invoice.tax_breakdown[0].exemption_reason = Some("Ma propre mention".to_string());
    invoice.lang = "fr-FR".to_string();
    let overridden = generate(&invoice, EInvoiceFormat::FacturX).expect("a document");
    assert!(
        overridden.xml.contains("Ma propre mention"),
        "{}",
        overridden.xml
    );
}

/// A cross-border service is not an exportation. UNCL 5305 "G" is the art. 146 goods
/// regime and BR-G-02 demands a seller VAT identifier; a service whose place of supply
/// is outside the seller's territory is outside the scope of that VAT — "O".
#[test]
fn a_cross_border_service_is_outside_scope_not_an_export() {
    let mut invoice = reverse_charge_invoice();
    invoice.customer.address.country = CountryCode::parse_as_opt("US");
    invoice.customer.tax_id = None;
    invoice.tax_breakdown[0].exemption_type = Some(TaxExemptionType::Export);
    invoice.tax_breakdown[0].exemption_reason = None;

    let doc = generate(&invoice, EInvoiceFormat::FacturX).expect("a document");
    assert!(
        doc.xml.contains("VATEX-EU-O"),
        "expected the outside-scope justification:\n{}",
        doc.xml
    );
    assert!(
        !doc.xml.contains("VATEX-EU-G"),
        "a service was described as a goods export:\n{}",
        doc.xml
    );
}

/// Guyane and Mayotte sit inside France's VAT territory, but VAT does not apply there
/// (CGI art. 294-1). That is a determinate position — "O", not the undetermined
/// refusal — and BT-120 must cite art. 294-1, not the place-of-supply article an
/// export cites: the supply never left the territory, and the XML states what the
/// PDF prints.
#[test]
fn a_supply_in_guyane_is_outside_scope_citing_art_294() {
    let mut invoice = reverse_charge_invoice();
    invoice.customer.address.country = CountryCode::parse_as_opt("GF");
    invoice.customer.tax_id = None;
    invoice.tax_breakdown[0].exemption_type = Some(TaxExemptionType::NoVatTerritory);
    invoice.tax_breakdown[0].exemption_reason = None;
    invoice.lang = "fr-FR".to_string();

    assert_valid(&invoice, EInvoiceFormat::FacturX);

    let doc = generate(&invoice, EInvoiceFormat::FacturX).expect("a document");
    assert!(
        doc.xml.contains("VATEX-EU-O"),
        "expected the outside-scope justification:\n{}",
        doc.xml
    );
    assert!(
        doc.xml.contains("294-1"),
        "BT-120 did not cite art. 294-1:\n{}",
        doc.xml
    );
    assert!(
        !doc.xml.contains("259-1"),
        "BT-120 cited the export's place-of-supply article:\n{}",
        doc.xml
    );
}

/// BR-O-02 and BR-O-04: an out-of-scope document may not carry the seller's or the
/// buyer's VAT identifier. The seller's number still reaches the document, in BT-32.
#[test]
fn an_outside_scope_document_moves_the_seller_number_out_of_bt31() {
    let mut invoice = reverse_charge_invoice();
    invoice.customer.address.country = CountryCode::parse_as_opt("US");
    invoice.customer.tax_id = Some("US-EIN-123456789".to_string());
    invoice.tax_breakdown[0].exemption_type = Some(TaxExemptionType::Export);

    let doc = generate(&invoice, EInvoiceFormat::FacturX).expect("a document");
    assert!(
        !doc.xml.contains("US-EIN-123456789"),
        "BR-O-04: the buyer VAT identifier must not appear:\n{}",
        doc.xml
    );
    // The seller's own number is still on the document, as a tax registration.
    assert!(
        doc.xml.contains("FR12889123456"),
        "the seller's number should still reach BT-32:\n{}",
        doc.xml
    );
}

/// EN 16931 has no code for "no tax applied and we cannot say why": every UNCL 5305
/// value is a positive legal assertion. The document is refused rather than
/// misdescribed — the PDF invoice is still issued.
#[test]
fn an_undetermined_tax_position_is_refused_rather_than_misdescribed() {
    let mut invoice = reverse_charge_invoice();
    invoice.tax_breakdown[0].exemption_type = Some(TaxExemptionType::NotRegistered);

    let err = generate(&invoice, EInvoiceFormat::FacturX)
        .expect_err("an undetermined position has no honest EN 16931 code");
    let message = err.to_string();
    assert!(
        message.contains("undetermined") || message.contains("did not establish"),
        "the refusal should say why: {message}"
    );
}

/// On a reverse-charge invoice the buyer accounts for the tax, so EN 16931 requires the
/// buyer to be identifiable: BR-AE-02 accepts either the VAT identifier (BT-48) *or* the
/// legal registration identifier (BT-47). Neither is mandatory on its own — but a
/// customer carrying neither cannot be issued a valid e-invoice, and the merchant should
/// learn that from `check` rather than from a rejected document.
#[test]
fn reverse_charge_needs_a_buyer_vat_number_or_a_legal_number() {
    let fatal_rules = |vat: Option<&str>, legal: Option<&str>| {
        let mut invoice = reverse_charge_invoice();
        invoice.customer.tax_id = vat.map(str::to_owned);
        invoice.customer.legal_number = legal.map(str::to_owned);
        let report = check(&invoice, EInvoiceFormat::FacturX).expect("the invoice maps");
        report
            .fatal()
            .map(|f| f.rule.to_string())
            .collect::<Vec<_>>()
    };

    // Either identifier satisfies the rule; the legal number is a genuine alternative,
    // not a lesser one.
    assert!(fatal_rules(Some("DE123456789"), Some("HRB 12345")).is_empty());
    assert!(fatal_rules(Some("DE123456789"), None).is_empty());
    assert!(fatal_rules(None, Some("HRB 12345")).is_empty());

    // Neither: the document is not issuable, and the report says which rule fails.
    assert_eq!(fatal_rules(None, None), vec!["BR-AE-02"]);
}

fn assert_valid(invoice: &Invoice, format: EInvoiceFormat) {
    let report = check(invoice, format).expect("the invoice maps onto the semantic model");
    let fatal: Vec<String> = report
        .fatal()
        .map(|f| format!("{} at {} — {}", f.rule, f.path, f.message))
        .collect();
    assert!(
        fatal.is_empty(),
        "{} rejected the invoice:\n  {}",
        format.profile_name(),
        fatal.join("\n  ")
    );
}

#[test]
fn domestic_invoice_is_a_valid_facturx_document() {
    assert_valid(&domestic_invoice(), EInvoiceFormat::FacturX);
}

#[test]
fn reverse_charge_invoice_is_a_valid_facturx_document() {
    assert_valid(&reverse_charge_invoice(), EInvoiceFormat::FacturX);
}

#[test]
fn corrected_invoice_carries_its_predecessor() {
    let mut invoice = domestic_invoice();
    invoice.metadata.number = "INV-2026-0043".to_string();
    invoice.metadata.parent_invoice_number = Some("INV-2026-0042".to_string());
    invoice.metadata.parent_invoice_date = NaiveDate::from_ymd_opt(2026, 4, 1);

    assert_valid(&invoice, EInvoiceFormat::FacturX);

    let doc = generate(&invoice, EInvoiceFormat::FacturX).expect("a document");
    assert!(doc.xml.contains("384"), "a correction is type code 384");
    assert!(
        doc.xml.contains("INV-2026-0042"),
        "BT-25 names the original"
    );
}

#[test]
fn a_discounted_line_states_where_the_difference_went() {
    let eur = eur();
    let mut invoice = domestic_invoice();
    invoice.lines[0].taxable_amount = Money::from_major(900, eur);
    invoice.metadata.discount = Money::from_major(100, eur);
    invoice.metadata.tax_amount = Money::from_major(180, eur);
    invoice.metadata.total_amount = Money::from_major(1080, eur);
    invoice.tax_breakdown[0].amount = Money::from_major(180, eur);
    invoice.tax_breakdown[0].taxable_amount = Money::from_major(900, eur);

    assert_valid(&invoice, EInvoiceFormat::FacturX);
}

/// The line allowances (BG-27) of a Factur-X document, read back: (reason, BT-140 code, amount).
fn line_allowances(xml: &str) -> Vec<(Option<String>, Option<String>, String)> {
    en16931_formats::cii::from_str(xml)
        .expect("the document reads back")
        .invoice
        .lines
        .iter()
        .flat_map(|line| line.allowances.iter())
        .map(|allowance| {
            (
                allowance.reason.clone(),
                allowance
                    .reason_code
                    .as_ref()
                    .map(|code| code.as_str().to_owned()),
                format!("{:?}", allowance.amount),
            )
        })
        .collect()
}

fn amount_of(value: &str) -> String {
    format!(
        "{:?}",
        en16931::InvoiceAmount::parse(value).expect("an amount")
    )
}

#[test]
fn a_promotional_credit_reconciles_as_a_line_allowance() {
    let eur = eur();
    let mut invoice = domestic_invoice();
    invoice.lines[0].taxable_amount = Money::from_major(750, eur);
    invoice.lines[0].promotional_credit = Some(Money::from_major(250, eur));
    invoice.metadata.promotional_credit = Money::from_major(250, eur);
    invoice.metadata.tax_amount = Money::from_major(150, eur);
    invoice.metadata.total_amount = Money::from_major(900, eur);
    invoice.tax_breakdown[0].amount = Money::from_major(150, eur);
    invoice.tax_breakdown[0].taxable_amount = Money::from_major(750, eur);

    assert_valid(&invoice, EInvoiceFormat::FacturX);
    let doc = generate(&invoice, EInvoiceFormat::FacturX).expect("a document");
    assert_eq!(
        line_allowances(&doc.xml),
        vec![(
            Some("Promotional credit".to_owned()),
            Some("100".to_owned()),
            amount_of("250.00")
        )],
        "a special rebate (UNCL 5189 100), not a discount nor a prepayment"
    );
}

/// A line both discounted and reduced by promotional credit states each share apart.
#[test]
fn a_discount_and_a_promotional_credit_are_separate_allowances() {
    let eur = eur();
    let mut invoice = domestic_invoice();
    invoice.lines[0].taxable_amount = Money::from_major(650, eur);
    invoice.lines[0].promotional_credit = Some(Money::from_major(250, eur));
    invoice.metadata.discount = Money::from_major(100, eur);
    invoice.metadata.promotional_credit = Money::from_major(250, eur);
    invoice.metadata.tax_amount = Money::from_major(130, eur);
    invoice.metadata.total_amount = Money::from_major(780, eur);
    invoice.tax_breakdown[0].amount = Money::from_major(130, eur);
    invoice.tax_breakdown[0].taxable_amount = Money::from_major(650, eur);

    assert_valid(&invoice, EInvoiceFormat::FacturX);
    let doc = generate(&invoice, EInvoiceFormat::FacturX).expect("a document");
    assert_eq!(
        line_allowances(&doc.xml),
        vec![
            (Some("Discount".to_owned()), None, amount_of("100.00")),
            (
                Some("Promotional credit".to_owned()),
                Some("100".to_owned()),
                amount_of("250.00")
            ),
        ]
    );
}

/// A line promotional credit took whole still states its rate, at nothing, and so does the
/// breakdown (BR-S-01); a reverse-charge one keeps its exemption and its mention.
#[test]
fn a_line_promotional_credit_took_whole_keeps_its_tax_position() {
    let eur = eur();
    for mut invoice in [domestic_invoice(), reverse_charge_invoice()] {
        invoice.lines[0].taxable_amount = Money::from_major(0, eur);
        invoice.lines[0].promotional_credit = Some(Money::from_major(1000, eur));
        invoice.metadata.promotional_credit = Money::from_major(1000, eur);
        invoice.metadata.tax_amount = Money::from_major(0, eur);
        invoice.metadata.total_amount = Money::from_major(0, eur);
        invoice.tax_breakdown[0].amount = Money::from_major(0, eur);
        invoice.tax_breakdown[0].taxable_amount = Money::from_major(0, eur);
        assert_valid(&invoice, EInvoiceFormat::FacturX);
    }
}

/// A line beside the subscription, at `rate`, with its own net amount.
fn extra_line(name: &str, net: i64, rate: Decimal) -> InvoiceLine {
    let eur = eur();
    let mut line = domestic_invoice().lines.remove(0);
    line.name = name.to_string();
    line.description = None;
    line.subtotal = Money::from_major(net, eur);
    line.taxable_amount = Money::from_major(net, eur);
    line.quantity = Some(Decimal::ONE);
    line.unit_price = Some(Money::from_major(net, eur));
    line.tax_rate = rate;
    line
}

/// A wallet top-up: currency credit sold, taxed when it is used.
fn top_up_line(net: i64) -> InvoiceLine {
    let mut line = extra_line("Wallet top-up", net, Decimal::ZERO);
    line.prepaid_credit = true;
    line
}

/// What the billing engine states for credit sold and credit spent: a top-up (currency credit,
/// taxed when used) is a zero-rated line with its own row; a credit-unit pack is taxed at sale
/// like any line; a line promotional credit paid whole keeps its rate and its row at nothing.
#[test]
fn credit_sold_and_spent_states_every_line_category() {
    let eur = eur();
    let zero_row = |taxable: i64| TaxBreakdownItem {
        name: "Tax".to_string(),
        tax_kind: None,
        rate: Decimal::ZERO,
        amount: Money::from_major(0, eur),
        taxable_amount: Money::from_major(taxable, eur),
        exemption_type: None,
        supply_basis: None,
        exemption_reason: None,
        prepaid_credit: true,
    };

    // A top-up alone: its zero row is the document's only VAT breakdown (BR-CO-18).
    let mut top_up = domestic_invoice();
    top_up.lines = vec![top_up_line(50)];
    top_up.prepaid_credit_purchase = true;
    top_up.metadata.subtotal = Money::from_major(50, eur);
    top_up.metadata.tax_amount = Money::from_major(0, eur);
    top_up.metadata.total_amount = Money::from_major(50, eur);
    top_up.tax_breakdown = vec![zero_row(50)];
    assert_valid(&top_up, EInvoiceFormat::FacturX);

    // A top-up beside the subscription: one row per category.
    let mixed = || {
        let mut mixed = domestic_invoice();
        mixed
            .lines
            .push(extra_line("Wallet top-up", 50, Decimal::ZERO));
        mixed.prepaid_credit_purchase = true;
        mixed.metadata.subtotal = Money::from_major(1050, eur);
        mixed.metadata.total_amount = Money::from_major(1250, eur);
        mixed.tax_breakdown.push(zero_row(50));
        mixed
    };
    assert_valid(&mixed(), EInvoiceFormat::FacturX);

    // A credit-unit pack is taxed at sale: a standard-rated line in the subscription's row.
    let mut pack = domestic_invoice();
    pack.lines
        .push(extra_line("5,000 Halyard credits", 50, rate(20)));
    pack.metadata.subtotal = Money::from_major(1050, eur);
    pack.metadata.tax_amount = Money::from_major(210, eur);
    pack.metadata.total_amount = Money::from_major(1260, eur);
    pack.tax_breakdown[0].amount = Money::from_major(210, eur);
    pack.tax_breakdown[0].taxable_amount = Money::from_major(1050, eur);
    assert_valid(&pack, EInvoiceFormat::FacturX);

    // The subscription paid whole by promotional credit beside a top-up: the 20% row at nothing
    // and the top-up's zero row.
    let mut promoted = mixed();
    promoted.lines[0].taxable_amount = Money::from_major(0, eur);
    promoted.lines[0].promotional_credit = Some(Money::from_major(1000, eur));
    promoted.metadata.promotional_credit = Money::from_major(1000, eur);
    promoted.metadata.tax_amount = Money::from_major(0, eur);
    promoted.metadata.total_amount = Money::from_major(50, eur);
    promoted.tax_breakdown[0].amount = Money::from_major(0, eur);
    promoted.tax_breakdown[0].taxable_amount = Money::from_major(0, eur);
    assert_valid(&promoted, EInvoiceFormat::FacturX);

    // Sold to a reverse-charge buyer: the top-up's row carries the document's exemption, apart
    // from the supply's in our breakdown, and the two join one AE group of 1050.
    let mut reverse_charge = reverse_charge_invoice();
    reverse_charge.lines.push(top_up_line(50));
    reverse_charge.prepaid_credit_purchase = true;
    reverse_charge.metadata.subtotal = Money::from_major(1050, eur);
    reverse_charge.metadata.total_amount = Money::from_major(1050, eur);
    let sold = TaxBreakdownItem {
        name: "Reverse charge".to_string(),
        tax_kind: None,
        rate: Decimal::ZERO,
        amount: Money::from_major(0, eur),
        taxable_amount: Money::from_major(50, eur),
        exemption_type: Some(TaxExemptionType::ReverseCharge),
        supply_basis: None,
        exemption_reason: Some("Autoliquidation — article 283-2 du CGI".to_string()),
        prepaid_credit: true,
    };
    reverse_charge.tax_breakdown.push(sold);
    assert_valid(&reverse_charge, EInvoiceFormat::FacturX);

    // Without the zero row the top-up's category is stated nowhere, which a receiver rejects.
    let mut missing = mixed();
    missing.tax_breakdown.pop();
    let report = check(&missing, EInvoiceFormat::FacturX).expect("maps");
    assert!(report.fatal().next().is_some(), "a Z line with no Z row");
}

#[test]
fn a_prepaid_invoice_states_only_what_is_left_to_pay() {
    let eur = eur();
    let mut invoice = domestic_invoice();
    invoice.metadata.prepaid_amount = Money::from_major(200, eur);

    assert_valid(&invoice, EInvoiceFormat::FacturX);
    let doc = generate(&invoice, EInvoiceFormat::FacturX).expect("a document");
    assert!(
        doc.xml.contains("1000.00"),
        "BT-115 is the balance, not the total"
    );
}

#[test]
fn generated_facturx_is_cii_under_the_en16931_profile() {
    let doc = generate(&domestic_invoice(), EInvoiceFormat::FacturX).expect("a document");

    assert_eq!(doc.filename, Some("factur-x.xml"));
    assert!(
        doc.xml.contains("CrossIndustryInvoice"),
        "Factur-X payloads are CII"
    );
    assert!(
        doc.xml.contains("urn:cen.eu:en16931:2017"),
        "BT-24 states the profile that was proved"
    );
    assert!(
        doc.dropped.is_empty(),
        "CII dropped terms the model carried: {:?}",
        doc.dropped
    );
}

#[test]
fn peppol_is_written_as_ubl_and_is_not_embedded() {
    let format = EInvoiceFormat::PeppolBis;
    assert_eq!(format.attachment_filename(), None);
    assert!(!format.is_hybrid());
}

#[test]
fn an_invoice_missing_a_country_names_the_term_rather_than_failing_opaquely() {
    let mut invoice = domestic_invoice();
    invoice.customer.address.country = None;

    let report = check(&invoice, EInvoiceFormat::FacturX).expect("still maps");
    let fatal: Vec<&str> = report.fatal().map(|f| f.rule.as_str()).collect();
    assert!(
        fatal.contains(&"BR-11"),
        "a missing buyer country is BR-11, got {fatal:?}"
    );
    // The rejection carries the same findings, which is what gets stored against
    // the invoice — the invoice itself is still issued.
    let rejection = generate(&invoice, EInvoiceFormat::FacturX).expect_err("no document");
    let terms: Vec<&str> = rejection
        .findings()
        .iter()
        .map(|f| f.term.as_str())
        .collect();
    assert!(
        terms.iter().any(|t| t.contains("BT-55")),
        "the finding names the field to fill, got {terms:?}"
    );
    assert_eq!(rejection.error(), None, "a rule failure is not an error");
}

/// The rules French law adds, as (rule, term) pairs — what the draft reports and
/// what finalization records.
fn french_findings(invoice: &Invoice) -> Vec<(String, String)> {
    missing_requirements(invoice, EInvoiceFormat::FacturX)
        .expect("maps")
        .into_iter()
        .filter(|f| f.rule.starts_with("FR-"))
        .map(|f| (f.rule, f.term))
        .collect()
}

fn pair(rule: &str, term: &str) -> (String, String) {
    (rule.to_owned(), term.to_owned())
}

/// EN 16931 accepts a seller identified by VAT number alone (BR-CO-26). French law
/// does not: the SIREN goes on every invoice (Code de commerce, art. R123-237). The
/// standard passes this document; the national layer must not.
#[test]
fn a_french_seller_without_a_siren_is_not_issued() {
    let mut invoice = domestic_invoice();
    invoice.organization.legal_number = None;

    assert!(
        check(&invoice, EInvoiceFormat::FacturX)
            .unwrap()
            .fatal()
            .next()
            .is_none(),
        "the standard alone accepts it — which is the gap"
    );
    assert_eq!(
        french_findings(&invoice),
        vec![pair("FR-R123-237", "BG-4/BT-30")]
    );

    // Finalization records the same finding: the gate and the draft agree.
    let rejection = generate(&invoice, EInvoiceFormat::FacturX).expect_err("no document");
    assert!(rejection.findings().iter().any(|f| f.rule == "FR-R123-237"));

    // A number that is not a SIREN or SIRET is reported too, not silently accepted.
    invoice.organization.legal_number = Some("RCS Paris".to_string());
    assert_eq!(
        french_findings(&invoice),
        vec![pair("FR-R123-237", "BG-4/BT-30")]
    );
}

/// A buyer address reduced to its country is valid EN 16931 (BR-10/BR-11 ask for
/// the country only) and not a valid French invoice, which states the customer's
/// full address. Each missing field is its own finding, naming what to fill.
#[test]
fn a_buyer_address_reduced_to_its_country_is_reported_field_by_field() {
    let mut invoice = domestic_invoice();
    invoice.customer.address.line1 = Some(String::new());
    invoice.customer.address.city = Some("  ".to_string());
    invoice.customer.address.zip_code = None;

    assert!(
        check(&invoice, EInvoiceFormat::FacturX)
            .unwrap()
            .fatal()
            .next()
            .is_none()
    );
    assert_eq!(
        french_findings(&invoice),
        vec![
            pair("FR-242nA", "BG-7/BT-50"),
            pair("FR-242nA", "BG-7/BT-52"),
            pair("FR-242nA", "BG-7/BT-53"),
        ]
    );
}

/// A foreign customer is asked for a street and a city, not a postcode: some
/// countries have none, and the law asks for the address, not for a French one.
#[test]
fn a_foreign_buyer_is_not_asked_for_a_postcode() {
    let mut invoice = reverse_charge_invoice();
    invoice.customer.address.country = CountryCode::parse_as_opt("IT");
    invoice.customer.address.zip_code = None;

    assert_eq!(french_findings(&invoice), vec![]);
    generate(&invoice, EInvoiceFormat::FacturX).expect("a complete document");
}

/// French law binds French sellers. A German seller with no registration number and
/// a sparse buyer address is judged by the standard alone.
#[test]
fn the_french_rules_do_not_reach_a_seller_outside_france() {
    let mut invoice = domestic_invoice();
    invoice.organization.address.country = CountryCode::parse_as_opt("DE");
    invoice.organization.tax_id = Some("DE123456789".to_string());
    invoice.organization.legal_number = None;
    invoice.customer.address.city = None;

    assert_eq!(french_findings(&invoice), vec![]);
}

/// An amount the standard cannot express is a different kind of rejection: no
/// business rule was broken, so there is nothing to name a field for.
#[test]
fn an_unmappable_invoice_is_reported_as_an_error_not_as_findings() {
    use meteroid_invoicing::einvoicing::EInvoiceRejection;

    let mut invoice = domestic_invoice();
    // Three decimals: representable as money, not as an EN 16931 amount.
    invoice.lines[0].taxable_amount =
        Money::from_decimal(Decimal::from_f64(1000.125).unwrap(), eur());

    let rejection = generate(&invoice, EInvoiceFormat::FacturX).expect_err("no document");
    assert!(matches!(rejection, EInvoiceRejection::Unmappable(_)));
    assert!(rejection.error().is_some());
    assert!(rejection.findings().is_empty());
}

/// The half a Rust test can check about a hybrid PDF: that the payload is
/// discoverable the way a receiver discovers it, and wired in the way PDF/A-3
/// requires. Conformance itself is veraPDF's answer, not this test's.
#[test]
fn the_pdf_carries_the_xml_as_an_associated_file() {
    use meteroid_invoicing::pdf::{PdfGenerator, TypstPdfGenerator};
    use meteroid_invoicing::typst_render::EInvoiceAttachment;

    let invoice = domestic_invoice();
    let doc = generate(&invoice, EInvoiceFormat::FacturX).expect("a document");
    let attachment = EInvoiceAttachment {
        filename: doc.filename.unwrap().to_string(),
        xml: doc.xml.into_bytes(),
        description: "Factur-X invoice".to_string(),
    };

    let generator = TypstPdfGenerator::new().expect("a generator");
    let pdf = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(generator.generate_pdf(&invoice, Some(&attachment)))
        .expect("a hybrid PDF");

    let stamped = meteroid_invoicing::einvoicing::stamp_facturx_xmp(
        &pdf,
        EInvoiceFormat::FacturX,
        "factur-x.xml",
    )
    .expect("the XMP is stampable");

    let extracted =
        en16931_formats::zugferd::extract(&stamped).expect("the payload comes back out");
    assert_eq!(extracted.filename, "factur-x.xml");
    assert!(
        extracted.divergence.is_empty(),
        "the payload is wired in wrongly: {:?}",
        extracted.divergence
    );
    // Only reachable once the XMP declares a profile at all: before stamping,
    // there is nothing for the reader to disagree with.
    assert_eq!(
        extracted.profile,
        en16931_formats::zugferd::Profile::En16931,
        "the XMP must announce the profile the payload proves"
    );

    let read = en16931_formats::cii::from_str(&extracted.xml).expect("readable CII");
    assert!(en16931::validate(&read.invoice).is_valid());
}

/// Writes a hybrid PDF to `FACTURX_OUT` for conformance checking with veraPDF,
/// which is the only authority on PDF/A and lives outside `cargo test`.
#[test]
#[ignore = "writes a file for an external validator"]
fn write_facturx_sample() {
    use meteroid_invoicing::pdf::{PdfGenerator, TypstPdfGenerator};
    use meteroid_invoicing::typst_render::EInvoiceAttachment;

    let path = std::env::var("FACTURX_OUT").expect("set FACTURX_OUT to the target path");
    let invoice = domestic_invoice();
    let doc = generate(&invoice, EInvoiceFormat::FacturX).unwrap();
    let attachment = EInvoiceAttachment {
        filename: doc.filename.unwrap().to_string(),
        xml: doc.xml.into_bytes(),
        description: "Structured invoice data (EN 16931)".to_string(),
    };
    let pdf = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(
            TypstPdfGenerator::new()
                .unwrap()
                .generate_pdf(&invoice, Some(&attachment)),
        )
        .unwrap();
    let stamped = meteroid_invoicing::einvoicing::stamp_facturx_xmp(
        &pdf,
        EInvoiceFormat::FacturX,
        "factur-x.xml",
    )
    .unwrap();
    std::fs::write(&path, stamped).unwrap();

    // The plain path must stay conformant too — it now carries a timestamp it
    // did not before, which PDF/A has opinions about.
    let plain = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(
            TypstPdfGenerator::new()
                .unwrap()
                .generate_pdf(&invoice, None),
        )
        .unwrap();
    std::fs::write(format!("{path}.plain.pdf"), plain).unwrap();

    // And the credit note, which travels the same way.
    use meteroid_invoicing::einvoicing::generate_credit_note;
    use meteroid_invoicing::pdf::{CreditNotePdfGenerator, TypstCreditNotePdfGenerator};
    let credit_note = credit_notes::credit_note();
    let cn_doc = generate_credit_note(&credit_note, EInvoiceFormat::FacturX).unwrap();
    let cn_attachment = EInvoiceAttachment {
        filename: cn_doc.filename.unwrap().to_string(),
        xml: cn_doc.xml.into_bytes(),
        description: "Structured credit note data (EN 16931)".to_string(),
    };
    let cn_pdf = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(
            TypstCreditNotePdfGenerator::new()
                .unwrap()
                .generate_credit_note_pdf(&credit_note, Some(&cn_attachment)),
        )
        .unwrap();
    let cn_stamped = meteroid_invoicing::einvoicing::stamp_facturx_xmp(
        &cn_pdf,
        EInvoiceFormat::FacturX,
        "factur-x.xml",
    )
    .unwrap();
    std::fs::write(format!("{path}.credit-note.pdf"), cn_stamped).unwrap();
}

mod credit_notes {
    use super::*;
    use meteroid_invoicing::credit_note_model::{
        CreditNote, CreditNoteLine, CreditNoteMetadata, CreditType, Flags as CnFlags,
    };
    use meteroid_invoicing::einvoicing::generate_credit_note;

    /// A credit note as the store builds one: header totals positive, lines and
    /// tax negated for the rendered document.
    pub(super) fn credit_note() -> CreditNote {
        let eur = eur();
        let invoice = domestic_invoice();

        CreditNote {
            lang: "fr-FR".to_string(),
            organization: invoice.organization,
            customer: invoice.customer,
            metadata: CreditNoteMetadata {
                number: "CN-2026-0007".to_string(),
                issue_date: NaiveDate::from_ymd_opt(2026, 4, 15).unwrap(),
                related_invoice_number: "INV-2026-0042".to_string(),
                related_invoice_date: NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
                subtotal: Money::from_major(1000, eur),
                tax_amount: Money::from_major(200, eur),
                total_amount: Money::from_major(1200, eur),
                currency: eur,
                reason: Some("Service interruption".to_string()),
                memo: None,
                credit_type: CreditType::Refund,
                refunded_amount: Money::from_major(1200, eur),
                credited_amount: Money::from_major(0, eur),
                debt_cancelled_amount: Money::from_major(0, eur),
                promotional_credit_restored: Money::from_major(0, eur),
                promotional_credit_restored_estimated: false,
                flags: CnFlags::default(),
            },
            lines: vec![CreditNoteLine {
                name: "Scale plan".to_string(),
                description: None,
                subtotal: Money::from_major(-1000, eur),
                taxable_amount: Money::from_major(-1000, eur),
                quantity: Some(Decimal::from(10)),
                unit_price: Some(Money::from_major(100, eur)),
                tax_rate: rate(20),
                start_date: NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
                end_date: NaiveDate::from_ymd_opt(2026, 3, 31).unwrap(),
                is_prorated: false,
                sub_lines: vec![],
                promotional_credit: None,
            }],
            tax_breakdown: vec![TaxBreakdownItem {
                name: "TVA 20%".to_string(),
                tax_kind: None,
                rate: rate(20),
                amount: Money::from_major(-200, eur),
                taxable_amount: Money::from_major(-1000, eur),
                exemption_type: None,
                supply_basis: None,
                exemption_reason: None,
                prepaid_credit: false,
            }],
        }
    }

    /// A credit note carries the same mentions as the invoice it corrects, so the
    /// national rules gate it the same way.
    #[test]
    fn a_credit_note_from_a_french_seller_without_a_siren_is_not_issued() {
        let mut credit_note = credit_note();
        credit_note.organization.legal_number = None;
        let rejection =
            generate_credit_note(&credit_note, EInvoiceFormat::FacturX).expect_err("no document");
        assert!(
            rejection.findings().iter().any(|f| f.rule == "FR-R123-237"),
            "{:?}",
            rejection.findings()
        );
    }

    #[test]
    fn a_credit_note_is_a_valid_facturx_document() {
        let doc = generate_credit_note(&credit_note(), EInvoiceFormat::FacturX)
            .unwrap_or_else(|rejection| panic!("rejected: {rejection}"));

        assert!(doc.xml.contains("381"), "BT-3 is 381 for a credit note");
        assert!(
            doc.xml
                .contains(r#"<qdt:DateTimeString format="102">20260401</qdt:DateTimeString>"#),
            "BT-26 is a qualified date in CII, or the XSD rejects it"
        );
        assert!(
            doc.xml.contains("INV-2026-0042"),
            "BG-3 names the invoice being credited"
        );
    }

    /// A note reversing a line promotional credit took part of gives that share back as the
    /// same allowance (UNCL 5189 100), not as a discount.
    #[test]
    fn a_credit_note_reverses_promotional_credit_as_its_own_allowance() {
        let eur = eur();
        let mut note = credit_note();
        note.lines[0].taxable_amount = Money::from_major(-750, eur);
        note.lines[0].promotional_credit = Some(Money::from_major(-250, eur));
        note.metadata.tax_amount = Money::from_major(150, eur);
        note.metadata.total_amount = Money::from_major(900, eur);
        note.metadata.refunded_amount = Money::from_major(900, eur);
        note.tax_breakdown[0].amount = Money::from_major(-150, eur);
        note.tax_breakdown[0].taxable_amount = Money::from_major(-750, eur);
        let doc = generate_credit_note(&note, EInvoiceFormat::FacturX)
            .unwrap_or_else(|rejection| panic!("rejected: {rejection}"));
        assert_eq!(
            super::line_allowances(&doc.xml),
            vec![(
                Some("Promotional credit".to_owned()),
                Some("100".to_owned()),
                super::amount_of("250.00")
            )]
        );
    }

    /// QA M8 (CN-000001), stated negative as the store writes it: 50.00 less 5.00 of
    /// promotional credit, taxed 9.00 at 20%, 54.00. The PDF summary and the XML state the same
    /// figures: the PDF's subtotal and promotional row reach the XML's line total, its tax and
    /// total are the XML's.
    #[test]
    fn the_pdf_summary_of_a_promotional_note_matches_the_xml() {
        use meteroid_invoicing::credit_note_render::TypstCreditNoteContent;
        let eur = eur();
        let money = |minor: i64| Money::from_minor(minor, eur);
        let mut note = credit_note();
        note.metadata.subtotal = money(-5_000);
        note.metadata.tax_amount = money(-900);
        note.metadata.total_amount = money(-5_400);
        note.metadata.refunded_amount = money(3_900);
        note.metadata.credited_amount = money(1_500);
        note.metadata.promotional_credit_restored = money(500);
        note.lines[0].subtotal = money(-5_000);
        note.lines[0].taxable_amount = money(-4_500);
        note.lines[0].quantity = Some(Decimal::ONE);
        note.lines[0].unit_price = Some(money(5_000));
        note.lines[0].promotional_credit = Some(money(-500));
        note.tax_breakdown[0].amount = money(-900);
        note.tax_breakdown[0].taxable_amount = money(-4_500);

        let doc = generate_credit_note(&note, EInvoiceFormat::FacturX)
            .unwrap_or_else(|rejection| panic!("rejected: {rejection}"));
        for term in [
            "<ram:LineTotalAmount>45.00</ram:LineTotalAmount>",
            "<ram:TaxBasisTotalAmount>45.00</ram:TaxBasisTotalAmount>",
            r#"<ram:TaxTotalAmount currencyID="EUR">9.00</ram:TaxTotalAmount>"#,
            "<ram:GrandTotalAmount>54.00</ram:GrandTotalAmount>",
        ] {
            assert!(doc.xml.contains(term), "{term}: {}", doc.xml);
        }
        assert_eq!(
            super::line_allowances(&doc.xml),
            vec![(
                Some("Promotional credit".to_owned()),
                Some("100".to_owned()),
                super::amount_of("5.00")
            )]
        );

        let content = TypstCreditNoteContent::try_from(&note).unwrap();
        let shown = |d: &typst::foundations::Decimal| d.to_string().replace('\u{2212}', "-");
        assert_eq!(shown(&content.subtotal), "-50.00");
        assert_eq!(shown(&content.discount), "0.00");
        assert_eq!(shown(&content.promotional_credit), "5.00");
        assert_eq!(shown(&content.tax_amount), "-9.00");
        assert_eq!(shown(&content.total_amount), "-54.00");
    }

    /// Half of a 100.00 line that took 3.33 of promotional credit, taxed 20%: 50.00 credited down
    /// to 48.34. The 1.66 reduction is one promotional allowance, with no 0.01 discount beside it,
    /// and the PDF summary states the same split.
    #[test]
    fn a_partial_promotional_note_has_no_phantom_discount() {
        use meteroid_invoicing::credit_note_render::TypstCreditNoteContent;
        let eur = eur();
        let money = |minor: i64| Money::from_minor(minor, eur);
        let mut note = credit_note();
        note.metadata.subtotal = money(-5_000);
        note.metadata.tax_amount = money(-967);
        note.metadata.total_amount = money(-5_801);
        note.metadata.refunded_amount = money(5_801);
        note.lines[0].subtotal = money(-5_000);
        note.lines[0].taxable_amount = money(-4_834);
        note.lines[0].quantity = Some(Decimal::ONE);
        note.lines[0].unit_price = Some(money(5_000));
        note.lines[0].promotional_credit = Some(money(-166));
        note.tax_breakdown[0].amount = money(-967);
        note.tax_breakdown[0].taxable_amount = money(-4_834);

        let doc = generate_credit_note(&note, EInvoiceFormat::FacturX)
            .unwrap_or_else(|rejection| panic!("rejected: {rejection}"));
        assert_eq!(
            super::line_allowances(&doc.xml),
            vec![(
                Some("Promotional credit".to_owned()),
                Some("100".to_owned()),
                super::amount_of("1.66")
            )]
        );
        assert!(
            doc.xml
                .contains("<ram:LineTotalAmount>48.34</ram:LineTotalAmount>")
        );

        let content = TypstCreditNoteContent::try_from(&note).unwrap();
        assert_eq!(content.discount.to_string(), "0.00");
        assert_eq!(content.promotional_credit.to_string(), "1.66");

        // A drawn share one cent above the reduction is held to it, never a negative discount.
        note.lines[0].promotional_credit = Some(money(-167));
        let content = TypstCreditNoteContent::try_from(&note).unwrap();
        assert_eq!(content.discount.to_string(), "0.00");
        assert_eq!(content.promotional_credit.to_string(), "1.66");
    }

    /// Usage priced in a credit unit carries its usage quantity but no money price: the money is
    /// what the credits did not cover, so it is one unit at that amount (PEPPOL-EN16931-R120).
    #[test]
    fn a_quantity_without_a_price_is_one_unit_at_the_amount() {
        let mut note = credit_note();
        note.lines[0].quantity = Some(Decimal::from(300));
        note.lines[0].unit_price = None;
        let doc = generate_credit_note(&note, EInvoiceFormat::FacturX)
            .unwrap_or_else(|rejection| panic!("rejected: {rejection}"));
        assert!(
            doc.xml
                .contains(r#"<ram:BilledQuantity unitCode="C62">1</ram:BilledQuantity>"#),
            "{}",
            doc.xml
        );
        assert!(
            doc.xml
                .contains("<ram:ChargeAmount>1000</ram:ChargeAmount>")
        );
    }

    /// 1,000,000 units at an effective 0.000004 make 4.00: BT-146 is written exact, never rounded
    /// to the currency, so quantity × price is the line amount under Peppol's R120. A price
    /// rounded to the cent would miss it by 9,996.00, and the rule says so.
    #[test]
    fn a_tiny_effective_unit_price_is_stated_exact() {
        let eur = eur();
        let mut note = credit_note();
        note.lines[0].subtotal = Money::from_major(-4, eur);
        note.lines[0].taxable_amount = Money::from_major(-4, eur);
        note.lines[0].quantity = Some(Decimal::from(1_000_000));
        note.lines[0].unit_price = Some(Money::from_decimal(Decimal::new(4, 6), eur));
        note.metadata.subtotal = Money::from_major(4, eur);
        note.metadata.tax_amount = Money::from_decimal(Decimal::new(80, 2), eur);
        note.metadata.total_amount = Money::from_decimal(Decimal::new(480, 2), eur);
        note.metadata.refunded_amount = note.metadata.total_amount;
        note.tax_breakdown[0].amount = Money::from_decimal(Decimal::new(-80, 2), eur);
        note.tax_breakdown[0].taxable_amount = Money::from_major(-4, eur);
        let r120 = |note: &CreditNote| match generate_credit_note(note, EInvoiceFormat::PeppolBis) {
            Ok(_) => false,
            Err(rejection) => rejection.findings().iter().any(|f| f.rule.contains("R120")),
        };
        let doc = generate_credit_note(&note, EInvoiceFormat::FacturX)
            .unwrap_or_else(|rejection| panic!("rejected: {rejection}"));
        assert!(
            doc.xml
                .contains("<ram:ChargeAmount>0.000004</ram:ChargeAmount>")
        );
        assert!(!r120(&note), "quantity × exact price is the line amount");

        note.lines[0].unit_price = Some(Money::from_decimal(Decimal::new(1, 2), eur));
        assert!(r120(&note), "a price rounded to the cent breaks R120");
    }

    /// The store negates credit-note lines and tax so the rendered document
    /// shows them as subtractions. EN 16931 states them positive and lets BT-3
    /// carry the direction, so a leaked sign is a `BR-27` rejection.
    #[test]
    fn the_document_states_positive_amounts_whatever_the_render_model_carries() {
        let doc = generate_credit_note(&credit_note(), EInvoiceFormat::FacturX).expect("valid");
        assert!(
            !doc.xml.contains(">-1000.00<") && !doc.xml.contains(">-200.00<"),
            "no negative amount reaches the document"
        );
        assert!(doc.xml.contains("1000.00") && doc.xml.contains("200.00"));
    }
}

/// A German invoice to a public body: the seller names a contact, and the buyer
/// gave a Leitweg-ID to route it by.
fn xrechnung_invoice() -> Invoice {
    let eur = eur();
    let mut invoice = domestic_invoice();
    invoice.lang = "de-DE".to_string();

    invoice.organization.name = "Meteroid GmbH".to_string();
    invoice.organization.legal_number = Some("HRB 123456 B".to_string());
    invoice.organization.address = Address {
        line1: Some("Friedrichstraße 68".to_string()),
        line2: None,
        city: Some("Berlin".to_string()),
        country: CountryCode::parse_as_opt("DE"),
        state: None,
        zip_code: Some("10117".to_string()),
    };
    invoice.organization.tax_id = Some("DE123456789".to_string());
    invoice.organization.contact_name = Some("Buchhaltung".to_string());
    invoice.organization.contact_phone = Some("+49 30 1234567".to_string());

    invoice.customer.name = "Bezirksamt Mitte von Berlin".to_string();
    invoice.customer.legal_number = None;
    invoice.customer.tax_id = None;
    invoice.customer.address = Address {
        line1: Some("Müllerstraße 146".to_string()),
        line2: None,
        city: Some("Berlin".to_string()),
        country: CountryCode::parse_as_opt("DE"),
        state: None,
        zip_code: Some("13353".to_string()),
    };
    invoice.customer.email = Some("rechnung@ba-mitte.berlin.de".to_string());
    invoice.customer.buyer_reference = Some("991-01234-56".to_string());

    invoice.payment_means.as_mut().unwrap().account_identifier =
        "DE89370400440532013000".to_string();
    invoice.lines[0].tax_rate = rate(19);
    invoice.metadata.tax_amount = Money::from_major(190, eur);
    invoice.metadata.total_amount = Money::from_major(1190, eur);
    invoice.tax_breakdown[0].rate = rate(19);
    invoice.tax_breakdown[0].amount = Money::from_major(190, eur);
    invoice
}

#[test]
fn a_german_invoice_is_a_valid_xrechnung_document() {
    assert_valid(&xrechnung_invoice(), EInvoiceFormat::XRechnung);

    let doc = generate(&xrechnung_invoice(), EInvoiceFormat::XRechnung)
        .unwrap_or_else(|rejection| panic!("rejected: {rejection}"));
    assert_eq!(doc.filename, Some("xrechnung.xml"));
    assert!(doc.xml.contains("urn:xeinkauf.de:kosit:xrechnung_3.0"));
    assert!(
        doc.xml.contains("991-01234-56"),
        "BT-10 carries the Leitweg-ID"
    );
    assert!(
        doc.xml
            .contains(r#"schemeID="EM">rechnung@ba-mitte.berlin.de<"#),
        "BT-49 is the buyer's email, schemed EM"
    );
    assert!(
        doc.xml
            .contains("urn:fdc:peppol.eu:2017:poacc:billing:01:1.0")
    );
    assert!(doc.dropped.is_empty(), "dropped: {:?}", doc.dropped);
}

#[test]
fn the_contact_and_buyer_reference_also_reach_a_facturx_document() {
    let doc = generate(&xrechnung_invoice(), EInvoiceFormat::FacturX).expect("valid");
    assert!(doc.xml.contains("991-01234-56"));
    assert!(doc.xml.contains("+49 30 1234567"));
    assert!(
        !doc.xml.contains(r#"schemeID="EM""#),
        "only XRechnung is addressed by email"
    );
}

/// What stops XRechnung for an entity that has not filled the German fields, named
/// where the operator fills them — the rule text itself is German.
#[test]
fn xrechnung_names_the_fields_it_still_needs() {
    let mut invoice = xrechnung_invoice();
    invoice.organization.contact_name = None;
    invoice.organization.contact_phone = None;
    invoice.customer.buyer_reference = None;
    invoice.payment_means = None;

    let findings = missing_requirements(&invoice, EInvoiceFormat::XRechnung).expect("maps");
    let mut rules: Vec<&str> = findings.iter().map(|f| f.rule.as_str()).collect();
    rules.sort();
    assert_eq!(rules, ["BR-DE-1", "BR-DE-15", "BR-DE-5", "BR-DE-6"]);
    assert!(
        findings.iter().all(|f| f.hint.is_some()),
        "every finding says where to fill it: {findings:?}"
    );
}

/// Without an email there is no electronic address either, and the Peppol rules
/// XRechnung merges in say so.
#[test]
fn xrechnung_without_emails_has_no_electronic_address() {
    let mut invoice = xrechnung_invoice();
    invoice.organization.email = None;
    invoice.customer.email = None;

    let findings = missing_requirements(&invoice, EInvoiceFormat::XRechnung).expect("maps");
    let mut rules: Vec<&str> = findings.iter().map(|f| f.rule.as_str()).collect();
    rules.sort();
    assert_eq!(
        rules,
        ["BR-DE-7", "PEPPOL-EN16931-R010", "PEPPOL-EN16931-R020"]
    );
}

/// Peppol BIS routes by participant identifier, which only a Peppol transport can
/// give us, so it stays out of the settings picker. When one arrives this test
/// fails and the picker should regain the option.
#[test]
fn peppol_is_not_satisfiable_without_a_transport() {
    let report = check(&xrechnung_invoice(), EInvoiceFormat::PeppolBis).expect("maps");
    let mut rules: Vec<String> = report.fatal().map(|f| f.rule.to_string()).collect();
    rules.sort();
    assert_eq!(
        rules,
        [
            "PEPPOL-EN16931-R001", // BT-23 business process
            "PEPPOL-EN16931-R010", // BT-49 buyer electronic address
            "PEPPOL-EN16931-R020", // BT-34 seller electronic address
        ]
    );
}

fn xrechnung_credit_note() -> meteroid_invoicing::credit_note_model::CreditNote {
    let invoice = xrechnung_invoice();
    let mut credit_note = credit_notes::credit_note();
    credit_note.lang = invoice.lang;
    credit_note.organization = invoice.organization;
    credit_note.customer = invoice.customer;
    credit_note.lines[0].tax_rate = rate(19);
    credit_note.tax_breakdown[0].rate = rate(19);
    credit_note.tax_breakdown[0].amount = Money::from_major(-190, eur());
    credit_note.metadata.tax_amount = Money::from_major(190, eur());
    credit_note.metadata.total_amount = Money::from_major(1190, eur());
    credit_note
}

/// BR-DE-1 wants payment instructions on a credit note too; they say how the credit
/// is settled rather than naming an account to pay into.
#[test]
fn a_credit_note_is_a_valid_xrechnung_document() {
    let doc = meteroid_invoicing::einvoicing::generate_credit_note(
        &xrechnung_credit_note(),
        EInvoiceFormat::XRechnung,
    )
    .unwrap_or_else(|rejection| panic!("rejected: {rejection}"));
    assert!(doc.xml.contains("991-01234-56"));
    assert!(
        doc.xml.contains("<ram:TypeCode>ZZZ</ram:TypeCode>"),
        "a refund"
    );
}

/// Writes the XRechnung documents to `$XRECHNUNG_OUT.{invoice,credit-note}.xml` and
/// the hybrid PDF, for KoSIT's validator — the reference German receivers use.
#[test]
#[ignore = "writes a file for an external validator"]
fn write_xrechnung_sample() {
    use meteroid_invoicing::einvoicing::generate_credit_note;
    use meteroid_invoicing::pdf::{PdfGenerator, TypstPdfGenerator};
    use meteroid_invoicing::typst_render::EInvoiceAttachment;

    let path = std::env::var("XRECHNUNG_OUT").expect("set XRECHNUNG_OUT to the target path");
    let invoice = xrechnung_invoice();
    let doc = generate(&invoice, EInvoiceFormat::XRechnung).unwrap();
    std::fs::write(format!("{path}.invoice.xml"), &doc.xml).unwrap();

    let cn = generate_credit_note(&xrechnung_credit_note(), EInvoiceFormat::XRechnung).unwrap();
    std::fs::write(format!("{path}.credit-note.xml"), &cn.xml).unwrap();

    let attachment = EInvoiceAttachment {
        filename: doc.filename.unwrap().to_string(),
        xml: doc.xml.into_bytes(),
        description: "XRechnung".to_string(),
    };
    let pdf = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(
            TypstPdfGenerator::new()
                .unwrap()
                .generate_pdf(&invoice, Some(&attachment)),
        )
        .unwrap();
    let stamped = meteroid_invoicing::einvoicing::stamp_facturx_xmp(
        &pdf,
        EInvoiceFormat::XRechnung,
        "xrechnung.xml",
    )
    .unwrap();
    std::fs::write(format!("{path}.pdf"), stamped).unwrap();
}

/// A stored report keeps the profile name, not the format; the API recovers the
/// format from it to say whether the PDF itself carries the XML. Every format must
/// round-trip, and a hybrid must be told apart from Peppol, which travels alone.
#[test]
fn a_stored_profile_name_recovers_its_format() {
    for format in [
        EInvoiceFormat::FacturX,
        EInvoiceFormat::XRechnung,
        EInvoiceFormat::PeppolBis,
    ] {
        assert_eq!(
            EInvoiceFormat::from_profile_name(format.profile_name()),
            Some(format)
        );
    }
    assert!(EInvoiceFormat::FacturX.is_hybrid());
    assert!(!EInvoiceFormat::PeppolBis.is_hybrid());
    assert_eq!(EInvoiceFormat::from_profile_name("not a profile"), None);
}
