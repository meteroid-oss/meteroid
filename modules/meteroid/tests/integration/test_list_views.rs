//! Saved list views: visibility (private/shared), edit rights, per-user defaults, duplication,
//! validation, quota and tenant isolation (store level), plus a gRPC smoke test covering the
//! RBAC classification and API-key rejection.

use std::str::FromStr;

use common_domain::ids::{BaseId, EntityListViewId, TenantId, UserId};
use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use diesel_models::enums::OrganizationUserRole as DbOrganizationUserRole;
use diesel_models::organization_members::OrganizationMemberRow;
use diesel_models::users::UserRowNew;
use http::{HeaderName, HeaderValue};
use rstest::rstest;
use serde_json::json;
use tonic::Code;
use tonic::transport::Channel;
use tower_http::set_header::{SetRequestHeader, SetRequestHeaderLayer};

use common_grpc::middleware::common::auth::API_KEY_HEADER;
use meteroid_grpc::meteroid::api::listviews::v1 as grpc;
use meteroid_grpc::meteroid::api::listviews::v1::EntityListType as ProtoEntityType;
use meteroid_grpc::meteroid::api::listviews::v1::list_views_service_client::ListViewsServiceClient;
use meteroid_store::StoreResult;
use meteroid_store::domain::entity_list_views::{
    EntityListView, EntityListViewNew, EntityListViewPatch,
};
use meteroid_store::domain::enums::{EntityListType, TenantEnvironmentEnum};
use meteroid_store::domain::tenants::TenantNew;
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::{EntityListViewsInterface, TenantInterface};

use crate::data::ids::*;
use crate::harness::{TestEnv, test_env_minimal};
use crate::meteroid_it;
use crate::meteroid_it::clients::AllClients;

const CUSTOMER: EntityListType = EntityListType::Customer;
const SUBSCRIPTION: EntityListType = EntityListType::Subscription;

/// Inserts a second org member ("Ada Lovelace") into the seeded organization.
async fn insert_member(env: &TestEnv, email: &str) -> UserId {
    use diesel_models::schema::user::dsl as u;

    let id = UserId::new();
    let mut conn = env.conn().await;
    UserRowNew {
        id,
        email: email.to_string(),
        password_hash: None,
    }
    .insert(&mut conn)
    .await
    .expect("insert user");
    diesel::update(u::user.filter(u::id.eq(id)))
        .set((u::first_name.eq("Ada"), u::last_name.eq("Lovelace")))
        .execute(&mut conn)
        .await
        .expect("name user");
    OrganizationMemberRow {
        user_id: id,
        organization_id: ORGANIZATION_ID,
        role: DbOrganizationUserRole::Member,
        role_id: None,
    }
    .insert(&mut conn)
    .await
    .expect("insert member");
    id
}

fn view(entity_type: EntityListType, name: &str, shared: bool) -> EntityListViewNew {
    EntityListViewNew {
        entity_type,
        name: name.to_string(),
        config: json!({ "v": 1, "filters": [] }),
        shared,
    }
}

async fn create(
    env: &TestEnv,
    actor: UserId,
    entity_type: EntityListType,
    name: &str,
    shared: bool,
) -> EntityListView {
    env.store()
        .create_view(TENANT_ID, actor, view(entity_type, name, shared))
        .await
        .expect("create view")
}

async fn visible_ids(
    env: &TestEnv,
    actor: UserId,
    entity_type: EntityListType,
) -> Vec<EntityListViewId> {
    env.store()
        .list_views(TENANT_ID, actor, entity_type)
        .await
        .expect("list views")
        .views
        .into_iter()
        .map(|v| v.id)
        .collect()
}

async fn default_of(
    env: &TestEnv,
    actor: UserId,
    entity_type: EntityListType,
) -> Option<EntityListViewId> {
    env.store()
        .list_views(TENANT_ID, actor, entity_type)
        .await
        .expect("list views")
        .default_view_id
}

fn rename(name: &str) -> EntityListViewPatch {
    EntityListViewPatch {
        name: Some(name.to_string()),
        ..Default::default()
    }
}

fn share(shared: bool) -> EntityListViewPatch {
    EntityListViewPatch {
        shared: Some(shared),
        ..Default::default()
    }
}

#[track_caller]
fn assert_not_found<T: std::fmt::Debug>(res: StoreResult<T>) {
    let err = res.expect_err("expected ValueNotFound");
    assert!(
        matches!(err.current_context(), StoreError::ValueNotFound(_)),
        "expected ValueNotFound, got {err:?}"
    );
}

#[track_caller]
fn assert_invalid<T: std::fmt::Debug>(res: StoreResult<T>) {
    let err = res.expect_err("expected InvalidArgument");
    assert!(
        matches!(err.current_context(), StoreError::InvalidArgument(_)),
        "expected InvalidArgument, got {err:?}"
    );
}

#[track_caller]
fn assert_forbidden<T: std::fmt::Debug>(res: StoreResult<T>) {
    let err = res.expect_err("expected Forbidden");
    assert!(
        matches!(err.current_context(), StoreError::Forbidden(_)),
        "expected Forbidden, got {err:?}"
    );
}

#[rstest]
#[tokio::test]
async fn test_private_view_visible_only_to_creator(#[future] test_env_minimal: TestEnv) {
    let env = test_env_minimal.await;
    let store = env.store();
    let other = insert_member(&env, "other@meteroid.dev").await;

    let private = create(&env, USER_ID, CUSTOMER, "Mine", false).await;
    assert!(!private.shared);
    assert_eq!(private.created_by, Some(USER_ID));

    assert_eq!(visible_ids(&env, USER_ID, CUSTOMER).await, vec![private.id]);
    assert!(visible_ids(&env, other, CUSTOMER).await.is_empty());

    // Every by-id operation treats it as missing for the other user.
    assert_not_found(
        store
            .update_view(TENANT_ID, other, private.id, rename("Hijacked"))
            .await,
    );
    assert_not_found(
        store
            .update_view(TENANT_ID, other, private.id, share(true))
            .await,
    );
    assert_not_found(
        store
            .duplicate_view(TENANT_ID, other, private.id, None)
            .await,
    );
    assert_not_found(
        store
            .set_default_view(TENANT_ID, other, CUSTOMER, Some(private.id))
            .await,
    );
    assert_not_found(store.delete_view(TENANT_ID, other, private.id).await);

    // Nothing changed: still there, still named "Mine".
    let listed = store
        .list_views(TENANT_ID, USER_ID, CUSTOMER)
        .await
        .unwrap()
        .views;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "Mine");
    assert!(!listed[0].shared);

    // And the other user's private view is hidden from the seeded user.
    let theirs = create(&env, other, CUSTOMER, "Theirs", false).await;
    assert_eq!(visible_ids(&env, USER_ID, CUSTOMER).await, vec![private.id]);
    assert_eq!(visible_ids(&env, other, CUSTOMER).await, vec![theirs.id]);

    // Unknown id is indistinguishable from a hidden one.
    assert_not_found(
        store
            .delete_view(TENANT_ID, USER_ID, EntityListViewId::new())
            .await,
    );

    // The creator can delete it.
    store
        .delete_view(TENANT_ID, USER_ID, private.id)
        .await
        .unwrap();
    assert!(visible_ids(&env, USER_ID, CUSTOMER).await.is_empty());
}

#[rstest]
#[tokio::test]
async fn test_shared_view_edit_rights(#[future] test_env_minimal: TestEnv) {
    let env = test_env_minimal.await;
    let store = env.store();
    let other = insert_member(&env, "other@meteroid.dev").await;

    let shared = create(&env, USER_ID, CUSTOMER, "Team", true).await;
    assert!(shared.shared);
    assert_eq!(visible_ids(&env, USER_ID, CUSTOMER).await, vec![shared.id]);
    assert_eq!(visible_ids(&env, other, CUSTOMER).await, vec![shared.id]);

    // Non-owner can rename and replace the config.
    let updated = store
        .update_view(
            TENANT_ID,
            other,
            shared.id,
            EntityListViewPatch {
                name: Some("  Team (edited)  ".to_string()),
                config: Some(json!({ "v": 1, "sort": "name" })),
                shared: None,
            },
        )
        .await
        .expect("non-owner edits shared view");
    assert_eq!(updated.name, "Team (edited)");
    assert_eq!(updated.config, json!({ "v": 1, "sort": "name" }));
    assert!(updated.shared);
    assert_eq!(updated.created_by, Some(USER_ID), "ownership is unchanged");
    assert!(updated.updated_at.is_some());

    // Non-owner cannot toggle sharing...
    assert_forbidden(
        store
            .update_view(TENANT_ID, other, shared.id, share(false))
            .await,
    );
    // ...and a forbidden patch applies nothing, even its other fields.
    assert_forbidden(
        store
            .update_view(
                TENANT_ID,
                other,
                shared.id,
                EntityListViewPatch {
                    name: Some("Sneaky".to_string()),
                    config: None,
                    shared: Some(false),
                },
            )
            .await,
    );
    let listed = store
        .list_views(TENANT_ID, USER_ID, CUSTOMER)
        .await
        .unwrap()
        .views;
    assert_eq!(listed[0].name, "Team (edited)");
    assert!(listed[0].shared);

    // Restating the current value is a no-op on sharing and is allowed.
    let same = store
        .update_view(
            TENANT_ID,
            other,
            shared.id,
            EntityListViewPatch {
                name: Some("Team again".to_string()),
                config: None,
                shared: Some(true),
            },
        )
        .await
        .expect("shared: Some(same) by non-owner is allowed");
    assert_eq!(same.name, "Team again");
    assert!(same.shared);

    // Other user makes it their default.
    store
        .set_default_view(TENANT_ID, other, CUSTOMER, Some(shared.id))
        .await
        .unwrap();
    assert_eq!(default_of(&env, other, CUSTOMER).await, Some(shared.id));

    // Creator unshares: the other user loses sight of it and of their default.
    let unshared = store
        .update_view(TENANT_ID, USER_ID, shared.id, share(false))
        .await
        .expect("owner unshares");
    assert!(!unshared.shared);
    assert!(visible_ids(&env, other, CUSTOMER).await.is_empty());
    assert_eq!(default_of(&env, other, CUSTOMER).await, None);
    assert_not_found(
        store
            .update_view(TENANT_ID, other, shared.id, rename("x"))
            .await,
    );
    assert_eq!(visible_ids(&env, USER_ID, CUSTOMER).await, vec![shared.id]);

    // Unsharing drops other users' defaults, so re-sharing doesn't silently restore them; the
    // creator's own default is kept.
    store
        .update_view(TENANT_ID, USER_ID, shared.id, share(true))
        .await
        .expect("owner re-shares");
    assert_eq!(default_of(&env, other, CUSTOMER).await, None);
    for user in [USER_ID, other] {
        store
            .set_default_view(TENANT_ID, user, CUSTOMER, Some(shared.id))
            .await
            .unwrap();
    }
    store
        .update_view(TENANT_ID, USER_ID, shared.id, share(false))
        .await
        .expect("owner unshares again");
    store
        .update_view(TENANT_ID, USER_ID, shared.id, share(true))
        .await
        .expect("owner re-shares again");
    assert_eq!(default_of(&env, other, CUSTOMER).await, None);
    assert_eq!(default_of(&env, USER_ID, CUSTOMER).await, Some(shared.id));

    // Non-owner can delete a shared view.
    store
        .delete_view(TENANT_ID, other, shared.id)
        .await
        .expect("non-owner deletes shared view");
    assert!(visible_ids(&env, USER_ID, CUSTOMER).await.is_empty());
    assert!(visible_ids(&env, other, CUSTOMER).await.is_empty());
}

#[rstest]
#[tokio::test]
async fn test_defaults_are_per_user(#[future] test_env_minimal: TestEnv) {
    let env = test_env_minimal.await;
    let store = env.store();
    let other = insert_member(&env, "other@meteroid.dev").await;

    let x = create(&env, USER_ID, CUSTOMER, "X", true).await;
    let y = create(&env, other, CUSTOMER, "Y", false).await;
    let sub_view = create(&env, USER_ID, SUBSCRIPTION, "Subs", true).await;

    assert_eq!(default_of(&env, USER_ID, CUSTOMER).await, None);

    let set = store
        .set_default_view(TENANT_ID, USER_ID, CUSTOMER, Some(x.id))
        .await
        .unwrap();
    assert_eq!(set, Some(x.id));
    assert_eq!(default_of(&env, USER_ID, CUSTOMER).await, Some(x.id));
    assert_eq!(default_of(&env, other, CUSTOMER).await, None);
    // Defaults are also per list.
    assert_eq!(default_of(&env, USER_ID, SUBSCRIPTION).await, None);

    // Other user picks a different default; A's is untouched.
    store
        .set_default_view(TENANT_ID, other, CUSTOMER, Some(y.id))
        .await
        .unwrap();
    assert_eq!(default_of(&env, other, CUSTOMER).await, Some(y.id));
    assert_eq!(default_of(&env, USER_ID, CUSTOMER).await, Some(x.id));

    // Switching overwrites (one default per user per list).
    store
        .set_default_view(TENANT_ID, other, CUSTOMER, Some(x.id))
        .await
        .unwrap();
    assert_eq!(default_of(&env, other, CUSTOMER).await, Some(x.id));

    // A view of another list is rejected.
    assert_invalid(
        store
            .set_default_view(TENANT_ID, USER_ID, CUSTOMER, Some(sub_view.id))
            .await,
    );
    // An invisible view is not found.
    assert_not_found(
        store
            .set_default_view(TENANT_ID, USER_ID, CUSTOMER, Some(y.id))
            .await,
    );
    assert_not_found(
        store
            .set_default_view(TENANT_ID, USER_ID, CUSTOMER, Some(EntityListViewId::new()))
            .await,
    );
    assert_eq!(default_of(&env, USER_ID, CUSTOMER).await, Some(x.id));

    // None clears (and is idempotent).
    let cleared = store
        .set_default_view(TENANT_ID, USER_ID, CUSTOMER, None)
        .await
        .unwrap();
    assert_eq!(cleared, None);
    assert_eq!(default_of(&env, USER_ID, CUSTOMER).await, None);
    store
        .set_default_view(TENANT_ID, USER_ID, CUSTOMER, None)
        .await
        .unwrap();
    assert_eq!(default_of(&env, other, CUSTOMER).await, Some(x.id));

    // Deleting a view removes every default pointing at it (FK cascade).
    store
        .set_default_view(TENANT_ID, USER_ID, CUSTOMER, Some(x.id))
        .await
        .unwrap();
    store.delete_view(TENANT_ID, USER_ID, x.id).await.unwrap();
    assert_eq!(default_of(&env, USER_ID, CUSTOMER).await, None);
    assert_eq!(default_of(&env, other, CUSTOMER).await, None);

    use diesel_models::schema::entity_list_view_default::dsl as d;
    let mut conn = env.conn().await;
    let remaining: i64 = d::entity_list_view_default
        .filter(d::view_id.eq(x.id))
        .count()
        .get_result(&mut conn)
        .await
        .unwrap();
    assert_eq!(remaining, 0, "default rows are cascaded with the view");
}

#[rstest]
#[tokio::test]
async fn test_duplicate_view(#[future] test_env_minimal: TestEnv) {
    let env = test_env_minimal.await;
    let store = env.store();
    let other = insert_member(&env, "other@meteroid.dev").await;

    let original = store
        .create_view(
            TENANT_ID,
            USER_ID,
            EntityListViewNew {
                entity_type: SUBSCRIPTION,
                name: "Churn risk".to_string(),
                config: json!({ "v": 1, "columns": ["name", "mrr"] }),
                shared: true,
            },
        )
        .await
        .unwrap();

    // Default name, private copy owned by the caller, same config and list.
    let copy = store
        .duplicate_view(TENANT_ID, other, original.id, None)
        .await
        .unwrap();
    assert_ne!(copy.id, original.id);
    assert_eq!(copy.name, "Churn risk (copy)");
    assert_eq!(copy.created_by, Some(other));
    assert_eq!(copy.created_by_name.as_deref(), Some("Ada Lovelace"));
    assert!(!copy.shared);
    assert_eq!(copy.entity_type, SUBSCRIPTION);
    assert_eq!(copy.config, original.config);
    // The copy is private to the duplicating user.
    assert_eq!(
        visible_ids(&env, other, SUBSCRIPTION).await,
        vec![original.id, copy.id]
    );
    assert_eq!(
        visible_ids(&env, USER_ID, SUBSCRIPTION).await,
        vec![original.id]
    );

    // Custom name (trimmed).
    let named = store
        .duplicate_view(
            TENANT_ID,
            USER_ID,
            original.id,
            Some("  Mine  ".to_string()),
        )
        .await
        .unwrap();
    assert_eq!(named.name, "Mine");
    assert_eq!(named.created_by, Some(USER_ID));
    assert_invalid(
        store
            .duplicate_view(TENANT_ID, USER_ID, original.id, Some("   ".to_string()))
            .await,
    );

    // A 100-char name stays within the limit once suffixed.
    let long_name = "n".repeat(100);
    let long = create(&env, USER_ID, CUSTOMER, &long_name, false).await;
    let long_copy = store
        .duplicate_view(TENANT_ID, USER_ID, long.id, None)
        .await
        .expect("duplicating a max-length name");
    assert!(long_copy.name.chars().count() <= 100);
    assert!(long_copy.name.ends_with(" (copy)"));
    assert_eq!(long_copy.name, format!("{} (copy)", "n".repeat(93)));

    // Same with multi-byte characters (the limit is in characters).
    let wide = create(&env, USER_ID, CUSTOMER, &"é".repeat(100), false).await;
    let wide_copy = store
        .duplicate_view(TENANT_ID, USER_ID, wide.id, None)
        .await
        .expect("duplicating a max-length multi-byte name");
    assert_eq!(wide_copy.name.chars().count(), 100);
}

#[rstest]
#[tokio::test]
async fn test_view_validation(#[future] test_env_minimal: TestEnv) {
    let env = test_env_minimal.await;
    let store = env.store();

    let mk = |name: &str, config: serde_json::Value| EntityListViewNew {
        entity_type: CUSTOMER,
        name: name.to_string(),
        config,
        shared: false,
    };

    assert_invalid(
        store
            .create_view(TENANT_ID, USER_ID, mk("", json!({})))
            .await,
    );
    assert_invalid(
        store
            .create_view(TENANT_ID, USER_ID, mk(" \t\n ", json!({})))
            .await,
    );
    assert_invalid(
        store
            .create_view(TENANT_ID, USER_ID, mk(&"a".repeat(101), json!({})))
            .await,
    );
    assert_invalid(
        store
            .create_view(TENANT_ID, USER_ID, mk("arr", json!([1, 2])))
            .await,
    );
    assert_invalid(
        store
            .create_view(TENANT_ID, USER_ID, mk("str", json!("x")))
            .await,
    );
    assert_invalid(
        store
            .create_view(TENANT_ID, USER_ID, mk("null", serde_json::Value::Null))
            .await,
    );
    assert_invalid(
        store
            .create_view(
                TENANT_ID,
                USER_ID,
                mk("big", json!({ "blob": "a".repeat(32 * 1024) })),
            )
            .await,
    );
    assert!(visible_ids(&env, USER_ID, CUSTOMER).await.is_empty());

    // Boundaries accepted; names trimmed (and trimming happens before the length check).
    let exact = store
        .create_view(TENANT_ID, USER_ID, mk(&"a".repeat(100), json!({})))
        .await
        .expect("100 chars is allowed");
    assert_eq!(exact.name.len(), 100);
    let padded = store
        .create_view(
            TENANT_ID,
            USER_ID,
            mk(&format!("  {}  ", "b".repeat(100)), json!({})),
        )
        .await
        .expect("padding does not count");
    assert_eq!(padded.name, "b".repeat(100));
    let trimmed = store
        .create_view(TENANT_ID, USER_ID, mk("  Trimmed  ", json!({})))
        .await
        .unwrap();
    assert_eq!(trimmed.name, "Trimmed");
    store
        .create_view(
            TENANT_ID,
            USER_ID,
            mk("near-limit", json!({ "blob": "a".repeat(30 * 1024) })),
        )
        .await
        .expect("config under 32KiB is allowed");

    // Same rules on update.
    assert_invalid(
        store
            .update_view(TENANT_ID, USER_ID, trimmed.id, rename("   "))
            .await,
    );
    assert_invalid(
        store
            .update_view(TENANT_ID, USER_ID, trimmed.id, rename(&"c".repeat(101)))
            .await,
    );
    assert_invalid(
        store
            .update_view(
                TENANT_ID,
                USER_ID,
                trimmed.id,
                EntityListViewPatch {
                    config: Some(json!([])),
                    ..Default::default()
                },
            )
            .await,
    );
    assert_invalid(
        store
            .update_view(
                TENANT_ID,
                USER_ID,
                trimmed.id,
                EntityListViewPatch {
                    config: Some(json!({ "blob": "a".repeat(32 * 1024) })),
                    ..Default::default()
                },
            )
            .await,
    );
    let renamed = store
        .update_view(TENANT_ID, USER_ID, trimmed.id, rename("  Renamed "))
        .await
        .unwrap();
    assert_eq!(renamed.name, "Renamed");
    assert_eq!(renamed.config, json!({}), "config untouched by a rename");
}

#[rstest]
#[tokio::test]
async fn test_view_quota(#[future] test_env_minimal: TestEnv) {
    let env = test_env_minimal.await;
    let store = env.store();
    let other = insert_member(&env, "other@meteroid.dev").await;

    for i in 0..50 {
        create(&env, USER_ID, CUSTOMER, &format!("v{i}"), i % 2 == 0).await;
    }
    assert_invalid(
        store
            .create_view(TENANT_ID, USER_ID, view(CUSTOMER, "v50", false))
            .await,
    );

    // Duplicating is subject to the same quota.
    let first = visible_ids(&env, USER_ID, CUSTOMER).await[0];
    assert_invalid(store.duplicate_view(TENANT_ID, USER_ID, first, None).await);

    // The quota is per list and per owner.
    create(&env, USER_ID, SUBSCRIPTION, "other list", false).await;
    create(&env, other, CUSTOMER, "other user", false).await;
    // Other user can duplicate a shared view (it counts against their quota, not the owner's).
    store
        .duplicate_view(TENANT_ID, other, first, None)
        .await
        .expect("other user duplicates");

    // Freeing one slot allows a new view.
    store.delete_view(TENANT_ID, USER_ID, first).await.unwrap();
    create(&env, USER_ID, CUSTOMER, "v50", false).await;
}

#[rstest]
#[tokio::test]
async fn test_list_order_and_creator_names(#[future] test_env_minimal: TestEnv) {
    let env = test_env_minimal.await;
    let store = env.store();
    let other = insert_member(&env, "other@meteroid.dev").await;

    let a = create(&env, USER_ID, CUSTOMER, "First", true).await;
    let b = create(&env, other, CUSTOMER, "Second", true).await;
    let c = create(&env, USER_ID, CUSTOMER, "Third", false).await;
    // A view on another list never leaks into this one.
    create(&env, USER_ID, SUBSCRIPTION, "Elsewhere", true).await;

    // Renaming the oldest doesn't reorder (order is by creation).
    store
        .update_view(TENANT_ID, USER_ID, a.id, rename("First (renamed)"))
        .await
        .unwrap();

    let listed = store
        .list_views(TENANT_ID, USER_ID, CUSTOMER)
        .await
        .unwrap()
        .views;
    assert_eq!(
        listed.iter().map(|v| v.id).collect::<Vec<_>>(),
        vec![a.id, b.id, c.id]
    );
    assert!(
        listed
            .windows(2)
            .all(|w| w[0].created_at <= w[1].created_at)
    );
    // Seeded user has no first/last name → email; second user → full name.
    assert_eq!(
        listed[0].created_by_name.as_deref(),
        Some("demo-user@meteroid.dev")
    );
    assert_eq!(listed[1].created_by_name.as_deref(), Some("Ada Lovelace"));
    assert_eq!(a.created_by_name.as_deref(), Some("demo-user@meteroid.dev"));
    assert!(listed.iter().all(|v| v.entity_type == CUSTOMER));

    // The other user sees the two shared ones, in the same order.
    assert_eq!(visible_ids(&env, other, CUSTOMER).await, vec![a.id, b.id]);
}

#[rstest]
#[tokio::test]
async fn test_views_are_tenant_isolated(#[future] test_env_minimal: TestEnv) {
    let env = test_env_minimal.await;
    let store = env.store();

    let tenant_b: TenantId = store
        .insert_tenant(
            TenantNew {
                name: "Other tenant".to_string(),
                environment: TenantEnvironmentEnum::Development,
                email_delivery_mode: Some(meteroid_store::domain::enums::EmailDeliveryMode::Off),
                invoicing_entity: None,
            },
            ORGANIZATION_ID,
        )
        .await
        .expect("insert tenant")
        .id;

    let shared = create(&env, USER_ID, CUSTOMER, "Tenant A shared", true).await;
    let private = create(&env, USER_ID, CUSTOMER, "Tenant A private", false).await;
    store
        .set_default_view(TENANT_ID, USER_ID, CUSTOMER, Some(shared.id))
        .await
        .unwrap();

    let in_b = store.list_views(tenant_b, USER_ID, CUSTOMER).await.unwrap();
    assert!(in_b.views.is_empty());
    assert_eq!(in_b.default_view_id, None);

    for id in [shared.id, private.id] {
        assert_not_found(store.update_view(tenant_b, USER_ID, id, rename("x")).await);
        assert_not_found(store.duplicate_view(tenant_b, USER_ID, id, None).await);
        assert_not_found(
            store
                .set_default_view(tenant_b, USER_ID, CUSTOMER, Some(id))
                .await,
        );
        assert_not_found(store.delete_view(tenant_b, USER_ID, id).await);
    }

    // Views created in B stay in B, with an independent quota/default.
    let b_view = store
        .create_view(tenant_b, USER_ID, view(CUSTOMER, "Tenant B", true))
        .await
        .unwrap();
    assert_eq!(b_view.tenant_id, tenant_b);
    assert_eq!(
        visible_ids(&env, USER_ID, CUSTOMER).await,
        vec![shared.id, private.id]
    );
    assert_eq!(default_of(&env, USER_ID, CUSTOMER).await, Some(shared.id));
}

// ── gRPC ────────────────────────────────────────────────────────────────────

#[rstest]
#[tokio::test]
async fn test_list_views_grpc_roundtrip(#[future] test_env_minimal: TestEnv) {
    let env = test_env_minimal.await;
    let channel = env.setup.channel.clone();

    let auth = meteroid_it::svc_auth::login(channel.clone()).await;
    let clients = AllClients::from_channel(channel, auth.token.as_str(), "TESTORG", "testslug");
    let mut svc = clients.list_views.clone();

    let empty = svc
        .list_views(grpc::ListViewsRequest {
            entity_type: ProtoEntityType::Customer as i32,
        })
        .await
        .expect("ListViews as a signed-in user")
        .into_inner();
    assert!(empty.views.is_empty());
    assert_eq!(empty.default_view_id, None);

    let created = svc
        .create_view(grpc::CreateViewRequest {
            entity_type: ProtoEntityType::Customer as i32,
            name: " Big accounts ".to_string(),
            config: r#"{"v":1,"filters":[]}"#.to_string(),
            shared: true,
        })
        .await
        .expect("CreateView")
        .into_inner()
        .view
        .unwrap();
    assert_eq!(created.name, "Big accounts");
    assert!(created.shared);
    assert!(created.is_owner);
    assert_eq!(
        created.created_by.as_deref(),
        Some(USER_ID.as_proto().as_str())
    );
    assert_eq!(
        created.created_by_name.as_deref(),
        Some("demo-user@meteroid.dev")
    );
    assert_eq!(
        serde_json::Value::from_str(&created.config).unwrap(),
        json!({"v": 1, "filters": []})
    );

    let set = svc
        .set_default_view(grpc::SetDefaultViewRequest {
            entity_type: ProtoEntityType::Customer as i32,
            view_id: Some(created.id.clone()),
        })
        .await
        .expect("SetDefaultView")
        .into_inner();
    assert_eq!(set.default_view_id.as_deref(), Some(created.id.as_str()));

    let listed = svc
        .list_views(grpc::ListViewsRequest {
            entity_type: ProtoEntityType::Customer as i32,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(listed.views.len(), 1);
    assert_eq!(listed.views[0].id, created.id);
    assert_eq!(listed.default_view_id.as_deref(), Some(created.id.as_str()));

    // Store errors map onto gRPC codes.
    let bad = svc
        .create_view(grpc::CreateViewRequest {
            entity_type: ProtoEntityType::Customer as i32,
            name: "   ".to_string(),
            config: "{}".to_string(),
            shared: false,
        })
        .await
        .unwrap_err();
    assert_eq!(bad.code(), Code::InvalidArgument);
    let missing = svc
        .delete_view(grpc::DeleteViewRequest {
            id: EntityListViewId::new().as_proto(),
        })
        .await
        .unwrap_err();
    assert_eq!(missing.code(), Code::NotFound);
}

fn api_key_svc(channel: &Channel, api_key: &str) -> SetRequestHeader<Channel, HeaderValue> {
    tower::ServiceBuilder::new()
        .layer(SetRequestHeaderLayer::if_not_present(
            HeaderName::from_str(API_KEY_HEADER).unwrap(),
            HeaderValue::from_str(api_key).unwrap(),
        ))
        .service(channel.clone())
}

#[rstest]
#[tokio::test]
async fn test_list_views_rejects_api_keys(#[future] test_env_minimal: TestEnv) {
    let env = test_env_minimal.await;
    let channel = env.setup.channel.clone();

    let api_key = crate::test_auth_api_key::generate_api_key(&channel)
        .await
        .api_key;
    let mut svc = ListViewsServiceClient::new(api_key_svc(&channel, &api_key));

    let list = svc
        .list_views(grpc::ListViewsRequest {
            entity_type: ProtoEntityType::Customer as i32,
        })
        .await
        .expect_err("API keys must not reach saved views");
    assert_eq!(list.code(), Code::Unauthenticated, "{list:?}");

    let create = svc
        .create_view(grpc::CreateViewRequest {
            entity_type: ProtoEntityType::Customer as i32,
            name: "From API key".to_string(),
            config: "{}".to_string(),
            shared: true,
        })
        .await
        .expect_err("API keys must not create views");
    assert_eq!(create.code(), Code::Unauthenticated, "{create:?}");
}
