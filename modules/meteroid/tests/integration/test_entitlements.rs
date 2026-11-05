use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use meteroid_app::conversions::ProtoConv;
use meteroid_grpc::meteroid::api::entitlements::v1::{
    CreateEntitlementRequest, CreateFeatureRequest, DeleteEntitlementRequest, EntitlementEntity,
    EntitlementValue, FeatureStatus, FeatureType, GetEffectiveEntitlementsRequest,
    GetEntitlementRequest, GetFeatureCodeHolderRequest, GetFeatureRequest,
    ListEntitlementsByEntityRequest, ListEntitlementsByFeatureRequest, ListFeaturesRequest,
    OverageBehavior, ResolvedOrigin, SetFeatureStatusRequest, UpdateEntitlementRequest,
    UpdateFeatureRequest, entitlement_entity, entitlement_value, feature_type, overage_behavior,
};
use meteroid_grpc::meteroid::api::subscriptions::v1 as sub_api;

use crate::data::ids;

#[tokio::test]
async fn test_features_crud() {
    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::MINIMAL).await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;

    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "TESTORG",
        "testslug",
    );

    // create
    let created = clients
        .entitlements
        .clone()
        .create_feature(CreateFeatureRequest {
            name: "api-access".into(),
            code: "api-access".into(),
            description: Some("API access feature".into()),
            product_id: None,
            feature_type: Some(FeatureType {
                inner: Some(feature_type::Inner::Boolean(
                    feature_type::BooleanFeature {},
                )),
            }),
            entitlement: None,
        })
        .await
        .unwrap()
        .into_inner()
        .feature
        .unwrap();

    assert_eq!(created.name, "api-access");
    assert_eq!(created.description, Some("API access feature".into()));
    assert!(matches!(
        created.feature_type.and_then(|ft| ft.inner),
        Some(feature_type::Inner::Boolean(_))
    ));
    assert_eq!(created.status, FeatureStatus::Active as i32);

    // get
    let fetched = clients
        .entitlements
        .clone()
        .get_feature(GetFeatureRequest {
            id: created.id.clone(),
        })
        .await
        .unwrap()
        .into_inner()
        .feature
        .unwrap();

    assert_eq!(fetched.id, created.id);

    // update
    let updated = clients
        .entitlements
        .clone()
        .update_feature(UpdateFeatureRequest {
            id: created.id.clone(),
            name: Some("api-access-v2".into()),
            description: None,
            product_id: None,
            clear_product_id: false,
            config_options: vec![],
            update_config_options: false,
        })
        .await
        .unwrap()
        .into_inner()
        .feature
        .unwrap();

    assert_eq!(updated.name, "api-access-v2");

    // list
    let listed = clients
        .entitlements
        .clone()
        .list_features(ListFeaturesRequest {
            pagination: None,
            statuses: vec![],
            search: None,
            product_id: None,
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(listed.features.len(), 1);
    assert_eq!(listed.features[0].id, created.id);

    // an entity's entitlements come with their features, each once
    clients
        .entitlements
        .clone()
        .create_entitlement(CreateEntitlementRequest {
            feature_id: created.id.clone(),
            entity: Some(EntitlementEntity {
                entity_id: Some(entitlement_entity::EntityId::FeatureId(created.id.clone())),
            }),
            value: Some(EntitlementValue {
                value: Some(entitlement_value::Value::BooleanValue(
                    entitlement_value::BooleanValue { enabled: true },
                )),
            }),
        })
        .await
        .unwrap();
    let features_listed_with_entitlements = || {
        let mut client = clients.entitlements.clone();
        let id = created.id.clone();
        async move {
            client
                .list_entitlements_by_entity(ListEntitlementsByEntityRequest {
                    entity: Some(EntitlementEntity {
                        entity_id: Some(entitlement_entity::EntityId::FeatureId(id)),
                    }),
                })
                .await
                .unwrap()
                .into_inner()
                .features
                .into_iter()
                .map(|f| (f.id, f.status))
                .collect::<Vec<_>>()
        }
    };
    assert_eq!(
        features_listed_with_entitlements().await,
        vec![(created.id.clone(), FeatureStatus::Active as i32)]
    );

    let set_status = |id: String, status: FeatureStatus| {
        let mut client = clients.entitlements.clone();
        async move {
            client
                .set_feature_status(SetFeatureStatusRequest {
                    id,
                    status: status.into(),
                })
                .await
        }
    };
    let listed_ids = |statuses: Vec<FeatureStatus>| {
        let mut client = clients.entitlements.clone();
        async move {
            client
                .list_features(ListFeaturesRequest {
                    pagination: None,
                    statuses: statuses.into_iter().map(Into::into).collect(),
                    search: None,
                    product_id: None,
                })
                .await
                .unwrap()
                .into_inner()
                .features
                .into_iter()
                .map(|f| f.id)
                .collect::<Vec<_>>()
        }
    };

    // disable (kill-switch — the feature stays listed by default)
    set_status(created.id.clone(), FeatureStatus::Disabled)
        .await
        .unwrap();
    assert_eq!(listed_ids(vec![]).await, vec![created.id.clone()]);
    assert_eq!(
        listed_ids(vec![FeatureStatus::Disabled]).await,
        vec![created.id.clone()]
    );
    assert!(listed_ids(vec![FeatureStatus::Active]).await.is_empty());
    assert_eq!(
        features_listed_with_entitlements().await,
        vec![(created.id.clone(), FeatureStatus::Disabled as i32)],
        "a disabled feature still comes with its entitlements"
    );

    // re-enable
    set_status(created.id.clone(), FeatureStatus::Active)
        .await
        .unwrap();
    let reactivated = clients
        .entitlements
        .clone()
        .get_feature(GetFeatureRequest {
            id: created.id.clone(),
        })
        .await
        .unwrap()
        .into_inner()
        .feature
        .unwrap();
    assert_eq!(reactivated.status, FeatureStatus::Active as i32);

    // archive: excluded from the default listing, visible when requested
    set_status(created.id.clone(), FeatureStatus::Archived)
        .await
        .unwrap();
    assert!(listed_ids(vec![]).await.is_empty());
    assert!(listed_ids(vec![FeatureStatus::Active]).await.is_empty());
    assert_eq!(
        listed_ids(vec![FeatureStatus::Archived]).await,
        vec![created.id.clone()]
    );
    let archived = clients
        .entitlements
        .clone()
        .get_feature(GetFeatureRequest {
            id: created.id.clone(),
        })
        .await
        .unwrap()
        .into_inner()
        .feature
        .unwrap();
    assert!(
        archived.entitlement.is_some(),
        "an archived feature still reads with its default, as in the list"
    );

    let code_holder = |id: String| {
        let mut client = clients.entitlements.clone();
        async move {
            client
                .get_feature_code_holder(GetFeatureCodeHolderRequest { id })
                .await
                .unwrap()
                .into_inner()
                .holder
                .map(|f| f.id)
        }
    };
    assert_eq!(code_holder(created.id.clone()).await, None);

    // archiving releases the code
    let reused = clients
        .entitlements
        .clone()
        .create_feature(CreateFeatureRequest {
            name: "api-access-v3".into(),
            code: created.code.clone(),
            description: None,
            product_id: None,
            feature_type: Some(FeatureType {
                inner: Some(feature_type::Inner::Boolean(
                    feature_type::BooleanFeature {},
                )),
            }),
            entitlement: None,
        })
        .await
        .unwrap()
        .into_inner()
        .feature
        .unwrap();
    assert_ne!(reused.id, created.id);
    assert_eq!(
        code_holder(created.id.clone()).await,
        Some(reused.id.clone()),
        "the new feature holds the archived feature's code"
    );

    // restoring fails while another feature holds the code
    let blocked = set_status(created.id.clone(), FeatureStatus::Active)
        .await
        .unwrap_err();
    assert_eq!(blocked.code(), tonic::Code::FailedPrecondition);
    assert!(
        blocked.message().contains("api-access-v3"),
        "names the feature holding the code: {blocked:?}"
    );

    set_status(reused.id.clone(), FeatureStatus::Archived)
        .await
        .unwrap();
    set_status(created.id.clone(), FeatureStatus::Active)
        .await
        .expect("restores once the code is free");
    assert_eq!(listed_ids(vec![]).await, vec![created.id.clone()]);
    assert_eq!(
        code_holder(created.id.clone()).await,
        None,
        "an active feature has no holder"
    );
}

#[tokio::test]
async fn test_entitlements_crud() {
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

    let feature = clients
        .entitlements
        .clone()
        .create_feature(CreateFeatureRequest {
            name: "seats".into(),
            code: "seats".into(),
            description: None,
            product_id: None,
            feature_type: Some(FeatureType {
                inner: Some(feature_type::Inner::Metered(feature_type::MeteredFeature {
                    metric_id: ids::METRIC_BANDWIDTH.as_proto(),
                })),
            }),
            entitlement: None,
        })
        .await
        .unwrap()
        .into_inner()
        .feature
        .unwrap();

    let plan_version_id = ids::PLAN_VERSION_NOTION_ID.as_proto();

    // create entitlement on a plan version
    let entitlement = clients
        .entitlements
        .clone()
        .create_entitlement(CreateEntitlementRequest {
            feature_id: feature.id.clone(),
            entity: Some(EntitlementEntity {
                entity_id: Some(entitlement_entity::EntityId::PlanVersionId(
                    plan_version_id.clone(),
                )),
            }),
            value: Some(EntitlementValue {
                value: Some(entitlement_value::Value::MeteredValue(
                    entitlement_value::MeteredValue {
                        limit: Some("100".into()),
                        reset_period: None,
                        enabled: true,
                        group_by: None,
                        ..Default::default()
                    },
                )),
            }),
        })
        .await
        .unwrap()
        .into_inner()
        .entitlement
        .unwrap();

    assert_eq!(entitlement.feature_id, feature.id);
    assert_eq!(
        entitlement.entity,
        Some(EntitlementEntity {
            entity_id: Some(entitlement_entity::EntityId::PlanVersionId(plan_version_id))
        })
    );

    // get
    let fetched = clients
        .entitlements
        .clone()
        .get_entitlement(GetEntitlementRequest {
            id: entitlement.id.clone(),
        })
        .await
        .unwrap()
        .into_inner()
        .entitlement
        .unwrap();

    assert_eq!(fetched.id, entitlement.id);

    // list by feature
    let by_feature = clients
        .entitlements
        .clone()
        .list_entitlements_by_feature(ListEntitlementsByFeatureRequest {
            feature_id: feature.id.clone(),
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(by_feature.entitlements.len(), 1);

    // update — raise limit
    let updated = clients
        .entitlements
        .clone()
        .update_entitlement(UpdateEntitlementRequest {
            id: entitlement.id.clone(),
            value: Some(EntitlementValue {
                value: Some(entitlement_value::Value::MeteredValue(
                    entitlement_value::MeteredValue {
                        limit: Some("500".into()),
                        reset_period: None,
                        enabled: true,
                        group_by: None,
                        ..Default::default()
                    },
                )),
            }),
        })
        .await
        .unwrap()
        .into_inner()
        .entitlement
        .unwrap();

    if let Some(entitlement_value::Value::MeteredValue(m)) = updated.value.and_then(|v| v.value) {
        assert_eq!(m.limit.as_deref(), Some("500"));
    } else {
        panic!("expected metered value");
    }

    // delete
    clients
        .entitlements
        .clone()
        .delete_entitlement(DeleteEntitlementRequest {
            id: entitlement.id.clone(),
        })
        .await
        .unwrap();

    let by_feature_after_delete = clients
        .entitlements
        .clone()
        .list_entitlements_by_feature(ListEntitlementsByFeatureRequest {
            feature_id: feature.id.clone(),
        })
        .await
        .unwrap()
        .into_inner();

    assert!(by_feature_after_delete.entitlements.is_empty());
}

#[tokio::test]
async fn test_get_effective_entitlements() {
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

    // create a subscription for Spotify on the Notion plan
    let now = chrono::Utc::now().date_naive();
    clients
        .subscriptions
        .clone()
        .create_subscription(tonic::Request::new(sub_api::CreateSubscriptionRequest {
            subscription: Some(sub_api::CreateSubscription {
                plan_version_id: (*ids::PLAN_VERSION_NOTION_ID).to_string(),
                start_date: now.as_proto(),
                billing_day_anchor: Some(1),
                customer_id: ids::CUST_SPOTIFY_ID.as_proto(),
                charge_automatically: Some(false),
                components: Some(sub_api::CreateSubscriptionComponents {
                    parameterized_components: vec![
                        sub_api::create_subscription_components::ComponentParameterization {
                            component_id: (*ids::COMP_NOTION_SEATS_ID).to_string(),
                            billing_period: Some(
                                meteroid_grpc::meteroid::api::shared::v1::BillingPeriod::Monthly
                                    .into(),
                            ),
                            initial_slot_count: Some(1),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }),
                ..Default::default()
            }),
        }))
        .await
        .unwrap();

    let feature = clients
        .entitlements
        .clone()
        .create_feature(CreateFeatureRequest {
            name: "api-calls".into(),
            code: "api-calls".into(),
            description: None,
            product_id: None,
            feature_type: Some(FeatureType {
                inner: Some(feature_type::Inner::Metered(feature_type::MeteredFeature {
                    metric_id: ids::METRIC_BANDWIDTH.as_proto(),
                })),
            }),
            entitlement: None,
        })
        .await
        .unwrap()
        .into_inner()
        .feature
        .unwrap();

    // plan-level grant
    clients
        .entitlements
        .clone()
        .create_entitlement(CreateEntitlementRequest {
            feature_id: feature.id.clone(),
            entity: Some(EntitlementEntity {
                entity_id: Some(entitlement_entity::EntityId::PlanId(
                    ids::PLAN_NOTION_ID.as_proto(),
                )),
            }),
            value: Some(EntitlementValue {
                value: Some(entitlement_value::Value::MeteredValue(
                    entitlement_value::MeteredValue {
                        limit: Some("1000".into()),
                        reset_period: None,
                        enabled: true,
                        group_by: None,
                        ..Default::default()
                    },
                )),
            }),
        })
        .await
        .unwrap();

    // plan-version-level grant — should win over plan grant (higher priority)
    clients
        .entitlements
        .clone()
        .create_entitlement(CreateEntitlementRequest {
            feature_id: feature.id.clone(),
            entity: Some(EntitlementEntity {
                entity_id: Some(entitlement_entity::EntityId::PlanVersionId(
                    ids::PLAN_VERSION_NOTION_ID.as_proto(),
                )),
            }),
            value: Some(EntitlementValue {
                value: Some(entitlement_value::Value::MeteredValue(
                    entitlement_value::MeteredValue {
                        limit: Some("5000".into()),
                        reset_period: None,
                        enabled: true,
                        group_by: None,
                        ..Default::default()
                    },
                )),
            }),
        })
        .await
        .unwrap();

    // Spotify is subscribed to Notion plan
    let resolved = clients
        .entitlements
        .clone()
        .get_effective_entitlements(GetEffectiveEntitlementsRequest {
            customer_id: ids::CUST_SPOTIFY_ID.as_proto(),
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(resolved.entitlements.len(), 1);
    let ent = &resolved.entitlements[0];
    assert_eq!(
        ent.feature.as_ref().map(|f| f.name.as_str()),
        Some("api-calls")
    );

    use meteroid_grpc::meteroid::api::entitlements::v1::effective_entitlement;
    if let Some(effective_entitlement::Value::Metered(m)) = &ent.value {
        assert_eq!(m.limit.as_deref(), Some("5000"));
    } else {
        panic!("expected metered resolved entitlement");
    }
}

#[tokio::test]
async fn test_create_feature_with_entitlement() {
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

    let response = clients
        .entitlements
        .clone()
        .create_feature(CreateFeatureRequest {
            name: "bulk-export".into(),
            code: "bulk-export".into(),
            description: Some("Bulk export access".into()),
            product_id: None,
            feature_type: Some(FeatureType {
                inner: Some(feature_type::Inner::Boolean(
                    feature_type::BooleanFeature {},
                )),
            }),
            entitlement: Some(EntitlementValue {
                value: Some(entitlement_value::Value::BooleanValue(
                    entitlement_value::BooleanValue { enabled: true },
                )),
            }),
        })
        .await
        .unwrap()
        .into_inner();

    let feature = response.feature.unwrap();
    let entitlement = response.entitlement.unwrap();

    assert_eq!(feature.name, "bulk-export");
    assert_eq!(feature.description, Some("Bulk export access".into()));
    assert!(matches!(
        feature.feature_type.and_then(|ft| ft.inner),
        Some(feature_type::Inner::Boolean(_))
    ));

    assert_eq!(entitlement.feature_id, feature.id);
    assert_eq!(
        entitlement.entity,
        Some(EntitlementEntity {
            entity_id: Some(entitlement_entity::EntityId::FeatureId(feature.id.clone()))
        })
    );
    assert!(matches!(
        entitlement.value.and_then(|v| v.value),
        Some(entitlement_value::Value::BooleanValue(b)) if b.enabled
    ));

    // Verify persistence: the entitlement appears when listing by feature
    let listed = clients
        .entitlements
        .clone()
        .list_entitlements_by_feature(ListEntitlementsByFeatureRequest {
            feature_id: feature.id.clone(),
        })
        .await
        .unwrap()
        .into_inner()
        .entitlements;

    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, entitlement.id);
}

fn metered_grouped(limit: &str, group_by: Option<&str>) -> Option<EntitlementValue> {
    Some(EntitlementValue {
        value: Some(entitlement_value::Value::MeteredValue(
            entitlement_value::MeteredValue {
                limit: Some(limit.into()),
                reset_period: None,
                enabled: true,
                group_by: group_by.map(str::to_owned),
                ..Default::default()
            },
        )),
    })
}

fn grouped_by(
    entitlement: &meteroid_grpc::meteroid::api::entitlements::v1::Entitlement,
) -> Option<String> {
    match entitlement.value.clone().and_then(|v| v.value) {
        Some(entitlement_value::Value::MeteredValue(m)) => m.group_by,
        other => panic!("expected a metered value, got {other:?}"),
    }
}

#[tokio::test]
async fn test_entitlement_group_by_is_written_and_read_through_grpc() {
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

    let feature = clients
        .entitlements
        .clone()
        .create_feature(CreateFeatureRequest {
            name: "api calls per key".into(),
            code: "api_calls_per_key".into(),
            description: None,
            product_id: None,
            feature_type: Some(FeatureType {
                inner: Some(feature_type::Inner::Metered(feature_type::MeteredFeature {
                    metric_id: ids::METRIC_BANDWIDTH.as_proto(),
                })),
            }),
            entitlement: None,
        })
        .await
        .unwrap()
        .into_inner()
        .feature
        .unwrap();

    let created = clients
        .entitlements
        .clone()
        .create_entitlement(CreateEntitlementRequest {
            feature_id: feature.id.clone(),
            entity: entity_plan_version(ids::PLAN_VERSION_NOTION_ID.as_proto()),
            value: metered_grouped("100", Some("api_key")),
        })
        .await
        .unwrap()
        .into_inner()
        .entitlement
        .unwrap();
    assert_eq!(grouped_by(&created).as_deref(), Some("api_key"));

    let fetched = clients
        .entitlements
        .clone()
        .get_entitlement(GetEntitlementRequest {
            id: created.id.clone(),
        })
        .await
        .unwrap()
        .into_inner()
        .entitlement
        .unwrap();
    assert_eq!(grouped_by(&fetched).as_deref(), Some("api_key"));

    let listed = clients
        .entitlements
        .clone()
        .list_entitlements_by_feature(ListEntitlementsByFeatureRequest {
            feature_id: feature.id.clone(),
        })
        .await
        .unwrap()
        .into_inner()
        .entitlements;
    assert_eq!(grouped_by(&listed[0]).as_deref(), Some("api_key"));

    let update = |value| {
        let mut client = clients.entitlements.clone();
        let id = created.id.clone();
        async move {
            client
                .update_entitlement(UpdateEntitlementRequest { id, value })
                .await
        }
    };

    let raised = update(metered_grouped("500", Some("api_key")))
        .await
        .unwrap()
        .into_inner()
        .entitlement
        .unwrap();
    assert_eq!(
        grouped_by(&raised).as_deref(),
        Some("api_key"),
        "an edit that keeps the property does not clear it"
    );

    let regrouped = update(metered_grouped("500", Some("tenant")))
        .await
        .unwrap()
        .into_inner()
        .entitlement
        .unwrap();
    assert_eq!(grouped_by(&regrouped).as_deref(), Some("tenant"));

    let ungrouped = update(metered_grouped("500", None))
        .await
        .unwrap()
        .into_inner()
        .entitlement
        .unwrap();
    assert_eq!(grouped_by(&ungrouped), None);

    let invalid = update(metered_grouped("500", Some("a,b")))
        .await
        .unwrap_err();
    assert_eq!(invalid.code(), tonic::Code::InvalidArgument);
    let blank = update(metered_grouped("500", Some(" "))).await.unwrap_err();
    assert_eq!(blank.code(), tonic::Code::InvalidArgument);
}

fn metered_policy(
    limit: &str,
    overage: Option<overage_behavior::Inner>,
    warning_threshold_pct: Option<u32>,
) -> Option<EntitlementValue> {
    Some(EntitlementValue {
        value: Some(entitlement_value::Value::MeteredValue(
            entitlement_value::MeteredValue {
                limit: Some(limit.into()),
                enabled: true,
                overage_behavior: overage.map(|inner| OverageBehavior { inner: Some(inner) }),
                warning_threshold_pct,
                ..Default::default()
            },
        )),
    })
}

fn block(grace_period_pct: Option<u32>) -> Option<overage_behavior::Inner> {
    Some(overage_behavior::Inner::Block(overage_behavior::Block {
        grace_period_pct,
    }))
}

fn policy_of(
    entitlement: &meteroid_grpc::meteroid::api::entitlements::v1::Entitlement,
) -> (Option<overage_behavior::Inner>, Option<u32>) {
    match entitlement.value.clone().and_then(|v| v.value) {
        Some(entitlement_value::Value::MeteredValue(m)) => (
            m.overage_behavior.and_then(|o| o.inner),
            m.warning_threshold_pct,
        ),
        other => panic!("expected a metered value, got {other:?}"),
    }
}

#[tokio::test]
async fn test_entitlement_overage_policy_round_trips_through_grpc() {
    use meteroid_grpc::meteroid::api::entitlements::v1::{
        GetResolvedForPlanVersionRequest, resolved_entitlement,
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

    let feature = clients
        .entitlements
        .clone()
        .create_feature(CreateFeatureRequest {
            name: "seats".into(),
            code: "seats".into(),
            description: None,
            product_id: None,
            feature_type: Some(FeatureType {
                inner: Some(feature_type::Inner::Metered(feature_type::MeteredFeature {
                    metric_id: ids::METRIC_BANDWIDTH.as_proto(),
                })),
            }),
            entitlement: None,
        })
        .await
        .unwrap()
        .into_inner()
        .feature
        .unwrap();

    let created = clients
        .entitlements
        .clone()
        .create_entitlement(CreateEntitlementRequest {
            feature_id: feature.id.clone(),
            entity: entity_plan_version(ids::PLAN_VERSION_NOTION_ID.as_proto()),
            value: metered_policy("100", block(Some(25)), Some(80)),
        })
        .await
        .unwrap()
        .into_inner()
        .entitlement
        .unwrap();
    assert_eq!(policy_of(&created), (block(Some(25)), Some(80)));

    let fetched = clients
        .entitlements
        .clone()
        .get_entitlement(GetEntitlementRequest {
            id: created.id.clone(),
        })
        .await
        .unwrap()
        .into_inner()
        .entitlement
        .unwrap();
    assert_eq!(policy_of(&fetched), (block(Some(25)), Some(80)));

    let resolved = clients
        .entitlements
        .clone()
        .get_resolved_entitlements_for_plan_version(GetResolvedForPlanVersionRequest {
            plan_version_id: ids::PLAN_VERSION_NOTION_ID.as_proto(),
        })
        .await
        .unwrap()
        .into_inner()
        .entitlements;
    let Some(resolved_entitlement::Value::Metered(metered)) = resolved
        .iter()
        .find(|r| r.feature.as_ref().map(|f| f.id.as_str()) == Some(feature.id.as_str()))
        .and_then(|r| r.value.clone())
    else {
        panic!("expected the seats entitlement to resolve as metered");
    };
    assert_eq!(
        metered.overage_behavior.and_then(|o| o.inner),
        block(Some(25))
    );
    assert_eq!(metered.warning_threshold_pct, Some(80));

    let update = |value| {
        let mut client = clients.entitlements.clone();
        let id = created.id.clone();
        async move {
            client
                .update_entitlement(UpdateEntitlementRequest { id, value })
                .await
        }
    };

    let raised = update(metered_policy("500", block(Some(25)), Some(80)))
        .await
        .unwrap()
        .into_inner()
        .entitlement
        .unwrap();
    assert_eq!(
        policy_of(&raised),
        (block(Some(25)), Some(80)),
        "an edit that resends the policy does not reset it"
    );

    let allowed = update(metered_policy(
        "500",
        Some(overage_behavior::Inner::Allow(overage_behavior::Allow {})),
        None,
    ))
    .await
    .unwrap()
    .into_inner()
    .entitlement
    .unwrap();
    assert_eq!(
        policy_of(&allowed),
        (
            Some(overage_behavior::Inner::Allow(overage_behavior::Allow {})),
            None
        )
    );

    let defaulted = update(metered_policy("500", None, None))
        .await
        .unwrap()
        .into_inner()
        .entitlement
        .unwrap();
    assert_eq!(policy_of(&defaulted), (block(None), None));

    for invalid in [
        metered_policy("500", block(Some(0)), None),
        metered_policy("500", block(Some(1001)), None),
        metered_policy("500", None, Some(0)),
        metered_policy("500", None, Some(101)),
        metered_policy(
            "500",
            Some(overage_behavior::Inner::Allow(overage_behavior::Allow {})),
            Some(101),
        ),
    ] {
        let status = update(invalid).await.unwrap_err();
        assert_eq!(status.code(), tonic::Code::InvalidArgument);
    }
    let missing_variant = update(Some(EntitlementValue {
        value: Some(entitlement_value::Value::MeteredValue(
            entitlement_value::MeteredValue {
                limit: Some("500".into()),
                enabled: true,
                overage_behavior: Some(OverageBehavior { inner: None }),
                ..Default::default()
            },
        )),
    }))
    .await
    .unwrap_err();
    assert_eq!(missing_variant.code(), tonic::Code::InvalidArgument);
}

// ── Helpers for the integration tests below ────────────────────────────────────

/// Build a metered entitlement value with just a hard limit; all other knobs unset.
fn metered_value(limit: &str) -> Option<EntitlementValue> {
    Some(EntitlementValue {
        value: Some(entitlement_value::Value::MeteredValue(
            entitlement_value::MeteredValue {
                limit: Some(limit.into()),
                reset_period: None,
                enabled: true,
                group_by: None,
                ..Default::default()
            },
        )),
    })
}

fn boolean_value(enabled: bool) -> Option<EntitlementValue> {
    Some(EntitlementValue {
        value: Some(entitlement_value::Value::BooleanValue(
            entitlement_value::BooleanValue { enabled },
        )),
    })
}

fn entity_plan(id_proto: String) -> Option<EntitlementEntity> {
    Some(EntitlementEntity {
        entity_id: Some(entitlement_entity::EntityId::PlanId(id_proto)),
    })
}

fn entity_plan_version(id_proto: String) -> Option<EntitlementEntity> {
    Some(EntitlementEntity {
        entity_id: Some(entitlement_entity::EntityId::PlanVersionId(id_proto)),
    })
}

fn entity_subscription_proto(id_proto: String) -> Option<EntitlementEntity> {
    Some(EntitlementEntity {
        entity_id: Some(entitlement_entity::EntityId::SubscriptionId(id_proto)),
    })
}

fn entity_feature(id_proto: String) -> Option<EntitlementEntity> {
    Some(EntitlementEntity {
        entity_id: Some(entitlement_entity::EntityId::FeatureId(id_proto)),
    })
}

fn entity_customer(id_proto: String) -> Option<EntitlementEntity> {
    Some(EntitlementEntity {
        entity_id: Some(entitlement_entity::EntityId::CustomerId(id_proto)),
    })
}

#[tokio::test]
async fn test_kill_switch_suppresses_feature_resolution() {
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

    let now = chrono::Utc::now().date_naive();
    clients
        .subscriptions
        .clone()
        .create_subscription(tonic::Request::new(sub_api::CreateSubscriptionRequest {
            subscription: Some(sub_api::CreateSubscription {
                plan_version_id: (*ids::PLAN_VERSION_NOTION_ID).to_string(),
                start_date: now.as_proto(),
                billing_day_anchor: Some(1),
                customer_id: ids::CUST_SPOTIFY_ID.as_proto(),
                charge_automatically: Some(false),
                components: Some(sub_api::CreateSubscriptionComponents {
                    parameterized_components: vec![
                        sub_api::create_subscription_components::ComponentParameterization {
                            component_id: (*ids::COMP_NOTION_SEATS_ID).to_string(),
                            billing_period: Some(
                                meteroid_grpc::meteroid::api::shared::v1::BillingPeriod::Monthly
                                    .into(),
                            ),
                            initial_slot_count: Some(1),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }),
                ..Default::default()
            }),
        }))
        .await
        .unwrap();

    let feature = clients
        .entitlements
        .clone()
        .create_feature(CreateFeatureRequest {
            name: "premium-export".into(),
            code: "premium-export".into(),
            description: None,
            product_id: None,
            feature_type: Some(FeatureType {
                inner: Some(feature_type::Inner::Boolean(
                    feature_type::BooleanFeature {},
                )),
            }),
            entitlement: None,
        })
        .await
        .unwrap()
        .into_inner()
        .feature
        .unwrap();

    // Plan grant enables the feature.
    clients
        .entitlements
        .clone()
        .create_entitlement(CreateEntitlementRequest {
            feature_id: feature.id.clone(),
            entity: entity_plan(ids::PLAN_NOTION_ID.as_proto()),
            value: boolean_value(true),
        })
        .await
        .unwrap();

    // Higher-priority plan-version KillSwitch wipes the feature.
    clients
        .entitlements
        .clone()
        .create_entitlement(CreateEntitlementRequest {
            feature_id: feature.id.clone(),
            entity: entity_plan_version(ids::PLAN_VERSION_NOTION_ID.as_proto()),
            value: boolean_value(false),
        })
        .await
        .unwrap();

    let resolved = clients
        .entitlements
        .clone()
        .get_effective_entitlements(GetEffectiveEntitlementsRequest {
            customer_id: ids::CUST_SPOTIFY_ID.as_proto(),
        })
        .await
        .unwrap()
        .into_inner();

    use meteroid_grpc::meteroid::api::entitlements::v1::effective_entitlement;

    // In the new design a higher-priority boolean(false) overrides a lower-priority boolean(true)
    // and the entitlement is still returned but with enabled=false.
    let ent = resolved
        .entitlements
        .iter()
        .find(|e| e.feature.as_ref().is_some_and(|f| f.id == feature.id))
        .expect("feature must resolve (higher-priority false overrides lower-priority true)");
    assert!(
        matches!(
            &ent.value,
            Some(effective_entitlement::Value::Boolean(b)) if !b.enabled
        ),
        "plan-version-level boolean(false) must override plan-level boolean(true)"
    );
}

#[tokio::test]
async fn test_feature_archive_and_restore_lifecycle() {
    use meteroid_grpc::meteroid::api::entitlements::v1::GetResolvedForSubscriptionRequest;

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

    let now = chrono::Utc::now().date_naive();
    clients
        .subscriptions
        .clone()
        .create_subscription(tonic::Request::new(sub_api::CreateSubscriptionRequest {
            subscription: Some(sub_api::CreateSubscription {
                plan_version_id: (*ids::PLAN_VERSION_NOTION_ID).to_string(),
                start_date: now.as_proto(),
                billing_day_anchor: Some(1),
                customer_id: ids::CUST_SPOTIFY_ID.as_proto(),
                charge_automatically: Some(false),
                components: Some(sub_api::CreateSubscriptionComponents {
                    parameterized_components: vec![
                        sub_api::create_subscription_components::ComponentParameterization {
                            component_id: (*ids::COMP_NOTION_SEATS_ID).to_string(),
                            billing_period: Some(
                                meteroid_grpc::meteroid::api::shared::v1::BillingPeriod::Monthly
                                    .into(),
                            ),
                            initial_slot_count: Some(1),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }),
                ..Default::default()
            }),
        }))
        .await
        .unwrap();

    let feature = clients
        .entitlements
        .clone()
        .create_feature(boolean_feature_request("dormant-feat"))
        .await
        .unwrap()
        .into_inner()
        .feature
        .unwrap();
    // A second feature whose only grant is its own default, staged while disabled.
    let defaulted = clients
        .entitlements
        .clone()
        .create_feature(boolean_feature_request("defaulted-feat"))
        .await
        .unwrap()
        .into_inner()
        .feature
        .unwrap();

    let resolves = |feature_id: String| {
        let mut client = clients.entitlements.clone();
        async move {
            client
                .get_effective_entitlements(GetEffectiveEntitlementsRequest {
                    customer_id: ids::CUST_SPOTIFY_ID.as_proto(),
                })
                .await
                .unwrap()
                .into_inner()
                .entitlements
                .iter()
                .any(|e| e.feature.as_ref().is_some_and(|f| f.id == feature_id))
        }
    };
    let resolves_on_subscription = |subscription_id: String, feature_id: String| {
        let mut client = clients.entitlements.clone();
        async move {
            client
                .get_resolved_entitlements_for_subscription(GetResolvedForSubscriptionRequest {
                    subscription_id,
                })
                .await
                .unwrap()
                .into_inner()
                .entitlements
                .iter()
                .any(|e| e.feature.as_ref().is_some_and(|f| f.id == feature_id))
        }
    };
    let grants_on = |entity: Option<EntitlementEntity>| {
        let mut client = clients.entitlements.clone();
        let feature_id = feature.id.clone();
        async move {
            client
                .list_entitlements_by_entity(ListEntitlementsByEntityRequest { entity })
                .await
                .unwrap()
                .into_inner()
                .entitlements
                .into_iter()
                .filter(|e| e.feature_id == feature_id)
                .map(|e| (e.id, e.value))
                .collect::<Vec<_>>()
        }
    };
    let grant = |feature_id: String, entity: Option<EntitlementEntity>| {
        let mut client = clients.entitlements.clone();
        async move {
            client
                .create_entitlement(CreateEntitlementRequest {
                    feature_id,
                    entity,
                    value: boolean_value(true),
                })
                .await
                .map(|r| r.into_inner().entitlement.unwrap())
        }
    };
    let subscribe_with_grant = |feature_id: String| {
        let mut client = clients.subscriptions.clone();
        let now = chrono::Utc::now().date_naive();
        async move {
            client
                .create_subscription(tonic::Request::new(sub_api::CreateSubscriptionRequest {
                    subscription: Some(sub_api::CreateSubscription {
                        plan_version_id: (*ids::PLAN_VERSION_NOTION_ID).to_string(),
                        start_date: now.as_proto(),
                        billing_day_anchor: Some(1),
                        customer_id: ids::CUST_SPOTIFY_ID.as_proto(),
                        charge_automatically: Some(false),
                        components: Some(sub_api::CreateSubscriptionComponents {
                            parameterized_components: vec![
                                sub_api::create_subscription_components::ComponentParameterization {
                                    component_id: (*ids::COMP_NOTION_SEATS_ID).to_string(),
                                    billing_period: Some(
                                        meteroid_grpc::meteroid::api::shared::v1::BillingPeriod::Monthly
                                            .into(),
                                    ),
                                    initial_slot_count: Some(1),
                                    ..Default::default()
                                },
                            ],
                            ..Default::default()
                        }),
                        entitlements: vec![
                            meteroid_grpc::meteroid::api::entitlements::v1::EntitlementSpec {
                                feature_id,
                                value: boolean_value(true),
                            },
                        ],
                        ..Default::default()
                    }),
                }))
                .await
        }
    };
    fn assert_archived_rejection<T: std::fmt::Debug>(r: Result<T, tonic::Status>, why: &str) {
        let status = r.expect_err(why);
        assert_eq!(
            status.code(),
            tonic::Code::InvalidArgument,
            "{why}: {status:?}"
        );
        assert!(status.message().contains("archived"), "{why}: {status:?}");
    }

    // Disabled: takes new grants (staging) but resolves nowhere.
    set_feature_status(&clients, &defaulted.id, FeatureStatus::Disabled)
        .await
        .unwrap();
    grant(defaulted.id.clone(), entity_feature(defaulted.id.clone()))
        .await
        .expect("a disabled feature accepts new grants");
    assert!(!resolves(defaulted.id.clone()).await);
    set_feature_status(&clients, &defaulted.id, FeatureStatus::Active)
        .await
        .unwrap();
    assert!(resolves(defaulted.id.clone()).await);

    let plan_version = entity_plan_version(ids::PLAN_VERSION_NOTION_ID.as_proto());
    let customer = entity_customer(ids::CUST_SPOTIFY_ID.as_proto());
    let plan_grant = grant(feature.id.clone(), plan_version.clone())
        .await
        .unwrap();
    grant(feature.id.clone(), customer.clone()).await.unwrap();
    let held_on_plan = grants_on(plan_version.clone()).await;
    let held_by_customer = grants_on(customer.clone()).await;
    assert!(resolves(feature.id.clone()).await);

    // ── Archived: every grant is hidden and kept ──
    set_feature_status(&clients, &feature.id, FeatureStatus::Archived)
        .await
        .unwrap();
    assert!(
        !resolves(feature.id.clone()).await,
        "an archived feature resolves nowhere"
    );
    assert!(grants_on(plan_version.clone()).await.is_empty());
    assert!(grants_on(customer.clone()).await.is_empty());
    let hidden_grant = clients
        .entitlements
        .clone()
        .get_entitlement(GetEntitlementRequest {
            id: plan_grant.id.clone(),
        })
        .await;
    assert_eq!(
        hidden_grant.unwrap_err().code(),
        tonic::Code::NotFound,
        "a hidden grant is not found"
    );
    let edit_hidden = clients
        .entitlements
        .clone()
        .update_entitlement(UpdateEntitlementRequest {
            id: plan_grant.id.clone(),
            value: boolean_value(false),
        })
        .await;
    assert_eq!(edit_hidden.unwrap_err().code(), tonic::Code::NotFound);
    let read = clients
        .entitlements
        .clone()
        .get_feature(GetFeatureRequest {
            id: feature.id.clone(),
        })
        .await
        .expect("an archived feature is readable by id")
        .into_inner()
        .feature
        .unwrap();
    assert_eq!(read.status, FeatureStatus::Archived as i32);

    assert_archived_rejection(
        grant(
            feature.id.clone(),
            entity_plan_version(ids::PLAN_VERSION_PRO_ID.as_proto()),
        )
        .await,
        "no new grant",
    );
    assert_archived_rejection(
        clients
            .entitlements
            .clone()
            .update_feature(UpdateFeatureRequest {
                id: feature.id.clone(),
                name: Some("renamed".into()),
                description: None,
                product_id: None,
                clear_product_id: false,
                config_options: vec![],
                update_config_options: false,
            })
            .await,
        "the feature is read-only",
    );
    let new_sub = subscribe_with_grant(feature.id.clone())
        .await
        .expect("an inline grant of an archived feature does not fail the subscription")
        .into_inner()
        .subscription
        .unwrap();
    assert!(
        !resolves_on_subscription(new_sub.id.clone(), feature.id.clone()).await,
        "a new subscription does not get an archived feature"
    );

    let draft = clients
        .plans
        .clone()
        .copy_version_to_draft(
            meteroid_grpc::meteroid::api::plans::v1::CopyVersionToDraftRequest {
                plan_id: ids::PLAN_NOTION_ID.as_proto(),
                plan_version_id: ids::PLAN_VERSION_NOTION_ID.as_proto(),
            },
        )
        .await
        .expect("copying a plan version that grants an archived feature still works")
        .into_inner()
        .plan_version
        .unwrap();
    let draft_entity = entity_plan_version(draft.id.clone());
    assert!(grants_on(draft_entity.clone()).await.is_empty());

    // ── Restored: every grant comes back as it was ──
    set_feature_status(&clients, &feature.id, FeatureStatus::Active)
        .await
        .unwrap();
    assert!(resolves(feature.id.clone()).await);
    assert_eq!(grants_on(plan_version.clone()).await, held_on_plan);
    assert_eq!(grants_on(customer.clone()).await, held_by_customer);
    assert!(
        grants_on(draft_entity).await.is_empty(),
        "a copy made while archived leaves out the hidden grant"
    );
    assert!(
        grants_on(entity_subscription_proto(new_sub.id.clone()))
            .await
            .is_empty(),
        "an inline grant skipped while archived stays skipped"
    );
    assert!(
        resolves_on_subscription(new_sub.id.clone(), feature.id.clone()).await,
        "that subscription gets the feature through its plan version again"
    );
    grant(
        feature.id.clone(),
        entity_plan_version(ids::PLAN_VERSION_PRO_ID.as_proto()),
    )
    .await
    .expect("a restored feature accepts new grants");
}

#[tokio::test]
async fn test_subscription_override_beats_plan_grant() {
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

    let now = chrono::Utc::now().date_naive();
    let sub = clients
        .subscriptions
        .clone()
        .create_subscription(tonic::Request::new(sub_api::CreateSubscriptionRequest {
            subscription: Some(sub_api::CreateSubscription {
                plan_version_id: (*ids::PLAN_VERSION_NOTION_ID).to_string(),
                start_date: now.as_proto(),
                billing_day_anchor: Some(1),
                customer_id: ids::CUST_SPOTIFY_ID.as_proto(),
                charge_automatically: Some(false),
                components: Some(sub_api::CreateSubscriptionComponents {
                    parameterized_components: vec![
                        sub_api::create_subscription_components::ComponentParameterization {
                            component_id: (*ids::COMP_NOTION_SEATS_ID).to_string(),
                            billing_period: Some(
                                meteroid_grpc::meteroid::api::shared::v1::BillingPeriod::Monthly
                                    .into(),
                            ),
                            initial_slot_count: Some(1),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }),
                ..Default::default()
            }),
        }))
        .await
        .unwrap()
        .into_inner()
        .subscription
        .unwrap();

    let feature = clients
        .entitlements
        .clone()
        .create_feature(CreateFeatureRequest {
            name: "api-quota".into(),
            code: "api-quota".into(),
            description: None,
            product_id: None,
            feature_type: Some(FeatureType {
                inner: Some(feature_type::Inner::Metered(feature_type::MeteredFeature {
                    metric_id: ids::METRIC_BANDWIDTH.as_proto(),
                })),
            }),
            entitlement: None,
        })
        .await
        .unwrap()
        .into_inner()
        .feature
        .unwrap();

    // Plan grants 1000.
    clients
        .entitlements
        .clone()
        .create_entitlement(CreateEntitlementRequest {
            feature_id: feature.id.clone(),
            entity: entity_plan(ids::PLAN_NOTION_ID.as_proto()),
            value: metered_value("1000"),
        })
        .await
        .unwrap();

    // Subscription Override bumps to 9000.
    clients
        .entitlements
        .clone()
        .create_entitlement(CreateEntitlementRequest {
            feature_id: feature.id.clone(),
            entity: entity_subscription_proto(sub.id.clone()),
            value: metered_value("9000"),
        })
        .await
        .unwrap();

    let resolved = clients
        .entitlements
        .clone()
        .get_effective_entitlements(GetEffectiveEntitlementsRequest {
            customer_id: ids::CUST_SPOTIFY_ID.as_proto(),
        })
        .await
        .unwrap()
        .into_inner();

    use meteroid_grpc::meteroid::api::entitlements::v1::effective_entitlement;
    let ent = resolved
        .entitlements
        .iter()
        .find(|e| e.feature.as_ref().is_some_and(|f| f.id == feature.id))
        .expect("feature must resolve");
    if let Some(effective_entitlement::Value::Metered(m)) = &ent.value {
        assert_eq!(m.limit.as_deref(), Some("9000"));
    } else {
        panic!("expected metered value");
    }
}

#[tokio::test]
async fn test_feature_baseline_entitlement_lowest_priority() {
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

    let now = chrono::Utc::now().date_naive();
    clients
        .subscriptions
        .clone()
        .create_subscription(tonic::Request::new(sub_api::CreateSubscriptionRequest {
            subscription: Some(sub_api::CreateSubscription {
                plan_version_id: (*ids::PLAN_VERSION_NOTION_ID).to_string(),
                start_date: now.as_proto(),
                billing_day_anchor: Some(1),
                customer_id: ids::CUST_SPOTIFY_ID.as_proto(),
                charge_automatically: Some(false),
                components: Some(sub_api::CreateSubscriptionComponents {
                    parameterized_components: vec![
                        sub_api::create_subscription_components::ComponentParameterization {
                            component_id: (*ids::COMP_NOTION_SEATS_ID).to_string(),
                            billing_period: Some(
                                meteroid_grpc::meteroid::api::shared::v1::BillingPeriod::Monthly
                                    .into(),
                            ),
                            initial_slot_count: Some(1),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }),
                ..Default::default()
            }),
        }))
        .await
        .unwrap();

    // Boolean feature with baseline grant attached at the Feature entity level.
    let feature = clients
        .entitlements
        .clone()
        .create_feature(CreateFeatureRequest {
            name: "everyone-default".into(),
            code: "everyone-default".into(),
            description: None,
            product_id: None,
            feature_type: Some(FeatureType {
                inner: Some(feature_type::Inner::Boolean(
                    feature_type::BooleanFeature {},
                )),
            }),
            entitlement: None,
        })
        .await
        .unwrap()
        .into_inner()
        .feature
        .unwrap();

    clients
        .entitlements
        .clone()
        .create_entitlement(CreateEntitlementRequest {
            feature_id: feature.id.clone(),
            entity: entity_feature(feature.id.clone()),
            value: boolean_value(true),
        })
        .await
        .unwrap();

    // No higher-priority entitlement — baseline must win.
    let resolved = clients
        .entitlements
        .clone()
        .get_effective_entitlements(GetEffectiveEntitlementsRequest {
            customer_id: ids::CUST_SPOTIFY_ID.as_proto(),
        })
        .await
        .unwrap()
        .into_inner();
    let ent = resolved
        .entitlements
        .iter()
        .find(|e| e.feature.as_ref().is_some_and(|f| f.id == feature.id))
        .expect("baseline must resolve");
    use meteroid_grpc::meteroid::api::entitlements::v1::effective_entitlement;
    assert!(matches!(
        &ent.value,
        Some(effective_entitlement::Value::Boolean(b)) if b.enabled
    ));
}

#[tokio::test]
async fn test_unique_feature_entity_collision_rejected() {
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

    let feature = clients
        .entitlements
        .clone()
        .create_feature(CreateFeatureRequest {
            name: "unique-test".into(),
            code: "unique-test".into(),
            description: None,
            product_id: None,
            feature_type: Some(FeatureType {
                inner: Some(feature_type::Inner::Boolean(
                    feature_type::BooleanFeature {},
                )),
            }),
            entitlement: None,
        })
        .await
        .unwrap()
        .into_inner()
        .feature
        .unwrap();

    let req = CreateEntitlementRequest {
        feature_id: feature.id.clone(),
        entity: entity_plan_version(ids::PLAN_VERSION_NOTION_ID.as_proto()),
        value: boolean_value(true),
    };

    clients
        .entitlements
        .clone()
        .create_entitlement(req.clone())
        .await
        .unwrap();

    // Second insert against the same (feature, entity) pair must fail.
    let err = clients
        .entitlements
        .clone()
        .create_entitlement(req)
        .await
        .expect_err("duplicate (feature_id, entity_id, entity_type) must collide on UNIQUE");
    assert!(
        matches!(
            err.code(),
            tonic::Code::AlreadyExists | tonic::Code::InvalidArgument | tonic::Code::Internal
        ),
        "expected a duplicate-related error, got: {err:?}"
    );
}

#[tokio::test]
async fn test_value_variant_must_match_feature_type() {
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

    // Boolean feature, but caller sends a Metered value → must reject.
    let bool_feature = clients
        .entitlements
        .clone()
        .create_feature(CreateFeatureRequest {
            name: "bool-feat".into(),
            code: "bool-feat".into(),
            description: None,
            product_id: None,
            feature_type: Some(FeatureType {
                inner: Some(feature_type::Inner::Boolean(
                    feature_type::BooleanFeature {},
                )),
            }),
            entitlement: None,
        })
        .await
        .unwrap()
        .into_inner()
        .feature
        .unwrap();

    let err = clients
        .entitlements
        .clone()
        .create_entitlement(CreateEntitlementRequest {
            feature_id: bool_feature.id.clone(),
            entity: entity_plan_version(ids::PLAN_VERSION_NOTION_ID.as_proto()),
            value: metered_value("100"),
        })
        .await
        .expect_err("metered value on boolean feature must be rejected");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);

    // Metered feature, but caller sends a Boolean value → also reject.
    let metered_feature = clients
        .entitlements
        .clone()
        .create_feature(CreateFeatureRequest {
            name: "metered-feat".into(),
            code: "metered-feat".into(),
            description: None,
            product_id: None,
            feature_type: Some(FeatureType {
                inner: Some(feature_type::Inner::Metered(feature_type::MeteredFeature {
                    metric_id: ids::METRIC_BANDWIDTH.as_proto(),
                })),
            }),
            entitlement: None,
        })
        .await
        .unwrap()
        .into_inner()
        .feature
        .unwrap();

    let err = clients
        .entitlements
        .clone()
        .create_entitlement(CreateEntitlementRequest {
            feature_id: metered_feature.id.clone(),
            entity: entity_plan_version(ids::PLAN_VERSION_NOTION_ID.as_proto()),
            value: boolean_value(true),
        })
        .await
        .expect_err("boolean value on metered feature must be rejected");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
}

/// Verify that inline `EntitlementSpec`s on `CreateSubscription` are persisted and then
/// returned via `ListEntitlementsByEntity`.
#[tokio::test]
async fn test_inline_subscription_entitlement_creation() {
    use meteroid_grpc::meteroid::api::entitlements::v1::{
        EntitlementEntity, EntitlementSpec, ListEntitlementsByEntityRequest, entitlement_entity,
        entitlement_value,
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

    // Create a boolean feature to use as the inline entitlement target.
    let feature = clients
        .entitlements
        .clone()
        .create_feature(CreateFeatureRequest {
            name: "inline-bool-feature".into(),
            code: "inline-bool-feature".into(),
            description: None,
            product_id: None,
            feature_type: Some(FeatureType {
                inner: Some(feature_type::Inner::Boolean(
                    feature_type::BooleanFeature {},
                )),
            }),
            entitlement: None,
        })
        .await
        .unwrap()
        .into_inner()
        .feature
        .unwrap();

    let now = chrono::Utc::now().date_naive();
    use meteroid_app::conversions::ProtoConv;

    // Create a subscription with one inline boolean entitlement spec.
    let sub = clients
        .subscriptions
        .clone()
        .create_subscription(tonic::Request::new(sub_api::CreateSubscriptionRequest {
            subscription: Some(sub_api::CreateSubscription {
                plan_version_id: (*ids::PLAN_VERSION_NOTION_ID).to_string(),
                start_date: now.as_proto(),
                billing_day_anchor: Some(1),
                customer_id: ids::CUST_SPOTIFY_ID.as_proto(),
                charge_automatically: Some(false),
                components: Some(sub_api::CreateSubscriptionComponents {
                    parameterized_components: vec![
                        sub_api::create_subscription_components::ComponentParameterization {
                            component_id: (*ids::COMP_NOTION_SEATS_ID).to_string(),
                            billing_period: Some(
                                meteroid_grpc::meteroid::api::shared::v1::BillingPeriod::Monthly
                                    .into(),
                            ),
                            initial_slot_count: Some(1),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }),
                entitlements: vec![EntitlementSpec {
                    feature_id: feature.id.clone(),
                    value: boolean_value(true),
                }],
                ..Default::default()
            }),
        }))
        .await
        .unwrap()
        .into_inner()
        .subscription
        .unwrap();

    // List entitlements for the subscription entity and assert the boolean entitlement is present.
    let listed = clients
        .entitlements
        .clone()
        .list_entitlements_by_entity(ListEntitlementsByEntityRequest {
            entity: Some(EntitlementEntity {
                entity_id: Some(entitlement_entity::EntityId::SubscriptionId(sub.id.clone())),
            }),
        })
        .await
        .unwrap()
        .into_inner();

    let ent = listed
        .entitlements
        .iter()
        .find(|e| e.feature_id == feature.id)
        .expect("inline entitlement must be present after subscription creation");

    // The value must be the boolean enabled=true we set inline.
    match &ent.value {
        Some(v) => match &v.value {
            Some(entitlement_value::Value::BooleanValue(b)) => {
                assert!(b.enabled, "inline entitlement enabled must be true");
            }
            other => panic!("expected boolean value, got: {:?}", other),
        },
        None => panic!("entitlement value must be set"),
    }
}

/// Verify that a feature-level metered entitlement with `enabled: false` is still returned by
/// `GetEffectiveEntitlements` (not filtered out), and that a higher-priority subscription
/// entitlement with `enabled: true` overrides it.
#[tokio::test]
async fn test_disabled_entitlement_resolved_and_overrideable() {
    use meteroid_grpc::meteroid::api::entitlements::v1::effective_entitlement;

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

    // Create a subscription for Spotify on the Notion plan.
    let now = chrono::Utc::now().date_naive();
    let sub = clients
        .subscriptions
        .clone()
        .create_subscription(tonic::Request::new(sub_api::CreateSubscriptionRequest {
            subscription: Some(sub_api::CreateSubscription {
                plan_version_id: (*ids::PLAN_VERSION_NOTION_ID).to_string(),
                start_date: now.as_proto(),
                billing_day_anchor: Some(1),
                customer_id: ids::CUST_SPOTIFY_ID.as_proto(),
                charge_automatically: Some(false),
                components: Some(sub_api::CreateSubscriptionComponents {
                    parameterized_components: vec![
                        sub_api::create_subscription_components::ComponentParameterization {
                            component_id: (*ids::COMP_NOTION_SEATS_ID).to_string(),
                            billing_period: Some(
                                meteroid_grpc::meteroid::api::shared::v1::BillingPeriod::Monthly
                                    .into(),
                            ),
                            initial_slot_count: Some(1),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }),
                ..Default::default()
            }),
        }))
        .await
        .unwrap()
        .into_inner()
        .subscription
        .unwrap();

    // Create a boolean feature.
    let feature = clients
        .entitlements
        .clone()
        .create_feature(CreateFeatureRequest {
            name: "disabled-test-feat".into(),
            code: "disabled-test-feat".into(),
            description: None,
            product_id: None,
            feature_type: Some(FeatureType {
                inner: Some(feature_type::Inner::Metered(feature_type::MeteredFeature {
                    metric_id: ids::METRIC_BANDWIDTH.as_proto(),
                })),
            }),
            entitlement: None,
        })
        .await
        .unwrap()
        .into_inner()
        .feature
        .unwrap();

    // Feature-level metered entitlement with enabled: false (lowest priority, acts as baseline).
    clients
        .entitlements
        .clone()
        .create_entitlement(CreateEntitlementRequest {
            feature_id: feature.id.clone(),
            entity: entity_feature(feature.id.clone()),
            value: Some(EntitlementValue {
                value: Some(entitlement_value::Value::MeteredValue(
                    entitlement_value::MeteredValue {
                        limit: Some("100".into()),
                        reset_period: None,
                        enabled: false,
                        group_by: None,
                        ..Default::default()
                    },
                )),
            }),
        })
        .await
        .unwrap();

    // Resolve: the feature-level disabled (enabled:false) entitlement must appear (not be filtered out).
    let resolved = clients
        .entitlements
        .clone()
        .get_effective_entitlements(GetEffectiveEntitlementsRequest {
            customer_id: ids::CUST_SPOTIFY_ID.as_proto(),
        })
        .await
        .unwrap()
        .into_inner();

    let ent = resolved
        .entitlements
        .iter()
        .find(|e| e.feature.as_ref().is_some_and(|f| f.id == feature.id))
        .expect("disabled entitlement must still be returned by the resolver");

    assert!(
        matches!(
            &ent.value,
            Some(effective_entitlement::Value::Metered(m)) if !m.enabled
        ),
        "resolved entitlement must carry enabled=false"
    );

    // Now create a higher-priority subscription entitlement with enabled: true.
    clients
        .entitlements
        .clone()
        .create_entitlement(CreateEntitlementRequest {
            feature_id: feature.id.clone(),
            entity: entity_subscription_proto(sub.id.clone()),
            value: Some(EntitlementValue {
                value: Some(entitlement_value::Value::MeteredValue(
                    entitlement_value::MeteredValue {
                        limit: Some("100".into()),
                        reset_period: None,
                        enabled: true,
                        group_by: None,
                        ..Default::default()
                    },
                )),
            }),
        })
        .await
        .unwrap();

    // Re-resolve: the subscription-level entitlement (enabled: true) must win.
    let resolved2 = clients
        .entitlements
        .clone()
        .get_effective_entitlements(GetEffectiveEntitlementsRequest {
            customer_id: ids::CUST_SPOTIFY_ID.as_proto(),
        })
        .await
        .unwrap()
        .into_inner();

    let ent2 = resolved2
        .entitlements
        .iter()
        .find(|e| e.feature.as_ref().is_some_and(|f| f.id == feature.id))
        .expect("feature must still resolve after subscription override");

    assert!(
        matches!(
            &ent2.value,
            Some(effective_entitlement::Value::Metered(m)) if m.enabled
        ),
        "subscription-level enabled=true must override feature-level enabled=false"
    );
}

/// Regression: a customer with **no active subscription** must still receive tenant-global
/// feature-level defaults. The resolver used to short-circuit to an empty result whenever the
/// customer had no subscription, so features that are enabled by default (e.g. the
/// operator's own gates) surfaced as absent/false even though a feature-level entitlement granted them.
#[tokio::test]
async fn test_feature_level_defaults_resolve_without_subscription() {
    use meteroid_grpc::meteroid::api::entitlements::v1::effective_entitlement;

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

    // Tenant-global boolean feature (product_id: None) with a feature-level entitlement
    // enabled=true — the "on by default for every customer" baseline.
    let feature = clients
        .entitlements
        .clone()
        .create_feature(CreateFeatureRequest {
            name: "default-on-feat".into(),
            code: "default-on-feat".into(),
            description: None,
            product_id: None,
            feature_type: Some(FeatureType {
                inner: Some(feature_type::Inner::Boolean(
                    feature_type::BooleanFeature {},
                )),
            }),
            entitlement: None,
        })
        .await
        .unwrap()
        .into_inner()
        .feature
        .unwrap();

    clients
        .entitlements
        .clone()
        .create_entitlement(CreateEntitlementRequest {
            feature_id: feature.id.clone(),
            entity: entity_feature(feature.id.clone()),
            value: boolean_value(true),
        })
        .await
        .unwrap();

    // CUST_SPOTIFY_ID exists at SeedLevel::PLANS but has no subscription.
    let resolved = clients
        .entitlements
        .clone()
        .get_effective_entitlements(GetEffectiveEntitlementsRequest {
            customer_id: ids::CUST_SPOTIFY_ID.as_proto(),
        })
        .await
        .unwrap()
        .into_inner();

    let ent = resolved
        .entitlements
        .iter()
        .find(|e| e.feature.as_ref().is_some_and(|f| f.id == feature.id))
        .expect("feature-level default must resolve for a customer with no subscription");

    assert!(
        matches!(
            &ent.value,
            Some(effective_entitlement::Value::Boolean(b)) if b.enabled
        ),
        "feature-level default (enabled=true) must surface as enabled for an unsubscribed customer"
    );
}

/// Verify that `GetResolvedEntitlementsForSubscription` reflects the full priority chain:
/// feature-level baseline → plan-level grant → subscription-level override.
#[tokio::test]
async fn test_resolved_entitlements_for_subscription_includes_chain() {
    use meteroid_grpc::meteroid::api::entitlements::v1::{
        GetResolvedForSubscriptionRequest, entitlement_entity, entitlement_value,
        resolved_entitlement,
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

    // Step 1: Create a boolean feature with a feature-level entitlement (enabled=true).
    let feature = clients
        .entitlements
        .clone()
        .create_feature(CreateFeatureRequest {
            name: "chain-test-bool".into(),
            code: "chain-test-bool".into(),
            description: None,
            product_id: None,
            feature_type: Some(FeatureType {
                inner: Some(feature_type::Inner::Boolean(
                    feature_type::BooleanFeature {},
                )),
            }),
            entitlement: None,
        })
        .await
        .unwrap()
        .into_inner()
        .feature
        .unwrap();

    // Feature-level baseline entitlement (lowest priority).
    clients
        .entitlements
        .clone()
        .create_entitlement(CreateEntitlementRequest {
            feature_id: feature.id.clone(),
            entity: entity_feature(feature.id.clone()),
            value: Some(EntitlementValue {
                value: Some(entitlement_value::Value::BooleanValue(
                    entitlement_value::BooleanValue { enabled: true },
                )),
            }),
        })
        .await
        .unwrap();

    // Step 2: Plan-level boolean entitlement (higher than feature-level).
    clients
        .entitlements
        .clone()
        .create_entitlement(CreateEntitlementRequest {
            feature_id: feature.id.clone(),
            entity: entity_plan(ids::PLAN_NOTION_ID.as_proto()),
            value: Some(EntitlementValue {
                value: Some(entitlement_value::Value::BooleanValue(
                    entitlement_value::BooleanValue { enabled: true },
                )),
            }),
        })
        .await
        .unwrap();

    // Step 3: Create a subscription for Spotify on the Notion plan.
    let now = chrono::Utc::now().date_naive();
    let sub = clients
        .subscriptions
        .clone()
        .create_subscription(tonic::Request::new(sub_api::CreateSubscriptionRequest {
            subscription: Some(sub_api::CreateSubscription {
                plan_version_id: (*ids::PLAN_VERSION_NOTION_ID).to_string(),
                start_date: now.as_proto(),
                billing_day_anchor: Some(1),
                customer_id: ids::CUST_SPOTIFY_ID.as_proto(),
                charge_automatically: Some(false),
                components: Some(sub_api::CreateSubscriptionComponents {
                    parameterized_components: vec![
                        sub_api::create_subscription_components::ComponentParameterization {
                            component_id: (*ids::COMP_NOTION_SEATS_ID).to_string(),
                            billing_period: Some(
                                meteroid_grpc::meteroid::api::shared::v1::BillingPeriod::Monthly
                                    .into(),
                            ),
                            initial_slot_count: Some(1),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }),
                ..Default::default()
            }),
        }))
        .await
        .unwrap()
        .into_inner()
        .subscription
        .unwrap();

    // Step 4: Subscription-level override entitlement (highest priority, enabled=false).
    clients
        .entitlements
        .clone()
        .create_entitlement(CreateEntitlementRequest {
            feature_id: feature.id.clone(),
            entity: entity_subscription_proto(sub.id.clone()),
            value: Some(EntitlementValue {
                value: Some(entitlement_value::Value::BooleanValue(
                    entitlement_value::BooleanValue { enabled: false },
                )),
            }),
        })
        .await
        .unwrap();

    // Step 5: Call GetResolvedEntitlementsForSubscription.
    let resolved = clients
        .entitlements
        .clone()
        .get_resolved_entitlements_for_subscription(GetResolvedForSubscriptionRequest {
            subscription_id: sub.id.clone(),
        })
        .await
        .unwrap()
        .into_inner();

    // Step 6: Assert exactly 1 row, boolean {enabled: false}, origin=Subscription.
    let matching: Vec<_> = resolved
        .entitlements
        .iter()
        .filter(|e| e.feature.as_ref().is_some_and(|f| f.id == feature.id))
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "expected exactly 1 resolved entitlement for the feature"
    );
    let ent = matching[0];

    assert!(
        matches!(
            &ent.value,
            Some(resolved_entitlement::Value::Boolean(b)) if !b.enabled
        ),
        "subscription-level override must yield enabled=false; got: {:?}",
        ent.value
    );

    // Origin must be the subscription.
    assert!(
        matches!(
            &ent.origin,
            Some(ResolvedOrigin { entity: Some(EntitlementEntity { entity_id: Some(entitlement_entity::EntityId::SubscriptionId(id)) }), .. })
            if id == &sub.id
        ),
        "origin must be the subscription entity; got: {:?}",
        ent.origin
    );
}

/// Verify that `GetResolvedEntitlementsForAddOn` surfaces both AddOn-level entitlements
/// and product-scoped feature-level baselines.
#[tokio::test]
async fn test_resolved_entitlements_for_addon_includes_product_features() {
    use meteroid_grpc::meteroid::api::entitlements::v1::{
        GetResolvedForAddOnRequest, entitlement_entity, entitlement_value, resolved_entitlement,
    };

    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;
    // SeedLevel::PLANS is required because METRIC_BANDWIDTH is seeded as part of the METERS seed.
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::PLANS).await;

    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "TESTORG",
        "testslug",
    );

    // Step 1: Create a new add-on (which also creates a product) as the target entity.
    let add_on = clients
        .add_ons
        .clone()
        .create_add_on(meteroid_grpc::meteroid::api::addons::v1::CreateAddOnRequest {
            name: "addon-ent-test".into(),
            product: Some(meteroid_grpc::meteroid::api::components::v1::ProductRef {
                r#ref: Some(
                    meteroid_grpc::meteroid::api::components::v1::product_ref::Ref::NewProduct(
                        meteroid_grpc::meteroid::api::components::v1::NewProduct {
                            name: "Addon-Ent Product".into(),
                            fee_type: meteroid_grpc::meteroid::api::prices::v1::FeeType::Rate
                                .into(),
                            fee_structure: Some(
                                meteroid_grpc::meteroid::api::prices::v1::FeeStructure {
                                    structure: Some(
                                        meteroid_grpc::meteroid::api::prices::v1::fee_structure::Structure::Rate(
                                            meteroid_grpc::meteroid::api::prices::v1::fee_structure::RateStructure {},
                                        ),
                                    ),
                                },
                            ),
                        },
                    ),
                ),
            }),
            price: Some(meteroid_grpc::meteroid::api::components::v1::PriceEntry {
                entry: Some(
                    meteroid_grpc::meteroid::api::components::v1::price_entry::Entry::NewPrice(
                        meteroid_grpc::meteroid::api::components::v1::PriceInput {
                            cadence: meteroid_grpc::meteroid::api::shared::v1::BillingPeriod::Monthly
                                .into(),
                            currency: "USD".into(),
                            pricing: Some(
                                meteroid_grpc::meteroid::api::components::v1::price_input::Pricing::RatePricing(
                                    meteroid_grpc::meteroid::api::prices::v1::RatePricing {
                                        rate: "9.99".into(),
                                    },
                                ),
                            ),
                            credit_unit: None,
                        },
                    ),
                ),
            }),
            description: None,
            self_serviceable: false,
            max_instances_per_subscription: None,
            product_family_local_id: None,
            entitlements: vec![],
        })
        .await
        .unwrap()
        .into_inner()
        .add_on
        .unwrap();

    let product_id = add_on.product_id.clone();

    // Step 2: Create feature F1 under the add-on's product with a feature-level boolean entitlement.
    let f1 = clients
        .entitlements
        .clone()
        .create_feature(CreateFeatureRequest {
            name: "addon-product-bool-f1".into(),
            code: "addon-product-bool-f1".into(),
            description: None,
            product_id: Some(product_id.clone()),
            feature_type: Some(FeatureType {
                inner: Some(feature_type::Inner::Boolean(
                    feature_type::BooleanFeature {},
                )),
            }),
            entitlement: None,
        })
        .await
        .unwrap()
        .into_inner()
        .feature
        .unwrap();

    // Feature-level entitlement for F1 (enabled=true).
    clients
        .entitlements
        .clone()
        .create_entitlement(CreateEntitlementRequest {
            feature_id: f1.id.clone(),
            entity: entity_feature(f1.id.clone()),
            value: Some(EntitlementValue {
                value: Some(entitlement_value::Value::BooleanValue(
                    entitlement_value::BooleanValue { enabled: true },
                )),
            }),
        })
        .await
        .unwrap();

    // Step 3: Create feature F2 under the same product (metered).
    let f2 = clients
        .entitlements
        .clone()
        .create_feature(CreateFeatureRequest {
            name: "addon-product-metered-f2".into(),
            code: "addon-product-metered-f2".into(),
            description: None,
            product_id: Some(product_id.clone()),
            feature_type: Some(FeatureType {
                inner: Some(feature_type::Inner::Metered(feature_type::MeteredFeature {
                    metric_id: ids::METRIC_BANDWIDTH.as_proto(),
                })),
            }),
            entitlement: None,
        })
        .await
        .unwrap()
        .into_inner()
        .feature
        .unwrap();

    // AddOn-level metered entitlement for F2 with limit=1000.
    clients
        .entitlements
        .clone()
        .create_entitlement(CreateEntitlementRequest {
            feature_id: f2.id.clone(),
            entity: Some(EntitlementEntity {
                entity_id: Some(entitlement_entity::EntityId::AddOnId(add_on.id.clone())),
            }),
            value: metered_value("1000"),
        })
        .await
        .unwrap();

    // Step 4: Call GetResolvedEntitlementsForAddOn.
    let resolved = clients
        .entitlements
        .clone()
        .get_resolved_entitlements_for_add_on(GetResolvedForAddOnRequest {
            add_on_id: add_on.id.clone(),
        })
        .await
        .unwrap()
        .into_inner();

    // Step 5: Assert exactly 2 rows.
    assert_eq!(
        resolved.entitlements.len(),
        2,
        "expected 2 resolved entitlements (F1 feature-baseline + F2 addon-level); got: {:?}",
        resolved
            .entitlements
            .iter()
            .map(|e| e.feature.as_ref().map(|f| &f.name))
            .collect::<Vec<_>>()
    );

    // F1's row: boolean, origin = Feature(F1.id).
    let f1_ent = resolved
        .entitlements
        .iter()
        .find(|e| e.feature.as_ref().is_some_and(|f| f.id == f1.id))
        .expect("F1 must be in resolved entitlements");

    assert!(
        matches!(
            &f1_ent.value,
            Some(resolved_entitlement::Value::Boolean(b)) if b.enabled
        ),
        "F1 must resolve as boolean enabled=true"
    );
    assert!(
        matches!(
            &f1_ent.origin,
            Some(ResolvedOrigin { entity: Some(EntitlementEntity { entity_id: Some(entitlement_entity::EntityId::FeatureId(id)) }), .. })
            if id == &f1.id
        ),
        "F1 origin must be Feature(F1.id); got: {:?}",
        f1_ent.origin
    );

    // F2's row: metered with limit=1000, origin = AddOn(A.id).
    let f2_ent = resolved
        .entitlements
        .iter()
        .find(|e| e.feature.as_ref().is_some_and(|f| f.id == f2.id))
        .expect("F2 must be in resolved entitlements");

    assert!(
        matches!(
            &f2_ent.value,
            Some(resolved_entitlement::Value::Metered(m)) if m.limit.as_deref() == Some("1000")
        ),
        "F2 must resolve as metered with limit=1000; got: {:?}",
        f2_ent.value
    );
    assert!(
        matches!(
            &f2_ent.origin,
            Some(ResolvedOrigin { entity: Some(EntitlementEntity { entity_id: Some(entitlement_entity::EntityId::AddOnId(id)) }), .. })
            if id == &add_on.id
        ),
        "F2 origin must be AddOn(A.id); got: {:?}",
        f2_ent.origin
    );
}

/// Verify that `BatchCreateEntitlements` uses INSERT … ON CONFLICT DO NOTHING and returns
/// only the newly inserted rows, silently skipping pre-existing (feature, entity) pairs.
#[tokio::test]
async fn test_batch_create_entitlements_skips_existing() {
    use meteroid_grpc::meteroid::api::entitlements::v1::{
        BatchCreateEntitlementsRequest, EntitlementEntity, EntitlementSpec,
        ListEntitlementsByEntityRequest, entitlement_entity,
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

    // Step 1: Create 3 boolean features.
    let mut features = Vec::with_capacity(3);
    for i in 0..3usize {
        let feat = clients
            .entitlements
            .clone()
            .create_feature(CreateFeatureRequest {
                name: format!("batch-skip-feat-{}", i),
                code: format!("batch-skip-feat-{}", i),
                description: None,
                product_id: None,
                feature_type: Some(FeatureType {
                    inner: Some(feature_type::Inner::Boolean(
                        feature_type::BooleanFeature {},
                    )),
                }),
                entitlement: None,
            })
            .await
            .unwrap()
            .into_inner()
            .feature
            .unwrap();
        features.push(feat);
    }

    // Step 2: Create a subscription.
    let now = chrono::Utc::now().date_naive();
    let sub = clients
        .subscriptions
        .clone()
        .create_subscription(tonic::Request::new(sub_api::CreateSubscriptionRequest {
            subscription: Some(sub_api::CreateSubscription {
                plan_version_id: (*ids::PLAN_VERSION_NOTION_ID).to_string(),
                start_date: now.as_proto(),
                billing_day_anchor: Some(1),
                customer_id: ids::CUST_SPOTIFY_ID.as_proto(),
                charge_automatically: Some(false),
                components: Some(sub_api::CreateSubscriptionComponents {
                    parameterized_components: vec![
                        sub_api::create_subscription_components::ComponentParameterization {
                            component_id: (*ids::COMP_NOTION_SEATS_ID).to_string(),
                            billing_period: Some(
                                meteroid_grpc::meteroid::api::shared::v1::BillingPeriod::Monthly
                                    .into(),
                            ),
                            initial_slot_count: Some(1),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }),
                ..Default::default()
            }),
        }))
        .await
        .unwrap()
        .into_inner()
        .subscription
        .unwrap();

    // Step 3: Pre-insert entitlement for features[0] via CreateEntitlement.
    clients
        .entitlements
        .clone()
        .create_entitlement(CreateEntitlementRequest {
            feature_id: features[0].id.clone(),
            entity: entity_subscription_proto(sub.id.clone()),
            value: boolean_value(true),
        })
        .await
        .unwrap();

    // Step 4: BatchCreateEntitlements for all 3 features.
    let batch_resp = clients
        .entitlements
        .clone()
        .batch_create_entitlements(BatchCreateEntitlementsRequest {
            entity: Some(EntitlementEntity {
                entity_id: Some(entitlement_entity::EntityId::SubscriptionId(sub.id.clone())),
            }),
            specs: features
                .iter()
                .map(|f| EntitlementSpec {
                    feature_id: f.id.clone(),
                    value: boolean_value(true),
                })
                .collect(),
        })
        .await
        .unwrap()
        .into_inner();

    // Step 5: Assert response `created` has exactly 2 entries (features[1] and features[2]).
    assert_eq!(
        batch_resp.created.len(),
        2,
        "BatchCreateEntitlements must return only the 2 newly inserted rows; got: {}",
        batch_resp.created.len()
    );

    let created_feature_ids: std::collections::HashSet<_> = batch_resp
        .created
        .iter()
        .map(|e| e.feature_id.as_str())
        .collect();
    assert!(
        !created_feature_ids.contains(features[0].id.as_str()),
        "features[0] was pre-existing and must not appear in the created response"
    );
    assert!(
        created_feature_ids.contains(features[1].id.as_str()),
        "features[1] must be in created response"
    );
    assert!(
        created_feature_ids.contains(features[2].id.as_str()),
        "features[2] must be in created response"
    );

    // Step 6 (optional): ListEntitlementsByEntity must return 3 total.
    let listed = clients
        .entitlements
        .clone()
        .list_entitlements_by_entity(ListEntitlementsByEntityRequest {
            entity: Some(EntitlementEntity {
                entity_id: Some(entitlement_entity::EntityId::SubscriptionId(sub.id.clone())),
            }),
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        listed.entitlements.len(),
        3,
        "after batch create, 3 entitlements must exist for the subscription"
    );
}

/// Smoke test: features belonging to different products (and one tenant-global feature)
/// are resolved with the correct `EffectiveEntitlement.feature.product` metadata.
#[tokio::test]
async fn test_product_grouping_in_effective_entitlements() {
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

    // Create a subscription for Spotify on the Starter plan — this plan has components from
    // BOTH PRODUCT_PLATFORM_FEE_ID (rate) AND PRODUCT_SEATS_ID (slot), so features tied to
    // either product will be in scope for resolution.
    let now = chrono::Utc::now().date_naive();
    clients
        .subscriptions
        .clone()
        .create_subscription(tonic::Request::new(sub_api::CreateSubscriptionRequest {
            subscription: Some(sub_api::CreateSubscription {
                plan_version_id: (*ids::PLAN_VERSION_STARTER_ID).to_string(),
                start_date: now.as_proto(),
                billing_day_anchor: Some(1),
                customer_id: ids::CUST_SPOTIFY_ID.as_proto(),
                charge_automatically: Some(false),
                components: Some(sub_api::CreateSubscriptionComponents {
                    parameterized_components: vec![
                        // Rate component (PRODUCT_PLATFORM_FEE_ID) — no slot count needed.
                        sub_api::create_subscription_components::ComponentParameterization {
                            component_id: (*ids::COMP_STARTER_PLATFORM_FEE_ID).to_string(),
                            billing_period: Some(
                                meteroid_grpc::meteroid::api::shared::v1::BillingPeriod::Monthly
                                    .into(),
                            ),
                            ..Default::default()
                        },
                        // Slot component (PRODUCT_SEATS_ID) — needs initial_slot_count.
                        sub_api::create_subscription_components::ComponentParameterization {
                            component_id: (*ids::COMP_STARTER_SEATS_ID).to_string(),
                            billing_period: Some(
                                meteroid_grpc::meteroid::api::shared::v1::BillingPeriod::Monthly
                                    .into(),
                            ),
                            initial_slot_count: Some(1),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }),
                ..Default::default()
            }),
        }))
        .await
        .unwrap();

    // Feature A — belongs to PRODUCT_PLATFORM_FEE_ID (in scope via Starter plan rate).
    let feat_a = clients
        .entitlements
        .clone()
        .create_feature(CreateFeatureRequest {
            name: "product-feat-a".into(),
            code: "product-feat-a".into(),
            description: None,
            product_id: Some(ids::PRODUCT_PLATFORM_FEE_ID.as_proto()),
            feature_type: Some(FeatureType {
                inner: Some(feature_type::Inner::Boolean(
                    feature_type::BooleanFeature {},
                )),
            }),
            entitlement: None,
        })
        .await
        .unwrap()
        .into_inner()
        .feature
        .unwrap();

    // Feature B — belongs to PRODUCT_SEATS_ID (in scope via Starter plan slot).
    let feat_b = clients
        .entitlements
        .clone()
        .create_feature(CreateFeatureRequest {
            name: "product-feat-b".into(),
            code: "product-feat-b".into(),
            description: None,
            product_id: Some(ids::PRODUCT_SEATS_ID.as_proto()),
            feature_type: Some(FeatureType {
                inner: Some(feature_type::Inner::Boolean(
                    feature_type::BooleanFeature {},
                )),
            }),
            entitlement: None,
        })
        .await
        .unwrap()
        .into_inner()
        .feature
        .unwrap();

    // Feature C — tenant-global (no product), always in scope.
    let feat_c = clients
        .entitlements
        .clone()
        .create_feature(CreateFeatureRequest {
            name: "global-feat-c".into(),
            code: "global-feat-c".into(),
            description: None,
            product_id: None,
            feature_type: Some(FeatureType {
                inner: Some(feature_type::Inner::Boolean(
                    feature_type::BooleanFeature {},
                )),
            }),
            entitlement: None,
        })
        .await
        .unwrap()
        .into_inner()
        .feature
        .unwrap();

    // Grant all three at the plan-version (Starter) level.
    for feat in [&feat_a, &feat_b, &feat_c] {
        clients
            .entitlements
            .clone()
            .create_entitlement(CreateEntitlementRequest {
                feature_id: feat.id.clone(),
                entity: entity_plan_version(ids::PLAN_VERSION_STARTER_ID.as_proto()),
                value: boolean_value(true),
            })
            .await
            .unwrap();
    }

    // Resolve effective entitlements for the customer.
    let resolved = clients
        .entitlements
        .clone()
        .get_effective_entitlements(GetEffectiveEntitlementsRequest {
            customer_id: ids::CUST_SPOTIFY_ID.as_proto(),
        })
        .await
        .unwrap()
        .into_inner();

    // Helper: find by feature id.
    let find = |feat_id: &str| {
        resolved
            .entitlements
            .iter()
            .find(|e| e.feature.as_ref().map(|f| f.id.as_str()) == Some(feat_id))
            .unwrap_or_else(|| panic!("expected feature {} in resolved entitlements", feat_id))
    };

    let ent_a = find(&feat_a.id);
    let ent_b = find(&feat_b.id);
    let ent_c = find(&feat_c.id);

    // Feature A should carry PRODUCT_PLATFORM_FEE_ID in its product ref.
    assert_eq!(
        ent_a
            .feature
            .as_ref()
            .and_then(|f| f.product.as_ref())
            .map(|p| p.id.as_str()),
        Some(ids::PRODUCT_PLATFORM_FEE_ID.as_proto().as_str()),
        "feat_a must be associated with PRODUCT_PLATFORM_FEE_ID"
    );

    // Feature B should carry PRODUCT_SEATS_ID.
    assert_eq!(
        ent_b
            .feature
            .as_ref()
            .and_then(|f| f.product.as_ref())
            .map(|p| p.id.as_str()),
        Some(ids::PRODUCT_SEATS_ID.as_proto().as_str()),
        "feat_b must be associated with PRODUCT_SEATS_ID"
    );

    // Feature C is tenant-global — product must be None.
    assert!(
        ent_c
            .feature
            .as_ref()
            .and_then(|f| f.product.as_ref())
            .is_none(),
        "global feat_c must have no product association"
    );
}

// ── Algorithm conformance: priority ordering, permissive merge, plan_version_add_on ─────

/// Helper: create an add-on with a new product (rate-priced, monthly), optional single-instance.
/// `currency` must match the plan version it will be attached to (seed uses "EUR").
async fn make_single_instance_add_on(
    clients: &meteroid_it::clients::AllClients,
    name: &str,
    product_name: &str,
    currency: &str,
    max_instances: Option<i32>,
) -> meteroid_grpc::meteroid::api::addons::v1::AddOn {
    clients
        .add_ons
        .clone()
        .create_add_on(meteroid_grpc::meteroid::api::addons::v1::CreateAddOnRequest {
            name: name.into(),
            product: Some(meteroid_grpc::meteroid::api::components::v1::ProductRef {
                r#ref: Some(
                    meteroid_grpc::meteroid::api::components::v1::product_ref::Ref::NewProduct(
                        meteroid_grpc::meteroid::api::components::v1::NewProduct {
                            name: product_name.into(),
                            fee_type: meteroid_grpc::meteroid::api::prices::v1::FeeType::Rate
                                .into(),
                            fee_structure: Some(
                                meteroid_grpc::meteroid::api::prices::v1::FeeStructure {
                                    structure: Some(
                                        meteroid_grpc::meteroid::api::prices::v1::fee_structure::Structure::Rate(
                                            meteroid_grpc::meteroid::api::prices::v1::fee_structure::RateStructure {},
                                        ),
                                    ),
                                },
                            ),
                        },
                    ),
                ),
            }),
            price: Some(meteroid_grpc::meteroid::api::components::v1::PriceEntry {
                entry: Some(
                    meteroid_grpc::meteroid::api::components::v1::price_entry::Entry::NewPrice(
                        meteroid_grpc::meteroid::api::components::v1::PriceInput {
                            cadence: meteroid_grpc::meteroid::api::shared::v1::BillingPeriod::Monthly
                                .into(),
                            currency: currency.into(),
                            pricing: Some(
                                meteroid_grpc::meteroid::api::components::v1::price_input::Pricing::RatePricing(
                                    meteroid_grpc::meteroid::api::prices::v1::RatePricing {
                                        rate: "1.00".into(),
                                    },
                                ),
                            ),
                            credit_unit: None,
                        },
                    ),
                ),
            }),
            description: None,
            self_serviceable: false,
            max_instances_per_subscription: max_instances,
            product_family_local_id: None,
            entitlements: vec![],
        })
        .await
        .unwrap()
        .into_inner()
        .add_on
        .unwrap()
}

/// Algorithm rule §1 + priority swap: a PlanVersion-level Override beats an AddOn-level Override
/// for the same feature (priority chain: ... AddOn(2) < PlanVersion(3) < Subscription(4)).
#[tokio::test]
async fn test_plan_version_overrides_addon_entitlement() {
    use meteroid_grpc::meteroid::api::entitlements::v1::{
        effective_entitlement, entitlement_entity,
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

    // Global metered feature.
    let feature = clients
        .entitlements
        .clone()
        .create_feature(CreateFeatureRequest {
            name: "pv-beats-addon".into(),
            code: "pv-beats-addon".into(),
            description: None,
            product_id: None,
            feature_type: Some(FeatureType {
                inner: Some(feature_type::Inner::Metered(feature_type::MeteredFeature {
                    metric_id: ids::METRIC_BANDWIDTH.as_proto(),
                })),
            }),
            entitlement: None,
        })
        .await
        .unwrap()
        .into_inner()
        .feature
        .unwrap();

    // Single-instance add-on grants 100.
    let add_on =
        make_single_instance_add_on(&clients, "pv-beats-addon-ao", "PVBeats-P", "EUR", Some(1))
            .await;
    clients
        .entitlements
        .clone()
        .create_entitlement(CreateEntitlementRequest {
            feature_id: feature.id.clone(),
            entity: Some(EntitlementEntity {
                entity_id: Some(entitlement_entity::EntityId::AddOnId(add_on.id.clone())),
            }),
            value: metered_value("100"),
        })
        .await
        .unwrap();

    // PlanVersion grants 200 — must override.
    clients
        .entitlements
        .clone()
        .create_entitlement(CreateEntitlementRequest {
            feature_id: feature.id.clone(),
            entity: entity_plan_version(ids::PLAN_VERSION_NOTION_ID.as_proto()),
            value: metered_value("200"),
        })
        .await
        .unwrap();

    // Subscription pulling that plan + the add-on.
    let now = chrono::Utc::now().date_naive();
    clients
        .subscriptions
        .clone()
        .create_subscription(tonic::Request::new(sub_api::CreateSubscriptionRequest {
            subscription: Some(sub_api::CreateSubscription {
                plan_version_id: (*ids::PLAN_VERSION_NOTION_ID).to_string(),
                start_date: now.as_proto(),
                billing_day_anchor: Some(1),
                customer_id: ids::CUST_SPOTIFY_ID.as_proto(),
                charge_automatically: Some(false),
                components: Some(sub_api::CreateSubscriptionComponents {
                    parameterized_components: vec![
                        sub_api::create_subscription_components::ComponentParameterization {
                            component_id: (*ids::COMP_NOTION_SEATS_ID).to_string(),
                            billing_period: Some(
                                meteroid_grpc::meteroid::api::shared::v1::BillingPeriod::Monthly
                                    .into(),
                            ),
                            initial_slot_count: Some(1),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }),
                add_ons: Some(sub_api::CreateSubscriptionAddOns {
                    add_ons: vec![sub_api::CreateSubscriptionAddOn {
                        add_on_id: add_on.id.clone(),
                        quantity: 1,
                        customization: None,
                    }],
                }),
                ..Default::default()
            }),
        }))
        .await
        .unwrap();

    let resolved = clients
        .entitlements
        .clone()
        .get_effective_entitlements(GetEffectiveEntitlementsRequest {
            customer_id: ids::CUST_SPOTIFY_ID.as_proto(),
        })
        .await
        .unwrap()
        .into_inner();

    let ent = resolved
        .entitlements
        .iter()
        .find(|e| e.feature.as_ref().is_some_and(|f| f.id == feature.id))
        .expect("feature must resolve");

    match &ent.value {
        Some(effective_entitlement::Value::Metered(m)) => {
            assert_eq!(
                m.limit.as_deref(),
                Some("200"),
                "PlanVersion must override AddOn"
            );
        }
        _ => panic!("expected metered value, got {:?}", ent.value),
    }
    assert!(
        matches!(
            ent.origin
                .as_ref()
                .and_then(|o| o.entity.as_ref())
                .and_then(|e| e.entity_id.as_ref()),
            Some(entitlement_entity::EntityId::PlanVersionId(_))
        ),
        "origin must be PlanVersion; got: {:?}",
        ent.origin
    );
}

/// Algorithm rule §2: two single-instance add-ons granting the same global feature at the
/// same priority — the more permissive (max) limit wins, not last-created.
#[tokio::test]
async fn test_same_priority_addons_take_permissive_max() {
    use meteroid_grpc::meteroid::api::entitlements::v1::{
        effective_entitlement, entitlement_entity,
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

    // Global metered feature.
    let feature = clients
        .entitlements
        .clone()
        .create_feature(CreateFeatureRequest {
            name: "permissive-same-pri".into(),
            code: "permissive-same-pri".into(),
            description: None,
            product_id: None,
            feature_type: Some(FeatureType {
                inner: Some(feature_type::Inner::Metered(feature_type::MeteredFeature {
                    metric_id: ids::METRIC_BANDWIDTH.as_proto(),
                })),
            }),
            entitlement: None,
        })
        .await
        .unwrap()
        .into_inner()
        .feature
        .unwrap();

    // Two single-instance add-ons: the LARGER limit (700) is created FIRST.
    // Under old last-wins logic that would lose to 300. Permissive merge keeps 700.
    let add_on_big =
        make_single_instance_add_on(&clients, "pri-big", "PriBig-P", "EUR", Some(1)).await;
    clients
        .entitlements
        .clone()
        .create_entitlement(CreateEntitlementRequest {
            feature_id: feature.id.clone(),
            entity: Some(EntitlementEntity {
                entity_id: Some(entitlement_entity::EntityId::AddOnId(add_on_big.id.clone())),
            }),
            value: metered_value("700"),
        })
        .await
        .unwrap();

    let add_on_small =
        make_single_instance_add_on(&clients, "pri-small", "PriSmall-P", "EUR", Some(1)).await;
    clients
        .entitlements
        .clone()
        .create_entitlement(CreateEntitlementRequest {
            feature_id: feature.id.clone(),
            entity: Some(EntitlementEntity {
                entity_id: Some(entitlement_entity::EntityId::AddOnId(
                    add_on_small.id.clone(),
                )),
            }),
            value: metered_value("300"),
        })
        .await
        .unwrap();

    let now = chrono::Utc::now().date_naive();
    clients
        .subscriptions
        .clone()
        .create_subscription(tonic::Request::new(sub_api::CreateSubscriptionRequest {
            subscription: Some(sub_api::CreateSubscription {
                plan_version_id: (*ids::PLAN_VERSION_NOTION_ID).to_string(),
                start_date: now.as_proto(),
                billing_day_anchor: Some(1),
                customer_id: ids::CUST_SPOTIFY_ID.as_proto(),
                charge_automatically: Some(false),
                components: Some(sub_api::CreateSubscriptionComponents {
                    parameterized_components: vec![
                        sub_api::create_subscription_components::ComponentParameterization {
                            component_id: (*ids::COMP_NOTION_SEATS_ID).to_string(),
                            billing_period: Some(
                                meteroid_grpc::meteroid::api::shared::v1::BillingPeriod::Monthly
                                    .into(),
                            ),
                            initial_slot_count: Some(1),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }),
                add_ons: Some(sub_api::CreateSubscriptionAddOns {
                    add_ons: vec![
                        sub_api::CreateSubscriptionAddOn {
                            add_on_id: add_on_big.id.clone(),
                            quantity: 1,
                            customization: None,
                        },
                        sub_api::CreateSubscriptionAddOn {
                            add_on_id: add_on_small.id.clone(),
                            quantity: 1,
                            customization: None,
                        },
                    ],
                }),
                ..Default::default()
            }),
        }))
        .await
        .unwrap();

    let resolved = clients
        .entitlements
        .clone()
        .get_effective_entitlements(GetEffectiveEntitlementsRequest {
            customer_id: ids::CUST_SPOTIFY_ID.as_proto(),
        })
        .await
        .unwrap()
        .into_inner();

    let ent = resolved
        .entitlements
        .iter()
        .find(|e| e.feature.as_ref().is_some_and(|f| f.id == feature.id))
        .expect("feature must resolve");

    match &ent.value {
        Some(effective_entitlement::Value::Metered(m)) => {
            assert_eq!(
                m.limit.as_deref(),
                Some("700"),
                "permissive merge must take max (700), not last-created (300)"
            );
        }
        _ => panic!("expected metered value, got {:?}", ent.value),
    }
}

/// PlanVersion target now includes entitlements from add-ons attached via `plan_version_add_on`
/// (regression test for the gap previously identified — code only included price-component products).
#[tokio::test]
async fn test_plan_version_target_excludes_linked_add_on_entitlements() {
    // Plan version inherits entitlements from its price-component products only, not from
    // linked add-ons. Add-on entitlements compose later, at the Subscription/Quote layer.
    use meteroid_grpc::meteroid::api::entitlements::v1::{
        GetResolvedForPlanVersionRequest, entitlement_entity,
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

    // Add-on owns its product; a feature is created under that product with an add-on-level grant.
    let add_on =
        make_single_instance_add_on(&clients, "pv-linked-ao", "PVLinked-P", "EUR", Some(1)).await;

    let feature = clients
        .entitlements
        .clone()
        .create_feature(CreateFeatureRequest {
            name: "pv-linked-addon-feat".into(),
            code: "pv-linked-addon-feat".into(),
            description: None,
            product_id: Some(add_on.product_id.clone()),
            feature_type: Some(FeatureType {
                inner: Some(feature_type::Inner::Metered(feature_type::MeteredFeature {
                    metric_id: ids::METRIC_BANDWIDTH.as_proto(),
                })),
            }),
            entitlement: None,
        })
        .await
        .unwrap()
        .into_inner()
        .feature
        .unwrap();

    clients
        .entitlements
        .clone()
        .create_entitlement(CreateEntitlementRequest {
            feature_id: feature.id.clone(),
            entity: Some(EntitlementEntity {
                entity_id: Some(entitlement_entity::EntityId::AddOnId(add_on.id.clone())),
            }),
            value: metered_value("5000"),
        })
        .await
        .unwrap();

    // Attach the add-on to the plan version (catalog-level linkage).
    clients
        .add_ons
        .clone()
        .attach_add_on_to_plan_version(
            meteroid_grpc::meteroid::api::addons::v1::AttachAddOnToPlanVersionRequest {
                plan_version_id: (*ids::PLAN_VERSION_NOTION_ID).to_string(),
                add_on_id: add_on.id.clone(),
                price_id: None,
                self_serviceable: None,
                max_instances_per_subscription: Some(1),
            },
        )
        .await
        .unwrap();

    // Resolve directly for the PlanVersion target — no subscription needed.
    let resolved = clients
        .entitlements
        .clone()
        .get_resolved_entitlements_for_plan_version(GetResolvedForPlanVersionRequest {
            plan_version_id: (*ids::PLAN_VERSION_NOTION_ID).to_string(),
        })
        .await
        .unwrap()
        .into_inner();

    assert!(
        resolved
            .entitlements
            .iter()
            .all(|e| e.feature.as_ref().is_none_or(|f| f.id != feature.id)),
        "feature from a linked add-on must NOT appear in PlanVersion resolution; \
         add-on entitlements compose at Subscription/Quote, not at PlanVersion. got: {:?}",
        resolved
            .entitlements
            .iter()
            .filter_map(|e| e.feature.as_ref().map(|f| f.name.clone()))
            .collect::<Vec<_>>()
    );
}

/// A customer-level entitlement resolves for a customer with **no subscription** (the primary
/// use case for customer-level grants): it sits just above the tenant-global feature default
/// and surfaces via the customer-wide effective lookup, with origin = the customer.
#[tokio::test]
async fn test_customer_entitlement_resolves_without_subscription() {
    use meteroid_grpc::meteroid::api::entitlements::v1::effective_entitlement;

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

    // Tenant-global boolean feature, no feature-level default.
    let feature = clients
        .entitlements
        .clone()
        .create_feature(CreateFeatureRequest {
            name: "customer-grant-feat".into(),
            code: "customer-grant-feat".into(),
            description: None,
            product_id: None,
            feature_type: Some(FeatureType {
                inner: Some(feature_type::Inner::Boolean(
                    feature_type::BooleanFeature {},
                )),
            }),
            entitlement: None,
        })
        .await
        .unwrap()
        .into_inner()
        .feature
        .unwrap();

    // Customer-level grant for CUST_SPOTIFY_ID, which has no subscription at SeedLevel::PLANS.
    clients
        .entitlements
        .clone()
        .create_entitlement(CreateEntitlementRequest {
            feature_id: feature.id.clone(),
            entity: entity_customer(ids::CUST_SPOTIFY_ID.as_proto()),
            value: boolean_value(true),
        })
        .await
        .unwrap();

    let resolved = clients
        .entitlements
        .clone()
        .get_effective_entitlements(GetEffectiveEntitlementsRequest {
            customer_id: ids::CUST_SPOTIFY_ID.as_proto(),
        })
        .await
        .unwrap()
        .into_inner();

    let ent = resolved
        .entitlements
        .iter()
        .find(|e| e.feature.as_ref().is_some_and(|f| f.id == feature.id))
        .expect("customer-level grant must resolve for a customer with no subscription");

    assert!(
        matches!(
            &ent.value,
            Some(effective_entitlement::Value::Boolean(b)) if b.enabled
        ),
        "customer-level grant (enabled=true) must surface as enabled; got: {:?}",
        ent.value
    );
    assert!(
        matches!(
            &ent.origin,
            Some(ResolvedOrigin { entity: Some(EntitlementEntity { entity_id: Some(entitlement_entity::EntityId::CustomerId(id)) }), .. })
            if id == &ids::CUST_SPOTIFY_ID.as_proto()
        ),
        "origin must be the customer entity; got: {:?}",
        ent.origin
    );
}

/// A feature enabled at the **customer** level but disabled on the customer's **subscription**.
/// The customer level is a top-priority force, so it wins over the subscription entry: both the
/// per-subscription resolution and the customer-wide view report enabled, with the customer as
/// the origin. (Cross-subscription OR for features with *no* customer force is covered by the
/// `aggregate_ors_enabled_across_scopes` unit test.)
#[tokio::test]
async fn test_customer_force_overrides_subscription() {
    use meteroid_grpc::meteroid::api::entitlements::v1::{
        GetResolvedForSubscriptionRequest, effective_entitlement, resolved_entitlement,
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

    let feature = clients
        .entitlements
        .clone()
        .create_feature(CreateFeatureRequest {
            name: "site-toggle-feat".into(),
            code: "site-toggle-feat".into(),
            description: None,
            product_id: None,
            feature_type: Some(FeatureType {
                inner: Some(feature_type::Inner::Boolean(
                    feature_type::BooleanFeature {},
                )),
            }),
            entitlement: None,
        })
        .await
        .unwrap()
        .into_inner()
        .feature
        .unwrap();

    // Customer-level grant: enabled=true across the account.
    clients
        .entitlements
        .clone()
        .create_entitlement(CreateEntitlementRequest {
            feature_id: feature.id.clone(),
            entity: entity_customer(ids::CUST_SPOTIFY_ID.as_proto()),
            value: boolean_value(true),
        })
        .await
        .unwrap();

    // A subscription for that customer.
    let now = chrono::Utc::now().date_naive();
    let sub = clients
        .subscriptions
        .clone()
        .create_subscription(tonic::Request::new(sub_api::CreateSubscriptionRequest {
            subscription: Some(sub_api::CreateSubscription {
                plan_version_id: (*ids::PLAN_VERSION_NOTION_ID).to_string(),
                start_date: now.as_proto(),
                billing_day_anchor: Some(1),
                customer_id: ids::CUST_SPOTIFY_ID.as_proto(),
                charge_automatically: Some(false),
                components: Some(sub_api::CreateSubscriptionComponents {
                    parameterized_components: vec![
                        sub_api::create_subscription_components::ComponentParameterization {
                            component_id: (*ids::COMP_NOTION_SEATS_ID).to_string(),
                            billing_period: Some(
                                meteroid_grpc::meteroid::api::shared::v1::BillingPeriod::Monthly
                                    .into(),
                            ),
                            initial_slot_count: Some(1),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }),
                ..Default::default()
            }),
        }))
        .await
        .unwrap()
        .into_inner()
        .subscription
        .unwrap();

    // Subscription-level entry that disagrees with the customer force: disabled for this site.
    clients
        .entitlements
        .clone()
        .create_entitlement(CreateEntitlementRequest {
            feature_id: feature.id.clone(),
            entity: entity_subscription_proto(sub.id.clone()),
            value: boolean_value(false),
        })
        .await
        .unwrap();

    // Per-subscription resolution: the customer force wins over the subscription → enabled,
    // with the customer as the origin.
    let per_sub = clients
        .entitlements
        .clone()
        .get_resolved_entitlements_for_subscription(GetResolvedForSubscriptionRequest {
            subscription_id: sub.id.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    let sub_ent = per_sub
        .entitlements
        .iter()
        .find(|e| e.feature.as_ref().is_some_and(|f| f.id == feature.id))
        .expect("feature must resolve for the subscription");
    assert!(
        matches!(
            &sub_ent.value,
            Some(resolved_entitlement::Value::Boolean(b)) if b.enabled
        ),
        "customer force must win over the subscription (enabled); got: {:?}",
        sub_ent.value
    );
    assert!(
        matches!(
            &sub_ent.origin,
            Some(ResolvedOrigin { entity: Some(EntitlementEntity { entity_id: Some(entitlement_entity::EntityId::CustomerId(id)) }), .. })
            if id == &ids::CUST_SPOTIFY_ID.as_proto()
        ),
        "origin must be the customer force; got: {:?}",
        sub_ent.origin
    );

    // Customer-wide effective view: the customer force applies account-wide → enabled.
    let customer_wide = clients
        .entitlements
        .clone()
        .get_effective_entitlements(GetEffectiveEntitlementsRequest {
            customer_id: ids::CUST_SPOTIFY_ID.as_proto(),
        })
        .await
        .unwrap()
        .into_inner();
    let cust_ent = customer_wide
        .entitlements
        .iter()
        .find(|e| e.feature.as_ref().is_some_and(|f| f.id == feature.id))
        .expect("feature must resolve customer-wide");
    assert!(
        matches!(
            &cust_ent.value,
            Some(effective_entitlement::Value::Boolean(b)) if b.enabled
        ),
        "customer-wide view must reflect the customer force (enabled); got: {:?}",
        cust_ent.value
    );
}

/// What each feature status promises, checked against a fresh feature holding one customer grant
/// made while it was active.
struct StatusContract {
    status: FeatureStatus,
    grant_resolves: bool,
    grant_listed_on_entity: bool,
    grant_readable_by_id: bool,
    listed_by_default: bool,
    listed_by_status_filter: bool,
    feature_readable_by_id: bool,
    accepts_writes: bool,
    code_reserved: bool,
    /// A subscription created now on a plan version that already grants the feature gets it.
    new_subscription_gets_plan_grant: bool,
    /// A grant inside a new subscription, quote, add-on or plan body is stored; otherwise it is
    /// skipped and the entity is still created.
    body_grant_kept: bool,
}

const STATUS_CONTRACTS: [StatusContract; 3] = [
    StatusContract {
        status: FeatureStatus::Active,
        grant_resolves: true,
        grant_listed_on_entity: true,
        grant_readable_by_id: true,
        listed_by_default: true,
        listed_by_status_filter: true,
        feature_readable_by_id: true,
        accepts_writes: true,
        code_reserved: true,
        new_subscription_gets_plan_grant: true,
        body_grant_kept: true,
    },
    StatusContract {
        status: FeatureStatus::Disabled,
        grant_resolves: false,
        grant_listed_on_entity: true,
        grant_readable_by_id: true,
        listed_by_default: true,
        listed_by_status_filter: true,
        feature_readable_by_id: true,
        accepts_writes: true,
        code_reserved: true,
        new_subscription_gets_plan_grant: false,
        body_grant_kept: true,
    },
    StatusContract {
        status: FeatureStatus::Archived,
        grant_resolves: false,
        grant_listed_on_entity: false,
        grant_readable_by_id: false,
        listed_by_default: false,
        listed_by_status_filter: true,
        feature_readable_by_id: true,
        accepts_writes: false,
        code_reserved: false,
        new_subscription_gets_plan_grant: false,
        body_grant_kept: false,
    },
];

async fn status_test_clients() -> (
    meteroid_it::container::MeteroidSetup,
    meteroid_it::clients::AllClients,
) {
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
    (setup, clients)
}

fn boolean_feature_request(code: &str) -> CreateFeatureRequest {
    CreateFeatureRequest {
        name: code.into(),
        code: code.into(),
        description: None,
        product_id: None,
        feature_type: Some(FeatureType {
            inner: Some(feature_type::Inner::Boolean(
                feature_type::BooleanFeature {},
            )),
        }),
        entitlement: None,
    }
}

async fn set_feature_status(
    clients: &meteroid_it::clients::AllClients,
    id: &str,
    status: FeatureStatus,
) -> Result<(), tonic::Status> {
    clients
        .entitlements
        .clone()
        .set_feature_status(SetFeatureStatusRequest {
            id: id.to_string(),
            status: status.into(),
        })
        .await
        .map(|_| ())
}

fn state_label(status: FeatureStatus) -> String {
    status.as_str_name().to_lowercase()
}

async fn listed_with_statuses(
    clients: &meteroid_it::clients::AllClients,
    statuses: Vec<i32>,
    id: &str,
) -> bool {
    clients
        .entitlements
        .clone()
        .list_features(ListFeaturesRequest {
            pagination: None,
            statuses,
            search: None,
            product_id: None,
        })
        .await
        .unwrap()
        .into_inner()
        .features
        .iter()
        .any(|f| f.id == id)
}

#[tokio::test]
async fn test_feature_status_contract() {
    let (_setup, clients) = status_test_clients().await;
    let customer = ids::CUST_SPOTIFY_ID.as_proto();

    for contract in STATUS_CONTRACTS {
        let label = state_label(contract.status);
        let code = format!("contract-{label}");
        let feature = clients
            .entitlements
            .clone()
            .create_feature(boolean_feature_request(&code))
            .await
            .unwrap()
            .into_inner()
            .feature
            .unwrap();
        let grant = clients
            .entitlements
            .clone()
            .create_entitlement(CreateEntitlementRequest {
                feature_id: feature.id.clone(),
                entity: entity_customer(customer.clone()),
                value: boolean_value(true),
            })
            .await
            .unwrap()
            .into_inner()
            .entitlement
            .unwrap();
        set_feature_status(&clients, &feature.id, contract.status)
            .await
            .unwrap();

        let resolves = clients
            .entitlements
            .clone()
            .get_effective_entitlements(GetEffectiveEntitlementsRequest {
                customer_id: customer.clone(),
            })
            .await
            .unwrap()
            .into_inner()
            .entitlements
            .iter()
            .any(|e| e.feature.as_ref().is_some_and(|f| f.id == feature.id));
        assert_eq!(resolves, contract.grant_resolves, "{label}: grant resolves");

        let listed_on_entity = clients
            .entitlements
            .clone()
            .list_entitlements_by_entity(ListEntitlementsByEntityRequest {
                entity: entity_customer(customer.clone()),
            })
            .await
            .unwrap()
            .into_inner()
            .entitlements
            .iter()
            .any(|e| e.feature_id == feature.id);
        assert_eq!(
            listed_on_entity, contract.grant_listed_on_entity,
            "{label}: grant listed on its entity"
        );

        assert_eq!(
            listed_with_statuses(&clients, vec![], &feature.id).await,
            contract.listed_by_default,
            "{label}: in the default feature listing"
        );
        assert_eq!(
            listed_with_statuses(&clients, vec![contract.status as i32], &feature.id).await,
            contract.listed_by_status_filter,
            "{label}: in a listing filtered by its own status"
        );

        let read = clients
            .entitlements
            .clone()
            .get_feature(GetFeatureRequest {
                id: feature.id.clone(),
            })
            .await;
        assert_eq!(
            read.is_ok(),
            contract.feature_readable_by_id,
            "{label}: readable by id"
        );
        let grant_by_id = clients
            .entitlements
            .clone()
            .get_entitlement(GetEntitlementRequest {
                id: grant.id.clone(),
            })
            .await;
        assert_eq!(
            grant_by_id.is_ok(),
            contract.grant_readable_by_id,
            "{label}: grant readable by id"
        );
        if !contract.grant_readable_by_id {
            assert_eq!(
                grant_by_id.unwrap_err().code(),
                tonic::Code::NotFound,
                "{label}: grant by id"
            );
        }

        let same_code = clients
            .entitlements
            .clone()
            .create_feature(boolean_feature_request(&code))
            .await;
        assert_eq!(
            same_code.is_err(),
            contract.code_reserved,
            "{label}: code reserved"
        );

        let writes: [(&str, Result<(), tonic::Status>); 3] = [
            (
                "new grant",
                clients
                    .entitlements
                    .clone()
                    .create_entitlement(CreateEntitlementRequest {
                        feature_id: feature.id.clone(),
                        entity: entity_plan_version(ids::PLAN_VERSION_NOTION_ID.as_proto()),
                        value: boolean_value(true),
                    })
                    .await
                    .map(|_| ()),
            ),
            (
                "edit existing grant",
                clients
                    .entitlements
                    .clone()
                    .update_entitlement(UpdateEntitlementRequest {
                        id: grant.id.clone(),
                        value: boolean_value(false),
                    })
                    .await
                    .map(|_| ()),
            ),
            (
                "edit feature",
                clients
                    .entitlements
                    .clone()
                    .update_feature(UpdateFeatureRequest {
                        id: feature.id.clone(),
                        name: Some(format!("{code}-renamed")),
                        description: None,
                        product_id: None,
                        clear_product_id: false,
                        config_options: vec![],
                        update_config_options: false,
                    })
                    .await
                    .map(|_| ()),
            ),
        ];
        for (write, result) in writes {
            match result {
                Ok(()) => assert!(contract.accepts_writes, "{label}: {write} must be rejected"),
                Err(status) => {
                    assert!(!contract.accepts_writes, "{label}: {write}: {status:?}");
                    // A hidden grant is not found, so editing one is a miss.
                    if !contract.grant_readable_by_id && write == "edit existing grant" {
                        assert_eq!(status.code(), tonic::Code::NotFound, "{label}: {write}");
                        continue;
                    }
                    assert_eq!(
                        status.code(),
                        tonic::Code::InvalidArgument,
                        "{label}: {write}: {status:?}"
                    );
                    assert!(
                        status.message().contains(&label),
                        "{label}: {write} rejected for its status: {status:?}"
                    );
                }
            }
        }
    }
}

/// Creating entities against a feature in each state: a plan version that already grants it
/// reaches a new subscription only while the grant resolves, and grants inside a new entity's
/// body are kept or skipped by status, never failing the create.
#[tokio::test]
async fn test_feature_status_contract_for_new_entities() {
    use meteroid_grpc::meteroid::api;
    use meteroid_grpc::meteroid::api::entitlements::v1::{
        EntitlementSpec, GetResolvedForSubscriptionRequest,
    };

    let (_setup, clients) = status_test_clients().await;

    let subscribe = |entitlements: Vec<EntitlementSpec>| {
        let mut client = clients.subscriptions.clone();
        async move {
            client
                .create_subscription(tonic::Request::new(sub_api::CreateSubscriptionRequest {
                    subscription: Some(sub_api::CreateSubscription {
                        plan_version_id: ids::PLAN_VERSION_NOTION_ID.as_proto(),
                        start_date: chrono::Utc::now().date_naive().as_proto(),
                        billing_day_anchor: Some(1),
                        customer_id: ids::CUST_SPOTIFY_ID.as_proto(),
                        charge_automatically: Some(false),
                        components: Some(sub_api::CreateSubscriptionComponents {
                            parameterized_components: vec![
                                sub_api::create_subscription_components::ComponentParameterization {
                                    component_id: ids::COMP_NOTION_SEATS_ID.as_proto(),
                                    billing_period: Some(
                                        api::shared::v1::BillingPeriod::Monthly.into(),
                                    ),
                                    initial_slot_count: Some(1),
                                    ..Default::default()
                                },
                            ],
                            ..Default::default()
                        }),
                        entitlements,
                        ..Default::default()
                    }),
                }))
                .await
                .map(|r| r.into_inner().subscription.unwrap().id)
        }
    };
    let grants_on = |entity: Option<EntitlementEntity>| {
        let mut client = clients.entitlements.clone();
        async move {
            client
                .list_entitlements_by_entity(ListEntitlementsByEntityRequest { entity })
                .await
                .unwrap()
                .into_inner()
                .entitlements
                .into_iter()
                .map(|e| e.feature_id)
                .collect::<Vec<_>>()
        }
    };

    for contract in STATUS_CONTRACTS {
        let label = state_label(contract.status);
        let feature = clients
            .entitlements
            .clone()
            .create_feature(boolean_feature_request(&format!("new-entities-{label}")))
            .await
            .unwrap()
            .into_inner()
            .feature
            .unwrap();
        clients
            .entitlements
            .clone()
            .create_entitlement(CreateEntitlementRequest {
                feature_id: feature.id.clone(),
                entity: entity_plan_version(ids::PLAN_VERSION_NOTION_ID.as_proto()),
                value: boolean_value(true),
            })
            .await
            .unwrap();
        set_feature_status(&clients, &feature.id, contract.status)
            .await
            .unwrap();

        let spec = || {
            vec![EntitlementSpec {
                feature_id: feature.id.clone(),
                value: boolean_value(true),
            }]
        };
        let check_body = |entity: &str, grants: Vec<String>| {
            assert_eq!(
                grants.contains(&feature.id),
                contract.body_grant_kept,
                "{label}: grant in a new {entity} body"
            );
        };

        let plain_sub = subscribe(vec![])
            .await
            .unwrap_or_else(|e| panic!("{label}: new subscription: {e:?}"));
        let gets_it = clients
            .entitlements
            .clone()
            .get_resolved_entitlements_for_subscription(GetResolvedForSubscriptionRequest {
                subscription_id: plain_sub,
            })
            .await
            .unwrap()
            .into_inner()
            .entitlements
            .iter()
            .any(|e| e.feature.as_ref().is_some_and(|f| f.id == feature.id));
        assert_eq!(
            gets_it, contract.new_subscription_gets_plan_grant,
            "{label}: new subscription gets the plan version's grant"
        );

        let sub = subscribe(spec())
            .await
            .unwrap_or_else(|e| panic!("{label}: subscription with a body grant: {e:?}"));
        check_body(
            "subscription",
            grants_on(entity_subscription_proto(sub)).await,
        );

        let quote = clients
            .quotes
            .clone()
            .create_quote(tonic::Request::new(crate::test_quote::quote_granting(
                &feature.id,
            )))
            .await
            .unwrap_or_else(|e| panic!("{label}: quote with a body grant: {e:?}"))
            .into_inner()
            .quote
            .unwrap()
            .quote
            .unwrap()
            .id;
        check_body(
            "quote",
            grants_on(Some(EntitlementEntity {
                entity_id: Some(entitlement_entity::EntityId::QuoteId(quote)),
            }))
            .await,
        );

        let add_on = clients
            .add_ons
            .clone()
            .create_add_on(api::addons::v1::CreateAddOnRequest {
                name: format!("add-on-{label}"),
                product: Some(api::components::v1::ProductRef {
                    r#ref: Some(api::components::v1::product_ref::Ref::NewProduct(
                        api::components::v1::NewProduct {
                            name: format!("add-on-product-{label}"),
                            fee_type: api::prices::v1::FeeType::Rate.into(),
                            fee_structure: Some(api::prices::v1::FeeStructure {
                                structure: Some(api::prices::v1::fee_structure::Structure::Rate(
                                    api::prices::v1::fee_structure::RateStructure {},
                                )),
                            }),
                        },
                    )),
                }),
                price: Some(api::components::v1::PriceEntry {
                    entry: Some(api::components::v1::price_entry::Entry::NewPrice(
                        api::components::v1::PriceInput {
                            credit_unit: None,
                            cadence: api::shared::v1::BillingPeriod::Monthly.into(),
                            currency: "EUR".into(),
                            pricing: Some(api::components::v1::price_input::Pricing::RatePricing(
                                api::prices::v1::RatePricing {
                                    rate: "9.99".into(),
                                },
                            )),
                        },
                    )),
                }),
                description: None,
                self_serviceable: true,
                max_instances_per_subscription: None,
                product_family_local_id: None,
                entitlements: spec(),
            })
            .await
            .unwrap_or_else(|e| panic!("{label}: add-on with a body grant: {e:?}"))
            .into_inner()
            .add_on
            .unwrap()
            .id;
        check_body(
            "add-on",
            grants_on(Some(EntitlementEntity {
                entity_id: Some(entitlement_entity::EntityId::AddOnId(add_on)),
            }))
            .await,
        );

        let plan_version = clients
            .plans
            .clone()
            .create_draft_plan(api::plans::v1::CreateDraftPlanRequest {
                name: format!("plan-{label}"),
                description: None,
                product_family_local_id: "default".into(),
                plan_type: api::plans::v1::PlanType::Standard.into(),
                currency: "EUR".into(),
                entitlements: spec(),
                tax_inclusive: false,
            })
            .await
            .unwrap_or_else(|e| panic!("{label}: plan with a body grant: {e:?}"))
            .into_inner()
            .plan
            .unwrap()
            .version
            .unwrap()
            .id;
        check_body("plan", grants_on(entity_plan_version(plan_version)).await);
    }
}

/// The status a change leads to and the activity it records, or `Err` when it is rejected. An
/// archived feature can only be restored to active.
fn apply_status_change(
    from: FeatureStatus,
    to: FeatureStatus,
) -> Result<(FeatureStatus, Option<&'static str>), ()> {
    use FeatureStatus::*;
    match (from, to) {
        _ if from == to => Ok((from, None)),
        (Archived, Disabled) => Err(()),
        (_, Archived) => Ok((Archived, Some("feature.archived"))),
        (Archived, Active) => Ok((Active, Some("feature.unarchived"))),
        (_, Active) => Ok((Active, Some("feature.enabled"))),
        (_, Disabled) => Ok((Disabled, Some("feature.disabled"))),
    }
}

/// Every status change from every status, allowed or not. Whatever the path, a grant made while
/// active resolves only while the feature is active, is hidden only while it is archived, keeps
/// its value throughout, and each change is recorded once.
#[tokio::test]
async fn test_feature_status_transitions() {
    use common_domain::ids::BaseId;
    use diesel_models::entity_activity::EntityActivityRow;

    let (setup, clients) = status_test_clients().await;
    let customer = ids::CUST_SPOTIFY_ID.as_proto();
    let statuses = STATUS_CONTRACTS.map(|c| c.status);

    let activities = |feature_id: &str| {
        let store = setup.store.clone();
        let uuid = common_domain::ids::FeatureId::from_proto(feature_id)
            .unwrap()
            .as_uuid();
        async move {
            let mut conn = store.get_conn().await.unwrap();
            EntityActivityRow::list_by_entity(&mut conn, ids::TENANT_ID, "feature", uuid, None, 50)
                .await
                .unwrap()
        }
    };

    for from in statuses {
        for to in statuses {
            let label = format!("{} -> {}", state_label(from), state_label(to));
            let code = format!("transition-{}-{}", state_label(from), state_label(to));
            let feature = clients
                .entitlements
                .clone()
                .create_feature(boolean_feature_request(&code))
                .await
                .unwrap()
                .into_inner()
                .feature
                .unwrap();
            let grant = clients
                .entitlements
                .clone()
                .create_entitlement(CreateEntitlementRequest {
                    feature_id: feature.id.clone(),
                    entity: entity_customer(customer.clone()),
                    value: boolean_value(true),
                })
                .await
                .unwrap()
                .into_inner()
                .entitlement
                .unwrap();
            set_feature_status(&clients, &feature.id, from)
                .await
                .unwrap();
            let before: Vec<_> = activities(&feature.id)
                .await
                .into_iter()
                .map(|a| a.id)
                .collect();

            let result = set_feature_status(&clients, &feature.id, to).await;
            let expected = apply_status_change(from, to);
            assert_eq!(result.is_ok(), expected.is_ok(), "{label}: {result:?}");
            let (status, activity) = expected.unwrap_or((from, None));

            let stored = clients
                .entitlements
                .clone()
                .get_feature(GetFeatureRequest {
                    id: feature.id.clone(),
                })
                .await
                .unwrap()
                .into_inner()
                .feature
                .unwrap();
            assert_eq!(stored.status, status as i32, "{label}: status");

            let resolves = clients
                .entitlements
                .clone()
                .get_effective_entitlements(GetEffectiveEntitlementsRequest {
                    customer_id: customer.clone(),
                })
                .await
                .unwrap()
                .into_inner()
                .entitlements
                .iter()
                .any(|e| e.feature.as_ref().is_some_and(|f| f.id == feature.id));
            assert_eq!(
                resolves,
                status == FeatureStatus::Active,
                "{label}: the grant resolves only while active"
            );

            let kept = clients
                .entitlements
                .clone()
                .list_entitlements_by_entity(ListEntitlementsByEntityRequest {
                    entity: entity_customer(customer.clone()),
                })
                .await
                .unwrap()
                .into_inner()
                .entitlements
                .into_iter()
                .find(|e| e.id == grant.id);
            if status == FeatureStatus::Archived {
                assert!(
                    kept.is_none(),
                    "{label}: an archived feature's grant is hidden"
                );
            } else {
                let kept = kept.unwrap_or_else(|| panic!("{label}: the grant is kept"));
                assert_eq!(
                    kept.value, grant.value,
                    "{label}: the grant keeps its value"
                );
            }

            let recorded: Vec<String> = activities(&feature.id)
                .await
                .into_iter()
                .filter(|a| !before.contains(&a.id))
                .map(|a| a.activity_type)
                .collect();
            let expected: Vec<String> = activity.map(String::from).into_iter().collect();
            assert_eq!(recorded, expected, "{label}: recorded activity");
        }
    }
}
