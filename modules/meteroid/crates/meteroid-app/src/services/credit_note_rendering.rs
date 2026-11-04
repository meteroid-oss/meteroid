use crate::errors::InvoicingRenderError;
use crate::services::einvoicing::{map_format, rejection_outcome};
use crate::services::invoice_rendering::line_credit_units;
use crate::services::storage::{self, ObjectKind, ObjectStoreService};
use common_domain::ids::{CreditNoteId, InvoicingEntityId, StoredDocumentId, TenantId};
use error_stack::{Report, ResultExt};
use image::ImageFormat::Png;
use meteroid_invoicing::{einvoicing, pdf, svg};
use meteroid_store::Store;
use meteroid_store::domain::einvoicing::EInvoicingOutcome;
use meteroid_store::domain::{CreditNote, InvoicingEntity};
use meteroid_store::repositories::CreditNoteInterface;
use meteroid_store::repositories::InvoiceInterface;
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
        let organization_logo = match invoicing_entity.logo_attachment_id {
            Some(logo_id) => get_logo_bytes(self.storage.as_ref(), &self.store, logo_id)
                .await
                .ok(),
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

        let related_invoice = self
            .store
            .get_invoice_by_id(credit_note.tenant_id, credit_note.invoice_id)
            .await
            .change_context(InvoicingRenderError::StoreError)?;

        let credit_units = line_credit_units(
            &self.store,
            related_invoice.tenant_id,
            &related_invoice.line_items,
        )
        .await?;

        let promotional_credit_restored = self
            .store
            .promotional_credit_restored(&credit_note)
            .await
            .change_context(InvoicingRenderError::StoreError)?;

        let mapped = mapper::map_credit_note_to_invoicing(
            credit_note,
            &invoicing_entity,
            organization_logo,
            rate,
            &related_invoice,
            &credit_units,
            promotional_credit_restored,
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
                    error: format!("{error:?}"),
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

        let organization_logo = match invoicing_entity.logo_attachment_id {
            Some(logo_id) => get_logo_bytes(self.storage.as_ref(), &self.store, logo_id)
                .await
                .ok(),
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

        let related_invoice = self
            .store
            .get_invoice_by_id(credit_note.tenant_id, credit_note.invoice_id)
            .await
            .change_context(InvoicingRenderError::StoreError)?;

        let credit_units = line_credit_units(
            &self.store,
            related_invoice.tenant_id,
            &related_invoice.line_items,
        )
        .await?;

        let promotional_credit_restored = self
            .store
            .promotional_credit_restored(&credit_note)
            .await
            .change_context(InvoicingRenderError::StoreError)?;

        let mapped_credit_note = mapper::map_credit_note_to_invoicing(
            credit_note,
            invoicing_entity,
            organization_logo,
            rate,
            &related_invoice,
            &credit_units,
            promotional_credit_restored,
        )?;

        // A credit note is an e-invoice too — BT-3 is `381` and BG-3 names the
        // invoice it credits. Never fatal, for the same reason as an invoice:
        // the credit note is legally issued whether or not it can be expressed
        // in the format, and the report says why it could not.
        let (einvoice, outcome) = match invoicing_entity.einvoicing_format.map(map_format) {
            Some(format) => match einvoicing::generate_credit_note(&mapped_credit_note, format) {
                Ok(document) => (
                    Some((format, document)),
                    Some(EInvoicingOutcome::generated(format.profile_name())),
                ),
                Err(rejection) => {
                    tracing::warn!(
                        credit_note_id = %credit_note_id,
                        profile = format.profile_name(),
                        reason = %rejection,
                        "the credit note is issued but could not be produced as an e-invoice"
                    );
                    (None, Some(rejection_outcome(format, &rejection)))
                }
            },
            None => (None, None),
        };

        let attachment = einvoice.as_ref().and_then(|(_, document)| {
            document.filename.map(
                |filename| meteroid_invoicing::typst_render::EInvoiceAttachment {
                    filename: filename.to_string(),
                    xml: document.xml.clone().into_bytes(),
                    description: "Structured credit note data (EN 16931)".to_string(),
                },
            )
        });

        let pdf = self
            .pdf
            .generate_credit_note_pdf(&mapped_credit_note, attachment.as_ref())
            .await
            .change_context(InvoicingRenderError::PdfError)?;

        let pdf = match (&einvoice, &attachment) {
            (Some((format, _)), Some(attachment)) => {
                einvoicing::stamp_facturx_xmp(&pdf, *format, &attachment.filename)
                    .map(bytes::Bytes::from)
                    .change_context(InvoicingRenderError::PdfError)?
            }
            _ => pdf,
        };

        let stored = self
            .storage
            .store(ObjectKind::CreditNotePdf, tenant_id, pdf)
            .await
            .change_context(InvoicingRenderError::StorageError)?;
        let pdf_id = stored.id;

        let stored_xml = match &einvoice {
            Some((_, document)) => Some(
                self.storage
                    .store(
                        ObjectKind::InvoiceXml,
                        tenant_id,
                        bytes::Bytes::from(document.xml.clone().into_bytes()),
                    )
                    .await
                    .change_context(InvoicingRenderError::StorageError)?,
            ),
            None => None,
        };

        self.store
            .save_credit_note_documents(credit_note_id, tenant_id, stored, stored_xml, outcome)
            .await
            .change_context(InvoicingRenderError::StoreError)?;

        Ok(pdf_id)
    }
}

pub mod mapper {
    use crate::errors::InvoicingRenderError;
    use error_stack::Report;
    use meteroid_invoicing::credit_note_model as invoicing_model;
    use meteroid_store::constants::Countries;
    use meteroid_store::domain as store_model;
    use meteroid_store::domain::credit_units::CreditUnitsById;
    use meteroid_store::domain::historical_rates::HistoricalRate;
    use rust_decimal::Decimal;
    use rust_decimal::prelude::FromPrimitive;

    pub fn map_credit_note_to_invoicing(
        credit_note: store_model::CreditNote,
        invoicing_entity: &store_model::InvoicingEntity,
        organization_logo_bytes: Option<Vec<u8>>,
        accounting_rate: Option<HistoricalRate>,
        related_invoice: &store_model::Invoice,
        credit_units: &CreditUnitsById,
        promotional_credit_restored: i64,
    ) -> Result<invoicing_model::CreditNote, Report<InvoicingRenderError>> {
        let related_invoice_date = related_invoice.invoice_date;
        let issue_date = credit_note
            .finalized_at
            .map(|d| d.date())
            .unwrap_or_else(|| credit_note.created_at.date());

        let currency = meteroid_money::iso::find(&credit_note.currency).ok_or_else(|| {
            Report::new(InvoicingRenderError::InvalidCurrency(
                credit_note.currency.clone(),
            ))
        })?;

        let accounting_currency = *meteroid_money::iso::find(&invoicing_entity.accounting_currency)
            .ok_or_else(|| {
                Report::new(InvoicingRenderError::InvalidCurrency(
                    invoicing_entity.accounting_currency.clone(),
                ))
            })?;

        let credit_type = match credit_note.credit_type {
            store_model::CreditType::CreditToBalance => {
                invoicing_model::CreditType::CreditToBalance
            }
            store_model::CreditType::Refund => invoicing_model::CreditType::Refund,
            store_model::CreditType::DebtCancellation => {
                invoicing_model::CreditType::DebtCancellation
            }
        };

        let promotional_credit_restored_estimated =
            credit_note.status == store_model::CreditNoteStatus::Draft;
        let metadata = invoicing_model::CreditNoteMetadata {
            currency,
            number: credit_note.credit_note_number,
            related_invoice_number: credit_note.invoice_number.clone(),
            related_invoice_date,
            issue_date,
            total_amount: meteroid_money::Money::from_minor(credit_note.total, currency),
            tax_amount: meteroid_money::Money::from_minor(credit_note.tax_amount, currency),
            subtotal: meteroid_money::Money::from_minor(credit_note.subtotal, currency),
            reason: credit_note.reason.clone(),
            memo: credit_note.memo.clone(),
            credit_type,
            refunded_amount: meteroid_money::Money::from_minor(
                credit_note.refunded_amount_cents,
                currency,
            ),
            credited_amount: meteroid_money::Money::from_minor(
                credit_note.credited_amount_cents,
                currency,
            ),
            debt_cancelled_amount: meteroid_money::Money::from_minor(
                credit_note.debt_cancelled_amount_cents,
                currency,
            ),
            promotional_credit_restored: meteroid_money::Money::from_minor(
                promotional_credit_restored,
                currency,
            ),
            promotional_credit_restored_estimated,
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

        // Resolve the document language up front so the footer can select its
        // locale-specific override. Both sides come from the credit note's own
        // snapshot, so re-rendering never changes the language it was worded in.
        let lang = Countries::resolve_template_lang(
            credit_note.customer_details.preferred_locales.as_slice(),
            credit_note.seller_details.invoicing_language.as_deref(),
            &invoicing_entity.country.code,
            format_args!("credit note {}", credit_note.id),
        );
        let (footer_info, footer_legal) = invoicing_entity.invoice_footer_translations.resolve(
            lang,
            invoicing_entity.invoice_footer_info.as_deref(),
            invoicing_entity.invoice_footer_legal.as_deref(),
        );

        let organization = invoicing_model::Organization {
            address: map_address(credit_note.seller_details.address),
            email: credit_note.seller_details.contact_email,
            logo_src: organization_logo_bytes,
            name: credit_note.seller_details.legal_name,
            tax_id: credit_note.seller_details.vat_number,
            legal_number: credit_note.seller_details.legal_number,
            footer_info,
            footer_legal,
            accounting_currency,
            exchange_rate: accounting_rate.and_then(|r| Decimal::from_f64(r.rate)),
            contact_name: credit_note.seller_details.contact_name,
            contact_phone: credit_note.seller_details.contact_phone,
        };

        let customer = invoicing_model::Customer {
            address: credit_note
                .customer_details
                .billing_address
                .map(map_address)
                .unwrap_or_default(),
            email: credit_note.customer_details.email,
            name: credit_note.customer_details.name,
            tax_id: credit_note.customer_details.vat_number,
            legal_number: credit_note.customer_details.legal_number,
            buyer_reference: credit_note.customer_details.buyer_reference,
        };

        let lines = credit_note
            .line_items
            .iter()
            .map(|line| {
                let source = related_invoice
                    .line_items
                    .iter()
                    .find(|source| source.local_id == line.local_id);
                let (name, description) = crate::services::invoice_rendering::purchase_line_text(
                    line,
                    source.unwrap_or(line),
                    lang,
                    &credit_note.currency,
                    credit_units,
                )
                .unwrap_or_else(|| (line.name.clone(), line.description.clone()));
                invoicing_model::CreditNoteLine {
                    description,
                    quantity: line.quantity,
                    is_prorated: line.is_prorated,
                    tax_rate: line.tax_rate,
                    unit_price: line
                        .unit_price
                        .map(|p| meteroid_money::Money::from_decimal(p, currency)),
                    name,
                    end_date: line.end_date,
                    start_date: line.start_date,
                    subtotal: meteroid_money::Money::from_minor(line.amount_subtotal, currency),
                    taxable_amount: meteroid_money::Money::from_minor(
                        line.taxable_amount,
                        currency,
                    ),
                    promotional_credit: Some(line.note_net_promotional_credit(source))
                        .filter(|share| *share != 0)
                        .map(|share| meteroid_money::Money::from_minor(share, currency)),
                    sub_lines: line
                        .sub_lines
                        .iter()
                        .map(|sub_line| invoicing_model::CreditNoteSubLine {
                            name: sub_line.name.clone(),
                            total: meteroid_money::Money::from_minor(sub_line.total, currency),
                            quantity: sub_line.quantity,
                            unit_price: meteroid_money::Money::from_decimal(
                                sub_line.unit_price,
                                currency,
                            ),
                        })
                        .collect(),
                }
            })
            .collect();

        let tax_breakdown = credit_note
            .tax_breakdown
            .iter()
            .map(|t| {
                use meteroid_invoicing::credit_note_model::SupplyBasis as InvoicingBasis;
                use meteroid_invoicing::credit_note_model::TaxExemptionType as InvoicingExemption;
                use meteroid_store::domain::invoices::SupplyBasis as StoreBasis;
                use meteroid_store::domain::invoices::TaxExemptionType as StoreExemption;

                let exemption_type = t.exemption_type.as_ref().map(|e| match e {
                    StoreExemption::ReverseCharge => InvoicingExemption::ReverseCharge,
                    StoreExemption::TaxExempt => InvoicingExemption::TaxExempt,
                    StoreExemption::NotRegistered => InvoicingExemption::NotRegistered,
                    StoreExemption::Export => InvoicingExemption::Export,
                    StoreExemption::NoVatTerritory => InvoicingExemption::NoVatTerritory,
                });

                let supply_basis = t.supply_basis.map(|b| match b {
                    StoreBasis::Business => InvoicingBasis::Business,
                    StoreBasis::Consumer => InvoicingBasis::Consumer,
                });

                // What the levy is, so the renderer can word the row label in the
                // document language; `None` (merchant custom taxes, rows persisted
                // before the kind was recorded) keeps the stored name verbatim.
                use meteroid_invoicing::credit_note_model::TaxKind as InvoicingKind;
                use meteroid_store::domain::invoices::TaxKind as StoreKind;
                let tax_kind = t.tax_kind.map(|k| match k {
                    StoreKind::Vat => InvoicingKind::Vat,
                    StoreKind::Gst => InvoicingKind::Gst,
                    StoreKind::Hst => InvoicingKind::Hst,
                    StoreKind::Pst => InvoicingKind::Pst,
                    StoreKind::Qst => InvoicingKind::Qst,
                    StoreKind::SalesTax => InvoicingKind::SalesTax,
                });

                invoicing_model::TaxBreakdownItem {
                    name: t.name.clone(),
                    tax_kind,
                    rate: t.tax_rate,
                    // Negate to match the credit note's negative subtotal/total rendering
                    amount: meteroid_money::Money::from_minor(-t.tax_amount, currency),
                    taxable_amount: meteroid_money::Money::from_minor(-t.taxable_amount, currency),
                    exemption_reason: t.exemption_reason.clone(),
                    exemption_type,
                    supply_basis,
                    prepaid_credit: t.prepaid_credit,
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
    object_store: &dyn ObjectStoreService,
    store: &Store,
    id: StoredDocumentId,
) -> Result<Vec<u8>, Report<InvoicingRenderError>> {
    let logo = storage::read(object_store, store, id)
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
