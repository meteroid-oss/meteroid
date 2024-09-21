// pub mod errors;
// pub mod html_render;
// pub mod pdf;
// pub mod storage;
//
// use crate::html_render::Invoice;
// use crate::html_render::InvoiceLine;
// use crate::html_render::Organization;
// use crate::html_render::{render_invoice, Customer};
// use crate::pdf::{GotenbergPdfGenerator, PdfGenerator};
// use crate::storage::Storage;
// use chrono::NaiveDate;
// use errors::InvoicingResult;
// use rust_decimal_macros::dec;
//
// // V - render html
//
// // V - html to pdf
//
// // - xml einvoice (CII, UBL, Factur-X)
//
// // - attach xml to pdf if needed
//
// // V - upload pdf/xml to s3
//
// // Following is for issuing, different consumer
//
// // - send email with pdf
//
// // - upload xml/pdf to public authority
//
// // - report tax to tax authority ?
//
// #[tokio::main]
// async fn main() -> Result<(), Box<dyn std::error::Error>> {
//     let organization = Organization {
//         name: "ACME Corporation".to_string(),
//         logo_url: Some("https://example.com/logo.png".to_string()),
//         legal_number: Some("123456789".to_string()),
//         address_line1: "123 Business St".to_string(),
//         address_line2: Some("Suite 100".to_string()),
//         zipcode: "12345".to_string(),
//         city: "Businessville".to_string(),
//         state: Some("BZ".to_string()),
//         country: "United States".to_string(),
//         email: "info@acmecorp.com".to_string(),
//         tax_identification_number: Some("TIN-987654321".to_string()),
//     };
//
//     let customer = Customer {
//         name: "John Doe".to_string(),
//         legal_number: None,
//         address_line1: "456 Customer Ave".to_string(),
//         address_line2: None,
//         zipcode: "67890".to_string(),
//         city: "Customertown".to_string(),
//         state: None,
//         country: "United States".to_string(),
//         email: "john.doe@example.com".to_string(),
//         tax_identification_number: None,
//     };
//
//     let invoice = Invoice {
//         number: "INV-2023-001".to_string(),
//         issue_date: NaiveDate::from_ymd_opt(2023, 9, 19).unwrap(),
//         payment_term: 30,
//         total_amount: dec!(1234.56),
//         currency: "USD".to_string(),
//         due_date: NaiveDate::from_ymd_opt(2023, 10, 19).unwrap(),
//     };
//
//     let invoice_lines = vec![
//         InvoiceLine {
//             description: "Web Development Services".to_string(),
//             quantity: dec!(40),
//             unit_price: dec!(25),
//             tax_rate: dec!(0.08),
//             amount: dec!(1000),
//         },
//         InvoiceLine {
//             description: "Server Hosting (Monthly)".to_string(),
//             quantity: dec!(1),
//             unit_price: dec!(100),
//             tax_rate: dec!(0.08),
//             amount: dec!(100),
//         },
//     ];
//
//     // TODO also conversion cf https://docs.rs/rusty-money/latest/rusty_money/#exchange
//
//     let html_output = render_invoice("fr-FR", &organization, &customer, &invoice, &invoice_lines)
//         .unwrap()
//         .into_string();
//     println!("{}", html_output);
//
//     let pdfgenerator = GotenbergPdfGenerator::new("https://demo.gotenberg.dev".to_string());
//     //
//     let pdf = pdfgenerator.generate_pdf(&html_output).await.unwrap();
//
//     println!("{}", pdf.len());
//     let crate_root = std::env::var("CARGO_MANIFEST_DIR").unwrap();
//     // let's create with file:// on the crate level
//     let storage = storage::S3Storage::create(format!("file://{}/pdfs", crate_root), None).unwrap();
//
//     let res = storage.store_pdf(pdf, None).await.unwrap();
//
//     println!("{}", res);
//
//     Ok(())
// }
