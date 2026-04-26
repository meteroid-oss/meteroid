use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use meteroid::api::shared::conversions::ProtoConv;
use meteroid_grpc::meteroid::api::entitlements::v1::{
    CreateEntitlementRequest, CreateFeatureRequest, DeleteEntitlementRequest, EntitlementEntity,
    EntitlementValue, FeatureEntitlementSpec, FeatureStatus, FeatureType,
    GetEffectiveEntitlementsRequest, GetEntitlementRequest, GetFeatureRequest,
    ListEntitlementsByFeatureRequest, ListFeaturesRequest, ResolvedOrigin, SetFeatureStatusRequest,
    UpdateEntitlementRequest, UpdateFeatureRequest, entitlement_entity, entitlement_value,
    feature_type,
};
use meteroid_grpc::meteroid::api::subscriptions::v1 as sub_api;

use crate::data::ids;

#[tokio::test]
async fn test_features_crud() {
    helpers::init::logging();
    let postgres_connection_string = meteroid_it::container::create_test_database().await;
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

    // disable (kill-switch — feature stays visible but is filtered out by Active filter)
    clients
        .entitlements
        .clone()
        .set_feature_status(SetFeatureStatusRequest {
            id: created.id.clone(),
            status: FeatureStatus::Disabled.into(),
        })
        .await
        .unwrap();

    let listed_active_after_disable = clients
        .entitlements
        .clone()
        .list_features(ListFeaturesRequest {
            pagination: None,
            statuses: vec![FeatureStatus::Active as i32],
            search: None,
            product_id: None,
        })
        .await
        .unwrap()
        .into_inner();
    assert!(listed_active_after_disable.features.is_empty());

    let listed_disabled = clients
        .entitlements
        .clone()
        .list_features(ListFeaturesRequest {
            pagination: None,
            statuses: vec![FeatureStatus::Disabled as i32],
            search: None,
            product_id: None,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(listed_disabled.features.len(), 1);
    assert_eq!(
        listed_disabled.features[0].status,
        FeatureStatus::Disabled as i32
    );

    // re-activate — Disabled → Active round-trip
    clients
        .entitlements
        .clone()
        .set_feature_status(SetFeatureStatusRequest {
            id: created.id.clone(),
            status: FeatureStatus::Active.into(),
        })
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

    // archive
    clients
        .entitlements
        .clone()
        .set_feature_status(SetFeatureStatusRequest {
            id: created.id.clone(),
            status: FeatureStatus::Archived.into(),
        })
        .await
        .unwrap();

    // archived features excluded from Active filter
    let listed_after_archive = clients
        .entitlements
        .clone()
        .list_features(ListFeaturesRequest {
            pagination: None,
            statuses: vec![FeatureStatus::Active as i32],
            search: None,
            product_id: None,
        })
        .await
        .unwrap()
        .into_inner();

    assert!(listed_after_archive.features.is_empty());

    // archived features visible when requested
    let listed_archived = clients
        .entitlements
        .clone()
        .list_features(ListFeaturesRequest {
            pagination: None,
            statuses: vec![FeatureStatus::Archived as i32],
            search: None,
            product_id: None,
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(listed_archived.features.len(), 1);
    assert_eq!(
        listed_archived.features[0].status,
        FeatureStatus::Archived as i32
    );
}

#[tokio::test]
async fn test_entitlements_crud() {
    helpers::init::logging();
    let postgres_connection_string = meteroid_it::container::create_test_database().await;
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
                        overage_behavior: None,
                        warning_threshold_pct: None,
                        enabled: true,
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
                        overage_behavior: None,
                        warning_threshold_pct: None,
                        enabled: true,
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
    let postgres_connection_string = meteroid_it::container::create_test_database().await;
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
    let now = chrono::offset::Local::now().date_naive();
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
                        overage_behavior: None,
                        warning_threshold_pct: None,
                        enabled: true,
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
                        overage_behavior: None,
                        warning_threshold_pct: None,
                        enabled: true,
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
    let postgres_connection_string = meteroid_it::container::create_test_database().await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::PLANS).await;

    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "TESTORG",
        "testslug",
    );

    let plan_version_id = ids::PLAN_VERSION_NOTION_ID.as_proto();

    let response = clients
        .entitlements
        .clone()
        .create_feature(CreateFeatureRequest {
            name: "bulk-export".into(),
            description: Some("Bulk export access".into()),
            product_id: None,
            feature_type: Some(FeatureType {
                inner: Some(feature_type::Inner::Boolean(
                    feature_type::BooleanFeature {},
                )),
            }),
            entitlement: Some(FeatureEntitlementSpec {
                entity: Some(EntitlementEntity {
                    entity_id: Some(entitlement_entity::EntityId::PlanVersionId(
                        plan_version_id.clone(),
                    )),
                }),
                value: Some(EntitlementValue {
                    value: Some(entitlement_value::Value::BooleanValue(
                        entitlement_value::BooleanValue { enabled: true },
                    )),
                }),
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
            entity_id: Some(entitlement_entity::EntityId::PlanVersionId(plan_version_id))
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

// ── Helpers for the integration tests below ────────────────────────────────────

/// Build a metered entitlement value with just a hard limit; all other knobs unset.
fn metered_value(limit: &str) -> Option<EntitlementValue> {
    Some(EntitlementValue {
        value: Some(entitlement_value::Value::MeteredValue(
            entitlement_value::MeteredValue {
                limit: Some(limit.into()),
                reset_period: None,
                overage_behavior: None,
                warning_threshold_pct: None,
                enabled: true,
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

#[tokio::test]
async fn test_kill_switch_suppresses_feature_resolution() {
    helpers::init::logging();
    let postgres_connection_string = meteroid_it::container::create_test_database().await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::PLANS).await;

    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "TESTORG",
        "testslug",
    );

    let now = chrono::offset::Local::now().date_naive();
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
async fn test_archived_feature_excluded_from_resolution() {
    helpers::init::logging();
    let postgres_connection_string = meteroid_it::container::create_test_database().await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::PLANS).await;

    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "TESTORG",
        "testslug",
    );

    let now = chrono::offset::Local::now().date_naive();
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
            name: "dormant-feat".into(),
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
            entity: entity_plan_version(ids::PLAN_VERSION_NOTION_ID.as_proto()),
            value: boolean_value(true),
        })
        .await
        .unwrap();

    // Sanity: visible before archive.
    let before = clients
        .entitlements
        .clone()
        .get_effective_entitlements(GetEffectiveEntitlementsRequest {
            customer_id: ids::CUST_SPOTIFY_ID.as_proto(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(
        before
            .entitlements
            .iter()
            .any(|e| e.feature.as_ref().is_some_and(|f| f.id == feature.id))
    );

    clients
        .entitlements
        .clone()
        .set_feature_status(SetFeatureStatusRequest {
            id: feature.id.clone(),
            status: FeatureStatus::Archived.into(),
        })
        .await
        .unwrap();

    let after = clients
        .entitlements
        .clone()
        .get_effective_entitlements(GetEffectiveEntitlementsRequest {
            customer_id: ids::CUST_SPOTIFY_ID.as_proto(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(
        !after
            .entitlements
            .iter()
            .any(|e| e.feature.as_ref().is_some_and(|f| f.id == feature.id)),
        "archived feature must drop out of resolution"
    );

    // Restore brings it back.
    clients
        .entitlements
        .clone()
        .set_feature_status(SetFeatureStatusRequest {
            id: feature.id.clone(),
            status: FeatureStatus::Active.into(),
        })
        .await
        .unwrap();

    let restored = clients
        .entitlements
        .clone()
        .get_effective_entitlements(GetEffectiveEntitlementsRequest {
            customer_id: ids::CUST_SPOTIFY_ID.as_proto(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(
        restored
            .entitlements
            .iter()
            .any(|e| e.feature.as_ref().is_some_and(|f| f.id == feature.id))
    );
}

#[tokio::test]
async fn test_subscription_override_beats_plan_grant() {
    helpers::init::logging();
    let postgres_connection_string = meteroid_it::container::create_test_database().await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::PLANS).await;

    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "TESTORG",
        "testslug",
    );

    let now = chrono::offset::Local::now().date_naive();
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
    let postgres_connection_string = meteroid_it::container::create_test_database().await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::PLANS).await;

    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "TESTORG",
        "testslug",
    );

    let now = chrono::offset::Local::now().date_naive();
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
    let postgres_connection_string = meteroid_it::container::create_test_database().await;
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
    let postgres_connection_string = meteroid_it::container::create_test_database().await;
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
    let postgres_connection_string = meteroid_it::container::create_test_database().await;
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

    let now = chrono::offset::Local::now().date_naive();
    use meteroid::api::shared::conversions::ProtoConv;

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
    let postgres_connection_string = meteroid_it::container::create_test_database().await;
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
    let now = chrono::offset::Local::now().date_naive();
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
                        overage_behavior: None,
                        warning_threshold_pct: None,
                        enabled: false,
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
                        overage_behavior: None,
                        warning_threshold_pct: None,
                        enabled: true,
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

/// Verify that `GetResolvedEntitlementsForSubscription` reflects the full priority chain:
/// feature-level baseline → plan-level grant → subscription-level override.
#[tokio::test]
async fn test_resolved_entitlements_for_subscription_includes_chain() {
    use meteroid_grpc::meteroid::api::entitlements::v1::{
        GetResolvedForSubscriptionRequest, entitlement_entity, entitlement_value,
        resolved_entitlement,
    };

    helpers::init::logging();
    let postgres_connection_string = meteroid_it::container::create_test_database().await;
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
    let now = chrono::offset::Local::now().date_naive();
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
    let postgres_connection_string = meteroid_it::container::create_test_database().await;
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
    let postgres_connection_string = meteroid_it::container::create_test_database().await;
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
    let now = chrono::offset::Local::now().date_naive();
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
    let postgres_connection_string = meteroid_it::container::create_test_database().await;
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
    let now = chrono::offset::Local::now().date_naive();
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
    let postgres_connection_string = meteroid_it::container::create_test_database().await;
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
    let now = chrono::offset::Local::now().date_naive();
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
    let postgres_connection_string = meteroid_it::container::create_test_database().await;
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

    let now = chrono::offset::Local::now().date_naive();
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
    let postgres_connection_string = meteroid_it::container::create_test_database().await;
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
