use crate::errors::InvoicingRenderError;
use crate::services::einvoicing::{map_format, rejection_outcome};
use crate::services::storage::{self, ObjectKind, ObjectStoreService};
use common_domain::ids::{InvoiceId, InvoicingEntityId, StoredDocumentId, TenantId};
use error_stack::{Report, ResultExt};
use image::ImageFormat::Png;
use meteroid_invoicing::{einvoicing, pdf, svg};
use meteroid_store::Store;
use meteroid_store::domain::credit_units::CreditUnitsById;
use meteroid_store::domain::einvoicing::{EInvoicingOutcome, EInvoicingReport};
use meteroid_store::domain::subscriptions::PaymentMethodsConfig;
use meteroid_store::domain::{Invoice, InvoicingEntity, LineItem, ResolvedPaymentMethod};
use meteroid_store::jwt_claims::{ResourceAccess, generate_portal_token};
use meteroid_store::repositories::CreditUnitsInterface;
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::repositories::bank_accounts::BankAccountsInterface;
use meteroid_store::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use meteroid_store::repositories::customers::CustomersInterfaceAuto;
use meteroid_store::repositories::historical_rates::HistoricalRatesInterface;
use meteroid_store::repositories::invoicing_entities::{
    InvoicingEntityInterface, InvoicingEntityInterfaceAuto,
};
use meteroid_store::repositories::subscriptions::SubscriptionInterface;
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

/// Builds a map of line `local_id` -> plan grouping label for a consolidated invoice. Returns
/// an empty map for non-consolidated invoices (or when all members share a single plan), in
/// which case the renderer falls back to a flat, ungrouped line list.
async fn build_line_groups(
    store: &Arc<Store>,
    invoice: &Invoice,
) -> Result<HashMap<String, String>, Report<InvoicingRenderError>> {
    // Only consolidated parents (which have no subscription of their own) can have members.
    if invoice.subscription_id.is_some() {
        return Ok(HashMap::new());
    }

    let children = store
        .list_consolidated_children(invoice.tenant_id, invoice.id)
        .await
        .change_context(InvoicingRenderError::StoreError)?;

    if children.is_empty() {
        return Ok(HashMap::new());
    }

    let mut map: HashMap<String, String> = HashMap::new();
    let mut distinct: std::collections::HashSet<String> = std::collections::HashSet::new();
    for child in &children {
        let label = child
            .plan_name
            .clone()
            .unwrap_or_else(|| "Subscription".to_string());
        distinct.insert(label.clone());
        for line in &child.line_items {
            map.insert(line.local_id.clone(), label.clone());
        }
    }

    // Only group when there is more than one plan to distinguish.
    if distinct.len() < 2 {
        return Ok(HashMap::new());
    }

    Ok(map)
}

/// The credit units the lines are priced in or sell, archived ones included: the documents
/// name a unit, never print its code.
pub async fn line_credit_units(
    store: &Arc<Store>,
    tenant_id: TenantId,
    lines: &[LineItem],
) -> Result<CreditUnitsById, Report<InvoicingRenderError>> {
    let ids = lines
        .iter()
        .flat_map(|line| {
            [
                line.credit_unit.as_ref().map(|unit| unit.unit_id),
                line.credit_grant_unit.as_ref().map(|unit| unit.unit_id),
            ]
        })
        .flatten()
        .collect::<std::collections::HashSet<_>>();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    store
        .get_credit_units_by_ids(tenant_id, ids.into_iter().collect())
        .await
        .change_context(InvoicingRenderError::StoreError)
}

fn document_purchase_name(
    name: meteroid_store::domain::CreditPurchaseLineName,
) -> meteroid_invoicing::typst_render::CreditPurchaseName {
    use meteroid_invoicing::typst_render::CreditPurchaseName;
    use meteroid_store::domain::CreditPurchaseLineName;
    match name {
        CreditPurchaseLineName::WalletTopUp => CreditPurchaseName::WalletTopUp,
        CreditPurchaseLineName::AutomaticTopUp => CreditPurchaseName::AutomaticTopUp,
        CreditPurchaseLineName::Purchase => CreditPurchaseName::Purchase,
    }
}

/// A purchase document's memo is its line's stored name: worded like the line.
pub fn purchase_memo(memo: Option<String>, lines: &[LineItem], lang: &str) -> Option<String> {
    let name = memo
        .as_deref()
        .and_then(meteroid_store::domain::CreditPurchaseLineName::from_stored_name)
        .filter(|_| {
            lines
                .iter()
                .any(|line| line.credit_purchase_name().is_some())
        });
    match name {
        Some(name) => Some(meteroid_invoicing::typst_render::credit_purchase_name(
            lang,
            document_purchase_name(name),
        )),
        None => memo,
    }
}

/// The name and description of a credit-purchase line the platform wrote, in the document's
/// language. `source` is the sold line (the invoice's own, or the one a credit note reverses).
/// A plan's pack, or a line under any other name, keeps the text it was stored with.
pub fn purchase_line_text(
    line: &LineItem,
    source: &LineItem,
    lang: &str,
    currency: &str,
    credit_units: &CreditUnitsById,
) -> Option<(String, Option<String>)> {
    use meteroid_invoicing::typst_render::{
        CreditPurchaseDetail, CreditPurchaseName, credit_purchase_description, credit_purchase_name,
    };
    use meteroid_store::domain::price_components::CreditExpiry;

    let name = document_purchase_name(line.credit_purchase_name()?);
    let valid_months = match source.credit_grant_expiry {
        Some(CreditExpiry::Month(months)) => Some(months),
        _ => None,
    };
    let stored = || Some((credit_purchase_name(lang, name), line.description.clone()));
    let detail = match &source.credit_grant_unit {
        // The credits this line moves (a note's reversal, not the whole sale), named by the unit
        // as sold: a unit renamed since keeps the wording it was issued with.
        Some(unit_ref) => match (
            credit_units.get(&unit_ref.unit_id),
            line.credit_grant_amount,
        ) {
            (Some(unit), Some(credits))
                if source
                    .description
                    .as_deref()
                    .is_none_or(|description| description.contains(&unit.plural_name)) =>
            {
                CreditPurchaseDetail::Units {
                    credits: rust_decimal::Decimal::new(
                        credits.abs(),
                        u32::from(unit_ref.precision),
                    ),
                    unit: &unit.plural_name,
                    valid_months,
                }
            }
            _ => return stored(),
        },
        None => match (valid_months, name) {
            (Some(months), _) => CreditPurchaseDetail::ValidMonths(months),
            (None, CreditPurchaseName::Purchase) => return stored(),
            (None, _) => CreditPurchaseDetail::CurrencyWallet { currency },
        },
    };
    Some((
        credit_purchase_name(lang, name),
        Some(credit_purchase_description(lang, detail)),
    ))
}

/// How this invoice is to be paid, in both the form the PDF prints and the form
/// the e-invoice states. `bank_details` is free text by design; `means` is the
/// same account structured, because BT-84 is read by a machine.
#[derive(Default)]
struct PaymentInfo {
    bank_details: Option<HashMap<String, String>>,
    means: Option<meteroid_invoicing::model::PaymentMeans>,
    payment_url: Option<String>,
}

impl PaymentInfo {
    fn url(payment_url: String) -> Self {
        PaymentInfo {
            payment_url: Some(payment_url),
            ..PaymentInfo::default()
        }
    }
}

async fn resolve_payment_info(
    store: &Arc<Store>,
    invoice: &Invoice,
    invoicing_entity: &InvoicingEntity,
    public_url: &str,
    jwt_secret: &secrecy::SecretString,
) -> Result<PaymentInfo, Report<InvoicingRenderError>> {
    let customer = store
        .find_customer_by_id(invoice.customer_id, invoice.tenant_id)
        .await
        .change_context(InvoicingRenderError::StoreError)?;

    // A consolidated child is superseded by its parent and must never be presented as payable.
    if invoice.is_consolidated_child() {
        return Ok(PaymentInfo::default());
    }

    // A consolidated parent has no subscription_id of its own; resolve payment via one of its
    // member subscriptions (they all share the same resolved method by construction), so the
    // parent — the document actually charged and emailed — carries the correct payment info.
    let effective_subscription_id = match invoice.subscription_id {
        Some(id) => Some(id),
        None => store
            .list_consolidated_children(invoice.tenant_id, invoice.id)
            .await
            .change_context(InvoicingRenderError::StoreError)?
            .into_iter()
            .find_map(|child| child.subscription_id),
    };

    if let Some(subscription_id) = effective_subscription_id {
        let subscription = store
            .get_subscription(invoice.tenant_id, subscription_id)
            .await
            .change_context(InvoicingRenderError::StoreError)?;

        let resolved = store
            .resolve_payment_method_for_subscription(invoice.tenant_id, subscription_id)
            .await
            .change_context(InvoicingRenderError::StoreError)?;

        let config = subscription
            .payment_methods_config
            .as_ref()
            .unwrap_or(&PaymentMethodsConfig::Online { config: None });

        match (resolved, subscription.charge_automatically) {
            // Has valid payment method AND charge_auto=true -> auto-charge will happen, no link needed
            (ResolvedPaymentMethod::CustomerPaymentMethod(_), true) => Ok(PaymentInfo::default()),

            // Has valid payment method but charge_auto=false -> show payment portal for manual pay
            (ResolvedPaymentMethod::CustomerPaymentMethod(_), false) => {
                let invoice_token = generate_portal_token(
                    jwt_secret,
                    invoice.tenant_id,
                    ResourceAccess::Invoice(invoice.id),
                    None,
                    None,
                    false,
                )
                .change_context(InvoicingRenderError::StoreError)?;
                let payment_url = format!(
                    "{}/portal/invoice-payment?token={}",
                    public_url, invoice_token
                );
                Ok(PaymentInfo::url(payment_url))
            }

            // BankTransfer -> show bank details (never auto-charge)
            (ResolvedPaymentMethod::BankTransfer(_), _) => {
                fetch_bank_details(store, invoicing_entity, invoice).await
            }

            // NotConfigured: depends on the underlying config type
            (ResolvedPaymentMethod::NotConfigured, _) => {
                match config {
                    PaymentMethodsConfig::Online { .. } => {
                        // Online config but no valid method -> show payment link so customer can add card
                        let invoice_token = generate_portal_token(
                            jwt_secret,
                            invoice.tenant_id,
                            ResourceAccess::Invoice(invoice.id),
                            None,
                            None,
                            false,
                        )
                        .change_context(InvoicingRenderError::StoreError)?;
                        let payment_url = format!(
                            "{}/portal/invoice-payment?token={}",
                            public_url, invoice_token
                        );
                        Ok(PaymentInfo::url(payment_url))
                    }
                    PaymentMethodsConfig::BankTransfer { .. } => {
                        // BankTransfer config but no bank account configured -> show nothing
                        // (this is a configuration error, but we don't want to fail invoice rendering)
                        Ok(PaymentInfo::default())
                    }
                    PaymentMethodsConfig::External => {
                        // External -> nothing (payment handled outside the system)
                        Ok(PaymentInfo::default())
                    }
                }
            }
        }
    } else {
        // Non-subscription invoice: use legacy behavior based on customer's payment method
        let has_payment_method = customer.current_payment_method_id.is_some();

        if has_payment_method {
            let invoice_token = generate_portal_token(
                jwt_secret,
                invoice.tenant_id,
                ResourceAccess::Invoice(invoice.id),
                None,
                None,
                false,
            )
            .change_context(InvoicingRenderError::StoreError)?;
            let payment_url = format!(
                "{}/portal/invoice-payment?token={}",
                public_url, invoice_token
            );
            Ok(PaymentInfo::url(payment_url))
        } else {
            fetch_bank_details(store, invoicing_entity, invoice).await
        }
    }
}

async fn fetch_bank_details(
    store: &Arc<Store>,
    invoicing_entity: &InvoicingEntity,
    invoice: &Invoice,
) -> Result<PaymentInfo, Report<InvoicingRenderError>> {
    let Some(bank_account_id) = invoicing_entity.bank_account_id else {
        return Ok(PaymentInfo::default());
    };
    let bank_account = store
        .get_bank_account_by_id(bank_account_id, invoice.tenant_id)
        .await
        .change_context(InvoicingRenderError::StoreError)?;
    let reference = invoice.reference.as_deref();

    Ok(PaymentInfo {
        bank_details: Some(mapper::format_bank_account(&bank_account, reference)),
        means: mapper::payment_means(&bank_account, reference),
        payment_url: None,
    })
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
        // NB: no identity overlay here. This renders exactly the invoice it is handed, because
        // preview_new_invoice_svg synthesizes one from the request — including the operator's
        // edited bill-to, which an overlay would silently replace with the customer record.
        // Callers that load a persisted invoice resolve identity themselves before calling in.
        let organization_logo = match invoicing_entity.logo_attachment_id {
            Some(logo_id) => {
                get_logo_bytes_for_invoice(self.storage.as_ref(), &self.store, logo_id)
                    .await
                    .ok()
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
        let payment = resolve_payment_info(
            &self.store,
            &invoice,
            &invoicing_entity,
            &self.public_url,
            &self.jwt_secret,
        )
        .await?;

        let parent_invoice = match invoice.parent_invoice_id {
            Some(parent_id) => Some(
                self.store
                    .get_invoice_by_id(invoice.tenant_id, parent_id)
                    .await
                    .change_context(InvoicingRenderError::StoreError)?,
            ),
            None => None,
        };

        let line_groups = build_line_groups(&self.store, &invoice).await?;
        let credit_units =
            line_credit_units(&self.store, invoice.tenant_id, &invoice.line_items).await?;

        let mapped = mapper::map_invoice_to_invoicing(
            invoice,
            &invoicing_entity,
            organization_logo,
            rate,
            payment,
            parent_invoice.as_ref(),
            &line_groups,
            &credit_units,
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
        let mut invoice = self
            .store
            .get_invoice_by_id(tenant_id, invoice_id)
            .await
            .change_context(InvoicingRenderError::StoreError)?;

        let invoicing_entity = self
            .store
            .get_invoicing_entity(tenant_id, Some(invoice.seller_details.id))
            .await
            .change_context(InvoicingRenderError::StoreError)?;

        // `get_invoice_by_id` returns the raw row, so this is where a stored draft picks up the
        // identity it would be issued with. No-op once frozen or overridden.
        invoice.overlay_live_seller(&invoicing_entity);
        if invoice.tracks_live_customer() {
            let customer = self
                .store
                .find_customer_by_id(invoice.customer_id, tenant_id)
                .await
                .change_context(InvoicingRenderError::StoreError)?;
            invoice.overlay_live_customer(&customer);
        }

        // Same reasoning as the identity overlay: a draft has no emission date yet, so it
        // renders the dates it would be issued with today — matching what finalization will
        // stamp. Backdated subscriptions (historical imports) and manual invoices keep their
        // stored dates, as they do at finalization.
        self.store
            .project_draft_emission_dates(&mut invoice)
            .await
            .change_context(InvoicingRenderError::StoreError)?;

        self.preview_invoice(invoice, invoicing_entity).await
    }

    /// Dry-runs the e-invoicing gate: the report finalization would record if it
    /// ran now, without producing anything.
    ///
    /// `None` means there is nothing to check — the entity has not opted in, or
    /// the invoice is a consolidated child, which is never issued on its own.
    /// The value of asking about a *draft* is timing: the draft still tracks the
    /// live customer and entity records, so every finding names a field that can
    /// still be fixed on this invoice. After finalization the identity is frozen
    /// and the same finding is only advice for the next one.
    ///
    /// The invoice is assembled the way `generate_pdf_and_save` will see it —
    /// identity overlays, projected emission dates, resolved payment means (the
    /// BG-16 rules hinge on whether means are stated), the corrected-invoice
    /// parent. Only inputs that shape the printed page and never the structured
    /// document are skipped: the logo, the accounting exchange rate, and the
    /// consolidated line grouping. Skipping them keeps this off the object store.
    pub async fn check_einvoicing_readiness(
        &self,
        invoice_id: InvoiceId,
        tenant_id: TenantId,
    ) -> Result<Option<EInvoicingReport>, Report<InvoicingRenderError>> {
        let mut invoice = self
            .store
            .get_invoice_by_id(tenant_id, invoice_id)
            .await
            .change_context(InvoicingRenderError::StoreError)?;

        let invoicing_entity = self
            .store
            .get_invoicing_entity(tenant_id, Some(invoice.seller_details.id))
            .await
            .change_context(InvoicingRenderError::StoreError)?;

        // Mirrors the finalization gate, including the consolidated-child carve-out.
        let format = match invoicing_entity.einvoicing_format {
            _ if invoice.is_consolidated_child() => None,
            format => format.map(map_format),
        };
        let Some(format) = format else {
            return Ok(None);
        };

        // Same identity/date resolution as `preview_invoice_by_id`: the raw row is a
        // draft's stale snapshot, and the check is about the document that would be
        // issued, not the one that was drafted.
        invoice.overlay_live_seller(&invoicing_entity);
        if invoice.tracks_live_customer() {
            let customer = self
                .store
                .find_customer_by_id(invoice.customer_id, tenant_id)
                .await
                .change_context(InvoicingRenderError::StoreError)?;
            invoice.overlay_live_customer(&customer);
        }
        self.store
            .project_draft_emission_dates(&mut invoice)
            .await
            .change_context(InvoicingRenderError::StoreError)?;

        let payment = resolve_payment_info(
            &self.store,
            &invoice,
            &invoicing_entity,
            &self.public_url,
            &self.jwt_secret,
        )
        .await?;

        let parent_invoice = match invoice.parent_invoice_id {
            Some(parent_id) => Some(
                self.store
                    .get_invoice_by_id(invoice.tenant_id, parent_id)
                    .await
                    .change_context(InvoicingRenderError::StoreError)?,
            ),
            None => None,
        };

        let credit_units =
            line_credit_units(&self.store, invoice.tenant_id, &invoice.line_items).await?;
        let mapped = mapper::map_invoice_to_invoicing(
            invoice,
            &invoicing_entity,
            None,
            None,
            payment,
            parent_invoice.as_ref(),
            &HashMap::new(),
            &credit_units,
        )?;

        // The same three outcomes `generate` produces, folded into the stored
        // report shape so the API speaks one vocabulary on both sides of
        // finalization.
        let report = match einvoicing::missing_requirements(&mapped, format) {
            Ok(findings) if findings.is_empty() => {
                EInvoicingReport::generated(format.profile_name())
            }
            Ok(findings) => {
                rejection_outcome(format, &einvoicing::EInvoiceRejection::NotValid(findings)).report
            }
            Err(error) => {
                rejection_outcome(
                    format,
                    &einvoicing::EInvoiceRejection::Unmappable(error.to_string()),
                )
                .report
            }
        };

        Ok(Some(report))
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
                    error: format!("{error:?}"),
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

        // Normally a no-op: PDFs are requested by the InvoiceFinalized event, by which point the
        // identity is frozen. It matters only for the manual request_pdf_generation escape hatch,
        // which can name a draft.
        let mut invoice = invoice;
        invoice.overlay_live_seller(invoicing_entity);
        if invoice.tracks_live_customer() {
            let customer = self
                .store
                .find_customer_by_id(invoice.customer_id, tenant_id)
                .await
                .change_context(InvoicingRenderError::StoreError)?;
            invoice.overlay_live_customer(&customer);
        }

        // let's resolve the logo as raw bytes
        let organization_logo = match invoicing_entity.logo_attachment_id {
            Some(logo_id) => {
                get_logo_bytes_for_invoice(self.storage.as_ref(), &self.store, logo_id)
                    .await
                    .ok()
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
        let payment = resolve_payment_info(
            &self.store,
            &invoice,
            invoicing_entity,
            &self.public_url,
            &self.jwt_secret,
        )
        .await?;

        let customer_id = invoice.customer_id;

        let parent_invoice = match invoice.parent_invoice_id {
            Some(parent_id) => Some(
                self.store
                    .get_invoice_by_id(invoice.tenant_id, parent_id)
                    .await
                    .change_context(InvoicingRenderError::StoreError)?,
            ),
            None => None,
        };

        // For consolidated (merged) invoices, map each line to its source plan so the PDF can
        // group line items under per-plan headings.
        let line_groups = build_line_groups(&self.store, &invoice).await?;
        let credit_units =
            line_credit_units(&self.store, invoice.tenant_id, &invoice.line_items).await?;
        let is_consolidated_child = invoice.is_consolidated_child();

        let mapped_invoice = mapper::map_invoice_to_invoicing(
            invoice,
            invoicing_entity,
            organization_logo,
            rate,
            payment,
            parent_invoice.as_ref(),
            &line_groups,
            &credit_units,
        )?;

        // Structured e-invoicing, when the entity opted in.
        //
        // Never fatal. An invoice that cannot be expressed as a Factur-X
        // document is still a legally issued invoice — numbered, owed, sent —
        // and refusing to produce it over a missing registration number would
        // take billing down over metadata. What a rejection produces instead is
        // a report on the invoice, naming the fields that were missing.
        let einvoicing_format = match invoicing_entity.einvoicing_format {
            // A consolidated child is superseded by its parent and is never
            // finalized on its own, so issuing one as an e-invoice would put a
            // second legal document into circulation for the same charges.
            _ if is_consolidated_child => None,
            format => format.map(map_format),
        };

        let (einvoice, outcome) = match einvoicing_format {
            Some(format) => match einvoicing::generate(&mapped_invoice, format) {
                Ok(document) => {
                    if !document.dropped.is_empty() {
                        tracing::warn!(
                            invoice_id = %invoice_id,
                            dropped = ?document.dropped,
                            "the e-invoice syntax had no place for some terms the invoice carries"
                        );
                    }
                    let outcome = EInvoicingOutcome::generated(format.profile_name());
                    (Some((format, document)), Some(outcome))
                }
                Err(rejection) => {
                    tracing::warn!(
                        invoice_id = %invoice_id,
                        profile = format.profile_name(),
                        reason = %rejection,
                        "the invoice is issued but could not be produced as an e-invoice"
                    );
                    (None, Some(rejection_outcome(format, &rejection)))
                }
            },
            None => (None, None),
        };

        // A hybrid document carries the payload; Peppol travels on its own.
        let attachment = einvoice.as_ref().and_then(|(_, document)| {
            document.filename.map(
                |filename| meteroid_invoicing::typst_render::EInvoiceAttachment {
                    filename: filename.to_string(),
                    xml: document.xml.clone().into_bytes(),
                    description: "Structured invoice data (EN 16931)".to_string(),
                },
            )
        });

        let pdf = self
            .pdf
            .generate_pdf(&mapped_invoice, attachment.as_ref())
            .await
            .change_context(InvoicingRenderError::PdfError)?;

        // The attachment alone does not make the PDF a Factur-X document: the
        // XMP is what announces it to a receiver.
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
            .store(ObjectKind::InvoicePdf, tenant_id, pdf)
            .await
            .change_context(InvoicingRenderError::StorageError)?;
        let pdf_id = stored.id;

        // Stored beside the PDF even when it is also inside it: a transport asks
        // for the payload, not for a PDF to dig it out of.
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
            .save_invoice_documents(
                invoice_id,
                tenant_id,
                customer_id,
                stored,
                stored_xml,
                outcome,
            )
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
    use meteroid_store::domain::credit_units::CreditUnitsById;
    use meteroid_store::domain::historical_rates::HistoricalRate;
    use rust_decimal::Decimal;
    use rust_decimal::prelude::FromPrimitive;
    use std::collections::HashMap;

    #[allow(clippy::too_many_arguments)]
    pub fn map_invoice_to_invoicing(
        invoice: store_model::Invoice,
        invoicing_entity: &store_model::InvoicingEntity,
        organization_logo_bytes: Option<Vec<u8>>,
        accounting_rate: Option<HistoricalRate>,
        payment: super::PaymentInfo,
        parent_invoice: Option<&store_model::Invoice>,
        line_groups: &HashMap<String, String>,
        credit_units: &CreditUnitsById,
    ) -> Result<invoicing_model::Invoice, Report<InvoicingRenderError>> {
        // Issue date is the invoice_date, not finalized_at: it's the date the rest of
        // the system anchors on (numbering, due date, VAT report), so the PDF must match
        // it to avoid printing an issue date that disagrees with the VAT declaration.
        let issue_date = invoice.invoice_date;

        let currency = meteroid_money::iso::find(&invoice.currency).ok_or_else(|| {
            Report::new(InvoicingRenderError::InvalidCurrency(
                invoice.currency.clone(),
            ))
        })?;

        let accounting_currency = *meteroid_money::iso::find(&invoicing_entity.accounting_currency)
            .ok_or_else(|| {
                Report::new(InvoicingRenderError::InvalidCurrency(
                    invoicing_entity.accounting_currency.clone(),
                ))
            })?;

        let mut metadata = invoicing_model::InvoiceMetadata {
            currency,
            due_date: invoice.due_at.map_or(
                issue_date + chrono::Duration::days(i64::from(invoice.net_terms)),
                |d| d.date(),
            ),
            number: invoice.invoice_number,
            issue_date,
            payment_term: invoice.net_terms as u32,
            total_amount: meteroid_money::Money::from_minor(invoice.total, currency),
            tax_amount: meteroid_money::Money::from_minor(invoice.tax_amount, currency),
            subtotal: meteroid_money::Money::from_minor(invoice.subtotal, currency),
            discount: meteroid_money::Money::from_minor(invoice.discount, currency),
            promotional_credit: meteroid_money::Money::from_minor(
                invoice.promotional_credits,
                currency,
            ),
            // Each line's share is in the figures it was quoted in, so their sum is the
            // tax-included amount the document shows.
            quoted_promotional_credit: invoice.tax_inclusive.then(|| {
                meteroid_money::Money::from_minor(
                    invoice
                        .line_items
                        .iter()
                        .filter_map(|line| line.promotional_credit)
                        .sum::<i64>(),
                    currency,
                )
            }),
            memo: invoice.memo.clone(),
            payment_url: payment.payment_url,
            prepaid_amount: meteroid_money::Money::from_minor(invoice.applied_credits, currency),
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
            parent_invoice_number: parent_invoice.map(|pi| pi.invoice_number.clone()),
            parent_invoice_date: parent_invoice.map(|pi| pi.invoice_date),
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
        // locale-specific override. Both sides come from the invoice's own snapshot,
        // so re-rendering an issued document never changes the language it was worded in.
        let lang = Countries::resolve_template_lang(
            invoice.customer_details.preferred_locales.as_slice(),
            invoice.seller_details.invoicing_language.as_deref(),
            &invoicing_entity.country.code,
            format_args!("invoice {}", invoice.id),
        );
        // Footer: the locale-specific override for this document's language, else the
        // base (untagged) footer text.
        let (footer_info, footer_legal) = invoicing_entity.invoice_footer_translations.resolve(
            lang,
            invoicing_entity.invoice_footer_info.as_deref(),
            invoicing_entity.invoice_footer_legal.as_deref(),
        );
        metadata.memo = super::purchase_memo(metadata.memo.take(), &invoice.line_items, lang);

        let organization = invoicing_model::Organization {
            address: map_address(invoice.seller_details.address),
            email: invoice.seller_details.contact_email,
            legal_number: invoice.seller_details.legal_number,
            logo_src: organization_logo_bytes,
            name: invoice.seller_details.legal_name,
            tax_id: invoice.seller_details.vat_number,
            footer_info,
            footer_legal,
            accounting_currency,
            exchange_rate: accounting_rate.and_then(|r| Decimal::from_f64(r.rate)),
            contact_name: invoice.seller_details.contact_name,
            contact_phone: invoice.seller_details.contact_phone,
        };

        let customer = invoicing_model::Customer {
            address: invoice
                .customer_details
                .billing_address
                .map(map_address)
                .unwrap_or_default(),
            email: invoice.customer_details.email,
            legal_number: invoice.customer_details.legal_number,
            buyer_reference: invoice.customer_details.buyer_reference,
            name: invoice.customer_details.name,
            tax_id: invoice.customer_details.vat_number,
        };

        let map_sub_line =
            |sub_line: &meteroid_store::domain::SubLineItem| invoicing_model::InvoiceSubLine {
                name: sub_line.name.clone(),
                total: meteroid_money::Money::from_minor(sub_line.total, currency),
                quantity: sub_line.quantity,
                unit_price: meteroid_money::Money::from_decimal(sub_line.unit_price, currency),
            };

        let prepaid_credit_purchase = invoice
            .line_items
            .iter()
            .any(store_model::LineItem::is_prepaid_credit_purchase);

        // Each line's promotional allowance, the rounding of the net shares settled on the largest
        // so they add up to the invoice's promotional credit.
        let mut promotional_shares = invoice
            .line_items
            .iter()
            .map(|line| line.net_promotional_credit())
            .collect::<Vec<_>>();
        let unsettled = invoice.promotional_credits - promotional_shares.iter().sum::<i64>();
        if let Some(largest) = (0..promotional_shares.len())
            .filter(|index| promotional_shares[*index] != 0)
            .max_by_key(|index| (promotional_shares[*index], std::cmp::Reverse(*index)))
        {
            promotional_shares[largest] += unsettled;
        }
        let lines = invoice
            .line_items
            .iter()
            .zip(promotional_shares)
            .map(|(line, promotional_share)| {
                let credit = line
                    .credit_unit
                    .as_ref()
                    .map(|credit| {
                        let unit = credit_units.get(&credit.unit_id).ok_or_else(|| {
                            Report::new(InvoicingRenderError::StoreError).attach(format!(
                                "credit unit {} of the invoice is missing",
                                credit.unit_id
                            ))
                        })?;
                        Ok::<_, Report<InvoicingRenderError>>(invoicing_model::LineCredit {
                            name: unit.name.clone(),
                            plural_name: unit.plural_name.clone(),
                            consumed: rust_decimal::Decimal::new(
                                credit.consumed,
                                u32::from(credit.precision),
                            ),
                            covered: rust_decimal::Decimal::new(
                                credit.covered,
                                u32::from(credit.precision),
                            ),
                        })
                    })
                    .transpose()?;
                let (name, description) =
                    super::purchase_line_text(line, line, lang, &invoice.currency, credit_units)
                        .unwrap_or_else(|| (line.name.clone(), line.description.clone()));
                Ok::<_, Report<InvoicingRenderError>>(invoicing_model::InvoiceLine {
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
                    sub_lines: line.sub_lines.iter().map(map_sub_line).collect(),
                    group_label: line_groups.get(&line.local_id).cloned(),
                    quoted: line.tax_inclusive_quote.as_ref().map(|quote| {
                        invoicing_model::QuotedLine {
                            subtotal: meteroid_money::Money::from_minor(
                                quote.amount_subtotal,
                                currency,
                            ),
                            unit_price: quote
                                .unit_price
                                .map(|p| meteroid_money::Money::from_decimal(p, currency)),
                            sub_lines: quote.sub_lines.iter().map(map_sub_line).collect(),
                        }
                    }),
                    promotional_credit: Some(promotional_share)
                        .filter(|share| *share != 0)
                        .map(|share| meteroid_money::Money::from_minor(share, currency)),
                    credit,
                    prepaid_credit: line.states_prepaid_credit(),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        let coupons = invoice
            .coupons
            .iter()
            .map(|coupon| invoicing_model::Coupon {
                name: coupon.name.clone(),
                total: meteroid_money::Money::from_minor(coupon.value, currency),
                quoted_total: coupon
                    .quoted_value
                    .map(|value| meteroid_money::Money::from_minor(value, currency)),
            })
            .collect();

        // Annexes are stored as a presentation snapshot (already projected & grouped); the
        // mapping is a 1:1 structural copy into the invoicing model (cells stay typed; the renderer
        // formats money & dates with the invoice's currency and language).
        fn map_annex_cell(
            cell: &meteroid_store::domain::invoices::InvoiceAnnexCell,
        ) -> invoicing_model::AnnexCell {
            use meteroid_store::domain::invoices::InvoiceAnnexCell as C;
            match cell {
                C::Empty => invoicing_model::AnnexCell::Empty,
                C::Text(s) => invoicing_model::AnnexCell::Text(s.clone()),
                C::Number(d) => invoicing_model::AnnexCell::Number(*d),
                C::Money(d) => invoicing_model::AnnexCell::Money(*d),
                C::Date { ts, fmt } => invoicing_model::AnnexCell::Date {
                    ts: *ts,
                    fmt: fmt.clone(),
                },
                C::DateRange { min, max, fmt } => invoicing_model::AnnexCell::DateRange {
                    min: *min,
                    max: *max,
                    fmt: fmt.clone(),
                },
            }
        }

        let annexes = invoice
            .annexes
            .iter()
            .map(|annex| invoicing_model::Annex {
                title: annex.title.clone(),
                description: annex.description.clone(),
                table: invoicing_model::AnnexTable {
                    columns: annex.table.columns.clone(),
                    groups: annex
                        .table
                        .groups
                        .iter()
                        .map(|group| invoicing_model::AnnexGroup {
                            heading: group.heading.clone(),
                            rows: group
                                .rows
                                .iter()
                                .map(|row| invoicing_model::AnnexRow {
                                    cells: row.cells.iter().map(map_annex_cell).collect(),
                                    emphasis: row.emphasis,
                                })
                                .collect(),
                        })
                        .collect(),
                },
            })
            .collect();

        let tax_breakdown = invoice
            .tax_breakdown
            .iter()
            .map(|t| {
                use meteroid_invoicing::model::SupplyBasis as InvoicingBasis;
                use meteroid_invoicing::model::TaxExemptionType as InvoicingExemption;
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
                use meteroid_invoicing::model::TaxKind as InvoicingKind;
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
                    amount: meteroid_money::Money::from_minor(t.tax_amount, currency),
                    taxable_amount: meteroid_money::Money::from_minor(t.taxable_amount, currency),
                    exemption_type,
                    exemption_reason: t.exemption_reason.clone(),
                    supply_basis,
                    prepaid_credit: t.prepaid_credit,
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
            bank_details: payment.bank_details,
            payment_means: payment.means,
            transactions: Vec::new(), // TODO
            payment_status: None,     // TODO
            annexes,
            tax_inclusive: invoice.tax_inclusive,
            prepaid_credit_purchase,
        })
    }

    /// The same account as `format_bank_account`, structured for BT-84.
    ///
    /// `account_numbers` is one free-text field whose meaning is decided by the
    /// format, so the split is positional — the same reading the printed form
    /// already relies on.
    pub fn payment_means(
        bank_account: &store_model::BankAccount,
        payment_reference: Option<&str>,
    ) -> Option<invoicing_model::PaymentMeans> {
        let numbers: Vec<&str> = bank_account.account_numbers.split_whitespace().collect();
        let account_identifier = (*numbers.first()?).to_string();

        let (is_iban, bic) = match bank_account.format {
            store_model::BankAccountFormat::IbanBicSwift => (true, numbers.get(1)),
            store_model::BankAccountFormat::AccountBicSwift => (false, numbers.get(1)),
            store_model::BankAccountFormat::AccountRouting
            | store_model::BankAccountFormat::SortCodeAccount => (false, None),
        };

        Some(invoicing_model::PaymentMeans {
            account_identifier,
            is_iban,
            bic: bic.map(|b| (*b).to_string()),
            account_name: Some(bank_account.bank_name.clone()),
            reference: payment_reference.map(str::to_string),
        })
    }

    pub fn format_bank_account(
        bank_account: &store_model::BankAccount,
        payment_reference: Option<&str>,
    ) -> HashMap<String, String> {
        let mut details = HashMap::new(); // TODO ordermap::OrderMap , also translation

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

        details.insert("Bank Name".to_string(), bank_account.bank_name.clone());

        // Add payment reference if available
        if let Some(reference) = payment_reference {
            details.insert("Payment Reference".to_string(), reference.to_string());
        }

        details
    }
}

async fn get_logo_bytes_for_invoice(
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

#[cfg(test)]
mod tests {
    use super::{purchase_line_text, purchase_memo};
    use common_domain::ids::{BaseId, CreditUnitId, TenantId};
    use meteroid_store::domain::LineItem;
    use meteroid_store::domain::credit_units::{CreditUnit, CreditUnitCode, CreditUnitsById};

    fn line(fields: serde_json::Value) -> LineItem {
        let mut base = serde_json::json!({
            "local_id": "l1",
            "name": "Purchase credits",
            "amount_subtotal": 5000,
            "amount_total": 5000,
            "quantity": "1",
            "unit_price": "50",
            "start_date": "2026-10-17",
            "end_date": "2026-10-17",
            "sub_lines": [],
            "is_prorated": false,
            "kind": "Credit",
            "credit_grant_amount": 5000,
        });
        for (key, value) in fields.as_object().unwrap() {
            base[key] = value.clone();
        }
        serde_json::from_value(base).unwrap()
    }

    #[test]
    fn a_top_up_line_is_worded_in_the_documents_language() {
        let top_up = line(serde_json::json!({
            "name": "Wallet top-up",
            "description": "Prepaid credit added to the EUR wallet",
        }));
        let units = CreditUnitsById::new();
        assert_eq!(
            purchase_line_text(&top_up, &top_up, "fr-FR", "EUR", &units),
            Some((
                "Recharge du portefeuille".to_owned(),
                Some("Crédit prépayé ajouté au portefeuille EUR".to_owned())
            ))
        );
        let automatic = line(serde_json::json!({ "name": "Automatic top-up" }));
        assert_eq!(
            purchase_memo(
                Some("Automatic top-up".to_owned()),
                std::slice::from_ref(&automatic),
                "fr-FR"
            ),
            Some("Recharge automatique".to_owned())
        );
        // English documents keep the stored wording.
        assert_eq!(
            purchase_line_text(&top_up, &top_up, "en-US", "EUR", &units),
            Some((
                "Wallet top-up".to_owned(),
                Some("Prepaid credit added to the EUR wallet".to_owned())
            ))
        );
    }

    #[test]
    fn a_unit_sale_names_its_credits_and_expiry() {
        let unit_id = CreditUnitId::new();
        let sale = line(serde_json::json!({
            "credit_grant_amount": 150000,
            "credit_grant_expiry": { "unit": "MONTH", "value": 12 },
            "credit_grant_unit": { "unit_id": unit_id, "code": "FAST", "precision": 2 },
            "description": "1,500 requêtes rapides, valid for 12 months after payment",
        }));
        let units = CreditUnitsById::from([(
            unit_id,
            CreditUnit {
                id: unit_id,
                tenant_id: TenantId::new(),
                code: CreditUnitCode::new("FAST").unwrap(),
                name: "requête rapide".to_owned(),
                plural_name: "requêtes rapides".to_owned(),
                symbol: None,
                precision: 2,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                archived_at: None,
                product_id: None,
            },
        )]);
        assert_eq!(
            purchase_line_text(&sale, &sale, "fr-FR", "EUR", &units),
            Some((
                "Achat de crédits".to_owned(),
                Some("1\u{a0}500 requêtes rapides, valables 12 mois après paiement".to_owned())
            ))
        );
        // A note reversing 500 of them states the 500 it reverses.
        let mut reversal = sale.clone();
        reversal.credit_grant_amount = Some(-50_000);
        assert_eq!(
            purchase_line_text(&reversal, &sale, "fr-FR", "EUR", &units),
            Some((
                "Achat de crédits".to_owned(),
                Some("500 requêtes rapides, valables 12 mois après paiement".to_owned())
            ))
        );
        // Renamed since the sale: the description it was issued with stands.
        let mut renamed = units.clone();
        renamed.get_mut(&unit_id).unwrap().plural_name = "requêtes turbo".to_owned();
        assert_eq!(
            purchase_line_text(&sale, &sale, "fr-FR", "EUR", &renamed),
            Some((
                "Achat de crédits".to_owned(),
                Some("1,500 requêtes rapides, valid for 12 months after payment".to_owned())
            ))
        );
    }

    #[test]
    fn a_purchase_of_the_retired_balance_is_worded_too() {
        let legacy = line(serde_json::json!({
            "kind": "Regular",
            "credit_grant_amount": null,
            "description": "Prepaid balance",
        }));
        assert_eq!(
            purchase_line_text(&legacy, &legacy, "fr-FR", "EUR", &CreditUnitsById::new()),
            Some((
                "Achat de crédits".to_owned(),
                Some("Prepaid balance".to_owned())
            ))
        );
    }

    #[test]
    fn a_plan_pack_or_a_merchant_line_keeps_its_stored_text() {
        let units = CreditUnitsById::new();
        let pack = line(serde_json::json!({
            "price_component_id": common_domain::ids::PriceComponentId::new(),
        }));
        assert_eq!(
            purchase_line_text(&pack, &pack, "fr-FR", "EUR", &units),
            None
        );
        let named = line(serde_json::json!({ "name": "Prepaid hours" }));
        assert_eq!(
            purchase_line_text(&named, &named, "fr-FR", "EUR", &units),
            None
        );
        assert_eq!(
            purchase_memo(Some("Wallet top-up".to_owned()), &[named], "fr-FR"),
            Some("Wallet top-up".to_owned()),
            "a memo is only reworded on a purchase document"
        );
    }
}
