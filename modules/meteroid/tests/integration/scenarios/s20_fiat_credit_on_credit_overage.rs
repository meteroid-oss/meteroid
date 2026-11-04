//! S20. A $5 promotion code lapsing mid-period on Studio: the September usage beyond the plan's
//! 5,000 credits is billed in money ($0.01 a credit), so the lapsing dollars pay that overage
//! accrued by their expiry, whether the expiry sweep or the period's invoice comes first.
use common_domain::actor::Actor;
use common_domain::ids::{CustomerId, SubscriptionId};
use meteroid_store::domain::coupons::{CouponCreditScope, CouponDiscount, CouponNew};
use meteroid_store::services::sandbox_replay::SandboxReplayInterface;
use rust_decimal_macros::dec;

use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::services::{InvoiceDelivery, SellCustomerCredit};

use super::kit::{Halyard, LITE, M, Tax, at, at_time, day, grant, lines, money};

const CREDITS: &str = "HALYARD_CREDITS";

/// Studio from Sep 1 for an untaxed customer, INV-0 paid; a $5 code redeemed Sep 10, lapsing
/// Sep 15; 29.5M lite tokens (5,900 credits) on Sep 12: 900 credits, $9.00, beyond the plan's
/// credits before the code lapses.
async fn studio_with_a_five_dollar_code(h: &Halyard) -> (CustomerId, SubscriptionId) {
    studio_with_five_dollars(h, Tax::Untaxed, true).await
}

/// `denomination`'s `amount` sold on `on` and paid then, optionally lapsing at `expires_at`.
async fn sell(
    h: &Halyard,
    customer: CustomerId,
    denomination: &str,
    amount: i64,
    on: chrono::DateTime<chrono::Utc>,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
) {
    let wallet = h
        .store()
        .create_customer_credit_wallet(h.tenant, customer, denomination)
        .await
        .expect("the wallet")
        .id;
    let sold = h
        .services()
        .sell_customer_credit_at(
            uuid::Uuid::now_v7(),
            SellCustomerCredit {
                expires_at,
                tenant_id: h.tenant,
                customer_id: customer,
                wallet_id: wallet,
                amount_minor: amount,
                expires_in_months: expires_at.is_none().then_some(12),
                note: None,
                actor: Actor::System,
                unit_price: None,
                expected_unit_price: None,
            },
            on,
            InvoiceDelivery::Suppressed,
        )
        .await
        .expect("the credit is sold");
    h.pay(sold.invoice.invoice.id, on).await;
}

/// Studio from Sep 1, INV-0 paid; $5 lapsing Sep 15, as a promotion code redeemed Sep 10 or as
/// paid credit bought then; 29.5M lite tokens (5,900 credits) on Sep 12: 900 credits, $9.00,
/// beyond the plan's credits before the dollars lapse.
async fn studio_with_five_dollars(
    h: &Halyard,
    tax: Tax,
    promotional: bool,
) -> (CustomerId, SubscriptionId) {
    h.services()
        .create_coupon_at(
            Actor::System,
            CouponNew {
                code: "FIVE".to_owned(),
                description: "Five dollars".to_owned(),
                tenant_id: h.tenant,
                discount: CouponDiscount::Credit {
                    currency: "USD".to_owned(),
                    amount: dec!(5),
                    expires_in_days: Some(5),
                    scope: CouponCreditScope::Customer,
                },
                expires_at: None,
                redemption_limit: None,
                recurring_value: None,
                reusable: false,
                plan_ids: vec![],
            },
            at(1, 1),
        )
        .await
        .expect("the promotion code");
    let customer = h.customer("Five", tax).await;
    let (sub, first) = h.subscribe(customer, h.studio, day(9, 1)).await;
    h.pay(first.expect("INV-0").id, at(9, 1)).await;
    if promotional {
        let redeemed = h
            .services()
            .redeem_coupon_credit_code_at(h.tenant, customer, "FIVE", at(9, 10))
            .await
            .expect("redeemed in the portal");
        assert_eq!(
            (redeemed.amount_minor, redeemed.expires_at),
            (500, Some(at(9, 15)))
        );
    } else {
        sell(h, customer, "USD", 500, at(9, 10), Some(at(9, 15))).await;
    }
    h.tokens(customer, at_time(9, 12, 10, 0, 0), LITE, 59 * M / 2);
    h.check().await;
    (customer, sub)
}

/// INV-1: October's fee and September's 900 credits of overage, $5 of it paid by the code's
/// settlement: 4,900 + 900 − 500.
const OCTOBER: (i64, i64, i64, i64, i64, i64, i64) = (5_800, 0, 500, 0, 5_300, 0, 5_300);

#[tokio::test]
async fn s20_lapsing_dollars_pay_the_overage_the_sweep_settles() {
    let h = Halyard::new().await;
    let (customer, sub) = studio_with_a_five_dollar_code(&h).await;
    h.sweep(customer, at(9, 15) + Halyard::hours(1)).await;
    h.check().await;

    let october = h.renew(sub, at(10, 1)).await.expect("INV-1");
    let october = h.invoice(october.id).await;
    assert_eq!(money(&october), OCTOBER, "{:?}", lines(&october));
    h.check().await;
}

#[tokio::test]
async fn s20_lapsing_dollars_pay_the_overage_the_invoice_settles() {
    let h = Halyard::new().await;
    let (customer, sub) = studio_with_a_five_dollar_code(&h).await;

    let october = h.renew(sub, at(10, 1)).await.expect("INV-1");
    let october = h.invoice(october.id).await;
    assert_eq!(money(&october), OCTOBER, "{:?}", lines(&october));
    h.sweep(customer, at(10, 1) + Halyard::hours(1)).await;
    h.check().await;
}

/// 2,000 credits bought Sep 20, after the dollars lapsed: the October invoice covers all 5,900
/// September credits with units (S17: credit spendable at the invoice pays its period), so no
/// overage is billed and the $5 settled at the lapse goes back to its grant, lapsed.
const OCTOBER_WITH_UNITS: (i64, i64, i64, i64, i64, i64, i64) = (4_900, 0, 0, 0, 4_900, 0, 4_900);

async fn units_bought_after_the_lapse(h: &Halyard, customer: CustomerId) {
    sell(h, customer, CREDITS, 2_000, at(9, 20), None).await;
}

async fn assert_units_paid_the_overage(h: &Halyard, customer: CustomerId) {
    let grants = h.grants(customer).await;
    let units = grant(&grants, "CREDIT_PURCHASE", "PAID", 2_000);
    assert_eq!(h.movements(units.id).await, vec!["FUND 2000", "DRAW -900"]);
}

#[tokio::test]
async fn s20_units_bought_after_the_sweep_pay_the_overage_instead() {
    let h = Halyard::new().await;
    let (customer, sub) = studio_with_a_five_dollar_code(&h).await;
    h.sweep(customer, at(9, 15) + Halyard::hours(1)).await;
    units_bought_after_the_lapse(&h, customer).await;

    let october = h.renew(sub, at(10, 1)).await.expect("INV-1");
    let october = h.invoice(october.id).await;
    assert_eq!(money(&october), OCTOBER_WITH_UNITS, "{:?}", lines(&october));
    assert_units_paid_the_overage(&h, customer).await;
    h.check().await;
}

#[tokio::test]
async fn s20_units_bought_before_the_invoice_settles_the_lapse_pay_the_overage() {
    let h = Halyard::new().await;
    let (customer, sub) = studio_with_a_five_dollar_code(&h).await;
    units_bought_after_the_lapse(&h, customer).await;

    let october = h.renew(sub, at(10, 1)).await.expect("INV-1");
    let october = h.invoice(october.id).await;
    assert_eq!(money(&october), OCTOBER_WITH_UNITS, "{:?}", lines(&october));
    assert_units_paid_the_overage(&h, customer).await;
    h.sweep(customer, at(10, 1) + Halyard::hours(1)).await;
    h.check().await;
}

/// $5 of PAID credit lapsing Sep 15 for a New York customer settles the overage with its tax
/// (900 + 79.875): all 500. INV-1 taxes the fee and the overage together, 5,800 × 8.875% =
/// 514.75 → 515, and the settlement pays 500 of the 6,315 after tax.
#[tokio::test]
async fn s20_lapsing_paid_dollars_pay_the_overage_with_its_tax() {
    let h = Halyard::new().await;
    let (customer, sub) = studio_with_five_dollars(&h, Tax::NewYork, false).await;
    h.sweep(customer, at(9, 15) + Halyard::hours(1)).await;
    h.check().await;

    let october = h.renew(sub, at(10, 1)).await.expect("INV-1");
    let october = h.invoice(october.id).await;
    assert_eq!(
        money(&october),
        (5_800, 0, 0, 515, 6_315, 500, 5_815),
        "{:?}",
        lines(&october)
    );
    h.check().await;
}

/// Studio with a second credit-priced product, "Batch API": 100 credits per 1M tokens, priced
/// in Halyard credits at $0.01, on the same tokens. The $5 code lapses Sep 15 as above; the
/// 29.5M lite tokens of Sep 12 cost 5,900 credits on the plan's usage and 2,950 on Batch API.
async fn studio_with_batch_api(h: &Halyard) -> (CustomerId, SubscriptionId) {
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_async::RunQueryDsl;
    use meteroid_store::domain::enums::{BillingPeriodEnum, FeeTypeEnum};
    use meteroid_store::domain::price_components::{
        PriceEntry, PriceInput, ProductRef, UsagePricingModel,
    };
    use meteroid_store::domain::prices::{FeeStructure, Pricing, UsageModel};
    use meteroid_store::domain::subscription_components::{
        CreateSubscriptionComponents, ExtraComponent,
    };
    let metric = {
        use diesel_models::schema::billable_metric::dsl as m;
        m::billable_metric
            .filter(m::tenant_id.eq(h.tenant))
            .filter(m::code.eq("halyard_tokens"))
            .select(m::id)
            .first::<common_domain::ids::BillableMetricId>(&mut h.env.conn().await)
            .await
            .expect("the tokens metric")
    };
    let unit = {
        use diesel_models::schema::credit_unit::dsl as u;
        u::credit_unit
            .filter(u::tenant_id.eq(h.tenant))
            .filter(u::code.eq(CREDITS))
            .select(u::id)
            .first::<common_domain::ids::CreditUnitId>(&mut h.env.conn().await)
            .await
            .expect("the Halyard credit unit")
    };
    h.services()
        .create_coupon_at(
            Actor::System,
            CouponNew {
                code: "FIVE".to_owned(),
                description: "Five dollars".to_owned(),
                tenant_id: h.tenant,
                discount: CouponDiscount::Credit {
                    currency: "USD".to_owned(),
                    amount: dec!(5),
                    expires_in_days: Some(5),
                    scope: CouponCreditScope::Customer,
                },
                expires_at: None,
                redemption_limit: None,
                recurring_value: None,
                reusable: false,
                plan_ids: vec![],
            },
            at(1, 1),
        )
        .await
        .expect("the promotion code");
    let customer = h.customer("Batch", Tax::Untaxed).await;
    let model = UsagePricingModel::PerUnit { rate: dec!(100) };
    let mut request = h.request(customer, h.studio, day(9, 1));
    request.price_components = Some(CreateSubscriptionComponents {
        parameterized_components: vec![],
        overridden_components: vec![],
        extra_components: vec![ExtraComponent {
            name: "Batch API".to_owned(),
            product_ref: ProductRef::New {
                name: "Batch API".to_owned(),
                fee_type: FeeTypeEnum::Usage,
                fee_structure: FeeStructure::Usage {
                    metric_id: metric,
                    model: UsageModel::from(&model),
                },
            },
            price_entry: PriceEntry::New(PriceInput {
                cadence: BillingPeriodEnum::Monthly,
                currency: "USD".to_owned(),
                pricing: Pricing::Usage(model),
                credit_unit: Some(meteroid_store::domain::credit_units::PriceCreditUnit {
                    unit_id: unit,
                    rate: dec!(0.01),
                }),
            }),
        }],
        remove_components: vec![],
    });
    let (sub, first) = h.start(request).await;
    h.pay(first.expect("INV-0").id, at(9, 1)).await;
    h.services()
        .redeem_coupon_credit_code_at(h.tenant, customer, "FIVE", at(9, 10))
        .await
        .expect("redeemed in the portal");
    h.tokens(customer, at_time(9, 12, 10, 0, 0), LITE, 59 * M / 2);
    h.check().await;
    (customer, sub)
}

/// 2,000 credits bought Sep 20 join the plan's 5,000 at the invoice: 7,000 of the 8,850 credits
/// are covered, split 4,667 / 2,333 by consumption (largest remainder), leaving 1,233 + 617 credits
/// billed, $18.50; the $5 the code settled at its lapse (overage then 3,850 credits) pays $5 of it.
const OCTOBER_TWO_PRODUCTS: (i64, i64, i64, i64, i64, i64, i64) =
    (6_750, 0, 500, 0, 6_250, 0, 6_250);

async fn assert_two_products(h: &Halyard, customer: CustomerId, sub: SubscriptionId) {
    let october = h.renew(sub, at(10, 1)).await.expect("INV-1");
    let october = h.invoice(october.id).await;
    assert_eq!(
        money(&october),
        OCTOBER_TWO_PRODUCTS,
        "{:?}",
        lines(&october)
    );
    let grants = h.grants(customer).await;
    let units = grant(&grants, "CREDIT_PURCHASE", "PAID", 2_000);
    assert_eq!(h.movements(units.id).await, vec!["FUND 2000", "DRAW -2000"]);
}

#[tokio::test]
async fn s20_two_credit_priced_products_sweep_first() {
    let h = Halyard::new().await;
    let (customer, sub) = studio_with_batch_api(&h).await;
    h.sweep(customer, at(9, 15) + Halyard::hours(1)).await;
    units_bought_after_the_lapse(&h, customer).await;
    assert_two_products(&h, customer, sub).await;
    h.check().await;
}

#[tokio::test]
async fn s20_two_credit_priced_products_invoice_first() {
    let h = Halyard::new().await;
    let (customer, sub) = studio_with_batch_api(&h).await;
    units_bought_after_the_lapse(&h, customer).await;
    assert_two_products(&h, customer, sub).await;
    h.sweep(customer, at(10, 1) + Halyard::hours(1)).await;
    h.check().await;
}
