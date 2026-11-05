//! Customer emails platform: entity settings, the shared send path, producers of the new kinds,
//! previews, resend and the claim-before-send state machine.

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use chrono::Utc;
use http::{HeaderMap, HeaderName, HeaderValue};
use rstest::rstest;
use tonic::Code;
use tower_http::set_header::SetRequestHeaderLayer;

use common_domain::country::CountryCode;
use common_domain::ids::{
    BaseId, CreditOperationId, CustomerId, EntityActivityId, InvoiceId, InvoicingEntityId, QuoteId,
    RoleId, UserId, WalletId,
};
use common_domain::pgmq::{MessageReadQty, MessageReadVtSec};
use common_domain::rbac::Permission;
use common_grpc::middleware::common::auth::API_KEY_HEADER;
use diesel_models::email_send_claim::EmailSendClaimRow;
use diesel_models::entity_activity::EntityActivityRow;
use diesel_models::enums::{
    EmailSendClaimStateEnum, OrganizationUserRole as DbOrganizationUserRole,
};
use diesel_models::invoices::InvoiceRow;
use diesel_models::organization_members::OrganizationMemberRow;
use diesel_models::query::sent_email::SentEmailFilter;
use diesel_models::sent_email::SentEmailRow;
use diesel_models::users::UserRowNew;
use meteroid::api_rest::webhooks::event_handler::handle_normalized_event;
use meteroid::services::credit_note_rendering::CreditNotePdfRenderingService;
use meteroid::services::storage::ObjectKind;
use meteroid::workers::pgmq::processors::{
    handle_send_email_messages, run_once_credit_note_pdf_render, run_once_invoice_orchestration,
    run_once_send_email,
};
use meteroid_grpc::meteroid::api::customeremails::v1 as api;
use meteroid_grpc::meteroid::api::customeremails::v1::customer_emails_service_client::CustomerEmailsServiceClient;
use meteroid_grpc::meteroid::api::invoicingentities::v1::GetInvoicingEntityRequest;
use meteroid_mailer::model::Email;
use meteroid_store::adapters::payment::initialize_payment_connector;
use meteroid_store::domain::connectors::{
    Connector, ConnectorConfig, CredentialState, MockChargeBehavior, MockPublicData,
    MockSensitiveData, PaymentConnectorConfig, PaymentProviderConfig,
};
use meteroid_store::domain::customer_emails::{EmailContext, EmailKind};
use meteroid_store::domain::entity_activity::Actor;
use meteroid_store::domain::enums::{InvoicePaymentStatus, OrganizationUserRole};
use meteroid_store::domain::pgmq::{EmailSendAttempts, PgmqQueue, SendEmailRequest};
use meteroid_store::domain::tenants::TenantUpdate;
use meteroid_store::domain::{CustomerPatch, InvoicingEntityNew, InvoicingEntityPatch};
use meteroid_store::repositories::credit_notes::{CreateCreditNoteParams, CreditType};
use meteroid_store::repositories::invoicing_entities::{
    InvoicingEntityInterface, InvoicingEntityInterfaceAuto,
};
use meteroid_store::repositories::pgmq::PgmqInterface;
use meteroid_store::repositories::{
    CreditNoteInterface, CustomerEmailsInterface, CustomersInterface, InvoiceInterface,
    RolesInterface, TenantInterface,
};

use crate::data::ids::*;
use crate::harness::reminders::FinalizedInvoice;
use crate::harness::{TestEnv, subscription, test_env};
use crate::meteroid_it;
use crate::meteroid_it::clients::AllClients;

const PLACEHOLDER_PDF: &[u8] = b"%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[]/Count 0>>endobj\ntrailer<</Root 1 0 R>>\n%%EOF\n";
const RECIPIENT: &str = "billing@uber.example";

// ---- fixtures -----------------------------------------------------------------------------

/// The seeded tenant's emails are `Off`; every send test sets the mode explicitly.
pub(crate) async fn enable_emails(env: &TestEnv) {
    env.set_email_delivery_mode(diesel_models::enums::EmailDeliveryModeEnum::Live)
        .await;
}

pub(crate) async fn set_recipients(
    env: &TestEnv,
    customer: CustomerId,
    emails: &[&str],
    locales: &[&str],
) {
    env.store()
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

async fn set_kind(env: &TestEnv, kind: EmailKind, enabled: bool) {
    env.store()
        .set_email_kind_enabled(Actor::System, TENANT_ID, INVOICING_ENTITY_ID, kind, enabled)
        .await
        .expect("set kind");
}

/// Extra customer-facing locales of the seeded (French) entity, on top of its fr-FR default.
async fn set_customer_extra_locales(env: &TestEnv, locales: &[&str]) {
    env.store()
        .patch_invoicing_entity(
            Actor::System,
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                customer_extra_locales: Some(locales.iter().map(|l| l.to_string()).collect()),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .expect("customer extra locales");
}

/// The seeded entity's customer-facing default; `None` follows the invoice language again.
async fn set_customer_locale(env: &TestEnv, locale: Option<&str>) {
    env.store()
        .patch_invoicing_entity(
            Actor::System,
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                customer_locale: Some(locale.map(str::to_string)),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .expect("customer locale");
}

pub(crate) async fn clients(env: &TestEnv) -> AllClients {
    let auth = meteroid_it::svc_auth::login(env.setup.channel.clone()).await;
    AllClients::from_channel(
        env.setup.channel.clone(),
        &auth.token,
        "TESTORG",
        "testslug",
    )
}

/// A finalized, unpaid invoice that went through the pipeline (PDF stored, `invoice_ready`
/// queued for the customer's `invoicing_emails`).
async fn pipeline_invoice(env: &TestEnv, customer: CustomerId) -> InvoiceId {
    let sub_id = subscription()
        .customer(customer)
        .on_start()
        .no_trial()
        .no_auto_charge()
        .create(env.services())
        .await;
    env.run_outbox_and_orchestration().await;
    env.get_invoices(sub_id).await[0].id
}

/// A finalized unpaid invoice inserted directly, with a stored PDF and no queued email.
pub(crate) async fn invoice_with_pdf(env: &TestEnv, customer: CustomerId) -> InvoiceId {
    let invoice_id = env
        .finalized_invoice(FinalizedInvoice::one_off(customer))
        .await;
    let pdf = env
        .setup
        .object_store
        .store(
            ObjectKind::InvoicePdf,
            TENANT_ID,
            Bytes::from_static(PLACEHOLDER_PDF),
        )
        .await
        .expect("store pdf");
    env.store()
        .save_invoice_documents(invoice_id, TENANT_ID, customer, pdf, None, None)
        .await
        .expect("save invoice documents");
    invoice_id
}

/// Sends one invoice_ready email for `invoice_id` and returns its HTML body.
async fn send_invoice_ready(env: &TestEnv, invoice_id: InvoiceId) -> String {
    enqueue(
        env,
        EmailKind::InvoiceReady,
        EmailContext::Invoice { invoice_id },
        &format!("invoice_ready:{invoice_id}"),
    )
    .await;
    let emails = env.drain_email_queue().await;
    assert_eq!(emails.len(), 1);
    emails[0].body_html.clone()
}

async fn enqueue(env: &TestEnv, kind: EmailKind, context: EmailContext, key: &str) {
    env.store()
        .enqueue_customer_email(TENANT_ID, kind, context, key.to_string())
        .await
        .expect("enqueue");
}

async fn run_email_worker_once(env: &TestEnv) -> Vec<Email> {
    let config = &env.setup.config;
    run_once_send_email(
        Arc::new(env.store().clone()),
        Arc::new(env.services().clone()),
        env.mailer.clone(),
        env.setup.object_store.clone(),
        config.public_url.clone(),
        config.rest_api_external_url.clone(),
    )
    .await
    .expect("email worker");
    env.mailer.take()
}

async fn sent_rows(env: &TestEnv) -> Vec<SentEmailRow> {
    let mut conn = env.conn().await;
    SentEmailRow::list(&mut conn, TENANT_ID, &SentEmailFilter::default(), None, 100)
        .await
        .expect("list sent_email")
        .into_iter()
        .map(|r| r.email)
        .collect()
}

async fn claim(env: &TestEnv, key: &str) -> Option<EmailSendClaimRow> {
    let mut conn = env.conn().await;
    EmailSendClaimRow::find_by_dedupe_key(&mut conn, key)
        .await
        .expect("find claim")
}

async fn activities(env: &TestEnv, entity_type: &str, id: uuid::Uuid) -> Vec<EntityActivityRow> {
    let mut conn = env.conn().await;
    EntityActivityRow::list_by_entity(&mut conn, TENANT_ID, entity_type, id, None, 50)
        .await
        .expect("list activities")
}

fn reasons(rows: &[EntityActivityRow], activity_type: &str) -> Vec<String> {
    rows.iter()
        .filter(|a| a.activity_type == activity_type)
        .filter_map(|a| {
            a.metadata
                .as_ref()
                .and_then(|m| m.get("reason"))
                .and_then(|r| r.as_str())
                .map(String::from)
        })
        .collect()
}

/// Every message on the email queue, visible or not: `(attempts, seconds until visible)`.
async fn queued_email_messages(env: &TestEnv) -> Vec<(EmailSendAttempts, i64)> {
    use diesel_async::RunQueryDsl;

    #[derive(diesel::QueryableByName)]
    struct Row {
        #[diesel(sql_type = diesel::sql_types::Jsonb)]
        message: serde_json::Value,
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        visible_in: i64,
    }
    let mut conn = env.conn().await;
    diesel::sql_query(
        "SELECT message, CAST(EXTRACT(EPOCH FROM (vt - now())) AS BIGINT) AS visible_in \
         FROM pgmq.q_send_email_request ORDER BY msg_id",
    )
    .load::<Row>(&mut conn)
    .await
    .expect("read queue")
    .into_iter()
    .map(|r| {
        let attempts = serde_json::from_value::<EmailSendAttempts>(
            r.message["CustomerEmail"]["attempts"].clone(),
        )
        .unwrap_or_default();
        (attempts, r.visible_in)
    })
    .collect()
}

async fn make_queue_visible(env: &TestEnv) {
    use diesel_async::RunQueryDsl;
    let mut conn = env.conn().await;
    diesel::sql_query("UPDATE pgmq.q_send_email_request SET vt = now()")
        .execute(&mut conn)
        .await
        .expect("make visible");
}

async fn delete_settings_row(env: &TestEnv, entity: InvoicingEntityId) {
    use diesel_async::RunQueryDsl;
    let mut conn = env.conn().await;
    diesel::sql_query("DELETE FROM invoicing_entity_email_settings WHERE invoicing_entity_id = $1")
        .bind::<diesel::sql_types::Uuid, _>(entity.as_uuid())
        .execute(&mut conn)
        .await
        .expect("delete settings row");
}

async fn stored_disabled_kinds(env: &TestEnv, entity: InvoicingEntityId) -> Option<Vec<String>> {
    use diesel_models::invoicing_entity_email_settings::InvoicingEntityEmailSettingsRow;
    let mut conn = env.conn().await;
    InvoicingEntityEmailSettingsRow::find_by_entity_id(&mut conn, TENANT_ID, entity)
        .await
        .expect("find settings row")
        .map(|r| r.disabled_kinds.into_iter().flatten().collect())
}

async fn create_entity(env: &TestEnv, name: &str) -> InvoicingEntityId {
    env.store()
        .create_invoicing_entity(
            InvoicingEntityNew {
                legal_name: Some(name.to_string()),
                ..Default::default()
            },
            TENANT_ID,
            ORGANIZATION_ID,
        )
        .await
        .expect("create invoicing entity")
        .id
}

fn enabled_kinds(settings: &api::EmailSettings) -> Vec<(i32, bool)> {
    settings.kinds.iter().map(|k| (k.kind, k.enabled)).collect()
}

fn kind_enabled(settings: &api::EmailSettings, kind: api::EmailKind) -> bool {
    settings
        .kinds
        .iter()
        .find(|k| k.kind == kind as i32)
        .map(|k| k.enabled)
        .unwrap_or_else(|| panic!("{kind:?} missing from settings"))
}

fn mock_connector() -> Connector {
    Connector {
        status: meteroid_store::domain::enums::ConnectorStatusEnum::Active,
        credentials: CredentialState::Unverified,
        webhook_route: None,
        id: MOCK_CONNECTOR_ID,
        created_at: Utc::now().naive_utc(),
        tenant_id: TENANT_ID,
        alias: "mock-payment-provider".to_string(),
        config: ConnectorConfig::Payment(PaymentConnectorConfig::configured(
            PaymentProviderConfig::Mock(MockPublicData::default(), MockSensitiveData {}),
        )),
    }
}

async fn deliver_webhook(env: &TestEnv, payload: String) {
    let connector = mock_connector();
    let connector_impl = initialize_payment_connector(&connector).expect("mock connector");
    let event = connector_impl
        .parse_event(payload.as_bytes(), &HeaderMap::new())
        .expect("parse_event")
        .expect("an event");
    handle_normalized_event(
        event,
        &connector,
        connector_impl.as_ref(),
        env.store().clone(),
        env.services(),
    )
    .await
    .expect("dispatcher handled the event");
    env.run_outbox_and_orchestration().await;
}

fn failed_payload(tx: common_domain::ids::PaymentTransactionId, external_id: &str) -> String {
    format!(
        r#"{{"id":"evt_fail_{tx}","kind":"payment_failed","transaction_id":"{tx}","external_id":"{external_id}"}}"#,
        tx = tx.as_base62(),
    )
}

// ---- settings -----------------------------------------------------------------------------

#[rstest]
#[tokio::test]
async fn entity_email_settings_round_trip_and_audit(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let mut client = clients(&env).await.customer_emails;
    let entity_id = INVOICING_ENTITY_ID.as_proto();

    let updated = client
        .update_email_settings(api::UpdateEmailSettingsRequest {
            invoicing_entity_id: entity_id.clone(),
            data: Some(api::EmailSettingsData {
                sender_name: Some("Acme Facturation".into()),
                reply_to: Some("ar@acme.example".into()),
                bcc: Some(api::StringList {
                    values: vec!["audit@acme.example".into(), "Audit@acme.example".into()],
                }),
                timezone: Some("Europe/Paris".into()),
                reminder_send_hour: Some(10),
            }),
        })
        .await
        .expect("update settings")
        .into_inner()
        .settings
        .expect("settings");
    assert_eq!(updated.sender_name.as_deref(), Some("Acme Facturation"));
    assert_eq!(
        updated.from_address,
        "Acme Facturation <mailer@example.com>"
    );
    assert_eq!(updated.reply_to.as_deref(), Some("ar@acme.example"));
    assert_eq!(updated.bcc, vec!["audit@acme.example"]);
    assert_eq!(updated.timezone.as_deref(), Some("Europe/Paris"));
    assert_eq!(updated.reminder_send_hour, 10);
    assert_eq!(updated.effective_locale, "fr-FR");
    assert!(updated.extra_locales.is_empty());

    let fetched = client
        .get_email_settings(api::GetEmailSettingsRequest {
            invoicing_entity_id: None,
        })
        .await
        .expect("get settings")
        .into_inner()
        .settings
        .expect("settings");
    assert_eq!(fetched, updated);

    for (label, data) in [
        (
            "invalid reply-to",
            api::EmailSettingsData {
                reply_to: Some("not an address".into()),
                ..Default::default()
            },
        ),
        (
            "sixth bcc",
            api::EmailSettingsData {
                bcc: Some(api::StringList {
                    values: (0..6).map(|i| format!("a{i}@acme.example")).collect(),
                }),
                ..Default::default()
            },
        ),
        (
            "bad timezone",
            api::EmailSettingsData {
                timezone: Some("Mars/Olympus".into()),
                ..Default::default()
            },
        ),
    ] {
        let err = client
            .update_email_settings(api::UpdateEmailSettingsRequest {
                invoicing_entity_id: entity_id.clone(),
                data: Some(data),
            })
            .await
            .expect_err(label);
        assert_eq!(err.code(), Code::InvalidArgument, "{label}: {err}");
    }

    let audit = activities(&env, "invoicing_entity", INVOICING_ENTITY_ID.as_uuid()).await;
    let changed: Vec<String> = audit
        .iter()
        .filter(|a| a.activity_type == "entity.updated")
        .flat_map(|a| {
            a.metadata
                .as_ref()
                .and_then(|m| m.get("changes"))
                .and_then(|c| c.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|c| c.get("field").and_then(|f| f.as_str()).map(String::from))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
        .collect();
    assert!(
        changed.contains(&"email_settings.bcc".to_string()),
        "{changed:?}"
    );
    assert!(changed.contains(&"email_settings.sender_name".to_string()));
    // A rejected save audits nothing.
    assert_eq!(
        audit
            .iter()
            .filter(|a| a.activity_type == "entity.updated")
            .count(),
        1
    );

    // The locales are edited on the entity; the settings only mirror them, normalized.
    set_customer_extra_locales(&env, &["de-AT", "fr-FR", "de-DE"]).await;
    set_customer_locale(&env, Some("en-GB")).await;
    let mirrored = client
        .get_email_settings(api::GetEmailSettingsRequest {
            invoicing_entity_id: None,
        })
        .await
        .expect("get settings")
        .into_inner()
        .settings
        .expect("settings");
    assert_eq!(mirrored.effective_locale, "en-US");
    assert_eq!(
        mirrored.extra_locales,
        vec!["de-DE"],
        "normalized, deduped, the invoice-language default dropped at save"
    );
}

#[rstest]
#[tokio::test]
async fn only_payment_reminders_ship_disabled(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let mut client = clients(&env).await.customer_emails;
    let settings = client
        .get_email_settings(api::GetEmailSettingsRequest {
            invoicing_entity_id: Some(INVOICING_ENTITY_ID.as_proto()),
        })
        .await
        .expect("get")
        .into_inner()
        .settings
        .expect("settings");
    for kind in [
        api::EmailKind::InvoiceReady,
        api::EmailKind::InvoicePaid,
        api::EmailKind::QuoteReady,
        api::EmailKind::CreditNoteIssued,
        api::EmailKind::PaymentFailed,
        api::EmailKind::AuthenticationRequired,
        api::EmailKind::WalletBalanceLow,
        api::EmailKind::AutoTopUpFailed,
        api::EmailKind::BudgetThreshold,
        api::EmailKind::BudgetExceeded,
    ] {
        assert!(kind_enabled(&settings, kind), "{kind:?} must ship enabled");
    }
    assert!(
        !kind_enabled(&settings, api::EmailKind::PaymentReminder),
        "reminders need a deliberate sequence: off until enabled"
    );
    assert_eq!(settings.kinds.len(), 11, "{:?}", enabled_kinds(&settings));

    // An entity created after the migration gets the disabled set written explicitly.
    let created = create_entity(&env, "Fresh").await;
    let stored = stored_disabled_kinds(&env, created)
        .await
        .expect("row inserted");
    assert_eq!(stored, vec!["payment_reminder"]);
}

#[rstest]
#[tokio::test]
async fn lazy_settings_upsert_keeps_reminders_disabled(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let mut client = clients(&env).await.customer_emails;
    let a = create_entity(&env, "A").await;
    let b = create_entity(&env, "B").await;
    // Pre-feature entities: no settings row at all.
    delete_settings_row(&env, a).await;
    delete_settings_row(&env, b).await;

    let after_name = client
        .update_email_settings(api::UpdateEmailSettingsRequest {
            invoicing_entity_id: a.as_proto(),
            data: Some(api::EmailSettingsData {
                sender_name: Some("Only a name".into()),
                ..Default::default()
            }),
        })
        .await
        .expect("update")
        .into_inner()
        .settings
        .expect("settings");
    assert!(
        !kind_enabled(&after_name, api::EmailKind::PaymentReminder),
        "reminders switched on by a name save"
    );
    assert!(kind_enabled(&after_name, api::EmailKind::InvoicePaid));
    assert!(kind_enabled(&after_name, api::EmailKind::PaymentFailed));

    let after_toggle = client
        .set_email_kind_enabled(api::SetEmailKindEnabledRequest {
            invoicing_entity_id: b.as_proto(),
            kind: api::EmailKind::InvoicePaid as i32,
            enabled: false,
        })
        .await
        .expect("toggle")
        .into_inner()
        .settings
        .expect("settings");
    assert!(!kind_enabled(&after_toggle, api::EmailKind::InvoicePaid));
    assert!(!kind_enabled(
        &after_toggle,
        api::EmailKind::PaymentReminder
    ));
    assert!(kind_enabled(&after_toggle, api::EmailKind::InvoiceReady));
    let mut stored = stored_disabled_kinds(&env, b).await.expect("row");
    stored.sort();
    assert_eq!(stored, vec!["invoice_paid", "payment_reminder"]);

    // Enabling is the only path that removes a kind from the set.
    let enabled = client
        .set_email_kind_enabled(api::SetEmailKindEnabledRequest {
            invoicing_entity_id: b.as_proto(),
            kind: api::EmailKind::PaymentReminder as i32,
            enabled: true,
        })
        .await
        .expect("enable")
        .into_inner()
        .settings
        .expect("settings");
    assert!(kind_enabled(&enabled, api::EmailKind::PaymentReminder));
    assert!(!kind_enabled(&enabled, api::EmailKind::InvoicePaid));

    // Another tenant's entity id is refused, never written.
    let other = env
        .store()
        .insert_tenant(
            meteroid_store::domain::tenants::TenantNew {
                name: "Other".to_string(),
                environment: meteroid_store::domain::enums::TenantEnvironmentEnum::Development,
                email_delivery_mode: Some(meteroid_store::domain::enums::EmailDeliveryMode::Off),
                invoicing_entity: None,
            },
            ORGANIZATION_ID,
        )
        .await
        .expect("other tenant");
    let foreign = env
        .store()
        .get_invoicing_entity(other.id, None)
        .await
        .expect("foreign entity")
        .id;
    let err = client
        .set_email_kind_enabled(api::SetEmailKindEnabledRequest {
            invoicing_entity_id: foreign.as_proto(),
            kind: api::EmailKind::PaymentReminder as i32,
            enabled: true,
        })
        .await
        .expect_err("cross-tenant write");
    assert_eq!(err.code(), Code::NotFound, "{err}");
    let foreign_settings = env
        .store()
        .get_email_settings(other.id, foreign)
        .await
        .expect("foreign settings");
    assert!(!foreign_settings.is_enabled(EmailKind::PaymentReminder));
}

// ---- send path ----------------------------------------------------------------------------

#[rstest]
#[tokio::test]
async fn invoice_ready_uses_entity_sender_bcc_locale_and_footer(#[future] test_env: TestEnv) {
    let env = test_env.await;
    enable_emails(&env).await;
    set_recipients(&env, CUST_UBER_ID, &[RECIPIENT], &["fr-FR"]).await;
    env.store()
        .patch_invoicing_entity(
            Actor::System,
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                address_line1: Some("1 rue de la Paix".into()),
                zip_code: Some("75002".into()),
                city: Some("Paris".into()),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .expect("patch entity");
    env.store()
        .update_email_settings(
            Actor::System,
            TENANT_ID,
            INVOICING_ENTITY_ID,
            meteroid_store::domain::customer_emails::EmailSettingsPatch {
                sender_name: Some(Some("Acme Facturation".into())),
                reply_to: Some(Some("ar@acme.example".into())),
                bcc: Some(vec!["audit@acme.example".into()]),
                ..Default::default()
            },
        )
        .await
        .expect("settings");

    let invoice_id = pipeline_invoice(&env, CUST_UBER_ID).await;
    let emails = env.drain_email_queue().await;
    assert_eq!(emails.len(), 1, "one invoice_ready email");
    let email = &emails[0];
    assert_eq!(email.from, "Acme Facturation <mailer@example.com>");
    assert_eq!(email.reply_to.as_deref(), Some("ar@acme.example"));
    assert_eq!(
        email
            .to
            .iter()
            .map(|r| r.email.as_str())
            .collect::<Vec<_>>(),
        vec![RECIPIENT]
    );
    assert_eq!(
        email
            .bcc
            .iter()
            .map(|r| r.email.as_str())
            .collect::<Vec<_>>(),
        vec!["audit@acme.example"]
    );
    assert!(
        email.body_html.starts_with("<html lang=\"fr\">"),
        "{}",
        &email.body_html[..60]
    );
    assert!(email.body_html.contains("1 rue de la Paix"));
    assert!(email.body_html.contains("75002 Paris"));
    assert!(
        email.body_html.contains("Propulsé par Meteroid"),
        "no whitelabel in tests"
    );
    assert!(
        email
            .body_html
            .contains(">Email de test : envoyé depuis un environnement hors production."),
        "the harness tenant is non-production: test-mode banner"
    );
    assert!(
        !email.body_html.contains("Meteroid SAS"),
        "the Meteroid footer leaked"
    );
    assert_eq!(email.attachments.len(), 1);
    assert!(email.attachments[0].content.starts_with(b"%PDF"));
    assert!(email.attachments[0].filename.ends_with(".pdf"));
    assert_eq!(email.subject, "Votre facture ACME_UK est disponible");

    let rows = sent_rows(&env).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind.as_deref(), Some("invoice_ready"));
    assert_eq!(rows[0].locale.as_deref(), Some("fr-FR"));
    assert_eq!(rows[0].invoicing_entity_id, Some(INVOICING_ENTITY_ID));
    assert_eq!(
        rows[0].dedupe_key.as_deref(),
        Some(format!("invoice_ready:{invoice_id}").as_str())
    );
    let claim = claim(&env, &format!("invoice_ready:{invoice_id}"))
        .await
        .expect("claim row");
    assert_eq!(claim.state, EmailSendClaimStateEnum::Sent);
    assert_eq!(claim.sent_email_id, Some(rows[0].id));
}

async fn preview_invoice_ready_locale(env: &TestEnv, invoice_id: InvoiceId) -> (String, String) {
    let email = clients(env)
        .await
        .customer_emails
        .preview_invoice_email(api::PreviewInvoiceEmailRequest {
            invoice_id: invoice_id.as_proto(),
            kind: api::EmailKind::InvoiceReady as i32,
            locale: None,
            offset_days: None,
        })
        .await
        .expect("preview")
        .into_inner()
        .email
        .expect("email");
    (email.locale, email.body_html)
}

#[rstest]
#[tokio::test]
async fn email_follows_the_customer_where_the_law_binds_only_the_invoice_language(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    // A Belgian entity must word its invoices in Dutch; that law does not reach the email.
    env.store()
        .patch_invoicing_entity(
            Actor::System,
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                country: Some(CountryCode::from_str("BE").unwrap()),
                invoicing_language: Some(Some("nl-NL".into())),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .expect("patch entity");
    set_recipients(&env, CUST_UBER_ID, &[RECIPIENT], &["fr-FR"]).await;
    let invoice_id = invoice_with_pdf(&env, CUST_UBER_ID).await;

    let (locale, html) = preview_invoice_ready_locale(&env, invoice_id).await;
    assert_eq!(locale, "fr-FR");
    assert!(html.starts_with("<html lang=\"fr\">"));
}

#[rstest]
#[tokio::test]
async fn with_an_explicit_customer_language_only_enabled_preferences_win(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    set_customer_locale(&env, Some("de-DE")).await;
    set_recipients(&env, CUST_UBER_ID, &[RECIPIENT], &["fr-FR"]).await;
    let invoice_id = invoice_with_pdf(&env, CUST_UBER_ID).await;

    let (locale, _) = preview_invoice_ready_locale(&env, invoice_id).await;
    assert_eq!(locale, "de-DE", "French is not enabled");

    set_customer_extra_locales(&env, &["fr-FR"]).await;
    let (locale, html) = preview_invoice_ready_locale(&env, invoice_id).await;
    assert_eq!(locale, "fr-FR");
    assert!(html.starts_with("<html lang=\"fr\">"));
}

/// Tester report: a German customer of a French entity got a German PDF and a French email.
#[rstest]
#[tokio::test]
async fn german_customer_of_a_french_entity_gets_german_emails(#[future] test_env: TestEnv) {
    let env = test_env.await;
    enable_emails(&env).await;
    set_recipients(&env, CUST_UBER_ID, &[RECIPIENT], &["de-DE"]).await;

    let invoice_id = invoice_with_pdf(&env, CUST_UBER_ID).await;
    enqueue(
        &env,
        EmailKind::InvoiceReady,
        EmailContext::Invoice { invoice_id },
        &format!("invoice_ready:{invoice_id}"),
    )
    .await;
    let emails = env.drain_email_queue().await;
    assert_eq!(emails.len(), 1);
    assert!(emails[0].body_html.starts_with("<html lang=\"de\">"));
    let rows = sent_rows(&env).await;
    assert_eq!(rows[0].locale.as_deref(), Some("de-DE"));
}

#[rstest]
#[tokio::test]
async fn customer_locale_overrides_the_invoice_language_for_emails_only(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    enable_emails(&env).await;
    set_recipients(&env, CUST_UBER_ID, &[RECIPIENT], &[]).await;
    set_customer_locale(&env, Some("en-US")).await;

    // The French entity keeps French invoices; only customer-facing surfaces switch.
    let entity = clients(&env)
        .await
        .invoicing_entities
        .get_invoicing_entity(GetInvoicingEntityRequest { id: None })
        .await
        .expect("get entity")
        .into_inner()
        .entity
        .expect("entity");
    assert_eq!(entity.customer_locale.as_deref(), Some("en-US"));
    assert_eq!(entity.effective_customer_locale, "en-US");
    assert_eq!(entity.effective_document_language, "fr-FR");

    let first = invoice_with_pdf(&env, CUST_UBER_ID).await;
    assert!(
        send_invoice_ready(&env, first)
            .await
            .starts_with("<html lang=\"en\">"),
        "a customer without a preference gets the customer locale"
    );

    // Cleared: back to the invoice language.
    set_customer_locale(&env, None).await;
    let second = invoice_with_pdf(&env, CUST_UBER_ID).await;
    assert!(
        send_invoice_ready(&env, second)
            .await
            .starts_with("<html lang=\"fr\">")
    );
    let locales: Vec<Option<String>> = sent_rows(&env)
        .await
        .into_iter()
        .map(|r| r.locale)
        .collect();
    assert!(locales.contains(&Some("en-US".to_string())), "{locales:?}");
    assert!(locales.contains(&Some("fr-FR".to_string())), "{locales:?}");
}

/// An invoice voided after its `invoice_ready` was queued is not sent: there is nothing to pay.
#[rstest]
#[tokio::test]
async fn invoice_ready_skips_an_invoice_voided_since_it_was_queued(#[future] test_env: TestEnv) {
    let env = test_env.await;
    enable_emails(&env).await;
    set_recipients(&env, CUST_UBER_ID, &[RECIPIENT], &[]).await;
    let invoice_id = invoice_with_pdf(&env, CUST_UBER_ID).await;
    let key = format!("invoice_ready:{invoice_id}");
    enqueue(
        &env,
        EmailKind::InvoiceReady,
        EmailContext::Invoice { invoice_id },
        &key,
    )
    .await;
    env.try_void_invoice(invoice_id).await.expect("void");

    env.drain_email_queue().await;
    let skipped = activities(&env, "invoice", invoice_id.as_uuid()).await;
    assert_eq!(
        reasons(&skipped, "entity.email_skipped"),
        vec!["not_payable"]
    );
    assert_eq!(
        claim(&env, &key).await.expect("claim").state,
        EmailSendClaimStateEnum::Failed,
        "a skipped send releases its claim"
    );
}

#[rstest]
#[tokio::test]
async fn per_kind_toggle_skips_with_activity_but_tenant_switch_wins(#[future] test_env: TestEnv) {
    let env = test_env.await;
    enable_emails(&env).await;
    set_recipients(&env, CUST_UBER_ID, &[RECIPIENT], &[]).await;
    set_recipients(&env, CUST_SPOTIFY_ID, &["ap@spotify.example"], &[]).await;
    set_kind(&env, EmailKind::InvoiceReady, false).await;

    let invoice_id = pipeline_invoice(&env, CUST_UBER_ID).await;
    assert!(env.drain_email_queue().await.is_empty(), "kind disabled");
    let skipped = activities(&env, "invoice", invoice_id.as_uuid()).await;
    assert_eq!(
        reasons(&skipped, "entity.email_skipped"),
        vec!["kind_disabled"]
    );
    let key = format!("invoice_ready:{invoice_id}");
    assert_eq!(
        claim(&env, &key).await.expect("claim").state,
        EmailSendClaimStateEnum::Failed,
        "a skipped send releases its claim"
    );

    // Kind back on, but delivery Off holds everything, logged and unclaimed.
    set_kind(&env, EmailKind::InvoiceReady, true).await;
    env.store()
        .update_tenant(
            Actor::System,
            TenantUpdate {
                trade_name: None,
                name: None,
                slug: None,
                environment: None,
                reporting_currency: None,
                email_delivery_mode: Some(meteroid_store::domain::enums::EmailDeliveryMode::Off),
                email_test_recipients: None,
                legacy_disable_emails: None,
                alert_evaluation_interval_secs: None,
                keep_last_events: None,
                portal_embed_origins: None,
            },
            ORGANIZATION_ID,
            TENANT_ID,
        )
        .await
        .expect("disable emails");
    let held = pipeline_invoice(&env, CUST_SPOTIFY_ID).await;
    assert!(
        env.drain_email_queue().await.is_empty(),
        "delivery Off wins"
    );
    let held_activities = activities(&env, "invoice", held.as_uuid()).await;
    assert_eq!(
        reasons(&held_activities, "entity.email_skipped"),
        vec!["emails_off"]
    );
    assert!(
        claim(&env, &format!("invoice_ready:{held}"))
            .await
            .is_none()
    );
    assert_eq!(sent_rows(&env).await.len(), 0);
}

#[rstest]
#[tokio::test]
async fn off_mode_skips_are_listed_per_invoicing_entity(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_email_delivery_mode(diesel_models::enums::EmailDeliveryModeEnum::Off)
        .await;
    set_recipients(&env, CUST_UBER_ID, &[RECIPIENT], &[]).await;
    let invoice_id = pipeline_invoice(&env, CUST_UBER_ID).await;
    assert!(env.drain_email_queue().await.is_empty(), "delivery Off");

    let listed = list_skipped(&env, INVOICING_ENTITY_ID, None).await;
    assert_eq!(listed.emails.len(), 1);
    assert!(listed.next_cursor.is_none());
    let skip = &listed.emails[0];
    assert_eq!(skip.kind, api::EmailKind::InvoiceReady as i32);
    assert_eq!(skip.reason.as_deref(), Some("emails_off"));
    assert_eq!(skip.entity_type, "invoice");
    assert_eq!(skip.entity_id, invoice_id.as_proto());
    assert_eq!(
        skip.customer_id.as_deref(),
        Some(CUST_UBER_ID.as_proto().as_str())
    );
    assert!(skip.customer_name.is_some());

    let by_kind = list_skipped(
        &env,
        INVOICING_ENTITY_ID,
        Some(api::EmailKind::InvoiceReady),
    )
    .await;
    assert_eq!(by_kind.emails.len(), 1);
    let other_kind = list_skipped(
        &env,
        INVOICING_ENTITY_ID,
        Some(api::EmailKind::PaymentFailed),
    )
    .await;
    assert!(other_kind.emails.is_empty());

    // Scoped to the invoice's invoicing entity.
    let other_entity = create_entity(&env, "Elsewhere").await;
    assert!(
        list_skipped(&env, other_entity, None)
            .await
            .emails
            .is_empty()
    );
    assert_eq!(sent_rows(&env).await.len(), 0);
}

async fn list_skipped(
    env: &TestEnv,
    entity: InvoicingEntityId,
    kind: Option<api::EmailKind>,
) -> api::ListSkippedEmailsResponse {
    clients(env)
        .await
        .customer_emails
        .list_skipped_emails(api::ListSkippedEmailsRequest {
            invoicing_entity_id: Some(entity.as_proto()),
            kind: kind.map(|k| k as i32),
            cursor: None,
            limit: 10,
        })
        .await
        .expect("list skipped")
        .into_inner()
}

async fn set_test_mode(env: &TestEnv, allowlist: &[&str]) {
    env.store()
        .update_tenant(
            Actor::System,
            TenantUpdate {
                email_delivery_mode: Some(meteroid_store::domain::enums::EmailDeliveryMode::Test),
                email_test_recipients: Some(allowlist.iter().map(|s| s.to_string()).collect()),
                ..Default::default()
            },
            ORGANIZATION_ID,
            TENANT_ID,
        )
        .await
        .expect("test mode");
}

#[rstest]
#[tokio::test]
async fn test_mode_delivers_only_to_the_team_and_the_allowlist(#[future] test_env: TestEnv) {
    let env = test_env.await;
    set_test_mode(&env, &["@qa.example"]).await;
    env.store()
        .update_email_settings(
            Actor::System,
            TENANT_ID,
            INVOICING_ENTITY_ID,
            meteroid_store::domain::customer_emails::EmailSettingsPatch {
                bcc: Some(vec!["audit@acme.example".into(), "books@qa.example".into()]),
                ..Default::default()
            },
        )
        .await
        .expect("settings");

    // A real customer address is withheld; the member and the allowlisted domain get it.
    set_recipients(
        &env,
        CUST_UBER_ID,
        &[RECIPIENT, "demo-user@meteroid.dev", "Tester@QA.example"],
        &[],
    )
    .await;
    pipeline_invoice(&env, CUST_UBER_ID).await;
    let emails = env.drain_email_queue().await;
    assert_eq!(emails.len(), 1);
    let to: Vec<&str> = emails[0].to.iter().map(|r| r.email.as_str()).collect();
    assert_eq!(to, vec!["demo-user@meteroid.dev", "Tester@QA.example"]);
    let bcc: Vec<&str> = emails[0].bcc.iter().map(|r| r.email.as_str()).collect();
    assert_eq!(bcc, vec!["books@qa.example"]);

    // Nobody on the test list: skipped and logged, its claim released.
    set_recipients(&env, CUST_SPOTIFY_ID, &["ap@spotify.example"], &[]).await;
    let withheld = pipeline_invoice(&env, CUST_SPOTIFY_ID).await;
    assert!(env.drain_email_queue().await.is_empty());
    let acts = activities(&env, "invoice", withheld.as_uuid()).await;
    assert_eq!(reasons(&acts, "entity.email_skipped"), vec!["test_mode"]);
    assert_eq!(
        claim(&env, &format!("invoice_ready:{withheld}"))
            .await
            .expect("claim")
            .state,
        EmailSendClaimStateEnum::Failed
    );
}

#[rstest]
#[tokio::test]
async fn only_production_can_go_live_but_live_workspaces_keep_it(#[future] test_env: TestEnv) {
    use meteroid_store::domain::enums::{EmailDeliveryMode, TenantEnvironmentEnum};
    let env = test_env.await;
    let update = |mode: Option<EmailDeliveryMode>, environment: Option<TenantEnvironmentEnum>| {
        env.store().update_tenant(
            Actor::System,
            TenantUpdate {
                // Never an empty changeset.
                name: Some("Renamed".to_string()),
                email_delivery_mode: mode,
                environment,
                ..Default::default()
            },
            ORGANIZATION_ID,
            TENANT_ID,
        )
    };

    // The seeded Development workspace cannot be switched to Live.
    assert!(update(Some(EmailDeliveryMode::Live), None).await.is_err());

    // One already Live (before the rule) keeps it, and other edits leave it Live.
    enable_emails(&env).await;
    let kept = update(None, None).await.expect("unrelated edit");
    assert_eq!(kept.email_delivery_mode, EmailDeliveryMode::Live);
    let re_set = update(Some(EmailDeliveryMode::Live), None)
        .await
        .expect("still live");
    assert_eq!(re_set.email_delivery_mode, EmailDeliveryMode::Live);

    // A client predating the mode can still turn a Live workspace off, and only that.
    let legacy_off = env
        .store()
        .update_tenant(
            Actor::System,
            TenantUpdate {
                name: Some("Renamed".to_string()),
                legacy_disable_emails: Some(true),
                ..Default::default()
            },
            ORGANIZATION_ID,
            TENANT_ID,
        )
        .await
        .expect("legacy disable");
    assert_eq!(legacy_off.email_delivery_mode, EmailDeliveryMode::Off);
    enable_emails(&env).await;

    // Once it leaves Live it cannot come back.
    update(Some(EmailDeliveryMode::Test), None)
        .await
        .expect("to test");
    assert!(update(Some(EmailDeliveryMode::Live), None).await.is_err());

    let bad_list = env
        .store()
        .update_tenant(
            Actor::System,
            TenantUpdate {
                email_test_recipients: Some(vec!["not-an-address".into()]),
                ..Default::default()
            },
            ORGANIZATION_ID,
            TENANT_ID,
        )
        .await;
    assert!(bad_list.is_err());

    // A production workspace starts Live, and drops to Test when it leaves production.
    let prod = env
        .store()
        .insert_tenant(
            meteroid_store::domain::tenants::TenantNew {
                name: "Prod".to_string(),
                environment: TenantEnvironmentEnum::Production,
                email_delivery_mode: None,
                invoicing_entity: None,
            },
            ORGANIZATION_ID,
        )
        .await
        .expect("insert production tenant");
    assert_eq!(prod.email_delivery_mode, EmailDeliveryMode::Live);
    let moved = env
        .store()
        .update_tenant(
            Actor::System,
            TenantUpdate {
                environment: Some(TenantEnvironmentEnum::Staging),
                ..Default::default()
            },
            ORGANIZATION_ID,
            prod.id,
        )
        .await
        .expect("move to staging");
    assert_eq!(moved.email_delivery_mode, EmailDeliveryMode::Test);
}

#[rstest]
#[tokio::test]
async fn payment_failed_sent_only_when_invoice_marked_errored_and_not_customer_initiated(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    enable_emails(&env).await;
    set_kind(&env, EmailKind::PaymentFailed, true).await;
    set_recipients(&env, CUST_UBER_ID, &[RECIPIENT], &[]).await;
    env.seed_payments().await;
    let invoice_id = pipeline_invoice(&env, CUST_UBER_ID).await;
    env.drain_email_queue().await; // the invoice_ready mail
    env.set_mock_charge_behavior(MockChargeBehavior::Pending)
        .await;

    // The customer's own portal attempt fails: the invoice errors, nobody is emailed.
    let (portal_tx, _) = env
        .services()
        .complete_invoice_payment(
            TENANT_ID,
            invoice_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            false,
            None,
        )
        .await
        .expect("portal charge");
    deliver_webhook(&env, failed_payload(portal_tx.id, "pi_portal")).await;
    assert_eq!(
        env.get_customer_invoices(CUST_UBER_ID).await[0].payment_status,
        InvoicePaymentStatus::Errored
    );
    assert!(
        env.drain_email_queue().await.is_empty(),
        "customer-initiated"
    );

    // A system retry fails: exactly one payment_failed, even when the webhook is redelivered.
    let (auto_tx, _) = env
        .services()
        .complete_invoice_payment_automatic(
            TENANT_ID,
            invoice_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            None,
        )
        .await
        .expect("automatic charge");
    deliver_webhook(&env, failed_payload(auto_tx.id, "pi_auto")).await;
    deliver_webhook(&env, failed_payload(auto_tx.id, "pi_auto")).await;
    let emails = env.drain_email_queue().await;
    assert_eq!(emails.len(), 1, "one payment_failed email");
    assert!(emails[0].subject.contains("INV-"), "{}", emails[0].subject);
    assert_eq!(
        emails[0].attachments.len(),
        1,
        "the invoice PDF is attached"
    );
    let rows = sent_rows(&env).await;
    assert_eq!(
        rows.iter()
            .filter(|r| r.kind.as_deref() == Some("payment_failed"))
            .count(),
        1
    );

    // A failure that reaches an already settled invoice changes nothing and emails nobody.
    env.services()
        .mark_invoice_as_paid(
            Actor::System,
            TENANT_ID,
            invoice_id,
            rust_decimal::Decimal::from(35),
            Utc::now().fixed_offset(),
            None,
        )
        .await
        .expect("mark paid");
    env.run_outbox_and_orchestration().await;
    env.drain_email_queue().await;
    let invoice = env.get_customer_invoices(CUST_UBER_ID).await[0].clone();
    assert_eq!(invoice.payment_status, InvoicePaymentStatus::Paid);
    let stale_tx = common_domain::ids::PaymentTransactionId::new();
    {
        let mut conn = env.conn().await;
        diesel_models::payments::PaymentTransactionRowNew {
            id: stale_tx,
            tenant_id: TENANT_ID,
            invoice_id: Some(invoice_id),
            provider_transaction_id: Some("pi_stale".to_string()),
            amount: invoice.total,
            currency: invoice.currency.clone(),
            payment_method_id: Some(CUST_UBER_PAYMENT_METHOD_ID),
            status: diesel_models::enums::PaymentStatusEnum::Pending,
            payment_type: diesel_models::enums::PaymentTypeEnum::Payment,
            error_type: None,
            processed_at: None,
            checkout_session_id: None,
            pending_plan_version_id: None,
            next_action: None,
            initiated_by_customer_id: None,
            pending_provider_intent_id: None,
            pending_connection_id: None,
            parent_transaction_id: None,
            credit_note_id: None,
            refund_mode: None,
            reversal_reason: None,
        }
        .insert(&mut conn)
        .await
        .expect("insert stale attempt");
    }
    deliver_webhook(&env, failed_payload(stale_tx, "pi_stale")).await;
    assert!(env.drain_email_queue().await.is_empty(), "settled invoice");
    assert_eq!(
        env.get_customer_invoices(CUST_UBER_ID).await[0].payment_status,
        InvoicePaymentStatus::Paid
    );
}

#[rstest]
#[tokio::test]
async fn authentication_required_sent_once_when_charge_enters_awaiting_state(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    enable_emails(&env).await;
    set_kind(&env, EmailKind::AuthenticationRequired, true).await;
    set_recipients(&env, CUST_UBER_ID, &[RECIPIENT], &[]).await;
    env.seed_payments().await;
    let invoice_id = pipeline_invoice(&env, CUST_UBER_ID).await;
    env.drain_email_queue().await;
    env.set_mock_charge_behavior(MockChargeBehavior::RequiresAction)
        .await;

    let (tx, _) = env
        .services()
        .complete_invoice_payment_automatic(
            TENANT_ID,
            invoice_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            None,
        )
        .await
        .expect("charge");
    let external_id = tx.provider_transaction_id.clone().expect("provider id");

    // The provider then also announces the challenge by webhook.
    deliver_webhook(
        &env,
        format!(
            r#"{{"id":"evt_ra_{tx}","kind":"payment_requires_action","transaction_id":"{tx}","external_id":"{external_id}","client_secret":"secret"}}"#,
            tx = tx.id.as_base62(),
        ),
    )
    .await;

    let emails = env.drain_email_queue().await;
    assert_eq!(emails.len(), 1, "exactly one authentication_required email");
    assert!(
        emails[0]
            .body_html
            .contains("portal/invoice-payment?token="),
        "resume link"
    );
    assert!(emails[0].attachments.is_empty());
    let rows = sent_rows(&env).await;
    assert_eq!(
        rows.iter()
            .filter(|r| r.kind.as_deref() == Some("authentication_required"))
            .count(),
        1
    );
    let key = format!("authentication_required:{}", tx.id);
    assert_eq!(
        claim(&env, &key).await.expect("claim").state,
        EmailSendClaimStateEnum::Sent
    );
}

#[rstest]
#[tokio::test]
async fn credit_note_issued_attaches_pdf(#[future] test_env: TestEnv) {
    let env = test_env.await;
    enable_emails(&env).await;
    set_kind(&env, EmailKind::CreditNoteIssued, true).await;
    set_customer_extra_locales(&env, &["en-US"]).await;
    set_recipients(&env, CUST_UBER_ID, &[RECIPIENT], &["en-US"]).await;
    let invoice_id = pipeline_invoice(&env, CUST_UBER_ID).await;
    env.drain_email_queue().await;

    let credit_note = env
        .store()
        .create_credit_note(
            Actor::System,
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id,
                line_items: vec![],
                reason: Some("Goodwill".to_string()),
                memo: None,
                credit_type: CreditType::DebtCancellation,
                custom_properties: None,
            },
        )
        .await
        .expect("create credit note");
    let credit_note = env
        .store()
        .finalize_credit_note(Actor::System, TENANT_ID, credit_note.id)
        .await
        .expect("finalize credit note");
    let store = Arc::new(env.store().clone());
    // CreditNoteFinalized -> CreditNotePdfRequest
    run_once_invoice_orchestration(store.clone(), Arc::new(env.services().clone())).await;

    // The PDF is stored ahead of the worker, which then only records and emails it.
    let pdf = env
        .setup
        .object_store
        .store(
            ObjectKind::CreditNotePdf,
            TENANT_ID,
            Bytes::from_static(PLACEHOLDER_PDF),
        )
        .await
        .expect("store pdf");
    env.store()
        .save_credit_note_documents(credit_note.id, TENANT_ID, pdf, None, None)
        .await
        .expect("save credit note documents");
    let pdf_service = Arc::new(
        CreditNotePdfRenderingService::try_new(env.setup.object_store.clone(), store.clone())
            .expect("credit note pdf service"),
    );
    run_once_credit_note_pdf_render(store, pdf_service).await;

    let emails = env.drain_email_queue().await;
    assert_eq!(emails.len(), 1, "one credit_note_issued email");
    let email = &emails[0];
    assert_eq!(
        email.subject,
        format!(
            "Your ACME_UK credit note {}",
            credit_note.credit_note_number
        )
    );
    assert_eq!(email.attachments.len(), 1);
    assert!(email.attachments[0].filename.starts_with("credit_note_"));
    assert_eq!(email.attachments[0].content, PLACEHOLDER_PDF);
    let rows = sent_rows(&env).await;
    let cn_row = rows
        .iter()
        .find(|r| r.kind.as_deref() == Some("credit_note_issued"))
        .expect("credit note row");
    assert_eq!(
        cn_row.dedupe_key.as_deref(),
        Some(format!("credit_note_issued:{}", credit_note.id).as_str())
    );
    let acts = activities(&env, "credit_note", credit_note.id.as_uuid()).await;
    assert!(acts.iter().any(|a| a.activity_type == "entity.email_sent"));
}

/// Prepaid-balance notices carry their figures, go to the invoicing recipients, and honour the
/// entity's per-kind toggle like every other customer email.
#[rstest]
#[tokio::test]
async fn prepaid_balance_notices_render_and_follow_the_kind_toggle(#[future] test_env: TestEnv) {
    let env = test_env.await;
    enable_emails(&env).await;
    set_customer_extra_locales(&env, &["en-US"]).await;
    set_recipients(&env, CUST_UBER_ID, &[RECIPIENT], &["en-US"]).await;
    let wallet_id = WalletId::new();
    let low = |available_minor| EmailContext::WalletBalanceLow {
        customer_id: CUST_UBER_ID,
        wallet_id,
        currency: "EUR".to_string(),
        available_minor,
        threshold_minor: 10_000,
    };
    enqueue(
        &env,
        EmailKind::WalletBalanceLow,
        low(4_250),
        "wallet_balance_low:1",
    )
    .await;
    enqueue(
        &env,
        EmailKind::AutoTopUpFailed,
        EmailContext::AutoTopUpFailed {
            customer_id: CUST_UBER_ID,
            wallet_id,
            operation_id: CreditOperationId::new(),
            currency: "EUR".to_string(),
            amount_minor: 50_000,
            suspended: true,
            payment_reversed: false,
        },
        "auto_top_up_failed:1",
    )
    .await;
    let mut emails = env.drain_email_queue().await;
    emails.sort_by(|a, b| a.subject.cmp(&b.subject));
    let subjects = emails
        .iter()
        .map(|e| e.subject.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        subjects,
        vec![
            "Automatic top-ups of your ACME_UK balance are paused",
            "Your ACME_UK prepaid balance is running low",
        ]
    );
    assert!(
        emails
            .iter()
            .all(|e| e.to.iter().map(|r| r.email.as_str()).eq([RECIPIENT]))
    );
    assert!(emails.iter().all(|e| e.attachments.is_empty()));
    let threshold = meteroid_mailer::render::format_currency(10_000, "EUR");
    assert!(
        emails[1].body_html.contains(&threshold),
        "the threshold is shown"
    );
    let sent = activities(&env, "customer", CUST_UBER_ID.as_uuid()).await;
    assert_eq!(
        sent.iter()
            .filter(|a| a.activity_type == "entity.email_sent")
            .count(),
        2
    );

    set_kind(&env, EmailKind::WalletBalanceLow, false).await;
    enqueue(
        &env,
        EmailKind::WalletBalanceLow,
        low(900),
        "wallet_balance_low:2",
    )
    .await;
    assert!(env.drain_email_queue().await.is_empty(), "kind disabled");
    let skipped = activities(&env, "customer", CUST_UBER_ID.as_uuid()).await;
    assert_eq!(
        reasons(&skipped, "entity.email_skipped"),
        vec!["kind_disabled"]
    );
}

// ---- preview, test send, resend -----------------------------------------------------------

#[rstest]
#[tokio::test]
async fn preview_returns_unknown_variables_and_never_sends(#[future] test_env: TestEnv) {
    let env = test_env.await;
    enable_emails(&env).await;
    let mut client = clients(&env).await.customer_emails;

    let rendered = client
        .preview_email(api::PreviewEmailRequest {
            invoicing_entity_id: INVOICING_ENTITY_ID.as_proto(),
            kind: api::EmailKind::PaymentReminder as i32,
            locale: Some("en-US".into()),
            draft: Some(api::EmailCopy {
                subject: "Reminder for {{customer_name}} {{nope}}".into(),
                body: "Hi **{{customer_name}}**, {{amount_due}} is due {{also_bad}}.".into(),
            }),
            offset_days: Some(7),
        })
        .await
        .expect("preview")
        .into_inner()
        .email
        .expect("email");
    assert_eq!(rendered.unknown_variables, vec!["nope", "also_bad"]);
    assert_eq!(
        rendered.subject, "Reminder for Acme Corp {{nope}}",
        "unknown variables are left for the editor to fix"
    );
    assert!(
        rendered.body_html.contains("<strong>Acme Corp</strong>"),
        "{}",
        rendered.body_html
    );
    assert!(rendered.body_html.contains("INV-2026-0042"));
    assert!(
        !rendered
            .body_html
            .contains("sent from a non-production environment"),
        "on-screen previews leave the test banner out, even on this non-production tenant"
    );
    assert_eq!(rendered.locale, "en-US");
    assert_eq!(rendered.from, "ACME_UK <mailer@example.com>");
    assert_eq!(rendered.attachment_names.len(), 1);

    let german = client
        .preview_email(api::PreviewEmailRequest {
            invoicing_entity_id: INVOICING_ENTITY_ID.as_proto(),
            kind: api::EmailKind::InvoiceReady as i32,
            locale: Some("de-DE".into()),
            draft: None,
            offset_days: None,
        })
        .await
        .expect("preview")
        .into_inner()
        .email
        .expect("email");
    assert_eq!(german.locale, "de-DE");
    assert!(german.body_html.starts_with("<html lang=\"de\">"));
    assert!(german.unknown_variables.is_empty());

    let unsupported = client
        .preview_email(api::PreviewEmailRequest {
            invoicing_entity_id: INVOICING_ENTITY_ID.as_proto(),
            kind: api::EmailKind::InvoiceReady as i32,
            locale: Some("xx-XX".into()),
            draft: None,
            offset_days: None,
        })
        .await
        .expect_err("unsupported locale");
    assert_eq!(unsupported.code(), Code::InvalidArgument);

    assert!(env.drain_email_queue().await.is_empty());
    assert!(sent_rows(&env).await.is_empty());
    assert!(env.mailer.take().is_empty());
}

#[rstest]
#[tokio::test]
async fn preview_invoice_email_needs_invoices_read(#[future] test_env: TestEnv) {
    let env = test_env.await;
    set_recipients(&env, CUST_UBER_ID, &[RECIPIENT], &[]).await;
    let invoice_id = invoice_with_pdf(&env, CUST_UBER_ID).await;

    // A second member whose custom role only reads settings (same password as the seed user).
    let reader = UserId::new();
    let role_id = RoleId::new();
    {
        let mut conn = env.conn().await;
        UserRowNew {
            id: reader,
            email: "reader@meteroid.dev".to_string(),
            password_hash: Some(
                "$argon2id$v=19$m=19456,t=2,p=1$dawIX5+sybNHqfFoNvHFhw$uhtWJd50wiFDV8nR10RNZI4OCrOAJ1kiNZQF0OUSoGE"
                    .to_string(),
            ),
        }
        .insert(&mut conn)
        .await
        .expect("insert user");
        OrganizationMemberRow {
            user_id: reader,
            organization_id: ORGANIZATION_ID,
            role: DbOrganizationUserRole::Member,
            role_id: None,
        }
        .insert(&mut conn)
        .await
        .expect("insert member");
        // Custom-role authoring is EE; core only resolves a stored role.
        use diesel::ExpressionMethods;
        use diesel_async::RunQueryDsl;
        use diesel_models::schema::role::dsl as r;
        diesel::insert_into(r::role)
            .values((
                r::id.eq(role_id),
                r::organization_id.eq(ORGANIZATION_ID),
                r::name.eq("Settings reader"),
                r::permissions.eq(vec![Some(Permission::SettingsRead.code().to_string())]),
                r::is_system.eq(false),
            ))
            .execute(&mut conn)
            .await
            .expect("insert role");
    }
    env.store()
        .assign_member_role(
            ORGANIZATION_ID,
            USER_ID,
            reader,
            OrganizationUserRole::Member,
            Some(role_id),
            meteroid_store::domain::org_security_audit_log::SecurityRequestContext::default(),
        )
        .await
        .expect("assign role");
    let token = AllClients::from_channel(env.setup.channel.clone(), "", "", "")
        .users
        .clone()
        .login(meteroid_grpc::meteroid::api::users::v1::LoginRequest {
            email: "reader@meteroid.dev".to_string(),
            password: meteroid_it::svc_auth::SEED_PASSWORD.to_string(),
        })
        .await
        .expect("login reader")
        .into_inner()
        .token;
    let mut reader_client =
        AllClients::from_channel(env.setup.channel.clone(), &token, "TESTORG", "testslug")
            .customer_emails;

    reader_client
        .preview_email(api::PreviewEmailRequest {
            invoicing_entity_id: INVOICING_ENTITY_ID.as_proto(),
            kind: api::EmailKind::InvoiceReady as i32,
            locale: None,
            draft: None,
            offset_days: None,
        })
        .await
        .expect("settings:read renders synthetic previews");
    let denied = reader_client
        .preview_invoice_email(api::PreviewInvoiceEmailRequest {
            invoice_id: invoice_id.as_proto(),
            kind: api::EmailKind::InvoiceReady as i32,
            locale: None,
            offset_days: None,
        })
        .await
        .expect_err("a real invoice needs invoices:read");
    assert_eq!(denied.code(), Code::PermissionDenied, "{denied}");

    let mut admin = clients(&env).await.customer_emails;
    let rendered = admin
        .preview_invoice_email(api::PreviewInvoiceEmailRequest {
            invoice_id: invoice_id.as_proto(),
            kind: api::EmailKind::InvoiceReady as i32,
            locale: Some("en-US".into()),
            offset_days: None,
        })
        .await
        .expect("admin preview")
        .into_inner()
        .email
        .expect("email");
    assert!(
        rendered.body_html.contains("REM-"),
        "the real invoice number"
    );
    assert_eq!(rendered.locale, "en-US");
    let bad_kind = admin
        .preview_invoice_email(api::PreviewInvoiceEmailRequest {
            invoice_id: invoice_id.as_proto(),
            kind: api::EmailKind::CreditNoteIssued as i32,
            locale: None,
            offset_days: None,
        })
        .await
        .expect_err("not an invoice kind");
    assert_eq!(bad_kind.code(), Code::InvalidArgument);
    assert!(env.mailer.take().is_empty());
}

#[rstest]
#[tokio::test]
async fn send_test_goes_to_actor_without_audit_row(#[future] test_env: TestEnv) {
    let env = test_env.await;
    // Not gated by the tenant switch (still off here).
    let all = clients(&env).await;
    let response = all
        .customer_emails
        .clone()
        .send_test_email(api::SendTestEmailRequest {
            invoicing_entity_id: INVOICING_ENTITY_ID.as_proto(),
            kind: api::EmailKind::InvoiceReady as i32,
            locale: Some("en-US".into()),
            draft: None,
            offset_days: None,
        })
        .await
        .expect("send test")
        .into_inner();
    assert_eq!(response.to, meteroid_it::svc_auth::SEED_USERNAME);
    assert!(!response.delivered, "no SMTP configured in tests");
    let captured = env.mailer.take();
    assert_eq!(captured.len(), 1);
    assert_eq!(
        captured[0].to[0].email,
        meteroid_it::svc_auth::SEED_USERNAME
    );
    assert!(captured[0].subject.starts_with("[Test] "));
    assert!(captured[0].bcc.is_empty());
    assert!(captured[0].attachments.is_empty());
    assert!(sent_rows(&env).await.is_empty());
    assert!(queued_email_messages(&env).await.is_empty());

    // An API key has no mailbox to send to.
    let api_key = all
        .api_tokens
        .clone()
        .create_api_token(
            meteroid_grpc::meteroid::api::apitokens::v1::CreateApiTokenRequest {
                name: "emails".to_string(),
            },
        )
        .await
        .expect("api token")
        .into_inner()
        .api_key;
    let svc = tower::ServiceBuilder::new()
        .layer(SetRequestHeaderLayer::if_not_present(
            HeaderName::from_str(API_KEY_HEADER).unwrap(),
            HeaderValue::from_str(&api_key).unwrap(),
        ))
        .service(env.setup.channel.clone());
    let err = CustomerEmailsServiceClient::new(svc)
        .send_test_email(api::SendTestEmailRequest {
            invoicing_entity_id: INVOICING_ENTITY_ID.as_proto(),
            kind: api::EmailKind::InvoiceReady as i32,
            locale: None,
            draft: None,
            offset_days: None,
        })
        .await
        .expect_err("api key actor");
    assert_eq!(err.code(), Code::FailedPrecondition, "{err}");
}

#[rstest]
#[tokio::test]
async fn resend_rerenders_live_and_links_resent_from(#[future] test_env: TestEnv) {
    let env = test_env.await;
    enable_emails(&env).await;
    set_recipients(&env, CUST_UBER_ID, &[RECIPIENT], &["en-US"]).await;
    let invoice_id = pipeline_invoice(&env, CUST_UBER_ID).await;
    let first = env.drain_email_queue().await;
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].from, "ACME_UK <mailer@example.com>");
    let original = sent_rows(&env).await.remove(0);

    env.store()
        .update_email_settings(
            Actor::System,
            TENANT_ID,
            INVOICING_ENTITY_ID,
            meteroid_store::domain::customer_emails::EmailSettingsPatch {
                sender_name: Some(Some("Acme Billing".into())),
                ..Default::default()
            },
        )
        .await
        .expect("settings");

    let mut client = clients(&env).await.customer_emails;
    client
        .resend_email(api::ResendEmailRequest {
            sent_email_id: original.id.as_proto(),
            request_id: "click-1".into(),
        })
        .await
        .expect("resend");
    let resent = env.drain_email_queue().await;
    assert_eq!(resent.len(), 1);
    assert_eq!(
        resent[0].from, "Acme Billing <mailer@example.com>",
        "re-rendered live"
    );
    assert_eq!(resent[0].attachments.len(), 1);
    let rows = sent_rows(&env).await;
    assert_eq!(rows.len(), 2);
    let copy = rows
        .iter()
        .find(|r| r.id != original.id)
        .expect("resent row");
    assert_eq!(copy.resent_from, Some(original.id));
    assert_eq!(copy.locale, original.locale);
    assert_eq!(
        copy.dedupe_key.as_deref(),
        Some(format!("resend:{}:click-1", original.id.as_base62()).as_str())
    );
    let acts = activities(&env, "invoice", invoice_id.as_uuid()).await;
    let resent_act = acts
        .iter()
        .find(|a| a.activity_type == "entity.email_resent")
        .expect("email_resent activity");
    assert_eq!(
        resent_act.actor_type,
        diesel_models::enums::ActorTypeEnum::User
    );

    // Same click id twice: the second job finds the claim SENT.
    client
        .resend_email(api::ResendEmailRequest {
            sent_email_id: original.id.as_proto(),
            request_id: "click-1".into(),
        })
        .await
        .expect("resend again");
    assert!(env.drain_email_queue().await.is_empty());
    assert_eq!(sent_rows(&env).await.len(), 2);

    let listed = client
        .list_sent_emails(api::ListSentEmailsRequest {
            invoicing_entity_id: Some(INVOICING_ENTITY_ID.as_proto()),
            kind: Some(api::EmailKind::InvoiceReady as i32),
            entity_type: Some("invoice".into()),
            entity_id: Some(invoice_id.as_proto()),
            customer_id: None,
            cursor: None,
            limit: 1,
        })
        .await
        .expect("list")
        .into_inner();
    assert_eq!(listed.emails.len(), 1);
    assert_eq!(
        listed.emails[0].resent_from.as_deref(),
        Some(original.id.as_proto().as_str())
    );
    assert_eq!(
        listed.emails[0].customer_id.as_deref(),
        Some(CUST_UBER_ID.as_proto().as_str())
    );
    let next = listed.next_cursor.expect("a second page");
    let page2 = client
        .list_sent_emails(api::ListSentEmailsRequest {
            invoicing_entity_id: None,
            kind: None,
            entity_type: None,
            entity_id: None,
            customer_id: Some(CUST_UBER_ID.as_proto()),
            cursor: Some(next),
            limit: 10,
        })
        .await
        .expect("page 2")
        .into_inner();
    assert_eq!(page2.emails.len(), 1);
    assert_eq!(page2.emails[0].id, original.id.as_proto());
    assert!(page2.next_cursor.is_none());
    let detail = client
        .get_sent_email(api::GetSentEmailRequest {
            id: copy.id.as_proto(),
        })
        .await
        .expect("get")
        .into_inner()
        .email
        .expect("email");
    assert_eq!(detail.kind, api::EmailKind::InvoiceReady as i32);
    assert_eq!(
        detail.resent_from.as_deref(),
        Some(original.id.as_proto().as_str())
    );
    assert!(detail.attachments_share_key.is_some());
}

// ---- claim state machine ------------------------------------------------------------------

#[rstest]
#[tokio::test]
async fn claim_before_send_prevents_concurrent_double_send(#[future] test_env: TestEnv) {
    let env = test_env.await;
    enable_emails(&env).await;
    set_recipients(&env, CUST_UBER_ID, &[RECIPIENT], &[]).await;
    let invoice_id = invoice_with_pdf(&env, CUST_UBER_ID).await;
    let key = format!("invoice_ready:{invoice_id}");
    enqueue(
        &env,
        EmailKind::InvoiceReady,
        EmailContext::Invoice { invoice_id },
        &key,
    )
    .await;
    let msgs = env
        .store()
        .pgmq_read(
            PgmqQueue::SendEmailRequest,
            MessageReadQty(10),
            MessageReadVtSec(120),
        )
        .await
        .expect("read");
    assert_eq!(msgs.len(), 1);

    env.mailer.set_delay(Some(Duration::from_millis(500)));
    let config = &env.setup.config;
    let handle = || {
        handle_send_email_messages(
            Arc::new(env.store().clone()),
            Arc::new(env.services().clone()),
            env.mailer.clone(),
            env.setup.object_store.clone(),
            config.public_url.clone(),
            config.rest_api_external_url.clone(),
            &msgs,
        )
    };
    let (a, b) = tokio::join!(handle(), handle());
    assert_eq!(
        a.expect("first") + b.expect("second"),
        2,
        "both acked the message"
    );
    env.mailer.set_delay(None);

    assert_eq!(
        env.mailer.take().len(),
        1,
        "one email under a slow transport"
    );
    assert_eq!(sent_rows(&env).await.len(), 1);
    assert_eq!(
        claim(&env, &key).await.expect("claim").state,
        EmailSendClaimStateEnum::Sent
    );
    env.store()
        .pgmq_delete(
            PgmqQueue::SendEmailRequest,
            msgs.iter().map(|m| m.msg_id).collect(),
        )
        .await
        .expect("delete");
}

#[rstest]
#[tokio::test]
async fn stale_claim_becomes_delivery_unknown_not_resend(#[future] test_env: TestEnv) {
    let env = test_env.await;
    enable_emails(&env).await;
    set_recipients(&env, CUST_UBER_ID, &[RECIPIENT], &[]).await;
    let invoice_id = invoice_with_pdf(&env, CUST_UBER_ID).await;
    let key = format!("invoice_ready:{invoice_id}");
    {
        // A sender that died 20 minutes ago, mid-send.
        let mut conn = env.conn().await;
        let claimed_at = Utc::now().naive_utc() - chrono::Duration::minutes(20);
        EmailSendClaimRow::claim(&mut conn, TENANT_ID, &key, claimed_at)
            .await
            .expect("claim")
            .expect("won");
    }
    enqueue(
        &env,
        EmailKind::InvoiceReady,
        EmailContext::Invoice { invoice_id },
        &key,
    )
    .await;

    assert!(
        env.drain_email_queue().await.is_empty(),
        "at-most-once: never resent"
    );
    assert_eq!(
        claim(&env, &key).await.expect("claim").state,
        EmailSendClaimStateEnum::Unknown
    );
    let acts = activities(&env, "invoice", invoice_id.as_uuid()).await;
    assert!(
        acts.iter()
            .any(|a| a.activity_type == "entity.email_delivery_unknown")
    );
    assert!(sent_rows(&env).await.is_empty());

    // UNKNOWN is terminal: a replay is dropped too.
    enqueue(
        &env,
        EmailKind::InvoiceReady,
        EmailContext::Invoice { invoice_id },
        &key,
    )
    .await;
    assert!(env.drain_email_queue().await.is_empty());
    assert_eq!(
        claim(&env, &key).await.expect("claim").state,
        EmailSendClaimStateEnum::Unknown
    );
}

#[rstest]
#[tokio::test]
async fn delivery_unknown_is_sent_again_once_from_its_activity(#[future] test_env: TestEnv) {
    let env = test_env.await;
    enable_emails(&env).await;
    set_recipients(&env, CUST_UBER_ID, &[RECIPIENT], &[]).await;
    let invoice_id = invoice_with_pdf(&env, CUST_UBER_ID).await;
    let key = format!("invoice_ready:{invoice_id}");
    {
        // A sender that died 20 minutes ago, mid-send.
        let mut conn = env.conn().await;
        let claimed_at = Utc::now().naive_utc() - chrono::Duration::minutes(20);
        EmailSendClaimRow::claim(&mut conn, TENANT_ID, &key, claimed_at)
            .await
            .expect("claim")
            .expect("won");
    }
    enqueue(
        &env,
        EmailKind::InvoiceReady,
        EmailContext::Invoice { invoice_id },
        &key,
    )
    .await;
    assert!(env.drain_email_queue().await.is_empty());
    let unknown = activities(&env, "invoice", invoice_id.as_uuid())
        .await
        .into_iter()
        .find(|a| a.activity_type == "entity.email_delivery_unknown")
        .expect("delivery unknown activity")
        .id;
    let send_again = |request_id: &str| api::ResendUnknownEmailRequest {
        activity_id: unknown.as_proto(),
        request_id: request_id.into(),
    };
    let mut client = clients(&env).await.customer_emails;

    // A double click: both calls succeed, one email goes out.
    client
        .resend_unknown_email(send_again("click-1"))
        .await
        .expect("send again");
    client
        .resend_unknown_email(send_again("click-1"))
        .await
        .expect("same click again");
    let sent = env.drain_email_queue().await;
    assert_eq!(sent.len(), 1, "one email for one click");
    assert_eq!(recipients_of(&sent), vec![RECIPIENT.to_string()]);
    let rows = sent_rows(&env).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind.as_deref(), Some("invoice_ready"));
    assert_eq!(
        rows[0].dedupe_key.as_deref(),
        Some(format!("resend_unknown:{}", unknown.as_base62()).as_str())
    );
    let resent: Vec<EntityActivityRow> = activities(&env, "invoice", invoice_id.as_uuid())
        .await
        .into_iter()
        .filter(|a| a.activity_type == "entity.email_resent")
        .collect();
    assert_eq!(resent.len(), 1, "one operator action recorded");
    assert_eq!(
        resent[0].actor_type,
        diesel_models::enums::ActorTypeEnum::User
    );

    // The same click retried after delivery is still a no-op success.
    client
        .resend_unknown_email(send_again("click-1"))
        .await
        .expect("retried click");
    assert!(env.drain_email_queue().await.is_empty());

    // A new click once the email went out is refused rather than duplicating it.
    let err = client
        .resend_unknown_email(send_again("click-2"))
        .await
        .expect_err("sent since");
    assert_eq!(err.code(), Code::FailedPrecondition, "{err}");
    assert!(env.drain_email_queue().await.is_empty());
    assert_eq!(sent_rows(&env).await.len(), 1);

    // Only a delivery-unknown entry can be sent again (a sent email shares its activity's id).
    let err = client
        .resend_unknown_email(api::ResendUnknownEmailRequest {
            activity_id: rows[0].id.as_proto(),
            request_id: "click-3".into(),
        })
        .await
        .expect_err("not a delivery-unknown entry");
    assert_eq!(err.code(), Code::FailedPrecondition, "{err}");
}

#[rstest]
#[tokio::test]
async fn in_flight_claim_is_looked_at_again_not_acked(#[future] test_env: TestEnv) {
    let env = test_env.await;
    enable_emails(&env).await;
    set_recipients(&env, CUST_UBER_ID, &[RECIPIENT], &[]).await;
    let invoice_id = invoice_with_pdf(&env, CUST_UBER_ID).await;
    let key = format!("invoice_ready:{invoice_id}");
    {
        // A sender that won the claim 2 minutes ago and died before finishing it.
        let mut conn = env.conn().await;
        let claimed_at = Utc::now().naive_utc() - chrono::Duration::minutes(2);
        EmailSendClaimRow::claim(&mut conn, TENANT_ID, &key, claimed_at)
            .await
            .expect("claim")
            .expect("won");
    }
    enqueue(
        &env,
        EmailKind::InvoiceReady,
        EmailContext::Invoice { invoice_id },
        &key,
    )
    .await;

    // Still inside the in-flight window: nothing is sent, nothing is decided, and the message
    // is put back to be looked at once the claim can only be finished or stale.
    assert!(env.drain_email_queue().await.is_empty());
    assert_eq!(
        claim(&env, &key).await.expect("claim").state,
        EmailSendClaimStateEnum::Sending
    );
    let queued = queued_email_messages(&env).await;
    assert_eq!(queued.len(), 1, "re-enqueued, not acked");
    assert_eq!(queued[0].0.in_flight, 1, "in-flight look counted");
    assert!(
        queued[0].1 > 15 * 60,
        "visible after the in-flight window, got {}s",
        queued[0].1
    );
    assert!(
        activities(&env, "invoice", invoice_id.as_uuid())
            .await
            .iter()
            .all(|a| a.activity_type != "entity.email_delivery_unknown")
    );

    // The window elapses with the claim still SENDING: the delayed pass marks it UNKNOWN.
    {
        use diesel_async::RunQueryDsl;
        let mut conn = env.conn().await;
        diesel::sql_query(
            "UPDATE email_send_claim SET claimed_at = claimed_at - interval '16 minutes' \
             WHERE dedupe_key = $1",
        )
        .bind::<diesel::sql_types::Text, _>(&key)
        .execute(&mut conn)
        .await
        .expect("age the claim");
    }
    make_queue_visible(&env).await;
    assert!(
        env.drain_email_queue().await.is_empty(),
        "at-most-once: never resent"
    );
    assert_eq!(
        claim(&env, &key).await.expect("claim").state,
        EmailSendClaimStateEnum::Unknown
    );
    assert!(
        activities(&env, "invoice", invoice_id.as_uuid())
            .await
            .iter()
            .any(|a| a.activity_type == "entity.email_delivery_unknown")
    );
    assert!(sent_rows(&env).await.is_empty());
    assert!(queued_email_messages(&env).await.is_empty());
}

#[rstest]
#[tokio::test]
async fn deferred_message_reclaims_after_release(#[future] test_env: TestEnv) {
    let env = test_env.await;
    enable_emails(&env).await;
    set_kind(&env, EmailKind::PaymentReminder, true).await;
    set_customer_extra_locales(&env, &["en-US"]).await;
    set_recipients(&env, CUST_UBER_ID, &[RECIPIENT], &["en-US"]).await;
    let invoice_id = invoice_with_pdf(&env, CUST_UBER_ID).await;
    let sequence = env
        .create_sequence(INVOICING_ENTITY_ID, &[0, 7], true)
        .await;
    let row = env.enroll_invoice(invoice_id, sequence.id).await;
    let context = EmailContext::PaymentReminder {
        invoice_id,
        sequence_id: sequence.id,
        offset_days: 0,
        generation: row.generation,
        manual: false,
    };
    let key = format!("payment_reminder:{invoice_id}:{}:0", row.generation);

    // A SEPA debit is in flight: the step is deferred, the claim released.
    {
        let mut conn = env.conn().await;
        InvoiceRow::apply_payment_status(
            &mut conn,
            invoice_id,
            TENANT_ID,
            diesel_models::enums::InvoicePaymentStatus::Processing,
            None,
        )
        .await
        .expect("processing");
    }
    enqueue(&env, EmailKind::PaymentReminder, context.clone(), &key).await;
    assert!(run_email_worker_once(&env).await.is_empty());
    assert_eq!(
        claim(&env, &key).await.expect("claim").state,
        EmailSendClaimStateEnum::Failed
    );
    let queued = queued_email_messages(&env).await;
    assert_eq!(queued.len(), 1);
    assert_eq!(
        queued[0].0,
        EmailSendAttempts {
            payment_pending: 1,
            ..Default::default()
        },
        "only the pending-payment budget is spent"
    );
    assert!(
        queued[0].1 > 3000,
        "visible in about an hour, got {}s",
        queued[0].1
    );
    assert!(
        activities(&env, "invoice", invoice_id.as_uuid())
            .await
            .iter()
            .all(|a| a.activity_type != "payment_reminder.skipped")
    );

    // The debit bounced; the retried message wins the released claim and sends.
    {
        let mut conn = env.conn().await;
        InvoiceRow::apply_payment_status(
            &mut conn,
            invoice_id,
            TENANT_ID,
            diesel_models::enums::InvoicePaymentStatus::Unpaid,
            None,
        )
        .await
        .expect("unpaid");
    }
    make_queue_visible(&env).await;
    let emails = env.drain_email_queue().await;
    assert_eq!(emails.len(), 1);
    assert!(
        emails[0].subject.starts_with("Invoice REM-"),
        "{}",
        emails[0].subject
    );
    assert!(emails[0].subject.ends_with(" day 0"));
    assert!(emails[0].body_html.contains("Please pay"));
    assert!(
        emails[0].body_html.contains("please disregard"),
        "hedge line"
    );
    assert_eq!(emails[0].attachments.len(), 1);
    assert_eq!(
        claim(&env, &key).await.expect("claim").state,
        EmailSendClaimStateEnum::Sent
    );
    let rows = sent_rows(&env).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind.as_deref(), Some("payment_reminder"));

    // A replay of the original message is a no-op.
    enqueue(&env, EmailKind::PaymentReminder, context, &key).await;
    assert!(env.drain_email_queue().await.is_empty());
    assert_eq!(sent_rows(&env).await.len(), 1);
}

#[rstest]
#[tokio::test]
async fn transport_error_reenqueues_with_backoff_then_records_skip(#[future] test_env: TestEnv) {
    let env = test_env.await;
    enable_emails(&env).await;
    set_recipients(&env, CUST_UBER_ID, &[RECIPIENT], &[]).await;
    let invoice_id = invoice_with_pdf(&env, CUST_UBER_ID).await;
    let key = format!("invoice_ready:{invoice_id}");

    env.mailer.fail_next(1);
    enqueue(
        &env,
        EmailKind::InvoiceReady,
        EmailContext::Invoice { invoice_id },
        &key,
    )
    .await;
    assert!(run_email_worker_once(&env).await.is_empty());
    assert_eq!(
        claim(&env, &key).await.expect("claim").state,
        EmailSendClaimStateEnum::Failed
    );
    let queued = queued_email_messages(&env).await;
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].0.transport, 1);
    assert!(
        (30..=60).contains(&queued[0].1),
        "first retry in a minute, got {}s",
        queued[0].1
    );
    assert!(sent_rows(&env).await.is_empty());

    // The last allowed attempt fails too: recorded as a skip, nothing re-enqueued.
    let exhausted_key = format!("invoice_paid:{invoice_id}");
    enqueue_raw(
        &env,
        EmailContext::Invoice { invoice_id },
        &exhausted_key,
        None,
        EmailSendAttempts {
            transport: 30,
            ..Default::default()
        },
    )
    .await;
    env.mailer.fail_next(1);
    assert!(run_email_worker_once(&env).await.is_empty());
    assert_eq!(
        claim(&env, &exhausted_key).await.expect("claim").state,
        EmailSendClaimStateEnum::Failed
    );
    let acts = activities(&env, "invoice", invoice_id.as_uuid()).await;
    assert_eq!(reasons(&acts, "entity.email_skipped"), vec!["send_failed"]);
    assert_eq!(
        queued_email_messages(&env).await.len(),
        1,
        "only the first retry remains"
    );

    // The transport recovers: the delayed retry sends once.
    make_queue_visible(&env).await;
    let emails = env.drain_email_queue().await;
    assert_eq!(emails.len(), 1);
    assert_eq!(
        claim(&env, &key).await.expect("claim").state,
        EmailSendClaimStateEnum::Sent
    );
    assert_eq!(sent_rows(&env).await.len(), 1);
}

async fn enqueue_raw(
    env: &TestEnv,
    context: EmailContext,
    key: &str,
    resent_from: Option<EntityActivityId>,
    attempts: EmailSendAttempts,
) {
    let msg = SendEmailRequest::CustomerEmail {
        tenant_id: TENANT_ID,
        kind: EmailKind::InvoiceReady,
        context,
        dedupe_key: key.to_string(),
        resent_from,
        manual: false,
        attempts,
        recipient: None,
    }
    .try_into()
    .expect("message");
    env.store()
        .pgmq_send_batch(PgmqQueue::SendEmailRequest, vec![msg])
        .await
        .expect("enqueue");
}

#[rstest]
#[tokio::test]
async fn retry_budgets_are_per_reason(#[future] test_env: TestEnv) {
    let env = test_env.await;
    enable_emails(&env).await;
    set_recipients(&env, CUST_UBER_ID, &[RECIPIENT], &[]).await;
    let invoice_id = invoice_with_pdf(&env, CUST_UBER_ID).await;
    let key = format!("invoice_ready:{invoice_id}");

    // Long deferred for a pending payment: a first transport error is still retried.
    let deferred = EmailSendAttempts {
        payment_pending: 40,
        in_flight: 3,
        ..Default::default()
    };
    enqueue_raw(
        &env,
        EmailContext::Invoice { invoice_id },
        &key,
        None,
        deferred,
    )
    .await;
    env.mailer.fail_next(1);
    assert!(run_email_worker_once(&env).await.is_empty());
    let queued = queued_email_messages(&env).await;
    assert_eq!(queued.len(), 1, "re-enqueued, not skipped");
    assert_eq!(
        queued[0].0,
        EmailSendAttempts {
            transport: 1,
            ..deferred
        }
    );
    assert!(
        reasons(
            &activities(&env, "invoice", invoice_id.as_uuid()).await,
            "entity.email_skipped"
        )
        .is_empty()
    );
    make_queue_visible(&env).await;
    assert_eq!(env.drain_email_queue().await.len(), 1);
}

#[rstest]
#[tokio::test]
async fn in_flight_give_up_never_marks_a_fresh_claim_unknown(#[future] test_env: TestEnv) {
    let env = test_env.await;
    enable_emails(&env).await;
    set_recipients(&env, CUST_UBER_ID, &[RECIPIENT], &[]).await;
    let invoice_id = invoice_with_pdf(&env, CUST_UBER_ID).await;
    let key = format!("invoice_ready:{invoice_id}");
    {
        // Another sender won the claim a minute ago and is still sending.
        let mut conn = env.conn().await;
        let claimed_at = Utc::now().naive_utc() - chrono::Duration::minutes(1);
        EmailSendClaimRow::claim(&mut conn, TENANT_ID, &key, claimed_at)
            .await
            .expect("claim")
            .expect("won");
    }
    // A duplicate out of in-flight looks is dropped: the owner is left alone.
    enqueue_raw(
        &env,
        EmailContext::Invoice { invoice_id },
        &key,
        None,
        EmailSendAttempts {
            in_flight: 8,
            ..Default::default()
        },
    )
    .await;
    assert!(env.drain_email_queue().await.is_empty());
    assert!(queued_email_messages(&env).await.is_empty(), "acked");
    assert_eq!(
        claim(&env, &key).await.expect("claim").state,
        EmailSendClaimStateEnum::Sending
    );
    assert!(
        activities(&env, "invoice", invoice_id.as_uuid())
            .await
            .iter()
            .all(|a| a.activity_type != "entity.email_delivery_unknown")
    );

    // The owner finishes: SENDING is the only state `finish` leaves, UNKNOWN stays terminal.
    {
        let mut conn = env.conn().await;
        let now = Utc::now().naive_utc();
        let stale_before = now - chrono::Duration::minutes(15);
        assert_eq!(
            EmailSendClaimRow::mark_unknown(&mut conn, TENANT_ID, &key, stale_before, now)
                .await
                .expect("mark unknown"),
            0,
            "a fresh claim is not stale"
        );
        EmailSendClaimRow::finish(
            &mut conn,
            TENANT_ID,
            &key,
            EmailSendClaimStateEnum::Sent,
            None,
            now,
        )
        .await
        .expect("finish");
        assert_eq!(
            EmailSendClaimRow::finish(
                &mut conn,
                TENANT_ID,
                &key,
                EmailSendClaimStateEnum::Failed,
                None,
                now,
            )
            .await
            .expect("finish again"),
            0,
            "a SENT claim is never released"
        );
    }
    assert_eq!(
        claim(&env, &key).await.expect("claim").state,
        EmailSendClaimStateEnum::Sent
    );
}

#[rstest]
#[tokio::test]
async fn failed_resend_lookup_leaves_no_claim(#[future] test_env: TestEnv) {
    let env = test_env.await;
    enable_emails(&env).await;
    set_recipients(&env, CUST_UBER_ID, &[RECIPIENT], &[]).await;
    let invoice_id = invoice_with_pdf(&env, CUST_UBER_ID).await;
    let key = format!("invoice_ready:{invoice_id}:resend");
    // The resent email does not exist: the lookup fails before anything is claimed.
    enqueue_raw(
        &env,
        EmailContext::Invoice { invoice_id },
        &key,
        Some(EntityActivityId::new()),
        EmailSendAttempts::default(),
    )
    .await;
    assert!(run_email_worker_once(&env).await.is_empty());
    assert!(
        claim(&env, &key).await.is_none(),
        "no claim left to go stale"
    );
}

#[rstest]
#[tokio::test]
async fn test_recipients_are_dropped_and_the_rest_still_receive(#[future] test_env: TestEnv) {
    let env = test_env.await;
    enable_emails(&env).await;
    set_recipients(&env, CUST_UBER_ID, &["qa@uber.test", RECIPIENT], &[]).await;
    let invoice_id = invoice_with_pdf(&env, CUST_UBER_ID).await;
    let key = format!("invoice_ready:{invoice_id}");
    enqueue(
        &env,
        EmailKind::InvoiceReady,
        EmailContext::Invoice { invoice_id },
        &key,
    )
    .await;
    let emails = env.drain_email_queue().await;
    assert_eq!(emails.len(), 1);
    assert_eq!(recipients_of(&emails), vec![RECIPIENT.to_string()]);
    let rows = sent_rows(&env).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].recipients, vec![Some(RECIPIENT.to_string())]);
}

// ---- "Send to" audience rules -------------------------------------------------------------

const SPOTIFY_RECIPIENT: &str = "ap@spotify.example";

fn customers_audience(ids: &[CustomerId]) -> String {
    let ids: Vec<String> = ids.iter().map(|c| c.as_base62()).collect();
    serde_json::json!({ "predicates": [{ "customer": { "customer_ids": ids } }] }).to_string()
}

fn invoice_kind_audience(kind: &str) -> String {
    serde_json::json!({ "predicates": [{ "attribute": { "key": "invoice_kind", "values": [kind] } }] })
        .to_string()
}

async fn set_audience(
    client: &mut CustomerEmailsServiceClient<meteroid_it::clients::TestLayeredClientService>,
    entity: InvoicingEntityId,
    kind: api::EmailKind,
    mode: api::EmailAudienceMode,
    audience: String,
) -> Result<api::EmailSettings, tonic::Status> {
    client
        .set_email_kind_audience(api::SetEmailKindAudienceRequest {
            invoicing_entity_id: entity.as_proto(),
            kind: kind as i32,
            mode: mode as i32,
            audience,
        })
        .await
        .map(|r| r.into_inner().settings.expect("settings"))
}

fn kind_audience(
    settings: &api::EmailSettings,
    kind: api::EmailKind,
) -> Option<api::EmailKindAudience> {
    settings
        .kinds
        .iter()
        .find(|k| k.kind == kind as i32)
        .and_then(|k| k.audience.clone())
}

fn recipients_of(emails: &[Email]) -> Vec<String> {
    emails
        .iter()
        .flat_map(|e| e.to.iter().map(|r| r.email.clone()))
        .collect()
}

/// Unscoped on purpose: proves nothing was written, whichever tenant the entity belongs to.
async fn stored_audience_rows(env: &TestEnv, entity: InvoicingEntityId) -> i64 {
    use diesel_async::RunQueryDsl;
    #[derive(diesel::QueryableByName)]
    struct Row {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        count: i64,
    }
    let mut conn = env.conn().await;
    diesel::sql_query(
        "SELECT count(*) AS count FROM email_kind_audience WHERE invoicing_entity_id = $1",
    )
    .bind::<diesel::sql_types::Uuid, _>(entity.as_uuid())
    .get_result::<Row>(&mut conn)
    .await
    .expect("count audience rows")
    .count
}

async fn two_customers_with_recipients(env: &TestEnv) {
    enable_emails(env).await;
    set_recipients(env, CUST_UBER_ID, &[RECIPIENT], &["en-US"]).await;
    set_recipients(env, CUST_SPOTIFY_ID, &[SPOTIFY_RECIPIENT], &["en-US"]).await;
}

#[rstest]
#[tokio::test]
async fn exclude_audience_skips_matching_customer_only(#[future] test_env: TestEnv) {
    let env = test_env.await;
    two_customers_with_recipients(&env).await;
    let mut client = clients(&env).await.customer_emails;

    let settings = set_audience(
        &mut client,
        INVOICING_ENTITY_ID,
        api::EmailKind::InvoiceReady,
        api::EmailAudienceMode::Exclude,
        customers_audience(&[CUST_UBER_ID]),
    )
    .await
    .expect("set audience");
    let rule = kind_audience(&settings, api::EmailKind::InvoiceReady).expect("rule");
    assert_eq!(rule.mode, api::EmailAudienceMode::Exclude as i32);
    assert!(kind_audience(&settings, api::EmailKind::InvoicePaid).is_none());
    assert!(
        settings
            .kinds
            .iter()
            .find(|k| k.kind == api::EmailKind::QuoteReady as i32)
            .is_some_and(|k| k.customer_level_audience)
    );

    let uber_invoice = pipeline_invoice(&env, CUST_UBER_ID).await;
    let spotify_invoice = pipeline_invoice(&env, CUST_SPOTIFY_ID).await;
    let emails = env.drain_email_queue().await;
    assert_eq!(recipients_of(&emails), vec![SPOTIFY_RECIPIENT]);
    assert_eq!(sent_rows(&env).await.len(), 1);

    let uber_acts = activities(&env, "invoice", uber_invoice.as_uuid()).await;
    assert_eq!(
        reasons(&uber_acts, "entity.email_skipped"),
        vec!["audience_excluded"]
    );
    let spotify_acts = activities(&env, "invoice", spotify_invoice.as_uuid()).await;
    assert!(reasons(&spotify_acts, "entity.email_skipped").is_empty());

    // The invoice preview still renders but says the rule would hold it back.
    for (invoice, suppressed) in [(uber_invoice, true), (spotify_invoice, false)] {
        let preview = client
            .preview_invoice_email(api::PreviewInvoiceEmailRequest {
                invoice_id: invoice.as_proto(),
                kind: api::EmailKind::InvoiceReady as i32,
                locale: None,
                offset_days: None,
            })
            .await
            .expect("preview")
            .into_inner();
        assert!(preview.email.is_some());
        assert_eq!(preview.suppressed_by_audience, suppressed, "{invoice}");
    }

    let preview = client
        .preview_email_kind_audience(api::PreviewEmailKindAudienceRequest {
            invoicing_entity_id: INVOICING_ENTITY_ID.as_proto(),
            kind: api::EmailKind::InvoiceReady as i32,
            mode: api::EmailAudienceMode::Exclude as i32,
            audience: customers_audience(&[CUST_UBER_ID]),
        })
        .await
        .expect("preview audience")
        .into_inner()
        .preview
        .expect("preview");
    assert!(preview.matched_invoices >= 1, "{preview:?}");
    assert_eq!(preview.matched_customers, 1);
    assert_eq!(
        preview.receiving_invoices,
        preview.scanned_invoices - preview.matched_invoices
    );
    assert!(preview.receiving_customers >= 1);

    let audit = activities(&env, "invoicing_entity", INVOICING_ENTITY_ID.as_uuid()).await;
    assert!(audit.iter().any(|a| {
        a.metadata.as_ref().is_some_and(|m| {
            m["changes"][0]["field"] == "email_settings.audience.invoice_ready"
                && m["changes"][0]["to"] == "EXCLUDE"
        })
    }));
}

#[rstest]
#[tokio::test]
async fn only_audience_sends_to_matching_customer_only(#[future] test_env: TestEnv) {
    let env = test_env.await;
    two_customers_with_recipients(&env).await;
    let mut client = clients(&env).await.customer_emails;
    set_audience(
        &mut client,
        INVOICING_ENTITY_ID,
        api::EmailKind::InvoiceReady,
        api::EmailAudienceMode::Only,
        customers_audience(&[CUST_UBER_ID]),
    )
    .await
    .expect("set audience");

    let uber_invoice = pipeline_invoice(&env, CUST_UBER_ID).await;
    let spotify_invoice = pipeline_invoice(&env, CUST_SPOTIFY_ID).await;
    let emails = env.drain_email_queue().await;
    assert_eq!(recipients_of(&emails), vec![RECIPIENT]);

    let spotify_acts = activities(&env, "invoice", spotify_invoice.as_uuid()).await;
    assert_eq!(
        reasons(&spotify_acts, "entity.email_skipped"),
        vec!["audience_excluded"]
    );
    let uber_acts = activities(&env, "invoice", uber_invoice.as_uuid()).await;
    assert!(reasons(&uber_acts, "entity.email_skipped").is_empty());
}

#[rstest]
#[tokio::test]
async fn rule_is_read_at_send_time_and_clearing_restores_everyone(#[future] test_env: TestEnv) {
    let env = test_env.await;
    two_customers_with_recipients(&env).await;
    let mut client = clients(&env).await.customer_emails;
    set_audience(
        &mut client,
        INVOICING_ENTITY_ID,
        api::EmailKind::InvoiceReady,
        api::EmailAudienceMode::Exclude,
        customers_audience(&[CUST_UBER_ID]),
    )
    .await
    .expect("set audience");
    pipeline_invoice(&env, CUST_UBER_ID).await;
    assert!(env.drain_email_queue().await.is_empty());

    // Queued while Uber was excluded, sent after the rule changed: the send reads the live rule.
    let queued = pipeline_invoice(&env, CUST_UBER_ID).await;
    set_audience(
        &mut client,
        INVOICING_ENTITY_ID,
        api::EmailKind::InvoiceReady,
        api::EmailAudienceMode::Exclude,
        customers_audience(&[CUST_SPOTIFY_ID]),
    )
    .await
    .expect("change audience");
    let audit = activities(&env, "invoicing_entity", INVOICING_ENTITY_ID.as_uuid()).await;
    let conditions_only = audit.iter().any(|a| {
        a.metadata.as_ref().is_some_and(|m| {
            m["changes"].as_array().is_some_and(|c| c.len() == 1)
                && m["changes"][0]["field"] == "email_settings.audience.invoice_ready.conditions"
                && m["changes"][0]["to"]["predicates"].is_array()
        })
    });
    assert!(conditions_only, "same mode, new conditions is audited");
    assert_eq!(
        recipients_of(&env.drain_email_queue().await),
        vec![RECIPIENT]
    );
    let acts = activities(&env, "invoice", queued.as_uuid()).await;
    assert!(reasons(&acts, "entity.email_skipped").is_empty());

    pipeline_invoice(&env, CUST_SPOTIFY_ID).await;
    assert!(
        env.drain_email_queue().await.is_empty(),
        "Spotify now excluded"
    );

    let cleared = client
        .clear_email_kind_audience(api::ClearEmailKindAudienceRequest {
            invoicing_entity_id: INVOICING_ENTITY_ID.as_proto(),
            kind: api::EmailKind::InvoiceReady as i32,
        })
        .await
        .expect("clear")
        .into_inner()
        .settings
        .expect("settings");
    assert!(kind_audience(&cleared, api::EmailKind::InvoiceReady).is_none());
    assert_eq!(stored_audience_rows(&env, INVOICING_ENTITY_ID).await, 0);
    pipeline_invoice(&env, CUST_SPOTIFY_ID).await;
    assert_eq!(
        recipients_of(&env.drain_email_queue().await),
        vec![SPOTIFY_RECIPIENT]
    );

    let audit = activities(&env, "invoicing_entity", INVOICING_ENTITY_ID.as_uuid()).await;
    let cleared_audit = audit.iter().any(|a| {
        a.metadata.as_ref().is_some_and(|m| {
            m["changes"][0]["field"] == "email_settings.audience.invoice_ready"
                && m["changes"][0]["to"].is_null()
        })
    });
    assert!(cleared_audit, "clearing is audited");
}

#[rstest]
#[tokio::test]
async fn unreadable_stored_rules_are_ignored(#[future] test_env: TestEnv) {
    use diesel_async::RunQueryDsl;
    let env = test_env.await;
    two_customers_with_recipients(&env).await;
    let mut conn = env.conn().await;
    diesel::sql_query(
        "INSERT INTO email_kind_audience (invoicing_entity_id, tenant_id, kind, mode, audience) VALUES \
         ($1, $2, 'retired_kind', 'EXCLUDE', '{\"predicates\": []}'), \
         ($1, $2, 'invoice_ready', 'EXCLUDE', '{\"predicates\": [{\"bogus\": {}}]}')",
    )
    .bind::<diesel::sql_types::Uuid, _>(INVOICING_ENTITY_ID.as_uuid())
    .bind::<diesel::sql_types::Uuid, _>(TENANT_ID.as_uuid())
    .execute(&mut conn)
    .await
    .expect("insert unreadable rules");

    let mut client = clients(&env).await.customer_emails;
    let settings = client
        .get_email_settings(api::GetEmailSettingsRequest {
            invoicing_entity_id: Some(INVOICING_ENTITY_ID.as_proto()),
        })
        .await
        .expect("settings still load")
        .into_inner()
        .settings
        .expect("settings");
    assert!(kind_audience(&settings, api::EmailKind::InvoiceReady).is_none());

    pipeline_invoice(&env, CUST_UBER_ID).await;
    assert_eq!(
        recipients_of(&env.drain_email_queue().await),
        vec![RECIPIENT],
        "an unreadable rule sends to everyone instead of retrying"
    );
}

#[rstest]
#[tokio::test]
async fn resend_bypasses_the_audience_rule(#[future] test_env: TestEnv) {
    let env = test_env.await;
    two_customers_with_recipients(&env).await;
    pipeline_invoice(&env, CUST_UBER_ID).await;
    assert_eq!(env.drain_email_queue().await.len(), 1);
    let original = sent_rows(&env).await.remove(0);

    let mut client = clients(&env).await.customer_emails;
    set_audience(
        &mut client,
        INVOICING_ENTITY_ID,
        api::EmailKind::InvoiceReady,
        api::EmailAudienceMode::Exclude,
        customers_audience(&[CUST_UBER_ID]),
    )
    .await
    .expect("set audience");
    client
        .resend_email(api::ResendEmailRequest {
            sent_email_id: original.id.as_proto(),
            request_id: "click-1".into(),
        })
        .await
        .expect("resend");
    assert_eq!(
        recipients_of(&env.drain_email_queue().await),
        vec![RECIPIENT],
        "an explicit resend is not filtered"
    );
}

#[rstest]
#[tokio::test]
async fn credit_note_rule_is_evaluated_on_its_invoice(#[future] test_env: TestEnv) {
    let env = test_env.await;
    two_customers_with_recipients(&env).await;
    set_kind(&env, EmailKind::CreditNoteIssued, true).await;
    let invoice_id = pipeline_invoice(&env, CUST_UBER_ID).await;
    env.drain_email_queue().await;

    let credit_note = env
        .store()
        .create_credit_note(
            Actor::System,
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id,
                line_items: vec![],
                reason: Some("Goodwill".to_string()),
                memo: None,
                credit_type: CreditType::DebtCancellation,
                custom_properties: None,
            },
        )
        .await
        .expect("create credit note");
    env.store()
        .finalize_credit_note(Actor::System, TENANT_ID, credit_note.id)
        .await
        .expect("finalize credit note");
    let pdf = env
        .setup
        .object_store
        .store(
            ObjectKind::CreditNotePdf,
            TENANT_ID,
            Bytes::from_static(PLACEHOLDER_PDF),
        )
        .await
        .expect("store pdf");
    env.store()
        .save_credit_note_documents(credit_note.id, TENANT_ID, pdf, None, None)
        .await
        .expect("save credit note documents");

    // `invoice_kind` only exists on the invoice: the credit note borrows its invoice's facts.
    let mut client = clients(&env).await.customer_emails;
    set_audience(
        &mut client,
        INVOICING_ENTITY_ID,
        api::EmailKind::CreditNoteIssued,
        api::EmailAudienceMode::Exclude,
        invoice_kind_audience("subscription"),
    )
    .await
    .expect("set audience");
    let context = EmailContext::CreditNote {
        credit_note_id: credit_note.id,
    };
    enqueue(
        &env,
        EmailKind::CreditNoteIssued,
        context.clone(),
        &format!("credit_note_issued:{}", credit_note.id),
    )
    .await;
    assert!(env.drain_email_queue().await.is_empty());
    let acts = activities(&env, "credit_note", credit_note.id.as_uuid()).await;
    assert_eq!(
        reasons(&acts, "entity.email_skipped"),
        vec!["audience_excluded"]
    );

    set_audience(
        &mut client,
        INVOICING_ENTITY_ID,
        api::EmailKind::CreditNoteIssued,
        api::EmailAudienceMode::Only,
        invoice_kind_audience("subscription"),
    )
    .await
    .expect("flip audience");
    enqueue(
        &env,
        EmailKind::CreditNoteIssued,
        context,
        &format!("credit_note_issued:{}:again", credit_note.id),
    )
    .await;
    let emails = env.drain_email_queue().await;
    assert_eq!(recipients_of(&emails), vec![RECIPIENT]);
    assert!(
        emails[0].attachments[0]
            .filename
            .starts_with("credit_note_")
    );
}

async fn create_quote_for(env: &TestEnv, customer: CustomerId) -> QuoteId {
    use meteroid_grpc::meteroid::api::quotes::v1 as quotes;
    use meteroid_grpc::meteroid::api::subscriptions::v1 as subs;
    let quote = clients(env)
        .await
        .quotes
        .create_quote(quotes::CreateQuoteRequest {
            quote: Some(quotes::CreateQuote {
                plan_version_id: PLAN_VERSION_NOTION_ID.as_proto(),
                customer_id: customer.as_proto(),
                currency: "EUR".to_string(),
                quote_number: None,
                trial_duration: None,
                start_date: Some(Utc::now().date_naive().to_string()),
                billing_start_date: None,
                end_date: None,
                billing_day_anchor: Some(1),
                activation_condition: subs::ActivationCondition::OnStart.into(),
                valid_until: None,
                expires_at: None,
                internal_notes: None,
                cover_image: None,
                overview: None,
                terms_and_services: None,
                net_terms: Some(30),
                attachments: vec![],
                usage_examples: vec![],
                minimum_commitment_override: None,
                recipients: vec![quotes::RecipientDetails {
                    name: "Accounts".to_string(),
                    email: SPOTIFY_RECIPIENT.to_string(),
                }],
                components: Some(subs::CreateSubscriptionComponents {
                    parameterized_components: vec![
                        subs::create_subscription_components::ComponentParameterization {
                            component_id: COMP_NOTION_SEATS_ID.as_proto(),
                            billing_period: Some(
                                meteroid_grpc::meteroid::api::shared::v1::BillingPeriod::Monthly
                                    .into(),
                            ),
                            initial_slot_count: Some(5),
                            committed_capacity: None,
                        },
                    ],
                    ..Default::default()
                }),
                auto_advance_invoices: None,
                charge_automatically: None,
                invoice_memo: None,
                invoice_threshold: None,
                create_subscription_on_acceptance: None,
                add_ons: None,
                coupons: None,
                payment_methods_config: None,
                entitlements: vec![],
                custom_properties: None,
            }),
        })
        .await
        .expect("create quote")
        .into_inner()
        .quote
        .and_then(|q| q.quote)
        .expect("quote");
    QuoteId::from_proto(&quote.id).expect("quote id")
}

async fn enqueue_quote_ready(env: &TestEnv, quote_id: QuoteId) {
    let msg = SendEmailRequest::QuoteReady {
        tenant_id: TENANT_ID,
        quote_id,
        invoicing_entity_id: INVOICING_ENTITY_ID,
        quote_number: "Q-1".to_string(),
        expires_at: None,
        company_name: "ACME_UK".to_string(),
        logo_attachment_id: None,
        recipient_emails: vec![SPOTIFY_RECIPIENT.to_string()],
        portal_url: "https://portal.example/quote".to_string(),
        custom_message: None,
        currency: "EUR".to_string(),
    }
    .try_into()
    .expect("message");
    env.store()
        .pgmq_send_batch(PgmqQueue::SendEmailRequest, vec![msg])
        .await
        .expect("enqueue");
}

async fn set_customer_tier(env: &TestEnv, customer: CustomerId, tier: &str) {
    use diesel_async::RunQueryDsl;
    let mut conn = env.conn().await;
    diesel::sql_query("UPDATE customer SET custom_properties = jsonb_build_object('tier', $2::text) WHERE id = $1")
        .bind::<diesel::sql_types::Uuid, _>(customer.as_uuid())
        .bind::<diesel::sql_types::Text, _>(tier)
        .execute(&mut conn)
        .await
        .expect("set tier");
}

#[rstest]
#[tokio::test]
async fn quote_rule_uses_customer_level_facts_only(#[future] test_env: TestEnv) {
    use meteroid_store::domain::custom_property_definitions::{
        CustomPropertyDefinitionNew, PropertyConfig,
    };
    use meteroid_store::domain::enums::{CustomPropertyEntityType, CustomPropertyType};
    use meteroid_testkit::custom_properties::create_custom_property_definition;

    let env = test_env.await;
    enable_emails(&env).await;
    create_custom_property_definition(
        env.store(),
        CustomPropertyDefinitionNew {
            tenant_id: TENANT_ID,
            entity_type: CustomPropertyEntityType::Customer,
            key: "tier".to_string(),
            name: "Tier".to_string(),
            description: None,
            property_type: CustomPropertyType::Text,
            config: PropertyConfig::default(),
            required: false,
            default_value: None,
            display_order: 0,
        },
    )
    .await;
    set_customer_tier(&env, CUST_SPOTIFY_ID, "enterprise").await;

    let mut client = clients(&env).await.customer_emails;
    let err = set_audience(
        &mut client,
        INVOICING_ENTITY_ID,
        api::EmailKind::QuoteReady,
        api::EmailAudienceMode::Exclude,
        serde_json::json!({ "predicates": [
            { "property": { "key": "tier", "op": "in", "values": ["enterprise"] } },
            { "attribute": { "key": "plan_type", "values": ["FREE"] } }
        ] })
        .to_string(),
    )
    .await
    .expect_err("invoice-only leaf on a quote");
    assert_eq!(err.code(), Code::InvalidArgument, "{err}");
    assert!(err.message().contains("plan_type"), "{err}");
    assert_eq!(stored_audience_rows(&env, INVOICING_ENTITY_ID).await, 0);

    let tier_audience =
        serde_json::json!({ "predicates": [{ "property": { "key": "tier", "op": "in", "values": ["enterprise"] } }] })
            .to_string();
    set_audience(
        &mut client,
        INVOICING_ENTITY_ID,
        api::EmailKind::QuoteReady,
        api::EmailAudienceMode::Exclude,
        tier_audience.clone(),
    )
    .await
    .expect("customer-level rule");
    let preview = client
        .preview_email_kind_audience(api::PreviewEmailKindAudienceRequest {
            invoicing_entity_id: INVOICING_ENTITY_ID.as_proto(),
            kind: api::EmailKind::QuoteReady as i32,
            mode: api::EmailAudienceMode::Exclude as i32,
            audience: tier_audience,
        })
        .await
        .expect("preview")
        .into_inner()
        .preview
        .expect("preview");
    assert_eq!(preview.scanned_invoices, 0);
    assert_eq!(preview.matched_customers, 1);
    assert_eq!(preview.receiving_customers, preview.scanned_customers - 1);

    let quote_id = create_quote_for(&env, CUST_SPOTIFY_ID).await;
    enqueue_quote_ready(&env, quote_id).await;
    assert!(env.drain_email_queue().await.is_empty());
    let acts = activities(&env, "quote", quote_id.as_uuid()).await;
    assert_eq!(
        reasons(&acts, "entity.email_skipped"),
        vec!["audience_excluded"]
    );

    set_customer_tier(&env, CUST_SPOTIFY_ID, "startup").await;
    enqueue_quote_ready(&env, quote_id).await;
    assert_eq!(
        recipients_of(&env.drain_email_queue().await),
        vec![SPOTIFY_RECIPIENT],
        "live customer data"
    );
}

#[rstest]
#[tokio::test]
async fn audience_rules_refuse_foreign_entities_and_payment_reminders(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let mut client = clients(&env).await.customer_emails;

    let other = env
        .store()
        .insert_tenant(
            meteroid_store::domain::tenants::TenantNew {
                name: "Other".to_string(),
                environment: meteroid_store::domain::enums::TenantEnvironmentEnum::Development,
                email_delivery_mode: Some(meteroid_store::domain::enums::EmailDeliveryMode::Off),
                invoicing_entity: None,
            },
            ORGANIZATION_ID,
        )
        .await
        .expect("other tenant");
    let foreign = env
        .store()
        .get_invoicing_entity(other.id, None)
        .await
        .expect("foreign entity")
        .id;
    let country = serde_json::json!({ "predicates": [{ "attribute": { "key": "country", "values": ["FR"] } }] })
        .to_string();
    let err = set_audience(
        &mut client,
        foreign,
        api::EmailKind::InvoiceReady,
        api::EmailAudienceMode::Exclude,
        country.clone(),
    )
    .await
    .expect_err("cross-tenant write");
    assert_eq!(err.code(), Code::NotFound, "{err}");
    assert_eq!(stored_audience_rows(&env, foreign).await, 0);
    let err = client
        .clear_email_kind_audience(api::ClearEmailKindAudienceRequest {
            invoicing_entity_id: foreign.as_proto(),
            kind: api::EmailKind::InvoiceReady as i32,
        })
        .await
        .expect_err("cross-tenant clear");
    assert_eq!(err.code(), Code::NotFound, "{err}");
    let err = client
        .preview_email_kind_audience(api::PreviewEmailKindAudienceRequest {
            invoicing_entity_id: foreign.as_proto(),
            kind: api::EmailKind::InvoiceReady as i32,
            mode: api::EmailAudienceMode::Exclude as i32,
            audience: country.clone(),
        })
        .await
        .expect_err("cross-tenant preview");
    assert_eq!(err.code(), Code::NotFound, "{err}");

    let err = set_audience(
        &mut client,
        INVOICING_ENTITY_ID,
        api::EmailKind::PaymentReminder,
        api::EmailAudienceMode::Exclude,
        country.clone(),
    )
    .await
    .expect_err("reminders use sequences");
    assert_eq!(err.code(), Code::InvalidArgument, "{err}");
    let err = client
        .clear_email_kind_audience(api::ClearEmailKindAudienceRequest {
            invoicing_entity_id: INVOICING_ENTITY_ID.as_proto(),
            kind: api::EmailKind::PaymentReminder as i32,
        })
        .await
        .expect_err("reminders use sequences");
    assert_eq!(err.code(), Code::InvalidArgument, "{err}");
    let err = set_audience(
        &mut client,
        INVOICING_ENTITY_ID,
        api::EmailKind::InvoiceReady,
        api::EmailAudienceMode::Unspecified,
        country,
    )
    .await
    .expect_err("mode is required");
    assert_eq!(err.code(), Code::InvalidArgument, "{err}");
    assert_eq!(stored_audience_rows(&env, INVOICING_ENTITY_ID).await, 0);
}
