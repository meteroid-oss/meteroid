//! Startup re-encryption of the secrets sealed under the legacy key-derived nonce.

use common_domain::ids::WebhookEndpointId;
use diesel::sql_types;
use diesel_async::RunQueryDsl;
use rstest::rstest;
use secrecy::ExposeSecret;

use crate::data::ids::TENANT_ID;
use crate::harness::{TestEnv, test_env_minimal};
use meteroid::services::webhooks_out::tenant_max_in_flight;
use meteroid_store::domain::webhooks_out::WebhookEndpointNew;
use meteroid_store::repositories::WebhookOutInterface;

const LEGACY_PLAINTEXT: &str = "whsec_legacy-secret";
/// `LEGACY_PLAINTEXT` sealed by the previous `crypt::encrypt` with the test crypt key.
const LEGACY_SEALED: &str =
    "efc2fc0d6b96e6336f1005dcbc6a84f5deed1ea55c1f08f4462228ec17293943b392b0";

async fn endpoint_with_stored_secret(env: &TestEnv, url: &str, stored: &str) -> WebhookEndpointId {
    let (endpoint, _) = env
        .store()
        .create_webhook_endpoint(
            TENANT_ID,
            WebhookEndpointNew {
                url: url.to_string(),
                description: None,
                event_types: vec![],
                max_in_flight: tenant_max_in_flight(env.store(), TENANT_ID)
                    .await
                    .expect("resolve max in flight"),
                headers: vec![],
                rate_limit_per_sec: None,
            },
        )
        .await
        .expect("create webhook endpoint");

    diesel::sql_query("UPDATE webhook_out_endpoint SET secret = $1 WHERE id = $2")
        .bind::<sql_types::Text, _>(stored)
        .bind::<sql_types::Uuid, _>(endpoint.id)
        .execute(&mut env.conn().await)
        .await
        .expect("store the secret");

    endpoint.id
}

async fn stored_secret(env: &TestEnv, id: WebhookEndpointId) -> String {
    #[derive(diesel::QueryableByName)]
    struct Row {
        #[diesel(sql_type = sql_types::Text)]
        secret: String,
    }

    diesel::sql_query("SELECT secret FROM webhook_out_endpoint WHERE id = $1")
        .bind::<sql_types::Uuid, _>(id)
        .get_result::<Row>(&mut env.conn().await)
        .await
        .expect("read the secret")
        .secret
}

#[rstest]
#[tokio::test]
async fn legacy_secrets_are_reencrypted_and_still_decrypt(#[future] test_env_minimal: TestEnv) {
    let env = test_env_minimal.await;
    let legacy =
        endpoint_with_stored_secret(&env, "https://legacy.example/hook", LEGACY_SEALED).await;
    let corrupt =
        endpoint_with_stored_secret(&env, "https://corrupt.example/hook", "deadbeef").await;

    let report = env.store().reencrypt_legacy_secrets().await.unwrap();

    assert!(report.reencrypted >= 1, "{report:?}");
    assert!(report.undecryptable >= 1, "{report:?}");
    let reencrypted = stored_secret(&env, legacy).await;
    assert!(reencrypted.starts_with("v2:"), "{reencrypted}");
    let secret = env
        .store()
        .get_webhook_endpoint_secret(TENANT_ID, legacy)
        .await
        .unwrap();
    assert_eq!(secret.expose_secret(), LEGACY_PLAINTEXT);
    assert_eq!(
        stored_secret(&env, corrupt).await,
        "deadbeef",
        "a value that does not decrypt is left as is"
    );

    let rerun = env.store().reencrypt_legacy_secrets().await.unwrap();

    assert_eq!(rerun.reencrypted, 0, "{rerun:?}");
    assert_eq!(stored_secret(&env, legacy).await, reencrypted);
}
