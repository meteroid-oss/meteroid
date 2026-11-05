//! Stored portal links: reuse, expiry, revocation, expired-link recovery and the legacy JWTs.

use chrono::{Duration, Utc};
use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use rstest::rstest;
use serde_json::json;
use tonic::Code;

use crate::data::ids::*;
use crate::harness::{TestEnv, test_env};
use crate::test_customer_emails::{clients, enable_emails, invoice_with_pdf, set_recipients};
use crate::test_quote::create_test_quote;
use common_domain::ids::{BaseId, InvoiceId, QuoteId, SavedViewId};
use meteroid_grpc::meteroid::portal::invoice::v1::GetInvoiceForPaymentRequest;
use meteroid_grpc::meteroid::portal::invoice::v1::portal_invoice_service_client::PortalInvoiceServiceClient;
use meteroid_grpc::meteroid::portal::quotes::v1::GetQuotePortalRequest;
use meteroid_grpc::meteroid::portal::quotes::v1::portal_quote_service_client::PortalQuoteServiceClient;
use meteroid_mailer::model::Email;
use meteroid_store::domain::portal_tokens::{
    PORTAL_LINK_EXPIRED, PortalTokenStatus, hash_portal_token, is_portal_token,
};
use meteroid_store::jwt_claims::{ResourceAccess, generate_portal_token};
use meteroid_store::repositories::QuotesInterface;
use meteroid_store::repositories::portal_tokens::{
    PortalTokensInterface, forget_cached_portal_tokens,
};

fn with_portal_token<T>(message: T, token: &str) -> tonic::Request<T> {
    let mut request = tonic::Request::new(message);
    request
        .metadata_mut()
        .insert("x-portal-token", token.parse().unwrap());
    request
}

async fn open_invoice(
    env: &TestEnv,
    invoice_id: InvoiceId,
    token: &str,
) -> Result<(), tonic::Status> {
    PortalInvoiceServiceClient::new(env.setup.channel.clone())
        .get_invoice_payment(with_portal_token(
            GetInvoiceForPaymentRequest {
                invoice_id: Some(invoice_id.as_proto()),
            },
            token,
        ))
        .await
        .map(|_| ())
}

async fn open_quote(env: &TestEnv, token: &str) -> Result<(), tonic::Status> {
    PortalQuoteServiceClient::new(env.setup.channel.clone())
        .get_quote_portal(with_portal_token(GetQuotePortalRequest {}, token))
        .await
        .map(|_| ())
}

async fn expire(env: &TestEnv, token: &str) {
    use diesel_models::schema::portal_token::dsl as d;
    diesel::update(d::portal_token.filter(d::token_hash.eq(hash_portal_token(token))))
        .set(d::expires_at.eq(Some(Utc::now() - Duration::hours(1))))
        .execute(&mut env.conn().await)
        .await
        .expect("expire token");
    forget_cached_portal_tokens().await;
}

async fn recover(env: &TestEnv, token: &str, email: &str) {
    let res = http_client_tracing::client()
        .post(format!(
            "{}/v1/portal/links/recover",
            env.setup.config.rest_api_external_url
        ))
        .json(&json!({ "token": token, "email": email }))
        .send()
        .await
        .expect("recover request");
    assert_eq!(res.status(), reqwest::StatusCode::ACCEPTED);
}

fn link_token(email: &Email) -> String {
    email
        .body_html
        .split("token=")
        .skip(1)
        .map(|rest| {
            rest.chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                .collect::<String>()
        })
        .find(|token| is_portal_token(token))
        .expect("portal link in email")
}

fn recipients(email: &Email) -> Vec<String> {
    email.to.iter().map(|r| r.email.clone()).collect()
}

#[rstest]
#[tokio::test]
async fn delegated_and_direct_links_are_never_shared(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let invoice_id = invoice_with_pdf(&env, CUST_UBER_ID).await;
    let store = env.store();

    let direct = store
        .invoice_portal_token(TENANT_ID, invoice_id, false)
        .await
        .unwrap();
    let delegated = store
        .invoice_portal_token(TENANT_ID, invoice_id, true)
        .await
        .unwrap();
    assert_ne!(direct, delegated);
    assert_eq!(
        store
            .invoice_portal_token(TENANT_ID, invoice_id, true)
            .await
            .unwrap(),
        delegated
    );

    let resolved = |token: String| async move {
        store
            .resolve_portal_token(&token)
            .await
            .unwrap()
            .unwrap()
            .delegated
    };
    assert!(!resolved(direct).await);
    assert!(resolved(delegated).await);
}

#[rstest]
#[tokio::test]
async fn invoice_link_is_reused_then_recovered_once_expired(#[future] test_env: TestEnv) {
    let env = test_env.await;
    enable_emails(&env).await;
    set_recipients(
        &env,
        CUST_UBER_ID,
        &["billing@uber.example", "ap@uber.example"],
        &[],
    )
    .await;
    let invoice_id = invoice_with_pdf(&env, CUST_UBER_ID).await;

    let token = env
        .store()
        .invoice_portal_token(TENANT_ID, invoice_id, false)
        .await
        .unwrap();
    assert!(is_portal_token(&token));
    assert_eq!(
        env.store()
            .invoice_portal_token(TENANT_ID, invoice_id, false)
            .await
            .unwrap(),
        token
    );
    open_invoice(&env, invoice_id, &token).await.unwrap();

    // Active links are not recoverable.
    recover(&env, &token, "ap@uber.example").await;
    assert!(env.drain_email_queue().await.is_empty());

    expire(&env, &token).await;
    let expired = open_invoice(&env, invoice_id, &token).await.unwrap_err();
    assert_eq!(expired.code(), Code::Unauthenticated);
    assert_eq!(expired.message(), PORTAL_LINK_EXPIRED);

    recover(&env, &token, "stranger@elsewhere.example").await;
    assert!(env.drain_email_queue().await.is_empty());

    recover(&env, &token, " AP@uber.example ").await;
    let sent = env.drain_email_queue().await;
    assert_eq!(sent.len(), 1);
    assert_eq!(recipients(&sent[0]), vec!["ap@uber.example".to_string()]);
    let fresh = link_token(&sent[0]);
    assert_ne!(fresh, token);
    open_invoice(&env, invoice_id, &fresh).await.unwrap();

    // One recovery email per link and hour.
    recover(&env, &token, "billing@uber.example").await;
    assert!(env.drain_email_queue().await.is_empty());
}

#[rstest]
#[tokio::test]
async fn quote_link_is_recovered_for_its_recipient_only(#[future] test_env: TestEnv) {
    let env = test_env.await;
    enable_emails(&env).await;
    let quote = create_test_quote(&clients(&env).await).await;
    let quote_id = QuoteId::from_proto(&quote.quote.unwrap().id).unwrap();
    {
        use diesel_models::schema::quote::dsl as q;
        diesel::update(q::quote.filter(q::id.eq(quote_id)))
            .set(q::recipients.eq(json!([
                { "name": "Buyer", "email": "buyer@spotify.example" },
                { "name": "Legal", "email": "legal@spotify.example" },
            ])))
            .execute(&mut env.conn().await)
            .await
            .expect("set recipients");
    }

    env.store()
        .send_quote(
            meteroid_store::domain::entity_activity::Actor::System,
            quote_id,
            TENANT_ID,
            None,
        )
        .await
        .unwrap();
    let sent = env.drain_email_queue().await;
    assert_eq!(sent.len(), 2);
    let buyer_email = sent
        .iter()
        .find(|e| recipients(e) == vec!["buyer@spotify.example".to_string()])
        .expect("buyer email");
    let token = link_token(buyer_email);
    open_quote(&env, &token).await.unwrap();

    expire(&env, &token).await;
    assert_eq!(
        open_quote(&env, &token).await.unwrap_err().message(),
        PORTAL_LINK_EXPIRED
    );

    // Another recipient of the same quote has a link of their own.
    recover(&env, &token, "legal@spotify.example").await;
    assert!(env.drain_email_queue().await.is_empty());

    recover(&env, &token, "buyer@spotify.example").await;
    let sent = env.drain_email_queue().await;
    assert_eq!(sent.len(), 1);
    assert_eq!(
        recipients(&sent[0]),
        vec!["buyer@spotify.example".to_string()]
    );
    let fresh = link_token(&sent[0]);
    assert_ne!(fresh, token);
    open_quote(&env, &fresh).await.unwrap();
}

#[rstest]
#[tokio::test]
async fn revoked_dashboard_link_stays_dead_after_resharing(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let view = SavedViewId::new();
    let store = env.store();

    let token = store.dashboard_portal_token(TENANT_ID, view).await.unwrap();
    assert_eq!(
        store.dashboard_portal_token(TENANT_ID, view).await.unwrap(),
        token
    );

    store
        .revoke_portal_tokens(TENANT_ID, &ResourceAccess::Dashboard(view))
        .await
        .unwrap();
    let revoked = store.resolve_portal_token(&token).await.unwrap().unwrap();
    assert_eq!(revoked.status(Utc::now()), PortalTokenStatus::Revoked);

    let reshared = store.dashboard_portal_token(TENANT_ID, view).await.unwrap();
    assert_ne!(reshared, token);
    let still_revoked = store.resolve_portal_token(&token).await.unwrap().unwrap();
    assert_eq!(still_revoked.status(Utc::now()), PortalTokenStatus::Revoked);
}

#[rstest]
#[tokio::test]
async fn legacy_jwt_links_keep_working(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let invoice_id = invoice_with_pdf(&env, CUST_UBER_ID).await;

    let legacy = generate_portal_token(
        &env.setup.config.jwt_secret,
        TENANT_ID,
        ResourceAccess::Invoice(invoice_id),
        None,
        None,
        false,
    )
    .unwrap();
    open_invoice(&env, invoice_id, &legacy).await.unwrap();

    let unknown = open_invoice(&env, invoice_id, &"A".repeat(32))
        .await
        .unwrap_err();
    assert_eq!(unknown.code(), Code::Unauthenticated);
    assert_ne!(unknown.message(), PORTAL_LINK_EXPIRED);
}
