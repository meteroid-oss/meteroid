//! A customer changes plan in the portal only to a plan the portal offers: the active version of a
//! ranked plan of the same family and currency, from a self-servable subscription.
use rstest::rstest;
use tonic::Code;

use crate::data::ids::*;
use crate::harness::{TestEnv, subscription, test_env};
use common_domain::actor::Actor;
use common_domain::ids::{PlanId, PlanVersionId, SubscriptionId};
use meteroid_grpc::meteroid::portal::subscription::v1 as portal;
use meteroid_store::domain::PlanPatch;
use meteroid_store::repositories::{PlansInterface, PricingPagesInterface};

pub(crate) async fn rank(env: &TestEnv, plan_id: PlanId, rank: Option<i32>) {
    env.store()
        .patch_published_plan(
            Actor::User { id: USER_ID },
            PlanPatch {
                id: plan_id,
                tenant_id: TENANT_ID,
                name: None,
                description: None,
                active_version_id: None,
                self_service_rank: Some(rank),
                issues_license: None,
            },
        )
        .await
        .expect("rank plan");
}

pub(crate) fn request<T>(env: &TestEnv, message: T) -> tonic::Request<T> {
    let token = meteroid_store::jwt_claims::generate_portal_token(
        &env.setup.config.jwt_secret,
        TENANT_ID,
        meteroid_store::jwt_claims::ResourceAccess::Customer(CUST_UBER_ID),
        None,
        None,
        false,
    )
    .expect("portal token");
    let mut request = tonic::Request::new(message);
    request
        .metadata_mut()
        .insert("x-portal-token", token.parse().expect("token header"));
    request
}

pub(crate) fn client(
    env: &TestEnv,
) -> portal::portal_subscription_service_client::PortalSubscriptionServiceClient<
    tonic::transport::Channel,
> {
    portal::portal_subscription_service_client::PortalSubscriptionServiceClient::new(
        env.setup.channel.clone(),
    )
}

async fn preview(
    env: &TestEnv,
    sub_id: SubscriptionId,
    target: PlanVersionId,
) -> Result<portal::PreviewPlanChangeResponse, tonic::Status> {
    client(env)
        .preview_plan_change(request(
            env,
            portal::PreviewPlanChangeRequest {
                option: None,
                subscription_id: sub_id.to_string(),
                new_plan_version_id: target.to_string(),
                levels: vec![],
            },
        ))
        .await
        .map(tonic::Response::into_inner)
}

async fn confirm(
    env: &TestEnv,
    sub_id: SubscriptionId,
    target: PlanVersionId,
) -> Result<portal::ConfirmPlanChangeResponse, tonic::Status> {
    client(env)
        .confirm_plan_change(request(
            env,
            portal::ConfirmPlanChangeRequest {
                option: None,
                subscription_id: sub_id.to_string(),
                new_plan_version_id: target.to_string(),
                levels: vec![],
            },
        ))
        .await
        .map(tonic::Response::into_inner)
}

pub(crate) async fn available(env: &TestEnv, sub_id: SubscriptionId) -> Vec<portal::AvailablePlan> {
    client(env)
        .list_available_plans(request(
            env,
            portal::ListAvailablePlansRequest {
                subscription_id: sub_id.to_string(),
            },
        ))
        .await
        .expect("available plans")
        .into_inner()
        .plans
}

async fn can_change_plan(env: &TestEnv, sub_id: SubscriptionId) -> bool {
    client(env)
        .get_subscription_details(request(
            env,
            portal::GetSubscriptionDetailsRequest {
                subscription_id: sub_id.to_string(),
            },
        ))
        .await
        .expect("subscription details")
        .into_inner()
        .subscription
        .expect("subscription")
        .can_change_plan
}

#[rstest]
#[tokio::test]
async fn portal_plan_change_is_limited_to_the_offered_plans(#[future] test_env: TestEnv) {
    let env = test_env.await;
    rank(&env, PLAN_STARTER_ID, Some(1)).await;
    rank(&env, PLAN_PRO_ID, Some(2)).await;

    let sub_id = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(chrono::Utc::now().date_naive())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    assert!(can_change_plan(&env, sub_id).await);

    preview(&env, sub_id, PLAN_VERSION_PRO_ID)
        .await
        .expect("a ranked plan of the family is offered");

    // Same family and currency, but unranked (hidden from the portal), or not the active version.
    for target in [PLAN_VERSION_NOTION_ID, PLAN_VERSION_PRO_DRAFT_ID] {
        let refused = preview(&env, sub_id, target)
            .await
            .expect_err("an unoffered plan cannot be previewed");
        assert_eq!(refused.code(), Code::InvalidArgument, "{refused:?}");
        let refused = confirm(&env, sub_id, target)
            .await
            .expect_err("an unoffered plan cannot be confirmed");
        assert_eq!(refused.code(), Code::InvalidArgument, "{refused:?}");
    }
    assert!(
        env.get_subscription(sub_id).await.plan_version_id == PLAN_VERSION_STARTER_ID,
        "a refused change leaves the subscription on its plan"
    );
    assert!(
        available(&env, sub_id).await.iter().any(|p| !p.is_current),
        "a self-servable subscription is offered the ranked plans"
    );

    // A scheduled plan change is cancelled before another is chosen: nothing else is offered.
    let pending = env
        .services()
        .schedule_plan_change(
            Actor::User { id: USER_ID },
            sub_id,
            TENANT_ID,
            PLAN_VERSION_PRO_ID,
            vec![],
        )
        .await
        .expect("scheduled change");
    assert!(!can_change_plan(&env, sub_id).await);
    assert!(available(&env, sub_id).await.iter().all(|p| p.is_current));
    let refused = preview(&env, sub_id, PLAN_VERSION_PRO_ID)
        .await
        .expect_err("a pending change blocks another");
    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused:?}");
    assert!(
        refused.message().contains("already scheduled"),
        "{refused:?}"
    );
    env.services()
        .cancel_scheduled_event(Actor::User { id: USER_ID }, pending.id, sub_id, TENANT_ID)
        .await
        .expect("cancel the scheduled change");

    // A sales-led plan (no rank) is managed by the account team, even towards a ranked plan.
    rank(&env, PLAN_STARTER_ID, None).await;
    assert!(!can_change_plan(&env, sub_id).await);
    let refused = preview(&env, sub_id, PLAN_VERSION_PRO_ID)
        .await
        .expect_err("a sales-led subscription cannot self-serve a plan change");
    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused:?}");
    let refused = confirm(&env, sub_id, PLAN_VERSION_PRO_ID)
        .await
        .expect_err("a sales-led subscription cannot self-serve a plan change");
    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused:?}");
}

async fn sell_by_sales(env: &TestEnv, plan_id: PlanId, sales_led: bool) {
    use meteroid_store::domain::pricing_pages::PlanPricingPage;
    env.store()
        .update_plan_pricing_page(
            TENANT_ID,
            PlanPricingPage {
                plan_id,
                tagline: None,
                popular: false,
                cta_label: None,
                highlights: vec![],
                sales_led,
                hide_price: false,
                contact_url: Some("https://acme.test/contact".to_string()),
                product_order: vec![],
                hidden_product_ids: vec![],
            },
        )
        .await
        .expect("how the plan is sold");
}

/// A sales-led plan stays on the pricing page but is never offered in the portal: not as an
/// upgrade, and a subscription to one is managed by the account team.
#[rstest]
#[tokio::test]
async fn portal_never_offers_a_sales_led_plan(#[future] test_env: TestEnv) {
    let env = test_env.await;
    rank(&env, PLAN_STARTER_ID, Some(1)).await;
    rank(&env, PLAN_PRO_ID, Some(2)).await;
    sell_by_sales(&env, PLAN_PRO_ID, true).await;

    let sub_id = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(chrono::Utc::now().date_naive())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let offered = available(&env, sub_id).await;
    assert!(
        offered.iter().all(|p| p.plan_id != PLAN_PRO_ID.as_proto()),
        "a sales-led plan is not an upgrade: {offered:?}"
    );
    let refused = preview(&env, sub_id, PLAN_VERSION_PRO_ID)
        .await
        .expect_err("a sales-led plan cannot be previewed");
    assert_eq!(refused.code(), Code::InvalidArgument, "{refused:?}");
    let refused = confirm(&env, sub_id, PLAN_VERSION_PRO_ID)
        .await
        .expect_err("a sales-led plan cannot be confirmed");
    assert_eq!(refused.code(), Code::InvalidArgument, "{refused:?}");

    let ladder = env
        .store()
        .pricing_page(
            TENANT_ID,
            PRODUCT_FAMILY_ID,
            meteroid_store::domain::pricing_pages::PricingPageRequest::default(),
        )
        .await
        .expect("the pricing page");
    assert!(
        ladder
            .cards
            .iter()
            .any(|card| card.plan_id == PLAN_PRO_ID && card.sales_led),
        "the pricing page keeps it"
    );

    // Sold self-serve again, it is offered.
    sell_by_sales(&env, PLAN_PRO_ID, false).await;
    preview(&env, sub_id, PLAN_VERSION_PRO_ID)
        .await
        .expect("a self-serve plan is offered");

    // A customer on a sales-led plan is managed by the account team.
    sell_by_sales(&env, PLAN_STARTER_ID, true).await;
    assert!(!can_change_plan(&env, sub_id).await);
    let refused = preview(&env, sub_id, PLAN_VERSION_PRO_ID)
        .await
        .expect_err("a sales-led subscription cannot self-serve a plan change");
    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused:?}");
}

/// RF-37: two subscriptions to one plan are told apart in the portal by their usage filters.
#[rstest]
#[tokio::test]
async fn portal_lists_each_subscriptions_usage_filter(#[future] test_env: TestEnv) {
    use meteroid_grpc::meteroid::portal::customer::v1 as customer;
    use meteroid_store::domain::UsageFilter;

    let env = test_env.await;
    let mut create = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(chrono::Utc::now().date_naive())
        .on_start()
        .no_trial()
        .request();
    create.subscription.usage_filter =
        Some(UsageFilter::new("team".into(), "mobile".into()).expect("filter"));
    let filtered = env
        .services()
        .insert_subscription(Actor::User { id: USER_ID }, create, TENANT_ID)
        .await
        .expect("filtered subscription")
        .id;

    let overview = customer::portal_customer_service_client::PortalCustomerServiceClient::new(
        env.setup.channel.clone(),
    )
    .get_customer_portal_overview(request(&env, customer::GetCustomerPortalOverviewRequest {}))
    .await
    .expect("portal overview")
    .into_inner()
    .overview
    .expect("overview");
    let summary = overview
        .active_subscriptions
        .iter()
        .find(|s| s.id == filtered.to_string())
        .expect("the filtered subscription is listed");
    let filter = summary.usage_filter.as_ref().expect("its usage filter");
    assert_eq!(
        (filter.property.as_str(), filter.value.as_str()),
        ("team", "mobile")
    );
}

/// A hidden option is never offered in the portal: not among its plan's options, nor as a switch,
/// though the customer may switch option.
#[tokio::test]
async fn portal_never_offers_a_hidden_option() {
    use super::currency_portal::{journey, list, monthly_subscription, offered, plan, rate};
    use common_domain::ids::{BaseId, PriceId};
    use diesel_models::enums::BillingPeriodEnum as DieselBillingPeriodEnum;
    use rust_decimal_macros::dec;

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
    journey(&env, true).await;
    diesel_models::plan_version_options::PlanVersionOptionRow::set_hidden(
        &mut env.conn().await,
        TENANT_ID,
        crate::data::plans::option_id(pro, "annual"),
        true,
    )
    .await
    .expect("a hidden option");
    let sub = monthly_subscription(&env, pro).await;

    let response = list(&env, sub).await.expect("the plans");
    assert_eq!(
        offered(&response),
        vec![("Pro".to_string(), vec![("Monthly".to_string(), true)])],
        "Annual is hidden"
    );
    let refused = client(&env)
        .preview_plan_change(request(
            &env,
            portal::PreviewPlanChangeRequest {
                subscription_id: sub.to_string(),
                new_plan_version_id: pro.to_string(),
                option: Some("annual".to_string()),
                levels: vec![],
            },
        ))
        .await
        .expect_err("a hidden option is no switch");
    assert_eq!(refused.code(), Code::InvalidArgument, "{refused:?}");
}
