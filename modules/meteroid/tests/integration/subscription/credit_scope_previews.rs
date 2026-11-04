//! Credit scope in previews: a customer's pending documents spend shared credit once, in billing
//! order; a subscription's own credit shows only on its own invoices.
use chrono::{Days, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, BillableMetricId, CreditGrantId, InvoiceId, SubscriptionId};
use meteroid_store::clients::usage::{GroupedUsageData, UsageData};
use meteroid_store::domain::credit::{GrantClassification, GrantScope};
use meteroid_store::domain::credit_outlook::{CreditGroupKind, UpcomingInvoiceForecast};
use meteroid_store::domain::enums::{BillingPeriodEnum, FeeTypeEnum};
use meteroid_store::domain::price_components::{
    PriceEntry, PriceInput, ProductRef, UsagePricingModel,
};
use meteroid_store::domain::prices::{FeeStructure, Pricing};
use meteroid_store::domain::subscription_components::{
    CreateSubscriptionComponents, ExtraComponent,
};
use meteroid_store::domain::{
    CreateSubscription, LineItem, SubscriptionActivationCondition, SubscriptionNew, UsagePeriod,
};
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::credit::customer_credit::DEFAULT_PRIORITY;
use meteroid_store::repositories::credit::customer_credit_wallets::FundCustomerCreditWallet;
use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::HashMap;

use crate::data::ids::{
    COMP_LEETCODE_RATE_ID, CUST_UBER_ID, METRIC_BANDWIDTH, METRIC_DATABASE_SIZE,
    PLAN_VERSION_1_LEETCODE_ID, TENANT_ID,
};
use crate::harness::{TestEnv, test_env};
use crate::helpers::credit::{fund, grant_remaining};
use crate::helpers::credit_recompute::{Fixture, fixture, metric_component};
use crate::meteroid_it::container::MeteroidSetup;
use crate::test_customer_credit::{draft, finalize, line};

/// Subscribes the customer from `start` to `components` only (the plan's own fee removed).
async fn subscribe(
    setup: &MeteroidSetup,
    start: NaiveDate,
    components: Vec<ExtraComponent>,
) -> SubscriptionId {
    setup
        .services
        .insert_subscription(
            Actor::System,
            CreateSubscription {
                subscription: SubscriptionNew {
                    customer_id: CUST_UBER_ID,
                    plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
                    net_terms: None,
                    invoice_memo: None,
                    invoice_threshold: None,
                    start_date: start,
                    end_date: None,
                    billing_start_date: Some(start),
                    activation_condition: SubscriptionActivationCondition::OnStart,
                    trial_duration: None,
                    billing_day_anchor: None,
                    payment_methods_config: None,
                    auto_advance_invoices: true,
                    charge_automatically: false,
                    purchase_order: None,
                    backdate_invoices: false,
                    skip_checkout_session: false,
                    skip_past_invoices: false,
                    usage_filter: None,
                    custom_properties: serde_json::Value::Null,
                },
                price_components: Some(CreateSubscriptionComponents {
                    parameterized_components: vec![],
                    overridden_components: vec![],
                    extra_components: components,
                    remove_components: vec![COMP_LEETCODE_RATE_ID],
                }),
                add_ons: None,
                coupons: None,
                entitlements: vec![],
                minimum_override: None,
                drop_minimum: false,
            },
            TENANT_ID,
        )
        .await
        .expect("a subscription")
        .id
}

/// Usage at €1.00 a unit.
fn usage(name: &str, metric_id: BillableMetricId) -> ExtraComponent {
    metric_component(
        name,
        metric_id,
        UsagePricingModel::PerUnit { rate: dec!(1) },
    )
}

fn fee(name: &str, cents: i64) -> ExtraComponent {
    ExtraComponent {
        name: name.to_owned(),
        product_ref: ProductRef::New {
            name: format!("{name} Product"),
            fee_type: FeeTypeEnum::Rate,
            fee_structure: FeeStructure::Rate {},
        },
        price_entry: PriceEntry::New(PriceInput {
            cadence: BillingPeriodEnum::Monthly,
            currency: "EUR".to_owned(),
            pricing: Pricing::Rate {
                rate: Decimal::new(cents, 2),
            },
            credit_unit: None,
        }),
    }
}

/// `units` of the metric in any window the metering is asked for.
fn used(f: &Fixture, metric_id: BillableMetricId, units: i64) {
    let at = NaiveDateTime::default();
    f.usage.set_usage_for_any_nonempty_window(
        metric_id,
        UsageData {
            data: vec![GroupedUsageData {
                value: Decimal::from(units),
                dimensions: HashMap::new(),
            }],
            period: UsagePeriod { start: at, end: at },
        },
    );
}

async fn grant(
    setup: &MeteroidSetup,
    scope: GrantScope,
    classification: GrantClassification,
    amount_minor: i64,
) -> CreditGrantId {
    let wallet = setup
        .store
        .create_customer_credit_wallet(TENANT_ID, CUST_UBER_ID, "EUR")
        .await
        .expect("the EUR wallet");
    let grant = setup
        .store
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
                reason: "Credit scope preview test".to_owned(),
                reference: None,
                actor_id: None,
                cost: None,
            },
            scope,
        )
        .await
        .expect("fund credit")
        .grant_id;
    crate::helpers::credit::effective_from_fixture_start(&setup.store, grant).await;
    grant
}

async fn preview(
    setup: &MeteroidSetup,
    subscription_id: SubscriptionId,
) -> UpcomingInvoiceForecast {
    let details = setup
        .store
        .get_subscription_details(TENANT_ID, subscription_id)
        .await
        .expect("subscription details");
    let forecast = setup
        .services
        .upcoming_invoice_forecast(&details)
        .await
        .expect("the upcoming invoice");
    let content = setup
        .services
        .compute_upcoming_invoice(&details)
        .await
        .expect("the upcoming invoice");
    assert_eq!(
        (content.applied_credits, content.promotional_credits),
        (
            forecast.content.applied_credits,
            forecast.content.promotional_credits
        ),
        "every next-invoice preview is the forecast"
    );
    forecast
}

/// Finalizes what subscribing drafted, so only the documents a test makes are pending.
async fn finalize_drafts(setup: &MeteroidSetup) {
    let mut conn = setup.store.get_conn().await.expect("connection");
    let drafts = diesel_models::invoices::InvoiceRow::list_customer_drafts(
        &mut conn,
        TENANT_ID,
        CUST_UBER_ID,
        100,
    )
    .await
    .expect("drafts");
    drop(conn);
    for draft in drafts.into_iter().filter(|draft| draft.total > 0) {
        finalize(setup, draft.id).await.expect("finalize a draft");
    }
}

fn of(subscription_id: SubscriptionId, line: LineItem) -> LineItem {
    LineItem {
        subscription_id: Some(subscription_id),
        ..line
    }
}

fn today() -> NaiveDate {
    Utc::now().date_naive()
}

/// T8: a 10,000 top-up; A bills first (on the 10th as of the 25th), B after; both used 7,000.
/// A's preview takes 7,000, B's the 3,000 left, and B names A as billed first. A draft of A billed
/// before both takes its 1,000 first.
#[tokio::test]
async fn previews_never_double_count() {
    let f = fixture().await;
    let a = subscribe(
        &f.setup,
        today() - Days::new(15),
        vec![usage("A usage", METRIC_BANDWIDTH)],
    )
    .await;
    let b = subscribe(
        &f.setup,
        today() - Days::new(5),
        vec![usage("B usage", METRIC_DATABASE_SIZE)],
    )
    .await;
    finalize_drafts(&f.setup).await;
    used(&f, METRIC_BANDWIDTH, 70);
    used(&f, METRIC_DATABASE_SIZE, 70);
    let top_up = fund(&f.setup.store, TENANT_ID, CUST_UBER_ID, None, 10_000).await;

    let first = preview(&f.setup, a).await;
    assert_eq!(first.content.total, 7_000);
    assert_eq!(first.content.applied_credits, 7_000);
    assert!(first.credit_estimate_shared);
    assert!(first.billed_first.is_empty());
    assert_eq!(first.credit.len(), 1);
    assert_eq!(first.credit[0].grant_id, top_up);
    assert_eq!(first.credit[0].group.kind, CreditGroupKind::AccountBalance);
    assert_eq!(first.credit[0].group.pays_for, "Pays any invoice");

    let second = preview(&f.setup, b).await;
    assert_eq!(second.content.applied_credits, 3_000);
    assert_eq!(second.content.amount_due, 4_000);
    assert_eq!(second.billed_first.len(), 1);
    assert_eq!(second.billed_first[0].subscription_id, Some(a));
    assert_eq!(second.billed_first[0].invoice_date, first.invoice_date);
    assert!(second.billed_first[0].invoice_id.is_none());
    assert_eq!(
        first.content.applied_credits + second.content.applied_credits,
        10_000,
        "together they spend the top-up once"
    );

    // A's earlier period, still a draft: it finalizes first.
    draft(&f.setup, vec![of(a, line("A earlier usage", 1_000, true))])
        .await
        .expect("A's draft");
    assert_eq!(preview(&f.setup, a).await.content.applied_credits, 7_000);
    let after_draft = preview(&f.setup, b).await;
    assert_eq!(after_draft.content.applied_credits, 2_000);
    assert!(
        after_draft
            .billed_first
            .iter()
            .any(|first| first.invoice_id.is_some())
    );
    assert_eq!(
        grant_remaining(&f.setup.store, TENANT_ID, CUST_UBER_ID, top_up).await,
        10_000,
        "previews draw nothing"
    );
}

/// T8, scoped: B's own 5,000 pack pays B's usage before tax, and A cannot use it.
#[tokio::test]
async fn a_subscription_pack_shows_only_on_its_own_preview() {
    let f = fixture().await;
    let a = subscribe(
        &f.setup,
        today() - Days::new(15),
        vec![usage("A usage", METRIC_BANDWIDTH)],
    )
    .await;
    let b = subscribe(
        &f.setup,
        today() - Days::new(5),
        vec![usage("B usage", METRIC_DATABASE_SIZE)],
    )
    .await;
    finalize_drafts(&f.setup).await;
    used(&f, METRIC_BANDWIDTH, 70);
    used(&f, METRIC_DATABASE_SIZE, 70);
    fund(&f.setup.store, TENANT_ID, CUST_UBER_ID, None, 10_000).await;
    let pack = grant(
        &f.setup,
        GrantScope::subscription_usage(b),
        GrantClassification::Promotional,
        5_000,
    )
    .await;

    let first = preview(&f.setup, a).await;
    assert_eq!(
        (
            first.content.promotional_credits,
            first.content.applied_credits
        ),
        (0, 7_000)
    );
    let second = preview(&f.setup, b).await;
    assert_eq!(
        (
            second.content.promotional_credits,
            second.content.applied_credits
        ),
        (5_000, 2_000)
    );
    let included = second
        .credit
        .iter()
        .find(|deduction| deduction.grant_id == pack)
        .expect("the pack pays B");
    assert!(included.before_tax);
    assert_eq!(included.group.kind, CreditGroupKind::Bonus);
    assert_eq!(included.group.subscription_id, Some(b));
    assert!(included.group.pays_for.starts_with("Pays "));
    assert!(included.group.pays_for.ends_with(" usage"));
}

/// T8, single subscription: estimated alone, as before, and never shared.
#[tokio::test]
async fn a_single_subscription_is_estimated_alone() {
    let f = fixture().await;
    let a = subscribe(
        &f.setup,
        today() - Days::new(15),
        vec![usage("A usage", METRIC_BANDWIDTH)],
    )
    .await;
    finalize_drafts(&f.setup).await;
    used(&f, METRIC_BANDWIDTH, 70);
    fund(&f.setup.store, TENANT_ID, CUST_UBER_ID, None, 10_000).await;
    let alone = preview(&f.setup, a).await;
    assert_eq!(alone.content.applied_credits, 7_000);
    assert!(!alone.credit_estimate_shared);
    assert!(alone.billed_first.is_empty());
}

/// T2 (previews): two subscriptions to the same plan (a 2,000 fee in advance, no usage) and a
/// 1,500 grant of S1 that pays any of S1's charges: only S1's next invoice uses it.
#[tokio::test]
async fn two_subscriptions_to_the_same_plan() {
    let f = fixture().await;
    let s1 = subscribe(&f.setup, today(), vec![fee("Team", 2_000)]).await;
    let s2 = subscribe(&f.setup, today(), vec![fee("Team", 2_000)]).await;
    finalize_drafts(&f.setup).await;
    grant(
        &f.setup,
        GrantScope {
            subscription_id: Some(s1),
            ..GrantScope::customer_wide()
        },
        GrantClassification::Paid,
        1_500,
    )
    .await;
    let first = preview(&f.setup, s1).await;
    assert_eq!(first.content.applied_credits, 1_500);
    assert!(
        !first.credit_estimate_shared,
        "S1's own credit is not shared"
    );
    assert!(first.credit[0].group.pays_for.ends_with(" charges"));
    let second = preview(&f.setup, s2).await;
    assert_eq!(second.content.applied_credits, 0);
    assert_eq!(second.content.amount_due, 2_000);
}

/// T1: A's 10,000 pack and a 5,000 top-up. B's 4,000 of usage is paid by the top-up alone; A's
/// 12,000 by its pack (before tax), then the 1,000 left of the top-up. When B used 15,000, B gets
/// all of the top-up and the pack is untouched.
#[tokio::test]
async fn two_subscriptions_share_a_wallet() {
    for b_usage in [4_000, 15_000] {
        let f = fixture().await;
        let a = subscribe(&f.setup, today(), vec![usage("A usage", METRIC_BANDWIDTH)]).await;
        let b = subscribe(
            &f.setup,
            today(),
            vec![usage("B usage", METRIC_DATABASE_SIZE)],
        )
        .await;
        let pack = grant(
            &f.setup,
            GrantScope::subscription_usage(a),
            GrantClassification::Promotional,
            10_000,
        )
        .await;
        let top_up = fund(&f.setup.store, TENANT_ID, CUST_UBER_ID, None, 5_000).await;
        let remaining = |id| grant_remaining(&f.setup.store, TENANT_ID, CUST_UBER_ID, id);

        let for_b = draft(&f.setup, vec![of(b, line("B usage", b_usage, true))])
            .await
            .expect("B's draft");
        let billed_b = finalize(&f.setup, for_b.id)
            .await
            .expect("B's invoice")
            .invoice;
        assert_eq!(billed_b.promotional_credits, 0);
        assert_eq!(billed_b.applied_credits, b_usage.min(5_000));
        assert_eq!(billed_b.amount_due, (b_usage - 5_000).max(0));
        assert_eq!(remaining(pack).await, 10_000, "A's pack never pays B");
        assert_eq!(remaining(top_up).await, (5_000 - b_usage).max(0));
        if b_usage > 5_000 {
            continue;
        }

        let for_a = draft(&f.setup, vec![of(a, line("A usage", 12_000, true))])
            .await
            .expect("A's draft");
        let billed_a = finalize(&f.setup, for_a.id)
            .await
            .expect("A's invoice")
            .invoice;
        assert_eq!(billed_a.promotional_credits, 10_000);
        assert_eq!(billed_a.applied_credits, 1_000);
        assert_eq!(billed_a.amount_due, 1_000);
        assert_eq!(remaining(pack).await, 0);
        assert_eq!(remaining(top_up).await, 0);
    }
}

/// Forecast #1: a draft its shared promotional credit pays in full (total 0) still draws that
/// credit when it finalizes, so the next invoice billed after it cannot count it again.
#[tokio::test]
async fn a_draft_paid_in_full_by_promotional_credit_still_draws_it() {
    use diesel_async::SimpleAsyncConnection;

    let f = fixture().await;
    let a = subscribe(
        &f.setup,
        today() - Days::new(15),
        vec![usage("A usage", METRIC_BANDWIDTH)],
    )
    .await;
    let b = subscribe(
        &f.setup,
        today() - Days::new(5),
        vec![usage("B usage", METRIC_DATABASE_SIZE)],
    )
    .await;
    finalize_drafts(&f.setup).await;
    used(&f, METRIC_DATABASE_SIZE, 10);
    grant(
        &f.setup,
        GrantScope::customer_wide(),
        GrantClassification::Promotional,
        1_000,
    )
    .await;
    let earlier = draft(&f.setup, vec![of(a, line("A earlier usage", 1_000, true))])
        .await
        .expect("A's draft");
    let mut conn = f.setup.store.get_conn().await.expect("connection");
    conn.batch_execute(&format!(
        "UPDATE invoice SET total = 0, amount_due = 0, promotional_credits = 1000 WHERE id = '{}'",
        earlier.id.as_uuid()
    ))
    .await
    .expect("the draft as its promotional credit left it");
    drop(conn);

    let next = preview(&f.setup, b).await;
    assert_eq!(next.content.total, 1_000);
    assert_eq!(
        (
            next.content.promotional_credits,
            next.content.applied_credits
        ),
        (0, 0),
        "the draft finalizing first takes the 1,000"
    );
    assert!(
        next.billed_first
            .iter()
            .any(|first| first.invoice_id == Some(earlier.id))
    );
}

/// Forecast #5 to #7: a plan change scheduled for the period end, with five seats chosen when it
/// was scheduled, previews exactly the invoice its execution bills (€29 platform fee and five
/// €10 seats, LeetCode's €35 fee gone), and the preview changes nothing.
#[rstest::rstest]
#[tokio::test]
async fn the_next_invoice_applies_a_scheduled_plan_change_as_executing_it_will(
    #[future] test_env: TestEnv,
) {
    use crate::data::ids::{COMP_STARTER_SEATS_ID, PLAN_VERSION_STARTER_ID};
    use meteroid_store::domain::credit_outlook::UpcomingChange;
    use meteroid_store::domain::subscription_components::{
        ComponentParameterization, ComponentParameters,
    };
    use meteroid_store::repositories::subscriptions::slots::SubscriptionSlotsInterfaceAuto;

    let env = test_env.await;
    let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let sub_id = crate::harness::subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(start)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.services()
        .schedule_plan_change(
            Actor::System,
            sub_id,
            TENANT_ID,
            PLAN_VERSION_STARTER_ID,
            vec![ComponentParameterization {
                component_id: COMP_STARTER_SEATS_ID,
                parameters: ComponentParameters {
                    initial_slot_count: Some(5),
                    billing_period: None,
                    committed_capacity: None,
                },
            }],
        )
        .await
        .expect("schedule the plan change");

    let details = env
        .store()
        .get_subscription_details(TENANT_ID, sub_id)
        .await
        .expect("details");
    let forecast = env
        .services()
        .upcoming_invoice_forecast(&details)
        .await
        .expect("the next invoice");
    assert_eq!(
        forecast.change,
        Some(UpcomingChange::PlanChange {
            plan_version_id: PLAN_VERSION_STARTER_ID
        })
    );
    assert_eq!(forecast.content.total, 7_900);

    let unchanged = env.get_subscription(sub_id).await;
    assert_eq!(unchanged.plan_version_id, PLAN_VERSION_1_LEETCODE_ID);
    let seats = env
        .store()
        .get_active_slots_value(
            TENANT_ID,
            sub_id,
            "Seats".to_string(),
            NaiveDate::from_ymd_opt(2024, 2, 1).map(|d| d.and_time(NaiveTime::MIN)),
        )
        .await
        .expect("seats");
    assert_eq!(seats, 0, "the preview seeds no seat");

    env.process_cycles().await;
    let billed = env.get_invoices(sub_id).await;
    assert_eq!(billed.len(), 2);
    assert_eq!(billed[1].total, forecast.content.total);
}

/// Forecast #3: a subscription's draft finalizes priced again from current usage, keeping its
/// stored fee lines. February's draft was stored before January's usage arrived (its €10 fee
/// alone); January's usage now prices at €30, so it takes 4,000 of the 5,000 shared credit and
/// the next invoice (€40: February's usage and March's fee) keeps 1,000. Finalizing both bills
/// exactly that.
#[tokio::test]
async fn a_draft_reserves_what_finalizing_it_will_bill_from_current_usage() {
    use diesel_async::SimpleAsyncConnection;
    use meteroid_store::clients::usage::MutableMockUsageClient;

    let usage = MutableMockUsageClient::new();
    let at = NaiveDateTime::default();
    usage.set_usage_for_any_nonempty_window(
        METRIC_BANDWIDTH,
        UsageData {
            data: vec![GroupedUsageData {
                value: Decimal::from(300),
                dimensions: HashMap::new(),
            }],
            period: UsagePeriod { start: at, end: at },
        },
    );
    let env = crate::harness::test_env_with_usage(std::sync::Arc::new(usage)).await;
    let sub_id = crate::harness::subscription()
        .plan_version(crate::data::ids::PLAN_VERSION_USAGE_ALPHA_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.process_cycles().await;
    let customer_id = env.get_subscription(sub_id).await.customer_id;
    let draft = env.get_invoices(sub_id).await[1].clone();
    assert_eq!(
        draft.billing_period_start,
        NaiveDate::from_ymd_opt(2024, 2, 1).unwrap()
    );
    let fees = draft
        .line_items
        .iter()
        .filter(|line| line.metric_id.is_none())
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        fees.iter().map(|line| line.amount_subtotal).sum::<i64>(),
        1_000,
        "February's €10 platform fee"
    );
    let stored = serde_json::to_string(&fees)
        .expect("the fee lines")
        .replace('\'', "''");
    let mut conn = env.store().get_conn().await.expect("connection");
    conn.batch_execute(&format!(
        "UPDATE invoice SET status = 'DRAFT', finalized_at = NULL, line_items = '{stored}'::jsonb,
         subtotal = 1000, subtotal_recurring = 1000, total = 1000, amount_due = 1000,
         tax_amount = 0 WHERE id = '{}'",
        draft.id.as_uuid()
    ))
    .await
    .expect("a stale draft");
    drop(conn);
    fund(env.store(), TENANT_ID, customer_id, None, 5_000).await;

    let details = env
        .store()
        .get_subscription_details(TENANT_ID, sub_id)
        .await
        .expect("details");
    let next = env
        .services()
        .upcoming_invoice_forecast(&details)
        .await
        .expect("the next invoice");
    assert_eq!(next.content.total, 4_000);
    assert_eq!(next.content.applied_credits, 1_000);
    assert_eq!(next.billed_first[0].invoice_id, Some(draft.id));

    let finalized = env
        .services()
        .finalize_invoice(Actor::System, draft.id, TENANT_ID)
        .await
        .expect("February's invoice")
        .invoice;
    assert_eq!(finalized.total, 4_000);
    assert_eq!(finalized.applied_credits, 4_000);
    env.process_cycles().await;
    let march = env.get_invoices(sub_id).await[2].clone();
    assert_eq!(
        march.billing_period_start,
        NaiveDate::from_ymd_opt(2024, 3, 1).unwrap()
    );
    assert_ne!(
        march.status,
        meteroid_store::domain::enums::InvoiceStatusEnum::Draft,
        "billed, not estimated"
    );
    assert_eq!(march.total, next.content.total);
    assert_eq!(march.applied_credits, next.content.applied_credits);
}

/// `count` drafts of the customer, `cents` each, in the order they finalize.
async fn drafts(setup: &MeteroidSetup, count: usize, cents: i64) -> Vec<InvoiceId> {
    let mut ids = Vec::with_capacity(count);
    for n in 0..count {
        ids.push(
            draft(setup, vec![line(&format!("Draft {n}"), cents, false)])
                .await
                .expect("a draft")
                .id,
        );
    }
    ids
}

async fn execute(setup: &MeteroidSetup, sql: String) {
    use diesel_async::SimpleAsyncConnection;
    setup
        .store
        .get_conn()
        .await
        .expect("connection")
        .batch_execute(&sql)
        .await
        .expect("sql");
}

/// Truncation is never shown as complete. S's €20 fee draws on 10,000 of shared EUR credit; the
/// customer's drafts are in USD, so none draws it. With 50 drafts every document is read: S's
/// credit is exact. A 51st is past the forecast's bound: S's preview and the portal mark its
/// credit an estimate.
#[tokio::test]
async fn documents_past_a_forecast_bound_mark_it_an_estimate() {
    let f = fixture().await;
    let s = subscribe(&f.setup, today(), vec![fee("Team", 2_000)]).await;
    finalize_drafts(&f.setup).await;
    fund(&f.setup.store, TENANT_ID, CUST_UBER_ID, None, 10_000).await;
    let dollars = |ids: &[InvoiceId]| {
        format!(
            "UPDATE invoice SET currency = 'USD' WHERE id IN ({})",
            ids.iter()
                .map(|id| format!("'{}'", id.as_uuid()))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let first = drafts(&f.setup, 50, 100).await;
    execute(&f.setup, dollars(&first)).await;

    let read = || async {
        let preview = preview(&f.setup, s).await;
        let portal = f
            .setup
            .services
            .portal_credit_overview(TENANT_ID, CUST_UBER_ID)
            .await
            .expect("the portal overview");
        let upcoming = portal
            .upcoming
            .iter()
            .find(|invoice| invoice.details.subscription.id == s)
            .expect("S's next invoice")
            .clone();
        (preview, upcoming)
    };
    let (exact, portal) = read().await;
    assert_eq!(exact.content.applied_credits, 2_000);
    assert!(!exact.credit_estimate_shared);
    assert_eq!(portal.content.applied_credits, 2_000);
    assert!(!portal.credit_estimate_shared);

    let last = drafts(&f.setup, 1, 100).await;
    execute(&f.setup, dollars(&last)).await;
    let (estimated, portal) = read().await;
    assert_eq!(estimated.content.applied_credits, 2_000);
    assert!(estimated.credit_estimate_shared);
    assert!(portal.credit_estimate_shared);
}

/// A draft's estimate spends what every draft finalizing before it draws, however many: 51
/// drafts of €1.00 against €50.00 of credit. The 50th keeps the last €1.00; the 51st, once left
/// out of the drafts read and estimated on the whole ledger (€1.00), finds none.
#[tokio::test]
async fn a_draft_past_the_bound_still_counts_every_earlier_draft() {
    let f = fixture().await;
    fund(&f.setup.store, TENANT_ID, CUST_UBER_ID, None, 5_000).await;
    let ids = drafts(&f.setup, 51, 100).await;
    let refreshed = |id: InvoiceId| {
        let setup = &f.setup;
        async move {
            // Edited by hand: refreshed on its stored lines, which name no subscription.
            execute(
                setup,
                format!(
                    "UPDATE invoice SET manually_edited_at = now() WHERE id = '{}'",
                    id.as_uuid()
                ),
            )
            .await;
            setup
                .services
                .refresh_invoice_data(id, TENANT_ID)
                .await
                .expect("refresh")
                .invoice
                .applied_credits
        }
    };
    assert_eq!(refreshed(ids[49]).await, 100);
    assert_eq!(refreshed(ids[50]).await, 0);
}

/// Without credit left, the drafts finalizing before a draft are not estimated again: its
/// estimate is the one the whole ledger gives, with no credit or with credit spent to zero.
#[tokio::test]
async fn a_draft_with_no_credit_left_refreshes_as_the_ledger_stands() {
    let f = fixture().await;
    let refreshed = |id: InvoiceId| {
        let setup = &f.setup;
        async move {
            // Edited by hand: refreshed on its stored lines, which name no subscription.
            execute(
                setup,
                format!(
                    "UPDATE invoice SET manually_edited_at = now() WHERE id = '{}'",
                    id.as_uuid()
                ),
            )
            .await;
            let invoice = setup
                .services
                .refresh_invoice_data(id, TENANT_ID)
                .await
                .expect("refresh")
                .invoice;
            (invoice.applied_credits, invoice.amount_due, invoice.total)
        }
    };
    let ids = drafts(&f.setup, 3, 100).await;
    assert_eq!(refreshed(ids[2]).await, (0, 100, 100), "no credit at all");

    fund(&f.setup.store, TENANT_ID, CUST_UBER_ID, None, 200).await;
    assert_eq!(
        refreshed(ids[2]).await,
        (0, 100, 100),
        "the two drafts before it take it all"
    );
    for id in &ids[..2] {
        finalize(&f.setup, *id).await.expect("finalize");
    }
    assert_eq!(
        refreshed(ids[2]).await,
        (0, 100, 100),
        "credit spent to zero"
    );
}

/// Credit not effective yet is unspent, so the drafts before a draft are estimated; neither draws
/// it, and both keep their exact amounts.
#[tokio::test]
async fn drafts_draw_nothing_from_credit_not_effective_yet() {
    let f = fixture().await;
    let grant = crate::helpers::credit::fund_promotional_grant(
        &f.setup.store,
        TENANT_ID,
        CUST_UBER_ID,
        None,
        crate::helpers::credit::Grant::invoice(500),
    )
    .await;
    execute(
        &f.setup,
        format!(
            "SET session_replication_role = replica;
             UPDATE customer_credit_grant SET effective_at = now() + interval '30 days'
             WHERE id = '{}';
             SET session_replication_role = origin;",
            grant.as_uuid()
        ),
    )
    .await;
    for id in drafts(&f.setup, 2, 100).await {
        execute(
            &f.setup,
            format!(
                "UPDATE invoice SET manually_edited_at = now() WHERE id = '{}'",
                id.as_uuid()
            ),
        )
        .await;
        let invoice = f
            .setup
            .services
            .refresh_invoice_data(id, TENANT_ID)
            .await
            .expect("refresh")
            .invoice;
        assert_eq!(
            (
                invoice.promotional_credits,
                invoice.applied_credits,
                invoice.total,
                invoice.amount_due
            ),
            (0, 0, 100, 100)
        );
    }
    assert_eq!(
        grant_remaining(&f.setup.store, TENANT_ID, CUST_UBER_ID, grant).await,
        500
    );
}

/// A usage client whose first read once armed waits to be released.
struct BlockingUsage {
    inner: meteroid_store::clients::usage::MutableMockUsageClient,
    armed: std::sync::atomic::AtomicBool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

#[async_trait::async_trait]
impl meteroid_store::clients::usage::UsageClient for BlockingUsage {
    async fn fetch_usage(
        &self,
        tenant_id: &common_domain::ids::TenantId,
        customer_id: &common_domain::ids::CustomerId,
        metric: &meteroid_store::domain::BillableMetric,
        period: UsagePeriod,
    ) -> meteroid_store::StoreResult<UsageData> {
        if self.armed.swap(false, std::sync::atomic::Ordering::SeqCst) {
            self.entered.notify_one();
            self.release.notified().await;
        }
        self.inner
            .fetch_usage(tenant_id, customer_id, metric, period)
            .await
    }

    async fn fetch_usage_grouped_by(
        &self,
        tenant_id: &common_domain::ids::TenantId,
        customer_id: &common_domain::ids::CustomerId,
        metric: &meteroid_store::domain::BillableMetric,
        group_by: &str,
        period: UsagePeriod,
    ) -> meteroid_store::StoreResult<UsageData> {
        self.inner
            .fetch_usage_grouped_by(tenant_id, customer_id, metric, group_by, period)
            .await
    }

    async fn fetch_total_usage(
        &self,
        tenant_id: &common_domain::ids::TenantId,
        customer_id: &common_domain::ids::CustomerId,
        metric: &meteroid_store::domain::BillableMetric,
        period: UsagePeriod,
    ) -> meteroid_store::StoreResult<Decimal> {
        self.inner
            .fetch_total_usage(tenant_id, customer_id, metric, period)
            .await
    }

    async fn fetch_windowed_usage(
        &self,
        tenant_id: &common_domain::ids::TenantId,
        customer_id: &common_domain::ids::CustomerId,
        metric: &meteroid_store::domain::BillableMetric,
        period: UsagePeriod,
    ) -> meteroid_store::StoreResult<meteroid_store::clients::usage::WindowedUsageData> {
        self.inner
            .fetch_windowed_usage(tenant_id, customer_id, metric, period)
            .await
    }

    async fn fetch_usage_summary(
        &self,
        tenant_id: &common_domain::ids::TenantId,
        customer_id: Option<&common_domain::ids::CustomerId>,
        metric: &meteroid_store::domain::BillableMetric,
        period: UsagePeriod,
    ) -> meteroid_store::StoreResult<UsageData> {
        self.inner
            .fetch_usage_summary(tenant_id, customer_id, metric, period)
            .await
    }

    async fn search_events(
        &self,
        tenant_id: &common_domain::ids::TenantId,
        options: meteroid_store::clients::usage::EventSearchOptions,
    ) -> meteroid_store::StoreResult<meteroid_store::clients::usage::EventSearchResult> {
        self.inner.search_events(tenant_id, options).await
    }

    async fn ingest_events(
        &self,
        tenant_id: &common_domain::ids::TenantId,
        request: meteroid_store::clients::usage::IngestEventsRequest,
    ) -> meteroid_store::StoreResult<meteroid_store::clients::usage::IngestEventsResult> {
        self.inner.ingest_events(tenant_id, request).await
    }
}

/// A preview of a scheduled plan change never holds the subscription while it reads metering:
/// while its read waits, a period transition can lock the subscription at once. The preview then
/// bills what executing the change bills: January's 100 API calls at €0.10 and Usage Beta's €20
/// platform fee.
#[tokio::test]
async fn a_previewed_change_reads_metering_without_holding_its_subscription() {
    use crate::data::ids::{PLAN_VERSION_USAGE_ALPHA_ID, PLAN_VERSION_USAGE_BETA_ID};
    use diesel_async::SimpleAsyncConnection;
    use meteroid_store::domain::credit_outlook::UpcomingChange;

    let inner = meteroid_store::clients::usage::MutableMockUsageClient::new();
    let at = NaiveDateTime::default();
    inner.set_usage_for_any_nonempty_window(
        METRIC_BANDWIDTH,
        UsageData {
            data: vec![GroupedUsageData {
                value: Decimal::from(100),
                dimensions: HashMap::new(),
            }],
            period: UsagePeriod { start: at, end: at },
        },
    );
    let usage = std::sync::Arc::new(BlockingUsage {
        inner,
        armed: std::sync::atomic::AtomicBool::new(false),
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let env = crate::harness::test_env_with_usage(usage.clone()).await;
    let sub_id = crate::harness::subscription()
        .plan_version(PLAN_VERSION_USAGE_ALPHA_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let customer_id = env.get_subscription(sub_id).await.customer_id;
    let pending = {
        let mut conn = env.store().get_conn().await.expect("connection");
        diesel_models::invoices::InvoiceRow::list_customer_drafts(
            &mut conn,
            TENANT_ID,
            customer_id,
            100,
        )
        .await
        .expect("drafts")
    };
    // Only the previewed change reads metering.
    for draft in pending {
        env.services()
            .finalize_invoice(Actor::System, draft.id, TENANT_ID)
            .await
            .expect("finalize a draft");
    }
    env.services()
        .schedule_plan_change(
            Actor::System,
            sub_id,
            TENANT_ID,
            PLAN_VERSION_USAGE_BETA_ID,
            vec![],
        )
        .await
        .expect("schedule the plan change");
    let details = env
        .store()
        .get_subscription_details(TENANT_ID, sub_id)
        .await
        .expect("details");

    usage.armed.store(true, std::sync::atomic::Ordering::SeqCst);
    let services = env.services().clone();
    let previewing =
        tokio::spawn(async move { services.upcoming_invoice_forecast(&details).await });
    tokio::time::timeout(std::time::Duration::from_secs(60), usage.entered.notified())
        .await
        .expect("the preview reads metering");
    let mut conn = env.store().get_conn().await.expect("connection");
    conn.batch_execute(&format!(
        "BEGIN; SELECT id FROM subscription WHERE id = '{}' FOR UPDATE NOWAIT; COMMIT;",
        sub_id.as_uuid()
    ))
    .await
    .expect("the subscription is free while the preview reads metering");
    drop(conn);
    usage.release.notify_one();
    let forecast = previewing
        .await
        .expect("the preview task")
        .expect("the preview");
    assert_eq!(
        forecast.change,
        Some(UpcomingChange::PlanChange {
            plan_version_id: PLAN_VERSION_USAGE_BETA_ID
        })
    );
    assert_eq!(forecast.content.total, 1_000 + 2_000);

    env.process_cycles().await;
    let billed = env.get_invoices(sub_id).await;
    assert_eq!(billed.len(), 2);
    assert_eq!(billed[1].total, forecast.content.total);
}
