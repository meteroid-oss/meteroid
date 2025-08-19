use chrono::NaiveDate;
use meteroid_invoicing::model::{Coupon, Flags, PaymentStatus, TaxBreakdownItem};
use meteroid_invoicing::pdf::PdfGenerator;
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
use std::path::Path;
use std::str::FromStr;
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Starting invoice generation benchmark...");

    let generator = TypstPdfGenerator::new()?;
    let iterations = 10;
    let mut generation_times = Vec::with_capacity(iterations);
    let invoice = create_test_invoice();

    // Run the benchmark
    for i in 1..=iterations {
        println!("Running iteration {}/{}...", i, iterations);

        let start = Instant::now();

        let _pdf_data = generator.generate_pdf(&invoice).await?;

        let elapsed = start.elapsed();
        generation_times.push(elapsed);

        println!("  Iteration {} completed in {:.2?}", i, elapsed);
    }

    // Calculate and print statistics
    if !generation_times.is_empty() {
        // Calculate average, min, max
        let total_time: Duration = generation_times.iter().sum();
        let avg_time = total_time / generation_times.len() as u32;
        let min_time = generation_times.iter().min().unwrap();
        let max_time = generation_times.iter().max().unwrap();

        println!("\nBenchmark Results:");
        println!("  Total iterations: {}", iterations);
        println!("  Average generation time: {:.2?}", avg_time);
        println!("  Minimum generation time: {:.2?}", *min_time);
        println!("  Maximum generation time: {:.2?}", *max_time);

        let pdf_data = generator.generate_pdf(&invoice).await?;
        let output_path = Path::new("benchmark_invoice.pdf");
        std::fs::write(output_path, pdf_data)?;
        println!(
            "\nExample invoice saved at: {:?}",
            output_path.canonicalize()?
        );
    }

    Ok(())
}

fn create_test_invoice() -> Invoice {
    let issue_date = NaiveDate::from_ymd_opt(2025, 4, 1).unwrap();
    let due_date = NaiveDate::from_ymd_opt(2025, 4, 15).unwrap();
    let start_date = NaiveDate::from_ymd_opt(2025, 3, 1).unwrap();
    let end_date = NaiveDate::from_ymd_opt(2025, 3, 31).unwrap();

    let payment1_date = NaiveDate::from_ymd_opt(2025, 3, 15).unwrap();
    let payment2_date = NaiveDate::from_ymd_opt(2025, 3, 20).unwrap();

    let eur = iso::find("EUR").unwrap();

    let mut payment_info = HashMap::new();
    payment_info.insert("Bank Name".to_string(), "International Bank".to_string());
    payment_info.insert("Account Holder".to_string(), "Acme Inc.".to_string());
    payment_info.insert(
        "IBAN".to_string(),
        "FR76 1234 5678 9012 3456 7890 123".to_string(),
    );
    payment_info.insert("BIC/SWIFT".to_string(), "INTLFRPP".to_string());
    payment_info.insert("Reference".to_string(), "INV-2025-001".to_string());

    let transactions = vec![
        Transaction {
            method: "Card •••• 7726".to_string(),
            date: payment1_date,
            amount: Money::from_major(500, eur),
        },
        Transaction {
            method: "Bank Transfer".to_string(),
            date: payment2_date,
            amount: Money::from_major(300, eur),
        },
    ];

    Invoice {
        lang: "fr-FR".to_string(),
        organization: Organization {
            name: "Acme Inc.".to_string(),
            logo_src: None,
            legal_number: Some("123456789".to_string()),
            address: Address {
                line1: Some("123 Main St".to_string()),
                line2: None,
                city: Some("Paris".to_string()),
                country: Some("FR".to_string()),
                state: None,
                zip_code: Some("75001".to_string()),
            },
            email: Some("contact@acme.com".to_string()),
            tax_id: Some("FR123456789".to_string()),
            footer_info: Some("Payment to be made within 14 days".to_string()),
            footer_legal: Some("Acme Inc. is registered in France. All prices are in EUR and exclude VAT unless specified.".to_string()),
            accounting_currency: *eur,
            exchange_rate: Some(Decimal::from_str("1.08").unwrap()),
        },
        customer: Customer {
            name: "Client Corp".to_string(),
            legal_number: Some("987654321".to_string()),
            address: Address {
                line1: Some("456 Client Avenue".to_string()),
                line2: None,
                city: Some("New York".to_string()),
                country: Some("US".to_string()),
                state: Some("NY".to_string()),
                zip_code: Some("10001".to_string()),
            },
            email: Some("billing@clientcorp.com".to_string()),
            tax_id: Some("US987654321".to_string()),
        },
        metadata: InvoiceMetadata {
            number: "INV-2025-001".to_string(),
            issue_date,
            payment_term: 14,
            subtotal: Money::from_major(1000, eur),
            tax_amount: Money::from_major(200, eur),
            total_amount: Money::from_major(1200, eur),
            currency: eur,
            due_date,
            memo: Some("Thank you for your subscription!".to_string()),
            payment_url: Some("https://pay.example.com/inv-2025-001".to_string()),
            flags: Flags {
                show_payment_status: Some(true),
                show_payment_info: Some(true),
                show_terms: Some(true),
                show_tax_info: Some(true),
                show_legal_info: Some(true),
                whitelabel: None
            }
        },
        lines: vec![
            InvoiceLine {
                name: "Consulting Services".to_string(),
                description: Some("Technical consulting for Q1 2025".to_string()),
                subtotal: Money::from_major(750, eur),
                quantity: Some(Decimal::from_str("15.0").unwrap()),
                unit_price: Some(Money::from_major(50, eur)),
                tax_rate: Decimal::from_str("20.0").unwrap(),
                start_date,
                end_date,
                sub_lines: vec![
                    InvoiceSubLine {
                        name: "Frontend Development".to_string(),
                        total: Money::from_major(250, eur),
                        quantity: Decimal::from_str("5.0").unwrap(),
                        unit_price: Money::from_major(50, eur),
                    },
                    InvoiceSubLine {
                        name: "Backend Development".to_string(),
                        total: Money::from_major(500, eur),
                        quantity: Decimal::from_str("10.0").unwrap(),
                        unit_price: Money::from_major(50, eur),
                    },
                ],
            },
            InvoiceLine {
                name: "Software License".to_string(),
                description: Some("Annual license renewal".to_string()),
                subtotal: Money::from_major(250, eur),
                quantity: Some(Decimal::from_str("1.0").unwrap()),
                unit_price: Some(Money::from_major(250, eur)),
                tax_rate: Decimal::from_str("20.0").unwrap(),
                start_date,
                end_date,
                sub_lines: vec![],
            },
        ],
        coupons: vec![
            Coupon {
                name: "Spring promotion 10% off".to_string(),
                total: Money::from_major(25, eur),
            }
        ],
        payment_status: Some(PaymentStatus::Paid),
        transactions,
        bank_details: Some(payment_info),
        tax_breakdown: vec![
            TaxBreakdownItem {
                name: "VAT 20%".to_string(),
                rate: Decimal::from_str("20.0").unwrap(),
                amount: Money::from_major(200, eur),
                exemption_type: None
            },
        ],
    }
}
