//! S15. Enterprise's 5,000.00 monthly minimum, net 30: September is trued up on October's invoice,
//! measured on usage before promotional credit; the shortfall is billed (and taxed, OQ17),
//! promotional credit then paid credit pay what they may; usage-only plan credit never pays it.
use chrono::Duration;
use common_domain::actor::Actor;
use common_domain::ids::{CustomerId, SubscriptionId};
use meteroid_store::domain::credit::{GrantClassification, GrantScope};
use meteroid_store::domain::{Invoice, LineItemKind};
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::credit::customer_credit::DEFAULT_PRIORITY;
use meteroid_store::repositories::credit::customer_credit_wallets::FundCustomerCreditWallet;

use super::kit::{Halyard, M, MAX, STANDARD, Tax, at, at_time, day, grant, lines, money};
use super::s13_top_up_void_note_refund::{add_credit, top_up, wallet};

/// Enterprise from Sep 1 with a 100000 top-up paid Sep 2.
async fn on_enterprise(h: &Halyard, tax: Tax) -> (CustomerId, SubscriptionId) {
    let customer = h.customer("Enterprise", tax).await;
    let (sub, _) = h.subscribe(customer, h.enterprise, day(9, 1)).await;
    top_up(h, customer, 100_000, at(9, 2)).await;
    (customer, sub)
}

fn shortfall(invoice: &Invoice) -> (i64, i64) {
    invoice
        .line_items
        .iter()
        .find(|line| line.kind == LineItemKind::MinimumShortfall)
        .map(|line| (line.amount_subtotal, line.tax_amount))
        .unwrap_or_else(|| panic!("no shortfall line: {:?}", lines(invoice)))
}

/// Sep: standard 250M (240000) + max 12.5M (60000) = 300000 at Enterprise prices.
fn september_usage(h: &Halyard, customer: CustomerId) {
    h.tokens(customer, at_time(9, 10, 10, 0, 0), STANDARD, 250 * M);
    h.tokens(customer, at_time(9, 20, 10, 0, 0), MAX, 25 * M / 2);
}

#[tokio::test]
async fn s15_minimum_measured_before_promotional_credit() {
    let h = Halyard::new().await;
    let (customer, _) = september_billed(&h).await;
    if let Some(rev) = h.revenue(customer, day(10, 1)).await {
        assert_eq!(rev.billed, 450_000, "100000 (T) + 350000 due");
    }
}

/// S15's revenue as of Oct 1: the shortfall trues up September, so it is recognized with it.
#[tokio::test]
async fn s15_revenue_recognizes_the_shortfall_in_its_period() {
    let h = Halyard::new().await;
    let (customer, _) = september_billed(&h).await;
    if let Some(rev) = h.revenue(customer, day(10, 1)).await {
        assert_eq!(
            (rev.billed, rev.recognized, rev.deferred()),
            (450_000, 450_000, 0)
        );
    }
}

/// S15's timeline to INV-1, asserting its ledger and invoice figures.
async fn september_billed(h: &Halyard) -> (CustomerId, Invoice) {
    let (customer, sub) = on_enterprise(h, Tax::Untaxed).await;
    add_credit(
        h,
        customer,
        50_000,
        GrantClassification::Promotional,
        None,
        at(9, 1),
    )
    .await;
    h.check().await;

    let refused = h
        .services()
        .apply_plan_change_immediate_at(Actor::System, sub, h.tenant, h.scale, vec![], day(9, 15))
        .await;
    assert!(
        refused.is_err(),
        "R17: no immediate change with a minimum commitment"
    );
    h.services()
        .schedule_plan_change(Actor::System, sub, h.tenant, h.scale, vec![])
        .await
        .expect("a change at the period end is accepted");
    h.services()
        .cancel_plan_change(Actor::System, sub, h.tenant)
        .await
        .expect("and cancelled");

    september_usage(h, customer);
    let invoice = h.renew(sub, at(10, 1)).await.expect("INV-1");
    assert_eq!(
        shortfall(&invoice),
        (200_000, 0),
        "500000 − 300000; a floor after promotional credit would give 250000"
    );
    let trued_up = invoice
        .line_items
        .iter()
        .find(|line| line.kind == LineItemKind::MinimumShortfall)
        .map(|line| (line.start_date, line.end_date));
    assert_eq!(
        trued_up,
        Some((day(9, 1), day(10, 1))),
        "the true-up of September, dated over it"
    );
    assert_eq!(
        money(&invoice),
        (500_000, 0, 50_000, 0, 450_000, 100_000, 350_000),
        "{:?}",
        lines(&invoice)
    );
    assert_eq!(
        invoice.due_at.map(|due| due.date()),
        Some(day(10, 31)),
        "net 30"
    );
    let grants = h.grants(customer).await;
    assert_eq!(grant(&grants, "MANUAL", "PROMOTIONAL", 50_000).remaining, 0);
    assert_eq!(
        grant(&grants, "CREDIT_PURCHASE", "PAID", 100_000).remaining,
        0
    );
    h.check().await;
    (customer, invoice)
}

/// Ohio (8 %): an all-components minimum is taxed at the default rate (OQ17), so the tax is
/// 8 % of 500000 − 50000, of which 16000 on the shortfall.
#[tokio::test]
async fn s15_oh_shortfall_taxed_at_the_default_rate() {
    let h = Halyard::new().await;
    let (customer, sub) = on_enterprise(&h, Tax::Ohio).await;
    add_credit(
        &h,
        customer,
        50_000,
        GrantClassification::Promotional,
        None,
        at(9, 1),
    )
    .await;
    september_usage(&h, customer);
    let invoice = h.renew(sub, at(10, 1)).await.expect("INV-1");
    // The total is 8 % of what promotional credit leaves; how the 50000 is placed between the
    // usage line and the shortfall decides each line's share.
    let (amount, tax) = shortfall(&invoice);
    let line = invoice
        .line_items
        .iter()
        .find(|line| line.kind == LineItemKind::MinimumShortfall)
        .expect("the shortfall");
    assert_eq!(amount, 200_000);
    assert_eq!(
        tax,
        (line.taxable_amount * 8 + 50) / 100,
        "taxed at the default 8 %"
    );
    assert_eq!(
        money(&invoice),
        (500_000, 0, 50_000, 36_000, 486_000, 100_000, 386_000),
        "{:?}",
        lines(&invoice)
    );
    h.check().await;
}

/// D4: usage-only credit of the subscription pays the usage line only, never the shortfall.
#[tokio::test]
async fn s15_d4_usage_only_credit_never_pays_the_shortfall() {
    let h = Halyard::new().await;
    let (customer, sub) = on_enterprise(&h, Tax::Untaxed).await;
    let wallet_id = wallet(&h, customer).await;
    h.store()
        .fund_scoped_customer_credit_wallet_at(
            FundCustomerCreditWallet {
                tenant_id: h.tenant,
                customer_id: customer,
                wallet_id,
                amount_minor: 50_000,
                classification: GrantClassification::Promotional,
                priority: DEFAULT_PRIORITY,
                expires_at: None,
                idempotency_key: uuid::Uuid::now_v7(),
                reason: "Usage credit".to_owned(),
                reference: None,
                actor_id: None,
                cost: None,
            },
            GrantScope::subscription_usage(sub),
            at(9, 1),
        )
        .await
        .expect("usage-only credit of the subscription");
    // standard 31.25M = 30000.
    h.tokens(customer, at_time(9, 10, 10, 0, 0), STANDARD, 31_250_000);
    let invoice = h.renew(sub, at(10, 1)).await.expect("INV-1");
    assert_eq!(shortfall(&invoice), (470_000, 0));
    assert_eq!(
        money(&invoice),
        (500_000, 0, 30_000, 0, 470_000, 100_000, 370_000),
        "{:?}",
        lines(&invoice)
    );
    let grants = h.grants(customer).await;
    assert_eq!(
        grant(&grants, "MANUAL", "PROMOTIONAL", 50_000).remaining,
        20_000
    );
    h.sweep(customer, at(10, 1) + Duration::hours(1)).await;
    h.check().await;
}
