//! ADR 0011 §3.4 (resources) and §3.11 (client token claims), against the store and REST.
use crate::data::ids::{CUST_UBER_ID, TENANT_ID};
use crate::meteroid_it::container::SeedLevel;
use crate::{helpers, meteroid_it};
use common_domain::ids::{AliasOr, BaseId, CustomerId, ResourceTypeId};
use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use diesel_models::enums::SubscriptionStatusEnum;
use meteroid_grpc::meteroid::api;
use meteroid_store::Store;
use meteroid_store::domain::PaginationRequest;
use meteroid_store::domain::resources::{ResourceInput, ResourceRef, ResourceType};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::portal_tokens::PortalTokensInterface;
use meteroid_store::repositories::resources::{
    ResourceRefusal, ResourceTypeNew, ResourceTypePatch, ResourceUpsert, load_for_check,
};
use meteroid_store::repositories::{ResourcesInterface, TenantInterface};
use reqwest::StatusCode;
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn pairs(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn input(resources: &[(&str, &str)], attributes: &[(&str, &str)]) -> ResourceInput {
    ResourceInput {
        resources: pairs(resources),
        attributes: pairs(attributes),
    }
}

fn type_new(code: &str, keys: &[&str], attributes: &[(&str, &[&str])]) -> ResourceTypeNew {
    ResourceTypeNew {
        tenant_id: TENANT_ID,
        code: code.into(),
        name: code.into(),
        plural_name: format!("{code}s"),
        parent_type_id: None,
        keys: keys.iter().map(|k| k.to_string()).collect(),
        attributes: attributes
            .iter()
            .map(|(a, values)| {
                (
                    a.to_string(),
                    values.iter().map(|v| v.to_string()).collect(),
                )
            })
            .collect(),
    }
}

fn upsert(
    customer_id: CustomerId,
    code: &str,
    key_value: &str,
    attributes: &[(&str, &str)],
) -> ResourceUpsert {
    ResourceUpsert {
        tenant_id: TENANT_ID,
        customer_id,
        resource_type: AliasOr::Alias(code.into()),
        key_value: key_value.into(),
        name: None,
        attributes: pairs(attributes),
        archived: false,
    }
}

fn refusal<T: std::fmt::Debug>(result: meteroid_store::StoreResult<T>) -> ResourceRefusal {
    let error = result.expect_err("refused");
    ResourceRefusal::of(&error).unwrap_or_else(|| panic!("no refusal code on {error:?}"))
}

fn precondition<T: std::fmt::Debug>(result: meteroid_store::StoreResult<T>) -> String {
    match result.expect_err("refused").current_context() {
        StoreError::FailedPrecondition(message) => message.clone(),
        other => panic!("expected a failed precondition, got {other:?}"),
    }
}

async fn check(
    store: &Store,
    customer_id: CustomerId,
    given: ResourceInput,
) -> meteroid_store::StoreResult<Option<meteroid_store::domain::resources::ResolvedResource>> {
    let mut conn = store.get_conn().await.unwrap();
    load_for_check(&mut conn, TENANT_ID, customer_id, &given).await
}

async fn set_cap(store: &Store, max: i32) {
    use diesel_models::schema::tenant::dsl as t;
    diesel::update(t::tenant.filter(t::id.eq(TENANT_ID)))
        .set(t::max_resources_per_customer_type.eq(max))
        .execute(&mut store.get_conn().await.unwrap())
        .await
        .unwrap();
}

async fn marked(store: &Store) -> bool {
    use diesel_models::schema::credit_recompute_mark::dsl as m;
    diesel::select(diesel::dsl::exists(
        m::credit_recompute_mark
            .filter(m::tenant_id.eq(TENANT_ID))
            .filter(m::customer_id.eq(CUST_UBER_ID)),
    ))
    .get_result(&mut store.get_conn().await.unwrap())
    .await
    .unwrap()
}

async fn unmark(store: &Store) {
    use diesel_models::schema::credit_recompute_mark::dsl as m;
    diesel::delete(m::credit_recompute_mark)
        .execute(&mut store.get_conn().await.unwrap())
        .await
        .unwrap();
}

/// New or changed attributes and a delete mark the customer for its monitors; a resource an
/// override ever named is archived, never deleted, so the override's history stays.
#[tokio::test]
async fn test_a_resource_marks_its_customer_and_keeps_its_overrides() {
    use common_domain::actor::Actor;
    use common_domain::ids::EntitlementEntityId;
    use meteroid_store::domain::entitlement_fold::{GrantValue, LimitValue};
    use meteroid_store::domain::entitlements::{
        EntitlementNew, EntitlementSpec, FeatureKind, FeatureNew, LimitCountedBy,
    };
    use meteroid_store::repositories::EntitlementsInterface;
    helpers::init::logging();
    let db = meteroid_it::container::create_test_database().await;
    let setup = meteroid_it::container::start_meteroid(db, SeedLevel::SUBSCRIPTIONS).await;
    let store = &setup.store;
    let site = store
        .create_resource_type(type_new(
            "site",
            &["site_id"],
            &[("environment", &["production", "staging"][..])],
        ))
        .await
        .unwrap();

    unmark(store).await;
    let s1 = store
        .upsert_resource(upsert(
            CUST_UBER_ID,
            "site",
            "s1",
            &[("environment", "staging")],
        ))
        .await
        .unwrap();
    assert!(marked(store).await, "a new resource");
    unmark(store).await;
    store
        .upsert_resource(upsert(
            CUST_UBER_ID,
            "site",
            "s1",
            &[("environment", "staging")],
        ))
        .await
        .unwrap();
    assert!(!marked(store).await, "nothing changed");
    store
        .upsert_resource(upsert(
            CUST_UBER_ID,
            "site",
            "s1",
            &[("environment", "production")],
        ))
        .await
        .unwrap();
    assert!(marked(store).await, "its attributes changed");

    let pages = store
        .create_feature(
            Actor::System,
            FeatureNew {
                tenant_id: TENANT_ID,
                product_id: None,
                name: "pages".into(),
                code: "pages".into(),
                description: None,
                kind: FeatureKind::Limit(LimitCountedBy::App),
                unit: None,
                level_type_id: Some(site.id),
                policy: None,
                default: None,
            },
        )
        .await
        .unwrap();
    let row = store
        .create_entitlement(
            &Actor::System,
            EntitlementNew {
                tenant_id: TENANT_ID,
                entity: EntitlementEntityId::Customer(CUST_UBER_ID),
                resource_id: Some(s1.id),
                spec: EntitlementSpec::new(
                    pages.id,
                    GrantValue::Limit(LimitValue::Amount(rust_decimal::Decimal::from(3))),
                ),
            },
        )
        .await
        .unwrap();
    store
        .delete_entitlement(&Actor::System, row.id, TENANT_ID)
        .await
        .unwrap();
    let refused = precondition(store.delete_resource(TENANT_ID, s1.id).await);
    assert!(refused.contains("archive it instead"), "{refused}");
    use diesel_async::SimpleAsyncConnection;
    let cascaded = store
        .get_conn()
        .await
        .unwrap()
        .batch_execute(&format!(
            "DELETE FROM resource WHERE id = '{}'",
            s1.id.as_uuid()
        ))
        .await;
    assert!(
        cascaded.is_err(),
        "an override created past the check still keeps its resource"
    );

    let s2 = store
        .upsert_resource(upsert(CUST_UBER_ID, "site", "s2", &[]))
        .await
        .unwrap();
    unmark(store).await;
    store.delete_resource(TENANT_ID, s2.id).await.unwrap();
    assert!(marked(store).await, "a deleted resource");
}

#[tokio::test]
async fn test_resources_store_rules() {
    helpers::init::logging();
    let postgres_connection_string = meteroid_it::container::create_test_database().await;
    let setup = meteroid_it::container::start_meteroid(
        postgres_connection_string,
        SeedLevel::SUBSCRIPTIONS,
    )
    .await;
    let store = &setup.store;

    let site: ResourceType = store
        .create_resource_type(type_new(
            "site",
            &["site_id", "website_id"],
            &[("environment", &["production", "staging"][..])],
        ))
        .await
        .unwrap();
    assert_eq!(site.keys, ["site_id", "website_id"]);

    // Keys: an alias resolves to the canonical path; clashes give RESOURCE_KEY_CLASH on create,
    // on update, and against archived types.
    let by_key = check(store, CUST_UBER_ID, input(&[("site_id", "s1")], &[]))
        .await
        .unwrap();
    let by_alias = check(store, CUST_UBER_ID, input(&[("website_id", "s1")], &[]))
        .await
        .unwrap();
    assert_eq!(by_key, by_alias);
    assert_eq!(by_key.unwrap().path, "site=s1");

    assert_eq!(
        refusal(
            store
                .create_resource_type(type_new("workspace", &["website_id"], &[]))
                .await
        ),
        ResourceRefusal::KeyClash
    );
    let workspace = store
        .create_resource_type(type_new("workspace", &["workspace_id"], &[]))
        .await
        .unwrap();
    let add_alias = ResourceTypePatch {
        keys: Some(vec!["workspace_id".into(), "site_id".into()]),
        ..Default::default()
    };
    assert_eq!(
        refusal(
            store
                .update_resource_type(TENANT_ID, workspace.id.into(), add_alias)
                .await
        ),
        ResourceRefusal::KeyClash
    );
    store
        .update_resource_type(
            TENANT_ID,
            workspace.id.into(),
            ResourceTypePatch {
                archived: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        refusal(
            store
                .create_resource_type(type_new("team", &["workspace_id"], &[]))
                .await
        ),
        ResourceRefusal::KeyClash
    );
    let duplicate_code = store
        .create_resource_type(type_new("site", &["other_id"], &[]))
        .await;
    assert!(matches!(
        duplicate_code.unwrap_err().current_context(),
        StoreError::DuplicateValue { .. }
    ));

    // One level: any parent type is refused, on create and on update.
    let child = ResourceTypeNew {
        parent_type_id: Some(site.id),
        ..type_new("page", &["page_id"], &[])
    };
    assert_eq!(
        refusal(store.create_resource_type(child).await),
        ResourceRefusal::HierarchyUnsupported
    );
    let reparent = ResourceTypePatch {
        parent_type_id: Some(ResourceTypeId::new()),
        ..Default::default()
    };
    assert_eq!(
        refusal(
            store
                .update_resource_type(TENANT_ID, site.id.into(), reparent)
                .await
        ),
        ResourceRefusal::HierarchyUnsupported
    );
    let two = check(
        store,
        CUST_UBER_ID,
        input(&[("site_id", "s1"), ("workspace_id", "w1")], &[]),
    )
    .await;
    assert!(matches!(
        two.unwrap_err().current_context(),
        StoreError::InvalidArgument(_)
    ));

    // Attributes are closed; a used value can't be removed, and the refusal lists its usages.
    let undeclared = store
        .upsert_resource(upsert(CUST_UBER_ID, "site", "s1", &[("environment", "qa")]))
        .await;
    assert!(matches!(
        undeclared.unwrap_err().current_context(),
        StoreError::InvalidArgument(_)
    ));
    let ad_hoc_undeclared = check(
        store,
        CUST_UBER_ID,
        input(&[("site_id", "s9")], &[("site.environment", "qa")]),
    )
    .await;
    assert!(ad_hoc_undeclared.is_err());

    let s1 = store
        .upsert_resource(upsert(
            CUST_UBER_ID,
            "site",
            "s1",
            &[("environment", "staging")],
        ))
        .await
        .unwrap();
    let production_only = || ResourceTypePatch {
        attributes: Some(BTreeMap::from([(
            "environment".to_owned(),
            vec!["production".to_owned()],
        )])),
        ..Default::default()
    };
    let used = precondition(
        store
            .update_resource_type(TENANT_ID, site.id.into(), production_only())
            .await,
    );
    assert!(
        used.contains("environment=staging") && used.contains("`s1`"),
        "{used}"
    );

    store
        .upsert_resource(upsert(
            CUST_UBER_ID,
            "site",
            "s1",
            &[("environment", "production")],
        ))
        .await
        .unwrap();
    let entitlement_id = uuid::Uuid::now_v7();
    {
        let mut conn = store.get_conn().await.unwrap();
        let feature_id = uuid::Uuid::now_v7();
        diesel::sql_query("INSERT INTO feature (id, tenant_id, name, code, kind) VALUES ($1, $2, 'Projects', 'projects', 'BOOLEAN')")
            .bind::<diesel::sql_types::Uuid, _>(feature_id)
            .bind::<diesel::sql_types::Uuid, _>(TENANT_ID.as_uuid())
            .execute(&mut conn)
            .await
            .unwrap();
        diesel::sql_query(
            "INSERT INTO entitlement (id, tenant_id, feature_id, entity_id, entity_type, mode, value,
                                      condition, condition_hash, lineage_id, lineage_created_at,
                                      lineage_anchor, effect)
             VALUES ($1, $2, $3, $4, 'CUSTOMER', 'OVERRIDE', '{\"type\":\"boolean\",\"enabled\":true}',
                     '{\"site.environment\":[\"staging\"]}', 'staging', $1, now(), now(), 'SET')",
        )
        .bind::<diesel::sql_types::Uuid, _>(entitlement_id)
        .bind::<diesel::sql_types::Uuid, _>(TENANT_ID.as_uuid())
        .bind::<diesel::sql_types::Uuid, _>(feature_id)
        .bind::<diesel::sql_types::Uuid, _>(CUST_UBER_ID.as_uuid())
        .execute(&mut conn)
        .await
        .unwrap();
    }
    let used = precondition(
        store
            .update_resource_type(TENANT_ID, site.id.into(), production_only())
            .await,
    );
    assert!(
        used.contains("by entitlement") && !used.contains("`s1`"),
        "{used}"
    );
    diesel::sql_query("UPDATE entitlement SET revoked_at = now() WHERE id = $1")
        .bind::<diesel::sql_types::Uuid, _>(entitlement_id)
        .execute(&mut store.get_conn().await.unwrap())
        .await
        .unwrap();
    let narrowed = store
        .update_resource_type(TENANT_ID, site.id.into(), production_only())
        .await
        .unwrap();
    assert_eq!(narrowed.attributes["environment"], ["production"]);
    let widened = ResourceTypePatch {
        attributes: Some(BTreeMap::from([(
            "environment".to_owned(),
            vec!["production".to_owned(), "staging".to_owned()],
        )])),
        ..Default::default()
    };
    store
        .update_resource_type(TENANT_ID, site.id.into(), widened)
        .await
        .unwrap();

    // Attribute precedence: stored beats asserted, and the override is reported.
    let resolved = check(
        store,
        CUST_UBER_ID,
        input(&[("site_id", "s1")], &[("site.environment", "staging")]),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(resolved.reference, ResourceRef::Stored(s1.id));
    assert_eq!(resolved.attributes["site.environment"], "production");
    assert_eq!(resolved.attributes_ignored, ["site.environment"]);

    // Ad-hoc: type defaults and conditions apply on its own path; nothing is stored.
    let ad_hoc = check(
        store,
        CUST_UBER_ID,
        input(&[("website_id", "s9")], &[("site.environment", "staging")]),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(matches!(ad_hoc.reference, ResourceRef::AdHoc { .. }));
    assert_eq!(ad_hoc.path, "site=s9");
    assert!(
        store
            .find_resource(TENANT_ID, CUST_UBER_ID, AliasOr::Alias("site".into()), "s9")
            .await
            .is_err(),
        "a check never creates a resource"
    );

    // Cap: counted per customer and type, configurable per tenant, never exceeded concurrently.
    let tenant_default: i32 = {
        use diesel_models::schema::tenant::dsl as t;
        t::tenant
            .filter(t::id.eq(TENANT_ID))
            .select(t::max_resources_per_customer_type)
            .first(&mut store.get_conn().await.unwrap())
            .await
            .unwrap()
    };
    assert_eq!(tenant_default, 10_000);
    set_cap(store, 2).await;
    store
        .upsert_resource(upsert(CUST_UBER_ID, "site", "s2", &[]))
        .await
        .unwrap();
    assert_eq!(
        refusal(
            store
                .upsert_resource(upsert(CUST_UBER_ID, "site", "s3", &[]))
                .await
        ),
        ResourceRefusal::LimitReached
    );
    store
        .upsert_resource(upsert(
            CUST_UBER_ID,
            "site",
            "s2",
            &[("environment", "staging")],
        ))
        .await
        .expect("an existing resource still upserts at the cap");
    set_cap(store, 3).await;
    store
        .upsert_resource(upsert(CUST_UBER_ID, "site", "s3", &[]))
        .await
        .unwrap();

    store
        .create_resource_type(type_new("user", &["user_id"], &[]))
        .await
        .unwrap();
    set_cap(store, 5).await;
    let results = futures::future::join_all((0..8).map(|i| {
        let key_value = format!("u{i}");
        async move {
            store
                .upsert_resource(upsert(CUST_UBER_ID, "user", &key_value, &[]))
                .await
        }
    }))
    .await;
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 5);
    let users = store
        .list_resources(
            TENANT_ID,
            Some(CUST_UBER_ID),
            Some(AliasOr::Alias("user".into())),
            true,
            PaginationRequest {
                page: 0,
                per_page: Some(100),
            },
        )
        .await
        .unwrap();
    assert_eq!(users.total_results, 5);
    set_cap(store, 10_000).await;

    // Archived type: checks resolve and existing resources upsert; new ones are refused.
    store
        .update_resource_type(
            TENANT_ID,
            site.id.into(),
            ResourceTypePatch {
                archived: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let still = check(store, CUST_UBER_ID, input(&[("site_id", "s1")], &[]))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(still.reference, ResourceRef::Stored(s1.id));
    assert_eq!(
        refusal(
            store
                .upsert_resource(upsert(CUST_UBER_ID, "site", "s4", &[]))
                .await
        ),
        ResourceRefusal::TypeArchived
    );
    let renamed = store
        .upsert_resource(ResourceUpsert {
            name: Some("Main site".into()),
            ..upsert(CUST_UBER_ID, "site", "s1", &[("environment", "production")])
        })
        .await
        .unwrap();
    assert_eq!(renamed.name.as_deref(), Some("Main site"));
    let listed = store
        .list_resources(
            TENANT_ID,
            Some(CUST_UBER_ID),
            Some(site.id.into()),
            false,
            PaginationRequest {
                page: 0,
                per_page: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(listed.total_results, 3);

    // Delete: refused while a live subscription is for the resource, allowed once it ended.
    let (subscription_id, customer): (uuid::Uuid, uuid::Uuid) = {
        use diesel_models::schema::subscription::dsl as s;
        s::subscription
            .filter(s::tenant_id.eq(TENANT_ID))
            .select((s::id, s::customer_id))
            .first(&mut store.get_conn().await.unwrap())
            .await
            .unwrap()
    };
    let customer = CustomerId::from(customer);
    let user = store
        .upsert_resource(upsert(customer, "user", "subscribed", &[]))
        .await
        .unwrap()
        .id;
    let set_subscription = |status: SubscriptionStatusEnum| async move {
        use diesel_models::schema::subscription::dsl as s;
        diesel::update(s::subscription.filter(s::id.eq(subscription_id)))
            .set((
                s::resource_id.eq(Some(user.as_uuid())),
                s::status.eq(status),
            ))
            .execute(&mut store.get_conn().await.unwrap())
            .await
            .unwrap();
    };
    // An ended subscription keeps its scope on its drafts, so only an aborted one lets go.
    for status in [
        SubscriptionStatusEnum::Active,
        SubscriptionStatusEnum::Cancelled,
        SubscriptionStatusEnum::Completed,
    ] {
        set_subscription(status).await;
        let refused = precondition(store.delete_resource(TENANT_ID, user).await);
        assert!(refused.contains("archive it instead"), "{refused}");
    }
    let mut conn = store.get_conn().await.unwrap();
    let raw_delete = diesel::sql_query("DELETE FROM resource WHERE id = $1")
        .bind::<diesel::sql_types::Uuid, _>(user.as_uuid())
        .execute(&mut conn)
        .await;
    assert!(raw_delete.is_err(), "the foreign key restricts the delete");
    store
        .set_resource_archived(TENANT_ID, user, true)
        .await
        .unwrap();
    set_subscription(SubscriptionStatusEnum::Aborted).await;
    store.delete_resource(TENANT_ID, user).await.unwrap();
    assert!(store.get_resource(TENANT_ID, user).await.is_err());
    let released: Option<uuid::Uuid> = {
        use diesel_models::schema::subscription::dsl as s;
        s::subscription
            .filter(s::id.eq(subscription_id))
            .select(s::resource_id)
            .first(&mut conn)
            .await
            .unwrap()
    };
    assert_eq!(released, None);

    // Deleting the tenant still cascades through a subscription for a resource.
    let kept = store
        .upsert_resource(upsert(customer, "user", "kept", &[]))
        .await
        .unwrap()
        .id;
    {
        use diesel_models::schema::subscription::dsl as s;
        diesel::update(s::subscription.filter(s::id.eq(subscription_id)))
            .set((
                s::resource_id.eq(Some(kept.as_uuid())),
                s::status.eq(SubscriptionStatusEnum::Active),
            ))
            .execute(&mut conn)
            .await
            .unwrap();
    }
    drop(conn);
    store.hard_delete_tenants(&[TENANT_ID]).await.unwrap();
}

/// Rolling phase 4 back gives each resource subscription a usage filter on its resource, so none
/// becomes customer-wide: the attribution property of a metric it prices, else the canonical key.
#[tokio::test]
async fn test_phase_four_rollback_keeps_resource_subscriptions_scoped() {
    use diesel_async::SimpleAsyncConnection;
    helpers::init::logging();
    let postgres_connection_string = meteroid_it::container::create_test_database().await;
    let setup = meteroid_it::container::start_meteroid(
        postgres_connection_string,
        SeedLevel::SUBSCRIPTIONS,
    )
    .await;
    let store = &setup.store;
    let site = store
        .create_resource_type(type_new("site", &["site_id", "website_id"], &[]))
        .await
        .unwrap();
    store
        .create_resource_type(type_new("zone", &["zone_id"], &[]))
        .await
        .unwrap();
    let mut conn = store.get_conn().await.unwrap();
    #[derive(diesel::QueryableByName)]
    struct Priced {
        #[diesel(sql_type = diesel::sql_types::Uuid)]
        subscription_id: uuid::Uuid,
        #[diesel(sql_type = diesel::sql_types::Uuid)]
        customer_id: uuid::Uuid,
    }
    conn.batch_execute(
        "INSERT INTO subscription SELECT (jsonb_populate_record(NULL::subscription, \
         to_jsonb(s) || jsonb_build_object('id', gen_random_uuid()))).* FROM subscription s LIMIT 1",
    )
    .await
    .unwrap();
    let priced: Vec<Priced> = diesel::sql_query(
        "SELECT id AS subscription_id, customer_id FROM subscription ORDER BY id LIMIT 2",
    )
    .load(&mut conn)
    .await
    .unwrap();
    assert_eq!(priced.len(), 2, "the seeded subscription and its copy");
    // A usage fee on the bandwidth metric, as a legacy component names it.
    let metric_id = crate::data::ids::METRIC_BANDWIDTH.as_uuid();
    conn.batch_execute(&format!(
        "INSERT INTO subscription_component (id, name, subscription_id, period, legacy_fee) \
         VALUES (gen_random_uuid(), 'Bandwidth', '{}', 'MONTHLY', \
         '{{\"usage\": {{\"metric_id\": \"{metric_id}\"}}}}')",
        priced[0].subscription_id
    ))
    .await
    .unwrap();
    let (attributed, plain) = (&priced[0], &priced[1]);
    for (sub, code, key) in [(attributed, "site", "s1"), (plain, "zone", "z1")] {
        let resource = store
            .upsert_resource(upsert(sub.customer_id.into(), code, key, &[]))
            .await
            .unwrap();
        conn.batch_execute(&format!(
            "UPDATE subscription SET resource_id = '{}', usage_filter_property = NULL, \
             usage_filter_value = NULL WHERE id = '{}'",
            resource.id.as_uuid(),
            sub.subscription_id
        ))
        .await
        .unwrap();
    }
    conn.batch_execute(&format!(
        "UPDATE billable_metric SET attributed_to_type_id = '{}', attribution_property = 'page_id' \
         WHERE id = '{}'",
        site.id.as_uuid(),
        metric_id
    ))
    .await
    .unwrap();

    let down =
        include_str!("../../migrations/diesel/2026-11-26-011852_entitlements_v2_billing/down.sql");
    let write_back = &down[..down
        .find("ALTER TABLE subscription_add_on DROP COLUMN usage_from")
        .unwrap()];
    conn.batch_execute(write_back).await.unwrap();

    #[derive(diesel::QueryableByName, Debug, PartialEq)]
    struct Filter {
        #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
        property: Option<String>,
        #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
        value: Option<String>,
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        versions: i64,
    }
    for (sub, property, value) in [(attributed, "page_id", "s1"), (plain, "zone_id", "z1")] {
        let filter: Filter = diesel::sql_query(
            "SELECT usage_filter_property AS property, usage_filter_value AS value, \
             (SELECT count(*) FROM subscription_usage_filter f WHERE f.subscription_id = s.id \
              AND f.property = s.usage_filter_property AND f.value = s.usage_filter_value) AS versions \
             FROM subscription s WHERE id = $1",
        )
        .bind::<diesel::sql_types::Uuid, _>(sub.subscription_id)
        .get_result(&mut conn)
        .await
        .unwrap();
        assert_eq!(
            filter,
            Filter {
                property: Some(property.into()),
                value: Some(value.into()),
                versions: 1,
            }
        );
    }
}

struct Rest {
    http: reqwest::Client,
    base: String,
    api_key: String,
}

impl Rest {
    async fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let request = self
            .http
            .request(method, format!("{}/api/v1{path}", self.base))
            .bearer_auth(&self.api_key);
        let request = match body {
            Some(body) => request.json(&body),
            None => request,
        };
        let res = request.send().await.unwrap();
        let status = res.status();
        let text = res.text().await.unwrap();
        (
            status,
            serde_json::from_str(&text).unwrap_or(Value::String(text)),
        )
    }

    async fn mint(&self, customer: CustomerId, body: Value) -> (StatusCode, Value) {
        let path = format!("/customers/{}/portal-token", customer.as_base62());
        self.send(reqwest::Method::POST, &path, Some(body)).await
    }
}

#[tokio::test]
async fn test_rest_resources_and_client_claims() {
    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::PRODUCT).await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::PRODUCT)
            .await;
    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        "testslug",
    );
    let api_key = clients
        .api_tokens
        .clone()
        .create_api_token(tonic::Request::new(
            api::apitokens::v1::CreateApiTokenRequest {
                name: "resources".to_string(),
                ..Default::default()
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .api_key;
    let rest = Rest {
        http: http_client_tracing::client(),
        base: setup.config.rest_api_external_url.clone(),
        api_key,
    };
    use reqwest::Method;
    let uber = CUST_UBER_ID.as_base62();

    let site = json!({
        "code": "site", "name": "Site", "plural_name": "Sites",
        "keys": ["site_id", "website_id"],
        "attributes": { "environment": ["production", "staging"] },
    });
    let (status, created) = rest.send(Method::POST, "/resource-types", Some(site)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["parent_type_id"], Value::Null);
    assert_eq!(created["archived"], false);

    let (status, body) = rest
        .send(Method::POST, "/resource-types", Some(json!({
            "code": "workspace", "name": "Workspace", "plural_name": "Workspaces", "keys": ["site_id"],
        })))
        .await;
    assert_eq!(
        (status, body["code"].as_str()),
        (StatusCode::CONFLICT, Some("RESOURCE_KEY_CLASH")),
        "{body}"
    );
    let (status, body) = rest
        .send(
            Method::POST,
            "/resource-types",
            Some(json!({
                "code": "page", "name": "Page", "plural_name": "Pages", "keys": ["page_id"],
                "parent_type_id": created["id"],
            })),
        )
        .await;
    assert_eq!(
        (status, body["code"].as_str()),
        (
            StatusCode::BAD_REQUEST,
            Some("RESOURCE_HIERARCHY_UNSUPPORTED")
        ),
        "{body}"
    );

    let (status, s1) = rest
        .send(
            Method::PUT,
            &format!("/customers/{uber}/resources/site/s1"),
            Some(json!({ "name": "Main", "attributes": { "environment": "staging" } })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{s1}");
    assert_eq!(s1["key_value"], "s1");
    let (status, listed) = rest
        .send(
            Method::GET,
            &format!("/customers/{uber}/resources?type=site"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed["data"].as_array().unwrap().len(), 1);
    let (status, body) = rest
        .send(
            Method::PATCH,
            "/resource-types/site",
            Some(json!({ "attributes": { "environment": ["production"] } })),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["message"].as_str().unwrap().contains("s1"), "{body}");

    // Client tokens carry the claims, validated against the tenant's types when minted.
    let mint = |body: Value| rest.mint(CUST_UBER_ID, body);
    let (status, minted) = mint(json!({
        "scopes": ["read"],
        "resources": { "website_id": "s1" },
        "attributes": { "site.environment": "staging" },
    }))
    .await;
    assert_eq!(status, StatusCode::OK, "{minted}");
    let token_store = &setup.store;
    let stored_claims = |token: String| async move {
        token_store
            .resolve_portal_token(&token)
            .await
            .unwrap()
            .expect("a stored token")
            .client
    };
    let token = minted["token"].as_str().unwrap();
    let claims = stored_claims(token.to_string()).await;
    assert_eq!(claims.resources, pairs(&[("website_id", "s1")]));
    let (_, again) = mint(json!({
        "scopes": ["read"],
        "resources": { "website_id": "s1" },
        "attributes": { "site.environment": "staging" },
    }))
    .await;
    assert_eq!(
        again["token"], minted["token"],
        "the same claims reuse the token"
    );
    let (_, other) = mint(json!({ "scopes": ["read"], "resources": { "site_id": "s1" } })).await;
    assert_ne!(
        other["token"], minted["token"],
        "other claims get their own token"
    );
    assert_eq!(
        stored_claims(other["token"].as_str().unwrap().to_string())
            .await
            .resources,
        pairs(&[("site_id", "s1")])
    );
    let (status, feature) = rest
        .send(
            Method::POST,
            "/features",
            Some(json!({
                "name": "Site SSO", "code": "site_sso", "kind": "BOOLEAN", "level": created["id"],
                "entitlement": { "type": "BOOLEAN", "enabled": true },
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{feature}");
    // The claim names the alias; a request names the resource by its type and key, or its id.
    let s1_id = s1["id"].as_str().unwrap().to_string();
    for (query, expected) in [
        (
            vec![("resource[type]", "site"), ("resource[key]", "s1")],
            StatusCode::OK,
        ),
        (vec![("resource", s1_id.as_str())], StatusCode::OK),
        (
            vec![("resource[type]", "site"), ("resource[key]", "s2")],
            StatusCode::FORBIDDEN,
        ),
    ] {
        let res = rest
            .http
            .get(format!("{}/api/client/v1/entitlements", rest.base))
            .query(&query)
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), expected, "{query:?}");
        if expected == StatusCode::OK {
            let snapshot: Value = res.json().await.unwrap();
            let sso = snapshot["entitlements"]
                .as_array()
                .unwrap()
                .iter()
                .find(|f| f["feature"]["code"] == "site_sso")
                .unwrap_or_else(|| panic!("{snapshot}"));
            let on_site = sso["levels"]
                .as_array()
                .unwrap()
                .iter()
                .find(|l| l["resource"] == json!({"type": "site", "key": "s1"}))
                .unwrap_or_else(|| panic!("{sso}"));
            assert_eq!(on_site["level"], "resource_type", "{sso}");
            assert_eq!(on_site["resource_type"]["code"], "site", "{sso}");
        }
    }
    for refused in [
        json!({ "resources": { "user_id": "u1" } }),
        json!({ "resources": { "site_id": "s1", "website_id": "s1" } }),
        json!({ "attributes": { "site.environment": "staging" } }),
        json!({ "resources": { "site_id": "s1" }, "attributes": { "site.environment": "qa" } }),
    ] {
        let (status, body) = mint(refused.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}: {body}");
    }
    let grpc_mint = |resources: &[(&str, &str)]| {
        let mut customers = clients.customers.clone();
        let request = api::customers::v1::GenerateCustomerPortalTokenRequest {
            customer_id: uber.clone(),
            resources: pairs(resources).into_iter().collect(),
            ..Default::default()
        };
        async move { customers.generate_customer_portal_token(request).await }
    };
    let token = grpc_mint(&[("website_id", "s1")])
        .await
        .unwrap()
        .into_inner()
        .token;
    let grpc_claims = stored_claims(token).await;
    assert_eq!(grpc_claims.resources, pairs(&[("website_id", "s1")]));
    let refused = grpc_mint(&[("user_id", "u1")]).await.unwrap_err();
    assert_eq!(refused.code(), tonic::Code::InvalidArgument, "{refused:?}");

    let (status, unclaimed) = mint(json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(unclaimed["token"], minted["token"]);
    let unclaimed = stored_claims(unclaimed["token"].as_str().unwrap().to_string()).await;
    assert!(unclaimed.is_empty());
    assert!(!unclaimed.allows(&input(&[("site_id", "s9")], &[]), &[]));

    let (status, archived) = rest
        .send(
            Method::POST,
            &format!("/customers/{uber}/resources/site/s1/archive"),
            None,
        )
        .await;
    assert_eq!(
        (status, &archived["archived"]),
        (StatusCode::OK, &json!(true)),
        "{archived}"
    );
    let (status, _) = rest
        .send(
            Method::DELETE,
            &format!("/customers/{uber}/resources/site/s1"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = rest
        .send(
            Method::GET,
            &format!("/customers/{uber}/resources/site/s1"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
