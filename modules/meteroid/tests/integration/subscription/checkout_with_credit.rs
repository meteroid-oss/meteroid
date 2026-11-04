//! A self-serve checkout by a customer holding customer-wide paid credit: the portal quotes the
//! amount due after credit, the checkout charges exactly that, and the invoice draws the credit
//! the quote applied (Stripe Checkout with a customer balance).
use chrono::NaiveDate;
use rstest::rstest;

use crate::data::ids::*;
use crate::harness::{TestEnv, test_env};
use crate::helpers::credit::{Grant, assert_ledger_balances, fund_grant, grant_remaining};
use common_domain::ids::{CheckoutSessionId, CreditGrantId};
use diesel_models::payments::PaymentTransactionRow;
use meteroid_grpc::meteroid::portal::checkout::v1 as portal;
use meteroid_store::domain::checkout_sessions::{CheckoutCompletionResult, CheckoutType};
use meteroid_store::domain::{CreateCheckoutSession, InvoicePaymentStatus};
use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;

async fn leetcode_session(env: &TestEnv) -> CheckoutSessionId {
    env.store()
        .create_checkout_session(CreateCheckoutSession {
            tenant_id: TENANT_ID,
            customer_id: CUST_UBER_ID,
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
            coupon_code: None,
            coupon_ids: vec![],
            expires_in_hours: Some(24),
            metadata: None,
            checkout_type: CheckoutType::SelfServe,
            subscription_id: None,
            change_date: None,
            success_url: None,
            cancel_url: None,
        })
        .await
        .expect("checkout session")
        .id
}

/// (total, applied credits, amount due) as the portal quotes the checkout.
async fn quote(env: &TestEnv, session: CheckoutSessionId) -> (u64, u64, u64) {
    let token = meteroid_store::jwt_claims::generate_portal_token(
        &env.setup.config.jwt_secret,
        TENANT_ID,
        meteroid_store::jwt_claims::ResourceAccess::CheckoutSession(session),
        None,
        None,
        false,
    )
    .expect("portal token");
    let mut client = portal::portal_checkout_service_client::PortalCheckoutServiceClient::new(
        env.setup.channel.clone(),
    );
    let mut request = tonic::Request::new(portal::GetCheckoutRequest { coupon_code: None });
    request
        .metadata_mut()
        .insert("x-portal-token", token.parse().expect("token header"));
    let checkout = client
        .get_checkout(request)
        .await
        .expect("checkout page")
        .into_inner()
        .checkout
        .expect("checkout");
    (
        checkout.total_amount,
        checkout.applied_credits,
        checkout.amount_due,
    )
}

async fn credit(env: &TestEnv, amount: i64) -> CreditGrantId {
    fund_grant(
        env.store(),
        TENANT_ID,
        CUST_UBER_ID,
        None,
        Grant::invoice(amount),
    )
    .await
}

/// LeetCode bills 35.00; 10.00 of credit leaves 25.00. The quote, the charge and the invoice agree,
/// and the 35.00 the session confirmed before credit is refused before anything is charged.
#[rstest]
#[tokio::test]
async fn a_checkout_charges_what_customer_credit_leaves(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    let grant = credit(&env, 1_000).await;
    let session = leetcode_session(&env).await;

    assert_eq!(quote(&env, session).await, (3_500, 1_000, 2_500));

    let before_credit = env
        .services()
        .complete_checkout(
            TENANT_ID,
            session,
            CUST_UBER_PAYMENT_METHOD_ID,
            3_500,
            "EUR".to_string(),
            None,
        )
        .await
        .expect_err("the amount before credit is not what is due");
    assert!(format!("{before_credit:?}").contains("Amount mismatch: expected 2500, got 3500"));

    let result = env
        .services()
        .complete_checkout(
            TENANT_ID,
            session,
            CUST_UBER_PAYMENT_METHOD_ID,
            2_500,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("checkout");
    let CheckoutCompletionResult::Completed {
        subscription_id,
        transaction: Some(transaction),
    } = result
    else {
        panic!("a settled charge completes the checkout: {result:?}")
    };
    assert_eq!(transaction.amount, 2_500);

    let invoices = env.get_invoices(subscription_id).await;
    assert_eq!(invoices.len(), 1);
    let invoice = &invoices[0];
    assert_eq!(
        (invoice.total, invoice.applied_credits, invoice.amount_due),
        (3_500, 1_000, 0)
    );
    assert_eq!(invoice.payment_status, InvoicePaymentStatus::Paid);
    let mut conn = env.conn().await;
    let payments = PaymentTransactionRow::list_by_invoice_id(&mut conn, invoice.id, TENANT_ID)
        .await
        .expect("payments");
    assert_eq!(
        payments
            .iter()
            .map(|row| row.transaction.amount)
            .collect::<Vec<_>>(),
        vec![2_500],
        "charged once, what the credit left"
    );
    assert_eq!(
        grant_remaining(env.store(), TENANT_ID, CUST_UBER_ID, grant).await,
        0
    );
    assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// 50.00 of credit covers the 35.00: nothing is charged, the invoice draws 35.00 and is paid.
#[rstest]
#[tokio::test]
async fn a_checkout_covered_by_customer_credit_charges_nothing(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    let grant = credit(&env, 5_000).await;
    let session = leetcode_session(&env).await;

    assert_eq!(quote(&env, session).await, (3_500, 3_500, 0));

    let result = env
        .services()
        .complete_checkout(
            TENANT_ID,
            session,
            CUST_UBER_PAYMENT_METHOD_ID,
            0,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("checkout");
    let CheckoutCompletionResult::Completed {
        subscription_id,
        transaction: None,
    } = result
    else {
        panic!("nothing to charge: {result:?}")
    };

    let invoices = env.get_invoices(subscription_id).await;
    assert_eq!(invoices.len(), 1);
    let invoice = &invoices[0];
    assert_eq!(
        (invoice.total, invoice.applied_credits, invoice.amount_due),
        (3_500, 3_500, 0)
    );
    assert_eq!(invoice.payment_status, InvoicePaymentStatus::Paid);
    let mut conn = env.conn().await;
    assert!(
        PaymentTransactionRow::list_by_invoice_id(&mut conn, invoice.id, TENANT_ID)
            .await
            .expect("payments")
            .is_empty()
    );
    assert_eq!(
        grant_remaining(env.store(), TENANT_ID, CUST_UBER_ID, grant).await,
        1_500
    );
    assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

async fn rate_add_on(env: &TestEnv, rate_cents: i64) -> common_domain::ids::AddOnId {
    use meteroid_store::domain::BillingPeriodEnum;
    use meteroid_store::domain::enums::FeeTypeEnum;
    use meteroid_store::domain::price_components::{PriceEntry, PriceInput, ProductRef};
    use meteroid_store::domain::prices::{FeeStructure, Pricing};
    use meteroid_store::repositories::add_ons::AddOnInterface;
    env.store()
        .create_add_on_from_ref(
            "Priority support".to_string(),
            ProductRef::New {
                name: "Priority support".to_string(),
                fee_type: FeeTypeEnum::Rate,
                fee_structure: FeeStructure::Rate {},
            },
            PriceEntry::New(PriceInput {
                cadence: BillingPeriodEnum::Monthly,
                currency: "EUR".to_string(),
                pricing: Pricing::Rate {
                    rate: rust_decimal::Decimal::new(rate_cents, 2),
                },
                credit_unit: None,
            }),
            None,
            true,
            Some(10),
            TENANT_ID,
            PRODUCT_FAMILY_ID,
            vec![],
        )
        .await
        .expect("add-on")
        .id
}

/// An add-on bought through checkout that customer credit covers whole: nothing is charged, but
/// its invoice is issued, draws the credit and is paid by it (Stripe, Chargebee, Lago issue it
/// and mark it paid at zero due). It used to complete with no invoice and the credit untouched.
#[rstest]
#[tokio::test]
async fn an_add_on_covered_by_customer_credit_is_billed_and_paid_by_it(
    #[future] test_env: TestEnv,
) {
    use meteroid_store::domain::enums::{InvoiceStatusEnum, InvoiceType};
    let env = test_env.await;
    env.seed_payments().await;
    let subscription_id = crate::harness::subscription()
        .start_date(chrono::Utc::now().date_naive())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let add_on = rate_add_on(&env, 1_000).await;
    let grant = credit(&env, 5_000).await;
    let session = env
        .services()
        .create_addon_purchase_checkout_session(
            TENANT_ID,
            subscription_id,
            add_on,
            PLAN_VERSION_1_LEETCODE_ID,
            CUST_UBER_ID,
            None,
        )
        .await
        .expect("add-on checkout session")
        .id;

    let (total, applied, due) = quote(&env, session).await;
    assert!(total > 0, "the add-on bills its prorated fee");
    assert_eq!((applied, due), (total, 0), "credit covers it whole");

    let result = env
        .services()
        .complete_checkout(
            TENANT_ID,
            session,
            CUST_UBER_PAYMENT_METHOD_ID,
            0,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("checkout");
    let CheckoutCompletionResult::Completed {
        transaction: None, ..
    } = result
    else {
        panic!("nothing to charge: {result:?}")
    };

    let adjustments = env
        .get_invoices(subscription_id)
        .await
        .into_iter()
        .filter(|invoice| invoice.invoice_type == InvoiceType::Adjustment)
        .collect::<Vec<_>>();
    assert_eq!(adjustments.len(), 1, "the add-on is billed");
    let invoice = &adjustments[0];
    let total = i64::try_from(total).expect("an amount");
    assert_eq!(
        (invoice.total, invoice.applied_credits, invoice.amount_due),
        (total, total, 0)
    );
    assert_eq!(invoice.status, InvoiceStatusEnum::Finalized);
    assert_eq!(invoice.payment_status, InvoicePaymentStatus::Paid);
    assert!(invoice.paid_at.is_some(), "paid by its credit when issued");
    let mut conn = env.conn().await;
    assert!(
        PaymentTransactionRow::list_by_invoice_id(&mut conn, invoice.id, TENANT_ID)
            .await
            .expect("payments")
            .is_empty()
    );
    assert_eq!(
        grant_remaining(env.store(), TENANT_ID, CUST_UBER_ID, grant).await,
        5_000 - total
    );
    assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}
