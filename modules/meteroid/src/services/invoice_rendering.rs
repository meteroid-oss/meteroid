use error_stack::ResultExt;
use meteroid_invoicing::InvoicingPdfService;
use meteroid_store::domain::{Invoice, InvoicingEntity};
use meteroid_store::errors::StoreError;
use meteroid_store::external::invoice_rendering::{GenerateResult, InvoiceRenderingService};
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::{Store, StoreResult};
use std::sync::Arc;
use uuid::Uuid;

struct InvoiceRenderingImpl {
    invoicing_service: InvoicingPdfService,
    store: Arc<Store>,
}

#[async_trait::async_trait]
impl InvoiceRenderingService for InvoiceRenderingImpl {
    async fn preview_invoice_html(&self, invoice_id: Uuid, tenant_id: Uuid) -> StoreResult<String> {
        let invoice = self.store.find_invoice_by_id(tenant_id, invoice_id).await?;
        let invoicing_entity = self
            .store
            .get_invoicing_entity(tenant_id, Some(invoice.invoice.seller_details.id))
            .await?;

        Ok(self
            .invoicing_service
            .preview_invoice(mapper::map_invoice_to_invoicing(
                invoice.invoice,
                &invoicing_entity,
            )?)
            .change_context(StoreError::InvoicingError)?)
    }

    async fn generate_pdfs(&self, invoice_ids: Vec<Uuid>) -> StoreResult<Vec<GenerateResult>> {
        let invoices = self.store.list_invoices_by_ids(invoice_ids).await?;
        let invoicing_entity_ids = invoices
            .iter()
            .map(|invoice| invoice.seller_details.id)
            .collect::<Vec<Uuid>>();

        let invoicing_entities = self
            .store
            .list_invoicing_entities_by_ids(invoicing_entity_ids)
            .await?;

        async fn generate_pdf_and_save(
            this: &InvoiceRenderingImpl,
            invoice: Invoice,
            invoicing_entities: &[InvoicingEntity],
        ) -> StoreResult<String> {
            let invoicing_entity = invoicing_entities
                .iter()
                .find(|entity| entity.id == invoice.seller_details.id)
                .ok_or_else(|| {
                    StoreError::ValueNotFound("Failed to resolve invoicing entity".to_string())
                })?;

            let invoice_id = invoice.id.clone();
            let tenant_id = invoice.tenant_id.clone();

            let mapped_invoice = mapper::map_invoice_to_invoicing(invoice, invoicing_entity)?;
            let pdf_url = this
                .invoicing_service
                .generate_invoice_document(mapped_invoice)
                .await
                .change_context(StoreError::InvoicingError)?;

            this.store
                .save_invoice_documents(invoice_id, tenant_id, pdf_url.clone(), None)
                .await?;
            Ok(pdf_url)
        }

        let mut results = vec![];

        for invoice in invoices {
            let invoice_id = invoice.id.clone();
            let res = match generate_pdf_and_save(self, invoice, &invoicing_entities).await {
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
}

mod mapper {
    use meteroid_invoicing::model as invoicing_model;
    use meteroid_invoicing::model::Address;
    use meteroid_store::constants::Countries;
    use meteroid_store::errors::StoreError;
    use meteroid_store::{domain as store_model, StoreResult};
    use rust_decimal::Decimal;

    pub fn map_invoice_to_invoicing(
        invoice: store_model::Invoice,
        invoicing_entity: &store_model::InvoicingEntity,
    ) -> StoreResult<invoicing_model::Invoice> {
        let finalized_date = invoice
            .finalized_at
            .map(|d| d.date())
            .unwrap_or(invoice.invoice_date);

        let currency = rusty_money::iso::find(&invoice.currency)
            .ok_or_else(|| StoreError::InvalidCurrency(invoice.currency.clone()))?
            .clone();

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
                .unwrap_or_else(|| Address::default()),
            email: invoice.customer_details.email,
            legal_number: None, // TODO
            name: invoice.customer_details.name,
            tax_id: invoice.customer_details.vat_number,
        };

        // invoicing_model::InvoiceLine

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
