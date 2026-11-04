//! What the customer receives and may do after paying, or failing to: receipts for card-charged
//! purchases, billing emails without invoicing recipients, wallet notices that link the portal,
//! the default payment method after a decline, and promo codes at checkout (fix package P1).

use chrono::{NaiveDate, Utc};
use rstest::rstest;
use tonic::Code;

use common_domain::ids::{
    BaseId, CreditOperationId, CustomerId, CustomerPaymentMethodId, EventId, WalletId,
};
use meteroid_grpc::meteroid::portal::checkout::v1 as portal;
use meteroid_store::domain::checkout_sessions::{CheckoutCompletionResult, CheckoutType};
use meteroid_store::domain::connectors::MockChargeBehavior;
use meteroid_store::domain::coupons::{CouponCreditScope, CouponDiscount, CouponNew};
use meteroid_store::domain::credit::TopUpWallet;
use meteroid_store::domain::customer_emails::{EmailContext, EmailKind};
use meteroid_store::domain::entity_activity::Actor;
use meteroid_store::domain::outbox_event::InvoicePdfGeneratedEvent;
use meteroid_store::domain::{
    CreateCheckoutSession, CustomerPatch, InvoicePaymentStatus, InvoicingEntityPatch,
    PaymentStatusEnum,
};
use meteroid_store::errors::StoreError;
use meteroid_store::jwt_claims::ResourceAccess;
use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::customers::CustomersInterfaceAuto;
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;
use meteroid_store::repositories::{CustomerEmailsInterface, CustomersInterface, InvoiceInterface};
use meteroid_store::services::{DefaultMethodAdoption, SellCustomerCredit};
use rust_decimal_macros::dec;

use crate::data::ids::*;
use crate::data::payment::{create_customer_payment_method, set_mock_charge_behavior};
use crate::harness::{TestEnv, subscription, test_env};

const OWNER: &str = "owner@uber.example";

async fn enable_english_emails(env: &TestEnv) {
    env.set_email_delivery_mode(diesel_models::enums::EmailDeliveryModeEnum::Live)
        .await;
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
}

fn customer_patch(customer: CustomerId) -> CustomerPatch {
    CustomerPatch {
        id: customer,
        name: None,
        alias: None,
        billing_email: None,
        invoicing_emails: None,
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
        preferred_locales: None,
        customer_type: None,
        first_name: None,
        last_name: None,
    }
}

/// The customer's own email filled, as the create form does, and no invoicing recipients.
async fn only_customer_email(env: &TestEnv, customer: CustomerId) {
    env.store()
        .patch_customer(
            Actor::System,
            TENANT_ID,
            CustomerPatch {
                billing_email: Some(OWNER.to_string()),
                invoicing_emails: Some(vec![]),
                preferred_locales: Some(vec!["en-US".to_string()]),
                ..customer_patch(customer)
            },
        )
        .await
        .expect("customer email");
}

async fn set_default_method(
    env: &TestEnv,
    customer: CustomerId,
    method: Option<CustomerPaymentMethodId>,
) {
    env.store()
        .patch_customer(
            Actor::System,
            TENANT_ID,
            CustomerPatch {
                current_payment_method_id: Some(method),
                ..customer_patch(customer)
            },
        )
        .await
        .expect("default payment method");
}

async fn default_method(env: &TestEnv, customer: CustomerId) -> Option<CustomerPaymentMethodId> {
    env.store()
        .find_customer_by_id(customer, TENANT_ID)
        .await
        .expect("customer")
        .current_payment_method_id
}

async fn portal_switches(env: &TestEnv, enabled: bool) {
    env.store()
        .patch_invoicing_entity(
            Actor::System,
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                portal_credit_top_up_enabled: Some(enabled),
                portal_auto_top_up_enabled: Some(enabled),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .expect("portal switches");
}

async fn eur_wallet(env: &TestEnv, customer: CustomerId) -> WalletId {
    env.store()
        .create_customer_credit_wallet(TENANT_ID, customer, "EUR")
        .await
        .expect("wallet")
        .id
}

fn top_up(wallet: WalletId, method: CustomerPaymentMethodId, key: &str) -> TopUpWallet {
    TopUpWallet {
        tenant_id: TENANT_ID,
        customer_id: CUST_UBER_ID,
        wallet_id: wallet,
        amount_minor: 5_000,
        payment_method_id: method,
        idempotency_key: key.to_owned(),
        expires_at: None,
        unit_price: None,
        expected_unit_price: None,
    }
}

async fn enqueue(env: &TestEnv, kind: EmailKind, context: EmailContext, key: &str) {
    env.store()
        .enqueue_customer_email(TENANT_ID, kind, context, key.to_string())
        .await
        .expect("enqueue");
}

/// RB-8. A top-up charged on a card is paid inside its finalization, before its PDF exists: the
/// receipt goes out once the document is rendered, although the invoice has no subscription.
#[rstest]
#[tokio::test]
async fn a_card_charged_top_up_sends_its_receipt_once_its_document_exists(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.seed_payments().await;
    enable_english_emails(&env).await;
    only_customer_email(&env, CUST_UBER_ID).await;
    let wallet = eur_wallet(&env, CUST_UBER_ID).await;

    let receipt = env
        .services()
        .top_up_wallet(
            top_up(wallet, CUST_UBER_PAYMENT_METHOD_ID, "receipt-1"),
            None,
        )
        .await
        .expect("top-up");
    assert_eq!(receipt.transaction.status, PaymentStatusEnum::Settled);
    env.run_outbox_and_orchestration().await;

    let invoice = env
        .store()
        .get_invoice_by_id(TENANT_ID, receipt.invoice_id)
        .await
        .expect("invoice");
    assert_eq!(invoice.payment_status, InvoicePaymentStatus::Paid);
    assert!(invoice.subscription_id.is_none());
    assert!(invoice.pdf_document_id.is_some());
    let emails = env.drain_email_queue().await;
    let receipts = emails
        .iter()
        .filter(|email| email.subject == "Your ACME_UK receipt")
        .collect::<Vec<_>>();
    assert_eq!(
        receipts.len(),
        1,
        "{:?}",
        emails
            .iter()
            .map(|email| &email.subject)
            .collect::<Vec<_>>()
    );
    assert!(receipts[0].to.iter().map(|r| r.email.as_str()).eq([OWNER]));
    assert!(
        !receipts[0].attachments.is_empty(),
        "the documents are attached"
    );
}

/// Credit sold with "send an invoice" has no subscription and no payment: its invoice is emailed
/// for payment once, a redelivered PDF event sending nothing more.
#[rstest]
#[tokio::test]
async fn credit_sold_by_invoice_is_emailed_for_payment_once(#[future] test_env: TestEnv) {
    let env = test_env.await;
    enable_english_emails(&env).await;
    only_customer_email(&env, CUST_UBER_ID).await;
    let currency = env
        .store()
        .find_customer_by_id(CUST_UBER_ID, TENANT_ID)
        .await
        .expect("customer")
        .currency;
    let wallet = env
        .store()
        .create_customer_credit_wallet(TENANT_ID, CUST_UBER_ID, &currency)
        .await
        .expect("wallet")
        .id;
    let sold = env
        .services()
        .sell_customer_credit(
            uuid::Uuid::now_v7(),
            SellCustomerCredit {
                expires_at: None,
                tenant_id: TENANT_ID,
                customer_id: CUST_UBER_ID,
                wallet_id: wallet,
                amount_minor: 5_000,
                expires_in_months: None,
                note: None,
                actor: Actor::User { id: USER_ID },
                unit_price: None,
                expected_unit_price: None,
            },
        )
        .await
        .expect("sold");
    let invoice_id = sold.invoice.invoice.id;
    env.run_outbox_and_orchestration().await;

    let invoice = env
        .store()
        .get_invoice_by_id(TENANT_ID, invoice_id)
        .await
        .expect("invoice");
    assert!(invoice.subscription_id.is_none() && !invoice.manual);
    assert_eq!(invoice.payment_status, InvoicePaymentStatus::Unpaid);
    let pdf_id = invoice.pdf_document_id.expect("pdf");
    let emails = env.drain_email_queue().await;
    assert_eq!(
        emails.len(),
        1,
        "{:?}",
        emails
            .iter()
            .map(|email| &email.subject)
            .collect::<Vec<_>>()
    );
    assert!(emails[0].to.iter().map(|r| r.email.as_str()).eq([OWNER]));
    let mut conn = env.conn().await;
    assert!(
        diesel_models::email_send_claim::EmailSendClaimRow::find_by_dedupe_key(
            &mut conn,
            &format!("invoice_ready:{invoice_id}"),
        )
        .await
        .expect("claim")
        .is_some()
    );
    drop(conn);

    env.services()
        .on_invoice_accounting_pdf_generated(
            InvoicePdfGeneratedEvent {
                id: EventId::new(),
                invoice_id,
                tenant_id: TENANT_ID,
                customer_id: CUST_UBER_ID,
                pdf_id,
                xml_id: None,
                einvoicing_status: None,
                einvoicing_report: None,
            },
            TENANT_ID,
        )
        .await
        .expect("redelivered");
    assert!(env.drain_email_queue().await.is_empty());
}

/// RF-1. A customer with only their own email filled receives billing emails there; invoicing
/// recipients, when set, still replace it.
#[rstest]
#[tokio::test]
async fn billing_emails_fall_back_to_the_customer_email(#[future] test_env: TestEnv) {
    let env = test_env.await;
    enable_english_emails(&env).await;
    only_customer_email(&env, CUST_UBER_ID).await;
    let notice = EmailContext::WalletBalanceLow {
        customer_id: CUST_UBER_ID,
        wallet_id: WalletId::new(),
        currency: "EUR".to_string(),
        available_minor: 900,
        threshold_minor: 10_000,
    };
    enqueue(&env, EmailKind::WalletBalanceLow, notice.clone(), "low:1").await;
    let emails = env.drain_email_queue().await;
    assert_eq!(emails.len(), 1);
    assert!(emails[0].to.iter().map(|r| r.email.as_str()).eq([OWNER]));

    env.store()
        .patch_customer(
            Actor::System,
            TENANT_ID,
            CustomerPatch {
                invoicing_emails: Some(vec!["ap@uber.example".to_string()]),
                ..customer_patch(CUST_UBER_ID)
            },
        )
        .await
        .expect("invoicing emails");
    enqueue(&env, EmailKind::WalletBalanceLow, notice, "low:2").await;
    let emails = env.drain_email_queue().await;
    assert_eq!(emails.len(), 1);
    assert!(
        emails[0]
            .to
            .iter()
            .map(|r| r.email.as_str())
            .eq(["ap@uber.example"])
    );
}

/// RF-4 and DN-1. A wallet notice asks to add credit only when the portal can sell it, else
/// points to the merchant, and carries no portal link: an emailed session would sign in whoever
/// reads the email.
#[rstest]
#[tokio::test]
async fn a_wallet_notice_asks_only_what_the_portal_offers(#[future] test_env: TestEnv) {
    let env = test_env.await;
    enable_english_emails(&env).await;
    only_customer_email(&env, CUST_UBER_ID).await;
    let wallet = WalletId::new();
    let low = EmailContext::WalletBalanceLow {
        customer_id: CUST_UBER_ID,
        wallet_id: wallet,
        currency: "EUR".to_string(),
        available_minor: 900,
        threshold_minor: 10_000,
    };
    let suspended = EmailContext::AutoTopUpFailed {
        customer_id: CUST_UBER_ID,
        wallet_id: wallet,
        operation_id: CreditOperationId::new(),
        currency: "EUR".to_string(),
        amount_minor: 5_000,
        suspended: true,
        payment_reversed: false,
    };

    // Switches on, but no card provider and no saved card: nothing to pay with.
    portal_switches(&env, true).await;
    enqueue(&env, EmailKind::WalletBalanceLow, low.clone(), "low:1").await;
    enqueue(
        &env,
        EmailKind::AutoTopUpFailed,
        suspended.clone(),
        "auto:1",
    )
    .await;
    let mut emails = env.drain_email_queue().await;
    emails.sort_by(|a, b| a.subject.cmp(&b.subject));
    let (paused, low_body) = (&emails[0].body_html, &emails[1].body_html);
    assert!(
        low_body.contains("Contact ACME_UK to add credit."),
        "{low_body}"
    );
    assert!(paused.contains("Contact ACME_UK to resume it."), "{paused}");
    for body in [paused, low_body] {
        assert!(
            !body.contains("token="),
            "no portal session in an email: {body}"
        );
        assert!(!body.contains("/portal/customer"), "{body}");
    }

    env.seed_payments().await;
    enqueue(&env, EmailKind::WalletBalanceLow, low, "low:2").await;
    enqueue(&env, EmailKind::AutoTopUpFailed, suspended, "auto:2").await;
    let mut emails = env.drain_email_queue().await;
    emails.sort_by(|a, b| a.subject.cmp(&b.subject));
    let (paused, low_body) = (&emails[0].body_html, &emails[1].body_html);
    assert!(
        low_body.contains("Add credit to keep your usage covered."),
        "{low_body}"
    );
    assert!(paused.contains("top-up settings to resume it"), "{paused}");
    assert!(!low_body.contains("token=") && !paused.contains("token="));
}

/// DN-1. A portal switch offers nothing to a customer who cannot pay: no saved payment method
/// and no card provider to add one with.
#[rstest]
#[tokio::test]
async fn the_portal_sells_credit_only_with_a_way_to_pay(#[future] test_env: TestEnv) {
    let env = test_env.await;
    portal_switches(&env, true).await;
    let capabilities = || {
        env.services()
            .portal_credit_capabilities(TENANT_ID, CUST_UBER_ID)
    };
    let unpayable = capabilities().await.expect("capabilities");
    assert!(!unpayable.top_up_enabled && !unpayable.auto_top_up_enabled);

    env.seed_payments().await;
    let payable = capabilities().await.expect("capabilities");
    assert!(payable.top_up_enabled && payable.auto_top_up_enabled);
}

/// RF-2. A card used to pay becomes the default only once a payment with it settles, and only
/// when the customer has none: a declined card never does, and an existing default stays.
#[rstest]
#[tokio::test]
async fn a_new_card_becomes_the_default_only_after_it_pays(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    create_customer_payment_method(
        env.pool(),
        CUST_UBER_ID,
        CUST_UBER_CONNECTION_ID,
        CUST_UBER_PAYMENT_METHOD_2_ID,
        "mock_pm_uber_card_2",
    )
    .await;
    let wallet = eur_wallet(&env, CUST_UBER_ID).await;

    env.services()
        .top_up_wallet(top_up(wallet, CUST_UBER_PAYMENT_METHOD_2_ID, "kept"), None)
        .await
        .expect("top-up");
    env.run_outbox_and_orchestration().await;
    assert_eq!(
        default_method(&env, CUST_UBER_ID).await,
        Some(CUST_UBER_PAYMENT_METHOD_ID),
        "another card paying never replaces the default"
    );

    set_default_method(&env, CUST_UBER_ID, None).await;
    set_mock_charge_behavior(env.pool(), MockChargeBehavior::Failed).await;
    let declined = env
        .services()
        .top_up_wallet(
            top_up(wallet, CUST_UBER_PAYMENT_METHOD_2_ID, "declined"),
            None,
        )
        .await
        .expect_err("declined");
    assert!(matches!(
        declined.current_context(),
        StoreError::FailedPrecondition(_)
    ));
    env.run_outbox_and_orchestration().await;
    assert_eq!(default_method(&env, CUST_UBER_ID).await, None);

    set_mock_charge_behavior(env.pool(), MockChargeBehavior::Succeeded).await;
    env.services()
        .top_up_wallet(top_up(wallet, CUST_UBER_PAYMENT_METHOD_2_ID, "paid"), None)
        .await
        .expect("top-up");
    env.run_outbox_and_orchestration().await;
    assert_eq!(
        default_method(&env, CUST_UBER_ID).await,
        Some(CUST_UBER_PAYMENT_METHOD_2_ID)
    );

    // Filling a missing default never replaces a usable one; a method the customer sets up on its
    // own (portal settings, a hosted mandate) does.
    let adopt = |adoption| {
        env.services().adopt_default_payment_method(
            Actor::System,
            TENANT_ID,
            CUST_UBER_ID,
            CUST_UBER_PAYMENT_METHOD_ID,
            adoption,
        )
    };
    assert!(!adopt(DefaultMethodAdoption::IfNone).await.expect("adopt"));
    assert_eq!(
        default_method(&env, CUST_UBER_ID).await,
        Some(CUST_UBER_PAYMENT_METHOD_2_ID)
    );
    assert!(adopt(DefaultMethodAdoption::Replace).await.expect("adopt"));
    assert_eq!(
        default_method(&env, CUST_UBER_ID).await,
        Some(CUST_UBER_PAYMENT_METHOD_ID)
    );
}

/// Adds a card through the portal, as its payment panel (`for_payment`) or settings do.
async fn portal_add_card(
    env: &TestEnv,
    resource: ResourceAccess,
    external_id: &str,
    for_payment: bool,
) -> CustomerPaymentMethodId {
    use meteroid_grpc::meteroid::portal::shared::v1 as shared;
    let token = meteroid_store::jwt_claims::generate_portal_token(
        &env.setup.config.jwt_secret,
        TENANT_ID,
        resource,
        None,
        None,
        false,
    )
    .expect("portal token");
    let mut request = tonic::Request::new(shared::AddPaymentMethodRequest {
        connection_id: CUST_UBER_CONNECTION_ID.as_base62(),
        external_payment_method_id: external_id.to_owned(),
        for_payment,
    });
    request
        .metadata_mut()
        .insert("x-portal-token", token.parse().expect("token header"));
    let method = shared::portal_shared_service_client::PortalSharedServiceClient::new(
        env.setup.channel.clone(),
    )
    .add_payment_method(request)
    .await
    .expect("card added")
    .into_inner()
    .payment_method
    .expect("method");
    CustomerPaymentMethodId::parse_base62(&method.id).expect("method id")
}

async fn saved_for_payment(env: &TestEnv, method: CustomerPaymentMethodId) -> bool {
    let mut conn = env.conn().await;
    diesel_models::customer_payment_methods::CustomerPaymentMethodRow::get_by_id(
        &mut conn, &TENANT_ID, &method,
    )
    .await
    .expect("method")
    .saved_for_payment_at
    .is_some()
}

/// Paying on-session with an older saved card never changes the default: only a card saved for
/// that payment does.
#[rstest]
#[tokio::test]
async fn paying_with_an_older_saved_card_keeps_the_default(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let (_, invoice_id) = env.unpaid_card_invoice().await;
    create_customer_payment_method(
        env.pool(),
        CUST_UBER_ID,
        CUST_UBER_CONNECTION_ID,
        CUST_UBER_PAYMENT_METHOD_2_ID,
        "mock_pm_uber_card_2",
    )
    .await;
    assert_eq!(
        default_method(&env, CUST_UBER_ID).await,
        Some(CUST_UBER_PAYMENT_METHOD_ID)
    );

    let (paid, _) = env
        .services()
        .complete_invoice_payment(
            TENANT_ID,
            invoice_id,
            CUST_UBER_PAYMENT_METHOD_2_ID,
            true,
            None,
        )
        .await
        .expect("paid");
    assert_eq!(paid.status, PaymentStatusEnum::Settled);
    env.run_outbox_and_orchestration().await;
    assert_eq!(
        env.store()
            .get_invoice_by_id(TENANT_ID, invoice_id)
            .await
            .expect("invoice")
            .payment_status,
        InvoicePaymentStatus::Paid
    );
    assert_eq!(
        default_method(&env, CUST_UBER_ID).await,
        Some(CUST_UBER_PAYMENT_METHOD_ID)
    );
}

/// A card saved in the portal to pay an invoice becomes the default once that payment settles; a
/// declined one never does. A card added in the portal settings is the default at once.
#[rstest]
#[tokio::test]
async fn a_card_saved_to_pay_an_invoice_becomes_the_default_once_it_pays(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let (_, invoice_id) = env.unpaid_card_invoice().await;

    set_mock_charge_behavior(env.pool(), MockChargeBehavior::Failed).await;
    let declined_card = portal_add_card(
        &env,
        ResourceAccess::Invoice(invoice_id),
        "mock_pm_uber_declined",
        true,
    )
    .await;
    assert!(saved_for_payment(&env, declined_card).await);
    assert_eq!(
        default_method(&env, CUST_UBER_ID).await,
        Some(CUST_UBER_PAYMENT_METHOD_ID),
        "saved to pay, not yet the default"
    );
    if let Ok((declined, _)) = env
        .services()
        .complete_invoice_payment(TENANT_ID, invoice_id, declined_card, true, None)
        .await
    {
        assert_ne!(declined.status, PaymentStatusEnum::Settled);
    }
    env.run_outbox_and_orchestration().await;
    assert_eq!(
        default_method(&env, CUST_UBER_ID).await,
        Some(CUST_UBER_PAYMENT_METHOD_ID),
        "a declined card never becomes the default"
    );

    set_mock_charge_behavior(env.pool(), MockChargeBehavior::Succeeded).await;
    let new_card = portal_add_card(
        &env,
        ResourceAccess::Invoice(invoice_id),
        "mock_pm_uber_new",
        true,
    )
    .await;
    let (paid, _) = env
        .services()
        .complete_invoice_payment(TENANT_ID, invoice_id, new_card, true, None)
        .await
        .expect("paid");
    assert_eq!(paid.status, PaymentStatusEnum::Settled);
    env.run_outbox_and_orchestration().await;
    assert_eq!(default_method(&env, CUST_UBER_ID).await, Some(new_card));
    assert!(
        !saved_for_payment(&env, new_card).await,
        "its first payment used the mark"
    );

    let settings_card = portal_add_card(
        &env,
        ResourceAccess::Invoice(invoice_id),
        "mock_pm_uber_settings",
        false,
    )
    .await;
    assert_eq!(
        default_method(&env, CUST_UBER_ID).await,
        Some(settings_card)
    );
    assert!(!saved_for_payment(&env, settings_card).await);
}

/// The provider's webhook for a setup the customer confirmed in the portal (Stripe's
/// `setup_intent.succeeded`), carrying the setup intent's purpose.
async fn setup_webhook(env: &TestEnv, external_id: &str, purpose: Option<&str>) {
    let purpose = purpose
        .map(|purpose| format!(r#","setup_purpose":"{purpose}""#))
        .unwrap_or_default();
    env.deliver_mock_webhook(
        format!(
            r#"{{"id":"evt_attach_{external_id}","kind":"payment_method_attached","external_payment_method_id":"{external_id}","external_customer_id":"mock_cus_uber","connection_id":"{connection}","customer_id":"{customer}"{purpose}}}"#,
            connection = CUST_UBER_CONNECTION_ID.as_base62(),
            customer = CUST_UBER_ID.as_base62(),
        )
        .as_bytes(),
    )
    .await;
}

async fn method_with_external_id(env: &TestEnv, external_id: &str) -> CustomerPaymentMethodId {
    let mut conn = env.conn().await;
    diesel_models::customer_payment_methods::CustomerPaymentMethodRow::get_by_external_id(
        &mut conn,
        &TENANT_ID,
        external_id,
    )
    .await
    .expect("lookup")
    .expect("method recorded")
    .id
}

/// The setup webhook alone decides when the browser never reports the card: one added in the
/// portal settings is the default, one saved to pay is not (yet), and a setup intent created
/// before setups carried a purpose keeps the earlier rule (the default).
#[rstest]
#[tokio::test]
async fn the_setup_webhook_adopts_a_card_by_what_it_was_set_up_for(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    setup_webhook(&env, "mock_pm_uber_settings_hook", Some("settings")).await;
    let settings_card = method_with_external_id(&env, "mock_pm_uber_settings_hook").await;
    assert_eq!(
        default_method(&env, CUST_UBER_ID).await,
        Some(settings_card)
    );
    assert!(!saved_for_payment(&env, settings_card).await);

    setup_webhook(&env, "mock_pm_uber_payment_hook", Some("payment")).await;
    let payment_card = method_with_external_id(&env, "mock_pm_uber_payment_hook").await;
    assert_eq!(
        default_method(&env, CUST_UBER_ID).await,
        Some(settings_card),
        "saved to pay: the default only once that payment settles"
    );
    assert!(saved_for_payment(&env, payment_card).await);

    setup_webhook(&env, "mock_pm_uber_legacy_hook", None).await;
    let legacy_card = method_with_external_id(&env, "mock_pm_uber_legacy_hook").await;
    assert_eq!(default_method(&env, CUST_UBER_ID).await, Some(legacy_card));
}

/// The setup webhook can record a card saved to pay before the portal reports it: the portal gets
/// that same card back, and it becomes the default once its payment settles.
#[rstest]
#[tokio::test]
async fn a_card_saved_to_pay_whose_webhook_came_first_becomes_the_default_once_it_pays(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let (_, invoice_id) = env.unpaid_card_invoice().await;

    setup_webhook(&env, "mock_pm_uber_raced", Some("payment")).await;
    let recorded = method_with_external_id(&env, "mock_pm_uber_raced").await;
    let reported = portal_add_card(
        &env,
        ResourceAccess::Invoice(invoice_id),
        "mock_pm_uber_raced",
        true,
    )
    .await;
    assert_eq!(reported, recorded);
    assert_eq!(
        default_method(&env, CUST_UBER_ID).await,
        Some(CUST_UBER_PAYMENT_METHOD_ID)
    );

    let (paid, _) = env
        .services()
        .complete_invoice_payment(TENANT_ID, invoice_id, reported, true, None)
        .await
        .expect("paid");
    assert_eq!(paid.status, PaymentStatusEnum::Settled);
    env.run_outbox_and_orchestration().await;
    assert_eq!(default_method(&env, CUST_UBER_ID).await, Some(reported));
}

/// A card counts as saved for a payment for a day: a retry with it in the same session replaces
/// the default, but one saved for a payment that failed and paying weeks later only fills one.
#[rstest]
#[tokio::test]
async fn a_card_saved_to_pay_counts_as_new_for_a_day(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let (_, invoice_id) = env.unpaid_card_invoice().await;
    let retried = portal_add_card(
        &env,
        ResourceAccess::Invoice(invoice_id),
        "mock_pm_uber_retried",
        true,
    )
    .await;
    set_mock_charge_behavior(env.pool(), MockChargeBehavior::Failed).await;
    let _declined = env
        .services()
        .complete_invoice_payment(TENANT_ID, invoice_id, retried, true, None)
        .await;
    env.run_outbox_and_orchestration().await;
    set_mock_charge_behavior(env.pool(), MockChargeBehavior::Succeeded).await;
    let (paid, _) = env
        .services()
        .complete_invoice_payment(TENANT_ID, invoice_id, retried, true, None)
        .await
        .expect("paid on retry");
    assert_eq!(paid.status, PaymentStatusEnum::Settled);
    env.run_outbox_and_orchestration().await;
    assert_eq!(default_method(&env, CUST_UBER_ID).await, Some(retried));

    let weeks_old = portal_add_card(
        &env,
        ResourceAccess::Customer(CUST_UBER_ID),
        "mock_pm_uber_weeks_old",
        true,
    )
    .await;
    {
        use diesel::{ExpressionMethods, QueryDsl};
        use diesel_async::RunQueryDsl;
        use diesel_models::schema::customer_payment_method::dsl as cpm;
        let mut conn = env.conn().await;
        diesel::update(cpm::customer_payment_method.filter(cpm::id.eq(weeks_old)))
            .set(cpm::saved_for_payment_at.eq(Some(Utc::now() - chrono::Duration::days(21))))
            .execute(&mut conn)
            .await
            .expect("age the mark");
    }
    let wallet = eur_wallet(&env, CUST_UBER_ID).await;
    let receipt = env
        .services()
        .top_up_wallet(top_up(wallet, weeks_old, "weeks-old"), None)
        .await
        .expect("top-up");
    assert_eq!(receipt.transaction.status, PaymentStatusEnum::Settled);
    env.run_outbox_and_orchestration().await;
    assert_eq!(
        default_method(&env, CUST_UBER_ID).await,
        Some(retried),
        "saved weeks ago: an older card"
    );
    assert!(!saved_for_payment(&env, weeks_old).await);
}

/// A direct debit mandate saved in the portal to pay an invoice settles three days later, as SEPA
/// does: it becomes the default then, its day counted from when the payment started.
#[rstest]
#[tokio::test]
async fn a_mandate_saved_to_pay_an_invoice_becomes_the_default_when_it_settles_days_later(
    #[future] test_env: TestEnv,
) {
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_async::RunQueryDsl;
    use diesel_models::schema::customer_payment_method::dsl as cpm;
    use diesel_models::schema::payment_transaction::dsl as pt;

    let env = test_env.await;
    let (_, invoice_id) = env.unpaid_card_invoice().await;
    let mandate = portal_add_card(
        &env,
        ResourceAccess::Invoice(invoice_id),
        "mock_pm_uber_sepa_mandate",
        true,
    )
    .await;
    set_mock_charge_behavior(env.pool(), MockChargeBehavior::Pending).await;
    let (pending, _) = env
        .services()
        .complete_invoice_payment(TENANT_ID, invoice_id, mandate, true, None)
        .await
        .expect("accepted");
    assert_eq!(pending.status, PaymentStatusEnum::Pending);
    env.run_outbox_and_orchestration().await;
    assert_eq!(
        default_method(&env, CUST_UBER_ID).await,
        Some(CUST_UBER_PAYMENT_METHOD_ID),
        "not before it settles"
    );

    let saved_at = Utc::now() - chrono::Duration::days(3);
    {
        let mut conn = env.conn().await;
        diesel::update(cpm::customer_payment_method.filter(cpm::id.eq(mandate)))
            .set(cpm::saved_for_payment_at.eq(Some(saved_at)))
            .execute(&mut conn)
            .await
            .expect("saved three days ago");
        diesel::update(pt::payment_transaction.filter(pt::id.eq(pending.id)))
            .set(pt::created_at.eq(saved_at + chrono::Duration::minutes(1)))
            .execute(&mut conn)
            .await
            .expect("started a minute later");
    }
    let external_id = pending
        .provider_transaction_id
        .clone()
        .expect("a provider id");
    env.deliver_mock_webhook(
        format!(
            r#"{{"id":"evt_ok_{tx}","kind":"payment_succeeded","transaction_id":"{tx}","external_id":"{external_id}","amount":{amount},"currency":"{currency}"}}"#,
            tx = pending.id.as_base62(),
            amount = pending.amount,
            currency = pending.currency,
        )
        .as_bytes(),
    )
    .await;
    assert_eq!(
        env.store()
            .get_invoice_by_id(TENANT_ID, invoice_id)
            .await
            .expect("invoice")
            .payment_status,
        InvoicePaymentStatus::Paid
    );
    assert_eq!(default_method(&env, CUST_UBER_ID).await, Some(mandate));
    assert!(!saved_for_payment(&env, mandate).await);
}

/// A card saved to pay 30 hours before the payment started is an older card: paying with it
/// leaves the default.
#[rstest]
#[tokio::test]
async fn a_card_saved_a_day_before_its_payment_started_stays_an_older_card(
    #[future] test_env: TestEnv,
) {
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_async::RunQueryDsl;
    use diesel_models::schema::customer_payment_method::dsl as cpm;

    let env = test_env.await;
    let (_, invoice_id) = env.unpaid_card_invoice().await;
    let card = portal_add_card(
        &env,
        ResourceAccess::Invoice(invoice_id),
        "mock_pm_uber_saved_yesterday",
        true,
    )
    .await;
    diesel::update(cpm::customer_payment_method.filter(cpm::id.eq(card)))
        .set(cpm::saved_for_payment_at.eq(Some(Utc::now() - chrono::Duration::hours(30))))
        .execute(&mut env.conn().await)
        .await
        .expect("saved 30 hours ago");

    let (paid, _) = env
        .services()
        .complete_invoice_payment(TENANT_ID, invoice_id, card, true, None)
        .await
        .expect("paid");
    assert_eq!(paid.status, PaymentStatusEnum::Settled);
    env.run_outbox_and_orchestration().await;
    assert_eq!(
        default_method(&env, CUST_UBER_ID).await,
        Some(CUST_UBER_PAYMENT_METHOD_ID)
    );
    assert!(!saved_for_payment(&env, card).await, "the mark is used up");
}

/// A top-up the customer makes in the portal is on-session: a card newly saved for it becomes the
/// default once it settles, as for an invoice payment.
#[rstest]
#[tokio::test]
async fn a_card_saved_for_a_portal_top_up_becomes_the_default_once_it_settles(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.seed_payments().await;
    let wallet = eur_wallet(&env, CUST_UBER_ID).await;
    let new_card = portal_add_card(
        &env,
        ResourceAccess::Customer(CUST_UBER_ID),
        "mock_pm_uber_top_up",
        true,
    )
    .await;
    assert_eq!(
        default_method(&env, CUST_UBER_ID).await,
        Some(CUST_UBER_PAYMENT_METHOD_ID)
    );

    let receipt = env
        .services()
        .top_up_wallet(top_up(wallet, new_card, "new-card"), None)
        .await
        .expect("top-up");
    assert_eq!(receipt.transaction.status, PaymentStatusEnum::Settled);
    env.run_outbox_and_orchestration().await;
    assert_eq!(default_method(&env, CUST_UBER_ID).await, Some(new_card));
}

/// RF-2, finding 1. A checkout charged on the spot records no settlement event: the card the new
/// customer paid with is their default when the checkout completes.
#[rstest]
#[tokio::test]
async fn a_self_serve_checkout_leaves_its_card_as_the_default(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    set_default_method(&env, CUST_SPOTIFY_ID, None).await;
    let session = self_serve_session(&env, CUST_SPOTIFY_ID).await;
    let checkout = checkout_with_code(&env, session, None)
        .await
        .expect("checkout");
    let result = env
        .services()
        .complete_checkout(
            TENANT_ID,
            session,
            CUST_SPOTIFY_PAYMENT_METHOD_ID,
            checkout.amount_due,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("checkout");
    assert!(
        matches!(result, CheckoutCompletionResult::Completed { .. }),
        "{result:?}"
    );
    assert_eq!(
        default_method(&env, CUST_SPOTIFY_ID).await,
        Some(CUST_SPOTIFY_PAYMENT_METHOD_ID)
    );
}

/// A subscription checkout makes the card the customer completes it with the default, an older
/// saved card included: the subscription's renewals bill the customer's default.
#[rstest]
#[tokio::test]
async fn a_subscription_checkout_makes_an_older_saved_card_the_default(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.seed_payments().await;
    let other_card = CustomerPaymentMethodId::new();
    create_customer_payment_method(
        env.pool(),
        CUST_SPOTIFY_ID,
        CUST_SPOTIFY_CONNECTION_ID,
        other_card,
        "mock_pm_spotify_other",
    )
    .await;
    set_default_method(&env, CUST_SPOTIFY_ID, Some(other_card)).await;

    let session = self_serve_session(&env, CUST_SPOTIFY_ID).await;
    let checkout = checkout_with_code(&env, session, None)
        .await
        .expect("checkout");
    let result = env
        .services()
        .complete_checkout(
            TENANT_ID,
            session,
            CUST_SPOTIFY_PAYMENT_METHOD_ID,
            checkout.amount_due,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("checkout");
    assert!(
        matches!(result, CheckoutCompletionResult::Completed { .. }),
        "{result:?}"
    );
    env.run_outbox_and_orchestration().await;
    assert_eq!(
        default_method(&env, CUST_SPOTIFY_ID).await,
        Some(CUST_SPOTIFY_PAYMENT_METHOD_ID)
    );
}

fn welcome_credit() -> CouponNew {
    CouponNew {
        code: "WELCOME10".to_owned(),
        description: "Welcome credit".to_owned(),
        tenant_id: TENANT_ID,
        discount: CouponDiscount::Credit {
            currency: "EUR".to_owned(),
            amount: dec!(10),
            expires_in_days: Some(60),
            scope: CouponCreditScope::Customer,
        },
        expires_at: None,
        redemption_limit: None,
        recurring_value: None,
        reusable: false,
        plan_ids: vec![],
    }
}

async fn self_serve_session(
    env: &TestEnv,
    customer: CustomerId,
) -> common_domain::ids::CheckoutSessionId {
    env.store()
        .create_checkout_session(session_with_code(customer, None))
        .await
        .expect("checkout session")
        .id
}

fn session_with_code(customer: CustomerId, coupon_code: Option<&str>) -> CreateCheckoutSession {
    CreateCheckoutSession {
        tenant_id: TENANT_ID,
        customer_id: customer,
        plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
        billing_start_date: Some(NaiveDate::from_ymd_opt(2024, 1, 15).unwrap()),
        billing_day_anchor: Some(15),
        net_terms: None,
        trial_duration_days: Some(0),
        end_date: None,
        auto_advance_invoices: true,
        charge_automatically: true,
        invoice_memo: None,
        invoice_threshold: None,
        purchase_order: None,
        payment_methods_config: None,
        components: None,
        add_ons: None,
        coupon_code: coupon_code.map(str::to_owned),
        coupon_ids: vec![],
        expires_in_hours: Some(24),
        metadata: None,
        checkout_type: CheckoutType::SelfServe,
        subscription_id: None,
        change_date: None,
        success_url: None,
        cancel_url: None,
    }
}

async fn checkout_with_code(
    env: &TestEnv,
    session: common_domain::ids::CheckoutSessionId,
    code: Option<&str>,
) -> Result<portal::Checkout, tonic::Status> {
    let token = meteroid_store::jwt_claims::generate_portal_token(
        &env.setup.config.jwt_secret,
        TENANT_ID,
        ResourceAccess::CheckoutSession(session),
        None,
        None,
        false,
    )
    .expect("portal token");
    let mut client = portal::portal_checkout_service_client::PortalCheckoutServiceClient::new(
        env.setup.channel.clone(),
    );
    let mut request = tonic::Request::new(portal::GetCheckoutRequest {
        coupon_code: code.map(str::to_string),
    });
    request
        .metadata_mut()
        .insert("x-portal-token", token.parse().expect("token header"));
    client
        .get_checkout(request)
        .await
        .map(|response| response.into_inner().checkout.expect("checkout"))
}

/// RF-7. A promo code the customer already redeemed is refused when typed at checkout, as the
/// payment would refuse it; a fresh one shows the credit it adds. A payment that settles later
/// keeps the code on the session, so the subscription it builds carries it.
#[rstest]
#[tokio::test]
async fn promo_codes_at_checkout_are_checked_and_shown(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    env.create_coupon(welcome_credit()).await;
    subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(Utc::now().date_naive())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.services()
        .redeem_coupon_credit_code(TENANT_ID, CUST_UBER_ID, "WELCOME10", None)
        .await
        .expect("redeemed in the portal");

    let session = self_serve_session(&env, CUST_UBER_ID).await;
    let refused = checkout_with_code(&env, session, Some("WELCOME10"))
        .await
        .expect_err("already redeemed");
    assert_eq!(refused.code(), Code::InvalidArgument);
    assert_eq!(refused.message(), "You've already redeemed this code.");

    let fresh = self_serve_session(&env, CUST_SPOTIFY_ID).await;
    let checkout = checkout_with_code(&env, fresh, Some("WELCOME10"))
        .await
        .expect("checkout");
    assert!(checkout.applied_coupons.is_empty(), "it discounts nothing");
    assert_eq!(checkout.credit_coupons.len(), 1);
    let credit = &checkout.credit_coupons[0];
    assert_eq!(
        (
            credit.coupon_code.as_str(),
            credit.amount,
            credit.currency.as_str()
        ),
        ("WELCOME10", 1_000, "EUR")
    );
    assert_eq!(credit.expires_in_days, Some(60));

    set_mock_charge_behavior(env.pool(), MockChargeBehavior::Pending).await;
    let result = env
        .services()
        .complete_checkout(
            TENANT_ID,
            fresh,
            CUST_SPOTIFY_PAYMENT_METHOD_ID,
            checkout.amount_due,
            "EUR".to_string(),
            Some("WELCOME10".to_string()),
        )
        .await
        .expect("checkout");
    assert!(
        matches!(result, CheckoutCompletionResult::AwaitingPayment { .. }),
        "{result:?}"
    );
    let stored = env
        .store()
        .get_checkout_session(TENANT_ID, fresh)
        .await
        .expect("session");
    assert_eq!(stored.coupon_code.as_deref(), Some("WELCOME10"));
    assert_eq!(stored.coupon_ids.len(), 1);

    // The redemption is held while that payment settles: a second checkout cannot take it too.
    let second = self_serve_session(&env, CUST_SPOTIFY_ID).await;
    let held = env
        .services()
        .complete_checkout(
            TENANT_ID,
            second,
            CUST_SPOTIFY_PAYMENT_METHOD_ID,
            checkout.amount_due,
            "EUR".to_string(),
            Some("WELCOME10".to_string()),
        )
        .await
        .expect_err("held by the checkout awaiting its payment");
    assert!(
        format!("{held:?}").contains("Coupon WELCOME10 has already been used by this customer"),
        "{held:?}"
    );

    // The payment fails: the session gets back the coupons it had before the attempt.
    {
        use diesel::{ExpressionMethods, QueryDsl};
        use diesel_async::RunQueryDsl;
        use diesel_models::schema::payment_transaction::dsl as pt;
        let mut conn = env.conn().await;
        diesel::update(pt::payment_transaction.filter(pt::checkout_session_id.eq(fresh)))
            .set(pt::status.eq(diesel_models::enums::PaymentStatusEnum::Failed))
            .execute(&mut conn)
            .await
            .expect("fail the payment");
        diesel_models::checkout_sessions::CheckoutSessionRow::reopen_after_failed_payment(
            &mut conn, TENANT_ID, fresh,
        )
        .await
        .expect("reopen")
        .expect("reopened");
    }
    let reopened = env
        .store()
        .get_checkout_session(TENANT_ID, fresh)
        .await
        .expect("session");
    assert_eq!(reopened.coupon_code, None);
    assert!(reopened.coupon_ids.is_empty());
}

/// A checkout session's code is checked when the session is created, exact then ignoring the case
/// of ASCII letters, and again before the customer is charged: a code that stopped naming a live
/// coupon refuses the payment instead of charging full price.
#[rstest]
#[tokio::test]
async fn a_checkout_code_is_checked_at_creation_and_before_the_charge(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    let percent = |code: &str| CouponNew {
        code: code.to_owned(),
        description: code.to_owned(),
        tenant_id: TENANT_ID,
        discount: CouponDiscount::Percentage(dec!(10)),
        expires_at: None,
        redemption_limit: None,
        recurring_value: None,
        reusable: true,
        plan_ids: vec![],
    };
    env.create_coupon(percent("Summer10")).await;
    env.create_coupon(percent("SUMMER10")).await;
    let spring = env.create_coupon(percent("Spring5")).await;

    for (code, refusal) in [
        ("nope", "unknown coupon codes: nope"),
        (
            "summer10",
            "differ only in case, send the exact code: summer10",
        ),
    ] {
        let refused = env
            .store()
            .create_checkout_session(session_with_code(CUST_SPOTIFY_ID, Some(code)))
            .await
            .expect_err(code);
        assert!(
            matches!(refused.current_context(), StoreError::InvalidArgument(message) if message.contains(refusal)),
            "{refused:?}"
        );
    }
    let exact = env
        .store()
        .create_checkout_session(session_with_code(CUST_SPOTIFY_ID, Some("Summer10")))
        .await
        .expect("an exact code wins");
    assert_eq!(exact.coupon_code.as_deref(), Some("Summer10"));

    let session = env
        .store()
        .create_checkout_session(session_with_code(CUST_SPOTIFY_ID, Some("spring5")))
        .await
        .expect("matched ignoring case")
        .id;
    let checkout = checkout_with_code(&env, session, None)
        .await
        .expect("checkout");
    assert_eq!(checkout.applied_coupons.len(), 1);

    env.archive_coupon(spring).await;
    let refused = env
        .services()
        .complete_checkout(
            TENANT_ID,
            session,
            CUST_SPOTIFY_PAYMENT_METHOD_ID,
            checkout.amount_due,
            "EUR".to_string(),
            None,
        )
        .await
        .expect_err("the code no longer names a live coupon");
    assert!(
        matches!(
            refused.current_context(),
            StoreError::InvalidArgument(message) if message == "Coupon code 'spring5' is not valid"
        ),
        "{refused:?}"
    );
    assert!(
        env.get_transactions_by_checkout_session(session)
            .await
            .is_empty(),
        "nothing is charged"
    );
}

/// A checkout session created with a code a previous release stored padded ("SAVE10 ", beside a
/// newer "SAVE10") completes with the coupon it was created against, matched as sent.
#[rstest]
#[tokio::test]
async fn a_checkout_carrying_a_padded_legacy_code_completes_with_its_coupon(
    #[future] test_env: TestEnv,
) {
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_async::RunQueryDsl;
    use diesel_models::schema::coupon::dsl as c;

    let env = test_env.await;
    env.seed_payments().await;
    let percent = |code: &str| CouponNew {
        code: code.to_owned(),
        description: code.to_owned(),
        tenant_id: TENANT_ID,
        discount: CouponDiscount::Percentage(dec!(10)),
        expires_at: None,
        redemption_limit: None,
        recurring_value: None,
        reusable: true,
        plan_ids: vec![],
    };
    let legacy = env.create_coupon(percent("SAVE10-LEGACY")).await;
    diesel::update(c::coupon.filter(c::id.eq(legacy)))
        .set(c::code.eq("SAVE10 "))
        .execute(&mut env.conn().await)
        .await
        .expect("stored padded, as before codes were validated");
    env.create_coupon(percent("SAVE10")).await;

    let session = env
        .store()
        .create_checkout_session(session_with_code(CUST_SPOTIFY_ID, Some("SAVE10 ")))
        .await
        .expect("the padded code names its coupon exactly")
        .id;
    let checkout = checkout_with_code(&env, session, None)
        .await
        .expect("checkout");
    env.services()
        .complete_checkout(
            TENANT_ID,
            session,
            CUST_SPOTIFY_PAYMENT_METHOD_ID,
            checkout.amount_due,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("completed");
    assert_eq!(
        env.store()
            .get_checkout_session(TENANT_ID, session)
            .await
            .expect("session")
            .coupon_ids,
        vec![legacy]
    );
}

/// Finding 4. A coupon limited to one redemption, held by a checkout whose direct debit still
/// settles, is refused to a second in-flight checkout.
#[rstest]
#[tokio::test]
async fn a_limited_coupon_is_held_by_a_checkout_awaiting_payment(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    env.create_coupon(CouponNew {
        code: "ONLYONE".to_owned(),
        description: "One redemption".to_owned(),
        tenant_id: TENANT_ID,
        discount: CouponDiscount::Percentage(dec!(10)),
        expires_at: None,
        redemption_limit: Some(1),
        recurring_value: None,
        reusable: true,
        plan_ids: vec![],
    })
    .await;
    set_mock_charge_behavior(env.pool(), MockChargeBehavior::Pending).await;
    let confirm = |method, session, amount_due| {
        env.services().complete_checkout(
            TENANT_ID,
            session,
            method,
            amount_due,
            "EUR".to_string(),
            Some("ONLYONE".to_string()),
        )
    };
    let first = self_serve_session(&env, CUST_SPOTIFY_ID).await;
    let quoted = checkout_with_code(&env, first, Some("ONLYONE"))
        .await
        .expect("checkout")
        .amount_due;
    assert!(matches!(
        confirm(CUST_SPOTIFY_PAYMENT_METHOD_ID, first, quoted)
            .await
            .expect("first"),
        CheckoutCompletionResult::AwaitingPayment { .. }
    ));
    let second = self_serve_session(&env, CUST_UBER_ID).await;
    let refused = confirm(CUST_UBER_PAYMENT_METHOD_ID, second, quoted)
        .await
        .expect_err("the only redemption is held");
    assert!(
        format!("{refused:?}").contains("Coupon 'ONLYONE' has reached its redemption limit"),
        "{refused:?}"
    );
}
