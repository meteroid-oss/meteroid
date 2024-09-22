use crate::errors::InvoicingRenderError;
use error_stack::ResultExt;
use meteroid_invoicing::{html_render, pdf, storage};
use meteroid_store::domain::{Invoice, InvoicingEntity};
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::Store;
use std::sync::Arc;
use uuid::Uuid;

pub struct HtmlRenderingService {
    store: Arc<Store>,
}

impl HtmlRenderingService {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }

    pub async fn preview_invoice_html(
        &self,
        invoice_id: Uuid,
        tenant_id: Uuid,
    ) -> error_stack::Result<String, InvoicingRenderError> {
        let invoice = self
            .store
            .find_invoice_by_id(tenant_id, invoice_id)
            .await
            .change_context(InvoicingRenderError::StoreError)?;

        let invoicing_entity = self
            .store
            .get_invoicing_entity(tenant_id, Some(invoice.invoice.seller_details.id))
            .await
            .change_context(InvoicingRenderError::StoreError)?;

        let mapped = mapper::map_invoice_to_invoicing(invoice.invoice, &invoicing_entity)?;

        let html_string = html_render::render_invoice(&mapped)
            .change_context(InvoicingRenderError::RenderError)?
            .into_string();

        Ok(html_string)
    }
}

pub enum GenerateResult {
    Success { invoice_id: Uuid, pdf_url: String },
    Failure { invoice_id: Uuid, error: String },
}

pub struct PdfRenderingService {
    storage: Arc<dyn storage::Storage>,
    pdf: Arc<dyn pdf::PdfGenerator>,
    store: Arc<Store>,
}

impl PdfRenderingService {
    pub fn try_new(
        gotenberg_url: String,
        s3_uri: String,
        s3_prefix: Option<String>,
        store: Arc<Store>,
    ) -> error_stack::Result<Self, InvoicingRenderError> {
        let pdf_generator = Arc::new(pdf::GotenbergPdfGenerator::new(gotenberg_url.clone()));

        // accept an objectstore client instead of the config ?
        let s3_storage = Arc::new(
            storage::S3Storage::try_new(s3_uri.clone(), s3_prefix.clone())
                .change_context(InvoicingRenderError::InitializationError)?,
        );

        Ok(Self {
            storage: s3_storage,
            pdf: pdf_generator,
            store,
        })
    }

    pub async fn generate_pdfs(
        &self,
        invoice_ids: Vec<Uuid>,
    ) -> error_stack::Result<Vec<GenerateResult>, InvoicingRenderError> {
        let invoices = self
            .store
            .list_invoices_by_ids(invoice_ids)
            .await
            .change_context(InvoicingRenderError::StoreError)?;

        let invoicing_entity_ids = invoices
            .iter()
            .map(|invoice| invoice.seller_details.id)
            .collect::<Vec<Uuid>>();

        let invoicing_entities = self
            .store
            .list_invoicing_entities_by_ids(invoicing_entity_ids)
            .await
            .change_context(InvoicingRenderError::StoreError)?;

        let mut results = vec![];

        for invoice in invoices {
            let invoice_id = invoice.id;
            let res = match self
                .generate_pdf_and_save(invoice, &invoicing_entities)
                .await
            {
                Err(error) => GenerateResult::Failure {
                    invoice_id,
                    error: error.to_string(),
                },
                Ok(pdf_url) => GenerateResult::Success {
                    invoice_id,
                    pdf_url: pdf_url.clone(),
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
    ) -> error_stack::Result<String, InvoicingRenderError> {
        let invoicing_entity = invoicing_entities
            .iter()
            .find(|entity| entity.id == invoice.seller_details.id)
            .ok_or(InvoicingRenderError::StoreError)
            .attach_printable("Failed to resolve invoicing entity")?;

        let invoice_id = invoice.id;
        let tenant_id = invoice.tenant_id;

        let mapped_invoice = mapper::map_invoice_to_invoicing(invoice, invoicing_entity)?;

        let html = html_render::render_invoice(&mapped_invoice)
            .change_context(InvoicingRenderError::RenderError)?
            .into_string();

        let pdf = self
            .pdf
            .generate_pdf(&html)
            .await
            .change_context(InvoicingRenderError::PdfError)?;

        let pdf_url = self
            .storage
            .store_pdf(pdf, None)
            .await
            .change_context(InvoicingRenderError::StorageError)?;

        self.store
            .save_invoice_documents(invoice_id, tenant_id, pdf_url.clone(), None)
            .await
            .change_context(InvoicingRenderError::StoreError)?;

        Ok(pdf_url)
    }
}

mod mapper {
    use crate::errors::InvoicingRenderError;
    use error_stack::Report;
    use meteroid_invoicing::model as invoicing_model; 
    use meteroid_store::constants::Countries;

    use meteroid_store::domain as store_model;
    use rust_decimal::Decimal;

    pub fn map_invoice_to_invoicing(
        invoice: store_model::Invoice,
        invoicing_entity: &store_model::InvoicingEntity,
    ) -> error_stack::Result<invoicing_model::Invoice, InvoicingRenderError> {
        let finalized_date = invoice
            .finalized_at
            .map(|d| d.date())
            .unwrap_or(invoice.invoice_date);

        let currency = *rusty_money::iso::find(&invoice.currency)
            .ok_or_else(|| {
                Report::new(InvoicingRenderError::InvalidCurrency(
                    invoice.currency.clone(),
                ))
            })?;

        let metadata = invoicing_model::InvoiceMetadata {
            currency,
            due_date: invoice
                .due_at
                .map(|d| d.date())
                .unwrap_or(finalized_date + chrono::Duration::days(invoice.net_terms as i64)),
            number: invoice.invoice_number,
            issue_date: finalized_date,
            payment_term: invoice.net_terms as u32,
            total_amount: invoice.total,
            tax_amount: invoice.tax_amount,
            subtotal: invoice.subtotal,
            // memo/footer :
            // - either we have one and we use it
            // or we have none and we build from footer from invoicing entitry ?
            // or that's actually 2 differnt things
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
            email: None,                                                     // TODO
            legal_number: None,                                              // TODO
            logo_url: invoicing_entity.logo_attachment_id.as_ref().cloned(), // TODO retrieve the logo from s3
            name: invoice.seller_details.legal_name,
            tax_id: invoice.seller_details.vat_number,
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
                total: line.total,
                description: line.description.clone(),
                quantity: line.quantity,
                vat_rate: Some(Decimal::from(invoice.tax_rate) / Decimal::from(100)),
                unit_price: line.unit_price,
                name: line.name.clone(),
                end_date: line.end_date,
                start_date: line.start_date,
                subtotal: line.subtotal,
                sub_lines: line
                    .sub_lines
                    .iter()
                    .map(|sub_line| invoicing_model::InvoiceSubLine {
                        name: sub_line.name.clone(),
                        total: sub_line.total,
                        quantity: sub_line.quantity,
                        unit_price: sub_line.unit_price,
                    })
                    .collect(),
            })
            .collect();

        let lang = Countries::resolve_country(&invoicing_entity.country)
            .map(|c| c.locale)
            .unwrap_or_else(|| "en-US");

        Ok(invoicing_model::Invoice {
            lang: lang.to_string(),
            customer,
            lines,
            metadata,
            organization,
        })
    }
}
