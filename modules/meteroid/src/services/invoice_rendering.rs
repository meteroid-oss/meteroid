use crate::errors::InvoicingRenderError;
use crate::services::storage::{ObjectStoreService, Prefix};
use common_domain::ids::{InvoiceId, InvoicingEntityId, StoredDocumentId, TenantId};
use error_stack::ResultExt;
use image::ImageFormat::Png;
use meteroid_invoicing::{pdf, svg};
use meteroid_store::Store;
use meteroid_store::domain::{Invoice, InvoicingEntity};
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::repositories::historical_rates::HistoricalRatesInterface;
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;
use std::io::Cursor;
use std::sync::Arc;

pub struct InvoicePreviewRenderingService {
    store: Arc<Store>,
    generator: Arc<dyn svg::SvgGenerator>,
    storage: Arc<dyn ObjectStoreService>,
}

impl InvoicePreviewRenderingService {
    pub fn try_new(
        store: Arc<Store>,
        storage: Arc<dyn ObjectStoreService>,
    ) -> error_stack::Result<Self, InvoicingRenderError> {
        let generator = svg::TypstSvgGenerator::new()
            .change_context(InvoicingRenderError::InitializationError)
            .attach_printable("Typst SVG generator failed to initialize")?;

        Ok(Self {
            store,
            generator: Arc::new(generator),
            storage,
        })
    }

    pub async fn preview_invoice(
        &self,
        invoice: Invoice,
        invoicing_entity: InvoicingEntity,
    ) -> error_stack::Result<String, InvoicingRenderError> {
        let organization_logo = match invoicing_entity.logo_attachment_id.as_ref() {
            Some(logo_id) => {
                let res = get_logo_bytes_for_invoice(&self.storage, *logo_id).await?;
                Some(res)
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

        let mapped =
            mapper::map_invoice_to_invoicing(invoice, &invoicing_entity, organization_logo, rate)?;

        let svg_string = self
            .generator
            .generate_svg(&mapped)
            .await
            .change_context(InvoicingRenderError::RenderError)?;

        Ok(svg_string)
    }

    pub async fn preview_invoice_by_id(
        &self,
        invoice_id: InvoiceId,
        tenant_id: TenantId,
    ) -> error_stack::Result<String, InvoicingRenderError> {
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
}

impl PdfRenderingService {
    pub fn try_new(
        storage: Arc<dyn ObjectStoreService>,
        store: Arc<Store>,
    ) -> error_stack::Result<Self, InvoicingRenderError> {
        let pdf_generator = Arc::new(
            pdf::TypstPdfGenerator::new()
                .change_context(InvoicingRenderError::InitializationError)
                .attach_printable("Typst PDF generator failed to initialize")?,
        );

        Ok(Self {
            storage,
            pdf: pdf_generator,
            store,
        })
    }

    pub async fn generate_pdfs(
        &self,
        invoice_ids: Vec<InvoiceId>,
    ) -> error_stack::Result<Vec<GenerateResult>, InvoicingRenderError> {
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
    ) -> error_stack::Result<StoredDocumentId, InvoicingRenderError> {
        let invoicing_entity = invoicing_entities
            .iter()
            .find(|entity| entity.id == invoice.seller_details.id)
            .ok_or(InvoicingRenderError::StoreError)
            .attach_printable("Failed to resolve invoicing entity")?;

        let invoice_id = invoice.id;
        let tenant_id = invoice.tenant_id;

        // let's resolve the logo as raw bytes
        let organization_logo = match invoicing_entity.logo_attachment_id.as_ref() {
            Some(logo_id) => {
                let res = get_logo_bytes_for_invoice(&self.storage, *logo_id).await?;
                Some(res)
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

        let customer_id = invoice.customer_id;

        let mapped_invoice =
            mapper::map_invoice_to_invoicing(invoice, invoicing_entity, organization_logo, rate)?;

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

    pub fn map_invoice_to_invoicing(
        invoice: store_model::Invoice,
        invoicing_entity: &store_model::InvoicingEntity,
        organization_logo_bytes: Option<Vec<u8>>,
        accounting_rate: Option<HistoricalRate>,
    ) -> error_stack::Result<invoicing_model::Invoice, InvoicingRenderError> {
        let finalized_date = invoice
            .finalized_at
            .map(|d| d.date())
            .unwrap_or(invoice.invoice_date);

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
            due_date: invoice
                .due_at
                .map(|d| d.date())
                .unwrap_or(finalized_date + chrono::Duration::days(invoice.net_terms as i64)),
            number: invoice.invoice_number,
            issue_date: finalized_date,
            payment_term: invoice.net_terms as u32,
            total_amount: rusty_money::Money::from_minor(invoice.total, currency),
            tax_amount: rusty_money::Money::from_minor(invoice.tax_amount, currency),
            subtotal: rusty_money::Money::from_minor(invoice.subtotal, currency),
            discount: rusty_money::Money::from_minor(invoice.discount, currency),
            memo: invoice.memo.clone(),
            payment_url: None, // TODO
            flags: Flags::default(),
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

        let lang = Countries::resolve_country(&invoicing_entity.country)
            .map(|c| c.locale)
            .unwrap_or_else(|| "en-US");

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
            bank_details: None,       // TODO
            transactions: Vec::new(), // TODO
            payment_status: None,     // TODO
        })
    }
}

async fn get_logo_bytes_for_invoice(
    storage: &Arc<dyn ObjectStoreService>,
    logo_id: StoredDocumentId,
) -> error_stack::Result<Vec<u8>, InvoicingRenderError> {
    let logo = storage
        .retrieve(logo_id, Prefix::ImageLogo)
        .await
        .change_context(InvoicingRenderError::StorageError)?;

    let mut img =
        image::load_from_memory(&logo).change_context(InvoicingRenderError::RenderError)?;
    img = img.resize(350, 20, image::imageops::FilterType::Nearest);
    let mut buffer = Vec::new();
    img.write_to(&mut Cursor::new(&mut buffer), Png)
        .change_context(InvoicingRenderError::RenderError)?;

    Ok(buffer)
}
