use crate::errors::InvoicingRenderError;
use crate::services::storage::{ObjectStoreService, Prefix};
use common_domain::ids::{InvoiceId, InvoicingEntityId, StoredDocumentId, TenantId};
use error_stack::{Report, ResultExt};
use image::ImageFormat::Png;
use meteroid_invoicing::{pdf, svg};
use meteroid_store::Store;
use meteroid_store::domain::{Invoice, InvoicingEntity};
use meteroid_store::jwt_claims::{ResourceAccess, generate_portal_token};
use meteroid_store::repositories::{CustomersInterface, InvoiceInterface};
use meteroid_store::repositories::bank_accounts::BankAccountsInterface;
use meteroid_store::repositories::historical_rates::HistoricalRatesInterface;
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;
use meteroid_store::repositories::subscriptions::SubscriptionInterface;
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

async fn resolve_payment_info(
    store: &Arc<Store>,
    invoice: &Invoice,
    invoicing_entity: &InvoicingEntity,
    public_url: &str,
    jwt_secret: &secrecy::SecretString,
) -> Result<(Option<HashMap<String, String>>, Option<String>), Report<InvoicingRenderError>> {

    // TODO a bit complex here to resolve accurately whether we want the payment url or bank or none. We need to centralize it
    // ex: save the payment_option on the invoice : PaymentLink, Bank(id), None

    let customer = store
        .find_customer_by_id(invoice.customer_id, invoice.tenant_id)
        .await
        .change_context(InvoicingRenderError::StoreError)?;

    if let Some(subscription_id) = invoice.subscription_id {
        let subscription = store
            .get_subscription(invoice.tenant_id, subscription_id)
            .await
            .change_context(InvoicingRenderError::StoreError)?;

        let has_payment_method = subscription.card_connection_id.is_some()
            || subscription.direct_debit_connection_id.is_some()
            || customer.current_payment_method_id.is_some();

        match (has_payment_method, subscription.charge_automatically) {
            // Automatic charging enabled - no bank details, no payment URL needed
            (_, true) => Ok((None, None)),
            // Manual payment via portal - show payment URL, no bank details
            (true, false) => {
                let invoice_token = generate_portal_token(
                    jwt_secret,
                    invoice.tenant_id,
                    ResourceAccess::Invoice(invoice.id),
                )
                    .change_context(InvoicingRenderError::StoreError)?;
                let payment_url = format!("{}/portal/invoice-payment?token={}", public_url, invoice_token);
                Ok((None, Some(payment_url)))
            }
            // No payment method - show bank details
            (false, _) => fetch_bank_details(store, invoicing_entity, invoice).await,
        }
    } else {
        let has_payment_method = customer.current_payment_method_id.is_some();

        if has_payment_method {
            let invoice_token = generate_portal_token(
                jwt_secret,
                invoice.tenant_id,
                ResourceAccess::Invoice(invoice.id),
            )
                .change_context(InvoicingRenderError::StoreError)?;
            let payment_url = format!("{}/portal/invoice-payment?token={}", public_url, invoice_token);
            Ok((None, Some(payment_url)))
        } else {
            fetch_bank_details(store, invoicing_entity, invoice).await
        }
    }
}

async fn fetch_bank_details(
    store: &Arc<Store>,
    invoicing_entity: &InvoicingEntity,
    invoice: &Invoice,
) -> Result<(Option<HashMap<String, String>>, Option<String>), Report<InvoicingRenderError>> {
    let bank_details = if let Some(bank_account_id) = invoicing_entity.bank_account_id {
        let bank_account = store
            .get_bank_account_by_id(bank_account_id, invoice.tenant_id)
            .await
            .change_context(InvoicingRenderError::StoreError)?;
        Some(mapper::format_bank_account(&bank_account, invoice.reference.as_deref()))
    } else {
        None
    };
    Ok((bank_details, None))
}

#[derive(Clone)]
pub struct InvoicePreviewRenderingService {
    store: Arc<Store>,
    generator: Arc<dyn svg::SvgGenerator>,
    storage: Arc<dyn ObjectStoreService>,
    public_url: String,
    jwt_secret: secrecy::SecretString,
}

impl InvoicePreviewRenderingService {
    pub fn try_new(
        store: Arc<Store>,
        storage: Arc<dyn ObjectStoreService>,
        public_url: String,
        jwt_secret: secrecy::SecretString,
    ) -> Result<Self, Report<InvoicingRenderError>> {
        let generator = svg::TypstSvgGenerator::new()
            .change_context(InvoicingRenderError::InitializationError)
            .attach("Typst SVG generator failed to initialize")?;

        Ok(Self {
            store,
            generator: Arc::new(generator),
            storage,
            public_url,
            jwt_secret,
        })
    }

    pub async fn preview_invoice(
        &self,
        invoice: Invoice,
        invoicing_entity: InvoicingEntity,
    ) -> Result<Vec<String>, Report<InvoicingRenderError>> {
        let organization_logo = match invoicing_entity.logo_attachment_id.as_ref() {
            Some(logo_id) => {
                
                get_logo_bytes_for_invoice(&self.storage, *logo_id).await.ok()
            }
            None => None,
        };

        let mut rate = None;
        if invoice.currency != invoicing_entity.accounting_currency {
            rate = self
                .store
                .get_historical_rate(
                    &invoice.currency,
                    &invoicing_entity.accounting_currency,
                    invoice.invoice_date,
                )
                .await
                .change_context(InvoicingRenderError::StoreError)?;
        }

        // Determine payment information based on subscription payment method (same as PDF generation)
        let (bank_details, payment_url) = resolve_payment_info(
            &self.store,
            &invoice,
            &invoicing_entity,
            &self.public_url,
            &self.jwt_secret,
        ).await?;

        let mapped = mapper::map_invoice_to_invoicing(
            invoice,
            &invoicing_entity,
            organization_logo,
            rate,
            bank_details,
            payment_url,
        )?;

        let svgs = self
            .generator
            .generate_svg(&mapped)
            .await
            .change_context(InvoicingRenderError::RenderError)?;

        Ok(svgs)
    }

    pub async fn preview_invoice_by_id(
        &self,
        invoice_id: InvoiceId,
        tenant_id: TenantId,
    ) -> Result<Vec<String>, Report<InvoicingRenderError>> {
        let invoice = self
            .store
            .get_invoice_by_id(tenant_id, invoice_id)
            .await
            .change_context(InvoicingRenderError::StoreError)?;

        let invoicing_entity = self
            .store
            .get_invoicing_entity(tenant_id, Some(invoice.seller_details.id))
            .await
            .change_context(InvoicingRenderError::StoreError)?;

        self.preview_invoice(invoice, invoicing_entity).await
    }
}

pub enum GenerateResult {
    Success {
        invoice_id: InvoiceId,
        tenant_id: TenantId,
        pdf_id: StoredDocumentId,
    },
    Failure {
        invoice_id: InvoiceId,
        error: String,
    },
}

#[derive(Clone)]
pub struct PdfRenderingService {
    storage: Arc<dyn ObjectStoreService>,
    pdf: Arc<dyn pdf::PdfGenerator>,
    store: Arc<Store>,
    public_url: String,
    jwt_secret: secrecy::SecretString,
}

impl PdfRenderingService {
    pub fn try_new(
        storage: Arc<dyn ObjectStoreService>,
        store: Arc<Store>,
        public_url: String,
        jwt_secret: secrecy::SecretString,
    ) -> Result<Self, Report<InvoicingRenderError>> {
        let pdf_generator = Arc::new(
            pdf::TypstPdfGenerator::new()
                .change_context(InvoicingRenderError::InitializationError)
                .attach("Typst PDF generator failed to initialize")?,
        );

        Ok(Self {
            storage,
            pdf: pdf_generator,
            store,
            public_url,
            jwt_secret,
        })
    }

    pub async fn generate_pdfs(
        &self,
        invoice_ids: Vec<InvoiceId>,
    ) -> Result<Vec<GenerateResult>, Report<InvoicingRenderError>> {
        let invoices = self
            .store
            .list_invoices_by_ids(invoice_ids)
            .await
            .change_context(InvoicingRenderError::StoreError)?;

        let invoicing_entity_ids = invoices
            .iter()
            .map(|invoice| invoice.seller_details.id)
            .collect::<Vec<InvoicingEntityId>>();

        let invoicing_entities = self
            .store
            .list_invoicing_entities_by_ids(invoicing_entity_ids)
            .await
            .change_context(InvoicingRenderError::StoreError)?;

        let mut results = vec![];

        for invoice in invoices {
            let invoice_id = invoice.id;
            let tenant_id = invoice.tenant_id;

            if let Some(pdf_id) = invoice.pdf_document_id {
                results.push(GenerateResult::Success {
                    invoice_id,
                    tenant_id,
                    pdf_id,
                });
                continue;
            }

            let res = match self
                .generate_pdf_and_save(invoice, &invoicing_entities)
                .await
            {
                Err(error) => GenerateResult::Failure {
                    invoice_id,
                    error: error.to_string(),
                },
                Ok(pdf_id) => GenerateResult::Success {
                    invoice_id,
                    tenant_id,
                    pdf_id,
                },
            };
            results.push(res);
        }
        Ok(results)
    }

    async fn generate_pdf_and_save(
        &self,
        invoice: Invoice,
        invoicing_entities: &[InvoicingEntity],
    ) -> Result<StoredDocumentId, Report<InvoicingRenderError>> {
        let invoicing_entity = invoicing_entities
            .iter()
            .find(|entity| entity.id == invoice.seller_details.id)
            .ok_or(InvoicingRenderError::StoreError)
            .attach("Failed to resolve invoicing entity")?;

        let invoice_id = invoice.id;
        let tenant_id = invoice.tenant_id;

        // let's resolve the logo as raw bytes
        let organization_logo = match invoicing_entity.logo_attachment_id.as_ref() {
            Some(logo_id) => {
                
                get_logo_bytes_for_invoice(&self.storage, *logo_id).await.ok()
            }
            None => None,
        };

        let mut rate = None;
        if invoice.currency != invoicing_entity.accounting_currency {
            rate = self
                .store
                .get_historical_rate(
                    &invoice.currency,
                    &invoicing_entity.accounting_currency,
                    invoice.invoice_date,
                )
                .await
                .change_context(InvoicingRenderError::StoreError)?;
        }

        // Determine payment information based on subscription payment method
        let (bank_details, payment_url) = resolve_payment_info(
            &self.store,
            &invoice,
            invoicing_entity,
            &self.public_url,
            &self.jwt_secret,
        ).await?;

        let customer_id = invoice.customer_id;

        let mapped_invoice = mapper::map_invoice_to_invoicing(
            invoice,
            invoicing_entity,
            organization_logo,
            rate,
            bank_details,
            payment_url,
        )?;

        let pdf = self
            .pdf
            .generate_pdf(&mapped_invoice)
            .await
            .change_context(InvoicingRenderError::PdfError)?;

        let pdf_id = self
            .storage
            .store(pdf, Prefix::InvoicePdf)
            .await
            .change_context(InvoicingRenderError::StorageError)?;

        self.store
            .save_invoice_documents(invoice_id, tenant_id, customer_id, pdf_id, None)
            .await
            .change_context(InvoicingRenderError::StoreError)?;

        Ok(pdf_id)
    }
}

mod mapper {
    use crate::errors::InvoicingRenderError;
    use error_stack::Report;
    use meteroid_invoicing::model as invoicing_model;
    use meteroid_store::constants::Countries;

    use meteroid_invoicing::model::Flags;
    use meteroid_store::domain as store_model;
    use meteroid_store::domain::historical_rates::HistoricalRate;
    use rust_decimal::Decimal;
    use rust_decimal::prelude::FromPrimitive;
    use std::collections::HashMap;

    pub fn map_invoice_to_invoicing(
        invoice: store_model::Invoice,
        invoicing_entity: &store_model::InvoicingEntity,
        organization_logo_bytes: Option<Vec<u8>>,
        accounting_rate: Option<HistoricalRate>,
        bank_details: Option<HashMap<String, String>>,
        payment_url: Option<String>,
    ) -> Result<invoicing_model::Invoice, Report<InvoicingRenderError>> {
        let finalized_date = invoice
            .finalized_at
            .map_or(invoice.invoice_date, |d| d.date());

        let currency = rusty_money::iso::find(&invoice.currency).ok_or_else(|| {
            Report::new(InvoicingRenderError::InvalidCurrency(
                invoice.currency.clone(),
            ))
        })?;

        let accounting_currency = *rusty_money::iso::find(&invoicing_entity.accounting_currency)
            .ok_or_else(|| {
                Report::new(InvoicingRenderError::InvalidCurrency(
                    invoicing_entity.accounting_currency.clone(),
                ))
            })?;

        let metadata = invoicing_model::InvoiceMetadata {
            currency,
            due_date: invoice.due_at.map_or(
                finalized_date + chrono::Duration::days(i64::from(invoice.net_terms)),
                |d| d.date(),
            ),
            number: invoice.invoice_number,
            issue_date: finalized_date,
            payment_term: invoice.net_terms as u32,
            total_amount: rusty_money::Money::from_minor(invoice.total, currency),
            tax_amount: rusty_money::Money::from_minor(invoice.tax_amount, currency),
            subtotal: rusty_money::Money::from_minor(invoice.subtotal, currency),
            discount: rusty_money::Money::from_minor(invoice.discount, currency),
            memo: invoice.memo.clone(),
            payment_url,
            flags: Flags {
                show_payment_status: Some(true),
                show_payment_info: Some(true),
                show_terms: Some(true),
                show_tax_info: Some(true),
                show_legal_info: Some(true),
                show_footer_custom_info: Some(true),
                whitelabel: Some(false),
            },
            purchase_order: invoice.purchase_order.clone(),
        };

        fn map_address(address: store_model::Address) -> invoicing_model::Address {
            invoicing_model::Address {
                line1: address.line1,
                line2: address.line2,
                city: address.city,
                country: address.country,
                state: address.state,
                zip_code: address.zip_code,
            }
        }

        let organization = invoicing_model::Organization {
            address: map_address(invoice.seller_details.address),
            email: None,        // TODO
            legal_number: None, // TODO
            logo_src: organization_logo_bytes,
            name: invoice.seller_details.legal_name,
            tax_id: invoice.seller_details.vat_number,
            footer_info: invoicing_entity.invoice_footer_info.clone(),
            footer_legal: invoicing_entity.invoice_footer_legal.clone(),
            accounting_currency,
            exchange_rate: accounting_rate.and_then(|r| Decimal::from_f32(r.rate)),
        };

        let customer = invoicing_model::Customer {
            address: invoice
                .customer_details
                .billing_address
                .map(map_address)
                .unwrap_or_default(),
            email: invoice.customer_details.email,
            legal_number: None, // TODO
            name: invoice.customer_details.name,
            tax_id: invoice.customer_details.vat_number,
        };

        let lines = invoice
            .line_items
            .iter()
            .map(|line| invoicing_model::InvoiceLine {
                description: line.description.clone(),
                quantity: line.quantity,
                tax_rate: line.tax_rate,
                unit_price: line
                    .unit_price
                    .map(|p| rusty_money::Money::from_decimal(p, currency)),
                name: line.name.clone(),
                end_date: line.end_date,
                start_date: line.start_date,
                subtotal: rusty_money::Money::from_minor(line.amount_subtotal, currency),
                sub_lines: line
                    .sub_lines
                    .iter()
                    .map(|sub_line| invoicing_model::InvoiceSubLine {
                        name: sub_line.name.clone(),
                        total: rusty_money::Money::from_minor(sub_line.total, currency),
                        quantity: sub_line.quantity,
                        unit_price: rusty_money::Money::from_decimal(sub_line.unit_price, currency),
                    })
                    .collect(),
            })
            .collect();

        let coupons = invoice
            .coupons
            .iter()
            .map(|coupon| invoicing_model::Coupon {
                name: coupon.name.clone(),
                total: rusty_money::Money::from_minor(coupon.value, currency),
            })
            .collect();

        let lang = Countries::resolve_country(&invoicing_entity.country.code)
            .map_or_else(|| "en-US", |c| c.locale);

        let tax_breakdown = invoice
            .tax_breakdown
            .iter()
            .map(|t| {
                use meteroid_invoicing::model::TaxExemptionType as InvoicingExemption;
                use meteroid_store::domain::invoices::TaxExemptionType as StoreExemption;

                let exemption_type = t.exemption_type.as_ref().map(|e| match e {
                    StoreExemption::ReverseCharge => InvoicingExemption::ReverseCharge,
                    StoreExemption::TaxExempt => InvoicingExemption::TaxExempt,
                    StoreExemption::NotRegistered => InvoicingExemption::NotRegistered,
                    StoreExemption::Other(s) => InvoicingExemption::Other(s.clone()),
                });

                invoicing_model::TaxBreakdownItem {
                    name: t.name.clone(),
                    rate: t.tax_rate,
                    amount: rusty_money::Money::from_minor(t.tax_amount as i64, currency),
                    exemption_type,
                }
            })
            .collect();

        Ok(invoicing_model::Invoice {
            lang: lang.to_string(),
            customer,
            lines,
            metadata,
            organization,
            coupons,
            tax_breakdown,
            bank_details,
            transactions: Vec::new(), // TODO
            payment_status: None,     // TODO
        })
    }

    pub fn format_bank_account(
        bank_account: &store_model::BankAccount,
        payment_reference: Option<&str>,
    ) -> HashMap<String, String> {
        let mut details = HashMap::new(); // TODO ordermap::OrderMap , also translation

        // Parse account numbers based on format
        let numbers: Vec<&str> = bank_account.account_numbers.split_whitespace().collect();

        match bank_account.format {
            store_model::BankAccountFormat::IbanBicSwift => {
                if let Some(iban) = numbers.first() {
                    details.insert("IBAN".to_string(), iban.to_string());
                }
                if let Some(bic) = numbers.get(1) {
                    details.insert("BIC/SWIFT".to_string(), bic.to_string());
                }
            }
            store_model::BankAccountFormat::AccountRouting => {
                if let Some(account) = numbers.first() {
                    details.insert("Account Number".to_string(), account.to_string());
                }
                if let Some(routing) = numbers.get(1) {
                    details.insert("Routing Number".to_string(), routing.to_string());
                }
            }
            store_model::BankAccountFormat::SortCodeAccount => {
                if let Some(sort_code) = numbers.first() {
                    details.insert("Sort Code".to_string(), sort_code.to_string());
                }
                if let Some(account) = numbers.get(1) {
                    details.insert("Account Number".to_string(), account.to_string());
                }
            }
            store_model::BankAccountFormat::AccountBicSwift => {
                if let Some(account) = numbers.first() {
                    details.insert("Account Number".to_string(), account.to_string());
                }
                if let Some(bic) = numbers.get(1) {
                    details.insert("BIC/SWIFT".to_string(), bic.to_string());
                }
            }
        }

        // Add bank name
        details.insert("Bank Name".to_string(), bank_account.bank_name.clone());

        // Add payment reference if available
        if let Some(reference) = payment_reference {
            details.insert("Payment Reference".to_string(), reference.to_string());
        }

        details
    }
}

async fn get_logo_bytes_for_invoice(
    storage: &Arc<dyn ObjectStoreService>,
    logo_id: StoredDocumentId,
) -> Result<Vec<u8>, Report<InvoicingRenderError>> {
    let logo = storage
        .retrieve(logo_id, Prefix::ImageLogo)
        .await
        .change_context(InvoicingRenderError::StorageError)?;

    let mut img =
        image::load_from_memory(&logo).change_context(InvoicingRenderError::RenderError)?;
    img = img.resize(1024, 100, image::imageops::FilterType::Nearest);
    let mut buffer = Vec::new();
    img.write_to(&mut Cursor::new(&mut buffer), Png)
        .change_context(InvoicingRenderError::RenderError)?;

    Ok(buffer)
}
