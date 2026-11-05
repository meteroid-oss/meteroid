//! Payment reminders in every edition: only writing a step's copy needs `email_customization`.
use std::collections::HashMap;

use meteroid::module::Modules;
use meteroid_grpc::meteroid::api::customeremails::v1::{
    EmailCopy, EmailCopyByLocale, EmailKind, EmailSettingsData, SetEmailKindEnabledRequest,
    UpdateEmailSettingsRequest,
};
use meteroid_grpc::meteroid::api::paymentreminders::v1::{
    ListPaymentReminderSequencesRequest, PaymentReminderStepInput, PaymentReminderStepList,
    UpdatePaymentReminderSequenceRequest,
};
use meteroid_store::domain::entity_activity::Actor;
use meteroid_store::domain::payment_reminders::PaymentReminderSequencePatch;
use meteroid_store::repositories::PaymentRemindersInterface;
use meteroid_testkit::editions::core_test_modules;
use meteroid_testkit::harness::reminders::copies;
use tonic::Code;

use crate::data::ids::{INVOICING_ENTITY_ID, TENANT_ID, USER_ID};
use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::clients::AllClients;
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};

struct Env {
    setup: MeteroidSetup,
    clients: AllClients,
    default_sequence_id: String,
}

async fn env(modules: Modules) -> Env {
    helpers::init::logging();
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
    let default_sequence_id = clients
        .payment_reminders
        .clone()
        .list_payment_reminder_sequences(ListPaymentReminderSequencesRequest {
            invoicing_entity_id: INVOICING_ENTITY_ID.as_proto(),
            include_archived: false,
        })
        .await
        .unwrap()
        .into_inner()
        .sequences
        .into_iter()
        .find(|s| s.is_default)
        .expect("default sequence")
        .id;
    Env {
        setup,
        clients,
        default_sequence_id,
    }
}

fn copy(subject: &str) -> Option<EmailCopyByLocale> {
    Some(EmailCopyByLocale {
        by_locale: copies(
            subject,
            "Hi {{customer_name}},\n\nPlease pay {{amount_due}}.",
        )
        .0
        .into_iter()
        .map(|(locale, c)| {
            (
                locale,
                EmailCopy {
                    subject: c.subject,
                    body: c.body,
                },
            )
        })
        .collect::<HashMap<_, _>>(),
    })
}

fn step(offset_days: i32, content: Option<EmailCopyByLocale>) -> PaymentReminderStepInput {
    PaymentReminderStepInput {
        offset_days,
        content,
    }
}

impl Env {
    async fn set_steps(&self, steps: Vec<PaymentReminderStepInput>) -> Result<(), Code> {
        self.clients
            .payment_reminders
            .clone()
            .update_payment_reminder_sequence(UpdatePaymentReminderSequenceRequest {
                id: self.default_sequence_id.clone(),
                steps: Some(PaymentReminderStepList { steps }),
                ..Default::default()
            })
            .await
            .map(|_| ())
            .map_err(|status| status.code())
    }

    async fn update_settings(&self, data: EmailSettingsData) -> Result<(), Code> {
        self.clients
            .customer_emails
            .clone()
            .update_email_settings(UpdateEmailSettingsRequest {
                invoicing_entity_id: INVOICING_ENTITY_ID.as_proto(),
                data: Some(data),
            })
            .await
            .map(|_| ())
            .map_err(|status| status.code())
    }

    async fn enable_kind(&self, kind: EmailKind) -> Result<(), Code> {
        self.clients
            .customer_emails
            .clone()
            .set_email_kind_enabled(SetEmailKindEnabledRequest {
                invoicing_entity_id: INVOICING_ENTITY_ID.as_proto(),
                kind: kind.into(),
                enabled: true,
            })
            .await
            .map(|_| ())
            .map_err(|status| status.code())
    }
}

#[tokio::test]
async fn oss_runs_reminders_on_the_built_in_copy() {
    let env = env(Modules::core()).await;

    assert_eq!(env.enable_kind(EmailKind::PaymentReminder).await, Ok(()));
    assert_eq!(
        env.enable_kind(EmailKind::InvoiceReady).await,
        Err(Code::PermissionDenied)
    );
    assert_eq!(
        env.update_settings(EmailSettingsData {
            reminder_send_hour: Some(10),
            timezone: Some("Europe/Paris".to_string()),
            ..Default::default()
        })
        .await,
        Ok(())
    );
    assert_eq!(
        env.update_settings(EmailSettingsData {
            sender_name: Some("Acme billing".to_string()),
            ..Default::default()
        })
        .await,
        Err(Code::PermissionDenied)
    );

    assert_eq!(
        env.set_steps(vec![step(-3, None), step(7, None), step(14, None)])
            .await,
        Ok(())
    );
    assert_eq!(
        env.set_steps(vec![step(-3, None), step(7, copy("Overdue"))])
            .await,
        Err(Code::PermissionDenied)
    );
}

#[tokio::test]
async fn oss_keeps_sending_the_stored_copy() {
    let env = env(Modules::core()).await;
    env.setup
        .store
        .update_reminder_sequence(
            Actor::User { id: USER_ID },
            TENANT_ID,
            PaymentReminderSequencePatch {
                id: env.default_sequence_id.parse().unwrap(),
                steps: Some(vec![
                    meteroid_store::domain::payment_reminders::PaymentReminderStepInput {
                        offset_days: 7,
                        content: copies(
                            "Overdue",
                            "Hi {{customer_name}},\n\nPlease pay {{amount_due}}.",
                        ),
                    },
                ]),
                ..Default::default()
            },
            chrono::Utc::now().naive_utc(),
        )
        .await
        .unwrap();

    assert_eq!(env.set_steps(vec![step(7, copy("Overdue"))]).await, Ok(()));
    assert_eq!(
        env.set_steps(vec![step(-3, None), step(10, copy("Overdue"))])
            .await,
        Ok(())
    );
    assert_eq!(
        env.set_steps(vec![step(10, copy("Pay now"))]).await,
        Err(Code::PermissionDenied)
    );
    assert_eq!(env.set_steps(vec![step(10, None)]).await, Ok(()));
}

#[tokio::test]
async fn email_customization_writes_the_copy() {
    let env = env(core_test_modules()).await;
    assert_eq!(
        env.set_steps(vec![step(-3, None), step(7, copy("Overdue"))])
            .await,
        Ok(())
    );
}
