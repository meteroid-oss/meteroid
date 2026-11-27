//! The level a usage-levels charge starts on, or lands on at a portal plan change (R-135): the
//! one picked, of its cell only; else the lowest for a new subscription, and for a plan change the
//! one held when the new plan sells it, else the next one up. Volume: €199 for €50K managed
//! revenue, €479 for €125K, €899 for €250K; Growth: €899 for €250K, €1,499 for €1M.
use rstest::rstest;
use rust_decimal::Decimal;
use tonic::Code;

use crate::data::ids::*;
use crate::data::plans::{PlanSeed, SeedComp, SeedPrice};
use crate::harness::{TestEnv, subscription, test_env};
use common_domain::actor::Actor;
use common_domain::ids::{
    BaseId, PlanId, PlanVersionId, PriceComponentId, PriceId, SubscriptionId,
};
use diesel_models::enums::BillingPeriodEnum as DieselBillingPeriodEnum;
use meteroid_grpc::meteroid::portal::subscription::v1 as portal;
use meteroid_store::domain::checkout_sessions::CheckoutType;
use meteroid_store::domain::prices::Pricing;
use meteroid_store::domain::product_family_journeys::ProductFamilyJourney;
use meteroid_store::domain::usage_levels::ChosenLevel;
use meteroid_store::domain::{
    ComponentParameterization, ComponentParameters, CreateCheckoutSession,
    CreateSubscriptionComponents, SubscriptionFee,
};
use meteroid_store::repositories::ProductFamilyJourneysInterface;
use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;
use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;

use super::portal_plan_change::{client, rank, request};

/// A plan of `name` selling the managed-revenue charge at `levels` (units, monthly rate).
struct LevelsPlan {
    plan: PlanId,
    version: PlanVersionId,
    component: PriceComponentId,
}

async fn levels_plan(
    env: &TestEnv,
    name: &'static str,
    levels: &[(u64, i64)],
    trial_days: Option<i32>,
) -> LevelsPlan {
    let seeded = LevelsPlan {
        plan: PlanId::new(),
        version: PlanVersionId::new(),
        component: PriceComponentId::new(),
    };
    let (first_units, first_rate) = levels[0];
    let mut charge = SeedComp::capacity(
        seeded.component,
        "Managed revenue",
        PRODUCT_CAPACITY_ID,
        METRIC_BANDWIDTH,
        PriceId::new(),
        DieselBillingPeriodEnum::Monthly,
        Decimal::from(first_rate),
        first_units,
        Decimal::new(5, 3),
    );
    for (included, rate) in &levels[1..] {
        charge.prices.push(SeedPrice {
            id: PriceId::new(),
            cadence: DieselBillingPeriodEnum::Monthly,
            pricing: Pricing::Capacity {
                rate: Decimal::from(*rate),
                included: *included,
                overage_rate: Decimal::new(5, 3),
            },
        });
    }
    let mut plan = PlanSeed::new(seeded.plan, name, seeded.version).components(vec![charge]);
    if let Some(days) = trial_days {
        plan = plan.trial(days, seeded.plan, true);
    }
    plan.seed(&mut env.conn().await)
        .await
        .expect("a plan with usage levels");
    seeded
}

const VOLUME: &[(u64, i64)] = &[(50_000, 199), (125_000, 479), (250_000, 899)];
const GROWTH: &[(u64, i64)] = &[(250_000, 899), (1_000_000, 1_499)];

/// The units each usage-levels charge of the subscription includes now.
async fn included(env: &TestEnv, sub_id: SubscriptionId) -> Vec<u64> {
    env.store()
        .get_subscription_details(TENANT_ID, sub_id)
        .await
        .expect("details")
        .price_components
        .iter()
        .filter_map(|c| match c.fee {
            SubscriptionFee::Capacity { included, .. } => Some(included),
            _ => None,
        })
        .collect()
}

fn committed(params: &[ComponentParameterization]) -> Vec<Option<u64>> {
    params
        .iter()
        .map(|p| p.parameters.committed_capacity)
        .collect()
}

async fn signup_plan(env: &TestEnv, plan: PlanId) {
    env.store()
        .save_product_family_journey(
            TENANT_ID,
            ProductFamilyJourney {
                signup_plan_id: Some(plan),
                ..ProductFamilyJourney::default_for(PRODUCT_FAMILY_ID)
            },
        )
        .await
        .expect("the sign-up plan");
}

/// A sign-up starts on the level picked, else the lowest, the one a pricing page reads the plan's
/// price from; a level the plan does not sell is refused before anything is created.
/// Rules: R-135
#[rstest]
#[tokio::test]
async fn a_signup_starts_on_the_level_picked_else_the_lowest(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let volume = levels_plan(&env, "Volume", VOLUME, Some(14)).await;
    signup_plan(&env, volume.plan).await;

    let refused = env
        .services()
        .signup_offer_at_levels(
            TENANT_ID,
            None,
            "EUR",
            &[ChosenLevel {
                product_id: None,
                included: 1_000_000,
            }],
        )
        .await
        .expect_err("not a level of the plan");
    let refused = format!("{refused:?}");
    assert!(
        refused.contains("1000000 is not a level of Managed revenue"),
        "{refused}"
    );
    let refused = env
        .services()
        .signup_offer_at_levels(
            TENANT_ID,
            None,
            "EUR",
            &[ChosenLevel {
                product_id: Some(PRODUCT_PLATFORM_FEE_ID),
                included: 125_000,
            }],
        )
        .await
        .expect_err("no usage levels for that product");
    assert!(format!("{refused:?}").contains("no usage levels"));

    let picked = env
        .services()
        .signup_offer_at_levels(
            TENANT_ID,
            None,
            "EUR",
            &[ChosenLevel {
                product_id: Some(PRODUCT_CAPACITY_ID),
                included: 125_000,
            }],
        )
        .await
        .expect("a level of the plan");
    let created = env
        .services()
        .subscribe_to_signup_plan(Actor::System, TENANT_ID, CUST_UBER_ID, picked)
        .await
        .expect("subscribed on the level picked");
    assert_eq!(included(&env, created.id).await, vec![125_000]);

    // None picked: the lowest, where it used to be refused as ambiguous.
    let offer = env
        .services()
        .signup_offer(TENANT_ID, None, "EUR")
        .await
        .expect("the sign-up offer");
    assert_eq!(committed(&offer.levels), vec![Some(50_000)]);
    let created = env
        .services()
        .subscribe_to_signup_plan(Actor::System, TENANT_ID, CUST_SPOTIFY_ID, offer)
        .await
        .expect("subscribed on the lowest level");
    assert_eq!(included(&env, created.id).await, vec![50_000]);
}

fn self_serve_checkout(
    version: PlanVersionId,
    components: Option<CreateSubscriptionComponents>,
) -> CreateCheckoutSession {
    CreateCheckoutSession {
        tenant_id: TENANT_ID,
        customer_id: CUST_UBER_ID,
        plan_version_id: version,
        option: None,
        billing_start_date: None,
        billing_day_anchor: None,
        net_terms: None,
        trial: Default::default(),
        end_date: None,
        auto_advance_invoices: true,
        charge_automatically: true,
        invoice_memo: None,
        invoice_threshold: None,
        purchase_order: None,
        payment_methods_config: None,
        components,
        add_ons: None,
        coupon_code: None,
        coupon_ids: vec![],
        expires_in_hours: Some(24),
        metadata: None,
        checkout_type: CheckoutType::SelfServe,
        subscription_id: None,
        change_date: None,
        success_url: None,
        cancel_url: None,
        resource_id: None,
    }
}

fn at_level(component: PriceComponentId, included: u64) -> CreateSubscriptionComponents {
    CreateSubscriptionComponents {
        parameterized_components: vec![ComponentParameterization {
            component_id: component,
            parameters: ComponentParameters {
                initial_slot_count: None,
                billing_period: None,
                committed_capacity: Some(included),
            },
        }],
        overridden_components: vec![],
        extra_components: vec![],
        remove_components: vec![],
    }
}

/// A checkout keeps the level it asks, else the lowest, and its page previews that level; one the
/// plan does not sell is refused when the checkout is created.
/// Rules: R-135
#[rstest]
#[tokio::test]
async fn a_checkout_bills_the_level_asked_else_the_lowest(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let volume = levels_plan(&env, "Volume", VOLUME, None).await;

    let unasked = env
        .store()
        .create_checkout_session(self_serve_checkout(volume.version, None))
        .await
        .expect("a checkout without a level");
    assert_eq!(committed(&unasked.component_params()), vec![Some(50_000)]);
    let previewed = env
        .services()
        .build_preview_subscription_details(&unasked, TENANT_ID, None)
        .await
        .expect("its page previews it");
    assert!(previewed.price_components.iter().any(|c| matches!(
        c.fee,
        SubscriptionFee::Capacity {
            included: 50_000,
            ..
        }
    )));

    let asked = env
        .store()
        .create_checkout_session(self_serve_checkout(
            volume.version,
            Some(at_level(volume.component, 250_000)),
        ))
        .await
        .expect("a checkout at a level of the plan");
    assert_eq!(committed(&asked.component_params()), vec![Some(250_000)]);

    let refused = env
        .store()
        .create_checkout_session(self_serve_checkout(
            volume.version,
            Some(at_level(volume.component, 60_000)),
        ))
        .await
        .expect_err("not a level of the plan");
    assert!(
        format!("{refused:?}").contains("60000 is not a level of Managed revenue"),
        "{refused:?}"
    );
}

fn plan_change_levels(response: &portal::PreviewPlanChangeResponse) -> Vec<(u64, bool)> {
    let [charge] = response.levels.as_slice() else {
        panic!("one usage-levels charge to choose: {:?}", response.levels);
    };
    charge
        .levels
        .iter()
        .map(|l| (l.included, l.current))
        .collect()
}

async fn preview(
    env: &TestEnv,
    sub_id: SubscriptionId,
    growth: &LevelsPlan,
    levels: Vec<portal::LevelChoice>,
) -> Result<portal::PreviewPlanChangeResponse, tonic::Status> {
    client(env)
        .preview_plan_change(request(
            env,
            portal::PreviewPlanChangeRequest {
                subscription_id: sub_id.to_string(),
                new_plan_version_id: growth.version.to_string(),
                option: None,
                levels,
            },
        ))
        .await
        .map(tonic::Response::into_inner)
}

async fn confirm(
    env: &TestEnv,
    sub_id: SubscriptionId,
    growth: &LevelsPlan,
    levels: Vec<portal::LevelChoice>,
) -> Result<portal::ConfirmPlanChangeResponse, tonic::Status> {
    client(env)
        .confirm_plan_change(request(
            env,
            portal::ConfirmPlanChangeRequest {
                subscription_id: sub_id.to_string(),
                new_plan_version_id: growth.version.to_string(),
                option: None,
                levels,
            },
        ))
        .await
        .map(tonic::Response::into_inner)
}

async fn on_volume(
    env: &TestEnv,
    volume: &LevelsPlan,
    included: u64,
    external_payment: bool,
) -> SubscriptionId {
    let mut create = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(volume.version)
        .start_date(chrono::Utc::now().date_naive())
        .on_start()
        .no_trial();
    if external_payment {
        create = create.external_payment();
    }
    let mut create = create.request();
    create.price_components = Some(at_level(volume.component, included));
    env.services()
        .insert_subscription(Actor::User { id: USER_ID }, create, TENANT_ID)
        .await
        .expect("a Volume subscription")
        .id
}

/// From €125K on Volume to Growth, which has no €125K level: the change lands on the next level
/// up unless the customer picks another of Growth's, where it was refused as ambiguous. A paid
/// change keeps the level picked on its checkout.
/// Rules: R-135
#[rstest]
#[tokio::test]
async fn a_portal_plan_change_lands_on_the_level_picked_else_the_next_up(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let volume = levels_plan(&env, "Volume", VOLUME, None).await;
    let growth = levels_plan(&env, "Growth", GROWTH, None).await;
    rank(&env, volume.plan, Some(1)).await;
    rank(&env, growth.plan, Some(2)).await;
    let sub_id = on_volume(&env, &volume, 125_000, true).await;

    let landed = preview(&env, sub_id, &growth, vec![])
        .await
        .expect("a change without a pick");
    assert_eq!(
        plan_change_levels(&landed),
        vec![(250_000, true), (1_000_000, false)]
    );
    assert_eq!(landed.levels[0].held, Some(125_000));
    assert_eq!(landed.levels[0].component_id, growth.component.to_string());

    let pick = |included: u64| {
        vec![portal::LevelChoice {
            component_id: growth.component.to_string(),
            included,
        }]
    };
    let picked = preview(&env, sub_id, &growth, pick(1_000_000))
        .await
        .expect("a change at the level picked");
    assert_eq!(
        plan_change_levels(&picked),
        vec![(250_000, false), (1_000_000, true)]
    );

    let refused = preview(&env, sub_id, &growth, pick(125_000))
        .await
        .expect_err("Growth has no €125K level");
    assert_eq!(refused.code(), Code::InvalidArgument, "{refused:?}");

    let applied = confirm(&env, sub_id, &growth, pick(1_000_000))
        .await
        .expect("an upgrade, applied at once without online payment");
    assert_eq!(
        applied.status,
        portal::PlanChangeStatus::PlanChangeCompleted as i32
    );
    assert_eq!(included(&env, sub_id).await, vec![1_000_000]);

    // Paid online: the checkout the change waits on keeps the level picked.
    let paid = on_volume(&env, &volume, 125_000, false).await;
    let checkout = confirm(&env, paid, &growth, pick(1_000_000))
        .await
        .expect("an upgrade through checkout");
    assert_eq!(
        checkout.status,
        portal::PlanChangeStatus::PlanChangeCheckoutRequired as i32
    );
    let session = env
        .store()
        .list_checkout_sessions(TENANT_ID, Some(CUST_UBER_ID), None)
        .await
        .expect("checkouts")
        .into_iter()
        .find(|s| s.subscription_id == Some(paid) && s.checkout_type == CheckoutType::PlanChange)
        .expect("the plan change checkout");
    assert_eq!(
        committed(&session.component_params()),
        vec![Some(1_000_000)]
    );
    let today = chrono::Utc::now().date_naive();
    env.services()
        .compute_plan_change_checkout_invoice_with(
            paid,
            TENANT_ID,
            growth.version,
            &session.component_params(),
            today,
        )
        .await
        .expect("its page prices the level picked");
    let ambiguous = env
        .services()
        .compute_plan_change_checkout_invoice(paid, TENANT_ID, growth.version, today)
        .await
        .expect_err("without the level, Growth has two to land on");
    assert!(
        format!("{ambiguous:?}").contains("Multiple prices match"),
        "{ambiguous:?}"
    );
}
