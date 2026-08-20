use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use diesel_async::RunQueryDsl;
use diesel_models::connectors::ConnectorRowNew;
use diesel_models::enums::{ConnectorProviderEnum, ConnectorTypeEnum};
use diesel_models::oauth_verifiers::OauthVerifierRow;
use meteroid_store::Store;
use meteroid_store::crypt::{ENVELOPE_V1_PREFIX, decrypt, is_legacy_envelope};
use meteroid_store::crypt_migration::migrate_legacy_secrets;
use secrecy::{ExposeSecret, SecretString};

use crate::data::ids::TENANT_ID;
use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;

/// Reproduces the pre-migration scheme: one nonce derived from the key, shared by every record.
fn legacy_encrypt(key: &SecretString, value: &str) -> String {
    let cipher = ChaCha20Poly1305::new_from_slice(key.expose_secret().as_bytes()).unwrap();
    let nonce = Nonce::from_slice(&key.expose_secret().as_bytes()[0..12]);

    hex::encode(cipher.encrypt(nonce, value.as_bytes()).unwrap())
}

#[tokio::test]
async fn test_legacy_secret_migration() {
    helpers::init::logging();
    let postgres_connection_string = meteroid_it::container::create_test_database().await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;

    let store = setup.store.clone();
    let key = store.settings.crypt_key.clone();

    let connector_secret =
        r#"{"Stripe":{"api_secret_key":"sk_test_legacy","webhook_secret":"whsec_legacy"}}"#;
    let pkce_verifier = "legacy-pkce-verifier-value";

    let connector_id = insert_connector(&store, &legacy_encrypt(&key, connector_secret)).await;
    let verifier_id = insert_verifier(&store, &legacy_encrypt(&key, pkce_verifier)).await;

    // The rows really are in the vulnerable format before the migration runs.
    assert!(is_legacy_envelope(
        &read_connector(&store, connector_id).await
    ));
    assert!(is_legacy_envelope(
        &read_verifier(&store, verifier_id).await
    ));

    let migrated = migrate_legacy_secrets(&store).await.unwrap();
    assert_eq!(migrated.connectors, 1);
    assert_eq!(migrated.oauth_verifiers, 1);

    let migrated_connector = read_connector(&store, connector_id).await;
    let migrated_verifier = read_verifier(&store, verifier_id).await;

    assert!(migrated_connector.starts_with(ENVELOPE_V1_PREFIX));
    assert!(migrated_verifier.starts_with(ENVELOPE_V1_PREFIX));
    assert_eq!(
        decrypt(&key, &migrated_connector).unwrap().expose_secret(),
        connector_secret
    );
    assert_eq!(
        decrypt(&key, &migrated_verifier).unwrap().expose_secret(),
        pkce_verifier
    );

    // A second run finds nothing left to do.
    let rerun = migrate_legacy_secrets(&store).await.unwrap();
    assert!(rerun.is_empty());
    assert_eq!(
        read_connector(&store, connector_id).await,
        migrated_connector,
        "idempotent rerun rewrote an already-migrated value"
    );

    // A value that will not authenticate must abort the whole run, leaving every row untouched.
    let poisoned_id = insert_connector(&store, "deadbeef").await;
    let salvageable_id = insert_connector(&store, &legacy_encrypt(&key, "another-secret")).await;

    assert!(migrate_legacy_secrets(&store).await.is_err());

    assert_eq!(read_connector(&store, poisoned_id).await, "deadbeef");
    assert!(
        is_legacy_envelope(&read_connector(&store, salvageable_id).await),
        "a failed migration committed a partial rewrite"
    );
}

async fn insert_connector(store: &Store, sensitive: &str) -> common_domain::ids::ConnectorId {
    use common_domain::ids::BaseId;

    let mut conn = store.get_conn().await.unwrap();
    let id = common_domain::ids::ConnectorId::new();

    ConnectorRowNew {
        id,
        tenant_id: TENANT_ID,
        alias: format!("legacy-{id}"),
        connector_type: ConnectorTypeEnum::PaymentProvider,
        provider: ConnectorProviderEnum::Stripe,
        data: None,
        sensitive: Some(sensitive.to_string()),
    }
    .insert(&mut conn)
    .await
    .unwrap();

    id
}

async fn insert_verifier(store: &Store, pkce_verifier: &str) -> uuid::Uuid {
    let mut conn = store.get_conn().await.unwrap();
    let id = uuid::Uuid::now_v7();

    OauthVerifierRow {
        id,
        csrf_token: format!("csrf-{id}"),
        pkce_verifier: pkce_verifier.to_string(),
        created_at: chrono::Utc::now().naive_utc(),
        data: Some(serde_json::json!({"SignIn": {"is_signup": false, "invite_key": null}})),
    }
    .insert(&mut conn)
    .await
    .unwrap();

    id
}

async fn read_connector(store: &Store, id: common_domain::ids::ConnectorId) -> String {
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_models::schema::connector::dsl as c_dsl;

    let mut conn = store.get_conn().await.unwrap();

    c_dsl::connector
        .filter(c_dsl::id.eq(id))
        .select(c_dsl::sensitive)
        .first::<Option<String>>(&mut conn)
        .await
        .unwrap()
        .unwrap()
}

async fn read_verifier(store: &Store, id: uuid::Uuid) -> String {
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_models::schema::oauth_verifier::dsl as ov_dsl;

    let mut conn = store.get_conn().await.unwrap();

    ov_dsl::oauth_verifier
        .filter(ov_dsl::id.eq(id))
        .select(ov_dsl::pkce_verifier)
        .first::<String>(&mut conn)
        .await
        .unwrap()
}
