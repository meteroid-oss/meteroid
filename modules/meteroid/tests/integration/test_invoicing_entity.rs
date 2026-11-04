//! Audit-trail coverage for invoicing-entity ("billing settings") edits.

use common_domain::actor::Actor;
use common_domain::ids::BaseId;
use diesel_models::entity_activity::EntityActivityRow;
use diesel_models::enums::ActorTypeEnum;
use diesel_models::invoicing_entities::InvoicingEntityRowProvidersPatch;
use rstest::rstest;

use meteroid_store::domain::{InvoicingEntityPatch, InvoicingEntityProvidersPatch};
use meteroid_store::repositories::invoicing_entities::{
    InvoicingEntityInterface, InvoicingEntityInterfaceAuto,
};

use crate::data::ids::*;
use crate::harness::{TestEnv, test_env};

/// Field names carried by an `entity.updated` audit row's `{ changes: [{ field }] }` metadata.
fn changed_fields(row: &EntityActivityRow) -> Vec<String> {
    row.metadata
        .as_ref()
        .and_then(|m| m.get("changes"))
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|c| c.get("field").and_then(|f| f.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

async fn entity_activities(env: &TestEnv) -> Vec<EntityActivityRow> {
    let mut conn = env.conn().await;
    EntityActivityRow::list_by_entity(
        &mut conn,
        TENANT_ID,
        "invoicing_entity",
        INVOICING_ENTITY_ID.as_uuid(),
        None,
        50,
    )
    .await
    .expect("list activities")
}

/// Editing an invoicing entity records one `entity.updated` audit row carrying the
/// value-level field diff — and a no-op patch records nothing.
#[rstest]
#[tokio::test]
async fn test_patch_invoicing_entity_records_field_diff(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let before = env
        .store()
        .get_invoicing_entity(TENANT_ID, Some(INVOICING_ENTITY_ID))
        .await
        .expect("get invoicing entity");
    let new_net_terms = before.net_terms + 5;

    env.store()
        .patch_invoicing_entity(
            Actor::User { id: USER_ID },
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                net_terms: Some(new_net_terms),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .expect("patch invoicing entity");

    let activities = entity_activities(&env).await;
    assert_eq!(activities.len(), 1, "expected exactly one audit row");
    let activity = &activities[0];
    assert_eq!(activity.activity_type, "entity.updated");
    assert_eq!(activity.actor_type, ActorTypeEnum::User);
    assert_eq!(activity.actor_uuid, Some(USER_ID.as_uuid()));
    assert!(
        changed_fields(activity).contains(&"net_terms".to_string()),
        "expected net_terms in changes, got {:?}",
        changed_fields(activity)
    );

    // Same value again → nothing changed → no new audit row.
    env.store()
        .patch_invoicing_entity(
            Actor::User { id: USER_ID },
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                net_terms: Some(new_net_terms),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .expect("no-op patch invoicing entity");

    assert_eq!(
        entity_activities(&env).await.len(),
        1,
        "a no-op patch must not record an audit row"
    );
}

/// Changing payment providers (the Payment methods tab) records an `entity.updated`
/// audit row naming the providers that changed.
#[rstest]
#[tokio::test]
async fn test_patch_invoicing_entity_providers_records_change(#[future] test_env: TestEnv) {
    let env = test_env.await;

    // Seeds a mock connector and points card_provider at it via the raw row patch,
    // which does not itself record an activity.
    crate::data::payment::run_mock_payment_provider_seed(env.pool(), false).await;
    assert!(
        entity_activities(&env).await.is_empty(),
        "seeding providers must not record an audit row"
    );

    // Clear the just-seeded card provider; leave the other providers untouched so the
    // diff is deterministic regardless of the base seed's provider configuration.
    env.store()
        .patch_invoicing_entity_providers(
            Actor::User { id: USER_ID },
            InvoicingEntityProvidersPatch {
                id: INVOICING_ENTITY_ID,
                card_provider_id: Some(None),
                direct_debit_provider_id: None,
                bank_account_id: None,
            },
            TENANT_ID,
        )
        .await
        .expect("patch invoicing entity providers");

    let activities = entity_activities(&env).await;
    assert_eq!(activities.len(), 1, "expected one provider audit row");
    let activity = &activities[0];
    assert_eq!(activity.activity_type, "entity.updated");
    assert_eq!(activity.actor_type, ActorTypeEnum::User);
    assert_eq!(
        changed_fields(activity),
        vec!["card_provider".to_string()],
        "expected only card_provider to change"
    );
}

/// Rows returned by the write paths must map `bank_account_id` and
/// `direct_debit_provider_id` to their own columns, not swap them.
#[rstest]
#[tokio::test]
async fn test_patch_invoicing_entity_providers_keeps_bank_account_distinct(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;

    crate::data::payment::run_mock_payment_provider_seed(env.pool(), false).await;
    crate::data::payment::run_bank_account_seed(env.pool()).await;

    let patched = {
        let mut conn = env.conn().await;
        InvoicingEntityRowProvidersPatch {
            id: INVOICING_ENTITY_ID,
            card_provider_id: None,
            direct_debit_provider_id: None,
            bank_account_id: Some(Some(TEST_BANK_ACCOUNT_ID)),
        }
        .patch_invoicing_entity_providers(&mut conn, TENANT_ID)
        .await
        .expect("raw providers patch")
    };
    assert_eq!(patched.bank_account_id, Some(TEST_BANK_ACCOUNT_ID));
    assert_eq!(patched.direct_debit_provider_id, None);

    env.store()
        .patch_invoicing_entity_providers(
            Actor::User { id: USER_ID },
            InvoicingEntityProvidersPatch {
                id: INVOICING_ENTITY_ID,
                card_provider_id: Some(None),
                direct_debit_provider_id: None,
                bank_account_id: None,
            },
            TENANT_ID,
        )
        .await
        .expect("patch invoicing entity providers");

    let activities = entity_activities(&env).await;
    assert_eq!(activities.len(), 1, "expected one provider audit row");
    assert_eq!(
        changed_fields(&activities[0]),
        vec!["card_provider".to_string()],
        "bank account / direct debit must not show as changed"
    );
}

/// The customer-facing language: normalized on save, blank clears it, unsupported tags and
/// oversized sets are rejected, and extras drop the (post-patch) default.
#[rstest]
#[tokio::test]
async fn test_patch_invoicing_entity_customer_locales(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let patch = |customer_locale: Option<&str>, extras: Option<&[&str]>| InvoicingEntityPatch {
        id: INVOICING_ENTITY_ID,
        customer_locale: customer_locale.map(|l| Some(l.to_string())),
        customer_extra_locales: extras.map(|e| e.iter().map(|l| l.to_string()).collect()),
        ..Default::default()
    };
    let save = |p: InvoicingEntityPatch| {
        env.store()
            .patch_invoicing_entity(Actor::User { id: USER_ID }, p, TENANT_ID)
    };

    // The seeded entity is French: "fr-FR" is the default until `customer_locale` moves it.
    let saved = save(patch(
        Some(" en-GB "),
        Some(&["de-AT", "en-US", "fr-FR", "de-DE"]),
    ))
    .await
    .expect("valid locales");
    assert_eq!(saved.customer_locale.as_deref(), Some("en-US"));
    assert_eq!(saved.customer_extra_locales, vec!["de-DE", "fr-FR"]);
    assert_eq!(
        saved.invoicing_language, None,
        "the invoice language is untouched"
    );

    let audit = entity_activities(&env).await;
    let fields = changed_fields(&audit[0]);
    assert!(
        fields.contains(&"customer_locale".to_string()),
        "{fields:?}"
    );
    assert!(
        fields.contains(&"customer_extra_locales".to_string()),
        "{fields:?}"
    );

    // Absent = unchanged; blank clears back to the invoice language.
    let cleared = save(patch(Some("  "), None)).await.expect("clear");
    assert_eq!(cleared.customer_locale, None);
    assert_eq!(cleared.customer_extra_locales, vec!["de-DE", "fr-FR"]);

    let invalid = |err: error_stack::Report<meteroid_store::errors::StoreError>| {
        matches!(
            err.current_context(),
            meteroid_store::errors::StoreError::InvalidArgument(_)
        )
    };
    for (label, p) in [
        ("unsupported customer_locale", patch(Some("pl-PL"), None)),
        ("unsupported extra", patch(None, Some(&["de-DE", "pl-PL"]))),
        ("too many extras", patch(None, Some(&["de-DE"; 9]))),
    ] {
        let err = save(p).await.expect_err(label);
        assert!(invalid(err), "{label}");
    }
    let unchanged = env
        .store()
        .get_invoicing_entity(TENANT_ID, Some(INVOICING_ENTITY_ID))
        .await
        .expect("get invoicing entity");
    assert_eq!(unchanged.customer_locale, None);
    assert_eq!(unchanged.customer_extra_locales, vec!["de-DE", "fr-FR"]);
}
