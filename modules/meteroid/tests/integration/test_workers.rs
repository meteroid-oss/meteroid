use chrono::NaiveDate;
use std::collections::HashSet;
use std::sync::Arc;

use meteroid::eventbus::create_eventbus_noop;
use uuid::Uuid;

use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::db::seed::*;
use meteroid::workers::invoicing::draft_worker::draft_worker;
use meteroid_mailer::config::MailerConfig;
use meteroid_oauth::config::OauthConfig;
use meteroid_store::compute::clients::usage::MockUsageClient;
use meteroid_store::domain::enums::InvoiceStatusEnum;
use meteroid_store::domain::{InvoiceWithCustomer, OrderByRequest, PaginationRequest};
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::store::StoreConfig;
use meteroid_store::Store;
use stripe_client::client::StripeClient;

#[tokio::test]
async fn test_draft_worker() {
    helpers::init::logging();
    let (_pg_container, postgres_connection_string) =
        meteroid_it::container::start_postgres().await;

    let worker_run_date = date("2023-11-06");

    let store = Store::new(StoreConfig {
        database_url: postgres_connection_string,
        crypt_key: secrecy::SecretString::new("test-key".into()),
        jwt_secret: secrecy::SecretString::new("test-jwt-key".into()),
        multi_organization_enabled: false,
        public_url: "http://localhost:8080".to_owned(),
        eventbus: create_eventbus_noop().await,
        usage_client: Arc::new(MockUsageClient::noop()),
        svix: None,
        mailer: meteroid_mailer::service::mailer_service(MailerConfig::dummy()),
        stripe: Arc::new(StripeClient::new()),
        oauth: meteroid_oauth::service::OauthServices::new(OauthConfig::dummy()),
    })
    .unwrap();

    meteroid_it::container::populate_postgres(
        &store.pool,
        meteroid_it::container::SeedLevel::SUBSCRIPTIONS,
    )
    .await;

    draft_worker(&store, worker_run_date).await.unwrap();

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
        HashSet::from_iter(invoices.iter().map(|i| i.invoice.subscription_id.unwrap()));

    assert_eq!(expected_sub_ids, actual_sub_ids);

    for invoice in invoices.iter().map(|x| &x.invoice) {
        assert_eq!(invoice.status, InvoiceStatusEnum::Draft);

        let subscription_id = invoice.subscription_id.unwrap();

        if subscription_id == SUBSCRIPTION_SPORTIFY_ID1
            || subscription_id == SUBSCRIPTION_SPORTIFY_ID2
        {
            assert_eq!(invoice.customer_id, CUSTOMER_SPORTIFY_ID.into());
            assert_eq!(invoice.invoice_date, date("2023-12-01"));
        } else if subscription_id == SUBSCRIPTION_UBER_ID1 {
            assert_eq!(invoice.customer_id, CUSTOMER_UBER_ID.into());
            assert_eq!(invoice.invoice_date, date("2024-11-01"));
        } else if subscription_id == SUBSCRIPTION_UBER_ID2 {
            assert_eq!(invoice.customer_id, CUSTOMER_UBER_ID.into());
            assert_eq!(invoice.invoice_date, date("2023-11-15"));
        } else if subscription_id == SUBSCRIPTION_COMODO_ID1 {
            assert_eq!(invoice.customer_id, CUSTOMER_COMODO_ID.into());
            assert_eq!(invoice.invoice_date, date("2023-12-01"));
        } else if subscription_id == SUBSCRIPTION_COMODO_ID2 {
            assert_eq!(invoice.customer_id, CUSTOMER_COMODO_ID.into());
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
}

fn date(date_str: &str) -> NaiveDate {
    NaiveDate::parse_from_str(date_str, "%Y-%m-%d").expect("Invalid date format")
}

async fn list_invoices(store: &Store) -> Vec<InvoiceWithCustomer> {
    store
        .list_invoices(
            TENANT_ID.into(),
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
