use crate::data::ids::{SUB_UBER_LEETCODE_ID, TENANT_ID};
use crate::helpers;
use crate::meteroid_it;
use chrono::NaiveDateTime;
use common_domain::ids::SubscriptionId;
use meteroid::eventbus::create_eventbus_memory;
use meteroid_mailer::config::MailerConfig;
use meteroid_oauth::config::OauthConfig;
use meteroid_store::Store;
use meteroid_store::repositories::subscriptions::SubscriptionSlotsInterface;
use meteroid_store::store::StoreConfig;
use secrecy::SecretString;
use std::str::FromStr;
use std::sync::Arc;
use stripe_client::client::StripeClient;
use uuid::{Uuid, uuid};

const SLOT_SUBSCRIPTION_ID: SubscriptionId = SUB_UBER_LEETCODE_ID;
const SLOT_PRICE_COMPONENT_ID: Uuid = uuid!("018c344c-9ec9-7608-b115-1537b6985e73");

#[tokio::test]
#[ignore] // add_slot_transaction is not implemented for Store yet
async fn test_slot_transaction_active_slots() {
    helpers::init::logging();
    let (_, postgres_connection_string) = meteroid_it::container::start_postgres().await;

    let store = Store::new(StoreConfig {
        database_url: postgres_connection_string.clone(),
        crypt_key: SecretString::new("00000000000000000000000000000000".into()),
        jwt_secret: SecretString::new("secret".into()),
        skip_email_validation: true,
        multi_organization_enabled: false,
        public_url: "http://localhost:8080".to_owned(),
        eventbus: create_eventbus_memory(),
        svix: None,
        mailer: meteroid_mailer::service::mailer_service(MailerConfig::dummy()),
        stripe: Arc::new(StripeClient::new()),
        oauth: meteroid_oauth::service::OauthServices::new(OauthConfig::dummy()),
    })
    .expect("Could not create store");

    meteroid_it::container::populate_postgres(
        &store.pool,
        meteroid_it::container::SeedLevel::SUBSCRIPTIONS,
    )
    .await;

    // no active slots before first transaction
    let active = get_active_slots(&store, datetime("2024-01-01T00:00:00")).await;
    assert_eq!(active, 0);

    create_slot_transaction(
        &store,
        15,
        datetime("2024-01-01T00:00:00"),
        datetime("2024-01-01T00:00:00"),
    )
    .await;

    // 15 active slots after first transaction as it is upgrade
    let active = get_active_slots(&store, datetime("2024-01-01T00:00:00")).await;
    assert_eq!(active, 15);

    create_slot_transaction(
        &store,
        1,
        datetime("2024-01-01T02:00:00"),
        datetime("2024-01-01T02:00:00"),
    )
    .await;

    // 16 active slots after second transaction as it is upgrade
    let active = get_active_slots(&store, datetime("2024-01-01T02:00:00")).await;
    assert_eq!(active, 16);

    create_slot_transaction(
        &store,
        -3,
        datetime("2024-01-01T04:00:00"),
        datetime("2024-02-01T00:00:00"),
    )
    .await;

    // still 16 active slots after third transaction as it is downgrade so it is effective after billing period ends
    let active = get_active_slots(&store, datetime("2024-01-01T04:00:00")).await;
    assert_eq!(active, 16);

    create_slot_transaction(
        &store,
        4,
        datetime("2024-01-01T06:00:00"),
        datetime("2024-01-01T06:00:00"),
    )
    .await;

    // 20 active slots after fourth transaction as it is upgrade
    let active = get_active_slots(&store, datetime("2024-01-01T06:00:00")).await;
    assert_eq!(active, 20);

    // 17 active slots after fifth transaction as it is after billing period end
    let active = get_active_slots(&store, datetime("2024-02-01T02:00:00")).await;
    assert_eq!(active, 17);
}

async fn create_slot_transaction(
    store: &Store,
    delta: i32,
    _transaction_at: NaiveDateTime,
    _effective_at: NaiveDateTime,
) {
    // store.add_slot_transaction is not implemented yet
    store
        .add_slot_transaction(
            TENANT_ID.into(),
            SLOT_SUBSCRIPTION_ID.into(),
            SLOT_PRICE_COMPONENT_ID.into(),
            delta,
        )
        .await
        .unwrap();
}

async fn get_active_slots(_store: &Store, _timestamp: NaiveDateTime) -> u32 {
    // store
    //     .get_current_slots_value(
    //         TENANT_ID.into(),
    //         SLOT_SUBSCRIPTION_ID.into(),
    //         SLOT_PRICE_COMPONENT_ID.into(),
    //         Some(timestamp),
    //     )
    //     .await
    //     .unwrap()
    todo!()
}

fn datetime(str: &str) -> NaiveDateTime {
    NaiveDateTime::from_str(str).unwrap()
}
