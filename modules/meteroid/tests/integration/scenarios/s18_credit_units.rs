//! S18. Halyard credits bought and consumed by a New York customer: a unit purchase is taxed at
//! sale on what was paid (half-up, per rate), usage the units cover is untaxed, usage beyond them
//! is billed in money and taxed; subscription credits burn before customer-wide purchased ones.
use common_domain::actor::Actor;
use common_domain::ids::{CreditUnitId, CustomerId, InvoiceId, SubscriptionId, WalletId};
use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::CreditUnitsInterface;
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::services::{InvoiceDelivery, SellCustomerCredit};
use rust_decimal_macros::dec;
use uuid::Uuid;

use super::kit::{
    Halyard, LITE, M, STANDARD, Tax, at, at_time, day, expiring, grant, lines, money,
};

const CREDITS: &str = "HALYARD_CREDITS";

async fn credits_unit(h: &Halyard) -> CreditUnitId {
    use diesel_models::schema::credit_unit::dsl as u;
    u::credit_unit
        .filter(u::tenant_id.eq(h.tenant))
        .filter(u::code.eq(CREDITS))
        .select(u::id)
        .first::<CreditUnitId>(&mut h.env.conn().await)
        .await
        .expect("the Halyard credit unit")
}

async fn credits_wallet(h: &Halyard, customer: CustomerId) -> WalletId {
    h.store()
        .create_customer_credit_wallet(h.tenant, customer, CREDITS)
        .await
        .expect("the Halyard credits wallet")
        .id
}

/// The portal's purchase of `credits` at list price on `on`, card paid then; 12 months to use.
async fn buy(
    h: &Halyard,
    customer: CustomerId,
    credits: i64,
    on: chrono::DateTime<chrono::Utc>,
) -> InvoiceId {
    let wallet = credits_wallet(h, customer).await;
    let sold = h
        .services()
        .sell_customer_credit_at(
            Uuid::now_v7(),
            SellCustomerCredit {
                expires_at: None,
                tenant_id: h.tenant,
                customer_id: customer,
                wallet_id: wallet,
                amount_minor: credits,
                expires_in_months: Some(12),
                note: None,
                actor: Actor::System,
                unit_price: None,
                expected_unit_price: None,
            },
            on,
            InvoiceDelivery::Suppressed,
        )
        .await
        .expect("the credits are sold");
    sold.invoice.invoice.id
}

/// Studio from Sep 1 for a New York customer: INV-0 bills the fee with 8.875% of tax.
async fn studio_in_new_york(h: &Halyard) -> (CustomerId, SubscriptionId) {
    let customer = h.customer("NY", Tax::NewYork).await;
    let (sub, first) = h.subscribe(customer, h.studio, day(9, 1)).await;
    let first = first.expect("INV-0");
    assert_eq!(
        money(&first),
        (4_900, 0, 0, 435, 5_335, 0, 5_335),
        "4900 × 0.08875 = 434.875, half-up"
    );
    h.pay(first.id, at(9, 1)).await;
    let grants = h.grants(customer).await;
    let s1 = expiring(&grants, "INCLUDED_CREDIT", at(10, 1));
    assert_eq!(h.denomination(s1).await, CREDITS);
    assert_eq!((s1.amount, s1.subscription_id), (5_000, Some(sub)));
    h.check().await;
    (customer, sub)
}

/// U: 2000 credits bought Sep 10 at $0.01, paid then.
async fn bought_two_thousand(h: &Halyard, customer: CustomerId) {
    let wallet = credits_wallet(h, customer).await;
    let quote = h
        .services()
        .quote_credit_sale(h.tenant, customer, wallet, 2_000, None)
        .await
        .expect("a quote");
    assert_eq!(
        (
            quote.quote.net_minor,
            quote.quote.tax_minor,
            quote.quote.charge_minor
        ),
        (2_000, 178, 2_178),
        "177.5 → 178, half-up"
    );
    assert_eq!(quote.unit_price, Some(dec!(0.01)));
    let invoice = buy(h, customer, 2_000, at(9, 10)).await;
    let invoice = h.invoice(invoice).await;
    assert_eq!(money(&invoice), (2_000, 0, 0, 178, 2_178, 0, 2_178));
    h.pay(invoice.id, at(9, 10)).await;
    let grants = h.grants(customer).await;
    let units = grant(&grants, "CREDIT_PURCHASE", "PAID", 2_000);
    assert_eq!(h.denomination(units).await, CREDITS);
    assert_eq!(
        (
            units.cost_minor,
            units.cost_currency.as_deref(),
            units.subscription_id,
            units.expires_at
        ),
        (
            Some(2_000),
            Some("USD"),
            None,
            Some(
                chrono::NaiveDate::from_ymd_opt(2026, 9, 10)
                    .unwrap()
                    .and_time(chrono::NaiveTime::MIN)
                    .and_utc()
            )
        ),
        "customer-wide, at cost, 12 months after the payment"
    );
    h.check().await;
}

#[tokio::test]
async fn s18_units_bought_then_burnt_after_the_plan_s_credits() {
    let h = Halyard::new().await;
    let (customer, sub) = studio_in_new_york(&h).await;
    bought_two_thousand(&h, customer).await;
    if let Some(rev) = h.revenue(customer, day(9, 10)).await {
        assert_eq!(
            rev.billed,
            4_900 + 2_000,
            "the purchase is billed net of tax"
        );
    }

    // September: standard 5M (6000 credits) + lite 2.5M (500 credits) = 6500 credits.
    h.tokens(customer, at_time(9, 15, 10, 0, 0), STANDARD, 5 * M);
    h.tokens(customer, at_time(9, 25, 10, 0, 0), LITE, 5 * M / 2);

    let october = h.renew(sub, at(10, 1)).await.expect("INV-1");
    h.sweep(customer, at(10, 1) + Halyard::hours(1)).await;
    let october = h.invoice(october.id).await;
    assert_eq!(
        money(&october),
        (4_900, 0, 0, 435, 5_335, 0, 5_335),
        "6500 credits covered and untaxed, October's fee taxed: {:?}",
        lines(&october)
    );
    let grants = h.grants(customer).await;
    assert_eq!(
        h.movements(expiring(&grants, "INCLUDED_CREDIT", at(10, 1)).id)
            .await,
        vec!["FUND 5000", "DRAW -5000"],
        "the subscription's credits first, though the purchase is paid; nothing rolls"
    );
    let units = grant(&grants, "CREDIT_PURCHASE", "PAID", 2_000);
    assert_eq!(h.movements(units.id).await, vec!["FUND 2000", "DRAW -1500"]);
    h.check().await;

    if let Some(rev) = h.revenue(customer, day(10, 1)).await {
        assert_eq!(
            (rev.billed, rev.recognized, rev.deferred()),
            (4_900 + 2_000 + 4_900, 4_900 + 1_500, 500 + 4_900),
            "units: 1500 used at cost, 500 deferred; September's fee recognized, October's deferred"
        );
    }
}

/// Variant: 7800 credits spend S1 and U whole; 800 credits are billed at $0.01 and taxed with the
/// fee at the one rate: 5700 × 0.08875 = 505.875 → 506, split 435 + 71 by largest remainder.
#[tokio::test]
async fn s18_usage_beyond_the_units_is_billed_in_money_and_taxed() {
    let h = Halyard::new().await;
    let (customer, sub) = studio_in_new_york(&h).await;
    bought_two_thousand(&h, customer).await;
    // standard 6.5M = 7800 credits.
    h.tokens(customer, at_time(9, 15, 10, 0, 0), STANDARD, 13 * M / 2);

    let october = h.renew(sub, at(10, 1)).await.expect("INV-1");
    h.sweep(customer, at(10, 1) + Halyard::hours(1)).await;
    let october = h.invoice(october.id).await;
    assert_eq!(
        money(&october),
        (5_700, 0, 0, 506, 6_206, 0, 6_206),
        "{:?}",
        lines(&october)
    );
    let mut taxed: Vec<(i64, i64)> = october
        .line_items
        .iter()
        .filter(|line| line.amount_subtotal != 0)
        .map(|line| (line.amount_subtotal, line.tax_amount))
        .collect();
    taxed.sort();
    assert_eq!(taxed, vec![(800, 71), (4_900, 435)]);
    let grants = h.grants(customer).await;
    assert_eq!(
        grant(&grants, "CREDIT_PURCHASE", "PAID", 2_000).remaining,
        0
    );
    h.check().await;
    if let Some(rev) = h.revenue(customer, day(10, 1)).await {
        assert_eq!(
            rev.recognized,
            4_900 + 2_000 + 800,
            "every unit used at cost, September's fee and the usage billed in money"
        );
    }
}

/// Variant: the list price moves to $0.012 between the quote and the payment: paying the old
/// quote is refused and nothing is sold (R28).
#[tokio::test]
async fn s18_a_changed_price_refuses_the_old_quote() {
    let h = Halyard::new().await;
    let (customer, _) = studio_in_new_york(&h).await;
    let wallet = credits_wallet(&h, customer).await;
    let quote = h
        .services()
        .quote_credit_sale(h.tenant, customer, wallet, 2_000, None)
        .await
        .expect("a quote")
        .quote;
    h.store()
        .set_credit_unit_price(h.tenant, credits_unit(&h).await, "USD", dec!(0.012))
        .await
        .expect("a new list price");
    let refused = h
        .services()
        .create_top_up_checkout(
            h.tenant,
            customer,
            wallet,
            2_000,
            "s18-price-change".to_owned(),
            None,
            None,
            Some(quote),
        )
        .await
        .expect_err("the page showed the old price");
    match refused.current_context() {
        StoreError::FailedPrecondition(message) => {
            assert_eq!(message, "The price changed; refresh before paying.")
        }
        other => panic!("expected a precondition, got {other:?}"),
    }
    let grants = h.grants(customer).await;
    assert!(
        grants.iter().all(|g| g.source_kind != "CREDIT_PURCHASE"),
        "nothing was sold"
    );
    h.check().await;
}

/// Variant: 1200 credits cost $12.00 with 106.5 of tax, rounded half-up to 107, never half-even.
#[tokio::test]
async fn s18_a_purchase_s_tax_rounds_half_up() {
    let h = Halyard::new().await;
    let (customer, _) = studio_in_new_york(&h).await;
    let wallet = credits_wallet(&h, customer).await;
    let quote = h
        .services()
        .quote_credit_sale(h.tenant, customer, wallet, 1_200, None)
        .await
        .expect("a quote")
        .quote;
    assert_eq!(
        (quote.net_minor, quote.tax_minor, quote.charge_minor),
        (1_200, 107, 1_307)
    );
    let invoice = h.invoice(buy(&h, customer, 1_200, at(9, 10)).await).await;
    assert_eq!(money(&invoice), (1_200, 0, 0, 107, 1_307, 0, 1_307));
    h.check().await;
}

/// S18-late (OQ11): Studio with a paid unit pack, $20 for 2,000 credits ending with the period,
/// INV-0 paid on Oct 3, after INV-1 billed September. On time, September's 6,000 credits would
/// have drawn the plan's 5,000 (lapsing with the pack, promotional first), then 1,000 of the pack:
/// that half of its fee comes back to the balance; its other 1,000 credits lapse.
#[tokio::test]
async fn s18_a_unit_pack_paid_after_its_period_credits_back_what_it_would_have_paid() {
    use meteroid_store::domain::enums::{BillingPeriodEnum, FeeTypeEnum};
    use meteroid_store::domain::price_components::{
        CreditExpiry, PriceEntry, PriceInput, ProductRef,
    };
    use meteroid_store::domain::prices::{FeeStructure, Pricing};
    use meteroid_store::domain::subscription_components::{
        CreateSubscriptionComponents, ExtraComponent,
    };
    use meteroid_store::repositories::CreditNoteInterface;
    use meteroid_store::repositories::credit_notes::CreditType;

    let h = Halyard::new().await;
    let unit = credits_unit(&h).await;
    let customer = h.customer("Late", Tax::Untaxed).await;
    let mut request = h.request(customer, h.studio, day(9, 1));
    request.price_components = Some(CreateSubscriptionComponents {
        parameterized_components: vec![],
        overridden_components: vec![],
        extra_components: vec![ExtraComponent {
            name: "Credit pack".to_owned(),
            product_ref: ProductRef::New {
                name: "Credit pack".to_owned(),
                fee_type: FeeTypeEnum::Credit,
                fee_structure: FeeStructure::Credit {},
            },
            price_entry: PriceEntry::New(PriceInput {
                cadence: BillingPeriodEnum::Monthly,
                currency: "USD".to_owned(),
                pricing: Pricing::Credit {
                    unit_price: dec!(20),
                    quantity: 1,
                    credit_amount: dec!(2000),
                    expiry: Some(CreditExpiry::BillingPeriod),
                    credit_unit_id: Some(unit),
                    rollover: None,
                    once: false,
                },
                credit_unit: None,
            }),
        }],
        remove_components: vec![],
    });
    let (sub, first) = h.start(request).await;
    let first = first.expect("INV-0");
    assert_eq!(
        money(&first),
        (6_900, 0, 0, 0, 6_900, 0, 6_900),
        "{:?}",
        lines(&first)
    );
    // standard 5M = 6000 credits.
    h.tokens(customer, at_time(9, 15, 10, 0, 0), STANDARD, 5 * M);
    let october = h.renew(sub, at(10, 1)).await.expect("INV-1");
    h.sweep(customer, at(10, 1) + Halyard::hours(1)).await;
    let october = h.invoice(october.id).await;
    assert_eq!(
        money(&october),
        (7_900, 0, 0, 0, 7_900, 0, 7_900),
        "October's fee and pack, 1000 credits beyond the plan's billed at $0.01: {:?}",
        lines(&october)
    );

    h.pay(first.id, at(10, 3)).await;
    h.sweep(customer, at(10, 3) + Halyard::hours(1)).await;
    let notes = h
        .store()
        .list_credit_notes_by_invoice_id(h.tenant, first.id)
        .await
        .expect("notes");
    assert_eq!(notes.len(), 1);
    assert_eq!(
        (notes[0].credit_type, notes[0].subtotal, notes[0].total),
        (CreditType::CreditToBalance, -1_000, -1_000)
    );
    let grants = h.grants(customer).await;
    let pack = grant(&grants, "CREDIT_PURCHASE", "PAID", 2_000);
    assert_eq!(h.denomination(pack).await, CREDITS);
    assert_eq!(
        h.movements_with_reasons(pack.id).await,
        vec![
            "FUND 2000",
            "REVERSAL -1000 Removed: paid after its period was invoiced",
            "EXPIRY -1000 Removed: paid after its period was invoiced",
        ]
    );
    let balance = grant(&grants, "CREDIT_NOTE", "PAID", 1_000);
    assert_eq!(h.denomination(balance).await, "USD");
    assert_eq!((balance.subscription_id, balance.remaining), (None, 1_000));
    h.check().await;
}
