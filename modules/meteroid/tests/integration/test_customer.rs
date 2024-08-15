use meteroid_grpc::meteroid::api;
use meteroid_grpc::meteroid::api::users::v1::UserRole;
use testcontainers::clients::Cli;
use tonic::Code;

use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;

#[tokio::test]
async fn test_customers_basic() {
    // Generic setup
    helpers::init::logging();
    let docker = Cli::default();
    let (_postgres_container, postgres_connection_string) =
        meteroid_it::container::start_postgres(&docker);
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;

    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    assert_eq!(auth.user.unwrap().role, UserRole::Admin as i32);

    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "a712afi5lzhk",
    );

    let customer_name = "friends and co".to_owned();
    let customer_alias = "fffrrriiieeennndddsss".to_owned();
    let customer_email = "fake@fake.com".to_owned();

    // create customer
    let created = clients
        .customers
        .clone()
        .create_customer(api::customers::v1::CreateCustomerRequest {
            name: customer_name.to_string(),
            alias: Some(customer_alias.to_string()),
            email: Some(customer_email.to_string()),
            billing_config: Some(api::customers::v1::CustomerBillingConfig {
                billing_config_oneof: Some(
                    api::customers::v1::customer_billing_config::BillingConfigOneof::Stripe(
                        api::customers::v1::customer_billing_config::Stripe {
                            customer_id: "customer_id".to_string(),
                            collection_method: 0,
                        },
                    ),
                ),
            }),
        })
        .await
        .unwrap()
        .into_inner()
        .customer
        .unwrap();

    let created_manual = clients
        .customers
        .clone()
        .create_customer(api::customers::v1::CreateCustomerRequest {
            name: "created_manual".to_string(),
            alias: Some("created_manual".to_string()),
            email: Some("created_manual@meteroid.com".to_string()),
            billing_config: Some(api::customers::v1::CustomerBillingConfig {
                billing_config_oneof: Some(
                    api::customers::v1::customer_billing_config::BillingConfigOneof::Manual(
                        api::customers::v1::customer_billing_config::Manual {},
                    ),
                ),
            }),
        })
        .await
        .unwrap()
        .into_inner()
        .customer
        .unwrap();

    assert_eq!(created.name, customer_name.clone());
    assert_eq!(created.alias, Some(customer_alias.clone()));
    assert_eq!(created.email, Some(customer_email.clone()));

    // list by [fake] search
    let list_by_fake = clients
        .customers
        .clone()
        .list_customers(api::customers::v1::ListCustomerRequest {
            search: Some("fake".to_string()),
            sort_by: api::customers::v1::list_customer_request::SortBy::NameAsc as i32,
            pagination: Some(common_grpc::meteroid::common::v1::Pagination {
                limit: 10,
                offset: 0,
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
                limit: 10,
                offset: 0,
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

    // patch
    let _ = clients
        .customers
        .clone()
        .patch_customer(api::customers::v1::PatchCustomerRequest {
            customer: Some(api::customers::v1::PatchCustomer {
                id: created.id.clone(),
                name: Some("new name".to_string()),
                email: None,
                alias: None,
                invoicing_email: None,
                phone: None,
                balance_value_cents: None,
                balance_currency: None,
                billing_address: None,
                shipping_address: None,
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
