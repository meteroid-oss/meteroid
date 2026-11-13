//! Known deviations the invoice model follows, each pinned by its smallest case: fixing one
//! fails its test here, and the model's reference then follows the rule instead.
use chrono::NaiveDate;

use super::engine::Engine;
use super::reference::{Action, Billing, Catalog, CouponSpec, Currency, Discount, PlanSpec};

/// KW-01: 33 % of a 49.99 fee is 16.4967, taken as 16.49 where R-001 rounds it to 16.50.
#[tokio::test]
async fn kw01_a_percentage_coupon_truncates_to_the_minor_unit() {
    let mut engine = Engine::new().await;
    let plan = PlanSpec {
        fee: 4_999,
        billing: Billing::Advance,
        rate: 2_500,
    };
    engine
        .apply(&Action::Start {
            day: NaiveDate::from_ymd_opt(2025, 1, 6).expect("a day"),
            catalog: Catalog {
                currency: Currency::Usd,
                plans: [plan, plan],
                coupons: vec![CouponSpec {
                    discount: Discount::Percent(33),
                    cycles: Some(1),
                }],
                taxes: vec![],
            },
        })
        .await
        .expect("started");
    let opening = engine.invoice(0).await;
    assert_eq!(
        (opening.subtotal, opening.discount, opening.total),
        (4_999, 1_649, 3_350)
    );
    assert_eq!(engine.invariants().await, Vec::<String>::new());
}
