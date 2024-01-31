use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::db::seed::*;
use cornucopia_async::Params;
use deadpool_postgres::Pool;
use meteroid::api::services::utils::uuid_gen;
use testcontainers::clients::Cli;
use time::macros::datetime;
use time::PrimitiveDateTime;
use uuid::{uuid, Uuid};

const SLOT_SUBSCRIPTION_ID: Uuid = SUBSCRIPTION_UBER_ID1;
const SLOT_PRICE_COMPONENT_ID: Uuid = uuid!("018c344c-9ec9-7608-b115-1537b6985e73");

#[tokio::test]
async fn test_slot_transaction_active_slots() {
    helpers::init::logging();
    let docker = Cli::default();
    let (container, postgres_connection_string) = meteroid_it::container::start_postgres(&docker);

    let pool = meteroid_repository::create_pool(postgres_connection_string.as_str());

    meteroid_it::container::populate_postgres(
        pool.clone(),
        meteroid_it::container::SeedLevel::SUBSCRIPTIONS,
    )
    .await;

    // no active slots before first transaction
    let active = get_active_slots(pool.clone(), datetime!(2024-01-01 00:00:00)).await;
    assert_eq!(active, 0);

    create_slot_transaction(
        pool.clone(),
        15,
        datetime!(2024-01-01 00:00:00),
        datetime!(2024-01-01 00:00:00),
    )
    .await;

    // 15 active slots after first transaction as it is upgrade
    let active = get_active_slots(pool.clone(), datetime!(2024-01-01 00:00:00)).await;
    assert_eq!(active, 15);

    create_slot_transaction(
        pool.clone(),
        1,
        datetime!(2024-01-01 02:00:00),
        datetime!(2024-01-01 02:00:00),
    )
    .await;

    // 16 active slots after second transaction as it is upgrade
    let active = get_active_slots(pool.clone(), datetime!(2024-01-01 02:00:00)).await;
    assert_eq!(active, 16);

    create_slot_transaction(
        pool.clone(),
        -3,
        datetime!(2024-01-01 04:00:00),
        datetime!(2024-02-01 00:00:00),
    )
    .await;

    // still 16 active slots after third transaction as it is downgrade so it is effective after billing period ends
    let active = get_active_slots(pool.clone(), datetime!(2024-01-01 04:00:00)).await;
    assert_eq!(active, 16);

    create_slot_transaction(
        pool.clone(),
        4,
        datetime!(2024-01-01 06:00:00),
        datetime!(2024-01-01 06:00:00),
    )
    .await;

    // 20 active slots after fourth transaction as it is upgrade
    let active = get_active_slots(pool.clone(), datetime!(2024-01-01 06:00:00)).await;
    assert_eq!(active, 20);

    // 17 active slots after fifth transaction as it is after billing period end
    let active = get_active_slots(pool.clone(), datetime!(2024-02-01 02:00:00)).await;
    assert_eq!(active, 17);

    container.stop();
}

async fn create_slot_transaction(
    pool: Pool,
    delta: i32,
    transaction_at: PrimitiveDateTime,
    effective_at: PrimitiveDateTime,
) -> Uuid {
    let conn = meteroid::db::get_connection(&pool).await.unwrap();

    let prev_active_slots = get_active_slots(pool.clone(), transaction_at).await;

    meteroid_repository::slot_transactions::create_slot_transaction()
        .params(
            &conn,
            &meteroid_repository::slot_transactions::CreateSlotTransactionParams {
                id: uuid_gen::v7(),
                price_component_id: SLOT_PRICE_COMPONENT_ID,
                subscription_id: SLOT_SUBSCRIPTION_ID,
                delta,
                prev_active_slots,
                effective_at,
                transaction_at,
            },
        )
        .one()
        .await
        .unwrap()
}

async fn get_active_slots(pool: Pool, timestamp: PrimitiveDateTime) -> i32 {
    meteroid_it::db::slot_transaction::get_active_slots(
        &pool,
        SLOT_SUBSCRIPTION_ID,
        SLOT_PRICE_COMPONENT_ID,
        timestamp,
    )
    .await
}
