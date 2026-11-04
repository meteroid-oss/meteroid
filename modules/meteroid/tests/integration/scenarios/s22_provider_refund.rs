//! S22. Purchased credit refunded in the provider's dashboard, without a credit note: the refund
//! gives back what was collected above the invoice first, then takes back its share of each grant
//! the purchase minted, in each grant's own denomination.
use chrono::{DateTime, Utc};
use common_domain::actor::Actor;
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::services::{InvoiceDelivery, SellCustomerCredit};
use uuid::Uuid;

use super::kit::{Halyard, Tax, at, day, grant};
use super::s13_top_up_void_note_refund::unrecovered;
use super::s14_overpayment::{collect, deliver, overpaid, overpayment, provider_payment};

/// The provider reports the charge's refunds reaching `cumulative`, the last one `refund_id`.
async fn refund_at_provider(
    h: &Halyard,
    external_id: &str,
    cumulative: i64,
    refund_id: &str,
    at: DateTime<Utc>,
) {
    deliver(
        h,
        format!(
            r#"{{"id":"evt_re_{refund_id}","kind":"payment_refunded","external_id":"{external_id}","amount":{cumulative},"currency":"USD","refunds":[{{"id":"{refund_id}","succeeded":true}}],"occurred_at":"{at}"}}"#,
            at = at.to_rfc3339(),
        ),
    )
    .await;
}

/// S14's adjustment invoice (25000 asked, 30000 collected: a 25000 pack, its 2500 bonus and 5000
/// of OVERPAYMENT credit), 15000 refunded at the provider: the 5000 excess first, then 10000 of the
/// purchase, which takes back 10000 of the pack and 1000 of its bonus. Nothing was spent.
#[tokio::test]
async fn s22_an_overpaid_pack_refunded_at_the_provider_gives_back_the_excess_first() {
    let h = Halyard::new().await;
    let (customer, _, adjustment) = overpaid(&h).await;
    let external_id = provider_payment(&h, adjustment).await;

    refund_at_provider(&h, &external_id, 15_000, "re_s22_part", at(9, 20)).await;
    let grants = h.grants(customer).await;
    let pack = grant(&grants, "CREDIT_PURCHASE", "PAID", 25_000);
    let bonus = grant(&grants, "CREDIT_PURCHASE", "PROMOTIONAL", 2_500);
    assert_eq!(
        h.movements(overpayment(&grants).id).await,
        vec!["FUND 5000", "REVERSAL -5000"]
    );
    assert_eq!(
        h.movements_with_reasons(pack.id).await,
        vec!["FUND 25000", "REVERSAL -10000 Payment refunded"]
    );
    assert_eq!(
        h.movements(bonus.id).await,
        vec!["FUND 2500", "REVERSAL -1000"]
    );
    let paid = h.invoice(adjustment).await;
    assert_eq!(
        (paid.amount_due, paid.payment_status),
        (0, meteroid_store::domain::InvoicePaymentStatus::Paid)
    );
    h.check().await;

    refund_at_provider(&h, &external_id, 30_000, "re_s22_rest", at(9, 21)).await;
    let grants = h.grants(customer).await;
    assert_eq!(
        h.movements(pack.id).await,
        vec!["FUND 25000", "REVERSAL -10000", "REVERSAL -15000"]
    );
    assert_eq!(
        h.movements(bonus.id).await,
        vec!["FUND 2500", "REVERSAL -1000", "REVERSAL -1500"]
    );
    assert_eq!(
        h.movements(overpayment(&grants).id).await,
        vec!["FUND 5000", "REVERSAL -5000"],
        "the excess is given back once"
    );
    assert!(unrecovered(&h, adjustment).await.is_empty());
    h.check().await;
}

/// 2000 Halyard credits bought by a New York customer for $21.78 (tax included), paid by card;
/// half the charge refunded at the provider takes back 1000 credits: the share of the money, in
/// the purchase's unit.
#[tokio::test]
async fn s22_a_credit_unit_purchase_refunded_by_half_takes_back_half_its_credits() {
    let h = Halyard::new().await;
    let customer = h.customer("NY", Tax::NewYork).await;
    let (_, first) = h.subscribe(customer, h.studio, day(9, 1)).await;
    h.pay(first.expect("INV-0").id, at(9, 1)).await;
    let wallet = h
        .store()
        .create_customer_credit_wallet(h.tenant, customer, "HALYARD_CREDITS")
        .await
        .expect("the Halyard credits wallet")
        .id;
    let sold = h
        .services()
        .sell_customer_credit_at(
            Uuid::now_v7(),
            SellCustomerCredit {
                expires_at: None,
                tenant_id: h.tenant,
                customer_id: customer,
                wallet_id: wallet,
                amount_minor: 2_000,
                expires_in_months: Some(12),
                note: None,
                actor: Actor::System,
                unit_price: None,
                expected_unit_price: None,
            },
            at(9, 10),
            InvoiceDelivery::Suppressed,
        )
        .await
        .expect("the credits are sold");
    let purchase = sold.invoice.invoice;
    assert_eq!(purchase.total, 2_178);
    let external_id = collect(&h, purchase.id, 2_178, at(9, 10)).await;
    let units = grant(&h.grants(customer).await, "CREDIT_PURCHASE", "PAID", 2_000).id;

    refund_at_provider(&h, &external_id, 1_089, "re_s22_units", at(9, 12)).await;
    assert_eq!(
        h.movements(units).await,
        vec!["FUND 2000", "REVERSAL -1000"]
    );
    assert!(unrecovered(&h, purchase.id).await.is_empty());
    h.check().await;
}
