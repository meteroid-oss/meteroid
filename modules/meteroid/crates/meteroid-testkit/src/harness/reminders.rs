//! Payment reminder test helpers: sequences, finalized invoice fixtures, audience evaluation.

use std::collections::{BTreeMap, HashSet};

use bytes::Bytes;
use chrono::NaiveDateTime;
use common_domain::ids::{
    CustomerId, InvoiceId, InvoicingEntityId, PaymentReminderSequenceId, PlanVersionId,
    SubscriptionId,
};
use diesel_models::enums::ReminderEnrolledViaEnum;
use diesel_models::payment_reminders::InvoicePaymentReminderRow;
use diesel_models::query::payment_reminders::InvoiceReminderUpdates;
use meteroid::services::storage::ObjectKind;
use meteroid_store::domain::customer_emails::EmailKind;
use meteroid_store::domain::entity_activity::Actor;
use meteroid_store::domain::enums::{InvoicePaymentStatus, InvoiceStatusEnum, InvoiceType};
use meteroid_store::domain::payment_reminders::{
    Audience, CustomerReminderPreference, EmailCopy, EmailCopyByLocale, PaymentReminderSequence,
    PaymentReminderSequenceNew, PaymentReminderSequencePatch, PaymentReminderStepInput,
};
use meteroid_store::domain::{CustomerPatch, InvoiceNew};
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterfaceAuto;
use meteroid_store::repositories::payment_reminders::SequenceWithInFlight;
use meteroid_store::repositories::{
    CustomerEmailsInterface, CustomersInterface, InvoiceInterface, PaymentRemindersInterface,
};
use meteroid_store::services::payment_reminders::audience::{compile, evaluate_audience};
use meteroid_store::services::payment_reminders::sweep::SweepStats;

use crate::data::ids::{INVOICING_ENTITY_ID, TENANT_ID, USER_ID};

use super::TestEnv;

/// The seeded invoicing entity is French with no explicit language, so its copy locale is
/// `fr-FR` and every saved step needs a non-blank entry for it.
pub const ENTITY_LOCALE: &str = "fr-FR";

/// One step per offset with copy in the entity locale and English that passes every rule.
pub fn steps(offsets: &[i32]) -> Vec<PaymentReminderStepInput> {
    offsets
        .iter()
        .map(|offset| PaymentReminderStepInput {
            offset_days: *offset,
            content: copies(
                &format!("Invoice {{{{invoice_number}}}} day {offset}"),
                "Hi {{customer_name}},\n\nPlease pay {{amount_due}}.",
            ),
        })
        .collect()
}

/// The same subject and body under `ENTITY_LOCALE` and `en-US`.
pub fn copies(subject: &str, body: &str) -> EmailCopyByLocale {
    EmailCopyByLocale(
        [ENTITY_LOCALE, "en-US"]
            .into_iter()
            .map(|locale| {
                (
                    locale.to_string(),
                    EmailCopy {
                        subject: subject.to_string(),
                        body: body.to_string(),
                    },
                )
            })
            .collect(),
    )
}

/// A single-locale entry with a valid body.
pub fn copy(locale: &str, subject: &str) -> EmailCopyByLocale {
    EmailCopyByLocale(BTreeMap::from([(
        locale.to_string(),
        EmailCopy {
            subject: subject.to_string(),
            body: "Hi {{customer_name}},\n\nPlease pay {{amount_due}}.".to_string(),
        },
    )]))
}

pub fn audience(predicates: serde_json::Value) -> Audience {
    serde_json::from_value(serde_json::json!({ "predicates": predicates })).expect("audience json")
}

/// An audience every invoice matches, for sequences whose targeting is irrelevant.
pub fn match_all() -> Audience {
    audience(serde_json::json!([
        { "attribute": { "key": "invoice_kind", "values": ["subscription", "one_off", "consolidated"] } }
    ]))
}

pub struct FinalizedInvoice {
    pub customer_id: CustomerId,
    pub subscription_id: Option<SubscriptionId>,
    pub plan_version_id: Option<PlanVersionId>,
    pub amount_due: i64,
    pub currency: &'static str,
    pub consolidated_into: Option<InvoiceId>,
    pub invoice_type: InvoiceType,
    /// `None` = due in 30 days.
    pub due_at: Option<NaiveDateTime>,
    /// `None` = now.
    pub finalized_at: Option<NaiveDateTime>,
}

const PLACEHOLDER_PDF: &[u8] = b"%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[]/Count 0>>endobj\ntrailer<</Root 1 0 R>>\n%%EOF\n";

impl FinalizedInvoice {
    pub fn one_off(customer_id: CustomerId) -> Self {
        Self {
            customer_id,
            subscription_id: None,
            plan_version_id: None,
            amount_due: 10_000,
            currency: "EUR",
            consolidated_into: None,
            invoice_type: InvoiceType::OneOff,
            due_at: None,
            finalized_at: None,
        }
    }

    /// Explicit anchor inputs, for scheduling tests driven by a bound `now`.
    pub fn dates(mut self, due_at: NaiveDateTime, finalized_at: NaiveDateTime) -> Self {
        self.due_at = Some(due_at);
        self.finalized_at = Some(finalized_at);
        self
    }

    pub fn subscription(
        customer_id: CustomerId,
        subscription_id: SubscriptionId,
        plan_version_id: PlanVersionId,
    ) -> Self {
        Self {
            subscription_id: Some(subscription_id),
            plan_version_id: Some(plan_version_id),
            invoice_type: InvoiceType::Recurring,
            ..Self::one_off(customer_id)
        }
    }

    pub fn invoice_type(mut self, invoice_type: InvoiceType) -> Self {
        self.invoice_type = invoice_type;
        self
    }

    pub fn amount(mut self, amount_due: i64, currency: &'static str) -> Self {
        self.amount_due = amount_due;
        self.currency = currency;
        self
    }

    pub fn consolidated_into(mut self, parent: InvoiceId) -> Self {
        self.consolidated_into = Some(parent);
        self
    }
}

impl TestEnv {
    pub async fn create_sequence(
        &self,
        entity: InvoicingEntityId,
        step_offsets: &[i32],
        enabled: bool,
    ) -> PaymentReminderSequence {
        self.create_sequence_with_audience(entity, "Sequence", match_all(), step_offsets, enabled)
            .await
    }

    pub async fn create_sequence_with_audience(
        &self,
        entity: InvoicingEntityId,
        name: &str,
        audience: Audience,
        step_offsets: &[i32],
        enabled: bool,
    ) -> PaymentReminderSequence {
        self.store()
            .create_reminder_sequence(
                Actor::User { id: USER_ID },
                TENANT_ID,
                PaymentReminderSequenceNew {
                    invoicing_entity_id: entity,
                    name: name.to_string(),
                    enabled,
                    audience: Some(audience),
                    steps: steps(step_offsets),
                },
            )
            .await
            .expect("create sequence")
            .sequence
    }

    pub async fn list_sequences(&self, entity: InvoicingEntityId) -> Vec<SequenceWithInFlight> {
        self.store()
            .list_reminder_sequences(TENANT_ID, entity, false)
            .await
            .expect("list sequences")
    }

    /// A finalized, unpaid invoice of the test entity, due in 30 days, bypassing the pipeline.
    pub async fn finalized_invoice(&self, fixture: FinalizedInvoice) -> InvoiceId {
        let customer = self.get_customer(fixture.customer_id).await;
        let invoicing_entity = self
            .store()
            .get_invoicing_entity(TENANT_ID, Some(customer.invoicing_entity_id))
            .await
            .expect("get invoicing entity");
        let entity_id = invoicing_entity.id;
        let now = fixture
            .finalized_at
            .unwrap_or_else(|| common_domain::clock::now().naive_utc());
        let today = now.date();
        let due = fixture.due_at.unwrap_or_else(|| {
            (today + chrono::Days::new(30))
                .and_hms_opt(0, 0, 0)
                .unwrap()
        });

        self.store()
            .insert_invoice(InvoiceNew {
                status: InvoiceStatusEnum::Finalized,
                tenant_id: TENANT_ID,
                customer_id: fixture.customer_id,
                subscription_id: fixture.subscription_id,
                cycle_index: None,
                currency: fixture.currency.to_string(),
                invoice_number: format!("REM-{}", uuid::Uuid::now_v7()),
                line_items: vec![crate::helpers::invoice_lines::line(
                    "Services",
                    fixture.amount_due,
                    false,
                )],
                coupons: vec![],
                data_updated_at: None,
                invoice_date: today,
                billing_period_start: today,
                tax_inclusive: false,
                plan_version_id: fixture.plan_version_id,
                invoice_type: fixture.invoice_type,
                finalized_at: Some(now),
                subtotal: fixture.amount_due,
                subtotal_recurring: 0,
                discount: 0,
                tax_amount: 0,
                total: fixture.amount_due,
                amount_due: fixture.amount_due,
                applied_credits: 0,
                promotional_credits: 0,
                net_terms: 30,
                reference: None,
                purchase_order: None,
                memo: None,
                due_at: Some(due),
                plan_name: None,
                customer_details: customer.into(),
                seller_details: invoicing_entity.into(),
                auto_advance: false,
                payment_status: InvoicePaymentStatus::Unpaid,
                tax_breakdown: vec![],
                manual: fixture.subscription_id.is_none(),
                invoicing_entity_id: entity_id,
                parent_invoice_id: None,
                consolidated_into_invoice_id: fixture.consolidated_into,
                annexes: vec![],
                custom_properties: serde_json::Value::Null,
            })
            .await
            .expect("insert finalized invoice")
            .id
    }

    /// A finalized invoice with a stored PDF (the candidate scan requires one).
    pub async fn finalized_invoice_with_pdf(&self, fixture: FinalizedInvoice) -> InvoiceId {
        let customer_id = fixture.customer_id;
        let invoice_id = self.finalized_invoice(fixture).await;
        self.attach_pdf(invoice_id, customer_id).await;
        invoice_id
    }

    pub async fn attach_pdf(&self, invoice_id: InvoiceId, customer_id: CustomerId) {
        let pdf = self
            .setup
            .object_store
            .store(
                ObjectKind::InvoicePdf,
                TENANT_ID,
                Bytes::from_static(PLACEHOLDER_PDF),
            )
            .await
            .expect("store pdf");
        self.store()
            .save_invoice_documents(invoice_id, TENANT_ID, customer_id, pdf, None, None)
            .await
            .expect("save invoice documents");
    }

    /// One sweep (due scan, then enrolment) at an explicit `now`.
    pub async fn run_sweep(&self, now: NaiveDateTime) -> SweepStats {
        self.services()
            .run_payment_reminder_sweep(now)
            .await
            .expect("payment reminder sweep")
    }

    /// The seeded tenant's emails are `Off` and the kind ships disabled: flip both.
    pub async fn enable_reminders(&self) {
        self.set_tenant_emails(true).await;
        self.set_reminder_kind(INVOICING_ENTITY_ID, true).await;
    }

    pub async fn set_tenant_emails(&self, enabled: bool) {
        self.set_email_delivery_mode(if enabled {
            diesel_models::enums::EmailDeliveryModeEnum::Live
        } else {
            diesel_models::enums::EmailDeliveryModeEnum::Off
        })
        .await;
    }

    pub async fn set_reminder_kind(&self, entity: InvoicingEntityId, enabled: bool) {
        self.store()
            .set_email_kind_enabled(
                Actor::System,
                TENANT_ID,
                entity,
                EmailKind::PaymentReminder,
                enabled,
            )
            .await
            .expect("set kind");
    }

    /// Seeds the default sequence when needed and returns it, enabled or not.
    pub async fn default_sequence(
        &self,
        entity: InvoicingEntityId,
        enabled: bool,
    ) -> PaymentReminderSequence {
        let default = self
            .list_sequences(entity)
            .await
            .into_iter()
            .map(|s| s.sequence)
            .find(|s| s.is_default())
            .expect("default sequence");
        if default.enabled == enabled {
            return default;
        }
        self.set_sequence_enabled(default.id, enabled).await
    }

    pub async fn set_sequence_enabled(
        &self,
        id: PaymentReminderSequenceId,
        enabled: bool,
    ) -> PaymentReminderSequence {
        self.patch_sequence(PaymentReminderSequencePatch {
            id,
            enabled: Some(enabled),
            ..Default::default()
        })
        .await
    }

    pub async fn set_sequence_steps(
        &self,
        id: PaymentReminderSequenceId,
        offsets: &[i32],
    ) -> PaymentReminderSequence {
        self.patch_sequence(PaymentReminderSequencePatch {
            id,
            steps: Some(steps(offsets)),
            ..Default::default()
        })
        .await
    }

    pub async fn patch_sequence(
        &self,
        patch: PaymentReminderSequencePatch,
    ) -> PaymentReminderSequence {
        self.patch_sequence_at(patch, common_domain::clock::now().naive_utc())
            .await
    }

    /// `now` drives the re-arm of frozen invoices.
    pub async fn patch_sequence_at(
        &self,
        patch: PaymentReminderSequencePatch,
        now: NaiveDateTime,
    ) -> PaymentReminderSequence {
        self.store()
            .update_reminder_sequence(Actor::User { id: USER_ID }, TENANT_ID, patch, now)
            .await
            .expect("update sequence")
            .sequence
    }

    pub async fn set_customer_reminder_preference(
        &self,
        customer_id: CustomerId,
        opt_out: bool,
        sequence_id: Option<PaymentReminderSequenceId>,
    ) {
        self.store()
            .set_customer_reminder_preference(
                Actor::User { id: USER_ID },
                TENANT_ID,
                customer_id,
                CustomerReminderPreference {
                    opt_out,
                    sequence_id,
                },
                common_domain::clock::now().naive_utc(),
            )
            .await
            .expect("set preference");
    }

    pub async fn set_customer_emails(
        &self,
        customer: CustomerId,
        emails: &[&str],
        locales: &[&str],
    ) {
        self.store()
            .patch_customer(
                Actor::System,
                TENANT_ID,
                CustomerPatch {
                    id: customer,
                    name: None,
                    alias: None,
                    billing_email: None,
                    invoicing_emails: Some(emails.iter().map(|e| e.to_string()).collect()),
                    phone: None,
                    currency: None,
                    billing_address: None,
                    shipping_address: None,
                    invoicing_entity_id: None,
                    vat_number: None,
                    legal_number: None,
                    buyer_reference: None,
                    custom_taxes: None,
                    current_payment_method_id: None,
                    tax_status: None,
                    exemption_reason: None,
                    connected_account_id: None,
                    custom_properties: None,
                    preferred_locales: Some(locales.iter().map(|l| l.to_string()).collect()),
                    customer_type: None,
                    first_name: None,
                    last_name: None,
                },
            )
            .await
            .expect("patch customer");
    }

    /// Enrols an invoice on a sequence with a reminder due tomorrow, as the sweeper would.
    pub async fn enroll_invoice(
        &self,
        invoice_id: InvoiceId,
        sequence_id: PaymentReminderSequenceId,
    ) -> InvoicePaymentReminderRow {
        let mut conn = self.conn().await;
        let now = common_domain::clock::now().naive_utc();
        InvoiceReminderUpdates::enroll(
            &mut conn,
            invoice_id,
            TENANT_ID,
            sequence_id,
            ReminderEnrolledViaEnum::Audience,
            None,
            Some(now + chrono::Duration::days(1)),
            now,
        )
        .await
        .expect("enroll invoice")
    }

    pub async fn reminder_row(&self, invoice_id: InvoiceId) -> Option<InvoicePaymentReminderRow> {
        let mut conn = self.conn().await;
        InvoicePaymentReminderRow::find_by_invoice_id(&mut conn, TENANT_ID, invoice_id)
            .await
            .expect("find reminder row")
    }

    /// The subset of `ids` an audience matches.
    pub async fn matching(&self, audience: &Audience, ids: &[InvoiceId]) -> HashSet<InvoiceId> {
        let compiled = compile(audience).expect("compile audience");
        let mut conn = self.conn().await;
        evaluate_audience(&mut conn, TENANT_ID, ids, &compiled)
            .await
            .expect("evaluate audience")
            .into_iter()
            .collect()
    }
}
