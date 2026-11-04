//! Integration tests for plan-level minimum commitment (true-up floor).

use crate::data::ids::*;
use crate::harness::{TestEnv, subscription, test_env, test_env_with_seed_and_usage};
use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use chrono::NaiveDate;
use diesel_async::RunQueryDsl;
use diesel_models::plan_version_adjustments::PlanVersionAdjustmentRowNew;
use meteroid_store::clients::usage::MockUsageClient;
use meteroid_store::domain::coupons::{CouponDiscount, CouponNew};
use meteroid_store::domain::enums::InvoiceType;
use meteroid_store::domain::{
    CreateSubscription, CreateSubscriptionCoupon, CreateSubscriptionCoupons,
    SubscriptionActivationCondition, SubscriptionNew,
};
use meteroid_store::repositories::coupons::CouponInterface;
use meteroid_store::repositories::{PlansInterface, SubscriptionInterface};
use meteroid_testkit::mailer::MockMailerService;
use rstest::rstest;
use rust_decimal::Decimal;
use std::sync::Arc;

// Extra imports used by the tax / proration / trial regression tests below.
use common_domain::country::CountryCode;
use common_domain::ids::{BaseId, CustomerId};
use meteroid_store::domain::enums::TaxResolverEnum;
use meteroid_store::domain::plan_minimums::MinimumScope;
use meteroid_store::domain::{
    Address, InvoicingEntityPatch, LineItemKind, SubscriptionMinimum, SubscriptionStatusEnum,
};
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;
use serde_json::json;
use std::str::FromStr;

/// The Feb 1 renewal trues up January: the $35.00 LeetCode fee billed on January's invoice falls
/// $15.00 short of the $50.00 floor, billed as a line dated over January.
#[tokio::test]
async fn test_minimum_shortfall_full_period() {
    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;

    let mock_mailer = Arc::new(MockMailerService::new());
    let setup = meteroid_it::container::start_meteroid_with_clients(
        postgres_connection_string,
        SeedLevel::PLANS,
        Arc::new(MockUsageClient::noop()),
        mock_mailer.clone(),
    )
    .await;

    let services = setup.services.clone();
    let store = setup.store.clone();
    let mut conn = store.pool.get().await.unwrap();

    diesel::insert_into(diesel_models::schema::plan_version_adjustment::table)
        .values(&PlanVersionAdjustmentRowNew {
            kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
            plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
            amount_subunit: 5_000, // $50.00
            includes_all_components: true,
        })
        .execute(&mut conn)
        .await
        .unwrap();

    let subscription_id = services
        .insert_subscription(
            common_domain::actor::Actor::System,
            CreateSubscription {
                subscription: SubscriptionNew {
                    customer_id: CUST_UBER_ID,
                    plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
                    net_terms: None,
                    invoice_memo: None,
                    invoice_threshold: None,
                    start_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                    end_date: None,
                    billing_start_date: Some(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()),
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
                price_components: None,
                add_ons: None,
                coupons: None,
                entitlements: vec![],
                minimum_override: None,
                drop_minimum: false,
            },
            TENANT_ID,
        )
        .await
        .unwrap()
        .id;

    let mut sub_details = store
        .get_subscription_details_with_conn(&mut conn, TENANT_ID, subscription_id)
        .await
        .unwrap();
    // The renewal closing January, as the lifecycle bills it.
    sub_details.subscription.cycle_index = Some(1);
    let minimum = sub_details
        .minimum
        .as_ref()
        .expect("subscription carries the snapshot");
    assert_eq!(minimum.amount_subunit, 5_000);
    assert!(!minimum.is_override, "plan-snapshot, not an override");

    let invoice_date = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    let result = services
        .compute_invoice(&invoice_date, &sub_details, None, InvoiceType::Recurring)
        .await
        .unwrap();

    assert_eq!(
        result.subtotal, 5_000,
        "subtotal equals the floor in subunits"
    );

    let regular: Vec<_> = result
        .invoice_lines
        .iter()
        .filter(|l| l.kind == meteroid_store::domain::LineItemKind::Regular)
        .collect();
    let shortfall: Vec<_> = result
        .invoice_lines
        .iter()
        .filter(|l| l.kind == meteroid_store::domain::LineItemKind::MinimumShortfall)
        .collect();

    assert_eq!(regular.len(), 1, "one recurring line");
    assert_eq!(regular[0].amount_subtotal, 3_500, "LeetCode @ $35.00");
    assert_eq!(shortfall.len(), 1, "one shortfall line");
    assert_eq!(shortfall[0].amount_subtotal, 1_500, "$50 - $35 = $15");
    assert_eq!(
        (shortfall[0].start_date, shortfall[0].end_date),
        (
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 2, 1).unwrap()
        ),
        "dated over the period it trues up"
    );
}

/// Floor skipped on OneOff invoices — they're not tied to a billing period.
#[tokio::test]
async fn test_minimum_skipped_on_oneoff_invoice() {
    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;

    let mock_mailer = Arc::new(MockMailerService::new());
    let setup = meteroid_it::container::start_meteroid_with_clients(
        postgres_connection_string,
        SeedLevel::PLANS,
        Arc::new(MockUsageClient::noop()),
        mock_mailer.clone(),
    )
    .await;

    let services = setup.services.clone();
    let store = setup.store.clone();
    let mut conn = store.pool.get().await.unwrap();

    diesel::insert_into(diesel_models::schema::plan_version_adjustment::table)
        .values(&PlanVersionAdjustmentRowNew {
            kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
            plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
            amount_subunit: 5_000,
            includes_all_components: true,
        })
        .execute(&mut conn)
        .await
        .unwrap();

    let subscription_id = services
        .insert_subscription(
            common_domain::actor::Actor::System,
            CreateSubscription {
                subscription: SubscriptionNew {
                    customer_id: CUST_UBER_ID,
                    plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
                    net_terms: None,
                    invoice_memo: None,
                    invoice_threshold: None,
                    start_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                    end_date: None,
                    billing_start_date: Some(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()),
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
                price_components: None,
                add_ons: None,
                coupons: None,
                entitlements: vec![],
                minimum_override: None,
                drop_minimum: false,
            },
            TENANT_ID,
        )
        .await
        .unwrap()
        .id;

    let sub_details = store
        .get_subscription_details_with_conn(&mut conn, TENANT_ID, subscription_id)
        .await
        .unwrap();

    let invoice_date = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    let result = services
        .compute_invoice(&invoice_date, &sub_details, None, InvoiceType::OneOff)
        .await
        .unwrap();

    let shortfall: Vec<_> = result
        .invoice_lines
        .iter()
        .filter(|l| l.kind == meteroid_store::domain::LineItemKind::MinimumShortfall)
        .collect();
    assert!(shortfall.is_empty(), "no shortfall on OneOff invoices");
}

/// No true-up when January's spend ($35 fee) meets the $10 floor.
#[tokio::test]
async fn test_minimum_not_applied_when_usage_meets_floor() {
    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;

    let mock_mailer = Arc::new(MockMailerService::new());
    let setup = meteroid_it::container::start_meteroid_with_clients(
        postgres_connection_string,
        SeedLevel::PLANS,
        Arc::new(MockUsageClient::noop()),
        mock_mailer.clone(),
    )
    .await;

    let services = setup.services.clone();
    let store = setup.store.clone();
    let mut conn = store.pool.get().await.unwrap();

    diesel::insert_into(diesel_models::schema::plan_version_adjustment::table)
        .values(&PlanVersionAdjustmentRowNew {
            kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
            plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
            amount_subunit: 1_000, // $10.00 — below the $35 recurring
            includes_all_components: true,
        })
        .execute(&mut conn)
        .await
        .unwrap();

    let subscription_id = services
        .insert_subscription(
            common_domain::actor::Actor::System,
            CreateSubscription {
                subscription: SubscriptionNew {
                    customer_id: CUST_UBER_ID,
                    plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
                    net_terms: None,
                    invoice_memo: None,
                    invoice_threshold: None,
                    start_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                    end_date: None,
                    billing_start_date: Some(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()),
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
                price_components: None,
                add_ons: None,
                coupons: None,
                entitlements: vec![],
                minimum_override: None,
                drop_minimum: false,
            },
            TENANT_ID,
        )
        .await
        .unwrap()
        .id;

    let mut sub_details = store
        .get_subscription_details_with_conn(&mut conn, TENANT_ID, subscription_id)
        .await
        .unwrap();
    // The renewal closing January, as the lifecycle bills it.
    sub_details.subscription.cycle_index = Some(1);

    let invoice_date = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    let result = services
        .compute_invoice(&invoice_date, &sub_details, None, InvoiceType::Recurring)
        .await
        .unwrap();

    let shortfall: Vec<_> = result
        .invoice_lines
        .iter()
        .filter(|l| l.kind == meteroid_store::domain::LineItemKind::MinimumShortfall)
        .collect();
    assert!(shortfall.is_empty());
    assert_eq!(result.subtotal, 3_500);
}

/// Plan-version edits don't retroactively change a live subscription's snapshot.
#[tokio::test]
async fn test_plan_edit_does_not_change_existing_subscription_minimum() {
    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;

    let mock_mailer = Arc::new(MockMailerService::new());
    let setup = meteroid_it::container::start_meteroid_with_clients(
        postgres_connection_string,
        SeedLevel::PLANS,
        Arc::new(MockUsageClient::noop()),
        mock_mailer.clone(),
    )
    .await;

    let services = setup.services.clone();
    let store = setup.store.clone();
    let mut conn = store.pool.get().await.unwrap();

    diesel::insert_into(diesel_models::schema::plan_version_adjustment::table)
        .values(&PlanVersionAdjustmentRowNew {
            kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
            plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
            amount_subunit: 5_000,
            includes_all_components: true,
        })
        .execute(&mut conn)
        .await
        .unwrap();

    let subscription_id = services
        .insert_subscription(
            common_domain::actor::Actor::System,
            CreateSubscription {
                subscription: SubscriptionNew {
                    customer_id: CUST_UBER_ID,
                    plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
                    net_terms: None,
                    invoice_memo: None,
                    invoice_threshold: None,
                    start_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                    end_date: None,
                    billing_start_date: Some(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()),
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
                price_components: None,
                add_ons: None,
                coupons: None,
                entitlements: vec![],
                minimum_override: None,
                drop_minimum: false,
            },
            TENANT_ID,
        )
        .await
        .unwrap()
        .id;

    // Mutate the plan AFTER subscription creation.
    use diesel::ExpressionMethods;
    use diesel_models::schema::plan_version_adjustment::dsl as pvm;
    diesel::update(pvm::plan_version_adjustment)
        .set(pvm::amount_subunit.eq(10_000_i64))
        .execute(&mut conn)
        .await
        .unwrap();

    let sub_details = store
        .get_subscription_details_with_conn(&mut conn, TENANT_ID, subscription_id)
        .await
        .unwrap();

    let snap = sub_details
        .minimum
        .as_ref()
        .expect("subscription minimum snapshot present");
    assert_eq!(snap.amount_subunit, 5_000, "snapshot retained");
}

/// Plan-change refreshes the snapshot from the new plan version when `is_override = false`.
/// Setup: Starter plan with $50 floor, Pro plan with $80 floor. Change Starter→Pro at January's
/// end: the snapshot becomes Pro's $80, while the Feb 1 renewal trues up January under Starter's
/// $50 it ran on (50 − 29 − 10 = 11), dated over January.
#[rstest]
#[tokio::test]
async fn test_plan_change_refreshes_non_override_snapshot(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let mut conn = env.conn().await;

    // Seed minimums on both plans.
    diesel::insert_into(diesel_models::schema::plan_version_adjustment::table)
        .values(vec![
            PlanVersionAdjustmentRowNew {
                kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
                plan_version_id: PLAN_VERSION_STARTER_ID,
                amount_subunit: 5_000,
                includes_all_components: true,
            },
            PlanVersionAdjustmentRowNew {
                kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
                plan_version_id: PLAN_VERSION_PRO_ID,
                amount_subunit: 8_000,
                includes_all_components: true,
            },
        ])
        .execute(&mut conn)
        .await
        .unwrap();

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let change_date = NaiveDate::from_ymd_opt(2024, 1, 16).unwrap();
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let sub_details = env
        .store()
        .get_subscription_details_with_conn(&mut conn, TENANT_ID, sub_id)
        .await
        .unwrap();
    assert_eq!(
        sub_details.minimum.as_ref().unwrap().amount_subunit,
        5_000,
        "initial snapshot from Starter"
    );
    assert!(!sub_details.minimum.as_ref().unwrap().is_override);

    // A minimum commitment holds a plan change to the period's end.
    let refused = env
        .services()
        .apply_plan_change_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            PLAN_VERSION_PRO_ID,
            vec![],
            change_date,
        )
        .await
        .expect_err("immediate change refused");
    assert!(format!("{refused:?}").contains("minimum commitment"));
    env.services()
        .schedule_plan_change(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            PLAN_VERSION_PRO_ID,
            vec![],
        )
        .await
        .expect("plan change scheduled");
    env.process_cycles().await;

    let sub_details = env
        .store()
        .get_subscription_details_with_conn(&mut conn, TENANT_ID, sub_id)
        .await
        .unwrap();
    let min = sub_details.minimum.as_ref().expect("refreshed snapshot");
    assert_eq!(min.amount_subunit, 8_000, "snapshot refreshed from Pro");
    assert!(!min.is_override, "still a plan-snapshot, not an override");

    let feb1 = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    let renewal = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .find(|i| i.invoice_type == InvoiceType::Recurring && i.billing_period_start == feb1)
        .expect("the renewal at the change");
    let true_up: Vec<_> = renewal
        .line_items
        .iter()
        .filter(|l| l.kind == LineItemKind::MinimumShortfall)
        .map(|l| (l.amount_subtotal, l.start_date, l.end_date))
        .collect();
    assert_eq!(true_up, vec![(5_000 - 2_900 - 1_000, start_date, feb1)]);
}

/// A plan change books MRR floored by the minimum it snapshots, as creation does: Pro's $10 000
/// floor is the MRR, whatever Pro's rate. The minimum holds the change to the period's end.
#[rstest]
#[tokio::test]
async fn test_scheduled_plan_change_floors_mrr(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let mut conn = env.conn().await;

    diesel::insert_into(diesel_models::schema::plan_version_adjustment::table)
        .values(&PlanVersionAdjustmentRowNew {
            kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
            plan_version_id: PLAN_VERSION_PRO_ID,
            amount_subunit: 1_000_000,
            includes_all_components: true,
        })
        .execute(&mut conn)
        .await
        .unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // A minimum commitment holds a plan change to the period's end.
    let refused = env
        .services()
        .apply_plan_change_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            PLAN_VERSION_PRO_ID,
            vec![],
            NaiveDate::from_ymd_opt(2024, 1, 16).unwrap(),
        )
        .await
        .expect_err("immediate change refused");
    assert!(format!("{refused:?}").contains("minimum commitment"));
    env.services()
        .schedule_plan_change(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            PLAN_VERSION_PRO_ID,
            vec![],
        )
        .await
        .expect("plan change scheduled");
    env.process_cycles().await;

    let sub_details = env
        .store()
        .get_subscription_details_with_conn(&mut conn, TENANT_ID, sub_id)
        .await
        .unwrap();
    assert_eq!(sub_details.subscription.mrr_cents, 1_000_000);
}

/// Plan-change preserves operator overrides. Setup: Starter plan with $50 floor, operator
/// sets a $200 override on the sub, change Starter→Pro: override survives unchanged.
#[rstest]
#[tokio::test]
async fn test_plan_change_preserves_override(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let mut conn = env.conn().await;

    diesel::insert_into(diesel_models::schema::plan_version_adjustment::table)
        .values(vec![
            PlanVersionAdjustmentRowNew {
                kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
                plan_version_id: PLAN_VERSION_STARTER_ID,
                amount_subunit: 5_000,
                includes_all_components: true,
            },
            PlanVersionAdjustmentRowNew {
                kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
                plan_version_id: PLAN_VERSION_PRO_ID,
                amount_subunit: 8_000,
                includes_all_components: true,
            },
        ])
        .execute(&mut conn)
        .await
        .unwrap();

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let change_date = NaiveDate::from_ymd_opt(2024, 1, 16).unwrap();
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Operator override: $200, all components.
    env.services()
        .set_subscription_minimum(
            TENANT_ID,
            sub_id,
            20_000,
            meteroid_store::domain::plan_minimums::MinimumScope::AllComponents,
        )
        .await
        .unwrap();

    // A minimum commitment holds a plan change to the period's end.
    let refused = env
        .services()
        .apply_plan_change_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            PLAN_VERSION_PRO_ID,
            vec![],
            change_date,
        )
        .await
        .expect_err("immediate change refused");
    assert!(format!("{refused:?}").contains("minimum commitment"));
    env.services()
        .schedule_plan_change(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            PLAN_VERSION_PRO_ID,
            vec![],
        )
        .await
        .expect("plan change scheduled");
    env.process_cycles().await;

    let sub_details = env
        .store()
        .get_subscription_details_with_conn(&mut conn, TENANT_ID, sub_id)
        .await
        .unwrap();
    let min = sub_details.minimum.as_ref().expect("override preserved");
    assert_eq!(
        min.amount_subunit, 20_000,
        "override survives plan change unchanged"
    );
    assert!(min.is_override);
}

/// `copy_plan_version_to_draft` must carry the minimum forward into the new draft.
#[tokio::test]
async fn test_copy_plan_version_to_draft_carries_minimum() {
    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;

    let mock_mailer = Arc::new(MockMailerService::new());
    let setup = meteroid_it::container::start_meteroid_with_clients(
        postgres_connection_string,
        SeedLevel::PLANS,
        Arc::new(MockUsageClient::noop()),
        mock_mailer.clone(),
    )
    .await;

    let store = setup.store.clone();
    let mut conn = store.pool.get().await.unwrap();

    diesel::insert_into(diesel_models::schema::plan_version_adjustment::table)
        .values(&PlanVersionAdjustmentRowNew {
            kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
            plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
            amount_subunit: 4_200,
            includes_all_components: true,
        })
        .execute(&mut conn)
        .await
        .unwrap();

    let draft = store
        .copy_plan_version_to_draft(PLAN_VERSION_1_LEETCODE_ID, TENANT_ID)
        .await
        .unwrap();

    let draft_min = store
        .get_plan_minimum(draft.id, TENANT_ID)
        .await
        .unwrap()
        .expect("draft inherits the minimum");
    assert_eq!(draft_min.amount_subunit, 4_200);
    assert!(matches!(
        draft_min.scope,
        meteroid_store::domain::plan_minimums::MinimumScope::AllComponents
    ));
}

/// The floor is measured on GROSS (pre-discount) component spend, not on the post-discount
/// amount, and coupons then apply to everything the customer owes, the floor top-up included.
///
/// LeetCode = $35.00 recurring, floor = $50.00 (5000 subunit), with a 100% coupon, at the Feb 1
/// renewal closing January:
///   - gross counted toward the floor = 3_500 (January's $35 line, `amount_subtotal`)
///   - shortfall = 5000 - 3500 = 1_500   (NOT 5000, which a post-discount floor would charge)
///   - the 100% coupon discounts February's fee and the shortfall to a taxable amount of 0
#[tokio::test]
async fn test_minimum_floor_measured_on_gross_spend() {
    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;

    let mock_mailer = Arc::new(MockMailerService::new());
    let setup = meteroid_it::container::start_meteroid_with_clients(
        postgres_connection_string,
        SeedLevel::PLANS,
        Arc::new(MockUsageClient::noop()),
        mock_mailer.clone(),
    )
    .await;

    let services = setup.services.clone();
    let store = setup.store.clone();
    let mut conn = store.pool.get().await.unwrap();

    diesel::insert_into(diesel_models::schema::plan_version_adjustment::table)
        .values(&PlanVersionAdjustmentRowNew {
            kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
            plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
            amount_subunit: 5_000, // $50.00
            includes_all_components: true,
        })
        .execute(&mut conn)
        .await
        .unwrap();

    // 100%-off coupon — drives the post-discount amount of the recurring line to zero.
    let coupon_id = store
        .create_coupon(
            common_domain::actor::Actor::System,
            CouponNew {
                code: "FULLOFF".to_string(),
                description: "100% off".to_string(),
                tenant_id: TENANT_ID,
                discount: CouponDiscount::Percentage(Decimal::from(100)),
                expires_at: None,
                redemption_limit: None,
                recurring_value: None,
                reusable: false,
                plan_ids: vec![],
            },
        )
        .await
        .unwrap()
        .id;

    let subscription_id = services
        .insert_subscription(
            common_domain::actor::Actor::System,
            CreateSubscription {
                subscription: SubscriptionNew {
                    customer_id: CUST_UBER_ID,
                    plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
                    net_terms: None,
                    invoice_memo: None,
                    invoice_threshold: None,
                    start_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                    end_date: None,
                    billing_start_date: Some(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()),
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
                price_components: None,
                add_ons: None,
                coupons: Some(CreateSubscriptionCoupons {
                    coupons: vec![CreateSubscriptionCoupon { coupon_id }],
                }),
                entitlements: vec![],
                minimum_override: None,
                drop_minimum: false,
            },
            TENANT_ID,
        )
        .await
        .unwrap()
        .id;

    let mut sub_details = store
        .get_subscription_details_with_conn(&mut conn, TENANT_ID, subscription_id)
        .await
        .unwrap();
    // The renewal closing January, as the lifecycle bills it.
    sub_details.subscription.cycle_index = Some(1);

    let invoice_date = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    let result = services
        .compute_invoice(&invoice_date, &sub_details, None, InvoiceType::Recurring)
        .await
        .unwrap();

    let regular: Vec<_> = result
        .invoice_lines
        .iter()
        .filter(|l| l.kind == meteroid_store::domain::LineItemKind::Regular)
        .collect();
    let shortfall: Vec<_> = result
        .invoice_lines
        .iter()
        .filter(|l| l.kind == meteroid_store::domain::LineItemKind::MinimumShortfall)
        .collect();

    assert_eq!(regular.len(), 1, "one recurring line");
    assert_eq!(regular[0].amount_subtotal, 3_500, "gross LeetCode @ $35.00");
    assert_eq!(
        regular[0].taxable_amount, 0,
        "the recurring line is fully discounted by the 100% coupon"
    );

    assert_eq!(shortfall.len(), 1, "one shortfall line");
    assert_eq!(
        shortfall[0].amount_subtotal, 1_500,
        "gross floor: 5000 - 3500 gross spend = 1500 (a post-discount floor would wrongly charge 5000)"
    );
    assert_eq!(
        shortfall[0].taxable_amount, 0,
        "the 100% coupon also discounts the floor top-up"
    );

    assert_eq!(
        result.discount, 5_000,
        "the 100% coupon discounts the recurring line and the floor top-up"
    );
    assert_eq!(
        result.subtotal, 5_000,
        "subtotal is the gross sum: 3500 recurring + 1500 shortfall"
    );
}

/// Zero-usage refresh guard (P0). A minimum-bearing invoice that carries NO usage line must
/// still be recomputed on refresh — otherwise a shortfall-only / floor-bearing invoice freezes
/// stale when the floor (or usage) changes after it was first computed.
///
///   1. Usage plan (Rate €20 advance + bandwidth arrear), zero usage, no minimum: the Feb 1
///      renewal draft bills February's €20 rate, no usage line, no shortfall.
///   2. Set a €50 subscription minimum AFTER that draft exists, then refresh it.
/// The refresh trues up January: €50 floor − €20 rate billed on January's invoice = €30.
#[tokio::test]
async fn test_refresh_recomputes_floor_on_zero_usage_invoice() {
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let renewal_date = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();

    // Empty mock => zero usage for every period, so no usage line is produced.
    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        // Keep the invoices in Draft so the renewal can be refreshed (refresh only edits drafts).
        .no_auto_advance()
        .create(env.services())
        .await;
    env.process_cycles().await;

    let invoices = env.get_invoices(sub_id).await;
    let invoice = invoices
        .iter()
        .find(|i| {
            i.invoice_type == InvoiceType::Recurring && i.billing_period_start == renewal_date
        })
        .expect("the Feb 1 renewal draft exists");
    assert!(
        !invoice
            .line_items
            .iter()
            .any(|l| l.kind == meteroid_store::domain::LineItemKind::MinimumShortfall),
        "no shortfall before a minimum is set"
    );
    assert!(
        !invoice.line_items.iter().any(|l| l.metric_id.is_some()),
        "zero usage => no usage line (so the floor guard is the only thing forcing recompute)"
    );
    let invoice_id = invoice.id;

    env.services()
        .set_subscription_minimum(
            TENANT_ID,
            sub_id,
            5_000,
            meteroid_store::domain::plan_minimums::MinimumScope::AllComponents,
        )
        .await
        .unwrap();

    let refreshed = env
        .services()
        .refresh_invoice_data(invoice_id, TENANT_ID)
        .await
        .unwrap();

    let shortfall: Vec<_> = refreshed
        .invoice
        .line_items
        .iter()
        .filter(|l| l.kind == meteroid_store::domain::LineItemKind::MinimumShortfall)
        .collect();
    assert_eq!(
        shortfall.len(),
        1,
        "refresh recomputed the floor on a no-usage invoice (the guard did not freeze it)"
    );
    assert_eq!(
        shortfall[0].amount_subtotal, 3_000,
        "€50 floor − €20 rate billed for January = €30 shortfall"
    );
    assert_eq!(
        (shortfall[0].start_date, shortfall[0].end_date),
        (start_date, renewal_date)
    );
}

/// P0 — the synthesized shortfall line must be TAXED like any other line when a tax resolver
/// is configured. The line is appended before `process_invoice_lines_taxes`, carries a positive
/// `taxable_amount` and no `product_id`, so it picks up the default rate. Without this, the
/// whole suite passes even if the shortfall were silently zero-rated (every other minimum test
/// runs under `TaxResolverEnum::None`, which short-circuits taxation).
///
/// FR→FR domestic VAT = 20%. The Feb 1 renewal bills February's $35 plus January's $15 true-up:
/// $50 subtotal; tax = $10 ($7 on the recurring line + $3 on the shortfall).
#[tokio::test]
async fn test_minimum_shortfall_line_is_taxed() {
    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;
    let mut conn = env.conn().await;

    // EU VAT resolver on the (French) invoicing entity.
    env.store()
        .patch_invoicing_entity(
            common_domain::actor::Actor::System,
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                tax_resolver: Some(TaxResolverEnum::MeteroidEuVat),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .unwrap();

    diesel::insert_into(diesel_models::schema::plan_version_adjustment::table)
        .values(&PlanVersionAdjustmentRowNew {
            kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
            plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
            amount_subunit: 5_000, // $50.00 floor
            includes_all_components: true,
        })
        .execute(&mut conn)
        .await
        .unwrap();

    let french_customer_id = create_french_b2b_customer(&mut conn).await;

    let subscription_id = subscription()
        .customer(french_customer_id)
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let mut sub_details = env
        .store()
        .get_subscription_details_with_conn(&mut conn, TENANT_ID, subscription_id)
        .await
        .unwrap();
    sub_details.subscription.cycle_index = Some(1);

    let invoice_date = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    let result = env
        .services()
        .compute_invoice(&invoice_date, &sub_details, None, InvoiceType::Recurring)
        .await
        .unwrap();

    let shortfall: Vec<_> = result
        .invoice_lines
        .iter()
        .filter(|l| l.kind == LineItemKind::MinimumShortfall)
        .collect();
    assert_eq!(shortfall.len(), 1, "one shortfall line");
    assert_eq!(
        shortfall[0].amount_subtotal, 1_500,
        "$50 − $35 = $15 floor top-up"
    );
    assert_eq!(
        shortfall[0].tax_rate,
        Decimal::new(2, 1),
        "shortfall is taxed at the default 20% rate (not zero-rated)"
    );
    assert_eq!(
        shortfall[0].tax_amount, 300,
        "20% of the $15 shortfall = $3 tax"
    );
    assert_eq!(shortfall[0].taxable_amount, 1_500);

    assert_eq!(
        result.tax_amount, 1_000,
        "total tax = $7 (recurring) + $3 (shortfall) = $10"
    );
    assert_eq!(result.subtotal, 5_000);
    assert_eq!(result.total, 6_000, "$50 subtotal + $10 tax");
}

/// P1 — the prorated floor rounds half-up, not truncates. Plain integer division would floor
/// every partial period (a small but systematic under-charge). A subscription ending on Jan 3 is
/// trued up on its final invoice over 2 of January's 31 days:
///   floor = 5000·2/31 = 322.58… → round-half-up 323   (truncation would give 322)
/// A pure-usage plan with no usage keeps the counted spend at 0, isolating the rounding rule.
#[tokio::test]
async fn test_minimum_floor_prorated_rounds_half_up() {
    use crate::data::ids::PLAN_VERSION_USAGE_BETA_ID;

    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;
    let mut conn = env.conn().await;
    strip_usage_beta_rate(&mut conn).await;

    let subscription_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_BETA_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let mut sub_details = env
        .store()
        .get_subscription_details_with_conn(&mut conn, TENANT_ID, subscription_id)
        .await
        .unwrap();
    sub_details.minimum = Some(SubscriptionMinimum {
        subscription_id,
        amount_subunit: 5_000,
        scope: MinimumScope::AllComponents,
        is_override: true,
    });

    // Ended on Jan 3, as termination leaves it: the period settles early at that date.
    let end = NaiveDate::from_ymd_opt(2024, 1, 3).unwrap();
    let sub = &mut sub_details.subscription;
    sub.period = meteroid_store::domain::BillingPeriodEnum::Monthly;
    sub.billing_day_anchor = 1;
    sub.billing_start_date = Some(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());
    sub.current_period_start = end;
    sub.current_period_end = None;
    sub.pending_checkout = false;
    sub.cycle_index = Some(0);
    sub.end_date = Some(end);

    let result = env
        .services()
        .compute_invoice(&end, &sub_details, None, InvoiceType::Recurring)
        .await
        .unwrap();

    let shortfall: Vec<_> = result
        .invoice_lines
        .iter()
        .filter(|l| l.kind == LineItemKind::MinimumShortfall)
        .collect();
    assert_eq!(shortfall.len(), 1, "one prorated shortfall line");
    assert_eq!(
        shortfall[0].amount_subtotal, 323,
        "round-half-up: (5000·2 + 15)/31 = 323 (truncation would wrongly give 322)"
    );
    assert_eq!(
        (shortfall[0].start_date, shortfall[0].end_date),
        (NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(), end)
    );
}

/// P1 — paid trial days are not committed days: a renewal closing January after a 31-day paid
/// trial trues nothing up; after a 14-day one it trues up the 17 other days, 10000 × 17/31 = 5484
/// less January's 3500: 1984. The subscription's status plays no part.
#[tokio::test]
async fn test_minimum_skipped_during_active_trial() {
    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;
    let mut conn = env.conn().await;

    let subscription_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let mut sub_details = env
        .store()
        .get_subscription_details_with_conn(&mut conn, TENANT_ID, subscription_id)
        .await
        .unwrap();

    sub_details.minimum = Some(SubscriptionMinimum {
        subscription_id,
        amount_subunit: 10_000,
        scope: MinimumScope::AllComponents,
        is_override: true,
    });
    sub_details.trial_config = Some(meteroid_store::domain::TrialConfig {
        duration_days: 31,
        is_free: false,
        trialing_plan_id: None,
        trialing_plan_name: None,
    });
    sub_details.subscription.status = SubscriptionStatusEnum::Active;
    sub_details.subscription.cycle_index = Some(1);

    let invoice_date = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    let shortfall = |trial_days: u32| {
        let mut details = sub_details.clone();
        details.subscription.trial_duration = Some(trial_days);
        let env = &env;
        async move {
            env.services()
                .compute_invoice(&invoice_date, &details, None, InvoiceType::Recurring)
                .await
                .unwrap()
                .invoice_lines
                .iter()
                .filter(|l| l.kind == LineItemKind::MinimumShortfall)
                .map(|l| (l.amount_subtotal, l.start_date, l.end_date))
                .collect::<Vec<_>>()
        }
    };
    assert!(
        shortfall(31).await.is_empty(),
        "no committed day in a period all trial"
    );
    assert_eq!(
        shortfall(14).await,
        vec![(
            1_984,
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            invoice_date
        )]
    );
}

/// P1 — the FIRST post-trial period is trued up against the FULL floor, not a prorated one. A
/// trial-started subscription keeps `billing_start_date` at the original (pre-trial) date; the
/// renewal closing [Feb 1, Mar 1) must still measure a whole period. Components cleared and no
/// invoice dated in February, so the shortfall equals the floor exactly.
#[tokio::test]
async fn test_minimum_full_floor_on_first_post_trial_period() {
    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;
    let mut conn = env.conn().await;

    let subscription_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let mut sub_details = env
        .store()
        .get_subscription_details_with_conn(&mut conn, TENANT_ID, subscription_id)
        .await
        .unwrap();

    sub_details.price_components.clear();
    sub_details.add_ons.clear();
    sub_details.minimum = Some(SubscriptionMinimum {
        subscription_id,
        amount_subunit: 5_000,
        scope: MinimumScope::AllComponents,
        is_override: true,
    });

    // First real period after a January trial: billing_start_date stays at the pre-trial date.
    let sub = &mut sub_details.subscription;
    sub.status = SubscriptionStatusEnum::Active;
    sub.billing_start_date = Some(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());
    sub.current_period_start = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    sub.current_period_end = Some(NaiveDate::from_ymd_opt(2024, 3, 1).unwrap());
    sub.cycle_index = Some(1);
    sub.end_date = None;

    let invoice_date = NaiveDate::from_ymd_opt(2024, 3, 1).unwrap();
    let result = env
        .services()
        .compute_invoice(&invoice_date, &sub_details, None, InvoiceType::Recurring)
        .await
        .unwrap();

    let shortfall: Vec<_> = result
        .invoice_lines
        .iter()
        .filter(|l| l.kind == LineItemKind::MinimumShortfall)
        .collect();
    assert_eq!(shortfall.len(), 1, "one shortfall line");
    assert_eq!(
        shortfall[0].amount_subtotal, 5_000,
        "full floor on the first post-trial period (pre-trial billing_start must not prorate it)"
    );
}

/// OnCheckout / self-serve subscriptions are previewed (`GetCheckout`) and billed while still
/// `pending_checkout` with `current_period_end = None`. That first invoice opens the first period:
/// no period has completed, so no minimum is billed at checkout; the period is trued up by the
/// invoice that closes it.
#[tokio::test]
async fn test_minimum_not_billed_on_checkout_pending_period() {
    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;
    let mut conn = env.conn().await;

    let subscription_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let mut sub_details = env
        .store()
        .get_subscription_details_with_conn(&mut conn, TENANT_ID, subscription_id)
        .await
        .unwrap();

    // Isolate the floor math: no component lines means counted = 0, so shortfall == prorated floor.
    sub_details.price_components.clear();
    sub_details.add_ons.clear();
    sub_details.minimum = Some(SubscriptionMinimum {
        subscription_id,
        amount_subunit: 5_000,
        scope: MinimumScope::AllComponents,
        is_override: true,
    });

    // Reproduce the OnCheckout state seen at preview/billing time: not yet activated, so the
    // period end is still open and `pending_checkout` is set.
    let sub = &mut sub_details.subscription;
    sub.status = SubscriptionStatusEnum::PendingActivation;
    sub.pending_checkout = true;
    sub.period = meteroid_store::domain::BillingPeriodEnum::Monthly;
    sub.billing_day_anchor = 1;
    sub.billing_start_date = Some(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());
    sub.current_period_start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    sub.current_period_end = None;
    sub.end_date = None;

    let invoice_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let result = env
        .services()
        .compute_invoice(&invoice_date, &sub_details, None, InvoiceType::Recurring)
        .await
        .unwrap();

    let shortfall: Vec<_> = result
        .invoice_lines
        .iter()
        .filter(|l| l.kind == LineItemKind::MinimumShortfall)
        .collect();
    assert!(
        shortfall.is_empty(),
        "nothing has completed at checkout, so no minimum is billed there"
    );
}

/// The displayed subscription MRR honors the minimum-commitment floor: a plan rate below the
/// minimum still recognizes the committed floor as recurring revenue (rate $35 / minimum $90 ⇒
/// MRR $90), not just the line spend. Regression for the minimum being ignored in the MRR rollup.
#[tokio::test]
async fn test_subscription_mrr_reflects_minimum_floor() {
    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;
    let mut conn = env.conn().await;

    // $90.00 floor on the $35.00 LeetCode plan.
    diesel::insert_into(diesel_models::schema::plan_version_adjustment::table)
        .values(&PlanVersionAdjustmentRowNew {
            kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
            plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
            amount_subunit: 9_000,
            includes_all_components: true,
        })
        .execute(&mut conn)
        .await
        .unwrap();

    let subscription_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let sub_details = env
        .store()
        .get_subscription_details_with_conn(&mut conn, TENANT_ID, subscription_id)
        .await
        .unwrap();

    assert_eq!(
        sub_details.subscription.mrr_cents, 9_000,
        "MRR is floored to the $90 minimum commitment, not the $35 plan rate"
    );
}

/// A usage-only plan's minimum is recurring revenue from the start although its first invoice
/// only comes at the period end: New Business at creation, a plan change books the difference of
/// the floors where it applies, and the cancellation churns the floor reached.
#[tokio::test]
async fn test_usage_only_minimum_books_mrr_without_an_invoice() {
    use crate::data::plans::{PlanSeed, SeedComp};
    use common_domain::ids::{PlanId, PlanVersionId, PriceComponentId, PriceId};
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_models::enums::{BillingPeriodEnum, MrrMovementType};
    use diesel_models::schema::bi_mrr_movement_log::dsl as m;
    use meteroid_store::domain::price_components::UsagePricingModel;
    use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;

    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;
    let mut conn = env.conn().await;
    let mut versions = vec![];
    for (name, floor) in [("Usage, 90 floor", 9_000), ("Usage, 150 floor", 15_000)] {
        let version_id = PlanVersionId::new();
        PlanSeed::new(PlanId::new(), name, version_id)
            .components(vec![SeedComp::usage(
                PriceComponentId::new(),
                "Bandwidth",
                PRODUCT_BANDWIDTH_ID,
                METRIC_BANDWIDTH,
                PriceId::new(),
                BillingPeriodEnum::Monthly,
                UsagePricingModel::PerUnit {
                    rate: Decimal::new(10, 2),
                },
            )])
            .seed(&mut conn)
            .await
            .unwrap();
        diesel::insert_into(diesel_models::schema::plan_version_adjustment::table)
            .values(&PlanVersionAdjustmentRowNew {
                kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
                plan_version_id: version_id,
                amount_subunit: floor,
                includes_all_components: true,
            })
            .execute(&mut conn)
            .await
            .unwrap();
        versions.push(version_id);
    }

    let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let subscription_id = subscription()
        .plan_version(versions[0])
        .start_date(start)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    assert!(
        env.get_invoices(subscription_id).await.is_empty(),
        "usage is billed in arrears"
    );
    assert_eq!(env.get_subscription(subscription_id).await.mrr_cents, 9_000);

    // The movement carries its customer, subscription and plan version itself.
    let created: (CustomerId, i64, PlanVersionId) = m::bi_mrr_movement_log
        .filter(m::subscription_id.eq(subscription_id))
        .order((m::created_at, m::id))
        .select((m::customer_id, m::net_mrr_change, m::plan_version_id))
        .first(&mut conn)
        .await
        .expect("the creation is logged without an invoice");
    assert_eq!(created, (CUST_UBER_ID, 9_000, versions[0]));

    // A minimum defers the change to the period end.
    env.services()
        .schedule_plan_change(
            common_domain::actor::Actor::System,
            subscription_id,
            TENANT_ID,
            versions[1],
            vec![],
        )
        .await
        .expect("plan change scheduled");
    env.process_cycles().await;
    let change = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    assert_eq!(
        env.get_subscription(subscription_id).await.mrr_cents,
        15_000
    );

    let cancel = NaiveDate::from_ymd_opt(2024, 2, 15).unwrap();
    env.services()
        .cancel_subscription(
            common_domain::actor::Actor::System,
            subscription_id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(cancel),
        )
        .await
        .expect("cancellation");
    env.process_cycles().await;
    env.process_cycles().await;
    assert_eq!(env.get_subscription(subscription_id).await.mrr_cents, 0);

    let movements: Vec<(MrrMovementType, i64, NaiveDate)> = m::bi_mrr_movement_log
        .filter(m::subscription_id.eq(subscription_id))
        .order((m::created_at, m::id))
        .select((m::movement_type, m::net_mrr_change, m::applies_to))
        .load(&mut conn)
        .await
        .unwrap();
    assert_eq!(
        movements,
        vec![
            (MrrMovementType::NewBusiness, 9_000, start),
            (MrrMovementType::Expansion, 6_000, change),
            (MrrMovementType::Churn, -15_000, cancel),
        ]
    );
}

/// Gross-floor corollary: a 100%-off coupon on spend that ALREADY meets the floor produces NO
/// shortfall (a post-discount floor would wrongly re-bill the entire minimum). LeetCode $35 with
/// a $30 floor: gross $35 ≥ $30, so the floor is satisfied and the 100% coupon just zeroes the
/// payable amount — nothing is clawed back.
#[tokio::test]
async fn test_minimum_no_shortfall_when_gross_meets_floor_under_full_coupon() {
    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;
    let mut conn = env.conn().await;

    diesel::insert_into(diesel_models::schema::plan_version_adjustment::table)
        .values(&PlanVersionAdjustmentRowNew {
            kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
            plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
            amount_subunit: 3_000, // $30 floor — below the $35 gross recurring
            includes_all_components: true,
        })
        .execute(&mut conn)
        .await
        .unwrap();

    let coupon_id = env
        .store()
        .create_coupon(
            common_domain::actor::Actor::System,
            CouponNew {
                code: "FULLOFF2".to_string(),
                description: "100% off".to_string(),
                tenant_id: TENANT_ID,
                discount: CouponDiscount::Percentage(Decimal::from(100)),
                expires_at: None,
                redemption_limit: None,
                recurring_value: None,
                reusable: false,
                plan_ids: vec![],
            },
        )
        .await
        .unwrap()
        .id;

    let subscription_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .coupon(coupon_id)
        .create(env.services())
        .await;

    let mut sub_details = env
        .store()
        .get_subscription_details_with_conn(&mut conn, TENANT_ID, subscription_id)
        .await
        .unwrap();
    // The renewal closing January, whose $35 fee is the period's spend.
    sub_details.subscription.cycle_index = Some(1);

    let invoice_date = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    let result = env
        .services()
        .compute_invoice(&invoice_date, &sub_details, None, InvoiceType::Recurring)
        .await
        .unwrap();

    let shortfall: Vec<_> = result
        .invoice_lines
        .iter()
        .filter(|l| l.kind == LineItemKind::MinimumShortfall)
        .collect();
    assert!(
        shortfall.is_empty(),
        "gross spend ($35) meets the $30 floor → no top-up, even under a 100% coupon"
    );
    assert_eq!(
        result.discount, 3_500,
        "the 100% coupon zeroes the recurring line"
    );
    assert_eq!(
        result.subtotal, 3_500,
        "only the gross recurring line, no shortfall"
    );
}

/// Plan-level (`AllComponents`) minimum on Usage Beta (Rate €20 + 2 usage comps). With usage below
/// the floor, the arrears invoice must keep the per-component consumption lines beside the floor
/// top-up (counted 3500 < 5000 floor ⇒ €15 shortfall), not replace them.
#[tokio::test]
async fn test_multiple_usage_components_below_minimum_keeps_consumption_lines() {
    use crate::data::ids::{METRIC_BANDWIDTH, METRIC_DATABASE_SIZE, PLAN_VERSION_USAGE_BETA_ID};
    use chrono::NaiveTime;
    use meteroid_store::clients::usage::{GroupedUsageData, MockUsageDataParams, UsageData};
    use meteroid_store::domain::UsagePeriod;
    use std::collections::HashMap;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let period1_end = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();

    // Usage for the first (arrears) period for BOTH metrics, kept below the floor.
    let mut data = HashMap::new();
    for (metric_id, units) in [(METRIC_BANDWIDTH, 50i64), (METRIC_DATABASE_SIZE, 10i64)] {
        data.insert(
            MockUsageDataParams {
                metric_id,
                period_start: start_date.and_time(NaiveTime::MIN),
                period_end: period1_end.and_time(NaiveTime::MIN),
            },
            UsageData {
                data: vec![GroupedUsageData {
                    value: Decimal::from(units),
                    dimensions: HashMap::new(),
                }],
                period: UsagePeriod {
                    start: start_date.and_time(NaiveTime::MIN),
                    end: period1_end.and_time(NaiveTime::MIN),
                },
            },
        );
    }

    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient { data })).await;
    let mut conn = env.conn().await;

    // €50 plan-level floor (AllComponents) on Usage Beta.
    diesel::insert_into(diesel_models::schema::plan_version_adjustment::table)
        .values(&PlanVersionAdjustmentRowNew {
            kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
            plan_version_id: PLAN_VERSION_USAGE_BETA_ID,
            amount_subunit: 5_000,
            includes_all_components: true,
        })
        .execute(&mut conn)
        .await
        .unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_BETA_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        // Keep drafts so the cycle-1 invoice can be refreshed (refresh only edits drafts) — this
        // mirrors the UI "Refresh" button / grace-period finalization on a usage invoice.
        .no_auto_advance()
        .create(env.services())
        .await;

    // Cycle 1 renewal → arrears invoice that bills period-0 usage + the floor top-up.
    env.process_cycles().await;

    let invoices = env.get_invoices(sub_id).await;
    // The arrears invoice is the SECOND one (cycle 1): it bills period-0 usage. The cycle-0
    // advance invoice legitimately has no usage line, so pick the latest invoice by date.
    let arrears_id = invoices
        .iter()
        .filter(|i| i.invoice_type == InvoiceType::Recurring)
        .max_by_key(|i| i.invoice_date)
        .expect("an arrears invoice exists")
        .id;

    // As created, the arrears invoice carries both consumption lines plus the floor top-up.
    let before = invoices.iter().find(|i| i.id == arrears_id).unwrap();
    assert!(
        before
            .line_items
            .iter()
            .any(|l| l.metric_id == Some(METRIC_BANDWIDTH)),
        "sanity: consumption lines present before refresh"
    );

    // The draft is left unfinalized while the subscription renews again (manual-invoicing flow):
    // the subscription advances to the next cycle before the old draft is refreshed.
    env.process_cycles().await;

    // Refresh the draft invoice (UI Refresh / finalization). This must NOT drop the consumption
    // lines just because the floor tops the invoice up to the minimum.
    env.services()
        .refresh_invoice_data(arrears_id, TENANT_ID)
        .await
        .expect("refresh failed");

    let invoices = env.get_invoices(sub_id).await;
    let invoice = invoices.iter().find(|i| i.id == arrears_id).unwrap();

    let names: Vec<&str> = invoice.line_items.iter().map(|l| l.name.as_str()).collect();

    let api = invoice
        .line_items
        .iter()
        .find(|l| l.metric_id == Some(METRIC_BANDWIDTH));
    let db = invoice
        .line_items
        .iter()
        .find(|l| l.metric_id == Some(METRIC_DATABASE_SIZE));
    let shortfall: Vec<_> = invoice
        .line_items
        .iter()
        .filter(|l| l.kind == LineItemKind::MinimumShortfall)
        .collect();

    assert!(
        api.is_some(),
        "the API Calls consumption line must remain on the invoice (lines: {names:?})"
    );
    assert!(
        db.is_some(),
        "the DB Storage consumption line must remain on the invoice (lines: {names:?})"
    );
    assert_eq!(api.unwrap().amount_subtotal, 1_000, "50 × €0.20 = €10");
    assert_eq!(db.unwrap().amount_subtotal, 500, "10 × €0.50 = €5");
    assert_eq!(shortfall.len(), 1, "exactly one floor top-up line");
    assert_eq!(
        shortfall[0].amount_subtotal, 1_500,
        "€50 floor − €35 counted (rate €20 + usage €15) = €15 top-up"
    );
    assert_eq!(invoice.subtotal, 5_000, "subtotal trued up to the floor");
}

/// As above but a pure-usage plan: the Platform Fee is stripped from Usage Beta, then the full
/// billing pipeline runs (create → cycle → finalize). The arrears invoice must still carry both
/// consumption lines beside the floor top-up — exercising the real bill/finalize path.
#[tokio::test]
async fn test_pure_usage_plan_real_pipeline_below_minimum_keeps_consumption_lines() {
    use crate::data::ids::{
        COMP_USAGE_BETA_RATE_ID, METRIC_BANDWIDTH, METRIC_DATABASE_SIZE, PLAN_VERSION_USAGE_BETA_ID,
    };
    use chrono::NaiveTime;
    use diesel::ExpressionMethods;
    use meteroid_store::clients::usage::{GroupedUsageData, MockUsageDataParams, UsageData};
    use meteroid_store::domain::UsagePeriod;
    use std::collections::HashMap;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let period1_end = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();

    let mut data = HashMap::new();
    for (metric_id, units) in [(METRIC_BANDWIDTH, 50i64), (METRIC_DATABASE_SIZE, 10i64)] {
        data.insert(
            MockUsageDataParams {
                metric_id,
                period_start: start_date.and_time(NaiveTime::MIN),
                period_end: period1_end.and_time(NaiveTime::MIN),
            },
            UsageData {
                data: vec![GroupedUsageData {
                    value: Decimal::from(units),
                    dimensions: HashMap::new(),
                }],
                period: UsagePeriod {
                    start: start_date.and_time(NaiveTime::MIN),
                    end: period1_end.and_time(NaiveTime::MIN),
                },
            },
        );
    }

    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient { data })).await;
    let mut conn = env.conn().await;

    // Strip the advance Platform Fee → a pure usage/arrears plan (only API Calls + DB Storage).
    {
        use diesel::QueryDsl;
        use diesel_models::schema::price_component::dsl as pc;
        diesel::delete(pc::price_component.filter(pc::id.eq(COMP_USAGE_BETA_RATE_ID)))
            .execute(&mut conn)
            .await
            .unwrap();
    }

    // €50 plan-level floor (AllComponents).
    diesel::insert_into(diesel_models::schema::plan_version_adjustment::table)
        .values(&PlanVersionAdjustmentRowNew {
            kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
            plan_version_id: PLAN_VERSION_USAGE_BETA_ID,
            amount_subunit: 5_000,
            includes_all_components: true,
        })
        .execute(&mut conn)
        .await
        .unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_BETA_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    env.process_cycles().await;

    let invoices = env.get_invoices(sub_id).await;

    // The arrears invoice (the one carrying the floor top-up) must keep both consumption lines.
    let invoice = invoices
        .iter()
        .find(|i| {
            i.invoice_type == InvoiceType::Recurring
                && i.line_items
                    .iter()
                    .any(|l| l.kind == LineItemKind::MinimumShortfall)
        })
        .expect("an arrears invoice with a floor top-up exists");

    let names: Vec<&str> = invoice.line_items.iter().map(|l| l.name.as_str()).collect();
    assert!(
        invoice
            .line_items
            .iter()
            .any(|l| l.metric_id == Some(METRIC_BANDWIDTH)),
        "API Calls consumption line must remain (lines: {names:?})"
    );
    assert!(
        invoice
            .line_items
            .iter()
            .any(|l| l.metric_id == Some(METRIC_DATABASE_SIZE)),
        "DB Storage consumption line must remain (lines: {names:?})"
    );
}

/// Pure-usage plan (no advance component) with a plan-level minimum, driving `compute_invoice`
/// directly on the renewal closing January: with usage below the floor, both per-component
/// consumption lines must remain beside the floor top-up.
#[tokio::test]
async fn test_pure_usage_components_below_minimum_keeps_consumption_lines() {
    use crate::data::ids::{METRIC_BANDWIDTH, METRIC_DATABASE_SIZE, PLAN_VERSION_USAGE_BETA_ID};
    use chrono::NaiveTime;
    use meteroid_store::clients::usage::{GroupedUsageData, MockUsageDataParams, UsageData};
    use meteroid_store::domain::UsagePeriod;
    use std::collections::HashMap;

    let period0_start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let period0_end = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    let period1_end = NaiveDate::from_ymd_opt(2024, 3, 1).unwrap();

    let mut data = HashMap::new();
    for (metric_id, units) in [(METRIC_BANDWIDTH, 50i64), (METRIC_DATABASE_SIZE, 10i64)] {
        data.insert(
            MockUsageDataParams {
                metric_id,
                period_start: period0_start.and_time(NaiveTime::MIN),
                period_end: period0_end.and_time(NaiveTime::MIN),
            },
            UsageData {
                data: vec![GroupedUsageData {
                    value: Decimal::from(units),
                    dimensions: HashMap::new(),
                }],
                period: UsagePeriod {
                    start: period0_start.and_time(NaiveTime::MIN),
                    end: period0_end.and_time(NaiveTime::MIN),
                },
            },
        );
    }

    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient { data })).await;
    let mut conn = env.conn().await;
    strip_usage_beta_rate(&mut conn).await;

    let subscription_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_BETA_ID)
        .start_date(period0_start)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let mut sub_details = env
        .store()
        .get_subscription_details_with_conn(&mut conn, TENANT_ID, subscription_id)
        .await
        .unwrap();

    // Drop the advance Rate component → a pure usage/arrears plan. Keep both usage components.
    sub_details
        .price_components
        .retain(|c| c.fee.metric_id().is_some());
    sub_details.add_ons.clear();
    assert_eq!(
        sub_details.price_components.len(),
        2,
        "two usage components remain (API Calls + DB Storage)"
    );

    // €50 plan-level floor across all (usage) components.
    sub_details.minimum = Some(SubscriptionMinimum {
        subscription_id,
        amount_subunit: 5_000,
        scope: MinimumScope::AllComponents,
        is_override: true,
    });

    // Arrears (cycle-1) state: the just-completed period [Jan1, Feb1] is billed in arrears, the
    // active period is now [Feb1, Mar1].
    let sub = &mut sub_details.subscription;
    sub.billing_start_date = Some(period0_start);
    sub.current_period_start = period0_end;
    sub.current_period_end = Some(period1_end);
    sub.cycle_index = Some(1);
    sub.end_date = None;

    let result = env
        .services()
        .compute_invoice(&period0_end, &sub_details, None, InvoiceType::Recurring)
        .await
        .unwrap();

    let names: Vec<&str> = result
        .invoice_lines
        .iter()
        .map(|l| l.name.as_str())
        .collect();
    let api = result
        .invoice_lines
        .iter()
        .find(|l| l.metric_id == Some(METRIC_BANDWIDTH));
    let db = result
        .invoice_lines
        .iter()
        .find(|l| l.metric_id == Some(METRIC_DATABASE_SIZE));
    let shortfall: Vec<_> = result
        .invoice_lines
        .iter()
        .filter(|l| l.kind == LineItemKind::MinimumShortfall)
        .collect();

    assert!(
        api.is_some(),
        "the API Calls consumption line must remain (lines: {names:?})"
    );
    assert!(
        db.is_some(),
        "the DB Storage consumption line must remain (lines: {names:?})"
    );
    assert_eq!(api.unwrap().amount_subtotal, 1_000, "50 × €0.20 = €10");
    assert_eq!(db.unwrap().amount_subtotal, 500, "10 × €0.50 = €5");
    assert_eq!(shortfall.len(), 1, "one floor top-up line");
    assert_eq!(
        shortfall[0].amount_subtotal, 3_500,
        "€50 floor − €15 usage = €35 top-up"
    );
}

/// Coupons discount the minimum commitment, not only the usage it tops up (issue #1266).
/// Pure-usage plan with no usage at all and a €50 floor: the arrears invoice is the €50 top-up
/// alone, and a €5 (or 10%) coupon must bring it down to €45. Before the fix, the coupon was
/// computed on the €0 of usage and the customer was billed the full €50.
#[rstest]
#[case::fixed(false)]
#[case::percentage(true)]
#[tokio::test]
async fn test_coupon_discounts_minimum_shortfall_with_zero_usage(#[case] percentage: bool) {
    use crate::data::ids::PLAN_VERSION_USAGE_BETA_ID;

    let period0_start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let period0_end = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    let period1_end = NaiveDate::from_ymd_opt(2024, 3, 1).unwrap();

    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;
    let mut conn = env.conn().await;
    strip_usage_beta_rate(&mut conn).await;

    let coupon_id = if percentage {
        env.create_percentage_coupon("TENOFF", 10).await
    } else {
        env.create_fixed_coupon("FIVEOFF", 5, "EUR").await
    };

    let subscription_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_BETA_ID)
        .start_date(period0_start)
        .on_start()
        .no_trial()
        .coupon(coupon_id)
        .create(env.services())
        .await;

    let mut sub_details = env
        .store()
        .get_subscription_details_with_conn(&mut conn, TENANT_ID, subscription_id)
        .await
        .unwrap();

    // Drop the advance Rate component → a pure usage/arrears plan.
    sub_details
        .price_components
        .retain(|c| c.fee.metric_id().is_some());
    sub_details.add_ons.clear();

    sub_details.minimum = Some(SubscriptionMinimum {
        subscription_id,
        amount_subunit: 5_000,
        scope: MinimumScope::AllComponents,
        is_override: true,
    });

    // Arrears (cycle-1) state: [Jan1, Feb1] is billed in arrears, the active period is [Feb1, Mar1].
    let sub = &mut sub_details.subscription;
    sub.billing_start_date = Some(period0_start);
    sub.current_period_start = period0_end;
    sub.current_period_end = Some(period1_end);
    sub.cycle_index = Some(1);
    sub.end_date = None;

    let result = env
        .services()
        .compute_invoice(&period0_end, &sub_details, None, InvoiceType::Recurring)
        .await
        .unwrap();

    let shortfall: Vec<_> = result
        .invoice_lines
        .iter()
        .filter(|l| l.kind == LineItemKind::MinimumShortfall)
        .collect();
    assert_eq!(shortfall.len(), 1, "one floor top-up line");
    assert_eq!(
        shortfall[0].amount_subtotal, 5_000,
        "no usage → the full €50 floor is topped up"
    );

    assert_eq!(result.subtotal, 5_000);
    assert_eq!(result.discount, 500, "the coupon takes €5 off the floor");
    assert_eq!(
        result.applied_coupons.len(),
        1,
        "the coupon shows on the invoice"
    );
    assert_eq!(
        shortfall[0].taxable_amount, 4_500,
        "€50 floor − €5 coupon = €45 to pay"
    );
}

/// Credit held when the €50 floor is billed alone with a 10% coupon under 20% VAT.
#[derive(Clone, Copy, Debug)]
enum FloorCredit {
    Promotional,
    PromotionalAndPaid,
    UsageOnlyPromotional,
}

/// The floor top-up is coupon-discounted first; unrestricted promotional credit then pays it
/// before tax, paid credit pays the taxed total, and usage-only credit never pays it (it has no
/// metric). €50 − €5 coupon − €20 promotional + €5 tax = €30.
#[rstest]
#[case::promotional(FloorCredit::Promotional, 2_000, 500, 3_000, 0, 3_000)]
#[case::promotional_and_paid(FloorCredit::PromotionalAndPaid, 2_000, 500, 3_000, 1_000, 2_000)]
#[case::usage_only_promotional(FloorCredit::UsageOnlyPromotional, 0, 900, 5_400, 0, 5_400)]
#[tokio::test]
async fn test_coupon_and_credit_on_minimum_shortfall_with_zero_usage(
    #[case] credit: FloorCredit,
    #[case] promotional_credits: i64,
    #[case] tax_amount: i64,
    #[case] total: i64,
    #[case] applied_credits: i64,
    #[case] amount_due: i64,
) {
    use crate::data::ids::PLAN_VERSION_USAGE_BETA_ID;
    use crate::helpers::credit::{Grant, fund, fund_promotional_grant};
    use meteroid_store::domain::credit::CreditApplication;

    let period0_start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let period0_end = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    let period1_end = NaiveDate::from_ymd_opt(2024, 3, 1).unwrap();

    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;
    let mut conn = env.conn().await;
    strip_usage_beta_rate(&mut conn).await;
    env.store()
        .patch_invoicing_entity(
            common_domain::actor::Actor::System,
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                tax_resolver: Some(TaxResolverEnum::MeteroidEuVat),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .unwrap();
    let customer_id = create_french_b2b_customer(&mut conn).await;
    let coupon_id = env.create_percentage_coupon("TENOFF", 10).await;

    let subscription_id = subscription()
        .customer(customer_id)
        .plan_version(PLAN_VERSION_USAGE_BETA_ID)
        .start_date(period0_start)
        .on_start()
        .no_trial()
        .coupon(coupon_id)
        .create(env.services())
        .await;

    let promotional = match credit {
        FloorCredit::UsageOnlyPromotional => Grant {
            application: CreditApplication::Usage,
            ..Grant::invoice(2_000)
        },
        FloorCredit::Promotional | FloorCredit::PromotionalAndPaid => Grant::invoice(2_000),
    };
    fund_promotional_grant(env.store(), TENANT_ID, customer_id, None, promotional).await;
    if matches!(credit, FloorCredit::PromotionalAndPaid) {
        fund(env.store(), TENANT_ID, customer_id, None, 1_000).await;
    }

    let mut sub_details = env
        .store()
        .get_subscription_details_with_conn(&mut conn, TENANT_ID, subscription_id)
        .await
        .unwrap();
    sub_details
        .price_components
        .retain(|c| c.fee.metric_id().is_some());
    sub_details.add_ons.clear();
    sub_details.minimum = Some(SubscriptionMinimum {
        subscription_id,
        amount_subunit: 5_000,
        scope: MinimumScope::AllComponents,
        is_override: true,
    });
    let sub = &mut sub_details.subscription;
    sub.billing_start_date = Some(period0_start);
    sub.current_period_start = period0_end;
    sub.current_period_end = Some(period1_end);
    sub.cycle_index = Some(1);
    sub.end_date = None;

    let result = env
        .services()
        .compute_invoice(&period0_end, &sub_details, None, InvoiceType::Recurring)
        .await
        .unwrap();

    let shortfall: Vec<_> = result
        .invoice_lines
        .iter()
        .filter(|l| l.kind == LineItemKind::MinimumShortfall)
        .collect();
    assert_eq!(shortfall.len(), 1, "{credit:?}: one floor top-up line");
    assert_eq!(shortfall[0].amount_subtotal, 5_000, "{credit:?}");
    assert_eq!(shortfall[0].tax_rate, Decimal::new(2, 1), "{credit:?}");
    assert_eq!(shortfall[0].tax_amount, tax_amount, "{credit:?}");

    assert_eq!(result.subtotal, 5_000, "{credit:?}");
    assert_eq!(result.discount, 500, "{credit:?}: 10% off the floor");
    assert_eq!(
        result.promotional_credits, promotional_credits,
        "{credit:?}: promotional credit on the coupon-reduced top-up"
    );
    assert_eq!(result.tax_amount, tax_amount, "{credit:?}");
    assert_eq!(result.total, total, "{credit:?}");
    assert_eq!(result.applied_credits, applied_credits, "{credit:?}");
    assert_eq!(result.amount_due, amount_due, "{credit:?}");
}

/// Immediate (mid-period) cancellation: renew once, then cancel on Feb 16. The final invoice must
/// bill bandwidth arrears for `[Feb 1, Feb 16]` only, not `[Jan 1, Feb 16]` (re-billing the
/// already-invoiced previous month), and true up those 15 of February's 29 days: the €50 floor
/// prorated to 2586, less €5 of usage and the €20 rate net of its 14 unused days (966) = 1052.
#[tokio::test]
async fn test_immediate_cancellation_bills_only_partial_current_period() {
    use crate::data::ids::{METRIC_BANDWIDTH, PLAN_VERSION_USAGE_ID};
    use chrono::NaiveTime;
    use meteroid_store::clients::usage::{GroupedUsageData, MockUsageDataParams, UsageData};
    use meteroid_store::domain::UsagePeriod;
    use meteroid_store::repositories::credit_notes::CreditNoteInterface;
    use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;
    use std::collections::HashMap;

    let jan1 = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let feb1 = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    let feb16 = NaiveDate::from_ymd_opt(2024, 2, 16).unwrap();
    let mar1 = NaiveDate::from_ymd_opt(2024, 3, 1).unwrap();

    // Provide usage for every plausible arrear window so whatever the code queries yields a line.
    let mut data = HashMap::new();
    for (s, e, units) in [
        (jan1, feb1, 100i64),  // Jan, billed at the Feb-1 renewal
        (feb1, feb16, 50i64),  // CORRECT partial Feb window
        (jan1, feb16, 999i64), // BUGGY window [prev-period-start, cancel] (distinct sentinel)
        (feb1, mar1, 70i64),
    ] {
        data.insert(
            MockUsageDataParams {
                metric_id: METRIC_BANDWIDTH,
                period_start: s.and_time(NaiveTime::MIN),
                period_end: e.and_time(NaiveTime::MIN),
            },
            UsageData {
                data: vec![GroupedUsageData {
                    value: Decimal::from(units),
                    dimensions: HashMap::new(),
                }],
                period: UsagePeriod {
                    start: s.and_time(NaiveTime::MIN),
                    end: e.and_time(NaiveTime::MIN),
                },
            },
        );
    }

    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient { data })).await;
    let mut conn = env.conn().await;

    diesel::insert_into(diesel_models::schema::plan_version_adjustment::table)
        .values(&PlanVersionAdjustmentRowNew {
            kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
            plan_version_id: PLAN_VERSION_USAGE_ID,
            amount_subunit: 5_000, // €50 floor
            includes_all_components: true,
        })
        .execute(&mut conn)
        .await
        .unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_ID)
        .start_date(jan1)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Renew once so cycle_index = 1 and there is a real previous (Jan) period already billed.
    env.process_cycles().await;

    // Immediate mid-period cancellation (Feb 16), then process ONLY due events (the cancel),
    // so no Mar-1 renewal runs first.
    env.services()
        .cancel_subscription(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            Some("immediate".to_string()),
            CancellationEffectiveAt::Date(feb16),
        )
        .await
        .unwrap();
    env.process_due_events().await;

    let invoices = env.get_invoices(sub_id).await;
    // The final invoice is the one dated at the cancellation day.
    let final_invoice = invoices
        .iter()
        .find(|i| i.billing_period_start == feb16)
        .expect("a final invoice was produced at the cancellation date");

    let bandwidth: Vec<_> = final_invoice
        .line_items
        .iter()
        .filter(|l| l.metric_id == Some(METRIC_BANDWIDTH))
        .collect();
    assert_eq!(
        bandwidth.len(),
        1,
        "one bandwidth line on the final invoice"
    );
    let bw = bandwidth[0];

    // The arrear window must be the partial CURRENT period, not the previous-month overshoot.
    assert_eq!(
        bw.start_date, feb1,
        "final-invoice usage must start at the current period start (Feb 1), not the previous \
         period (got {})",
        bw.start_date
    );
    assert_eq!(
        bw.end_date, feb16,
        "final-invoice usage ends at the cancel date"
    );
    assert_eq!(
        bw.amount_subtotal, 500,
        "only the partial-period usage (50 units × €0.10 = €5) is billed, not the previous month"
    );

    let true_up: Vec<_> = final_invoice
        .line_items
        .iter()
        .filter(|l| l.kind == LineItemKind::MinimumShortfall)
        .map(|l| (l.amount_subtotal, l.start_date, l.end_date))
        .collect();
    assert_eq!(
        true_up,
        vec![(2_586 - 500 - (2_000 - 966), feb1, feb16)],
        "the period cut short is trued up on the final invoice"
    );
    let notes = env
        .store()
        .list_credit_notes_by_invoice_id(
            TENANT_ID,
            invoices
                .iter()
                .find(|i| i.billing_period_start == feb1)
                .expect("February's invoice")
                .id,
        )
        .await
        .expect("credit notes");
    assert!(
        notes
            .iter()
            .flat_map(|n| &n.line_items)
            .all(|l| l.kind != LineItemKind::MinimumShortfall),
        "January's true-up is never credited"
    );
}

/// Cancelling a subscription mid-period refunds the unused, advance-billed portion
/// of the period as credit note(s) against the original recurring invoice (the
/// EU-correct treatment: a credit note, not a negative invoice).
#[rstest]
#[tokio::test]
async fn test_immediate_cancellation_refunds_unused_advance(#[future] test_env: TestEnv) {
    use meteroid_store::repositories::credit_notes::CreditNoteInterface;
    use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;

    let env = test_env.await;
    let start_date = NaiveDate::from_ymd_opt(2024, 4, 1).unwrap();
    let cancel_date = NaiveDate::from_ymd_opt(2024, 4, 16).unwrap(); // 15 of 30 days remaining

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    env.run_outbox_and_orchestration().await;

    let recurring = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .find(|i| i.invoice_type == InvoiceType::Recurring && i.billing_period_start == start_date)
        .expect("recurring invoice for the first period");

    // All Starter components (flat Rate + Slot) are advance-billed, so the whole
    // recurring subtotal is advance spend; half the period is unused at cancel.
    let advance_subtotal: i64 = recurring.line_items.iter().map(|l| l.amount_subtotal).sum();
    let expected_credit = advance_subtotal / 2;
    assert!(expected_credit > 0, "Starter advance spend should be > 0");

    env.services()
        .cancel_subscription(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            Some("immediate".to_string()),
            CancellationEffectiveAt::Date(cancel_date),
        )
        .await
        .expect("cancel should succeed");
    env.process_due_events().await;

    let credit_notes = env
        .store()
        .list_credit_notes_by_invoice_id(TENANT_ID, recurring.id)
        .await
        .expect("list credit notes");

    let refunded: i64 = credit_notes.iter().map(|cn| cn.subtotal).sum();
    assert_eq!(
        refunded, -expected_credit,
        "the unused half of the advance-billed period should be refunded as credit notes"
    );
    assert!(
        credit_notes
            .iter()
            .flat_map(|cn| &cn.line_items)
            .any(|l| l.is_prorated),
        "a partial-period refund should be flagged prorated"
    );
}

/// A €1000 floor over Starter, cancelled on Apr 16 (15 of 30 days served). The first invoice
/// bills the fees alone; the cancellation credits their unused half and nothing else; the final
/// invoice trues up the 15 days served: the prorated €500 floor less the fees kept.
#[rstest]
#[tokio::test]
async fn test_immediate_cancellation_holds_prorated_minimum(#[future] test_env: TestEnv) {
    use meteroid_store::repositories::credit_notes::CreditNoteInterface;
    use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;

    let env = test_env.await;
    let mut conn = env.conn().await;

    let minimum_subunit = 100_000i64;
    diesel::insert_into(diesel_models::schema::plan_version_adjustment::table)
        .values(&PlanVersionAdjustmentRowNew {
            kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
            plan_version_id: PLAN_VERSION_STARTER_ID,
            amount_subunit: minimum_subunit,
            includes_all_components: true,
        })
        .execute(&mut conn)
        .await
        .unwrap();

    let start_date = NaiveDate::from_ymd_opt(2024, 4, 1).unwrap();
    let cancel_date = NaiveDate::from_ymd_opt(2024, 4, 16).unwrap(); // 15 of 30 days served
    let prorated_minimum = minimum_subunit / 2;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    env.run_outbox_and_orchestration().await;

    let recurring = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .find(|i| i.invoice_type == InvoiceType::Recurring && i.billing_period_start == start_date)
        .expect("recurring invoice for the first period");
    assert!(
        recurring
            .line_items
            .iter()
            .all(|l| l.kind != LineItemKind::MinimumShortfall),
        "no period has completed on the first invoice"
    );
    let paid_subtotal: i64 = recurring.line_items.iter().map(|l| l.amount_subtotal).sum();

    env.services()
        .cancel_subscription(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            Some("immediate".to_string()),
            CancellationEffectiveAt::Date(cancel_date),
        )
        .await
        .expect("cancel should succeed");
    env.process_due_events().await;

    let credit_notes = env
        .store()
        .list_credit_notes_by_invoice_id(TENANT_ID, recurring.id)
        .await
        .expect("list credit notes");
    let refunded: i64 = credit_notes.iter().map(|cn| -cn.subtotal).sum();
    assert_eq!(refunded, paid_subtotal / 2, "the fees' unused half");
    let retained = paid_subtotal - refunded;

    let final_invoice = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .find(|i| i.billing_period_start == cancel_date)
        .expect("the final invoice");
    let true_up: Vec<_> = final_invoice
        .line_items
        .iter()
        .filter(|l| l.kind == LineItemKind::MinimumShortfall)
        .map(|l| (l.amount_subtotal, l.start_date, l.end_date))
        .collect();
    assert_eq!(
        true_up,
        vec![(prorated_minimum - retained, start_date, cancel_date)],
        "the days served are trued up to the prorated minimum, net of the fees kept"
    );
}

/// Local FR B2B customer (domestic 20% VAT) for the tax regression test.
async fn create_french_b2b_customer(conn: &mut meteroid_store::store::PgConn) -> CustomerId {
    use diesel_models::customers::CustomerRowNew;

    let customer_id = CustomerId::new();
    let french_address = Address {
        line1: Some("123 Avenue des Champs-Élysées".to_string()),
        line2: None,
        city: Some("Paris".to_string()),
        country: Some(CountryCode::from_str("FR").expect("country code")),
        state: Some("Île-de-France".to_string()),
        zip_code: Some("75008".to_string()),
    };

    CustomerRowNew {
        id: customer_id,
        name: "French Business Customer SAS".to_string(),
        created_at: Some(chrono::Utc::now().naive_utc()),
        tenant_id: TENANT_ID,
        alias: None,
        currency: "EUR".to_string(),
        invoicing_entity_id: INVOICING_ENTITY_ID,
        billing_address: Some(serde_json::to_value(&french_address).unwrap()),
        shipping_address: None,
        billing_email: None,
        current_payment_method_id: None,
        vat_number: Some("FR12345678901".to_string()),
        legal_number: None,
        buyer_reference: None,
        custom_taxes: json!([]),
        invoicing_emails: vec![],
        phone: None,
        tax_status: diesel_models::enums::CustomerTaxStatusEnum::Taxable,
        exemption_reason: None,
        vat_number_format_valid: true,
        connected_account_id: None,
        vat_number_validation_status: None,
        custom_properties: serde_json::Value::Null,
        preferred_locales: vec![],
        customer_type: diesel_models::enums::CustomerTypeEnum::Company,
        first_name: None,
        last_name: None,
    }
    .insert(conn)
    .await
    .unwrap();

    customer_id
}

/// Drops Usage Beta's advance rate, so the plan bills usage only and a subscription's first
/// invoice bills nothing toward the floor.
async fn strip_usage_beta_rate(conn: &mut meteroid_store::store::PgConn) {
    use crate::data::ids::COMP_USAGE_BETA_RATE_ID;
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_models::schema::price_component::dsl as pc;
    diesel::delete(pc::price_component.filter(pc::id.eq(COMP_USAGE_BETA_RATE_ID)))
        .execute(conn)
        .await
        .unwrap();
}

// ---------------------------------------------------------------------------------------------
// No minimum on the first (checkout) invoice, whatever the minimum's scope.
//
// The minimum trues up a completed period on the invoice that closes it; the first invoice opens
// the first period, so nothing has completed there yet.
//
// These build a real usage subscription (Rate advance + Bandwidth usage arrears) so the components
// carry real fee types/metrics, then drive the in-memory `SubscriptionDetails` into the checkout
// (pending, open-period, cycle 0) state to exercise `append_minimum_shortfall_if_any`.
// ---------------------------------------------------------------------------------------------

/// Put `sub_details` into the first/advance checkout state: pending checkout, open period end,
/// cycle 0, billing from Jan 1.
fn drive_to_checkout_state(sub_details: &mut meteroid_store::domain::SubscriptionDetails) {
    let sub = &mut sub_details.subscription;
    sub.status = SubscriptionStatusEnum::PendingActivation;
    sub.pending_checkout = true;
    sub.period = meteroid_store::domain::BillingPeriodEnum::Monthly;
    sub.billing_day_anchor = 1;
    sub.billing_start_date = Some(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());
    sub.current_period_start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    sub.current_period_end = None;
    sub.cycle_index = Some(0);
    sub.end_date = None;
}

/// Product id of the usage (metric-backed) component in the usage plan.
fn usage_product_id(
    sub_details: &meteroid_store::domain::SubscriptionDetails,
) -> common_domain::ids::ProductId {
    sub_details
        .price_components
        .iter()
        .find(|c| c.fee.metric_id().is_some())
        .and_then(|c| c.product_id)
        .expect("usage component has a product id")
}

/// Case 1 — a minimum scoped to the arrears/usage component does not appear on the first
/// (checkout) invoice: up front the invoice carries only the advance rate, no minimum top-up.
#[tokio::test]
async fn test_minimum_on_arrears_skipped_on_advance_checkout_invoice() {
    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;
    let mut conn = env.conn().await;

    let subscription_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let mut sub_details = env
        .store()
        .get_subscription_details_with_conn(&mut conn, TENANT_ID, subscription_id)
        .await
        .unwrap();

    // Floor scoped to the usage product only.
    sub_details.minimum = Some(SubscriptionMinimum {
        subscription_id,
        amount_subunit: 5_000,
        scope: MinimumScope::Products(vec![usage_product_id(&sub_details)]),
        is_override: true,
    });
    drive_to_checkout_state(&mut sub_details);

    let invoice_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let result = env
        .services()
        .compute_invoice(&invoice_date, &sub_details, None, InvoiceType::Recurring)
        .await
        .unwrap();

    let shortfall = result
        .invoice_lines
        .iter()
        .filter(|l| l.kind == LineItemKind::MinimumShortfall)
        .count();
    assert_eq!(
        shortfall, 0,
        "a usage-scoped minimum must not bill on the advance/checkout invoice (no usage yet)"
    );
    // The advance rate line is still billed as usual.
    assert!(
        result
            .invoice_lines
            .iter()
            .any(|l| l.kind == LineItemKind::Regular),
        "the advance rate component still bills on the checkout invoice"
    );
}

/// The minimum progress projects the true-up of the current period, counting what the period
/// billed on other invoices: January's $35 fee and the $30 Support charge of Jan 16 (on its own
/// invoice) against a $90 floor. February's advance fees, on the upcoming invoice, never count.
#[tokio::test]
async fn test_minimum_progress_counts_the_period_spend_of_every_invoice() {
    use meteroid_store::domain::BillingPeriodEnum;
    use meteroid_store::domain::enums::FeeTypeEnum;
    use meteroid_store::domain::price_components::{PriceEntry, PriceInput, ProductRef};
    use meteroid_store::domain::prices::{FeeStructure, Pricing};
    use meteroid_store::domain::subscription_amendment::{
        AddExtraComponent, AddOnChanges, ComponentChanges, SubscriptionAmendment,
    };
    use meteroid_store::domain::subscription_changes::PlanChangeMode;

    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;
    let subscription_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            subscription_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges {
                    edited: vec![],
                    added: vec![AddExtraComponent {
                        name: "Support".to_string(),
                        product: ProductRef::New {
                            name: "Support Product".to_string(),
                            fee_type: FeeTypeEnum::Rate,
                            fee_structure: FeeStructure::Rate {},
                        },
                        price_entry: PriceEntry::New(PriceInput {
                            cadence: BillingPeriodEnum::Monthly,
                            currency: "EUR".to_string(),
                            pricing: Pricing::Rate {
                                rate: Decimal::new(3_000, 2),
                            },
                            credit_unit: None,
                        }),
                    }],
                    removed: vec![],
                },
                add_on_changes: AddOnChanges::default(),
            },
            NaiveDate::from_ymd_opt(2024, 1, 16).unwrap(),
        )
        .await
        .expect("Support added on Jan 16");
    env.services()
        .set_subscription_minimum(
            TENANT_ID,
            subscription_id,
            9_000,
            MinimumScope::AllComponents,
        )
        .await
        .unwrap();
    let support_charged: i64 = env
        .get_invoices(subscription_id)
        .await
        .iter()
        .filter(|i| i.invoice_type == InvoiceType::Adjustment)
        .flat_map(|i| &i.line_items)
        .filter(|l| l.kind == LineItemKind::Regular)
        .map(|l| l.amount_subtotal)
        .sum();
    assert!(
        support_charged > 0,
        "January's rest of Support is charged at once"
    );

    let progress = env
        .services()
        .subscription_minimum_progress(
            TENANT_ID,
            subscription_id,
            NaiveDate::from_ymd_opt(2024, 1, 20)
                .unwrap()
                .and_hms_opt(12, 0, 0)
                .unwrap(),
        )
        .await
        .unwrap()
        .expect("a minimum applies to the current period");
    assert_eq!(progress.minimum_minor, 9_000);
    assert_eq!(progress.counted_to_date_minor, 3_500 + support_charged);
    assert_eq!(progress.projected_minor, 3_500 + support_charged);
    assert_eq!(
        progress.projected_shortfall_minor,
        9_000 - 3_500 - support_charged
    );
}

/// Case 2 — a plan whose in-scope components are ALL arrears/usage (no advance component)
/// produces no minimum on the checkout invoice (and thus no checkout invoice made solely of it).
#[tokio::test]
async fn test_minimum_skipped_when_only_arrears_components_at_checkout() {
    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;
    let mut conn = env.conn().await;

    let subscription_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let mut sub_details = env
        .store()
        .get_subscription_details_with_conn(&mut conn, TENANT_ID, subscription_id)
        .await
        .unwrap();

    // Simulate a pure-usage plan: keep only the arrears/usage component.
    sub_details
        .price_components
        .retain(|c| c.fee.metric_id().is_some());
    sub_details.add_ons.clear();
    sub_details.minimum = Some(SubscriptionMinimum {
        subscription_id,
        amount_subunit: 5_000,
        scope: MinimumScope::AllComponents,
        is_override: true,
    });
    drive_to_checkout_state(&mut sub_details);

    let invoice_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let result = env
        .services()
        .compute_invoice(&invoice_date, &sub_details, None, InvoiceType::Recurring)
        .await
        .unwrap();

    let shortfall = result
        .invoice_lines
        .iter()
        .filter(|l| l.kind == LineItemKind::MinimumShortfall)
        .count();
    assert_eq!(
        shortfall, 0,
        "no advance component in scope ⇒ no minimum on the checkout invoice"
    );
}

/// Case 3 — an ADVANCE component in scope (AllComponents floor on the Rate+usage plan) bills at
/// checkout, yet no minimum is charged there: the floor is never collected up front.
#[tokio::test]
async fn test_minimum_not_billed_at_checkout_when_advance_component_in_scope() {
    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;
    let mut conn = env.conn().await;

    let subscription_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let mut sub_details = env
        .store()
        .get_subscription_details_with_conn(&mut conn, TENANT_ID, subscription_id)
        .await
        .unwrap();

    sub_details.minimum = Some(SubscriptionMinimum {
        subscription_id,
        amount_subunit: 5_000,
        scope: MinimumScope::AllComponents,
        is_override: true,
    });
    drive_to_checkout_state(&mut sub_details);

    let invoice_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let result = env
        .services()
        .compute_invoice(&invoice_date, &sub_details, None, InvoiceType::Recurring)
        .await
        .unwrap();

    let shortfall = result
        .invoice_lines
        .iter()
        .filter(|l| l.kind == LineItemKind::MinimumShortfall)
        .count();
    assert_eq!(
        shortfall, 0,
        "the advance rate bills at checkout, but no period has completed to true up"
    );
    assert!(
        result
            .invoice_lines
            .iter()
            .any(|l| l.kind == LineItemKind::Regular),
        "the advance rate component still bills on the checkout invoice"
    );
}

/// #2 — a product-scoped minimum names the covered component on the shortfall line, so the invoice
/// (and checkout preview) shows which commitment the top-up is for, e.g. "Minimum commitment (...)".
#[tokio::test]
async fn test_product_scoped_minimum_line_names_the_product() {
    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;
    let mut conn = env.conn().await;

    let subscription_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let mut sub_details = env
        .store()
        .get_subscription_details_with_conn(&mut conn, TENANT_ID, subscription_id)
        .await
        .unwrap();

    // Scope the floor to the (single) plan component's product and set it high enough to fire.
    let component = sub_details.price_components[0].clone();
    let product_id = component.product_id.expect("component has a product");
    sub_details.minimum = Some(SubscriptionMinimum {
        subscription_id,
        amount_subunit: 9_000,
        scope: MinimumScope::Products(vec![product_id]),
        is_override: true,
    });

    // The renewal closing January, whose $35 falls short of the $90 floor.
    sub_details.subscription.cycle_index = Some(1);
    let invoice_date = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    let result = env
        .services()
        .compute_invoice(&invoice_date, &sub_details, None, InvoiceType::Recurring)
        .await
        .unwrap();

    let shortfall = result
        .invoice_lines
        .iter()
        .find(|l| l.kind == LineItemKind::MinimumShortfall)
        .expect("a shortfall line");
    let expected = format!("Minimum commitment ({})", component.name);
    assert_eq!(
        shortfall.name, expected,
        "the product-scoped minimum line should name the covered component"
    );
}

/// Under a $90 floor on the $35 LeetCode plan, adding $30 a month at once on Jan 16 is an increase:
/// it applies immediately, moves no MRR, and its charge counts toward January's true-up (Feb 1
/// bills 90 − 35 − that charge). Removing it at once is a decrease, refused until the period's end.
/// Adding $100 a month at February's end moves the MRR to $165: the preview books what apply does.
#[tokio::test]
async fn test_amendment_preview_and_apply_agree_under_a_minimum() {
    use meteroid_store::domain::BillingPeriodEnum;
    use meteroid_store::domain::enums::FeeTypeEnum;
    use meteroid_store::domain::price_components::{PriceEntry, PriceInput, ProductRef};
    use meteroid_store::domain::prices::{FeeStructure, Pricing};
    use meteroid_store::domain::subscription_amendment::{
        AddExtraComponent, AddOnChanges, ComponentChanges, SubscriptionAmendment,
    };
    use meteroid_store::domain::subscription_changes::PlanChangeMode;

    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;
    let mut conn = env.conn().await;
    diesel::insert_into(diesel_models::schema::plan_version_adjustment::table)
        .values(&PlanVersionAdjustmentRowNew {
            kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
            plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
            amount_subunit: 9_000,
            includes_all_components: true,
        })
        .execute(&mut conn)
        .await
        .unwrap();

    let subscription_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    assert_eq!(env.get_subscription(subscription_id).await.mrr_cents, 9_000);

    let adding = |name: &str, cents: i64, apply_mode: PlanChangeMode| SubscriptionAmendment {
        apply_mode,
        component_changes: ComponentChanges {
            edited: vec![],
            added: vec![AddExtraComponent {
                name: name.to_string(),
                product: ProductRef::New {
                    name: format!("{name} Product"),
                    fee_type: FeeTypeEnum::Rate,
                    fee_structure: FeeStructure::Rate {},
                },
                price_entry: PriceEntry::New(PriceInput {
                    cadence: BillingPeriodEnum::Monthly,
                    currency: "EUR".to_string(),
                    pricing: Pricing::Rate {
                        rate: Decimal::new(cents, 2),
                    },
                    credit_unit: None,
                }),
            }],
            removed: vec![],
        },
        add_on_changes: AddOnChanges::default(),
    };

    let jan1 = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let feb1 = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    let previewed = env
        .services()
        .preview_amendment(
            subscription_id,
            TENANT_ID,
            adding("Support", 3_000, PlanChangeMode::Immediate),
        )
        .await
        .expect("an immediate increase is previewed under a minimum");
    env.services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            subscription_id,
            TENANT_ID,
            adding("Support", 3_000, PlanChangeMode::Immediate),
            NaiveDate::from_ymd_opt(2024, 1, 16).unwrap(),
        )
        .await
        .expect("an immediate increase applies under a minimum");
    let mrr = env.get_subscription(subscription_id).await.mrr_cents as i64;
    assert_eq!(mrr, 9_000, "$65 a month stays under the $90 floor");
    assert_eq!(
        previewed.mrr_after_cents, mrr,
        "Support: the preview books what apply does"
    );

    let support = env
        .get_subscription_components(subscription_id)
        .await
        .into_iter()
        .find(|c| c.name == "Support")
        .expect("Support component");
    let removal = |apply_mode| SubscriptionAmendment {
        apply_mode,
        component_changes: ComponentChanges {
            edited: vec![],
            added: vec![],
            removed: vec![support.id],
        },
        add_on_changes: AddOnChanges::default(),
    };
    let refused = env
        .services()
        .preview_amendment(
            subscription_id,
            TENANT_ID,
            removal(PlanChangeMode::Immediate),
        )
        .await
        .expect_err("an immediate removal is refused under a minimum");
    assert!(format!("{refused:?}").contains("minimum commitment"));
    let refused = env
        .services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            subscription_id,
            TENANT_ID,
            removal(PlanChangeMode::Immediate),
            NaiveDate::from_ymd_opt(2024, 1, 20).unwrap(),
        )
        .await
        .expect_err("an immediate removal is refused under a minimum");
    assert!(format!("{refused:?}").contains("minimum commitment"));

    env.process_cycles().await;
    let invoices = env.get_invoices(subscription_id).await;
    let support_charged: i64 = invoices
        .iter()
        .filter(|i| i.invoice_type == InvoiceType::Adjustment)
        .flat_map(|i| &i.line_items)
        .filter(|l| l.kind == LineItemKind::Regular)
        .map(|l| l.amount_subtotal)
        .sum();
    assert!(
        support_charged > 0,
        "January's rest of Support is charged at once"
    );
    let renewal = invoices
        .iter()
        .find(|i| i.invoice_type == InvoiceType::Recurring && i.billing_period_start == feb1)
        .expect("February's renewal");
    let true_up: Vec<_> = renewal
        .line_items
        .iter()
        .filter(|l| l.kind == LineItemKind::MinimumShortfall)
        .map(|l| (l.amount_subtotal, l.start_date, l.end_date))
        .collect();
    assert_eq!(
        true_up,
        vec![(9_000 - 3_500 - support_charged, jan1, feb1)],
        "January is trued up net of its fee and of the Support charged mid-period"
    );

    for (name, cents, expected, boundary) in [(
        "Premium",
        10_000,
        16_500,
        NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
    )] {
        let preview = env
            .services()
            .preview_amendment(
                subscription_id,
                TENANT_ID,
                adding(name, cents, PlanChangeMode::EndOfPeriod),
            )
            .await
            .expect("preview_amendment");
        let scheduled = env
            .services()
            .schedule_amendment(
                common_domain::actor::Actor::System,
                subscription_id,
                TENANT_ID,
                adding(name, cents, PlanChangeMode::EndOfPeriod),
            )
            .await
            .expect("schedule_amendment");
        assert_eq!(
            scheduled.scheduled_time.date(),
            boundary,
            "{name}: at period end"
        );

        env.process_cycles().await;
        let subscription = env.get_subscription(subscription_id).await;
        assert_eq!(
            subscription.current_period_start, boundary,
            "{name}: renewed"
        );
        let applied = subscription.mrr_cents as i64;
        assert_eq!(applied, expected, "{name}: applied MRR");
        assert_eq!(
            preview.mrr_after_cents, applied,
            "{name}: the preview books what apply does"
        );
    }
}
