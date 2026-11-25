//! Extra recurring and one-time fees whose quantity is priced through a model (price model v3,
//! step 2b): the model's total is billed, the line shows the quantity and a subline per tier or
//! package the model priced.

use chrono::NaiveDate;
use common_domain::ids::{
    BaseId, PlanId, PlanVersionId, PriceComponentId, PriceId, SubscriptionId,
};
use diesel_models::enums::BillingPeriodEnum as DieselBillingPeriodEnum;
use meteroid_store::domain::price_components::{RatingModel, TierRow};
use meteroid_store::domain::prices::Pricing;
use meteroid_store::domain::{BillingType, Invoice};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::data::ids::*;
use crate::data::plans::{PlanSeed, SeedComp};
use crate::harness::{TestEnv, subscription, test_env_with_seed};
use crate::meteroid_it::container::SeedLevel;

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn tier(first_unit: u64, rate: Decimal) -> TierRow {
    TierRow {
        first_unit,
        rate,
        flat_fee: None,
        flat_cap: None,
    }
}

/// "Extra": 15 units a month in advance through graduated tiers, 10.00 for the first ten and
/// 8.00 after. "Setup": 25 units once, in packages of 10 at 30.00. Their unit prices, 99.00, are
/// ignored.
async fn modelled_plan(env: &TestEnv) -> PlanVersionId {
    let version_id = PlanVersionId::new();
    let mut extra = SeedComp::extra_recurring(
        PriceComponentId::new(),
        "Extra",
        PRODUCT_EXTRA_ADVANCE_ID,
        PriceId::new(),
        DieselBillingPeriodEnum::Monthly,
        dec!(99),
        15,
        BillingType::Advance,
    );
    extra.prices[0].pricing = Pricing::ExtraRecurring {
        unit_price: dec!(99),
        quantity: 15,
        model: Some(RatingModel::Tiered {
            tiers: vec![tier(0, dec!(10)), tier(10, dec!(8))],
        }),
    };
    let mut setup = SeedComp::one_time(
        PriceComponentId::new(),
        "Setup",
        PRODUCT_ONETIME_SETUP_ID,
        PriceId::new(),
        dec!(99),
        25,
    );
    setup.prices[0].pricing = Pricing::OneTime {
        unit_price: dec!(99),
        quantity: 25,
        model: Some(RatingModel::Package {
            block_size: 10,
            rate: dec!(30),
        }),
    };
    PlanSeed::new(PlanId::new(), "Modelled fees", version_id)
        .components(vec![extra, setup])
        .seed(&mut env.conn().await)
        .await
        .unwrap();
    version_id
}

async fn start(env: &TestEnv, plan_version_id: PlanVersionId, on: NaiveDate) -> SubscriptionId {
    subscription()
        .plan_version(plan_version_id)
        .start_date(on)
        .on_start()
        .no_trial()
        .create(env.services())
        .await
}

async fn invoice_of(env: &TestEnv, sub_id: SubscriptionId, on: NaiveDate) -> Invoice {
    let invoices = env.get_invoices(sub_id).await;
    invoices
        .iter()
        .find(|invoice| invoice.billing_period_start == on)
        .cloned()
        .unwrap_or_else(|| {
            let seen: Vec<_> = invoices
                .iter()
                .map(|i| (i.billing_period_start, lines(i)))
                .collect();
            panic!("no invoice starting {on}: {seen:?}")
        })
}

/// Each line's name, amount, quantity and unit price.
fn lines(invoice: &Invoice) -> Vec<(String, i64, Option<Decimal>, Option<Decimal>)> {
    let mut lines: Vec<_> = invoice
        .line_items
        .iter()
        .map(|l| (l.name.clone(), l.amount_subtotal, l.quantity, l.unit_price))
        .collect();
    lines.sort_by(|a, b| a.0.cmp(&b.0));
    lines
}

/// Each subline's name, quantity, unit price and total, of the line named after `name`.
fn sub_lines(invoice: &Invoice, name: &str) -> Vec<(String, Decimal, Decimal, i64)> {
    invoice
        .line_items
        .iter()
        .find(|l| l.name.starts_with(name))
        .map(|l| {
            l.sub_lines
                .iter()
                .map(|s| (s.name.clone(), s.quantity, s.unit_price, s.total))
                .collect()
        })
        .unwrap_or_default()
}

/// The first invoice bills 10 × 10.00 + 5 × 8.00 = 140.00 for the 15 extra units and 3 packages
/// of 30.00 for the 25 setup units; the renewal bills the extra units again, never the setup.
/// MRR counts the extra units' model total.
#[tokio::test]
async fn fixed_quantities_are_billed_through_their_model() {
    let env = test_env_with_seed(SeedLevel::PLANS).await;
    let plan = modelled_plan(&env).await;
    let sub_id = start(&env, plan, day(2024, 1, 1)).await;

    let january = invoice_of(&env, sub_id, day(2024, 1, 1)).await;
    let january_lines = lines(&january);
    let extra = january_lines
        .iter()
        .find(|line| line.0.starts_with("Extra"))
        .unwrap_or_else(|| panic!("an Extra line: {january_lines:?}"));
    assert_eq!(
        (extra.1, extra.2, extra.3),
        (14_000, Some(dec!(15)), None),
        "{january_lines:?}"
    );
    assert_eq!(
        sub_lines(&january, "Extra"),
        vec![
            ("First 10".to_owned(), dec!(10), dec!(10), 10_000),
            ("Next 5".to_owned(), dec!(5), dec!(8), 4_000)
        ]
    );
    let setup = january_lines
        .iter()
        .find(|line| line.0.starts_with("Setup"))
        .unwrap_or_else(|| panic!("a Setup line: {january_lines:?}"));
    assert_eq!(
        (setup.1, setup.2, setup.3),
        (9_000, Some(dec!(25)), None),
        "{january_lines:?}"
    );
    assert_eq!(
        sub_lines(&january, "Setup"),
        vec![("Packs of 10".to_owned(), dec!(3), dec!(30), 9_000)]
    );
    assert_eq!(january.subtotal, 23_000, "{january_lines:?}");

    env.process_cycles().await;
    let february = invoice_of(&env, sub_id, day(2024, 2, 1)).await;
    let february_lines = lines(&february);
    assert!(
        february_lines
            .iter()
            .all(|line| !line.0.starts_with("Setup")),
        "the setup is billed once: {february_lines:?}"
    );
    assert_eq!(february.subtotal, 14_000, "{february_lines:?}");

    assert_eq!(env.get_subscription(sub_id).await.mrr_cents, 14_000);
}

/// Started on Jan 15 with its billing anchored on the 1st, the first period is 17 of January's
/// 31 days: the extra units' 140.00 is prorated on its total, 14000 × 17/31 = 7677, never
/// through 15 rounded unit prices, and shown as 15 at 5.118. The setup is billed whole.
#[tokio::test]
async fn a_modelled_recurring_quantity_is_prorated_on_its_total() {
    let env = test_env_with_seed(SeedLevel::PLANS).await;
    let plan = modelled_plan(&env).await;
    let sub_id = subscription()
        .plan_version(plan)
        .start_date(day(2024, 1, 15))
        .billing_day_anchor(1)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let first = invoice_of(&env, sub_id, day(2024, 1, 15)).await;
    let first_lines = lines(&first);
    let extra = first_lines
        .iter()
        .find(|line| line.0.starts_with("Extra"))
        .unwrap_or_else(|| panic!("an Extra line: {first_lines:?}"));
    assert_eq!(extra.1, 7_677, "{first_lines:?}");
    // A part of the period shows the rate its total works out to, not the period's tiers.
    assert_eq!(extra.3, Some(dec!(5.118)), "{first_lines:?}");
    assert!(sub_lines(&first, "Extra").is_empty());
    let setup = first_lines
        .iter()
        .find(|line| line.0.starts_with("Setup"))
        .unwrap_or_else(|| panic!("a Setup line: {first_lines:?}"));
    assert_eq!(setup.1, 9_000, "{first_lines:?}");
}

/// "Seats": 15 seats a month at the volume rate of 8.00 (10.00 under 10 seats), at least 5,
/// 120.00. Crediting 7 seats gives back 120.00 less the 80.00 the 8 seats kept are priced, 40.00,
/// not 7 × 8.00; half a seat is refused.
#[tokio::test]
async fn a_seat_line_credited_by_quantity_gives_back_what_its_model_priced() {
    use common_domain::actor::Actor;
    use meteroid_store::domain::enums::{CreditType, InvoiceStatusEnum};
    use meteroid_store::domain::{
        ComponentParameterization, ComponentParameters, CreateSubscriptionComponents,
    };
    use meteroid_store::repositories::CreditNoteInterface;
    use meteroid_store::repositories::credit_notes::{CreateCreditNoteParams, CreditLineItem};

    let env = test_env_with_seed(SeedLevel::PLANS).await;
    let version_id = PlanVersionId::new();
    let component_id = PriceComponentId::new();
    let mut seats = SeedComp::slot(
        component_id,
        "Seats",
        PRODUCT_SEATS_ID,
        PriceId::new(),
        DieselBillingPeriodEnum::Monthly,
        dec!(99),
    );
    seats.prices[0].pricing = Pricing::Slot {
        model: RatingModel::Volume {
            tiers: vec![tier(0, dec!(10)), tier(10, dec!(8))],
        },
        min_slots: Some(5),
        max_slots: None,
    };
    PlanSeed::new(PlanId::new(), "Volume seats", version_id)
        .components(vec![seats])
        .seed(&mut env.conn().await)
        .await
        .unwrap();
    let mut request = subscription()
        .plan_version(version_id)
        .start_date(day(2024, 1, 1))
        .on_start()
        .no_trial()
        .request();
    request.price_components = Some(CreateSubscriptionComponents {
        parameterized_components: vec![ComponentParameterization {
            component_id,
            parameters: ComponentParameters {
                initial_slot_count: Some(15),
                billing_period: None,
                committed_capacity: None,
            },
        }],
        overridden_components: vec![],
        extra_components: vec![],
        remove_components: vec![],
    });
    let sub_id = env
        .services()
        .insert_subscription(Actor::System, request, TENANT_ID)
        .await
        .expect("15 seats")
        .id;

    let january = invoice_of(&env, sub_id, day(2024, 1, 1)).await;
    assert_eq!(january.status, InvoiceStatusEnum::Finalized);
    let line = january
        .line_items
        .iter()
        .find(|line| line.name.starts_with("Seats"))
        .expect("the seat line");
    assert_eq!(
        (line.amount_subtotal, line.quantity),
        (12_000, Some(dec!(15)))
    );
    let credit = |quantity| CreateCreditNoteParams {
        invoice_id: january.id,
        line_items: vec![CreditLineItem::Line {
            local_id: line.local_id.clone(),
            quantity,
        }],
        reason: None,
        memo: None,
        credit_type: CreditType::DebtCancellation,
        custom_properties: None,
    };

    let half_seat = env
        .store()
        .create_credit_note(Actor::System, TENANT_ID, credit(dec!(0.5)))
        .await
        .expect_err("seats are credited whole");
    assert!(
        format!("{half_seat:?}").contains("whole seats"),
        "{half_seat:?}"
    );

    let note = env
        .store()
        .create_credit_note(Actor::System, TENANT_ID, credit(dec!(7)))
        .await
        .expect("7 seats credited");
    assert_eq!(note.subtotal, -4_000, "120.00 − 80.00 for the 8 seats kept");
}
