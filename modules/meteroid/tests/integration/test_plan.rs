use testcontainers::clients::Cli;

use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use meteroid_grpc::meteroid::api;
use meteroid_grpc::meteroid::api::plans::v1::plan_billing_configuration::BillingCycles;
use meteroid_grpc::meteroid::api::plans::v1::plan_billing_configuration::SubscriptionAnniversary;
use meteroid_grpc::meteroid::api::plans::v1::plan_billing_configuration::{
    Forever, ServicePeriodStart,
};
use meteroid_grpc::meteroid::api::plans::v1::PlanBillingConfiguration;
use meteroid_grpc::meteroid::api::users::v1::UserRole;

#[tokio::test]
async fn test_plans_basic() {
    // Generic setup
    helpers::init::logging();
    let docker = Cli::default();
    let (_postgres_container, postgres_connection_string) =
        meteroid_it::container::start_postgres(&docker);
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::PRODUCT)
            .await;

    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    assert_eq!(auth.user.unwrap().role, UserRole::Admin as i32);

    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "a712afi5lzhk",
    );

    // create plan
    let created_plan_details = clients
        .plans
        .clone()
        .create_draft_plan(api::plans::v1::CreateDraftPlanRequest {
            name: "plan_name".into(),
            external_id: "plan_external_id".into(),
            product_family_external_id: "default".into(),
            description: Some("plan_description".into()),
            plan_type: api::plans::v1::PlanType::Standard as i32,
        })
        .await
        .unwrap()
        .into_inner()
        .plan
        .unwrap();

    let created_plan = created_plan_details.plan.clone().unwrap();
    let created_version = created_plan_details.current_version.clone().unwrap();
    let created_metadata = created_plan_details.metadata.clone();

    assert_eq!(created_plan.name.as_str(), "plan_name");
    assert_eq!(created_plan.external_id.as_str(), "plan_external_id");
    assert_eq!(
        created_plan.description,
        Some("plan_description".to_string())
    );
    assert_eq!(
        created_plan.plan_status(),
        api::plans::v1::PlanStatus::Draft
    );
    assert_eq!(created_plan.plan_type(), api::plans::v1::PlanType::Standard);

    assert_eq!(created_version.version, 1);
    assert_eq!(created_version.currency.as_str(), "EUR");
    assert_eq!(created_version.is_draft, true);
    assert_eq!(created_version.trial_config, None);
    assert_eq!(
        created_version.billing_config,
        Some(PlanBillingConfiguration {
            billing_periods: vec![],
            net_terms: 0,
            service_period_start: Some(ServicePeriodStart::SubscriptionAnniversary(
                SubscriptionAnniversary {}
            )),
            billing_cycles: Some(BillingCycles::Forever(Forever {})),
        })
    );

    assert_eq!(created_metadata.len(), 0);

    // get plan by external_id
    let plan_details = clients
        .plans
        .clone()
        .get_plan_by_external_id(api::plans::v1::GetPlanByExternalIdRequest {
            external_id: "plan_external_id".into(),
        })
        .await
        .unwrap()
        .into_inner()
        .plan_details
        .unwrap();

    assert_eq!(&plan_details, &created_plan_details);

    // list plans
    let plans = clients
        .plans
        .clone()
        .list_plans(api::plans::v1::ListPlansRequest {
            product_family_external_id: None,
            sort_by: 0,
            search: None,
            pagination: None,
        })
        .await
        .unwrap()
        .into_inner()
        .plans;

    assert_eq!(plans.len(), 1);
    let plan_list = plans.first().unwrap();
    assert_eq!(plan_list.name.as_str(), "plan_name");
    assert_eq!(plan_list.external_id.as_str(), "plan_external_id");
    assert_eq!(plan_list.description, Some("plan_description".to_string()));
    assert_eq!(plan_list.plan_status(), api::plans::v1::PlanStatus::Draft);
    assert_eq!(plan_list.plan_type(), api::plans::v1::PlanType::Standard);

    let plan_versions = clients
        .plans
        .clone()
        .list_subscribable_plan_version(api::plans::v1::ListSubscribablePlanVersionRequest {})
        .await
        .unwrap()
        .into_inner()
        .plan_versions;

    assert_eq!(plan_versions.len(), 0);

    // get_plan_version_by_id
    let plan_version = clients
        .plans
        .clone()
        .get_plan_version_by_id(api::plans::v1::GetPlanVersionByIdRequest {
            plan_version_id: created_version.id.clone(),
        })
        .await
        .unwrap()
        .into_inner()
        .plan_version
        .unwrap();

    assert_eq!(&plan_version, &created_version);

    // list_plan_version_by_id
    let plan_versions = clients
        .plans
        .clone()
        .list_plan_version_by_id(api::plans::v1::ListPlanVersionByIdRequest {
            plan_id: created_plan.id.clone(),
            pagination: None,
        })
        .await
        .unwrap()
        .into_inner()
        .plan_versions;

    assert_eq!(plan_versions.len(), 1);
    let plan_version = plan_versions.first().unwrap();
    assert_eq!(&plan_version.id, &created_version.id);
    assert_eq!(&plan_version.version, &created_version.version);
    assert_eq!(&plan_version.is_draft, &created_version.is_draft);
    assert_eq!(&plan_version.currency, &created_version.currency);

    // publish plan version
    let published_version = clients
        .plans
        .clone()
        .publish_plan_version(api::plans::v1::PublishPlanVersionRequest {
            plan_id: created_plan.id.clone(),
            plan_version_id: created_version.id.clone(),
        })
        .await
        .unwrap()
        .into_inner()
        .plan_version
        .unwrap();

    assert_eq!(&published_version.is_draft, &false);

    // ListSubscribablePlanVersion
    let plan_versions = clients
        .plans
        .clone()
        .list_subscribable_plan_version(api::plans::v1::ListSubscribablePlanVersionRequest {})
        .await
        .unwrap()
        .into_inner()
        .plan_versions;

    assert_eq!(plan_versions.len(), 1);

    // copy version to draft
    let copied_draft_version = clients
        .plans
        .clone()
        .copy_version_to_draft(api::plans::v1::CopyVersionToDraftRequest {
            plan_id: created_plan.id.clone(),
            plan_version_id: created_version.id.clone(),
        })
        .await
        .unwrap()
        .into_inner()
        .plan_version
        .unwrap();

    assert_ne!(copied_draft_version.id.clone(), created_version.id.clone());
    assert_eq!(copied_draft_version.is_draft, true);
    assert_eq!(copied_draft_version.version, 2);

    // get last published version
    let last_published_version = clients
        .plans
        .clone()
        .get_last_published_plan_version(api::plans::v1::GetLastPublishedPlanVersionRequest {
            plan_id: created_plan.id.clone(),
        })
        .await
        .unwrap()
        .into_inner()
        .version
        .unwrap();

    assert_eq!(&last_published_version, &published_version);

    // discard plan version
    clients
        .plans
        .clone()
        .discard_draft_version(api::plans::v1::DiscardDraftVersionRequest {
            plan_id: created_plan.id.clone(),
            plan_version_id: copied_draft_version.id.clone(),
        })
        .await
        .unwrap()
        .into_inner();

    let plan_versions = clients
        .plans
        .clone()
        .list_plan_version_by_id(api::plans::v1::ListPlanVersionByIdRequest {
            plan_id: created_plan.id.clone(),
            pagination: None,
        })
        .await
        .unwrap()
        .into_inner()
        .plan_versions;

    assert_eq!(plan_versions.len(), 1);

    // teardown
    meteroid_it::container::terminate_meteroid(setup.token, setup.join_handle).await
}
