use std::collections::HashSet;

use cornucopia_async::Params;
use deadpool_postgres::Pool;
use testcontainers::clients::Cli;
use time::macros::date;
use uuid::Uuid;

use meteroid::eventbus::create_eventbus_noop;
use meteroid::singletons;
use meteroid::workers::invoicing::draft_worker::draft_worker;
use meteroid_repository::invoices::ListInvoice;
use meteroid_repository::InvoiceStatusEnum;
use meteroid_store::Store;

use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::db::seed::*;

#[tokio::test]
async fn test_draft_worker() {
    helpers::init::logging();
    let docker = Cli::default();
    let (container, postgres_connection_string) = meteroid_it::container::start_postgres(&docker);

    let pool = meteroid_repository::create_pool(postgres_connection_string.as_str());

    meteroid_it::container::populate_postgres(
        pool.clone(),
        meteroid_it::container::SeedLevel::SUBSCRIPTIONS,
    )
    .await;

    let worker_run_date = date!(2023 - 11 - 04);

    let store = Store::new(
        postgres_connection_string,
        secrecy::SecretString::new("test-key".into()),
        //create_eventbus_noop().await,
        singletons::get_store().await.eventbus.clone(),
    )
    .unwrap();

    draft_worker(&store, &pool, worker_run_date.clone())
        .await
        .unwrap();

    let invoices = fetch_invoices(pool.clone()).await;

    assert_eq!(invoices.len(), 6);

    let expected_sub_ids: HashSet<Uuid> = HashSet::from_iter(vec![
        SUBSCRIPTION_SPORTIFY_ID1,
        SUBSCRIPTION_SPORTIFY_ID2,
        SUBSCRIPTION_UBER_ID1,
        SUBSCRIPTION_UBER_ID2,
        SUBSCRIPTION_COMODO_ID1,
        SUBSCRIPTION_COMODO_ID2,
    ]);

    let actual_sub_ids: HashSet<Uuid> =
        HashSet::from_iter(invoices.iter().map(|i| i.subscription_id));

    assert_eq!(expected_sub_ids, actual_sub_ids);

    for invoice in invoices.iter() {
        assert_eq!(invoice.status, InvoiceStatusEnum::DRAFT);

        if invoice.subscription_id == SUBSCRIPTION_SPORTIFY_ID1 {
            assert_eq!(invoice.customer_id, CUSTOMER_SPORTIFY_ID);
            assert_eq!(invoice.invoice_date, date!(2023 - 12 - 01));
        }

        if invoice.subscription_id == SUBSCRIPTION_SPORTIFY_ID2 {
            assert_eq!(invoice.customer_id, CUSTOMER_SPORTIFY_ID);
            assert_eq!(invoice.invoice_date, date!(2023 - 12 - 01));
        }

        if invoice.subscription_id == SUBSCRIPTION_UBER_ID1 {
            assert_eq!(invoice.customer_id, CUSTOMER_UBER_ID);
            assert_eq!(invoice.invoice_date, date!(2024 - 11 - 01));
        }

        if invoice.subscription_id == SUBSCRIPTION_UBER_ID2 {
            assert_eq!(invoice.customer_id, CUSTOMER_UBER_ID);
            assert_eq!(invoice.invoice_date, date!(2023 - 11 - 15));
        }

        if invoice.subscription_id == SUBSCRIPTION_COMODO_ID1 {
            assert_eq!(invoice.customer_id, CUSTOMER_COMODO_ID);
            assert_eq!(invoice.invoice_date, date!(2023 - 12 - 01));
        }

        if invoice.subscription_id == SUBSCRIPTION_COMODO_ID2 {
            assert_eq!(invoice.customer_id, CUSTOMER_COMODO_ID);
            assert_eq!(invoice.invoice_date, date!(2023 - 11 - 30));
        }
    }

    // second run should not create new invoices
    draft_worker(&store, &pool, worker_run_date.next_day().unwrap())
        .await
        .unwrap();

    let invoices2 = fetch_invoices(pool.clone()).await;

    assert_eq!(invoices2, invoices);

    container.stop();
}

async fn fetch_invoices(pool: Pool) -> Vec<ListInvoice> {
    let conn = meteroid::db::get_connection(&pool).await.unwrap();

    let search: Option<String> = None;

    let params = meteroid_repository::invoices::ListTenantInvoicesParams {
        tenant_id: TENANT_ID,
        limit: 100,
        offset: 0,
        status: None,
        order_by: "DATE_ASC",
        customer_id: None,
        search,
    };

    meteroid_repository::invoices::list_tenant_invoices()
        .params(&conn, &params)
        .all()
        .await
        .unwrap()
}
