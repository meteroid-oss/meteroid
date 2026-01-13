use crate::errors::InvoicingRenderError;
use crate::services::storage::{ObjectStoreService, Prefix};
use common_domain::ids::{CreditNoteId, InvoicingEntityId, StoredDocumentId, TenantId};
use error_stack::{Report, ResultExt};
use image::ImageFormat::Png;
use meteroid_invoicing::{pdf, svg};
use meteroid_store::Store;
use meteroid_store::domain::{CreditNote, InvoicingEntity};
use meteroid_store::repositories::CreditNoteInterface;
use meteroid_store::repositories::historical_rates::HistoricalRatesInterface;
use meteroid_store::repositories::invoicing_entities::{
    InvoicingEntityInterface, InvoicingEntityInterfaceAuto,
};
use std::io::Cursor;
use std::sync::Arc;

#[derive(Clone)]
pub struct CreditNotePreviewRenderingService {
    store: Arc<Store>,
    generator: Arc<dyn svg::CreditNoteSvgGenerator>,
    storage: Arc<dyn ObjectStoreService>,
}

impl CreditNotePreviewRenderingService {
    pub fn try_new(
        store: Arc<Store>,
        storage: Arc<dyn ObjectStoreService>,
    ) -> Result<Self, Report<InvoicingRenderError>> {
        let generator = svg::TypstCreditNoteSvgGenerator::new()
            .change_context(InvoicingRenderError::InitializationError)
            .attach("Typst credit note SVG generator failed to initialize")?;

        Ok(Self {
            store,
            generator: Arc::new(generator),
            storage,
        })
    }

    pub async fn preview_credit_note(
        &self,
        credit_note: CreditNote,
        invoicing_entity: InvoicingEntity,
    ) -> Result<Vec<String>, Report<InvoicingRenderError>> {
        let organization_logo = match invoicing_entity.logo_attachment_id.as_ref() {
            Some(logo_id) => get_logo_bytes(&self.storage, *logo_id).await.ok(),
            None => None,
        };

        let mut rate = None;
        if credit_note.currency != invoicing_entity.accounting_currency {
            rate = self
                .store
                .get_historical_rate(
                    &credit_note.currency,
                    &invoicing_entity.accounting_currency,
                    credit_note
                        .finalized_at
                        .map(|d| d.date())
                        .unwrap_or_else(|| credit_note.created_at.date()),
                )
                .await
                .change_context(InvoicingRenderError::StoreError)?;
        }

        let mapped = mapper::map_credit_note_to_invoicing(
            credit_note,
            &invoicing_entity,
            organization_logo,
            rate,
        )?;

        let svgs = self
            .generator
            .generate_credit_note_svg(&mapped)
            .await
            .change_context(InvoicingRenderError::RenderError)?;

        Ok(svgs)
    }

    pub async fn preview_credit_note_by_id(
        &self,
        credit_note_id: CreditNoteId,
        tenant_id: TenantId,
    ) -> Result<Vec<String>, Report<InvoicingRenderError>> {
        let credit_note = self
            .store
            .get_credit_note_by_id(tenant_id, credit_note_id)
            .await
            .change_context(InvoicingRenderError::StoreError)?;

        let invoicing_entity = self
            .store
            .get_invoicing_entity(tenant_id, Some(credit_note.invoicing_entity_id))
            .await
            .change_context(InvoicingRenderError::StoreError)?;

        self.preview_credit_note(credit_note, invoicing_entity)
            .await
    }
}

pub enum CreditNoteGenerateResult {
    Success {
        credit_note_id: CreditNoteId,
        tenant_id: TenantId,
        pdf_id: StoredDocumentId,
    },
    Failure {
        credit_note_id: CreditNoteId,
        error: String,
    },
}

#[derive(Clone)]
pub struct CreditNotePdfRenderingService {
    storage: Arc<dyn ObjectStoreService>,
    pdf: Arc<dyn pdf::CreditNotePdfGenerator>,
    store: Arc<Store>,
}

impl CreditNotePdfRenderingService {
    pub fn try_new(
        storage: Arc<dyn ObjectStoreService>,
        store: Arc<Store>,
    ) -> Result<Self, Report<InvoicingRenderError>> {
        let pdf_generator = Arc::new(
            pdf::TypstCreditNotePdfGenerator::new()
                .change_context(InvoicingRenderError::InitializationError)
                .attach("Typst credit note PDF generator failed to initialize")?,
        );

        Ok(Self {
            storage,
            pdf: pdf_generator,
            store,
        })
    }

    pub async fn generate_pdfs(
        &self,
        credit_note_ids: Vec<CreditNoteId>,
    ) -> Result<Vec<CreditNoteGenerateResult>, Report<InvoicingRenderError>> {
        let credit_notes = self
            .store
            .list_credit_notes_by_ids(credit_note_ids)
            .await
            .change_context(InvoicingRenderError::StoreError)?;

        let invoicing_entity_ids = credit_notes
            .iter()
            .map(|cn| cn.invoicing_entity_id)
            .collect::<Vec<InvoicingEntityId>>();

        let invoicing_entities = self
            .store
            .list_invoicing_entities_by_ids(invoicing_entity_ids)
            .await
            .change_context(InvoicingRenderError::StoreError)?;

        let mut results = vec![];

        for credit_note in credit_notes {
            let credit_note_id = credit_note.id;
            let tenant_id = credit_note.tenant_id;

            if let Some(pdf_id) = credit_note.pdf_document_id {
                results.push(CreditNoteGenerateResult::Success {
                    credit_note_id,
                    tenant_id,
                    pdf_id,
                });
                continue;
            }

            let res = match self
                .generate_pdf_and_save(credit_note, &invoicing_entities)
                .await
            {
                Err(error) => CreditNoteGenerateResult::Failure {
                    credit_note_id,
                    error: error.to_string(),
                },
                Ok(pdf_id) => CreditNoteGenerateResult::Success {
                    credit_note_id,
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
        credit_note: CreditNote,
        invoicing_entities: &[InvoicingEntity],
    ) -> Result<StoredDocumentId, Report<InvoicingRenderError>> {
        let invoicing_entity = invoicing_entities
            .iter()
            .find(|entity| entity.id == credit_note.invoicing_entity_id)
            .ok_or(InvoicingRenderError::StoreError)
            .attach("Failed to resolve invoicing entity")?;

        let credit_note_id = credit_note.id;
        let tenant_id = credit_note.tenant_id;

        let organization_logo = match invoicing_entity.logo_attachment_id.as_ref() {
            Some(logo_id) => get_logo_bytes(&self.storage, *logo_id).await.ok(),
            None => None,
        };

        let mut rate = None;
        if credit_note.currency != invoicing_entity.accounting_currency {
            rate = self
                .store
                .get_historical_rate(
                    &credit_note.currency,
                    &invoicing_entity.accounting_currency,
                    credit_note
                        .finalized_at
                        .map(|d| d.date())
                        .unwrap_or_else(|| credit_note.created_at.date()),
                )
                .await
                .change_context(InvoicingRenderError::StoreError)?;
        }

        let mapped_credit_note = mapper::map_credit_note_to_invoicing(
            credit_note,
            invoicing_entity,
            organization_logo,
            rate,
        )?;

        let pdf = self
            .pdf
            .generate_credit_note_pdf(&mapped_credit_note)
            .await
            .change_context(InvoicingRenderError::PdfError)?;

        let pdf_id = self
            .storage
            .store(pdf, Prefix::CreditNotePdf)
            .await
            .change_context(InvoicingRenderError::StorageError)?;

        self.store
            .save_credit_note_pdf_document(credit_note_id, tenant_id, pdf_id)
            .await
            .change_context(InvoicingRenderError::StoreError)?;

        Ok(pdf_id)
    }
}

mod mapper {
    use crate::errors::InvoicingRenderError;
    use error_stack::Report;
    use meteroid_invoicing::credit_note_model as invoicing_model;
    use meteroid_store::constants::Countries;
    use meteroid_store::domain as store_model;
    use meteroid_store::domain::historical_rates::HistoricalRate;
    use rust_decimal::Decimal;
    use rust_decimal::prelude::FromPrimitive;

    pub fn map_credit_note_to_invoicing(
        credit_note: store_model::CreditNote,
        invoicing_entity: &store_model::InvoicingEntity,
        organization_logo_bytes: Option<Vec<u8>>,
        accounting_rate: Option<HistoricalRate>,
    ) -> Result<invoicing_model::CreditNote, Report<InvoicingRenderError>> {
        let issue_date = credit_note
            .finalized_at
            .map(|d| d.date())
            .unwrap_or_else(|| credit_note.created_at.date());

        let currency = rusty_money::iso::find(&credit_note.currency).ok_or_else(|| {
            Report::new(InvoicingRenderError::InvalidCurrency(
                credit_note.currency.clone(),
            ))
        })?;

        let accounting_currency = *rusty_money::iso::find(&invoicing_entity.accounting_currency)
            .ok_or_else(|| {
                Report::new(InvoicingRenderError::InvalidCurrency(
                    invoicing_entity.accounting_currency.clone(),
                ))
            })?;

        // Determine credit type from amounts
        let credit_type = if credit_note.refunded_amount_cents > 0 {
            invoicing_model::CreditType::Refund
        } else {
            invoicing_model::CreditType::CreditToBalance
        };

        let metadata = invoicing_model::CreditNoteMetadata {
            currency,
            number: credit_note.credit_note_number,
            related_invoice_number: credit_note.invoice_number.clone(),
            issue_date,
            total_amount: rusty_money::Money::from_minor(credit_note.total, currency),
            tax_amount: rusty_money::Money::from_minor(credit_note.tax_amount, currency),
            subtotal: rusty_money::Money::from_minor(credit_note.subtotal, currency),
            reason: credit_note.reason.clone(),
            memo: credit_note.memo.clone(),
            credit_type,
            refunded_amount: rusty_money::Money::from_minor(
                credit_note.refunded_amount_cents,
                currency,
            ),
            credited_amount: rusty_money::Money::from_minor(
                credit_note.credited_amount_cents,
                currency,
            ),
            flags: invoicing_model::Flags {
                show_tax_info: Some(true),
                show_legal_info: Some(true),
                show_footer_custom_info: Some(true),
                whitelabel: Some(false),
            },
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
            address: map_address(credit_note.seller_details.address),
            email: None,
            legal_number: None,
            logo_src: organization_logo_bytes,
            name: credit_note.seller_details.legal_name,
            tax_id: credit_note.seller_details.vat_number,
            footer_info: invoicing_entity.invoice_footer_info.clone(),
            footer_legal: invoicing_entity.invoice_footer_legal.clone(),
            accounting_currency,
            exchange_rate: accounting_rate.and_then(|r| Decimal::from_f32(r.rate)),
        };

        let customer = invoicing_model::Customer {
            address: credit_note
                .customer_details
                .billing_address
                .map(map_address)
                .unwrap_or_default(),
            email: credit_note.customer_details.email,
            legal_number: None,
            name: credit_note.customer_details.name,
            tax_id: credit_note.customer_details.vat_number,
        };

        let lines = credit_note
            .line_items
            .iter()
            .map(|line| invoicing_model::CreditNoteLine {
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
                    .map(|sub_line| invoicing_model::CreditNoteSubLine {
                        name: sub_line.name.clone(),
                        total: rusty_money::Money::from_minor(sub_line.total, currency),
                        quantity: sub_line.quantity,
                        unit_price: rusty_money::Money::from_decimal(sub_line.unit_price, currency),
                    })
                    .collect(),
            })
            .collect();

        let lang = Countries::resolve_country(&invoicing_entity.country.code)
            .map_or_else(|| "en-US", |c| c.locale);

        let tax_breakdown = credit_note
            .tax_breakdown
            .iter()
            .map(|t| {
                use meteroid_invoicing::credit_note_model::TaxExemptionType as InvoicingExemption;
                use meteroid_store::domain::invoices::TaxExemptionType as StoreExemption;

                let exemption_type = t.exemption_type.as_ref().map(|e| match e {
                    StoreExemption::ReverseCharge => InvoicingExemption::ReverseCharge,
                    StoreExemption::TaxExempt => InvoicingExemption::TaxExempt,
                    StoreExemption::NotRegistered => InvoicingExemption::NotRegistered,
                });

                invoicing_model::TaxBreakdownItem {
                    name: t.name.clone(),
                    rate: t.tax_rate,
                    amount: rusty_money::Money::from_minor(t.tax_amount as i64, currency),
                    exemption_type,
                }
            })
            .collect();

        Ok(invoicing_model::CreditNote {
            lang: lang.to_string(),
            customer,
            lines,
            metadata,
            organization,
            tax_breakdown,
        })
    }
}

async fn get_logo_bytes(
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
