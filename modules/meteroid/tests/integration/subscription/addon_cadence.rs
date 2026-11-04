//! A monthly add-on on an annual subscription: its stub is prorated over its own month, and from
//! then on the subscription renews monthly, billing the add-on every month and the annual fee on
//! its anniversary — as if the subscription had been created with the add-on.
use chrono::{Datelike, NaiveDate};
use common_domain::actor::Actor;
use common_domain::ids::{AddOnId, BaseId, PlanId, PlanVersionId, PriceComponentId, PriceId};
use diesel_models::enums::BillingPeriodEnum as DieselBillingPeriodEnum;
use meteroid_store::domain::checkout_sessions::CheckoutCompletionResult;
use meteroid_store::domain::enums::{FeeTypeEnum, InvoiceType};
use meteroid_store::domain::price_components::{PriceEntry, PriceInput, ProductRef};
use meteroid_store::domain::prices::{FeeStructure, Pricing};
use meteroid_store::domain::subscription_add_ons::{
    CreateSubscriptionAddOn, SubscriptionAddOnCustomization,
};
use meteroid_store::domain::subscription_amendment::{
    AddOnChanges, ComponentChanges, SubscriptionAmendment,
};
use meteroid_store::domain::subscription_changes::PlanChangeMode;
use meteroid_store::domain::{BillingPeriodEnum, Invoice};
use meteroid_store::repositories::add_ons::AddOnInterface;
use rust_decimal_macros::dec;

use crate::data::ids::*;
use crate::data::plans::{PlanSeed, SeedComp};
use crate::harness::{TestEnv, subscription, test_env};

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// A 10 000.00 EUR yearly platform fee, billed in advance.
async fn annual_plan(env: &TestEnv) -> PlanVersionId {
    let version_id = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), "Yearly", version_id)
        .components(vec![SeedComp::rate(
            PriceComponentId::new(),
            "Platform Fee",
            PRODUCT_PLATFORM_FEE_ID,
            PriceId::new(),
            DieselBillingPeriodEnum::Annual,
            dec!(10000),
        )])
        .seed(&mut env.conn().await)
        .await
        .unwrap();
    version_id
}

/// A 100.00 EUR monthly add-on.
async fn monthly_add_on(env: &TestEnv) -> AddOnId {
    env.store()
        .create_add_on_from_ref(
            "Add-on".to_string(),
            ProductRef::New {
                name: "Add-on Product".to_string(),
                fee_type: FeeTypeEnum::Rate,
                fee_structure: FeeStructure::Rate {},
            },
            PriceEntry::New(PriceInput {
                cadence: BillingPeriodEnum::Monthly,
                currency: "EUR".to_string(),
                pricing: Pricing::Rate { rate: dec!(100) },
                credit_unit: None,
            }),
            None,
            true,
            Some(10),
            TENANT_ID,
            PRODUCT_FAMILY_ID,
            vec![],
        )
        .await
        .expect("add-on")
        .id
}

fn add(add_on_id: AddOnId) -> SubscriptionAmendment {
    SubscriptionAmendment {
        apply_mode: PlanChangeMode::Immediate,
        component_changes: ComponentChanges::default(),
        add_on_changes: AddOnChanges {
            added: vec![CreateSubscriptionAddOn {
                add_on_id,
                customization: SubscriptionAddOnCustomization::None,
                quantity: 1,
            }],
            edited: vec![],
            removed: vec![],
        },
    }
}

/// `amount × part / whole`, rounded half away from zero.
fn prorate(amount: i64, part: i64, whole: i64) -> i64 {
    (amount * part * 2 + whole) / (whole * 2)
}

/// The month `[start, end)` holding `on`, on a grid anchored on the 1st.
fn month_of(on: NaiveDate) -> (NaiveDate, NaiveDate) {
    let start = on.with_day(1).unwrap();
    (
        start,
        start.checked_add_months(chrono::Months::new(1)).unwrap(),
    )
}

/// (line name, amount, start, end) of an invoice, sorted by start then name.
fn lines(invoice: &Invoice) -> Vec<(String, i64, NaiveDate, NaiveDate)> {
    let mut lines: Vec<_> = invoice
        .line_items
        .iter()
        .map(|l| (l.name.clone(), l.amount_subtotal, l.start_date, l.end_date))
        .collect();
    lines.sort_by(|a, b| (a.2, &a.0).cmp(&(b.2, &b.0)));
    lines
}

/// The renewal billing the period starting `on` (an invoice is dated when finalized).
async fn recurring_invoice_on(
    env: &TestEnv,
    sub_id: common_domain::ids::SubscriptionId,
    on: NaiveDate,
) -> Invoice {
    env.get_invoices(sub_id)
        .await
        .into_iter()
        .find(|i| {
            i.invoice_type == InvoiceType::Recurring
                && i.line_items.iter().any(|l| l.start_date == on)
        })
        .unwrap_or_else(|| panic!("a recurring invoice on {on}"))
}

/// The reported case: yearly plan from May 1, a 100.00 monthly add-on added on Oct 22. The add-on
/// is charged 10/31 of a month for [Oct 22, Nov 1), then 100.00 on every monthly renewal; the
/// yearly fee is billed again on May 1 only.
#[rstest::rstest]
#[tokio::test]
async fn a_monthly_add_on_on_an_annual_subscription_bills_monthly(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let plan = annual_plan(&env).await;
    let add_on = monthly_add_on(&env).await;
    let sub_id = subscription()
        .plan_version(plan)
        .start_date(day(2024, 5, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    env.services()
        .apply_amendment_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            add(add_on),
            day(2024, 10, 22),
        )
        .await
        .expect("add the add-on");

    let adjustments: Vec<Invoice> = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .filter(|i| i.invoice_type == InvoiceType::Adjustment)
        .collect();
    assert_eq!(adjustments.len(), 1);
    assert_eq!(
        lines(&adjustments[0]),
        vec![(
            "Add-on (prorated)".to_string(),
            prorate(10_000, 10, 31),
            day(2024, 10, 22),
            day(2024, 11, 1)
        )]
    );

    let sub = env.get_subscription(sub_id).await;
    assert!(matches!(sub.period, DieselBillingPeriodEnum::Monthly));
    assert_eq!(
        (sub.current_period_start, sub.current_period_end),
        (day(2024, 10, 1), Some(day(2024, 11, 1)))
    );

    for month in [11, 12, 1, 2, 3, 4] {
        env.process_cycles().await;
        let on = day(if month > 10 { 2024 } else { 2025 }, month, 1);
        let (start, end) = month_of(on);
        assert_eq!(
            lines(&recurring_invoice_on(&env, sub_id, on).await),
            vec![("Add-on".to_string(), 10_000, start, end)],
            "{on} bills the add-on only"
        );
    }

    env.process_cycles().await;
    assert_eq!(
        lines(&recurring_invoice_on(&env, sub_id, day(2025, 5, 1)).await),
        vec![
            (
                "Add-on".to_string(),
                10_000,
                day(2025, 5, 1),
                day(2025, 6, 1)
            ),
            (
                "Platform Fee".to_string(),
                1_000_000,
                day(2025, 5, 1),
                day(2026, 5, 1)
            ),
        ]
    );
    let sub = env.get_subscription(sub_id).await;
    assert_eq!(
        (sub.current_period_start, sub.current_period_end),
        (day(2025, 5, 1), Some(day(2025, 6, 1)))
    );
}

/// The preview of the same change, made today: the stub runs to the end of this month, the
/// summary counts this month's days, and the next invoice is next month's, billing the add-on only.
#[rstest::rstest]
#[tokio::test]
async fn the_preview_prorates_a_monthly_add_on_over_its_month(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let today = chrono::Utc::now().date_naive();
    let (month_start, month_end) = month_of(today);
    let plan = annual_plan(&env).await;
    let add_on = monthly_add_on(&env).await;
    let sub_id = subscription()
        .plan_version(plan)
        .start_date(
            month_start
                .checked_sub_months(chrono::Months::new(5))
                .unwrap(),
        )
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let preview = env
        .services()
        .preview_amendment(sub_id, TENANT_ID, add(add_on))
        .await
        .expect("preview");

    let (left, days) = (
        (month_end - today).num_days(),
        (month_end - month_start).num_days(),
    );
    let proration = preview.proration.expect("a proration");
    assert_eq!(
        (proration.days_remaining, proration.days_in_period),
        (left as u32, days as u32)
    );
    assert_eq!(proration.charges_total_cents, prorate(10_000, left, days));

    let adjustment = preview.adjustment_invoice.expect("an adjustment invoice");
    let charged: Vec<_> = adjustment
        .invoice_lines
        .iter()
        .map(|l| (l.amount_subtotal, l.start_date, l.end_date))
        .collect();
    assert_eq!(
        charged,
        vec![(prorate(10_000, left, days), today, month_end)]
    );

    let next = preview.next_invoice.expect("a next invoice");
    let next_lines: Vec<_> = next
        .invoice_lines
        .iter()
        .map(|l| (l.name.as_str(), l.amount_subtotal, l.start_date))
        .collect();
    assert_eq!(next_lines, vec![("Add-on", 10_000, month_end)]);
}

/// Bought through the portal's checkout, the add-on is priced over its month and the subscription
/// renews monthly from then on.
#[rstest::rstest]
#[tokio::test]
async fn a_monthly_add_on_bought_through_checkout_switches_the_subscription_to_monthly(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.seed_payments().await;
    let today = chrono::Utc::now().date_naive();
    let (month_start, month_end) = month_of(today);
    let plan = annual_plan(&env).await;
    let add_on = monthly_add_on(&env).await;
    let sub_id = subscription()
        .plan_version(plan)
        .start_date(
            month_start
                .checked_sub_months(chrono::Months::new(5))
                .unwrap(),
        )
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let session = env
        .services()
        .create_addon_purchase_checkout_session(TENANT_ID, sub_id, add_on, plan, CUST_UBER_ID, None)
        .await
        .expect("checkout session")
        .id;

    let expected = prorate(
        10_000,
        (month_end - today).num_days(),
        (month_end - month_start).num_days(),
    );
    let result = env
        .services()
        .complete_checkout(
            TENANT_ID,
            session,
            CUST_UBER_PAYMENT_METHOD_ID,
            expected as u64,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("checkout");
    assert!(
        matches!(result, CheckoutCompletionResult::Completed { .. }),
        "{result:?}"
    );

    let adjustment = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .find(|i| i.invoice_type == InvoiceType::Adjustment)
        .expect("the add-on's invoice");
    assert_eq!(
        lines(&adjustment)
            .into_iter()
            .map(|(_, amount, start, end)| (amount, start, end))
            .collect::<Vec<_>>(),
        vec![(expected, today, month_end)]
    );

    let sub = env.get_subscription(sub_id).await;
    assert!(matches!(sub.period, DieselBillingPeriodEnum::Monthly));
    assert_eq!(
        (sub.current_period_start, sub.current_period_end),
        (month_start, Some(month_end))
    );
}
