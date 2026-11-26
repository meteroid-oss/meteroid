//! The customer's confirmation of a cancellation in text form (§ 312k BGB): the plan, the day it
//! was received and the last day of service, once per ending.

use chrono::NaiveDate;
use rstest::rstest;

use common_domain::ids::SubscriptionId;
use diesel_models::enums::SubscriptionStatusEnum;
use meteroid_mailer::model::Email;
use meteroid_store::domain::InvoicingEntityPatch;
use meteroid_store::domain::entity_activity::Actor;
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;
use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;

use crate::data::ids::*;
use crate::harness::{SubscriptionAssertExt, TestEnv, subscription, test_env};
use crate::test_customer_emails::{enable_emails, set_recipients};

const RECIPIENT: &str = "billing@uber.example";
const SUBJECT: &str = "Your ACME_UK cancellation is confirmed";

fn at(day: NaiveDate) -> chrono::DateTime<chrono::Utc> {
    day.and_hms_opt(10, 0, 0).unwrap().and_utc()
}

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

async fn english_customer(env: &TestEnv) {
    enable_emails(env).await;
    env.store()
        .patch_invoicing_entity(
            Actor::System,
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                customer_extra_locales: Some(vec!["en-US".to_string()]),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .expect("customer extra locales");
    set_recipients(env, CUST_UBER_ID, &[RECIPIENT], &["en-US"]).await;
}

async fn cancel(env: &TestEnv, sub: SubscriptionId, effective_at: CancellationEffectiveAt) {
    env.services()
        .cancel_subscription(
            Actor::Customer { id: CUST_UBER_ID },
            sub,
            TENANT_ID,
            None,
            effective_at,
        )
        .await
        .expect("cancel");
}

async fn confirmations(env: &TestEnv) -> Vec<Email> {
    env.drain_email_queue()
        .await
        .into_iter()
        .filter(|email| email.subject == SUBJECT)
        .collect()
}

#[rstest]
#[tokio::test]
async fn a_cancellation_at_period_end_is_confirmed_with_its_dates(#[future] test_env: TestEnv) {
    let env = test_env.await;
    english_customer(&env).await;
    env.set_now(at(day(2024, 1, 1)));
    let sub = subscription()
        .start_date(day(2024, 1, 1))
        .on_start()
        .no_trial()
        .no_auto_charge()
        .create(env.services())
        .await;
    env.run_outbox_and_orchestration().await;
    env.drain_email_queue().await;

    env.set_now(at(day(2024, 1, 15)));
    cancel(&env, sub, CancellationEffectiveAt::EndOfBillingPeriod).await;
    let sent = confirmations(&env).await;
    assert_eq!(sent.len(), 1, "one confirmation");
    let email = &sent[0];
    assert!(email.to.iter().map(|r| r.email.as_str()).eq([RECIPIENT]));
    assert!(email.attachments.is_empty());
    let html = &email.body_html;
    assert!(html.contains("Your LeetCode subscription"), "{html}");
    assert!(html.contains(">Cancellation received</td>"), "{html}");
    assert!(
        html.contains(">January 15, 2024 at 10:00 UTC</td>"),
        "received: {html}"
    );
    assert!(html.contains(">Last day of service</td>"), "{html}");
    assert!(html.contains(">January 31, 2024</td>"), "last day: {html}");

    // The same request recorded again confirms nothing new; a moved ending confirms its new date.
    cancel(&env, sub, CancellationEffectiveAt::Date(day(2024, 2, 1))).await;
    assert!(confirmations(&env).await.is_empty(), "same ending");
    cancel(&env, sub, CancellationEffectiveAt::Date(day(2024, 1, 20))).await;
    let moved = confirmations(&env).await;
    assert_eq!(moved.len(), 1, "moved ending");
    assert!(
        moved[0].body_html.contains(">January 19, 2024</td>"),
        "{}",
        moved[0].body_html
    );
}

#[rstest]
#[tokio::test]
async fn an_aborted_pending_subscription_is_not_confirmed(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_mock_payment_provider(false).await;
    english_customer(&env).await;
    env.set_now(at(day(2024, 1, 1)));
    let sub = subscription()
        .start_date(day(2024, 1, 1))
        .on_checkout()
        .no_trial()
        .auto_charge()
        .create(env.services())
        .await;
    env.get_subscription(sub)
        .await
        .assert()
        .is_pending_activation();

    cancel(&env, sub, CancellationEffectiveAt::Date(day(2024, 1, 1))).await;
    env.run_outbox_and_orchestration().await;
    env.get_subscription(sub)
        .await
        .assert()
        .has_status(SubscriptionStatusEnum::Aborted);
    assert!(confirmations(&env).await.is_empty());
}
