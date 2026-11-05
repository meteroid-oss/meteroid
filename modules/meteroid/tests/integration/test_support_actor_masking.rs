use std::collections::HashSet;
use std::sync::Arc;

use common_domain::ids::{OrganizationId, UserId};
use diesel_async::SimpleAsyncConnection;
use meteroid_store::Store;
use meteroid_store::repositories::users::UserInterface;
use meteroid_store::store::{PgConfig, StoreConfig, diesel_make_pg_pool};
use uuid::Uuid;

use crate::meteroid_it;

const ADMIN_ORG: &str = "30000000-0000-0000-0000-00000000000a";
const CUSTOMER_ORG: &str = "30000000-0000-0000-0000-00000000000b";
const STAFF: &str = "30000000-0000-0000-0000-0000000000a1";
const STAFF_AND_MEMBER: &str = "30000000-0000-0000-0000-0000000000a2";
const MEMBER: &str = "30000000-0000-0000-0000-0000000000b1";

const FIXTURE: &str = r#"
INSERT INTO organization(id,trade_name,slug,default_country) VALUES
  ('30000000-0000-0000-0000-00000000000a','Platform','platform','FR'),
  ('30000000-0000-0000-0000-00000000000b','Customer','customer','FR');
INSERT INTO "user"(id,email) VALUES
  ('30000000-0000-0000-0000-0000000000a1','staff@platform.test'),
  ('30000000-0000-0000-0000-0000000000a2','both@platform.test'),
  ('30000000-0000-0000-0000-0000000000b1','member@customer.test');
INSERT INTO organization_member(user_id,organization_id,role) VALUES
  ('30000000-0000-0000-0000-0000000000a1','30000000-0000-0000-0000-00000000000a','ADMIN'),
  ('30000000-0000-0000-0000-0000000000a2','30000000-0000-0000-0000-00000000000a','ADMIN'),
  ('30000000-0000-0000-0000-0000000000a2','30000000-0000-0000-0000-00000000000b','MEMBER'),
  ('30000000-0000-0000-0000-0000000000b1','30000000-0000-0000-0000-00000000000b','ADMIN');
"#;

fn org(id: &str) -> OrganizationId {
    OrganizationId::from(Uuid::parse_str(id).unwrap())
}

fn user(id: &str) -> UserId {
    UserId::from(Uuid::parse_str(id).unwrap())
}

fn build_store(conn_str: &str, admin_organization_id: Option<OrganizationId>) -> Store {
    Store::new(StoreConfig {
        pg: PgConfig::new(conn_str.to_string()),
        crypt_key: "00000000000000000000000000000000".to_string().into(),
        jwt_secret: "test-jwt-secret".to_string().into(),
        multi_organization_enabled: true,
        mailer_enabled: false,
        public_url: "http://127.0.0.1".to_string(),
        rest_api_external_url: "http://127.0.0.1".to_string(),
        webhook_external_url: None,
        eventbus: meteroid::eventbus::create_eventbus_noop(),
        mailer: meteroid_mailer::service::mailer_service(
            meteroid_mailer::config::MailerConfig::dummy(),
        ),
        oauth: meteroid_oauth::service::OauthServices::new(
            meteroid_oauth::config::OauthConfig::dummy(),
            egress_http::EgressClient::new(egress_http::EgressClientConfig::default())
                .expect("egress"),
        ),
        domains_whitelist: vec![],
        organization_hooks: meteroid_store::hooks::no_organization_hooks(),
        instance: meteroid_store::hooks::core_instance(),
        hooks: Default::default(),
        admin_organization_id,
        usage_client: Arc::new(meteroid_store::clients::usage::MockUsageClient::noop()),
        invite_ttl_days: 7,
        mfa_issuer: "Meteroid".to_string(),
        portal_legacy_tokens_until: None,
    })
    .expect("store")
}

#[tokio::test]
async fn test_support_staff_outside_org() {
    let db = meteroid_it::container::create_test_database().await;
    let pool = diesel_make_pg_pool(PgConfig::new(db.url.clone())).expect("pool");
    let mut conn = pool.get().await.expect("db connection");
    conn.batch_execute(FIXTURE).await.expect("seed fixture");

    let actors = [user(STAFF), user(STAFF_AND_MEMBER), user(MEMBER)];
    let store = build_store(&db.url, Some(org(ADMIN_ORG)));

    let masked = store
        .support_staff_outside_org(org(CUSTOMER_ORG), &actors)
        .await
        .expect("customer org");
    assert_eq!(masked, HashSet::from([user(STAFF)]));

    let masked = store
        .support_staff_outside_org(org(ADMIN_ORG), &actors)
        .await
        .expect("admin org");
    assert!(masked.is_empty(), "staff stay named in their own org");

    let masked = build_store(&db.url, None)
        .support_staff_outside_org(org(CUSTOMER_ORG), &actors)
        .await
        .expect("no admin org");
    assert!(masked.is_empty());
}
