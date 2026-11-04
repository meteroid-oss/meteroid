//! A path waiting for a customer held elsewhere holds none of the customer's rows, nor the tenant
//! rows other paths take after that customer. Each test holds one side of a former cycle, lets the
//! real path wait on it, then takes without waiting what that side takes next.
use chrono::{NaiveDate, Utc};
use common_domain::actor::Actor;
use common_domain::ids::BaseId;
use diesel::sql_types::BigInt;
use diesel_async::{RunQueryDsl, SimpleAsyncConnection};
use meteroid_store::domain::enums::ReversalKind;
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::connectors::ConnectorsInterface;
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::credit::customer_credit_wallets::DebitCustomerCreditGrant;
use meteroid_store::repositories::payment_reminders::PaymentRemindersInterface;
use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;
use meteroid_store::services::ReverseManualPaymentCommand;
use rstest::rstest;
use rust_decimal::Decimal;
use std::time::Duration;

use crate::data::ids::{
    CUST_UBER_ID, INVOICING_ENTITY_ID, MOCK_CONNECTOR_ID, PLAN_VERSION_1_LEETCODE_ID, TENANT_ID,
    USER_ID,
};
use crate::harness::reminders::{FinalizedInvoice, match_all};
use crate::harness::{TestEnv, subscription, test_env};
use crate::helpers::credit::{Grant, fund_grant};

const DONE_WITHIN: Duration = Duration::from_secs(60);

fn actor() -> Actor {
    Actor::User { id: USER_ID }
}

/// A transaction that took `statement`'s locks and keeps them until `release`.
async fn hold(env: &TestEnv, statement: &str) -> meteroid_store::store::PgConn {
    let mut holder = env.conn().await;
    holder
        .batch_execute(&format!("BEGIN; {statement};"))
        .await
        .unwrap_or_else(|error| panic!("{statement}: {error}"));
    holder
}

/// Runs `statement` in the holder's transaction (with NOWAIT locks), then rolls it back.
async fn take_then_release(
    mut holder: meteroid_store::store::PgConn,
    statement: &str,
) -> Result<(), String> {
    let taken = holder
        .batch_execute(statement)
        .await
        .map_err(|error| format!("{statement}: {error}"));
    holder.batch_execute("ROLLBACK").await.unwrap();
    taken
}

/// Waits until a backend of this database waits on a lock.
async fn waiting_on_a_lock(env: &TestEnv) {
    let mut conn = env.conn().await;
    for _ in 0..300 {
        let waiting: i64 = diesel::select(diesel::dsl::sql::<BigInt>(
            "(SELECT count(*) FROM pg_stat_activity \
              WHERE datname = current_database() AND wait_event_type = 'Lock')",
        ))
        .get_result(&mut conn)
        .await
        .expect("read pg_stat_activity");
        if waiting > 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("nothing waits on a lock");
}

fn customer_lock(customer: impl BaseId) -> String {
    format!(
        "SELECT id FROM customer WHERE id = '{}' FOR NO KEY UPDATE",
        customer.as_uuid()
    )
}

/// A payment reversal (bank recall, chargeback) waits for the customer before it locks the
/// payment, so a path holding the customer may still write rows that reference the payment (a
/// refund, a top-up): the reversal used to hold the payment while it waited for the customer.
#[rstest]
#[tokio::test]
async fn a_payment_reversal_waits_for_its_customer_before_locking_the_payment(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let (subscription_id, invoice_id) = env.unpaid_card_invoice().await;
    let invoice = env.get_invoices(subscription_id).await.remove(0);
    let payment = env
        .services()
        .add_manual_payment_transaction(
            actor(),
            TENANT_ID,
            invoice_id,
            Decimal::new(invoice.amount_due, 2),
            Utc::now().fixed_offset(),
            Some("wire-lock-order".to_string()),
        )
        .await
        .expect("the wire is recorded");
    env.run_outbox_and_orchestration().await;

    let holder = hold(&env, &customer_lock(invoice.customer_id)).await;
    let services = env.services().clone();
    let reversing = tokio::spawn(async move {
        services
            .reverse_manual_payment(
                actor(),
                TENANT_ID,
                ReverseManualPaymentCommand {
                    transaction_id: payment.id,
                    kind: ReversalKind::DebtorRecall,
                    reason: Some("payer recalled the wire".to_string()),
                    reversed_at: Utc::now().naive_utc(),
                },
            )
            .await
    });
    waiting_on_a_lock(&env).await;
    let referenced = take_then_release(
        holder,
        &format!(
            "SELECT id FROM payment_transaction WHERE id = '{}' FOR KEY SHARE NOWAIT",
            payment.id.as_uuid()
        ),
    )
    .await;

    let reversed = tokio::time::timeout(DONE_WITHIN, reversing)
        .await
        .expect("the reversal completes once the customer is free")
        .expect("the reversal task");
    referenced.expect("the reversal holds nothing on the payment while it waits");
    reversed.expect("the reversal");
}

/// Enrolment takes the reminder sequence before the invoice and its customer, the order of an
/// archive or a customer override, which update the customers using the sequence once they hold it.
#[rstest]
#[tokio::test]
async fn a_reminder_enrolment_takes_its_sequence_before_the_customer(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.enable_reminders().await;
    let sequence = env.default_sequence(INVOICING_ENTITY_ID, true).await;
    env.finalized_invoice_with_pdf(FinalizedInvoice::one_off(CUST_UBER_ID))
        .await;

    let holder = hold(
        &env,
        &format!(
            "SELECT id FROM payment_reminder_sequence WHERE id = '{}' FOR UPDATE",
            sequence.id.as_uuid()
        ),
    )
    .await;
    let services = env.services().clone();
    let sweeping = tokio::spawn(async move {
        services
            .run_payment_reminder_sweep(Utc::now().naive_utc())
            .await
    });
    waiting_on_a_lock(&env).await;
    let customer =
        take_then_release(holder, &format!("{} NOWAIT", customer_lock(CUST_UBER_ID))).await;

    let swept = tokio::time::timeout(DONE_WITHIN, sweeping)
        .await
        .expect("the sweep completes once the sequence is free")
        .expect("the sweep task");
    customer.expect("the enrolment holds no customer while it waits for the sequence");
    assert_eq!(swept.expect("the sweep").enrolled, 1);
}

/// Deleting a payment connection takes the invoicing entities routed to it before the connection,
/// as a checkout does when it finalizes (numbering under the entity) and then charges (connection).
#[rstest]
#[tokio::test]
async fn a_connector_deletion_takes_the_routed_entities_before_the_connector(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.seed_payments().await;

    let holder = hold(
        &env,
        &format!(
            "SELECT id FROM invoicing_entity WHERE id = '{}' FOR NO KEY UPDATE",
            INVOICING_ENTITY_ID.as_uuid()
        ),
    )
    .await;
    let store = env.store().clone();
    let deleting = tokio::spawn(async move {
        store
            .delete_connector(actor(), MOCK_CONNECTOR_ID, TENANT_ID)
            .await
    });
    waiting_on_a_lock(&env).await;
    let charged = take_then_release(
        holder,
        &format!(
            "SELECT id FROM connector WHERE id = '{}' FOR KEY SHARE NOWAIT",
            MOCK_CONNECTOR_ID.as_uuid()
        ),
    )
    .await;

    let deleted = tokio::time::timeout(DONE_WITHIN, deleting)
        .await
        .expect("the deletion completes once the entity is free")
        .expect("the deletion task");
    charged.expect("the deletion holds no connector while it waits for the entity");
    deleted.expect("the deletion");
}

/// Creating a subscription with a coupon takes the customer before the coupon: billing it then
/// finalizes under the customer, and the customer's other invoices refresh the coupon after it.
#[rstest]
#[tokio::test]
async fn a_subscription_with_a_coupon_takes_its_customer_before_the_coupon(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let coupon = env.create_percentage_coupon("LOCKFIRST", 10).await;

    let holder = hold(&env, &customer_lock(CUST_UBER_ID)).await;
    let services = env.services().clone();
    let creating = tokio::spawn(async move {
        subscription()
            .customer(CUST_UBER_ID)
            .plan_version(PLAN_VERSION_1_LEETCODE_ID)
            .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
            .on_start()
            .no_trial()
            .no_auto_charge()
            .coupon(coupon)
            .create(&services)
            .await
    });
    waiting_on_a_lock(&env).await;
    let refreshed = take_then_release(
        holder,
        &format!(
            "SELECT id FROM coupon WHERE id = '{}' FOR UPDATE NOWAIT",
            coupon.as_uuid()
        ),
    )
    .await;

    tokio::time::timeout(DONE_WITHIN, creating)
        .await
        .expect("the creation completes once the customer is free")
        .expect("the creation");
    refreshed.expect("the creation holds no coupon while it waits for the customer");
}

/// An operator removing credit by hand waits for the customer before locking the grant, as every
/// draw does: it used to lock the grant alone.
#[rstest]
#[tokio::test]
async fn a_manual_credit_removal_waits_for_its_customer_before_the_grant(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let grant = fund_grant(
        env.store(),
        TENANT_ID,
        CUST_UBER_ID,
        None,
        Grant::invoice(1_000),
    )
    .await;

    let holder = hold(&env, &customer_lock(CUST_UBER_ID)).await;
    let store = env.store().clone();
    let removing = tokio::spawn(async move {
        store
            .debit_customer_credit_grant(DebitCustomerCreditGrant {
                tenant_id: TENANT_ID,
                customer_id: CUST_UBER_ID,
                grant_id: grant,
                amount_minor: Some(400),
                idempotency_key: uuid::Uuid::now_v7(),
                reason: "Granted in error".to_owned(),
                actor_id: None,
            })
            .await
    });
    waiting_on_a_lock(&env).await;
    let drawn = take_then_release(
        holder,
        &format!(
            "SELECT id FROM customer_credit_grant WHERE id = '{}' FOR NO KEY UPDATE NOWAIT",
            grant.as_uuid()
        ),
    )
    .await;

    let removed = tokio::time::timeout(DONE_WITHIN, removing)
        .await
        .expect("the removal completes once the customer is free")
        .expect("the removal task");
    drawn.expect("the removal holds no grant while it waits for the customer");
    assert_eq!(removed.expect("the removal").removed_minor, 400);
}

/// A payment starting through a connection holds it (KEY SHARE) until it numbers its invoice
/// under the invoicing entity: deleting the connection then refuses at once and lets the entity go,
/// instead of holding the entity while it waits for the connection.
#[rstest]
#[tokio::test]
async fn a_connector_deletion_refuses_a_connector_a_payment_holds(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let holder = hold(
        &env,
        &format!(
            "SELECT id FROM connector WHERE id = '{}' FOR KEY SHARE",
            MOCK_CONNECTOR_ID.as_uuid()
        ),
    )
    .await;
    let deleted = tokio::time::timeout(
        Duration::from_secs(10),
        env.store()
            .delete_connector(actor(), MOCK_CONNECTOR_ID, TENANT_ID),
    )
    .await;
    let numbered = take_then_release(
        holder,
        &format!(
            "SELECT id FROM invoicing_entity WHERE id = '{}' FOR NO KEY UPDATE NOWAIT",
            INVOICING_ENTITY_ID.as_uuid()
        ),
    )
    .await;

    let error = deleted
        .expect("the deletion does not wait on a connector in use")
        .expect_err("a connector in use is not deleted");
    assert!(
        matches!(error.current_context(), StoreError::Conflict(_)),
        "{error:?}"
    );
    numbered.expect("the refused deletion holds no invoicing entity");
}

/// Archiving a sequence locks the customers whose rows it writes before writing them, after the
/// sequence: a send (customer, invoice, reminder row) of an overriding customer then waits for
/// neither. The archive used to stop the reminder rows first, then wait for those customers.
#[rstest]
#[tokio::test]
async fn a_sequence_archive_takes_the_customers_before_their_reminders(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let sequence = env
        .create_sequence_with_audience(INVOICING_ENTITY_ID, "Override", match_all(), &[0, 7], true)
        .await;
    env.set_customer_reminder_preference(CUST_UBER_ID, false, Some(sequence.id))
        .await;
    let invoice = env
        .finalized_invoice(FinalizedInvoice::one_off(CUST_UBER_ID))
        .await;
    env.enroll_invoice(invoice, sequence.id).await;

    let holder = hold(&env, &customer_lock(CUST_UBER_ID)).await;
    let store = env.store().clone();
    let archiving = tokio::spawn(async move {
        store
            .archive_reminder_sequence(actor(), TENANT_ID, sequence.id, Utc::now().naive_utc())
            .await
    });
    waiting_on_a_lock(&env).await;
    let sent = take_then_release(
        holder,
        &format!(
            "SELECT invoice_id FROM invoice_payment_reminder WHERE invoice_id = '{}' FOR UPDATE NOWAIT",
            invoice.as_uuid()
        ),
    )
    .await;

    let archived = tokio::time::timeout(DONE_WITHIN, archiving)
        .await
        .expect("the archive completes once the customer is free")
        .expect("the archive task");
    sent.expect("the archive holds no reminder row while it waits for the customer");
    assert_eq!(archived.expect("the archive"), 1);
}

/// Cancelling with another tenant's credentials is refused before any customer is locked: the
/// tenant is checked as the subscription's customer is read, so a held customer does not delay it.
#[rstest]
#[tokio::test]
async fn a_cancel_from_another_tenant_never_locks_the_customer(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let subscription_id = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(Utc::now().date_naive())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let holder = hold(&env, &customer_lock(CUST_UBER_ID)).await;
    let services = env.services().clone();
    let cancelled = tokio::time::timeout(
        DONE_WITHIN,
        services.cancel_subscription(
            actor(),
            subscription_id,
            common_domain::ids::TenantId::new(),
            None,
            CancellationEffectiveAt::Immediate,
        ),
    )
    .await
    .expect("the refusal does not wait for the customer")
    .expect_err("another tenant's subscription");
    take_then_release(holder, "SELECT 1").await.unwrap();
    assert!(
        matches!(cancelled.current_context(), StoreError::ValueNotFound(_)),
        "{cancelled:?}"
    );
}
