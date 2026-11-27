//! Pre-dunning: the customer whose automatically charged card expires soon is told once, with a
//! link that may only update payment methods.

use chrono::NaiveDate;
use rstest::rstest;

use common_domain::auth::PortalScope;
use meteroid_mailer::model::Email;
use meteroid_store::domain::InvoicingEntityPatch;
use meteroid_store::domain::customer_emails::EmailKind;
use meteroid_store::domain::entity_activity::Actor;
use meteroid_store::repositories::customer_emails::CustomerEmailsInterface;
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;
use meteroid_store::repositories::portal_tokens::PortalTokensInterface;

use crate::data::ids::*;
use crate::harness::{TestEnv, subscription, test_env};
use crate::test_customer_emails::{enable_emails, set_recipients};

const SUBJECT: &str = "Your payment method for ACME_UK expires soon";

fn at(y: i32, m: u32, d: u32) -> chrono::NaiveDateTime {
    NaiveDate::from_ymd_opt(y, m, d)
        .unwrap()
        .and_hms_opt(10, 0, 0)
        .unwrap()
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
    set_recipients(env, CUST_UBER_ID, &["billing@uber.example"], &["en-US"]).await;
}

async fn sweep(env: &TestEnv, now: chrono::NaiveDateTime) -> usize {
    env.services()
        .run_card_expiry_sweep(now)
        .await
        .expect("card expiry sweep")
}

async fn notices(env: &TestEnv) -> Vec<Email> {
    env.drain_email_queue()
        .await
        .into_iter()
        .filter(|email| email.subject == SUBJECT)
        .collect()
}

/// The seeded card of Uber expires at the end of December 2030.
#[rstest]
#[tokio::test]
async fn a_card_expiring_soon_is_notified_once_when_the_tenant_opts_in(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    english_customer(&env).await;
    env.seed_payments().await;
    env.set_now(at(2030, 11, 1).and_utc());
    subscription()
        .start_date(NaiveDate::from_ymd_opt(2030, 11, 1).unwrap())
        .on_start()
        .no_trial()
        .auto_charge()
        .create(env.services())
        .await;
    env.drain_email_queue().await;

    assert_eq!(sweep(&env, at(2030, 11, 20)).await, 0, "off by default");
    env.store()
        .set_email_kind_enabled(
            Actor::System,
            TENANT_ID,
            INVOICING_ENTITY_ID,
            EmailKind::PaymentMethodExpiring,
            true,
        )
        .await
        .expect("turn the email on");
    assert_eq!(
        sweep(&env, at(2030, 11, 20)).await,
        0,
        "more than 30 days ahead"
    );

    assert_eq!(sweep(&env, at(2030, 12, 5)).await, 1);
    assert_eq!(sweep(&env, at(2030, 12, 6)).await, 0, "once per expiry");

    let sent = notices(&env).await;
    assert_eq!(sent.len(), 1);
    let html = &sent[0].body_html;
    assert!(html.contains("•••• 4242"), "the card is named");
    assert!(html.contains("December 31, 2030"), "its last valid day");
    let token = html
        .split("/portal/customer?token=")
        .nth(1)
        .and_then(|rest| rest.split('&').next())
        .expect("a portal link");
    assert!(
        html.contains("page=settings"),
        "it opens the payment methods"
    );

    let issued = env
        .store()
        .resolve_portal_token(token)
        .await
        .expect("resolve")
        .expect("a stored token");
    assert_eq!(issued.scopes, Some(vec![PortalScope::PaymentMethods]));
}
