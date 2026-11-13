//! Credit scope: a grant of a subscription pays only that subscription's charges, customer-wide
//! credit pays any, and concurrent draws of two subscriptions on shared credit stay exact.
use chrono::Utc;
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, CreditGrantId, InvoiceId, SubscriptionId};
use meteroid_store::domain::credit::{CreditApplication, GrantClassification, GrantScope};
use meteroid_store::domain::{Invoice, LineItem};
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::credit::customer_credit::DEFAULT_PRIORITY;
use meteroid_store::repositories::credit::customer_credit_wallets::FundCustomerCreditWallet;
use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;
use rstest::rstest;

use crate::data::ids::*;
use crate::harness::{TestEnv, subscription, test_env};
use crate::helpers::credit::{assert_ledger_balances, grant_remaining};
use crate::test_customer_credit::{draft, line};

async fn live_subscription(env: &TestEnv) -> SubscriptionId {
    subscription()
        .start_date(Utc::now().date_naive())
        .on_start()
        .no_trial()
        .create(env.services())
        .await
}

async fn grant(env: &TestEnv, scope: GrantScope, amount_minor: i64) -> CreditGrantId {
    grant_of(env, scope, amount_minor, GrantClassification::Paid).await
}

async fn grant_of(
    env: &TestEnv,
    scope: GrantScope,
    amount_minor: i64,
    classification: GrantClassification,
) -> CreditGrantId {
    let wallet = env
        .store()
        .create_customer_credit_wallet(TENANT_ID, CUST_UBER_ID, "EUR")
        .await
        .expect("the customer's EUR wallet");
    env.store()
        .fund_scoped_customer_credit_wallet(
            FundCustomerCreditWallet {
                tenant_id: TENANT_ID,
                customer_id: CUST_UBER_ID,
                wallet_id: wallet.id,
                amount_minor,
                classification,
                priority: DEFAULT_PRIORITY,
                expires_at: None,
                idempotency_key: uuid::Uuid::now_v7(),
                reason: "Credit scope test".to_owned(),
                reference: None,
                actor_id: None,
                cost: None,
            },
            scope,
        )
        .await
        .expect("fund scoped credit")
        .grant_id
}

fn of(subscription_id: SubscriptionId, line: LineItem) -> LineItem {
    LineItem {
        subscription_id: Some(subscription_id),
        ..line
    }
}

async fn finalize(env: &TestEnv, invoice: InvoiceId) -> Invoice {
    env.services()
        .finalize_invoice(Actor::System, invoice, TENANT_ID)
        .await
        .expect("finalize")
        .invoice
}

async fn drawn_from(env: &TestEnv, invoice: InvoiceId) -> Vec<(CreditGrantId, i64)> {
    let mut held = env
        .store()
        .invoice_credit_by_grant(TENANT_ID, invoice)
        .await
        .expect("invoice credit");
    held.sort_by_key(|(grant, _)| grant.as_uuid());
    held
}

fn usage_of(subscription_id: SubscriptionId) -> GrantScope {
    GrantScope {
        subscription_id: Some(subscription_id),
        application: CreditApplication::Usage,
        product_ids: Vec::new(),
    }
}

/// T13: A's and B's invoices finalize concurrently, 50 times, on a shared top-up and a pack each:
/// no deadlock, each pack pays only its own subscription's usage, and every sum is exact.
#[rstest]
#[tokio::test]
async fn concurrent_finalization_does_not_deadlock(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let (a, b) = (live_subscription(&env).await, live_subscription(&env).await);
    let pack_a = grant(&env, usage_of(a), 5_000).await;
    let pack_b = grant(&env, usage_of(b), 5_000).await;
    let shared = grant(&env, GrantScope::customer_wide(), 6_000).await;

    for _ in 0..50 {
        let for_a = draft(&env.setup, vec![of(a, line("Usage A", 100, true))])
            .await
            .expect("A's draft");
        let for_b = draft(
            &env.setup,
            vec![
                of(b, line("Usage B", 100, true)),
                of(b, line("Seats B", 100, false)),
            ],
        )
        .await
        .expect("B's draft");
        let (finalized_a, finalized_b) =
            tokio::join!(finalize(&env, for_a.id), finalize(&env, for_b.id));
        assert_eq!(
            (finalized_a.applied_credits, finalized_a.amount_due),
            (100, 0)
        );
        assert_eq!(
            (finalized_b.applied_credits, finalized_b.amount_due),
            (200, 0)
        );
        assert_eq!(drawn_from(&env, for_a.id).await, vec![(pack_a, 100)]);
        let mut expected_b = vec![(pack_b, 100), (shared, 100)];
        expected_b.sort_by_key(|(grant, _)| grant.as_uuid());
        assert_eq!(drawn_from(&env, for_b.id).await, expected_b);
    }

    let remaining = |id| grant_remaining(env.store(), TENANT_ID, CUST_UBER_ID, id);
    assert_eq!(remaining(pack_a).await, 0);
    assert_eq!(remaining(pack_b).await, 0);
    assert_eq!(
        remaining(shared).await,
        1_000,
        "only B's seats drew shared credit"
    );
    assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// T14: a line stored without its subscription, as an older release writes it, still draws its
/// subscription's credit through its component, exactly as the same line recording it.
#[rstest]
#[tokio::test]
async fn old_invoice_lines_resolve_their_subscription(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let a = live_subscription(&env).await;
    let component = env
        .store()
        .get_subscription_details(TENANT_ID, a)
        .await
        .expect("subscription details")
        .price_components
        .first()
        .map(|component| component.id)
        .expect("a component");
    let pack = grant(&env, usage_of(a), 1_000).await;

    let old = LineItem {
        sub_component_id: Some(component),
        ..line("Usage", 300, true)
    };
    let current = of(a, old.clone());
    let unowned = line("Usage of no subscription", 300, true);
    for lines in [vec![old], vec![current]] {
        let invoice = draft(&env.setup, lines).await.expect("draft");
        let finalized = finalize(&env, invoice.id).await;
        assert_eq!(finalized.applied_credits, 300);
        assert_eq!(drawn_from(&env, invoice.id).await, vec![(pack, 300)]);
    }
    // Usage that names no subscription is never paid by a subscription's credit.
    let invoice = draft(&env.setup, vec![unowned]).await.expect("draft");
    let finalized = finalize(&env, invoice.id).await;
    assert_eq!((finalized.applied_credits, finalized.amount_due), (0, 300));
    assert_eq!(
        grant_remaining(env.store(), TENANT_ID, CUST_UBER_ID, pack).await,
        400
    );
    assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// A grant ends with its subscription once, at the instant it ended: the term never moves again,
/// the expiry sweep lapses what is left with the reason, and customer-wide credit is untouched.
#[rstest]
#[tokio::test]
async fn a_grant_ends_once_with_its_subscription(#[future] test_env: TestEnv) {
    use diesel_async::SimpleAsyncConnection;
    use diesel_models::customer_credit::{CustomerCreditGrantRow, EndedSubscriptionCredit};
    use meteroid_store::domain::credit::ENDED_WITH_SUBSCRIPTION;

    let env = test_env.await;

    env.skip_checks(&["I5"], 

        "moves a grant's expiry or a subscription's end into the past by raw SQL after credit was drawn",

    );
    let a = live_subscription(&env).await;
    let spent = grant(&env, usage_of(a), 300).await;
    let pack = grant(&env, usage_of(a), 1_000).await;
    let shared = grant(&env, GrantScope::customer_wide(), 500).await;
    let usage = draft(&env.setup, vec![of(a, line("Usage", 300, true))])
        .await
        .expect("draft");
    finalize(&env, usage.id).await;
    assert_eq!(drawn_from(&env, usage.id).await, vec![(spent, 300)]);

    // A terminal status the end hook missed: the self-heal query finds it.
    let ended_at = Utc::now() - chrono::Duration::hours(1);
    let mut conn = env.store().get_conn().await.expect("connection");
    conn.batch_execute(&format!(
        "UPDATE subscription SET status = 'CANCELLED', end_date = NULL, terminated_at = '{}'
         WHERE id = '{}'",
        ended_at.naive_utc(),
        a.as_uuid()
    ))
    .await
    .expect("cancel the subscription");
    let unended = CustomerCreditGrantRow::list_unended_of_ended_subscriptions(&mut conn, None, 100)
        .await
        .expect("unended credit");
    assert!(unended.contains(&EndedSubscriptionCredit {
        tenant_id: TENANT_ID,
        subscription_id: a,
    }));

    let ended = CustomerCreditGrantRow::end_for_subscription(&mut conn, TENANT_ID, a, ended_at)
        .await
        .expect("end the subscription's credit");
    let mut ended_ids = ended.iter().map(|g| g.id.as_uuid()).collect::<Vec<_>>();
    ended_ids.sort();
    let mut expected = vec![spent.as_uuid(), pack.as_uuid()];
    expected.sort();
    assert_eq!(ended_ids, expected, "spent credit ends too");
    assert!(
        ended
            .iter()
            .all(|g| g.lapses_at.map(|at| at.timestamp_micros())
                == Some(ended_at.timestamp_micros()))
    );
    let again = CustomerCreditGrantRow::end_for_subscription(&mut conn, TENANT_ID, a, Utc::now())
        .await
        .expect("idempotent");
    assert!(again.is_empty(), "a grant ends once");
    let unended = CustomerCreditGrantRow::list_unended_of_ended_subscriptions(&mut conn, None, 100)
        .await
        .expect("unended credit");
    assert!(unended.iter().all(|row| row.subscription_id != a));
    drop(conn);

    env.store()
        .expire_customer_credit_for(TENANT_ID, CUST_UBER_ID, Utc::now())
        .await
        .expect("sweep");
    assert_eq!(
        grant_remaining(env.store(), TENANT_ID, CUST_UBER_ID, pack).await,
        1_000
    );
    let wallets = env
        .store()
        .customer_credit_wallets(TENANT_ID, CUST_UBER_ID)
        .await
        .expect("wallets");
    let lapsed = wallets[0]
        .grants
        .iter()
        .find(|g| g.id == pack)
        .expect("the pack");
    assert_eq!(lapsed.remaining_minor, 0);
    assert_eq!(lapsed.expired_minor, Some(1_000));
    assert_eq!(wallets[0].available_minor, 500);
    let (entries, _) = env
        .store()
        .customer_credit_ledger(TENANT_ID, wallets[0].id, 100, 0)
        .await
        .expect("statement");
    assert!(entries.iter().any(|entry| entry.grant_id == pack
        && entry.reason == ENDED_WITH_SUBSCRIPTION
        && entry.amount_minor == -1_000));
    assert_eq!(
        grant_remaining(env.store(), TENANT_ID, CUST_UBER_ID, shared).await,
        500
    );
    assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// Credit a note gives back to a grant that ended with its subscription, past the settlement
/// grace, lapses at once: it is never spendable again, and the note mints nothing customer-wide.
#[rstest]
#[tokio::test]
async fn credit_given_back_to_an_ended_grant_lapses_at_once(#[future] test_env: TestEnv) {
    use diesel_models::customer_credit::CustomerCreditGrantRow;
    use meteroid_store::domain::credit::{CreditGrantSource, ENDED_WITH_SUBSCRIPTION};
    use meteroid_store::repositories::CreditNoteInterface;
    use meteroid_store::repositories::credit_notes::{
        CreateCreditNoteParams, CreditLineItem, CreditType,
    };

    let env = test_env.await;

    env.skip_checks(&["I5"], 

        "moves a grant's expiry or a subscription's end into the past by raw SQL after credit was drawn",

    );
    // No invoicing grace: the settlement grace is its 30-minute floor, and the end an hour ago.
    env.set_consolidate_and_grace(false, 0).await;
    let a = live_subscription(&env).await;
    let pack = grant(&env, usage_of(a), 1_000).await;
    // The pack pays the usage; the fee is paid in cash, so the invoice is paid.
    let invoice = draft(
        &env.setup,
        vec![
            of(a, line("Usage", 400, true)),
            of(a, line("Seats", 100, false)),
        ],
    )
    .await
    .expect("draft");
    let finalized = finalize(&env, invoice.id).await;
    assert_eq!(
        (finalized.applied_credits, finalized.amount_due),
        (400, 100)
    );
    crate::test_customer_credit::pay_amount(&env.setup, invoice.id, 100)
        .await
        .expect("pay the fee");

    let mut conn = env.store().get_conn().await.expect("connection");
    CustomerCreditGrantRow::end_for_subscription(
        &mut conn,
        TENANT_ID,
        a,
        Utc::now() - chrono::Duration::hours(1),
    )
    .await
    .expect("end the subscription's credit");
    drop(conn);

    let local_id = finalized
        .line_items
        .iter()
        .find(|line| line.name == "Usage")
        .expect("the usage line")
        .local_id
        .clone();
    env.store()
        .create_and_finalize_credit_note(
            Actor::System,
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id: invoice.id,
                line_items: vec![CreditLineItem::Amount {
                    local_id,
                    amount_cents: 400,
                }],
                reason: Some("Usage credited".into()),
                memo: None,
                credit_type: CreditType::CreditToBalance,
                custom_properties: None,
            },
        )
        .await
        .expect("credit note");

    let wallets = env
        .store()
        .customer_credit_wallets(TENANT_ID, CUST_UBER_ID)
        .await
        .expect("wallets");
    let ended = wallets[0]
        .grants
        .iter()
        .find(|g| g.id == pack)
        .expect("pack");
    assert_eq!(ended.remaining_minor, 0, "never spendable again");
    assert_eq!(ended.expired_minor, Some(1_000));
    assert_eq!(wallets[0].available_minor, 0);
    assert!(
        wallets[0]
            .grants
            .iter()
            .all(|g| g.source != CreditGrantSource::CreditNote),
        "what went back to the grant is not minted again"
    );
    let (entries, _) = env
        .store()
        .customer_credit_ledger(TENANT_ID, wallets[0].id, 100, 0)
        .await
        .expect("statement");
    assert!(entries.iter().any(|entry| entry.grant_id == pack
        && entry.reason == ENDED_WITH_SUBSCRIPTION
        && entry.amount_minor == -400));
    assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// Editing a draft's lines keeps the subscription each one bills, so its scoped credit still pays.
#[rstest]
#[tokio::test]
async fn editing_a_line_keeps_its_subscription(#[future] test_env: TestEnv) {
    use meteroid_store::domain::{UpdateInvoiceParams, UpdateLineItemParams};

    let env = test_env.await;
    let a = live_subscription(&env).await;
    let any_charge = grant(
        &env,
        GrantScope {
            subscription_id: Some(a),
            ..GrantScope::customer_wide()
        },
        1_000,
    )
    .await;
    let invoice = draft(
        &env.setup,
        vec![of(a, line("Minimum commitment", 1_000, false))],
    )
    .await
    .expect("draft");
    let edited = &invoice.line_items[0];
    let updated = env
        .services()
        .update_draft_invoice(
            invoice.id,
            TENANT_ID,
            UpdateInvoiceParams {
                line_items: Some(vec![UpdateLineItemParams {
                    id: Some(edited.local_id.clone()),
                    name: "Minimum commitment (edited)".to_owned(),
                    start_date: edited.start_date,
                    end_date: edited.end_date,
                    quantity: Some(rust_decimal::Decimal::ONE),
                    unit_price: Some(rust_decimal::Decimal::new(800, 2)),
                    tax_rate: rust_decimal::Decimal::ZERO,
                    description: None,
                    sub_lines: Vec::new(),
                }]),
                ..Default::default()
            },
        )
        .await
        .expect("edit the line")
        .invoice;
    assert_eq!(updated.line_items[0].subscription_id, Some(a));
    let finalized = finalize(&env, invoice.id).await;
    assert_eq!((finalized.applied_credits, finalized.amount_due), (800, 0));
    assert_eq!(drawn_from(&env, invoice.id).await, vec![(any_charge, 800)]);
}

/// A retried grant request returns the grant its first attempt made, even once the subscription
/// it belongs to has ended; a new request for that subscription is refused.
#[rstest]
#[tokio::test]
async fn a_scoped_grant_replays_after_its_subscription_ended(#[future] test_env: TestEnv) {
    use diesel_async::SimpleAsyncConnection;

    let env = test_env.await;

    env.skip_checks(
        &["I9"],
        "cancels a subscription by raw SQL, without the terminal transition that ends its credit",
    );
    let a = live_subscription(&env).await;
    let wallet = env
        .store()
        .create_customer_credit_wallet(TENANT_ID, CUST_UBER_ID, "EUR")
        .await
        .expect("wallet");
    let key = uuid::Uuid::now_v7();
    let command = |idempotency_key| FundCustomerCreditWallet {
        tenant_id: TENANT_ID,
        customer_id: CUST_UBER_ID,
        wallet_id: wallet.id,
        amount_minor: 1_000,
        classification: GrantClassification::Promotional,
        priority: DEFAULT_PRIORITY,
        expires_at: None,
        idempotency_key,
        reason: "Goodwill".to_owned(),
        reference: None,
        actor_id: None,
        cost: None,
    };
    let first = env
        .store()
        .fund_scoped_customer_credit_wallet(command(key), usage_of(a))
        .await
        .expect("grant");
    env.store()
        .get_conn()
        .await
        .expect("connection")
        .batch_execute(&format!(
            "UPDATE subscription SET status = 'CANCELLED' WHERE id = '{}'",
            a.as_uuid()
        ))
        .await
        .expect("cancel");
    let replayed = env
        .store()
        .fund_scoped_customer_credit_wallet(command(key), usage_of(a))
        .await
        .expect("a replay");
    assert!(replayed.replayed);
    assert_eq!(replayed.grant_id, first.grant_id);
    let refused = env
        .store()
        .fund_scoped_customer_credit_wallet(command(uuid::Uuid::now_v7()), usage_of(a))
        .await;
    assert!(
        format!("{refused:?}").contains("subscription has ended"),
        "{refused:?}"
    );
}

/// Promotional credit comes off before tax, ahead of paid credit, but lands where that paid credit
/// cannot pay: whatever the line order, the customer-wide promotion pays B's usage, A's paid pack
/// all of A's, and nothing is due.
#[rstest]
#[tokio::test]
async fn promotional_credit_leaves_a_subscription_s_usage_to_its_paid_pack(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let (a, b) = (live_subscription(&env).await, live_subscription(&env).await);
    let pack = grant(&env, usage_of(a), 30_000).await;
    for a_first in [true, false] {
        let promo = grant_of(
            &env,
            GrantScope::customer_wide(),
            5_000,
            GrantClassification::Promotional,
        )
        .await;
        let mut lines = vec![
            of(a, line("Usage A", 10_000, true)),
            of(b, line("Usage B", 5_000, true)),
        ];
        if !a_first {
            lines.reverse();
        }
        let invoice = draft(&env.setup, lines).await.expect("draft");
        let finalized = finalize(&env, invoice.id).await;
        assert_eq!(
            (
                finalized.promotional_credits,
                finalized.applied_credits,
                finalized.amount_due
            ),
            (5_000, 10_000, 0),
            "A's lines first: {a_first}"
        );
        let mut expected = vec![(pack, 10_000), (promo, 5_000)];
        expected.sort_by_key(|(grant, _)| grant.as_uuid());
        assert_eq!(drawn_from(&env, invoice.id).await, expected);
    }
    assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}
