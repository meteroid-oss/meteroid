use chrono::NaiveDate;
use std::collections::HashSet;

use meteroid::eventbus::create_eventbus_noop;
use testcontainers::clients::Cli;
use uuid::Uuid;

use meteroid::workers::invoicing::draft_worker::draft_worker;
use meteroid_store::domain::enums::InvoiceStatusEnum;
use meteroid_store::domain::{InvoiceWithCustomer, OrderByRequest, PaginationRequest};
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::Store;

use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::db::seed::*;

#[tokio::test]
async fn test_draft_worker() {
    helpers::init::logging();
    let docker = Cli::default();
    let (container, postgres_connection_string) = meteroid_it::container::start_postgres(&docker);

    let pool = common_repository::create_pool(postgres_connection_string.as_str());

    meteroid_it::container::populate_postgres(
        pool.clone(),
        meteroid_it::container::SeedLevel::SUBSCRIPTIONS,
    )
    .await;

    let worker_run_date = date("2023-11-06");

    let store = Store::new(
        postgres_connection_string,
        secrecy::SecretString::new("test-key".into()),
        secrecy::SecretString::new("test-jwt-key".into()),
        create_eventbus_noop().await,
    )
    .unwrap();

    draft_worker(&store, worker_run_date.clone()).await.unwrap();

    let invoices = list_invoices(&store).await;

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
        HashSet::from_iter(invoices.iter().map(|i| i.invoice.subscription_id));

    assert_eq!(expected_sub_ids, actual_sub_ids);

    for invoice in invoices.iter().map(|x| &x.invoice) {
        assert_eq!(invoice.status, InvoiceStatusEnum::Draft);

        if invoice.subscription_id == SUBSCRIPTION_SPORTIFY_ID1 {
            assert_eq!(invoice.customer_id, CUSTOMER_SPORTIFY_ID);
            assert_eq!(invoice.invoice_date, date("2023-12-01"));
        } else if invoice.subscription_id == SUBSCRIPTION_SPORTIFY_ID2 {
            assert_eq!(invoice.customer_id, CUSTOMER_SPORTIFY_ID);
            assert_eq!(invoice.invoice_date, date("2023-12-01"));
        } else if invoice.subscription_id == SUBSCRIPTION_UBER_ID1 {
            assert_eq!(invoice.customer_id, CUSTOMER_UBER_ID);
            assert_eq!(invoice.invoice_date, date("2024-11-01"));
        } else if invoice.subscription_id == SUBSCRIPTION_UBER_ID2 {
            assert_eq!(invoice.customer_id, CUSTOMER_UBER_ID);
            assert_eq!(invoice.invoice_date, date("2023-11-15"));
        } else if invoice.subscription_id == SUBSCRIPTION_COMODO_ID1 {
            assert_eq!(invoice.customer_id, CUSTOMER_COMODO_ID);
            assert_eq!(invoice.invoice_date, date("2023-12-01"));
        } else if invoice.subscription_id == SUBSCRIPTION_COMODO_ID2 {
            assert_eq!(invoice.customer_id, CUSTOMER_COMODO_ID);
            assert_eq!(invoice.invoice_date, date("2023-11-30"));
        } else {
            panic!("Unexpected invoice: {:?}", invoice);
        }
    }

    // second run should not create new invoices
    draft_worker(
        &store,
        worker_run_date
            .checked_add_days(chrono::Days::new(1))
            .unwrap(),
    )
    .await
    .unwrap();

    let invoices2 = list_invoices(&store).await;

    assert_eq!(invoices2, invoices);

    container.stop();
}

fn date(date_str: &str) -> NaiveDate {
    NaiveDate::parse_from_str(date_str, "%Y-%m-%d").expect("Invalid date format")
}

async fn list_invoices(store: &Store) -> Vec<InvoiceWithCustomer> {
    store
        .list_invoices(
            TENANT_ID,
            None,
            None,
            None,
            OrderByRequest::DateAsc,
            PaginationRequest {
                per_page: Some(100),
                page: 0,
            },
        )
        .await
        .unwrap()
        .items
}
