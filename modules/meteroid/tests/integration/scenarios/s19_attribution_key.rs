//! S19. One metric priced by a filtered and an unfiltered subscription of one customer: the
//! filtered one bills its key's events, the unfiltered one the events no filter claims (D2
//! precision), and with every subscription filtered the rest is kept as unattributed usage.
use common_domain::ids::{CustomerId, SubscriptionId};
use rust_decimal::Decimal;

use super::kit::{Halyard, LITE, M, STANDARD, Tax, at, at_time, day, lines, money};
use super::s10_two_subscriptions::keyed;

/// A on Pay as you go with `api_key = key-A`, B with `b_key`, both from Sep 1; then E1 (key-A,
/// standard 5M = 6000), E2 (key-Z, standard 2M = 2400), E3 (no key, lite 10M = 2000).
async fn both(h: &Halyard, b_key: Option<&str>) -> (CustomerId, SubscriptionId, SubscriptionId) {
    let customer = h.customer("Untaxed", Tax::Untaxed).await;
    let mut a_request = h.request(customer, h.payg, day(9, 1));
    a_request.subscription.usage_filter = keyed("key-A");
    let (a, _) = h.start(a_request).await;
    let mut b_request = h.request(customer, h.payg, day(9, 1));
    b_request.subscription.usage_filter = b_key.and_then(keyed);
    let (b, _) = h.start(b_request).await;
    h.keyed_tokens(customer, at_time(9, 5, 10, 0, 0), STANDARD, 5 * M, "key-A");
    h.keyed_tokens(customer, at_time(9, 10, 10, 0, 0), STANDARD, 2 * M, "key-Z");
    h.tokens(customer, at_time(9, 15, 10, 0, 0), LITE, 10 * M);
    h.check().await;
    (customer, a, b)
}

/// What a subscription's period invoice of Oct 1 bills, 0 when it bills nothing.
async fn billed(h: &Halyard, subscription: SubscriptionId) -> (i64, Vec<(String, i64)>) {
    match h.renew(subscription, at(10, 1)).await {
        Some(invoice) => {
            let invoice = h.invoice(invoice.id).await;
            (money(&invoice).6, lines(&invoice))
        }
        None => (0, vec![]),
    }
}

async fn unattributed(h: &Halyard, customer: CustomerId) -> Decimal {
    h.services()
        .unattributed_usage(
            h.tenant,
            customer,
            day(9, 1).and_hms_opt(0, 0, 0).unwrap(),
            day(10, 1).and_hms_opt(0, 0, 0).unwrap(),
        )
        .await
        .expect("unattributed usage")
        .iter()
        .map(|usage| usage.quantity)
        .sum()
}

#[tokio::test]
async fn s19_the_unfiltered_subscription_bills_what_no_filter_claims() {
    let h = Halyard::new().await;
    let (customer, a, b) = both(&h, None).await;
    let (a_due, a_lines) = billed(&h, a).await;
    let (b_due, b_lines) = billed(&h, b).await;
    assert_eq!(a_due, 6_000, "E1 once, by A: {a_lines:?}");
    assert_eq!(b_due, 4_400, "E2 + E3, by the fallback: {b_lines:?}");
    assert_eq!(unattributed(&h, customer).await, Decimal::ZERO);
    h.check().await;
}

#[tokio::test]
async fn s19_all_filtered_keeps_the_rest_unattributed() {
    let h = Halyard::new().await;
    let (customer, a, b) = both(&h, Some("key-B")).await;
    let (a_due, a_lines) = billed(&h, a).await;
    let (b_due, b_lines) = billed(&h, b).await;
    assert_eq!(a_due, 6_000, "{a_lines:?}");
    assert_eq!(b_due, 0, "nothing of B's key: {b_lines:?}");
    assert_eq!(
        unattributed(&h, customer).await,
        Decimal::from(12 * M),
        "E2 + E3 tokens, billed to nobody"
    );
    h.check().await;
}
