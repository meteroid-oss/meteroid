use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::db::seed::*;
use chrono::NaiveDateTime;
use meteroid::eventbus::create_eventbus_memory;
use meteroid_store::compute::clients::usage::MockUsageClient;
use meteroid_store::repositories::subscriptions::SubscriptionSlotsInterface;
use meteroid_store::Store;
use secrecy::SecretString;
use std::str::FromStr;
use std::sync::Arc;
use uuid::{uuid, Uuid};

const SLOT_SUBSCRIPTION_ID: Uuid = SUBSCRIPTION_UBER_ID1;
const SLOT_PRICE_COMPONENT_ID: Uuid = uuid!("018c344c-9ec9-7608-b115-1537b6985e73");

#[tokio::test]
#[ignore] // add_slot_transaction is not implemented for Store yet
async fn test_slot_transaction_active_slots() {
    helpers::init::logging();
    let (_, postgres_connection_string) = meteroid_it::container::start_postgres().await;

    meteroid_it::container::populate_postgres(
        &postgres_connection_string,
        meteroid_it::container::SeedLevel::SUBSCRIPTIONS,
    );

    let store = Store::new(
        postgres_connection_string.clone(),
        SecretString::new("00000000000000000000000000000000".into()),
        SecretString::new("secret".into()),
        false,
        create_eventbus_memory(),
        Arc::new(MockUsageClient::noop()),
    )
    .expect("Could not create store");

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
            TENANT_ID,
            SLOT_SUBSCRIPTION_ID,
            SLOT_PRICE_COMPONENT_ID,
            delta,
        )
        .await
        .unwrap();
}

async fn get_active_slots(store: &Store, timestamp: NaiveDateTime) -> u32 {
    store
        .get_current_slots_value(
            TENANT_ID,
            SLOT_SUBSCRIPTION_ID,
            SLOT_PRICE_COMPONENT_ID,
            Some(timestamp),
        )
        .await
        .unwrap()
}

fn datetime(str: &str) -> NaiveDateTime {
    NaiveDateTime::from_str(str).unwrap()
}
