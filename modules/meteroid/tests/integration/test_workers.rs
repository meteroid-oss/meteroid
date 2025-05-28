use chrono::NaiveDate;
use std::collections::HashSet;
use std::sync::Arc;

use crate::data::ids::{
    CUST_COMODO_ID, CUST_SPOTIFY_ID, CUST_UBER_ID, SUB_COMODO_LEETCODE_ID, SUB_COMODO_SUPABASE_ID,
    SUB_SPOTIFY_NOTION_ID, SUB_SPOTIFY_SUPABASE_ID, SUB_UBER_LEETCODE_ID, SUB_UBER_NOTION_ID,
    TENANT_ID,
};
use crate::helpers;
use crate::meteroid_it;
use common_domain::ids::SubscriptionId;
use meteroid::eventbus::create_eventbus_noop;
use meteroid_mailer::config::MailerConfig;
use meteroid_oauth::config::OauthConfig;
use meteroid_store::Store;
use meteroid_store::domain::enums::InvoiceStatusEnum;
use meteroid_store::domain::{InvoiceWithCustomer, OrderByRequest, PaginationRequest};
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::store::StoreConfig;
use stripe_client::client::StripeClient;

#[tokio::test]
#[ignore] // TODO disabling temporary while rewriting the workers is in progress
async fn test_draft_worker() {
    helpers::init::logging();
    let (_pg_container, postgres_connection_string) =
        meteroid_it::container::start_postgres().await;

    let _worker_run_date = date("2023-11-06");

    let store = Store::new(StoreConfig {
        database_url: postgres_connection_string,
        crypt_key: secrecy::SecretString::new("test-key".into()),
        jwt_secret: secrecy::SecretString::new("test-jwt-key".into()),
        multi_organization_enabled: false,
        skip_email_validation: true,
        public_url: "http://localhost:8080".to_owned(),
        eventbus: create_eventbus_noop().await,
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

    // draft_worker(&store, worker_run_date).await.unwrap();

    let invoices = list_invoices(&store).await;

    assert_eq!(invoices.len(), 6);

    let expected_sub_ids: HashSet<SubscriptionId> = HashSet::from_iter(vec![
        SUB_SPOTIFY_NOTION_ID,
        SUB_SPOTIFY_SUPABASE_ID,
        SUB_UBER_NOTION_ID,
        SUB_UBER_LEETCODE_ID,
        SUB_COMODO_SUPABASE_ID,
        SUB_COMODO_LEETCODE_ID,
    ]);

    let actual_sub_ids: HashSet<SubscriptionId> =
        HashSet::from_iter(invoices.iter().map(|i| i.invoice.subscription_id.unwrap()));

    assert_eq!(expected_sub_ids, actual_sub_ids);

    for invoice in invoices.iter().map(|x| &x.invoice) {
        assert_eq!(invoice.status, InvoiceStatusEnum::Draft);

        let subscription_id = invoice.subscription_id.unwrap();

        if subscription_id == SUB_SPOTIFY_NOTION_ID || subscription_id == SUB_SPOTIFY_SUPABASE_ID {
            assert_eq!(invoice.customer_id, CUST_SPOTIFY_ID);
            assert_eq!(invoice.invoice_date, date("2023-12-01"));
        } else if subscription_id == SUB_UBER_NOTION_ID {
            assert_eq!(invoice.customer_id, CUST_UBER_ID);
            assert_eq!(invoice.invoice_date, date("2024-11-01"));
        } else if subscription_id == SUB_UBER_LEETCODE_ID {
            assert_eq!(invoice.customer_id, CUST_UBER_ID);
            assert_eq!(invoice.invoice_date, date("2023-11-15"));
        } else if subscription_id == SUB_COMODO_SUPABASE_ID {
            assert_eq!(invoice.customer_id, CUST_COMODO_ID);
            assert_eq!(invoice.invoice_date, date("2023-12-01"));
        } else if subscription_id == SUB_COMODO_LEETCODE_ID {
            assert_eq!(invoice.customer_id, CUST_COMODO_ID);
            assert_eq!(invoice.invoice_date, date("2023-11-30"));
        } else {
            panic!("Unexpected invoice: {:?}", invoice);
        }
    }

    // second run should not create new invoices
    // draft_worker(
    //     &store,
    //     worker_run_date
    //         .checked_add_days(chrono::Days::new(1))
    //         .unwrap(),
    // )
    // .await
    // .unwrap();

    let invoices2 = list_invoices(&store).await;

    assert_eq!(invoices2, invoices);
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
