//! Instance bootstrap: a second run converges on the objects the first one created rather
//! than duplicating them, and a pinned API key is installed verbatim and authenticates REST.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use common_domain::ids::{ApiTokenId, BaseId, TenantId};
use diesel_async::RunQueryDsl;
use meteroid::config::InstanceBootstrapConfig;
use meteroid_store::Store;
use meteroid_store::domain::enums::{ConnectorProviderEnum, ConnectorTypeEnum};
use meteroid_store::repositories::api_tokens::ApiTokensInterface;
use meteroid_store::repositories::connectors::ConnectorsInterface;
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterfaceAuto;
use meteroid_store::store::PgPool;
use secrecy::{ExposeSecret, SecretString};

use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;

fn bootstrap_config(output_path: &Path, api_key: Option<SecretString>) -> InstanceBootstrapConfig {
    InstanceBootstrapConfig {
        enabled: true,
        email: Some("bootstrap@meteroid.dev".to_string()),
        password: Some(SecretString::from("bootstrap-Pa55word")),
        organization: "Bootstrapped".to_string(),
        tenant: "Development".to_string(),
        country: "US".to_string(),
        currency: "USD".to_string(),
        api_key,
        output_path: Some(output_path.display().to_string()),
        test_gateway: true,
    }
}

fn output_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "meteroid-bootstrap-{}-{label}.env",
        std::process::id()
    ))
}

fn credentials(path: &Path) -> HashMap<String, String> {
    std::fs::read_to_string(path)
        .expect("the bootstrap writes a dotenv fragment to its output path")
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

/// `pv_<env>_<random>/<base62 token id>`: the shape auth decodes the token id from.
fn pinned_api_key() -> SecretString {
    SecretString::from(format!(
        "pv_dev_p1nnedBootstrapKey0123456789/{}",
        base62::encode(ApiTokenId::new().as_u128())
    ))
}

async fn count_rows(pool: &PgPool, table: &str) -> i64 {
    let mut conn = pool.get().await.unwrap();
    diesel::dsl::sql::<diesel::sql_types::BigInt>(&format!("SELECT count(*) FROM {table}"))
        .get_result(&mut conn)
        .await
        .unwrap()
}

async fn assert_one_of_each(store: &Store, tenant_id: TenantId) {
    assert_eq!(count_rows(&store.pool, "\"user\"").await, 1, "users");
    assert_eq!(count_rows(&store.pool, "organization").await, 1, "orgs");
    assert_eq!(count_rows(&store.pool, "tenant").await, 1, "tenants");
    assert_eq!(count_rows(&store.pool, "api_token").await, 1, "api tokens");

    let tokens = store.find_api_tokens_by_tenant_id(tenant_id).await.unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].name, "bootstrap");

    let connectors = store
        .list_connectors(Some(ConnectorTypeEnum::PaymentProvider), tenant_id)
        .await
        .unwrap();
    assert_eq!(connectors.len(), 1, "test gateway connectors");
    assert_eq!(connectors[0].provider(), ConnectorProviderEnum::Mock);

    let invoicing_entity = store.get_invoicing_entity(tenant_id, None).await.unwrap();
    assert_eq!(invoicing_entity.card_provider_id, Some(connectors[0].id));
    assert_eq!(
        invoicing_entity.direct_debit_provider_id,
        Some(connectors[0].id)
    );
}

#[tokio::test]
async fn test_instance_bootstrap_converges_and_installs_pinned_api_key() {
    helpers::init::logging();
    let database = meteroid_it::container::create_test_database().await;
    let setup = meteroid_it::container::start_meteroid(database, SeedLevel::NONE).await;

    let output = output_path("converges");
    let _ = std::fs::remove_file(&output);

    meteroid::bootstrap::instance::bootstrap_instance(
        &setup.store,
        &bootstrap_config(&output, None),
    )
    .await
    .expect("the first bootstrap run brings the instance up");

    let first = credentials(&output);
    let minted_key = first["METEROID_API_KEY"].clone();
    assert!(
        minted_key.starts_with("pv_dev_"),
        "the bootstrap tenant is a Development one: {minted_key}"
    );
    let tenant_id: TenantId = first["METEROID_TENANT_ID"].parse().unwrap();
    assert_one_of_each(&setup.store, tenant_id).await;

    let pinned = pinned_api_key();
    meteroid::bootstrap::instance::bootstrap_instance(
        &setup.store,
        &bootstrap_config(&output, Some(pinned.clone())),
    )
    .await
    .expect("a second run converges instead of erroring");

    let second = credentials(&output);
    for key in [
        "METEROID_ORGANIZATION_ID",
        "METEROID_ORGANIZATION_SLUG",
        "METEROID_TENANT_ID",
        "METEROID_TENANT_SLUG",
    ] {
        assert_eq!(second[key], first[key], "{key} moved on the second run");
    }
    assert_eq!(second["METEROID_API_KEY"], pinned.expose_secret());
    assert_one_of_each(&setup.store, tenant_id).await;

    let base = setup.config.rest_api_external_url.clone();
    let client = http_client_tracing::client();

    let authenticated = client
        .get(format!("{base}/api/v1/features"))
        .bearer_auth(pinned.expose_secret())
        .send()
        .await
        .unwrap();
    assert_eq!(
        authenticated.status(),
        reqwest::StatusCode::OK,
        "the pinned key must authenticate the REST API"
    );

    // The key the pinned one replaced is gone, not merely shadowed.
    let stale = client
        .get(format!("{base}/api/v1/features"))
        .bearer_auth(&minted_key)
        .send()
        .await
        .unwrap();
    assert_eq!(stale.status(), reqwest::StatusCode::UNAUTHORIZED);

    let _ = std::fs::remove_file(&output);
}

/// A malformed pinned key aborts startup with the tenant's existing key untouched.
#[tokio::test]
async fn test_instance_bootstrap_rejects_a_malformed_api_key() {
    helpers::init::logging();
    let database = meteroid_it::container::create_test_database().await;
    let setup = meteroid_it::container::start_meteroid(database, SeedLevel::NONE).await;

    let output = output_path("malformed-key");
    let _ = std::fs::remove_file(&output);

    meteroid::bootstrap::instance::bootstrap_instance(
        &setup.store,
        &bootstrap_config(&output, None),
    )
    .await
    .expect("the first bootstrap run brings the instance up");

    let first = credentials(&output);
    let minted_key = first["METEROID_API_KEY"].clone();
    let tenant_id: TenantId = first["METEROID_TENANT_ID"].parse().unwrap();
    let before = setup
        .store
        .find_api_tokens_by_tenant_id(tenant_id)
        .await
        .unwrap();

    meteroid::bootstrap::instance::bootstrap_instance(
        &setup.store,
        &bootstrap_config(&output, Some(SecretString::from("not-an-api-key"))),
    )
    .await
    .expect_err("a malformed INSTANCE_BOOTSTRAP_API_KEY aborts startup");

    let after = setup
        .store
        .find_api_tokens_by_tenant_id(tenant_id)
        .await
        .unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(
        after[0].id, before[0].id,
        "the rejected key must not have dropped the existing token"
    );

    let base = setup.config.rest_api_external_url.clone();
    let still_valid = http_client_tracing::client()
        .get(format!("{base}/api/v1/features"))
        .bearer_auth(&minted_key)
        .send()
        .await
        .unwrap();
    assert_eq!(
        still_valid.status(),
        reqwest::StatusCode::OK,
        "a rejected key must not take the tenant's working one with it"
    );

    let _ = std::fs::remove_file(&output);
}

/// A well-formed key carrying a weak secret used to pass validation and only fail at insert,
/// after the previous token had already been deleted.
#[tokio::test]
async fn test_instance_bootstrap_rejects_a_weak_pinned_api_key() {
    helpers::init::logging();
    let database = meteroid_it::container::create_test_database().await;
    let setup = meteroid_it::container::start_meteroid(database, SeedLevel::NONE).await;

    let output = output_path("weak-key");
    let _ = std::fs::remove_file(&output);

    meteroid::bootstrap::instance::bootstrap_instance(
        &setup.store,
        &bootstrap_config(&output, None),
    )
    .await
    .expect("the first bootstrap run brings the instance up");

    let minted_key = credentials(&output)["METEROID_API_KEY"].clone();
    let tenant_id: TenantId = credentials(&output)["METEROID_TENANT_ID"].parse().unwrap();

    let weak = SecretString::from(format!(
        "pv_dev_abcd/{}",
        base62::encode(ApiTokenId::new().as_u128())
    ));
    meteroid::bootstrap::instance::bootstrap_instance(
        &setup.store,
        &bootstrap_config(&output, Some(weak)),
    )
    .await
    .expect_err("a 4-character secret is not a usable api key");

    let tokens = setup
        .store
        .find_api_tokens_by_tenant_id(tenant_id)
        .await
        .unwrap();
    assert_eq!(
        tokens.len(),
        1,
        "the weak key must not have dropped the token"
    );

    let base = setup.config.rest_api_external_url.clone();
    let still_valid = http_client_tracing::client()
        .get(format!("{base}/api/v1/features"))
        .bearer_auth(&minted_key)
        .send()
        .await
        .unwrap();
    assert_eq!(still_valid.status(), reqwest::StatusCode::OK);

    let _ = std::fs::remove_file(&output);
}

/// Turning the bootstrap on against an instance someone else initialised used to abort
/// startup on every boot, because a second organization is refused.
#[tokio::test]
async fn test_instance_bootstrap_stands_down_on_a_claimed_instance() {
    helpers::init::logging();
    let database = meteroid_it::container::create_test_database().await;
    let setup = meteroid_it::container::start_meteroid(database, SeedLevel::NONE).await;

    let output = output_path("claimed");
    let _ = std::fs::remove_file(&output);

    meteroid::bootstrap::instance::bootstrap_instance(
        &setup.store,
        &bootstrap_config(&output, None),
    )
    .await
    .expect("the first bootstrap run brings the instance up");

    let mut other = bootstrap_config(&output, None);
    other.email = Some("someone-else@meteroid.dev".to_string());
    other.organization = "Another Org".to_string();

    meteroid::bootstrap::instance::bootstrap_instance(&setup.store, &other)
        .await
        .expect("a claimed instance stands the bootstrap down instead of aborting startup");

    assert_eq!(count_rows(&setup.store.pool, "organization").await, 1);
    assert_eq!(count_rows(&setup.store.pool, "tenant").await, 1);

    let _ = std::fs::remove_file(&output);
}

/// Auth ignores the prefix, so a key for another environment would work but carry a hint
/// that disagrees with it; the bootstrap refuses it instead.
#[tokio::test]
async fn test_instance_bootstrap_rejects_a_pinned_key_for_another_environment() {
    helpers::init::logging();
    let database = meteroid_it::container::create_test_database().await;
    let setup = meteroid_it::container::start_meteroid(database, SeedLevel::NONE).await;

    let output = output_path("prod-prefix");
    let _ = std::fs::remove_file(&output);

    let prod_key = SecretString::from(format!(
        "pv_prod_p1nnedBootstrapKey0123456789/{}",
        base62::encode(ApiTokenId::new().as_u128())
    ));
    let err = meteroid::bootstrap::instance::bootstrap_instance(
        &setup.store,
        &bootstrap_config(&output, Some(prod_key)),
    )
    .await
    .expect_err("a pv_prod_ key cannot be pinned on a Development tenant");

    assert!(err.to_string().contains("pv_dev_"), "{err}");
}
