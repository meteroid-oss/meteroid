//! Payment reminders in the open-source edition: one switch for the default reminders.
use meteroid::module::Modules;
use meteroid_grpc::meteroid::api::customeremails::v1::{
    EmailKind, EmailSettingsData, GetPaymentRemindersEnabledRequest, SetEmailKindEnabledRequest,
    SetPaymentRemindersEnabledRequest, UpdateEmailSettingsRequest,
};
use meteroid_grpc::meteroid::api::paymentreminders::v1::ListPaymentReminderSequencesRequest;
use meteroid_store::domain::customer_emails::EmailKind as DomainEmailKind;
use meteroid_store::repositories::{CustomerEmailsInterface, PaymentRemindersInterface};
use tonic::Code;

use crate::data::ids::{INVOICING_ENTITY_ID, TENANT_ID};
use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::clients::AllClients;
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};

struct Env {
    setup: MeteroidSetup,
    clients: AllClients,
}

async fn env() -> Env {
    helpers::init::logging();
    let modules = Modules::core();
    let database = meteroid_it::container::create_seeded_test_database(SeedLevel::MINIMAL).await;
    let instance = modules.edition.instance();
    let setup = meteroid_it::container::start_meteroid_with_modules(
        database,
        SeedLevel::MINIMAL,
        instance,
        modules,
    )
    .await;
    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let clients = AllClients::from_channel(
        setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        "testslug",
    );
    Env { setup, clients }
}

impl Env {
    async fn enabled(&self) -> bool {
        self.clients
            .customer_emails
            .clone()
            .get_payment_reminders_enabled(GetPaymentRemindersEnabledRequest {
                invoicing_entity_id: INVOICING_ENTITY_ID.as_proto(),
            })
            .await
            .unwrap()
            .into_inner()
            .enabled
    }

    async fn set_enabled(&self, enabled: bool) {
        self.clients
            .customer_emails
            .clone()
            .set_payment_reminders_enabled(SetPaymentRemindersEnabledRequest {
                invoicing_entity_id: INVOICING_ENTITY_ID.as_proto(),
                enabled,
            })
            .await
            .unwrap();
    }

    /// The reminder emails' kind switch and the default sequence's, as stored.
    async fn stored(&self) -> (bool, bool) {
        let store = &self.setup.store;
        let kind = store
            .get_email_settings(TENANT_ID, INVOICING_ENTITY_ID)
            .await
            .unwrap()
            .is_enabled(DomainEmailKind::PaymentReminder);
        let default = store
            .list_reminder_sequences(TENANT_ID, INVOICING_ENTITY_ID, false)
            .await
            .unwrap()
            .into_iter()
            .find(|s| s.sequence.is_default())
            .expect("default sequence")
            .sequence
            .enabled;
        (kind, default)
    }
}

#[tokio::test]
async fn the_switch_turns_the_default_reminders_on_and_off() {
    let env = env().await;
    assert!(!env.enabled().await);

    env.set_enabled(true).await;
    assert!(env.enabled().await);
    assert_eq!(env.stored().await, (true, true));

    env.set_enabled(false).await;
    assert!(!env.enabled().await);
    assert_eq!(env.stored().await, (false, true));
}

#[tokio::test]
async fn oss_does_not_edit_sequences_or_their_schedule() {
    let env = env().await;
    let sequences = env
        .clients
        .payment_reminders
        .clone()
        .list_payment_reminder_sequences(ListPaymentReminderSequencesRequest {
            invoicing_entity_id: INVOICING_ENTITY_ID.as_proto(),
            include_archived: false,
        })
        .await;
    assert_eq!(sequences.unwrap_err().code(), Code::Unimplemented);

    let kind = env
        .clients
        .customer_emails
        .clone()
        .set_email_kind_enabled(SetEmailKindEnabledRequest {
            invoicing_entity_id: INVOICING_ENTITY_ID.as_proto(),
            kind: EmailKind::PaymentReminder.into(),
            enabled: true,
        })
        .await;
    assert_eq!(kind.unwrap_err().code(), Code::PermissionDenied);

    let send_hour = env
        .clients
        .customer_emails
        .clone()
        .update_email_settings(UpdateEmailSettingsRequest {
            invoicing_entity_id: INVOICING_ENTITY_ID.as_proto(),
            data: Some(EmailSettingsData {
                reminder_send_hour: Some(10),
                timezone: Some("Europe/Paris".to_string()),
                ..Default::default()
            }),
        })
        .await;
    assert_eq!(send_hour.unwrap_err().code(), Code::PermissionDenied);
}
