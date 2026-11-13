//! One render entry for every customer email: the worker, `PreviewEmail`, `SendTestEmail` and
//! `ResendEmail` all build an [`EmailRenderInput`] here and hand it to `meteroid_mailer::render`.

use crate::domain::credit_units::Denomination;
use crate::domain::customer_emails::{AudienceSubject, EmailContext, EmailKind, EmailSettings};
use crate::domain::entity_activity::EntityType;
use crate::domain::enums::TenantEnvironmentEnum;
use crate::domain::enums::{InvoicePaymentStatus, InvoiceStatusEnum};
use crate::domain::payment_reminders::{EmailCopy, EmailCopyByLocale};
use crate::domain::{CreditNote, Customer, Invoice, InvoicingEntity};
use crate::errors::StoreError;
use crate::repositories::InvoiceInterface;
use crate::repositories::TenantInterface;
use crate::repositories::credit_notes::CreditNoteInterface;
use crate::repositories::customer_emails::CustomerEmailsInterface;
use crate::repositories::customers::CustomersInterfaceAuto;
use crate::repositories::entity_activity::SentEmailAttachmentKind;
use crate::repositories::invoicing_entities::InvoicingEntityInterfaceAuto;
use crate::repositories::payment_transactions::PaymentTransactionInterface;
use crate::services::customer_locale::{customer_default_locale, customer_locale_for};
use crate::services::payment_reminders::sweep::reminder_send_guard;
use crate::services::{Services, ServicesEdge};
use crate::{Store, StoreResult};
use chrono::NaiveDate;
use common_domain::ids::{
    BaseId, BudgetId, CustomerId, InvoiceId, PaymentReminderSequenceId, StoredDocumentId,
    SubscriptionId, TenantId,
};
use common_domain::l10n::long_date;
use diesel_models::payment_reminders::{InvoicePaymentReminderRow, PaymentReminderStepRow};
use error_stack::Report;
use meteroid_mailer::copy::{self, Vars};
use meteroid_mailer::l10n::email_l10n;
use meteroid_mailer::render::{
    Body, BrandCtx, Cta, EmailCreditUnit, EntityFooterCtx, FooterCtx, InvoiceSummary, LayoutCtx,
    Subject, WalletActions, format_currency,
};
use meteroid_mailer::service::SenderCtx;
use uuid::Uuid;

pub type PayLink<'a> = &'a (dyn Fn(InvoiceId) -> StoreResult<String> + Send + Sync);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentRef {
    pub document_id: StoredDocumentId,
    pub filename: String,
    pub kind: SentEmailAttachmentKind,
}

/// The activity row a send (or a skip) is recorded against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailTarget {
    pub entity_type: EntityType,
    pub entity_id: Uuid,
    pub agg_customer_id: Option<CustomerId>,
    pub agg_subscription_id: Option<SubscriptionId>,
}

#[derive(Debug, Clone)]
pub struct EmailRenderInput {
    pub entity: InvoicingEntity,
    pub settings: EmailSettings,
    pub lang: &'static str,
    pub layout: LayoutCtx,
    pub recipients: Vec<String>,
    pub subject: Subject,
    pub body: Body,
    pub attachments: Vec<AttachmentRef>,
    pub target: EmailTarget,
}

impl EmailRenderInput {
    /// From (configured sender name, else the legal name), reply-to and bcc of the entity.
    pub fn sender(&self, from_addr: &str) -> StoreResult<SenderCtx> {
        sender_ctx(&self.settings, &self.entity, from_addr)
    }
}

pub fn sender_ctx(
    settings: &EmailSettings,
    entity: &InvoicingEntity,
    from_addr: &str,
) -> StoreResult<SenderCtx> {
    let display = settings
        .sender_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&entity.legal_name);
    let from = SenderCtx::from_mailbox(display, from_addr).map_err(|e| {
        Report::new(StoreError::InvalidArgument(format!(
            "invalid sender address: {}",
            e.current_context()
        )))
    })?;
    Ok(SenderCtx {
        from,
        reply_to: settings.reply_to.clone(),
        bcc: settings.bcc.clone(),
    })
}

/// Recorded as `entity.email_skipped` / `payment_reminder.skipped` `{reason}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display, serde::Serialize)]
#[strum(serialize_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// The workspace's delivery mode is `Off`.
    EmailsOff,
    /// `Test` mode left none of the recipients.
    TestMode,
    KindDisabled,
    NoRecipients,
    NotPayable,
    Stopped,
    Paused,
    OptedOut,
    StepRemoved,
    Superseded,
    PaymentPending,
    DeliveryUnknown,
    PdfMissing,
    SendFailed,
    AudienceExcluded,
}

/// Where a send comes from: an operator's explicit resend is not filtered by "Send to" rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendOrigin {
    Automatic,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Build {
    Send(SendOrigin),
    Preview,
}

impl Build {
    fn applies_audience(self) -> bool {
        self == Self::Send(SendOrigin::Automatic)
    }
}

/// Why a message is put back with a delay instead of being sent or dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeferReason {
    PaymentPending,
    PdfMissing,
}

impl DeferReason {
    pub fn delay_seconds(self) -> i32 {
        match self {
            Self::PaymentPending => 3600,
            Self::PdfMissing => 600,
        }
    }

    pub fn max_attempts(self) -> u32 {
        match self {
            Self::PaymentPending => 48,
            Self::PdfMissing => 3,
        }
    }

    pub fn exhausted_reason(self) -> SkipReason {
        match self {
            Self::PaymentPending => SkipReason::PaymentPending,
            Self::PdfMissing => SkipReason::PdfMissing,
        }
    }
}

pub enum BuildOutcome {
    Render(Box<EmailRenderInput>),
    Skip(SkipReason, EmailTarget),
    Defer(DeferReason, EmailTarget),
}

fn is_payable(invoice: &Invoice) -> bool {
    invoice.status == InvoiceStatusEnum::Finalized
        && invoice.amount_due > 0
        && matches!(
            invoice.payment_status,
            InvoicePaymentStatus::Unpaid
                | InvoicePaymentStatus::PartiallyPaid
                | InvoicePaymentStatus::Errored
        )
}

/// Where a customer's billing emails go: the invoicing emails, else the customer's own email.
fn recipients(customer: &Customer) -> Vec<String> {
    email_recipients(
        &customer.invoicing_emails,
        customer.billing_email.as_deref(),
    )
}

fn email_recipients(invoicing_emails: &[String], billing_email: Option<&str>) -> Vec<String> {
    let mut recipients: Vec<String> = Vec::new();
    for email in invoicing_emails.iter().map(|e| e.trim()) {
        if !email.is_empty() && !recipients.iter().any(|r| r.eq_ignore_ascii_case(email)) {
            recipients.push(email.to_string());
        }
    }
    if recipients.is_empty()
        && let Some(email) = billing_email.map(str::trim).filter(|e| !e.is_empty())
    {
        recipients.push(email.to_string());
    }
    recipients
}

/// A sample notice reads as the entity's portal switches make it read.
fn sample_wallet_actions(entity: &InvoicingEntity) -> WalletActions {
    WalletActions {
        can_top_up: entity.portal_credit_top_up_enabled,
        can_manage_auto_top_up: entity.portal_auto_top_up_enabled,
    }
}

fn sanitize(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

pub fn invoice_pdf_filename(entity: &InvoicingEntity, invoice_number: &str) -> String {
    format!(
        "invoice_{}-{}.pdf",
        sanitize(&entity.legal_name),
        invoice_number
    )
}

fn address_lines(entity: &InvoicingEntity) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for line in [&entity.address_line1, &entity.address_line2] {
        if let Some(l) = line.as_deref().map(str::trim).filter(|l| !l.is_empty()) {
            lines.push(l.to_string());
        }
    }
    let city_line: Vec<&str> = [
        entity.zip_code.as_deref(),
        entity.city.as_deref(),
        entity.state.as_deref(),
    ]
    .into_iter()
    .flatten()
    .map(str::trim)
    .filter(|s| !s.is_empty())
    .collect();
    if !city_line.is_empty() {
        lines.push(city_line.join(" "));
    }
    lines.push(entity.country.code.clone());
    lines
}

/// Step copy and the language the WHOLE reminder renders in: `requested` when the copy has it,
/// else the entity's customer default, else the stored fallback entry (en-US, then the first one).
fn reminder_copy(
    content: &EmailCopyByLocale,
    requested: &'static str,
    default: &'static str,
) -> Option<(&'static str, EmailCopy)> {
    content
        .resolve(requested, default)
        .map(|(locale, copy)| (common_domain::lang::template_lang(locale), copy.clone()))
}

fn override_or(locale_override: Option<&str>, resolved: &'static str) -> &'static str {
    locale_override
        .filter(|l| common_domain::lang::is_supported(l))
        .map(common_domain::lang::template_lang)
        .unwrap_or(resolved)
}

/// Customer mails from a non-production tenant carry the test-mode banner (tenant lookup is cached).
/// Emails of a non-production workspace carry a test notice ("no real payment"), whatever
/// its delivery mode: a production workspace in `Test` still takes real payments.
pub async fn tenant_test_mode(store: &Store, tenant_id: TenantId) -> StoreResult<bool> {
    let tenant = store.find_tenant_by_id(tenant_id).await?;
    Ok(tenant.tenant.environment != TenantEnvironmentEnum::Production)
}

pub fn layout_for_entity(
    entity: &InvoicingEntity,
    lang: &'static str,
    rest_api_url: &str,
    whitelabel: bool,
    test_mode: bool,
) -> LayoutCtx {
    LayoutCtx {
        lang,
        brand: BrandCtx {
            company_name: entity.legal_name.clone(),
            logo_url: entity.logo_attachment_id.map(|id| {
                format!(
                    "{}/files/v1/logo/{}",
                    rest_api_url.trim_end_matches('/'),
                    id.as_base62()
                )
            }),
            brand_color: entity.brand_color.clone(),
            brand_color_light: entity.brand_color_light.clone(),
        },
        footer: FooterCtx::Entity(EntityFooterCtx {
            legal_name: entity.legal_name.clone(),
            address_lines: address_lines(entity),
            vat_number: entity.vat_number.clone(),
            legal_number: entity.legal_number.clone(),
            powered_by: !whitelabel,
        }),
        test_mode,
    }
}

fn invoice_summary(invoice: &Invoice, amount_minor: i64, label: Option<String>) -> InvoiceSummary {
    InvoiceSummary {
        number: invoice.invoice_number.clone(),
        invoice_date: invoice.invoice_date,
        due_date: invoice.due_at.map_or(invoice.invoice_date, |d| d.date()),
        amount_minor,
        currency: invoice.currency.clone(),
        label,
    }
}

/// Earliest line start to latest line end, as the invoice PDF shows them.
fn service_period(invoice: &Invoice) -> Option<(NaiveDate, NaiveDate)> {
    let start = invoice.line_items.iter().map(|l| l.start_date).min()?;
    let end = invoice.line_items.iter().map(|l| l.end_date).max()?;
    (start < end).then_some((start, end))
}

/// Context line under the heading; none for an invoice without a plan.
fn invoice_ready_label(
    lang: &str,
    plan_name: Option<&str>,
    period: Option<(NaiveDate, NaiveDate)>,
) -> Option<String> {
    let l10n = email_l10n(lang);
    let plan = plan_name?;
    Some(match period {
        Some((start, end)) => l10n
            .invoice_ready_plan_period(plan, long_date(lang, &start), long_date(lang, &end))
            .to_string(),
        None => l10n.invoice_ready_plan(plan).to_string(),
    })
}

fn invoice_paid_label(
    lang: &str,
    plan_name: Option<&str>,
    period: Option<(NaiveDate, NaiveDate)>,
) -> Option<String> {
    let l10n = email_l10n(lang);
    let plan = plan_name?;
    Some(match period {
        Some((start, end)) => l10n
            .invoice_paid_plan_period(plan, long_date(lang, &start), long_date(lang, &end))
            .to_string(),
        None => l10n.invoice_paid_plan(plan).to_string(),
    })
}

/// `{{var}}` values formatted for `lang`; `days_overdue` counts local days past the due date.
pub fn reminder_vars(
    lang: &str,
    customer_name: &str,
    company_name: &str,
    number: &str,
    invoice_date: NaiveDate,
    due_date: NaiveDate,
    amount_minor: i64,
    currency: &str,
    today: NaiveDate,
    plan_name: Option<&str>,
) -> Vars {
    let days_overdue = (today - due_date).num_days().max(0);
    Vars::new()
        .with("customer_name", customer_name)
        .with("company_name", company_name)
        .with("invoice_number", number)
        .with("invoice_date", long_date(lang, &invoice_date))
        .with("due_date", long_date(lang, &due_date))
        .with("amount_due", format_currency(amount_minor, currency))
        .with("currency", currency)
        .with("days_overdue", days_overdue.to_string())
        .with("plan_name", plan_name.unwrap_or(""))
}

/// Custom subject + body from operator copy, with the summary box, Pay button and hedge line.
fn custom_body(
    lang: &str,
    copy: &EmailCopy,
    vars: &Vars,
    summary: Option<InvoiceSummary>,
    cta: Option<Cta>,
) -> (Subject, Body) {
    let l10n = email_l10n(lang);
    (
        Subject::Custom(copy::substitute(&copy.subject, vars)),
        Body::Custom {
            html_fragment: copy::to_html(&copy.body, vars),
            summary,
            cta,
            hedge: Some(l10n.already_paid_disregard().to_string()),
        },
    )
}

/// Which of the seeded default reminder texts a preview without draft shows for `offset_days`.
fn default_reminder_copy(lang: &str, offset_days: i32) -> EmailCopy {
    let l10n = email_l10n(lang);
    let (subject, body) = match offset_days {
        d if d < 0 => (
            l10n.reminder_default_before_subject(),
            l10n.reminder_default_before_body(),
        ),
        0 => (
            l10n.reminder_default_due_subject(),
            l10n.reminder_default_due_body(),
        ),
        d if d < 14 => (
            l10n.reminder_default_overdue_subject(),
            l10n.reminder_default_overdue_body(),
        ),
        _ => (
            l10n.reminder_default_final_subject(),
            l10n.reminder_default_final_body(),
        ),
    };
    EmailCopy {
        subject: subject.to_string(),
        body: body.to_string(),
    }
}

struct InvoiceEmail {
    invoice: Invoice,
    customer: Customer,
    entity: InvoicingEntity,
    settings: EmailSettings,
}

impl Services {
    fn rest_api_url(&self) -> &str {
        &self.store.settings.rest_api_external_url
    }

    async fn load_invoice_email(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<InvoiceEmail> {
        let invoice = self.store.get_invoice_by_id(tenant_id, invoice_id).await?;
        let customer = self
            .store
            .find_customer_by_id(invoice.customer_id, tenant_id)
            .await?;
        let entity = self
            .store
            .get_invoicing_entity(tenant_id, Some(invoice.invoicing_entity_id))
            .await?;
        let settings = self.store.get_email_settings(tenant_id, entity.id).await?;
        Ok(InvoiceEmail {
            invoice,
            customer,
            entity,
            settings,
        })
    }

    /// The activity target of a context without rendering anything.
    pub async fn email_target(
        &self,
        tenant_id: TenantId,
        context: &EmailContext,
    ) -> StoreResult<EmailTarget> {
        match context {
            EmailContext::CreditNote { credit_note_id } => {
                let cn = self
                    .store
                    .get_credit_note_by_id(tenant_id, *credit_note_id)
                    .await?;
                Ok(credit_note_target(&cn))
            }
            EmailContext::WalletBalanceLow { customer_id, .. }
            | EmailContext::AutoTopUpFailed { customer_id, .. }
            | EmailContext::BudgetAlert { customer_id, .. } => Ok(customer_target(*customer_id)),
            other => {
                let invoice_id = other.invoice_id().ok_or_else(|| {
                    Report::new(StoreError::InvalidArgument(
                        "email context has no invoice".to_string(),
                    ))
                })?;
                let invoice = self.store.get_invoice_by_id(tenant_id, invoice_id).await?;
                Ok(invoice_target(&invoice))
            }
        }
    }

    /// Loads live state, resolves the language ([`customer_locale_for`]), builds the layout from the
    /// entity and picks the body for `kind`, applying every send-time gate. Never sends.
    #[allow(clippy::too_many_arguments)]
    pub async fn build_customer_email(
        &self,
        tenant_id: TenantId,
        kind: EmailKind,
        context: &EmailContext,
        locale_override: Option<&str>,
        draft: Option<&EmailCopy>,
        pay_link: PayLink<'_>,
        whitelabel: bool,
        origin: SendOrigin,
    ) -> StoreResult<BuildOutcome> {
        self.build_customer_email_for(
            tenant_id,
            kind,
            context,
            locale_override,
            draft,
            pay_link,
            whitelabel,
            Build::Send(origin),
        )
        .await
    }

    /// The same render without the send-time gates (kind toggle, recipients, reminder state,
    /// pending payment, missing PDF): what an operator sees in a preview.
    pub async fn preview_customer_email(
        &self,
        tenant_id: TenantId,
        kind: EmailKind,
        context: &EmailContext,
        locale_override: Option<&str>,
        draft: Option<&EmailCopy>,
        pay_link: PayLink<'_>,
        whitelabel: bool,
    ) -> StoreResult<BuildOutcome> {
        self.build_customer_email_for(
            tenant_id,
            kind,
            context,
            locale_override,
            draft,
            pay_link,
            whitelabel,
            Build::Preview,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn build_customer_email_for(
        &self,
        tenant_id: TenantId,
        kind: EmailKind,
        context: &EmailContext,
        locale_override: Option<&str>,
        draft: Option<&EmailCopy>,
        pay_link: PayLink<'_>,
        whitelabel: bool,
        build: Build,
    ) -> StoreResult<BuildOutcome> {
        let preview = build == Build::Preview;
        if let EmailContext::CreditNote { credit_note_id } = context {
            return self
                .build_credit_note_email(
                    tenant_id,
                    *credit_note_id,
                    locale_override,
                    whitelabel,
                    build,
                )
                .await;
        }
        if let Some(customer_id) = context.notice_customer_id() {
            return self
                .build_wallet_email(
                    tenant_id,
                    kind,
                    customer_id,
                    context,
                    locale_override,
                    whitelabel,
                    build,
                )
                .await;
        }
        let invoice_id = context.invoice_id().ok_or_else(|| {
            Report::new(StoreError::InvalidArgument(
                "email context has no invoice".to_string(),
            ))
        })?;
        let loaded = self.load_invoice_email(tenant_id, invoice_id).await?;
        let InvoiceEmail {
            invoice,
            customer,
            entity,
            settings,
        } = &loaded;
        let target = invoice_target(invoice);

        if let (
            false,
            EmailContext::PaymentReminder {
                sequence_id,
                generation,
                manual,
                ..
            },
        ) = (preview, context)
        {
            if let Some(reason) = reminder_send_guard(
                &self.store,
                tenant_id,
                invoice,
                customer,
                *sequence_id,
                *generation,
                *manual,
            )
            .await?
            {
                return Ok(BuildOutcome::Skip(reason, target));
            }
            if invoice.payment_status == InvoicePaymentStatus::Processing
                || self
                    .store
                    .invoice_has_live_payment(tenant_id, invoice.id)
                    .await?
            {
                return Ok(BuildOutcome::Defer(DeferReason::PaymentPending, target));
            }
        }
        let recipients = recipients(customer);
        if !preview {
            // An invoice voided, paid or in payment since it was queued is not asked for.
            if matches!(
                kind,
                EmailKind::InvoiceReady
                    | EmailKind::PaymentReminder
                    | EmailKind::PaymentFailed
                    | EmailKind::AuthenticationRequired
            ) && !is_payable(invoice)
            {
                return Ok(BuildOutcome::Skip(SkipReason::NotPayable, target));
            }
            if !settings.is_enabled(kind) {
                return Ok(BuildOutcome::Skip(SkipReason::KindDisabled, target));
            }
            if build.applies_audience()
                && self
                    .store
                    .email_audience_suppresses(
                        tenant_id,
                        entity.id,
                        kind,
                        AudienceSubject::Invoice(invoice.id),
                    )
                    .await?
            {
                return Ok(BuildOutcome::Skip(SkipReason::AudienceExcluded, target));
            }
            if recipients.is_empty() {
                return Ok(BuildOutcome::Skip(SkipReason::NoRecipients, target));
            }
        }

        // Reminders may still fall back to the entity default when the step copy lacks `lang`.
        let lang = override_or(
            locale_override,
            customer_locale_for(entity, &customer.preferred_locales),
        );
        let pdf = |kind: SentEmailAttachmentKind| -> Option<AttachmentRef> {
            invoice.pdf_document_id.map(|document_id| AttachmentRef {
                document_id,
                filename: invoice_pdf_filename(entity, &invoice.invoice_number),
                kind,
            })
        };
        // A kind that always attaches the PDF waits for it; a preview simply renders without.
        let required_pdf =
            |kind: SentEmailAttachmentKind| -> Result<Vec<AttachmentRef>, BuildOutcome> {
                match pdf(kind) {
                    Some(pdf) => Ok(vec![pdf]),
                    None if preview => Ok(vec![]),
                    None => Err(BuildOutcome::Defer(DeferReason::PdfMissing, target.clone())),
                }
            };

        let (lang, subject, body, attachments) = match (kind, context) {
            (EmailKind::InvoiceReady, EmailContext::Invoice { .. }) => {
                let attachments = match required_pdf(SentEmailAttachmentKind::InvoicePdf) {
                    Ok(a) => a,
                    Err(outcome) => return Ok(outcome),
                };
                (
                    lang,
                    Subject::Fixed,
                    Body::InvoiceReady {
                        summary: invoice_summary(
                            invoice,
                            invoice.amount_due,
                            invoice_ready_label(
                                lang,
                                invoice.plan_name.as_deref(),
                                service_period(invoice),
                            ),
                        ),
                        cta: Cta::new(pay_link(invoice.id)?),
                    },
                    attachments,
                )
            }
            (EmailKind::InvoicePaid, EmailContext::Invoice { .. }) => {
                let mut attachments = match required_pdf(SentEmailAttachmentKind::InvoicePdf) {
                    Ok(a) => a,
                    Err(outcome) => return Ok(outcome),
                };
                let receipt = self
                    .store
                    .last_settled_payment_tx_by_invoice_id(tenant_id, invoice.id)
                    .await?;
                let amount_paid = receipt.as_ref().map_or(invoice.total, |r| r.amount);
                if let Some(receipt_pdf_id) = receipt.and_then(|r| r.receipt_pdf_id) {
                    attachments.push(AttachmentRef {
                        document_id: receipt_pdf_id,
                        filename: format!(
                            "receipt_{}-{}.pdf",
                            sanitize(&entity.legal_name),
                            receipt_pdf_id.as_base62()
                        ),
                        kind: SentEmailAttachmentKind::ReceiptPdf,
                    });
                }
                (
                    lang,
                    Subject::Fixed,
                    Body::InvoicePaid {
                        summary: invoice_summary(
                            invoice,
                            amount_paid,
                            invoice_paid_label(
                                lang,
                                invoice.plan_name.as_deref(),
                                service_period(invoice),
                            ),
                        ),
                    },
                    attachments,
                )
            }
            // The transaction's `error_type` is provider free text (English, internal
            // detail): never shown to customers, and no normalized decline kind is stored.
            (EmailKind::PaymentFailed, EmailContext::PaymentFailed { .. }) => (
                lang,
                Subject::Fixed,
                Body::PaymentFailed {
                    summary: invoice_summary(invoice, invoice.amount_due, None),
                    cta: Cta::new(pay_link(invoice.id)?),
                },
                pdf(SentEmailAttachmentKind::InvoicePdf)
                    .into_iter()
                    .collect(),
            ),
            (EmailKind::AuthenticationRequired, EmailContext::AuthenticationRequired { .. }) => (
                lang,
                Subject::Fixed,
                Body::AuthenticationRequired {
                    summary: invoice_summary(invoice, invoice.amount_due, None),
                    cta: Cta::new(pay_link(invoice.id)?),
                },
                vec![],
            ),
            (
                EmailKind::PaymentReminder,
                EmailContext::PaymentReminder {
                    sequence_id,
                    offset_days,
                    ..
                },
            ) => {
                let attachments = match required_pdf(SentEmailAttachmentKind::InvoicePdf) {
                    Ok(a) => a,
                    Err(outcome) => return Ok(outcome),
                };
                let stored;
                let (lang, copy) = match draft {
                    Some(draft) => (lang, draft),
                    None => {
                        match self
                            .step_copy(tenant_id, *sequence_id, *offset_days, lang, entity)
                            .await?
                        {
                            Some((copy_lang, c)) => {
                                stored = c;
                                (copy_lang, &stored)
                            }
                            None => {
                                return Ok(BuildOutcome::Skip(SkipReason::StepRemoved, target));
                            }
                        }
                    }
                };
                let today = common_domain::clock::now()
                    .with_timezone(&settings.send_window().tz)
                    .date_naive();
                let summary = invoice_summary(invoice, invoice.amount_due, None);
                let vars = reminder_vars(
                    lang,
                    &customer.name,
                    &entity.legal_name,
                    &summary.number,
                    summary.invoice_date,
                    summary.due_date,
                    summary.amount_minor,
                    &summary.currency,
                    today,
                    invoice.plan_name.as_deref(),
                );
                let (subject, body) = custom_body(
                    lang,
                    copy,
                    &vars,
                    Some(summary),
                    Some(Cta::new(pay_link(invoice.id)?)),
                );
                (lang, subject, body, attachments)
            }
            (kind, context) => {
                return Err(Report::new(StoreError::InvalidArgument(format!(
                    "{kind} cannot be rendered from a {} context",
                    context_name(context)
                ))));
            }
        };
        let test_mode = tenant_test_mode(&self.store, tenant_id).await?;
        let layout = layout_for_entity(entity, lang, self.rest_api_url(), whitelabel, test_mode);

        Ok(BuildOutcome::Render(Box::new(EmailRenderInput {
            entity: entity.clone(),
            settings: settings.clone(),
            lang,
            layout,
            recipients,
            subject,
            body,
            attachments,
            target,
        })))
    }

    async fn build_credit_note_email(
        &self,
        tenant_id: TenantId,
        credit_note_id: common_domain::ids::CreditNoteId,
        locale_override: Option<&str>,
        whitelabel: bool,
        build: Build,
    ) -> StoreResult<BuildOutcome> {
        let cn = self
            .store
            .get_credit_note_by_id(tenant_id, credit_note_id)
            .await?;
        let target = credit_note_target(&cn);
        let entity = self
            .store
            .get_invoicing_entity(tenant_id, Some(cn.invoicing_entity_id))
            .await?;
        let settings = self.store.get_email_settings(tenant_id, entity.id).await?;
        if !settings.is_enabled(EmailKind::CreditNoteIssued) {
            return Ok(BuildOutcome::Skip(SkipReason::KindDisabled, target));
        }
        if build.applies_audience()
            && self
                .store
                .email_audience_suppresses(
                    tenant_id,
                    entity.id,
                    EmailKind::CreditNoteIssued,
                    AudienceSubject::Invoice(cn.invoice_id),
                )
                .await?
        {
            return Ok(BuildOutcome::Skip(SkipReason::AudienceExcluded, target));
        }
        let customer = self
            .store
            .find_customer_by_id(cn.customer_id, tenant_id)
            .await?;
        let recipients = recipients(&customer);
        if recipients.is_empty() {
            return Ok(BuildOutcome::Skip(SkipReason::NoRecipients, target));
        }
        let Some(pdf_id) = cn.pdf_document_id else {
            return Ok(BuildOutcome::Defer(DeferReason::PdfMissing, target));
        };
        let lang = override_or(
            locale_override,
            customer_locale_for(&entity, &customer.preferred_locales),
        );
        let test_mode = tenant_test_mode(&self.store, tenant_id).await?;
        let layout = layout_for_entity(&entity, lang, self.rest_api_url(), whitelabel, test_mode);
        Ok(BuildOutcome::Render(Box::new(EmailRenderInput {
            layout,
            lang,
            recipients,
            subject: Subject::Fixed,
            body: Body::CreditNoteIssued {
                credit_note_number: cn.credit_note_number.clone(),
                invoice_number: Some(cn.invoice_number.clone()),
                amount_minor: cn.total,
                currency: cn.currency.clone(),
                cta: None,
            },
            attachments: vec![AttachmentRef {
                document_id: pdf_id,
                filename: format!(
                    "credit_note_{}-{}.pdf",
                    sanitize(&entity.legal_name),
                    cn.credit_note_number
                ),
                kind: SentEmailAttachmentKind::CreditNotePdf,
            }],
            target,
            entity,
            settings,
        })))
    }

    /// Whether the budget refuses usage once reached, as it is set when the email is built.
    async fn budget_stops_usage(
        &self,
        tenant_id: TenantId,
        budget_id: BudgetId,
    ) -> StoreResult<bool> {
        let mut conn = self.store.get_conn().await?;
        self.store
            .hooks
            .credit
            .budget_refuses_usage(&mut conn, tenant_id, budget_id)
            .await
    }

    /// The credit unit named by `code`, as the customer reads it; None for a currency.
    async fn email_credit_unit(
        &self,
        tenant_id: TenantId,
        code: &str,
    ) -> StoreResult<Option<EmailCreditUnit>> {
        if code.trim().len() == 3 {
            return Ok(None);
        }
        let mut conn = self.store.get_conn().await?;
        let denomination = crate::repositories::credit::denomination::resolve_denomination(
            &mut conn, tenant_id, code,
        )
        .await?;
        Ok(match denomination {
            Denomination::Unit(unit) => Some(EmailCreditUnit {
                name: unit.name,
                plural_name: unit.plural_name,
                precision: unit.precision,
            }),
            Denomination::Currency { .. } => None,
        })
    }

    /// What the portal lets the customer do about a prepaid-balance notice: the copy asks for
    /// nothing else. No link: an emailed portal session would sign in whoever reads the email.
    async fn wallet_actions(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> StoreResult<WalletActions> {
        let capabilities = self
            .portal_credit_capabilities(tenant_id, customer_id)
            .await?;
        Ok(WalletActions {
            can_top_up: capabilities.top_up_enabled,
            can_manage_auto_top_up: capabilities.auto_top_up_enabled,
        })
    }

    /// A wallet or budget notice: the figures travel in the context, as the event reported them.
    #[allow(clippy::too_many_arguments)]
    async fn build_wallet_email(
        &self,
        tenant_id: TenantId,
        kind: EmailKind,
        customer_id: CustomerId,
        context: &EmailContext,
        locale_override: Option<&str>,
        whitelabel: bool,
        build: Build,
    ) -> StoreResult<BuildOutcome> {
        let target = customer_target(customer_id);
        let body = match (kind, context) {
            (
                EmailKind::WalletBalanceLow,
                EmailContext::WalletBalanceLow {
                    currency,
                    available_minor,
                    threshold_minor,
                    ..
                },
            ) => Body::WalletBalanceLow {
                available_minor: *available_minor,
                threshold_minor: *threshold_minor,
                currency: currency.clone(),
                credit_unit: self.email_credit_unit(tenant_id, currency).await?,
                actions: self.wallet_actions(tenant_id, customer_id).await?,
            },
            (
                EmailKind::AutoTopUpFailed,
                EmailContext::AutoTopUpFailed {
                    currency,
                    amount_minor,
                    suspended,
                    payment_reversed,
                    ..
                },
            ) => Body::AutoTopUpFailed {
                amount_minor: *amount_minor,
                currency: currency.clone(),
                credit_unit: self.email_credit_unit(tenant_id, currency).await?,
                suspended: *suspended,
                payment_reversed: *payment_reversed,
                actions: self.wallet_actions(tenant_id, customer_id).await?,
            },
            (
                EmailKind::BudgetThreshold | EmailKind::BudgetExceeded,
                EmailContext::BudgetAlert {
                    budget_id,
                    currency,
                    level_pct,
                    used_minor,
                    amount_minor,
                    window_start,
                    window_end,
                    ..
                },
            ) => Body::BudgetAlert {
                level_pct: *level_pct,
                used_minor: *used_minor,
                amount_minor: *amount_minor,
                currency: currency.clone(),
                credit_unit: self.email_credit_unit(tenant_id, currency).await?,
                window_start: window_start.date(),
                window_end: window_end.map(|end| end.date()),
                stops_usage: self.budget_stops_usage(tenant_id, *budget_id).await?,
            },
            (kind, context) => {
                return Err(Report::new(StoreError::InvalidArgument(format!(
                    "{kind} cannot be rendered from a {} context",
                    context_name(context)
                ))));
            }
        };
        let customer = self
            .store
            .find_customer_by_id(customer_id, tenant_id)
            .await?;
        let entity = self
            .store
            .get_invoicing_entity(tenant_id, Some(customer.invoicing_entity_id))
            .await?;
        let settings = self.store.get_email_settings(tenant_id, entity.id).await?;
        let recipients = recipients(&customer);
        if build != Build::Preview {
            if !settings.is_enabled(kind) {
                return Ok(BuildOutcome::Skip(SkipReason::KindDisabled, target));
            }
            if build.applies_audience()
                && self
                    .store
                    .email_audience_suppresses(
                        tenant_id,
                        entity.id,
                        kind,
                        AudienceSubject::Customer(customer_id),
                    )
                    .await?
            {
                return Ok(BuildOutcome::Skip(SkipReason::AudienceExcluded, target));
            }
            if recipients.is_empty() {
                return Ok(BuildOutcome::Skip(SkipReason::NoRecipients, target));
            }
        }
        let lang = override_or(
            locale_override,
            customer_locale_for(&entity, &customer.preferred_locales),
        );
        let test_mode = tenant_test_mode(&self.store, tenant_id).await?;
        let layout = layout_for_entity(&entity, lang, self.rest_api_url(), whitelabel, test_mode);
        Ok(BuildOutcome::Render(Box::new(EmailRenderInput {
            layout,
            lang,
            recipients,
            subject: Subject::Fixed,
            body,
            attachments: vec![],
            target,
            entity,
            settings,
        })))
    }

    /// The step's copy and the language the whole email uses (see [`reminder_copy`]);
    /// `None` when the step is gone.
    async fn step_copy(
        &self,
        tenant_id: TenantId,
        sequence_id: PaymentReminderSequenceId,
        offset_days: i32,
        lang: &'static str,
        entity: &InvoicingEntity,
    ) -> StoreResult<Option<(&'static str, EmailCopy)>> {
        let mut conn = self.store.get_conn().await?;
        let steps =
            PaymentReminderStepRow::list_by_sequence_ids(&mut conn, tenant_id, &[sequence_id])
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
        let Some(step) = steps.into_iter().find(|s| s.offset_days == offset_days) else {
            return Ok(None);
        };
        let content: EmailCopyByLocale = serde_json::from_value(step.content).map_err(|e| {
            StoreError::SerdeError("Failed to deserialize reminder step content".into(), e)
        })?;
        if content.0.is_empty() {
            return Ok(Some((lang, default_reminder_copy(lang, offset_days))));
        }
        Ok(reminder_copy(
            &content,
            lang,
            customer_default_locale(entity),
        ))
    }

    /// Synthetic invoice ("INV-2026-0042", due today + net_terms, 1 234,50 in the accounting
    /// currency, plan "Starter"); reminders take `draft` or the seeded default copy.
    pub fn build_preview(
        &self,
        entity: &InvoicingEntity,
        settings: &EmailSettings,
        kind: EmailKind,
        lang: &'static str,
        draft: Option<&EmailCopy>,
        offset_days: Option<i32>,
        whitelabel: bool,
        test_mode: bool,
    ) -> StoreResult<EmailRenderInput> {
        let today = common_domain::clock::now().date_naive();
        let due = today + chrono::Days::new(entity.net_terms.max(0) as u64);
        let number = "INV-2026-0042".to_string();
        let currency = entity.accounting_currency.clone();
        let summary = InvoiceSummary {
            number: number.clone(),
            invoice_date: today,
            due_date: due,
            amount_minor: 123_450,
            currency: currency.clone(),
            label: invoice_ready_label(
                lang,
                Some("Starter"),
                Some((today, today + chrono::Months::new(1))),
            ),
        };
        let pay = Cta::new(format!(
            "{}/portal/invoice-payment?token=preview",
            self.store.settings.public_url.trim_end_matches('/')
        ));
        let pdf = |kind: SentEmailAttachmentKind, filename: String| AttachmentRef {
            document_id: StoredDocumentId::new(),
            filename,
            kind,
        };
        let invoice_pdf = pdf(
            SentEmailAttachmentKind::InvoicePdf,
            invoice_pdf_filename(entity, &number),
        );
        let (subject, body, attachments) = match kind {
            EmailKind::InvoiceReady => (
                Subject::Fixed,
                Body::InvoiceReady { summary, cta: pay },
                vec![invoice_pdf],
            ),
            EmailKind::InvoicePaid => (
                Subject::Fixed,
                Body::InvoicePaid {
                    summary: InvoiceSummary {
                        label: invoice_paid_label(
                            lang,
                            Some("Starter"),
                            Some((today, today + chrono::Months::new(1))),
                        ),
                        ..summary
                    },
                },
                vec![
                    invoice_pdf,
                    pdf(
                        SentEmailAttachmentKind::ReceiptPdf,
                        format!("receipt_{}-{}.pdf", sanitize(&entity.legal_name), number),
                    ),
                ],
            ),
            EmailKind::QuoteReady => (
                Subject::Fixed,
                Body::QuoteReady {
                    quote_number: "Q-2026-0007".to_string(),
                    expires_at: Some(today + chrono::Days::new(30)),
                    custom_message: None,
                    cta: pay,
                },
                vec![],
            ),
            EmailKind::CreditNoteIssued => (
                Subject::Fixed,
                Body::CreditNoteIssued {
                    credit_note_number: "CN-2026-0003".to_string(),
                    invoice_number: Some(number.clone()),
                    amount_minor: 45_000,
                    currency,
                    cta: None,
                },
                vec![pdf(
                    SentEmailAttachmentKind::CreditNotePdf,
                    format!(
                        "credit_note_{}-CN-2026-0003.pdf",
                        sanitize(&entity.legal_name)
                    ),
                )],
            ),
            EmailKind::PaymentFailed => (
                Subject::Fixed,
                Body::PaymentFailed {
                    summary: InvoiceSummary {
                        label: None,
                        ..summary
                    },
                    cta: pay,
                },
                vec![invoice_pdf],
            ),
            EmailKind::AuthenticationRequired => (
                Subject::Fixed,
                Body::AuthenticationRequired {
                    summary: InvoiceSummary {
                        label: None,
                        ..summary
                    },
                    cta: pay,
                },
                vec![],
            ),
            EmailKind::WalletBalanceLow => (
                Subject::Fixed,
                Body::WalletBalanceLow {
                    available_minor: 4_250,
                    threshold_minor: 10_000,
                    currency,
                    credit_unit: None,
                    actions: sample_wallet_actions(entity),
                },
                vec![],
            ),
            EmailKind::BudgetThreshold | EmailKind::BudgetExceeded => (
                Subject::Fixed,
                Body::BudgetAlert {
                    level_pct: if kind == EmailKind::BudgetExceeded {
                        100
                    } else {
                        80
                    },
                    used_minor: if kind == EmailKind::BudgetExceeded {
                        100_000
                    } else {
                        80_000
                    },
                    amount_minor: 100_000,
                    currency,
                    credit_unit: None,
                    window_start: today,
                    window_end: Some(today + chrono::Months::new(1)),
                    stops_usage: false,
                },
                vec![],
            ),
            EmailKind::AutoTopUpFailed => (
                Subject::Fixed,
                Body::AutoTopUpFailed {
                    amount_minor: 50_000,
                    currency,
                    credit_unit: None,
                    suspended: false,
                    payment_reversed: false,
                    actions: sample_wallet_actions(entity),
                },
                vec![],
            ),
            EmailKind::PaymentReminder => {
                let offset = offset_days.unwrap_or(7);
                let default;
                let copy = match draft {
                    Some(d) => d,
                    None => {
                        default = default_reminder_copy(lang, offset);
                        &default
                    }
                };
                let due = today - chrono::Days::new(offset.max(0) as u64);
                let summary = InvoiceSummary {
                    due_date: due,
                    invoice_date: due - chrono::Days::new(entity.net_terms.max(0) as u64),
                    label: None,
                    ..summary
                };
                let vars = reminder_vars(
                    lang,
                    "Acme Corp",
                    &entity.legal_name,
                    &summary.number,
                    summary.invoice_date,
                    summary.due_date,
                    summary.amount_minor,
                    &summary.currency,
                    today,
                    Some("Starter"),
                );
                let (subject, body) = custom_body(lang, copy, &vars, Some(summary), Some(pay));
                (subject, body, vec![invoice_pdf])
            }
        };
        Ok(EmailRenderInput {
            entity: entity.clone(),
            settings: settings.clone(),
            lang,
            layout: layout_for_entity(entity, lang, self.rest_api_url(), whitelabel, test_mode),
            recipients: vec![],
            subject,
            body,
            attachments,
            target: EmailTarget {
                entity_type: EntityType::InvoicingEntity,
                entity_id: entity.id.as_uuid(),
                agg_customer_id: None,
                agg_subscription_id: None,
            },
        })
    }

    /// Frozen-sequence step to preview for an enrolled invoice: `offset_days` when given, else
    /// the next one after the cursor, else the last one.
    pub async fn preview_reminder_context(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        offset_days: Option<i32>,
    ) -> StoreResult<EmailContext> {
        let mut conn = self.store.get_conn().await?;
        let row = InvoicePaymentReminderRow::find_by_invoice_id(&mut conn, tenant_id, invoice_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let (sequence_id, generation, cursor) = match row {
            Some(InvoicePaymentReminderRow {
                sequence_id: Some(sequence_id),
                generation,
                last_offset_days,
                ..
            }) => (sequence_id, generation, last_offset_days),
            _ => {
                return Err(Report::new(StoreError::FailedPrecondition(
                    "The invoice is not enrolled in a reminder sequence".to_string(),
                )));
            }
        };
        let mut offsets: Vec<i32> =
            PaymentReminderStepRow::list_by_sequence_ids(&mut conn, tenant_id, &[sequence_id])
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .map(|s| s.offset_days)
                .collect();
        offsets.sort_unstable();
        let offset_days = match offset_days {
            Some(o) if offsets.contains(&o) => o,
            Some(o) => {
                return Err(Report::new(StoreError::InvalidArgument(format!(
                    "offset {o} is not a step of the invoice's reminder sequence"
                ))));
            }
            None => offsets
                .iter()
                .find(|o| Some(**o) > cursor)
                .or_else(|| offsets.last())
                .copied()
                .ok_or_else(|| {
                    Report::new(StoreError::FailedPrecondition(
                        "The reminder sequence has no steps".to_string(),
                    ))
                })?,
        };
        Ok(EmailContext::PaymentReminder {
            invoice_id,
            sequence_id,
            offset_days,
            generation,
            manual: true,
        })
    }
}

fn context_name(context: &EmailContext) -> &'static str {
    match context {
        EmailContext::Invoice { .. } => "invoice",
        EmailContext::CreditNote { .. } => "credit_note",
        EmailContext::PaymentFailed { .. } => "payment_failed",
        EmailContext::AuthenticationRequired { .. } => "authentication_required",
        EmailContext::PaymentReminder { .. } => "payment_reminder",
        EmailContext::WalletBalanceLow { .. } => "wallet_balance_low",
        EmailContext::AutoTopUpFailed { .. } => "auto_top_up_failed",
        EmailContext::BudgetAlert { .. } => "budget_alert",
    }
}

fn invoice_target(invoice: &Invoice) -> EmailTarget {
    EmailTarget {
        entity_type: EntityType::Invoice,
        entity_id: invoice.id.as_uuid(),
        agg_customer_id: Some(invoice.customer_id),
        agg_subscription_id: invoice.subscription_id,
    }
}

/// No rollup id: the activity is on the customer itself (`entity_activity_agg_customer_not_self`).
fn customer_target(customer_id: CustomerId) -> EmailTarget {
    EmailTarget {
        entity_type: EntityType::Customer,
        entity_id: customer_id.as_uuid(),
        agg_customer_id: None,
        agg_subscription_id: None,
    }
}

fn credit_note_target(cn: &CreditNote) -> EmailTarget {
    EmailTarget {
        entity_type: EntityType::CreditNote,
        entity_id: cn.id.as_uuid(),
        agg_customer_id: Some(cn.customer_id),
        agg_subscription_id: cn.subscription_id,
    }
}

impl ServicesEdge {
    #[allow(clippy::too_many_arguments)]
    pub async fn build_customer_email(
        &self,
        tenant_id: TenantId,
        kind: EmailKind,
        context: &EmailContext,
        locale_override: Option<&str>,
        draft: Option<&EmailCopy>,
        pay_link: PayLink<'_>,
        whitelabel: bool,
        origin: SendOrigin,
    ) -> StoreResult<BuildOutcome> {
        self.services
            .build_customer_email(
                tenant_id,
                kind,
                context,
                locale_override,
                draft,
                pay_link,
                whitelabel,
                origin,
            )
            .await
    }

    pub async fn preview_customer_email(
        &self,
        tenant_id: TenantId,
        kind: EmailKind,
        context: &EmailContext,
        locale_override: Option<&str>,
        draft: Option<&EmailCopy>,
        pay_link: PayLink<'_>,
        whitelabel: bool,
    ) -> StoreResult<BuildOutcome> {
        self.services
            .preview_customer_email(
                tenant_id,
                kind,
                context,
                locale_override,
                draft,
                pay_link,
                whitelabel,
            )
            .await
    }

    pub fn build_preview(
        &self,
        entity: &InvoicingEntity,
        settings: &EmailSettings,
        kind: EmailKind,
        lang: &'static str,
        draft: Option<&EmailCopy>,
        offset_days: Option<i32>,
        whitelabel: bool,
        test_mode: bool,
    ) -> StoreResult<EmailRenderInput> {
        self.services.build_preview(
            entity,
            settings,
            kind,
            lang,
            draft,
            offset_days,
            whitelabel,
            test_mode,
        )
    }

    pub async fn email_target(
        &self,
        tenant_id: TenantId,
        context: &EmailContext,
    ) -> StoreResult<EmailTarget> {
        self.services.email_target(tenant_id, context).await
    }

    pub async fn preview_reminder_context(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        offset_days: Option<i32>,
    ) -> StoreResult<EmailContext> {
        self.services
            .preview_reminder_context(tenant_id, invoice_id, offset_days)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn billing_emails_fall_back_to_the_customer_email() {
        let invoicing = |emails: &[&str]| emails.iter().map(|e| e.to_string()).collect::<Vec<_>>();
        assert_eq!(
            email_recipients(&[], Some(" ana@acme.test ")),
            vec!["ana@acme.test"]
        );
        assert_eq!(
            email_recipients(
                &invoicing(&["ap@acme.test", " ", "AP@acme.test"]),
                Some("ana@acme.test")
            ),
            vec!["ap@acme.test"]
        );
        assert!(email_recipients(&invoicing(&[" "]), Some("  ")).is_empty());
        assert!(email_recipients(&[], None).is_empty());
    }

    #[test]
    fn reminder_copy_decides_the_language_of_the_whole_email() {
        let copy = |s: &str| EmailCopy {
            subject: s.to_string(),
            body: s.to_string(),
        };
        let content = EmailCopyByLocale(BTreeMap::from([
            ("fr-FR".to_string(), copy("fr")),
            ("en-US".to_string(), copy("en")),
        ]));
        assert_eq!(
            reminder_copy(&content, "fr-FR", "fr-FR").unwrap().0,
            "fr-FR"
        );
        // Missing requested locale: the whole email goes out in the default.
        let (lang, c) = reminder_copy(&content, "de-DE", "fr-FR").unwrap();
        assert_eq!((lang, c.subject.as_str()), ("fr-FR", "fr"));
        // Default missing too: the stored fallback entry and its language.
        assert_eq!(
            reminder_copy(&content, "de-DE", "it-IT").unwrap().0,
            "en-US"
        );
        let only_pt = EmailCopyByLocale(BTreeMap::from([("pt-BR".to_string(), copy("pt"))]));
        assert_eq!(
            reminder_copy(&only_pt, "de-DE", "it-IT").unwrap().0,
            "pt-BR"
        );
        assert!(reminder_copy(&EmailCopyByLocale::default(), "de-DE", "fr-FR").is_none());
    }
}
