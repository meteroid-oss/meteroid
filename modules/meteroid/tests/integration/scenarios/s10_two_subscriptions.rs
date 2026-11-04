//! S10. Two subscriptions of one customer, each with a usage filter (attribution key): A's plan
//! credit pays A's usage only, a customer-wide top-up pays both in billing order, and an event
//! no subscription claims is kept as unattributed usage, billed to nobody.
use chrono::{DateTime, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{CreditGrantId, CustomerId};
use meteroid_store::domain::UsageFilter;
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::services::{InvoiceDelivery, SellCustomerCredit};
use rust_decimal::Decimal;

use super::kit::{Halyard, LITE, M, STANDARD, Tax, at, at_time, day, grant, lines, money};

/// A customer-wide top-up of `amount`, sold and paid at `at`: PAID, no expiry.
pub(super) async fn top_up(
    h: &Halyard,
    customer: CustomerId,
    amount: i64,
    at: DateTime<Utc>,
) -> CreditGrantId {
    let wallet = h
        .store()
        .create_customer_credit_wallet(h.tenant, customer, "USD")
        .await
        .expect("wallet");
    let sold = h
        .services()
        .sell_customer_credit_at(
            uuid::Uuid::now_v7(),
            SellCustomerCredit {
                expires_at: None,
                tenant_id: h.tenant,
                customer_id: customer,
                wallet_id: wallet.id,
                amount_minor: amount,
                expires_in_months: None,
                note: None,
                actor: Actor::System,
                unit_price: None,
                expected_unit_price: None,
            },
            at,
            InvoiceDelivery::Suppressed,
        )
        .await
        .expect("top-up sold");
    assert_eq!(
        sold.invoice.invoice.tax_amount, 0,
        "a currency top-up is untaxed"
    );
    h.pay(sold.invoice.invoice.id, at).await;
    h.grants(customer)
        .await
        .into_iter()
        .rev()
        .find(|g| {
            g.source_kind == "CREDIT_PURCHASE"
                && g.classification == "PAID"
                && g.subscription_id.is_none()
                && g.amount == amount
        })
        .expect("the top-up's grant")
        .id
}

pub(super) fn keyed(key: &str) -> Option<UsageFilter> {
    Some(UsageFilter::new("api_key".to_owned(), key.to_owned()).expect("a usage filter"))
}

#[tokio::test]
async fn s10_two_subscriptions_scoped_plan_credit_and_a_shared_top_up() {
    let h = Halyard::new().await;
    let customer = h.customer("Untaxed", Tax::Untaxed).await;
    let mut a_request = h.request(customer, h.scale, day(9, 1));
    a_request.subscription.usage_filter = keyed("key-A");
    let (a, a1) = h.start(a_request).await;
    let a1 = a1.expect("INV-A1");
    assert_eq!(money(&a1), (50_000, 0, 0, 0, 50_000, 0, 50_000));
    h.pay(a1.id, at(9, 1)).await;
    let top_up = top_up(&h, customer, 20_000, at(9, 2)).await;
    h.check().await;

    let mut b_request = h.request(customer, h.payg, day(9, 15));
    b_request.subscription.usage_filter = keyed("key-B");
    let (b, _) = h.start(b_request).await;
    h.check().await;

    // A: standard 50M (60000) + lite 50M (10000) = 70000; B: standard 12.5M = 15000 by Sep 30.
    h.keyed_tokens(customer, at_time(9, 3, 10, 0, 0), STANDARD, 50 * M, "key-A");
    h.keyed_tokens(customer, at_time(9, 18, 10, 0, 0), LITE, 50 * M, "key-A");
    h.keyed_tokens(
        customer,
        at_time(9, 20, 10, 0, 0),
        STANDARD,
        25 * M / 2,
        "key-B",
    );
    h.keyed_tokens(customer, at_time(9, 22, 10, 0, 0), LITE, 10 * M, "key-Z");

    let unattributed = h
        .services()
        .unattributed_usage(
            h.tenant,
            customer,
            day(9, 1).and_hms_opt(0, 0, 0).unwrap(),
            day(10, 1).and_hms_opt(0, 0, 0).unwrap(),
        )
        .await
        .expect("unattributed usage");
    assert_eq!(
        unattributed.iter().map(|u| u.quantity).sum::<Decimal>(),
        Decimal::from(10 * M),
        "key-Z matches no filter and no unfiltered subscription prices the metric"
    );
    assert!(
        unattributed.iter().all(|u| u.subscription_ids.contains(&a)),
        "each row names the subscriptions pricing its metric: {unattributed:?}"
    );

    let a2 = h.renew(a, at(10, 1)).await.expect("INV-A2");
    h.sweep(customer, at(10, 1) + Halyard::hours(1)).await;
    let a2 = h.invoice(a2.id).await;
    assert_eq!(
        money(&a2),
        (120_000, 0, 10_000, 0, 110_000, 60_000, 50_000),
        "A's pack and A's usage only; B 5000 + W 5000 before tax, P 50000 + T 10000 after: {:?}",
        lines(&a2)
    );
    let grants = h.grants(customer).await;
    let september_paid = grants
        .iter()
        .find(|g| {
            g.source_kind == "CREDIT_PURCHASE"
                && g.classification == "PAID"
                && g.subscription_id == Some(a)
        })
        .expect("A's September pack");
    assert_eq!(
        h.movements(september_paid.id).await,
        vec!["FUND 50000", "DRAW -50000"]
    );
    let top_up_row = grants.iter().find(|g| g.id == top_up).unwrap();
    assert_eq!(top_up_row.remaining, 10_000);
    h.pay(a2.id, at(10, 1)).await;
    h.check().await;

    let b1 = h.renew(b, at(10, 15)).await.expect("INV-B1");
    let b1 = h.invoice(b1.id).await;
    assert_eq!(
        money(&b1),
        (15_000, 0, 0, 0, 15_000, 10_000, 5_000),
        "B's usage only, paid by the rest of the top-up: {:?}",
        lines(&b1)
    );
    let grants = h.grants(customer).await;
    assert_eq!(grants.iter().find(|g| g.id == top_up).unwrap().remaining, 0);
    let october_paid = grant(&grants, "CREDIT_PURCHASE", "PAID", 50_000);
    assert_eq!(
        (october_paid.subscription_id, october_paid.remaining),
        (Some(a), 50_000),
        "A's October pack never pays B"
    );
    assert_eq!(
        grant(&grants, "CREDIT_PURCHASE", "PROMOTIONAL", 5_000).remaining,
        5_000
    );
    h.pay(b1.id, at(10, 15)).await;
    h.check().await;

    if let Some(rev) = h.revenue(customer, day(10, 15)).await {
        assert_eq!(
            (rev.billed, rev.recognized, rev.deferred()),
            (125_000, 75_000, 50_000),
            "deferred is A's October pack"
        );
    }
}
