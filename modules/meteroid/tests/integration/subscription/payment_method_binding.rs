//! An invoice is only charged to a payment method of its own customer, whoever picks the method
//! (portal pay link, customer portal, automatic collection).

use chrono::NaiveDate;
use rstest::rstest;

use common_domain::ids::{BaseId, CustomerPaymentMethodId, PaymentTransactionId};
use meteroid_store::domain::PaymentStatusEnum;

use crate::data::ids::*;
use crate::harness::{InvoicesAssertExt, TestEnv, subscription, test_env};

async fn transaction_count(env: &TestEnv) -> i64 {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    use diesel_models::schema::payment_transaction::dsl as pt;

    pt::payment_transaction
        .filter(pt::tenant_id.eq(TENANT_ID))
        .count()
        .get_result(&mut env.conn().await)
        .await
        .expect("count payment transactions")
}

#[rstest]
#[tokio::test]
async fn test_invoice_payment_rejects_other_customers_method(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    // Uber's card subscription, first invoice finalized and unpaid.
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .no_auto_charge()
        .create(env.services())
        .await;
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().invoice_at(0).is_finalized_unpaid();
    let invoice_id = invoices[0].id;
    let before = transaction_count(&env).await;

    for foreign in [
        CUST_SPOTIFY_PAYMENT_METHOD_ID,
        CustomerPaymentMethodId::new(),
    ] {
        let err = env
            .services()
            .complete_invoice_payment(TENANT_ID, invoice_id, foreign, true, None)
            .await
            .expect_err("a method of another customer is never charged");
        assert!(
            err.to_string().contains("Payment method not found"),
            "{err:?}"
        );
    }
    assert_eq!(transaction_count(&env).await, before, "nothing was charged");

    let (tx, _) = env
        .services()
        .complete_invoice_payment(
            TENANT_ID,
            invoice_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            true,
            None,
        )
        .await
        .expect("the customer's own method is charged");
    assert_eq!(tx.status, PaymentStatusEnum::Settled);
}

#[rstest]
#[tokio::test]
async fn test_archived_method_is_not_charged_but_keeps_its_attempt(#[future] test_env: TestEnv) {
    use diesel_models::customer_payment_methods::CustomerPaymentMethodRow;
    use diesel_models::enums::{PaymentStatusEnum as RowStatus, PaymentTypeEnum};
    use diesel_models::payments::PaymentTransactionRowNew;
    use meteroid_store::services::is_payment_method_unusable;

    let env = test_env.await;
    env.seed_payments().await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .no_auto_charge()
        .create(env.services())
        .await;
    let invoice = env.get_invoices(sub_id).await.remove(0);

    CustomerPaymentMethodRow::archive(
        &mut env.conn().await,
        &TENANT_ID,
        &CUST_UBER_PAYMENT_METHOD_ID,
    )
    .await
    .expect("archive the method");

    // A queued charge on a method removed since can't succeed on a retry.
    let before = transaction_count(&env).await;
    let err = env
        .services()
        .complete_invoice_payment_automatic(
            TENANT_ID,
            invoice.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            None,
        )
        .await
        .expect_err("an archived method is never charged");
    assert!(is_payment_method_unusable(&err), "{err:?}");
    assert_eq!(transaction_count(&env).await, before, "nothing was charged");

    // A charge already in flight on that method is still returned as is on a redelivery.
    let pending = PaymentTransactionRowNew {
        id: PaymentTransactionId::new(),
        tenant_id: TENANT_ID,
        invoice_id: Some(invoice.id),
        provider_transaction_id: None,
        amount: invoice.amount_due,
        currency: invoice.currency.clone(),
        payment_method_id: Some(CUST_UBER_PAYMENT_METHOD_ID),
        status: RowStatus::Pending,
        payment_type: PaymentTypeEnum::Payment,
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
    .insert(&mut env.conn().await)
    .await
    .expect("insert the pending charge");

    let (tx, _) = env
        .services()
        .complete_invoice_payment_automatic(
            TENANT_ID,
            invoice.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            None,
        )
        .await
        .expect("the charge in flight is returned");
    assert_eq!(tx.id, pending.id);
    assert_eq!(tx.status, PaymentStatusEnum::Pending);
}
