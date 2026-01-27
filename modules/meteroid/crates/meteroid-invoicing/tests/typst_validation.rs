use chrono::NaiveDate;
use common_domain::country::CountryCode;
use meteroid_invoicing::credit_note_model::{
    self as cn, CreditNote, CreditNoteMetadata, CreditType,
};
use meteroid_invoicing::model::{Coupon, Flags, PaymentStatus, TaxBreakdownItem};
use meteroid_invoicing::pdf::{CreditNotePdfGenerator, PdfGenerator, TypstCreditNotePdfGenerator};
use meteroid_invoicing::{
    model::{
        Address, Customer, Invoice, InvoiceLine, InvoiceMetadata, InvoiceSubLine, Organization,
        Transaction,
    },
    pdf::TypstPdfGenerator,
};
use rust_decimal::Decimal;
use rusty_money::{Money, iso};
use std::collections::HashMap;
use std::str::FromStr;

#[tokio::test]
async fn test_typst_invoice_generation_validates_template() {
    let generator = TypstPdfGenerator::new().expect("Failed to create TypstPdfGenerator");
    let invoice = create_minimal_invoice();

    let result = generator.generate_pdf(&invoice).await;
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

    let result = generator.generate_pdf(&invoice).await;
    assert!(
        result.is_ok(),
        "Failed to generate PDF with full data: {:?}",
        result.err()
    );

    let pdf_data = result.unwrap();
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
    let result_en = generator.generate_pdf(&invoice_en).await;
    assert!(
        result_en.is_ok(),
        "Failed to generate English PDF: {:?}",
        result_en.err()
    );

    // Test French invoice
    let mut invoice_fr = create_minimal_invoice();
    invoice_fr.lang = "fr-FR".to_string();
    let result_fr = generator.generate_pdf(&invoice_fr).await;
    assert!(
        result_fr.is_ok(),
        "Failed to generate French PDF: {:?}",
        result_fr.err()
    );
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
            tax_id: None,
            footer_info: None,
            footer_legal: None,
            accounting_currency: *eur,
            exchange_rate: None,
        },
        customer: Customer {
            name: "Test Customer".to_string(),
            legal_number: None,
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
            purchase_order: Some("some order".to_string()),
        },
        lines: vec![InvoiceLine {
            name: "Test Service".to_string(),
            description: Some("Basic test service".to_string()),
            subtotal: Money::from_major(100, eur),
            quantity: Some(Decimal::from_str("1.0").unwrap()),
            unit_price: Some(Money::from_major(100, eur)),
            tax_rate: Decimal::from_str("20.0").unwrap(),
            start_date,
            end_date,
            sub_lines: vec![],
        }],
        coupons: vec![],
        payment_status: Some(PaymentStatus::Unpaid),
        transactions: vec![],
        bank_details: None,
        tax_breakdown: vec![TaxBreakdownItem {
            name: "VAT 20%".to_string(),
            rate: Decimal::from_str("20.0").unwrap(),
            amount: Money::from_major(20, eur),
            exemption_type: None,
        }],
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
            tax_id: Some("US123456789".to_string()),
            footer_info: Some("Payment terms: Net 14 days".to_string()),
            footer_legal: Some("All prices exclude VAT unless specified.".to_string()),
            accounting_currency: *eur,
            exchange_rate: Some(Decimal::from_str("1.08").unwrap()),
        },
        customer: Customer {
            name: "Premium Customer Ltd".to_string(),
            legal_number: Some("987654321".to_string()),
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
            purchase_order: Some("some order".to_string()),
        },
        lines: vec![
            InvoiceLine {
                name: "Professional Services".to_string(),
                description: Some("Consulting and development services".to_string()),
                subtotal: Money::from_major(800, eur),
                quantity: Some(Decimal::from_str("40.0").unwrap()),
                unit_price: Some(Money::from_major(20, eur)),
                tax_rate: Decimal::from_str("20.0").unwrap(),
                start_date,
                end_date,
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
            },
            InvoiceLine {
                name: "Support Package".to_string(),
                description: Some("Monthly support and maintenance".to_string()),
                subtotal: Money::from_major(200, eur),
                quantity: Some(Decimal::from_str("1.0").unwrap()),
                unit_price: Some(Money::from_major(200, eur)),
                tax_rate: Decimal::from_str("20.0").unwrap(),
                start_date,
                end_date,
                sub_lines: vec![],
            },
        ],
        coupons: vec![Coupon {
            name: "Early Payment Discount".to_string(),
            total: Money::from_major(25, eur),
        }],
        payment_status: Some(PaymentStatus::PartiallyPaid),
        transactions: vec![Transaction {
            method: "Wire Transfer".to_string(),
            date: payment_date,
            amount: Money::from_major(600, eur),
        }],
        bank_details: Some(payment_info),
        tax_breakdown: vec![TaxBreakdownItem {
            name: "VAT 20%".to_string(),
            rate: Decimal::from_str("20.0").unwrap(),
            amount: Money::from_major(200, eur),
            exemption_type: None,
        }],
    }
}

// Credit Note Tests

#[tokio::test]
async fn test_typst_credit_note_generation_validates_template() {
    let generator =
        TypstCreditNotePdfGenerator::new().expect("Failed to create TypstCreditNotePdfGenerator");
    let credit_note = create_minimal_credit_note();

    let result = generator.generate_credit_note_pdf(&credit_note).await;
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

    let result = generator.generate_credit_note_pdf(&credit_note).await;
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
    let result_en = generator.generate_credit_note_pdf(&credit_note_en).await;
    assert!(
        result_en.is_ok(),
        "Failed to generate English credit note PDF: {:?}",
        result_en.err()
    );

    // Test French credit note
    let mut credit_note_fr = create_minimal_credit_note();
    credit_note_fr.lang = "fr-FR".to_string();
    let result_fr = generator.generate_credit_note_pdf(&credit_note_fr).await;
    assert!(
        result_fr.is_ok(),
        "Failed to generate French credit note PDF: {:?}",
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

    let result = generator.generate_credit_note_pdf(&credit_note).await;
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
            tax_id: None,
            footer_info: None,
            footer_legal: None,
            accounting_currency: *eur,
            exchange_rate: None,
        },
        customer: cn::Customer {
            name: "Test Customer".to_string(),
            legal_number: None,
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
            subtotal: Money::from_major(100, eur),
            tax_amount: Money::from_major(20, eur),
            total_amount: Money::from_major(120, eur),
            currency: eur,
            reason: Some("Billing error".to_string()),
            memo: None,
            credit_type: CreditType::CreditToBalance,
            refunded_amount: Money::from_major(0, eur),
            credited_amount: Money::from_major(120, eur),
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
            quantity: Some(Decimal::from_str("1.0").unwrap()),
            unit_price: Some(Money::from_major(100, eur)),
            tax_rate: Decimal::from_str("20.0").unwrap(),
            start_date,
            end_date,
            sub_lines: vec![],
        }],
        tax_breakdown: vec![cn::TaxBreakdownItem {
            name: "VAT 20%".to_string(),
            rate: Decimal::from_str("20.0").unwrap(),
            amount: Money::from_major(20, eur),
            exemption_type: None,
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
            tax_id: Some("US123456789".to_string()),
            footer_info: Some("Credit notes are non-transferable.".to_string()),
            footer_legal: Some("This credit note is valid for 12 months.".to_string()),
            accounting_currency: *eur,
            exchange_rate: Some(Decimal::from_str("1.08").unwrap()),
        },
        customer: cn::Customer {
            name: "Premium Customer Ltd".to_string(),
            legal_number: Some("987654321".to_string()),
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
            subtotal: Money::from_major(500, eur),
            tax_amount: Money::from_major(100, eur),
            total_amount: Money::from_major(600, eur),
            currency: eur,
            reason: Some("Partial refund for unused services".to_string()),
            memo: Some("We apologize for any inconvenience.".to_string()),
            credit_type: CreditType::Refund,
            refunded_amount: Money::from_major(600, eur),
            credited_amount: Money::from_major(0, eur),
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
                quantity: Some(Decimal::from_str("20.0").unwrap()),
                unit_price: Some(Money::from_major(20, eur)),
                tax_rate: Decimal::from_str("20.0").unwrap(),
                start_date,
                end_date,
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
            },
            cn::CreditNoteLine {
                name: "Support Package".to_string(),
                description: Some("Unused support credit".to_string()),
                subtotal: Money::from_major(100, eur),
                quantity: Some(Decimal::from_str("1.0").unwrap()),
                unit_price: Some(Money::from_major(100, eur)),
                tax_rate: Decimal::from_str("20.0").unwrap(),
                start_date,
                end_date,
                sub_lines: vec![],
            },
        ],
        tax_breakdown: vec![cn::TaxBreakdownItem {
            name: "VAT 20%".to_string(),
            rate: Decimal::from_str("20.0").unwrap(),
            amount: Money::from_major(100, eur),
            exemption_type: None,
        }],
    }
}
