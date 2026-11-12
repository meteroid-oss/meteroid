use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use meteroid_grpc::meteroid::api;

#[tokio::test]
async fn test_plans_basic() {
    // Generic setup
    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::PRODUCT).await;
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
            entitlements: vec![],
            tax_inclusive: false,
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

    let active_or_draft = |local_id: String| {
        let mut plans = clients.plans.clone();
        async move {
            plans
                .get_plan_with_version(api::plans::v1::GetPlanWithVersionRequest {
                    local_id,
                    filter: Some(
                        api::plans::v1::get_plan_with_version_request::Filter::ActiveOrDraft(()),
                    ),
                })
                .await
                .unwrap()
                .into_inner()
                .plan
                .unwrap()
        }
    };

    // never published: active_or_draft falls back to the draft
    assert_eq!(
        &active_or_draft(created_plan.local_id.clone()).await,
        &created_plan_details
    );

    // list plans
    let plans = clients
        .plans
        .clone()
        .list_plans(api::plans::v1::ListPlansRequest {
            product_family_local_id: None,
            order_by: None,
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
    assert_eq!(
        published_version
            .credit_exhausted_rule
            .as_ref()
            .map(|rule| rule.kind()),
        Some(api::plans::v1::credit_exhausted_rule::Kind::Invoice),
        "publishing a version without credit sets the default: invoice"
    );

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

    // published, with a newer draft: active_or_draft picks the published version
    assert_eq!(
        active_or_draft(created_plan.local_id.clone())
            .await
            .version
            .as_ref(),
        Some(&published_version)
    );

    // update draft plan
    let update = |kind: api::plans::v1::credit_exhausted_rule::Kind, caps| {
        api::plans::v1::UpdateDraftPlanOverviewRequest {
            plan_id: created_plan.id.clone(),
            plan_version_id: copied_draft_version.id.clone(),
            name: "new-plan-name".to_string(),
            description: Some("new-plan-desc".to_string()),
            currency: "AUD".to_string(),
            net_terms: 5,
            tax_inclusive: true,
            credit_exhausted_rule: Some(api::plans::v1::CreditExhaustedRule {
                kind: kind.into(),
                caps,
            }),
        }
    };
    // Refusing usage at zero or past a cap needs the usage controls, absent from this edition.
    for refused in [
        update(api::plans::v1::credit_exhausted_rule::Kind::Pause, vec![]),
        update(
            api::plans::v1::credit_exhausted_rule::Kind::InvoiceUpTo,
            vec![api::plans::v1::CreditCap {
                denomination: "aud".to_string(),
                amount: 50_000,
            }],
        ),
    ] {
        let status = clients
            .plans
            .clone()
            .update_draft_plan_overview(refused)
            .await
            .expect_err("a rule the edition cannot enforce");
        assert_eq!(status.code(), tonic::Code::InvalidArgument, "{status:?}");
        assert!(
            status
                .message()
                .contains("Usage controls are not available in this edition"),
            "{status:?}"
        );
    }
    let plan_with_version = clients
        .plans
        .clone()
        .update_draft_plan_overview(update(
            api::plans::v1::credit_exhausted_rule::Kind::Invoice,
            vec![],
        ))
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
    assert!(version.tax_inclusive);
    assert_eq!(
        version.credit_exhausted_rule,
        Some(api::plans::v1::CreditExhaustedRule {
            kind: api::plans::v1::credit_exhausted_rule::Kind::Invoice.into(),
            caps: vec![],
        })
    );

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
            self_service_rank: None,
            issues_license: None,
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

/// A published version's components price live subscriptions: removing or adding one is refused
/// there (removal would cascade to every subscriber's line), allowed on a draft.
#[tokio::test]
async fn a_price_component_is_removed_from_a_draft_only() {
    use crate::data::ids::{
        COMP_NOTION_SEATS_ID, PLAN_VERSION_NOTION_ID, PRODUCT_SEATS_ID, TENANT_ID,
    };
    use meteroid_store::repositories::PlansInterface;
    use meteroid_store::repositories::price_components::PriceComponentInterfaceAuto;

    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::PLANS).await;
    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "TESTORG",
        "testslug",
    );
    let remove = |id: String| {
        let mut client = clients.price_components.clone();
        async move {
            client
                .remove_price_component(api::components::v1::RemovePriceComponentRequest {
                    price_component_id: id,
                })
                .await
        }
    };

    let refused = remove(COMP_NOTION_SEATS_ID.as_proto())
        .await
        .expect_err("a published version keeps its components");
    assert_eq!(refused.code(), tonic::Code::FailedPrecondition);
    let published = setup
        .store
        .list_price_components(PLAN_VERSION_NOTION_ID, TENANT_ID)
        .await
        .expect("components");
    assert!(published.iter().any(|c| c.id == COMP_NOTION_SEATS_ID));

    let added = clients
        .price_components
        .clone()
        .create_price_component(api::components::v1::CreatePriceComponentRequest {
            plan_version_id: PLAN_VERSION_NOTION_ID.as_proto(),
            name: "One Time".to_string(),
            product: Some(api::components::v1::ProductRef {
                r#ref: Some(api::components::v1::product_ref::Ref::ExistingProductId(
                    PRODUCT_SEATS_ID.as_proto(),
                )),
            }),
            prices: vec![api::components::v1::PriceEntry {
                entry: Some(api::components::v1::price_entry::Entry::NewPrice(
                    api::components::v1::PriceInput {
                        cadence: api::shared::v1::BillingPeriod::Monthly.into(),
                        currency: "EUR".to_string(),
                        pricing: Some(api::components::v1::price_input::Pricing::OneTimePricing(
                            api::prices::v1::OneTimePricing {
                                unit_price: "1.00".to_string(),
                                quantity: 1,
                            },
                        )),
                        credit_unit: None,
                    },
                )),
            }],
        })
        .await
        .expect_err("a published version takes no new component");
    assert_eq!(added.code(), tonic::Code::FailedPrecondition);

    let draft = setup
        .store
        .copy_plan_version_to_draft(PLAN_VERSION_NOTION_ID, TENANT_ID)
        .await
        .expect("a draft");
    let seats = setup
        .store
        .list_price_components(draft.id, TENANT_ID)
        .await
        .expect("components")
        .into_iter()
        .find(|c| c.name == "Seats")
        .expect("the draft's seats");
    remove(seats.id.as_proto())
        .await
        .expect("removed from the draft");
    let missing = remove(seats.id.as_proto())
        .await
        .expect_err("nothing left to remove");
    assert_eq!(missing.code(), tonic::Code::NotFound);
    assert!(
        setup
            .store
            .list_price_components(draft.id, TENANT_ID)
            .await
            .expect("components")
            .iter()
            .all(|c| c.id != seats.id)
    );
}

/// The list carries the products its components reference and the metrics those products bill on,
/// so a plan renders without a lookup per component.
#[tokio::test]
async fn price_components_come_with_their_products_and_metrics() {
    use crate::data::ids::{
        METRIC_BANDWIDTH, PLAN_VERSION_USAGE_ID, PRODUCT_BANDWIDTH_ID, PRODUCT_PLATFORM_FEE_ID,
    };

    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::PLANS).await;
    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "TESTORG",
        "testslug",
    );

    let res = clients
        .price_components
        .clone()
        .list_price_components(api::components::v1::ListPriceComponentRequest {
            plan_version_id: PLAN_VERSION_USAGE_ID.as_proto(),
        })
        .await
        .unwrap()
        .into_inner();

    assert!(
        res.components
            .iter()
            .any(|c| c.product_id == Some(PRODUCT_BANDWIDTH_ID.as_proto()))
    );
    let mut product_ids: Vec<_> = res.products.iter().map(|p| p.id.clone()).collect();
    product_ids.sort();
    let mut expected = vec![
        PRODUCT_BANDWIDTH_ID.as_proto(),
        PRODUCT_PLATFORM_FEE_ID.as_proto(),
    ];
    expected.sort();
    assert_eq!(product_ids, expected);
    let metric_ids: Vec<_> = res.metrics.iter().map(|m| m.id.clone()).collect();
    assert_eq!(metric_ids, vec![METRIC_BANDWIDTH.as_proto()]);
}
