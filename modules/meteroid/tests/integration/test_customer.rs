use meteroid_grpc::meteroid::api;
use serde_json::json;

use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::clients::AllClients;
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};
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
                bank_account_id: None,
                vat_number: None,
                custom_vat_rate: None,
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
                bank_account_id: None,
                vat_number: None,
                custom_vat_rate: None,
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

    // update
    let _ = clients
        .customers
        .clone()
        .update_customer(api::customers::v1::UpdateCustomerRequest {
            customer: Some(api::customers::v1::UpdateCustomer {
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
                custom_vat_rate: None,
                bank_account_id: None,
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

    rest_api_test(&setup, &clients).await;
}

async fn rest_api_test(setup: &MeteroidSetup, clients: &AllClients) {
    let api_key = clients
        .api_tokens
        .clone()
        .create_api_token(tonic::Request::new(
            api::apitokens::v1::CreateApiTokenRequest {
                name: "test-api-key".to_string(),
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .api_key;

    let client = reqwest::Client::new();

    // CREATE CUSTOMER
    let mut created = client
        .post(format!(
            "{}/api/v1/customers",
            setup.config.rest_api_external_url
        ))
        .bearer_auth(&api_key)
        .json(&json!({
            "name": "Test Customer REST",
            "alias": "test-customer-rest",
            "billing_email": "billing@meteroid.com",
            "invoicing_emails": ["invoicing@meteroid.com"],
            "phone": "123456789",
            "currency": "EUR",
            "billing_address": {
                "line1": "123 Test St",
                "city": "Test City",
                "zip_code": "12345",
                "country": "Testland"
            },
            "shipping_address": {
               "same_as_billing": true
            },
            "vat_number": "VAT123456",
            "custom_vat_rate": 20,
        }))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();

    let created = scrub_customer_json(
        &mut created,
        &["id", "invoicing_entity_id", "bank_account_id"],
    );

    insta::assert_json_snapshot!("rest_created", &created);

    // GET BY ALIAS
    let mut get_by_alias = client
        .get(format!(
            "{}/api/v1/customers/test-customer-rest",
            setup.config.rest_api_external_url
        ))
        .bearer_auth(&api_key)
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();

    let invoicing_entity_id = get_by_alias
        .get("invoicing_entity_id")
        .cloned()
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();

    let get_by_alias = scrub_customer_json(
        &mut get_by_alias,
        &["id", "invoicing_entity_id", "bank_account_id"],
    );

    insta::assert_json_snapshot!("rest_get_by_alias", &get_by_alias);

    // LIST CUSTOMERS
    let mut list = client
        .get(format!(
            "{}/api/v1/customers",
            setup.config.rest_api_external_url
        ))
        .bearer_auth(&api_key)
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();

    let list = scrub_customer_json(&mut list, &["id", "invoicing_entity_id", "bank_account_id"]);
    insta::assert_json_snapshot!("rest_list", &list);

    // UPDATE CUSTOMER
    let mut updated = client
        .put(format!(
            "{}/api/v1/customers/test-customer-rest",
            setup.config.rest_api_external_url
        ))
        .bearer_auth(&api_key)
        .json(&json!({
            "name": "Test Customer REST",
            "alias": "test-customer-rest",
            "billing_email": "billing@meteroid.com",
            "invoicing_emails": ["invoicing@meteroid.com"],
            "phone": "123456789",
            "currency": "USD",
            "billing_address": {
                "line1": "123 Test St",
                "city": "Test City",
                "zip_code": "12345",
                "country": "Testland"
            },
            "shipping_address": {
               "same_as_billing": true
            },
            "vat_number": "VAT123456",
            "custom_vat_rate": 20,
            "invoicing_entity_id": invoicing_entity_id,
        }))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();

    let updated = scrub_customer_json(
        &mut updated,
        &["id", "invoicing_entity_id", "bank_account_id"],
    );
    insta::assert_json_snapshot!("rest_updated", &updated);

    // DELETE CUSTOMER
    let delete_response = client
        .delete(format!(
            "{}/api/v1/customers/test-customer-rest",
            setup.config.rest_api_external_url
        ))
        .bearer_auth(&api_key)
        .send()
        .await
        .unwrap();

    assert_eq!(delete_response.status(), reqwest::StatusCode::NO_CONTENT);
}

/// Mask non-static fields like ids/timestamps in JSON values.
fn scrub_customer_json(value: &mut serde_json::Value, ids: &[&'static str]) -> serde_json::Value {
    match value {
        serde_json::Value::Object(obj) => {
            for (k, v) in obj.iter_mut() {
                if ids.contains(&k.as_str()) {
                    if let Some(id_str) = v.as_str() {
                        if let Some(pos) = id_str.rfind('_') {
                            let obfuscated = format!("{}{}", &id_str[..=pos], "xxx");
                            *v = serde_json::Value::String(obfuscated);
                        }
                    }
                } else {
                    *v = scrub_customer_json(&mut v.take(), ids);
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr.iter_mut() {
                *v = scrub_customer_json(&mut v.take(), ids);
            }
        }
        _ => {}
    }
    value.clone()
}
