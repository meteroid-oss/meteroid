use meteroid_grpc::meteroid::api;

use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use common_domain::ids::{BaseId, ConnectorId, CustomerId, TenantId};
use meteroid_store::domain::ConnectorProviderEnum;
use meteroid_store::repositories::CustomersInterface;
use tonic::Code;

#[tokio::test]
async fn test_customers_basic() {
    // Generic setup
    helpers::init::logging();
    let (_postgres_container, postgres_connection_string) =
        meteroid_it::container::start_postgres().await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;

    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;

    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "TESTORG",
        "testslug",
    );

    let customer_name = "friends and co".to_owned();
    let customer_alias = "fffrrriiieeennndddsss".to_owned();
    let customer_email = "fake@fake.com".to_owned();

    // create customer
    let created = clients
        .customers
        .clone()
        .create_customer(api::customers::v1::CreateCustomerRequest {
            data: Some(api::customers::v1::CustomerNew {
                name: customer_name.to_string(),
                alias: Some(customer_alias.to_string()),
                billing_email: Some(customer_email.to_string()),
                invoicing_emails: Vec::new(),
                phone: None,
                currency: "EUR".to_string(),
                billing_address: None,
                shipping_address: None,
                invoicing_entity_id: None,
            }),
        })
        .await
        .unwrap()
        .into_inner()
        .customer
        .unwrap();

    setup
        .store
        .patch_customer_conn_meta(
            CustomerId::from_proto(created.id.as_str()).unwrap(),
            ConnectorId::new(),
            ConnectorProviderEnum::Hubspot,
            "idk",
        )
        .await
        .unwrap();

    let patched_conn_meta = setup
        .store
        .find_customer_by_id(
            CustomerId::from_proto(created.id.as_str()).unwrap(),
            TenantId::from_proto("018c2c82-3df1-7e84-9e05-6e141d0e751a").unwrap(),
        )
        .await
        .unwrap();

    assert_ne!(patched_conn_meta.conn_meta, None);

    let created_manual = clients
        .customers
        .clone()
        .create_customer(api::customers::v1::CreateCustomerRequest {
            data: Some(api::customers::v1::CustomerNew {
                name: "created_manual".to_string(),
                alias: Some("created_manual".to_string()),
                billing_email: Some("created_manual@meteroid.com".to_string()),
                invoicing_emails: Vec::new(),
                phone: None,
                currency: "EUR".to_string(),
                billing_address: None,
                shipping_address: None,
                invoicing_entity_id: None,
            }),
        })
        .await
        .unwrap()
        .into_inner()
        .customer
        .unwrap();

    assert_eq!(created.name, customer_name.clone());
    assert_eq!(created.alias, Some(customer_alias.clone()));
    assert_eq!(created.billing_email, Some(customer_email.clone()));

    // list by [fake] search
    let list_by_fake = clients
        .customers
        .clone()
        .list_customers(api::customers::v1::ListCustomerRequest {
            search: Some("fake".to_string()),
            sort_by: api::customers::v1::list_customer_request::SortBy::NameAsc as i32,
            pagination: Some(common_grpc::meteroid::common::v1::Pagination {
                per_page: Some(10),
                page: 0,
            }),
        })
        .await
        .unwrap()
        .into_inner()
        .customers;

    assert_eq!(list_by_fake.len(), 0);

    // list by [alias] search
    let list_by_fake = clients
        .customers
        .clone()
        .list_customers(api::customers::v1::ListCustomerRequest {
            search: Some("iiieeennn".to_string()), // part of alias
            sort_by: api::customers::v1::list_customer_request::SortBy::NameAsc as i32,
            pagination: Some(common_grpc::meteroid::common::v1::Pagination {
                per_page: Some(10),
                page: 0,
            }),
        })
        .await
        .unwrap()
        .into_inner()
        .customers;

    assert_eq!(list_by_fake.len(), 1);

    // get by id
    let get_by_id = clients
        .customers
        .clone()
        .get_customer_by_id(api::customers::v1::GetCustomerByIdRequest {
            id: created.id.to_string(),
        })
        .await
        .unwrap()
        .into_inner()
        .customer
        .unwrap();

    assert_eq!(get_by_id.id, created.id.clone());
    assert_eq!(get_by_id.name, customer_name.clone());
    assert_eq!(get_by_id.alias, Some(customer_alias.clone()));
    assert_eq!(get_by_id.currency, "EUR");

    // get by alias
    let get_by_alias = clients
        .customers
        .clone()
        .get_customer_by_alias(api::customers::v1::GetCustomerByAliasRequest {
            alias: customer_alias.to_string(),
        })
        .await
        .unwrap()
        .into_inner()
        .customer
        .unwrap();

    assert_eq!(get_by_alias.id, created.id.clone());
    assert_eq!(get_by_alias.name, customer_name.clone());
    assert_eq!(get_by_alias.alias, Some(customer_alias.clone()));
    assert_eq!(get_by_alias.currency, "EUR");

    // patch
    let _ = clients
        .customers
        .clone()
        .patch_customer(api::customers::v1::PatchCustomerRequest {
            customer: Some(api::customers::v1::PatchCustomer {
                id: created.id.clone(),
                name: Some("new name".to_string()),
                billing_email: None,
                alias: None,
                invoicing_emails: None,
                phone: None,
                balance_value_cents: None,
                currency: None,
                billing_address: None,
                shipping_address: None,
                invoicing_entity_id: None,
                vat_number: None,
            }),
        })
        .await
        .unwrap()
        .into_inner();

    let get_by_id_patched = clients
        .customers
        .clone()
        .get_customer_by_id(api::customers::v1::GetCustomerByIdRequest {
            id: created.id.to_string(),
        })
        .await
        .unwrap()
        .into_inner()
        .customer
        .unwrap();

    assert_eq!(get_by_id_patched.id, created.id.clone());
    assert_eq!(get_by_id_patched.name, "new name".to_string());
    assert_eq!(get_by_id_patched.alias, Some(customer_alias.clone()));
    assert_eq!(get_by_id_patched.currency, "EUR");

    // top up start
    let topped_up = clients
        .customers
        .clone()
        .top_up_customer_balance(api::customers::v1::TopUpCustomerBalanceRequest {
            customer_id: created.id.to_string(),
            cents: 1024,
            notes: Some("test".into()),
        })
        .await
        .unwrap()
        .into_inner()
        .customer
        .unwrap();

    assert_eq!(topped_up.balance_value_cents, 1024);

    let topped_up = clients
        .customers
        .clone()
        .top_up_customer_balance(api::customers::v1::TopUpCustomerBalanceRequest {
            customer_id: created.id.to_string(),
            cents: -1000,
            notes: Some("test".into()),
        })
        .await
        .unwrap()
        .into_inner()
        .customer
        .unwrap();

    assert_eq!(topped_up.balance_value_cents, 24);

    let topped_up_fail = clients
        .customers
        .clone()
        .top_up_customer_balance(api::customers::v1::TopUpCustomerBalanceRequest {
            customer_id: created.id.to_string(),
            cents: -25,
            notes: Some("test".into()),
        })
        .await
        .err()
        .unwrap();

    assert_eq!(topped_up_fail.message(), "negative customer balance");
    assert_eq!(topped_up_fail.code(), Code::FailedPrecondition);

    // top up end

    // bue credits start
    let credits_invoice = clients
        .customers
        .clone()
        .buy_customer_credits(api::customers::v1::BuyCustomerCreditsRequest {
            customer_id: created_manual.id.clone(),
            cents: 1024,
            notes: Some("test".into()),
        })
        .await
        .unwrap()
        .into_inner()
        .invoice
        .unwrap();

    assert_eq!(credits_invoice.total, 1024);
    assert_eq!(credits_invoice.subtotal, 1024);
    assert_eq!(credits_invoice.applied_credits, 0);
    assert_eq!(credits_invoice.customer_id, created_manual.id);
    // bue credits end

    // teardown
    meteroid_it::container::terminate_meteroid(setup.token, setup.join_handle).await
}
