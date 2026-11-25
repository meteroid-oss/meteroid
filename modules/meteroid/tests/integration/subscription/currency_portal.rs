//! The portal offers the ladder priced in the subscription's currency, by plan option: their own
//! option unless they may switch.
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, PlanId, PlanVersionId, PriceId, SubscriptionId};
use diesel_models::enums::BillingPeriodEnum as DieselBillingPeriodEnum;
use meteroid_grpc::meteroid::api::subscriptions::v1::SubscriptionFeeBillingPeriod;
use meteroid_grpc::meteroid::portal::subscription::v1 as portal;
use meteroid_store::domain::prices::Pricing;
use meteroid_store::domain::product_family_journeys::ProductFamilyJourney;
use meteroid_store::repositories::ProductFamilyJourneysInterface;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tonic::Code;

use super::portal_plan_change::{client, rank, request};
use crate::data::ids::*;
use crate::data::plans::{PlanSeed, SeedComp, SeedPrice};
use crate::harness::{TestEnv, subscription, test_env};

pub(crate) fn rate(id: PriceId, cadence: DieselBillingPeriodEnum, rate: Decimal) -> SeedPrice {
    SeedPrice {
        id,
        cadence,
        pricing: Pricing::Rate { rate },
    }
}

/// A ranked plan of the product line, its platform fee priced in `currency` on each of `prices`:
/// several are sold as an option per cadence, named after it.
pub(crate) async fn plan(
    env: &TestEnv,
    name: &'static str,
    ladder_rank: i32,
    currency: &'static str,
    prices: Vec<SeedPrice>,
) -> PlanVersionId {
    let (plan_id, version) = (PlanId::new(), PlanVersionId::new());
    let mut platform = SeedComp::rate(
        common_domain::ids::PriceComponentId::new(),
        "Platform",
        PRODUCT_PLATFORM_FEE_ID,
        prices[0].id,
        prices[0].cadence.clone(),
        dec!(1),
    );
    let options: Vec<(&'static str, DieselBillingPeriodEnum)> = if prices.len() > 1 {
        prices
            .iter()
            .map(|p| match p.cadence {
                DieselBillingPeriodEnum::Annual => ("Annual", p.cadence.clone()),
                _ => ("Monthly", p.cadence.clone()),
            })
            .collect()
    } else {
        vec![]
    };
    platform.prices = prices;
    PlanSeed::new(plan_id, name, version)
        .currency(currency)
        .options(&options)
        .components(vec![platform])
        .seed(&mut env.conn().await)
        .await
        .expect("plan");
    rank(env, plan_id, Some(ladder_rank)).await;
    version
}

pub(crate) async fn journey(env: &TestEnv, period_switch: bool) {
    env.store()
        .save_product_family_journey(
            TENANT_ID,
            ProductFamilyJourney {
                period_switch,
                self_serve_downgrade: false,
                ..ProductFamilyJourney::default_for(PRODUCT_FAMILY_ID)
            },
        )
        .await
        .expect("journey");
}

/// A subscription to `version`'s default option.
pub(crate) async fn monthly_subscription(env: &TestEnv, version: PlanVersionId) -> SubscriptionId {
    let create = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(version)
        // Today: the portal changes a plan within the current period.
        .start_date(chrono::Utc::now().date_naive())
        .on_start()
        .no_trial()
        .request();
    env.services()
        .insert_subscription(Actor::User { id: USER_ID }, create, TENANT_ID)
        .await
        .expect("a monthly subscription")
        .id
}

pub(crate) async fn list(
    env: &TestEnv,
    sub: SubscriptionId,
) -> Result<portal::ListAvailablePlansResponse, tonic::Status> {
    client(env)
        .list_available_plans(request(
            env,
            portal::ListAvailablePlansRequest {
                subscription_id: sub.to_string(),
            },
        ))
        .await
        .map(tonic::Response::into_inner)
}

/// Each plan offered, with its options by name and whether each is the subscription's.
pub(crate) fn offered(
    response: &portal::ListAvailablePlansResponse,
) -> Vec<(String, Vec<(String, bool)>)> {
    response
        .plans
        .iter()
        .map(|plan| {
            (
                plan.plan_name.clone(),
                plan.options
                    .iter()
                    .map(|v| (v.name.clone(), v.is_current))
                    .collect(),
            )
        })
        .collect()
}

fn annual() -> Option<String> {
    Some("annual".to_string())
}

/// A customer on Pro Monthly in EUR sees only the plans priced in EUR, in their option unless they
/// may switch it; a plan priced in USD only is never offered to them.
#[tokio::test]
async fn the_portal_offers_only_plans_priced_in_the_customers_currency() {
    let env = test_env().await;
    let pro = plan(
        &env,
        "Pro",
        1,
        "EUR",
        vec![
            rate(PriceId::new(), DieselBillingPeriodEnum::Monthly, dec!(20)),
            rate(PriceId::new(), DieselBillingPeriodEnum::Annual, dec!(200)),
        ],
    )
    .await;
    let team = plan(
        &env,
        "Team",
        2,
        "USD",
        vec![rate(
            PriceId::new(),
            DieselBillingPeriodEnum::Monthly,
            dec!(50),
        )],
    )
    .await;
    journey(&env, false).await;
    let sub = monthly_subscription(&env, pro).await;

    let response = list(&env, sub).await.expect("the plans");
    assert_eq!(
        offered(&response),
        vec![("Pro".to_string(), vec![("Monthly".to_string(), true)])],
        "their option only, never a plan unpriced in their currency"
    );

    journey(&env, true).await;
    let response = list(&env, sub).await.expect("the plans");
    assert_eq!(
        offered(&response),
        vec![(
            "Pro".to_string(),
            vec![("Monthly".to_string(), true), ("Annual".to_string(), false)]
        )],
        "every option priced in their currency"
    );
    let offered_annual = response.plans[0]
        .options
        .iter()
        .find(|v| v.alias == "annual")
        .expect("the annual option");
    assert_eq!(offered_annual.currency, "EUR");

    let confirm = |version: PlanVersionId, option| {
        let env = &env;
        async move {
            client(env)
                .confirm_plan_change(request(
                    env,
                    portal::ConfirmPlanChangeRequest {
                        subscription_id: sub.to_string(),
                        new_plan_version_id: version.to_string(),
                        option,
                        levels: vec![],
                    },
                ))
                .await
        }
    };
    for version in [pro, team] {
        let refused = confirm(version, None)
            .await
            .expect_err("not offered to them");
        assert_eq!(refused.code(), Code::InvalidArgument, "{refused:?}");
    }

    client(&env)
        .preview_plan_change(request(
            &env,
            portal::PreviewPlanChangeRequest {
                subscription_id: sub.to_string(),
                new_plan_version_id: pro.to_string(),
                option: annual(),
                levels: vec![],
            },
        ))
        .await
        .expect("the annual option of their plan");
    confirm(pro, annual())
        .await
        .expect("a switch to the annual option: never a downgrade to arrange");
}

/// A plan of one option billing seats monthly and its platform fee yearly is offered in it, and a
/// move to it bills the platform fee yearly.
#[tokio::test]
async fn a_plan_of_fees_on_their_own_cadences_is_offered() {
    let env = test_env().await;
    let pro = plan(
        &env,
        "Pro",
        1,
        "EUR",
        vec![rate(
            PriceId::new(),
            DieselBillingPeriodEnum::Monthly,
            dec!(20),
        )],
    )
    .await;
    let (scale_id, scale) = (PlanId::new(), PlanVersionId::new());
    PlanSeed::new(scale_id, "Scale", scale)
        .components(vec![
            SeedComp::slot(
                common_domain::ids::PriceComponentId::new(),
                "Seats",
                PRODUCT_SEATS_ID,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                dec!(10),
            ),
            SeedComp::rate(
                common_domain::ids::PriceComponentId::new(),
                "Platform",
                PRODUCT_PLATFORM_FEE_ID,
                PriceId::new(),
                DieselBillingPeriodEnum::Annual,
                dec!(1200),
            ),
        ])
        .seed(&mut env.conn().await)
        .await
        .expect("plan");
    rank(&env, scale_id, Some(2)).await;
    journey(&env, false).await;
    let sub = monthly_subscription(&env, pro).await;

    let response = list(&env, sub).await.expect("the plans");
    let offered_scale = response
        .plans
        .iter()
        .find(|p| p.plan_name == "Scale")
        .expect("Scale is offered");
    assert_eq!(
        offered_scale
            .options
            .iter()
            .map(|v| (v.name.clone(), v.is_current))
            .collect::<Vec<_>>(),
        vec![("Standard".to_string(), false)],
        "one option: nothing to choose"
    );

    let preview = client(&env)
        .preview_plan_change(request(
            &env,
            portal::PreviewPlanChangeRequest {
                subscription_id: sub.to_string(),
                new_plan_version_id: scale.to_string(),
                option: None,
                levels: vec![],
            },
        ))
        .await
        .expect("a move to Scale")
        .into_inner();
    let changes = preview.preview.expect("the preview").component_changes;
    let platform = changes
        .iter()
        .find(|c| c.component_name == "Platform")
        .and_then(|c| c.new_fee.as_ref())
        .expect("the platform fee");
    assert_eq!(
        platform.cadence,
        i32::from(SubscriptionFeeBillingPeriod::Yearly),
        "the platform fee stays yearly"
    );
    assert_eq!(platform.amount.parse::<Decimal>().ok(), Some(dec!(1200)));
}
