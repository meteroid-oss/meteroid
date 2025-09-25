use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use meteroid_grpc::meteroid::api;

#[tokio::test]
async fn test_plans_basic() {
    // Generic setup
    helpers::init::logging();
    let (_postgres_container, postgres_connection_string) =
        meteroid_it::container::start_postgres().await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::PRODUCT)
            .await;

    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;

    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "TESTORG",
        "testslug",
    );

    // create plan
    let created_plan_details = clients
        .plans
        .clone()
        .create_draft_plan(api::plans::v1::CreateDraftPlanRequest {
            name: "plan_name".into(),
            product_family_local_id: "default".into(),
            description: Some("plan_description".into()),
            plan_type: api::plans::v1::PlanType::Standard as i32,
            currency: "EUR".to_string(),
        })
        .await
        .unwrap()
        .into_inner()
        .plan
        .unwrap();

    let created_plan = created_plan_details.plan.clone().unwrap();
    let created_version = created_plan_details.version.clone().unwrap();

    println!("{:?}", created_plan);

    assert_eq!(
        created_plan.draft_version_id.clone().unwrap(),
        created_version.id.clone()
    );
    assert_eq!(created_plan.name.as_str(), "plan_name");
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
    assert!(created_version.is_draft);
    assert_eq!(created_version.trial_config, None);

    // get plan by local_id
    let plan_details = clients
        .plans
        .clone()
        .get_plan_with_version(api::plans::v1::GetPlanWithVersionRequest {
            filter: Some(api::plans::v1::get_plan_with_version_request::Filter::Draft(())),
            local_id: created_plan.local_id.clone(),
        })
        .await
        .unwrap()
        .into_inner()
        .plan
        .unwrap();

    assert_eq!(&plan_details, &created_plan_details);

    // list plans
    let plans = clients
        .plans
        .clone()
        .list_plans(api::plans::v1::ListPlansRequest {
            product_family_local_id: None,
            sort_by: 0,
            pagination: None,
            filters: None,
        })
        .await
        .unwrap()
        .into_inner()
        .plans;

    assert_eq!(plans.len(), 1);
    let plan_list = plans.first().unwrap();
    assert_eq!(plan_list.name.as_str(), "plan_name");
    assert_eq!(plan_list.local_id.as_str(), created_plan.local_id);
    assert_eq!(plan_list.description, Some("plan_description".to_string()));
    assert_eq!(plan_list.plan_status(), api::plans::v1::PlanStatus::Draft);
    assert_eq!(plan_list.plan_type(), api::plans::v1::PlanType::Standard);

    // get_plan_with_version
    let plan_version = clients
        .plans
        .clone()
        .get_plan_with_version(api::plans::v1::GetPlanWithVersionRequest {
            local_id: created_plan.local_id.clone(),
            filter: Some(
                api::plans::v1::get_plan_with_version_request::Filter::Version(
                    created_version.version,
                ),
            ),
        })
        .await
        .unwrap()
        .into_inner()
        .plan
        .unwrap();

    assert_eq!(&plan_version.version.unwrap(), &created_version);

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
    assert!(copied_draft_version.is_draft);
    assert_eq!(copied_draft_version.version, 2);

    // get last published version
    let last_published_version = clients
        .plans
        .clone()
        .get_plan_with_version(api::plans::v1::GetPlanWithVersionRequest {
            local_id: created_plan.local_id.clone(),
            filter: Some(api::plans::v1::get_plan_with_version_request::Filter::Active(())),
        })
        .await
        .unwrap()
        .into_inner()
        .plan
        .unwrap()
        .version
        .unwrap();

    assert_eq!(&last_published_version, &published_version);

    // update draft plan
    let plan_with_version = clients
        .plans
        .clone()
        .update_draft_plan_overview(api::plans::v1::UpdateDraftPlanOverviewRequest {
            plan_id: created_plan.id.clone(),
            plan_version_id: copied_draft_version.id.clone(),
            name: "new-plan-name".to_string(),
            description: Some("new-plan-desc".to_string()),
            currency: "AUD".to_string(),
            net_terms: 5,
        })
        .await
        .unwrap()
        .into_inner()
        .plan
        .unwrap();

    let plan = plan_with_version.plan.unwrap();
    let version = plan_with_version.version.unwrap();

    assert_eq!(&plan.id, &created_plan.id);
    assert_eq!(&version.id, &copied_draft_version.id);
    assert_eq!(&plan.name, "new-plan-name");
    assert_eq!(&plan.description, &Some("new-plan-desc".to_string()));
    assert_eq!(&version.currency, "AUD");
    assert_eq!(&version.net_terms, &5);

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

    // get plan overview by local_id
    let plan_overview = clients
        .plans
        .clone()
        .get_plan_overview(api::plans::v1::GetPlanOverviewRequest {
            local_id: created_plan.local_id,
        })
        .await
        .unwrap()
        .into_inner()
        .plan_overview
        .unwrap();

    assert_eq!(&plan_overview.id, &created_plan.id);
    assert_eq!(
        &plan_overview.active_version.map(|a| a.version),
        &Some(created_version.version)
    );

    // update published plan
    let plan_overview = clients
        .plans
        .clone()
        .update_published_plan_overview(api::plans::v1::UpdatePublishedPlanOverviewRequest {
            plan_id: created_plan.id.clone(),
            plan_version_id: created_version.id.clone(),
            name: "new-plan-name".to_string(),
            description: Some("new-plan-desc".to_string()),
        })
        .await
        .unwrap()
        .into_inner()
        .plan_overview
        .unwrap();

    assert_eq!(&plan_overview.id, &created_plan.id);
    assert_eq!(&plan_overview.name, "new-plan-name");
    assert_eq!(
        &plan_overview.description,
        &Some("new-plan-desc".to_string())
    );

    // teardown
    // meteroid_it::container::terminate_meteroid(setup.token, &setup.join_handle).await
}
