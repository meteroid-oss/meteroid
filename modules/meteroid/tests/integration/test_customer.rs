use meteroid_grpc::meteroid::api;
use serde_json::json;

use crate::data::ids::TENANT_ID;
use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::clients::AllClients;
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};
use common_domain::ids::{BaseId, ConnectorId, CustomerId, InvoiceId, TenantId};
use diesel_models::connectors::ConnectorRowNew;
use diesel_models::enums::{ConnectorProviderEnum as DbConnectorProviderEnum, ConnectorTypeEnum};
use meteroid_store::domain::{ConnectorProviderEnum, LineItemKind};
use meteroid_store::repositories::CustomersInterface;
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::repositories::customers::CustomersInterfaceAuto;

#[tokio::test]
async fn test_customers_basic() {
    // Generic setup
    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::MINIMAL).await;
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
                customer_type: api::customers::v1::CustomerType::Company as i32,
                first_name: None,
                last_name: None,
                alias: Some(customer_alias.to_string()),
                billing_email: Some(customer_email.to_string()),
                invoicing_emails: Vec::new(),
                phone: None,
                currency: "EUR".to_string(),
                billing_address: None,
                shipping_address: None,
                invoicing_entity_id: None,
                vat_number: None,
                legal_number: None,
                buyer_reference: None,
                custom_taxes: vec![],
                tax_status: None,
                exemption_reason: None,
                connected_account_id: None,
                custom_properties: None,
                preferred_locales: vec![],
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
            data: Some(api::customers::v1::CustomerNew {
                name: "created_manual".to_string(),
                customer_type: api::customers::v1::CustomerType::Company as i32,
                first_name: None,
                last_name: None,
                alias: Some("created_manual".to_string()),
                billing_email: Some("created_manual@meteroid.com".to_string()),
                invoicing_emails: Vec::new(),
                phone: None,
                currency: "EUR".to_string(),
                billing_address: None,
                shipping_address: None,
                invoicing_entity_id: None,
                vat_number: None,
                legal_number: None,
                buyer_reference: None,
                custom_taxes: vec![],
                tax_status: None,
                exemption_reason: None,
                connected_account_id: None,
                custom_properties: None,
                preferred_locales: vec![],
            }),
        })
        .await
        .unwrap()
        .into_inner()
        .customer
        .unwrap();

    // Create a connector to use for testing customer connections
    let connector_id = ConnectorId::new();
    {
        let mut conn = setup.store.pool.get().await.unwrap();
        ConnectorRowNew {
            id: connector_id,
            tenant_id: TENANT_ID,
            alias: "test-hubspot-connector".to_string(),
            connector_type: ConnectorTypeEnum::Crm,
            provider: DbConnectorProviderEnum::Hubspot,
            data: None,
            sensitive: None,
            status: diesel_models::enums::ConnectorStatusEnum::Active,
            account: None,
            webhook_route: Some(diesel_models::enums::ConnectorWebhookRouteEnum::ConnectorId),
        }
        .insert(&mut conn)
        .await
        .unwrap();
    }

    setup
        .store
        .patch_customer_conn_meta(
            TENANT_ID,
            CustomerId::from_proto(created.id.as_str()).unwrap(),
            connector_id,
            ConnectorProviderEnum::Hubspot,
            "idk",
            "123",
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

    assert_eq!(created.name, customer_name.clone());
    assert_eq!(created.alias, Some(customer_alias.clone()));
    assert_eq!(created.billing_email, Some(customer_email.clone()));

    // list by [email] search
    let list_by_fake = clients
        .customers
        .clone()
        .list_customers(api::customers::v1::ListCustomerRequest {
            search: Some("fake".to_string()),
            archived: None,
            order_by: None,
            pagination: Some(common_grpc::meteroid::common::v1::Pagination {
                per_page: Some(10),
                page: 0,
            }),
            lifecycle_state: vec![],
            country: vec![],
            filters: vec![],
        })
        .await
        .unwrap()
        .into_inner()
        .customers;

    assert_eq!(list_by_fake.len(), 1);

    // list by [alias] search
    let list_by_fake = clients
        .customers
        .clone()
        .list_customers(api::customers::v1::ListCustomerRequest {
            search: Some("iiieeennn".to_string()), // part of alias
            archived: None,
            order_by: None,
            pagination: Some(common_grpc::meteroid::common::v1::Pagination {
                per_page: Some(10),
                page: 0,
            }),
            lifecycle_state: vec![],
            country: vec![],
            filters: vec![],
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
                customer_type: None,
                first_name: None,
                last_name: None,
                billing_email: None,
                alias: None,
                invoicing_emails: None,
                phone: None,
                currency: None,
                billing_address: None,
                shipping_address: None,
                invoicing_entity_id: None,
                vat_number: None,
                legal_number: None,
                buyer_reference: None,
                custom_taxes: None,
                tax_status: None,
                exemption_reason: None,
                custom_properties: None,
                preferred_locales: vec![],
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

    // sell credits start
    let wallet = clients
        .wallets
        .clone()
        .create_wallet(api::wallets::v1::CreateWalletRequest {
            customer_id: created_manual.id.clone(),
            currency: "EUR".to_string(),
        })
        .await
        .unwrap()
        .into_inner()
        .wallet
        .unwrap();
    let sold = clients
        .wallets
        .clone()
        .sell_credit(api::wallets::v1::SellCreditRequest {
            customer_id: created_manual.id.clone(),
            wallet_id: wallet.id,
            amount: 1024,
            expires_in_months: None,
            note: Some("test".into()),
            idempotency_key: uuid::Uuid::now_v7().to_string(),
            unit_price: None,
            expected_unit_price: None,
        })
        .await
        .unwrap()
        .into_inner();
    let credits_invoice = setup
        .store
        .get_detailed_invoice_by_id(TENANT_ID, InvoiceId::from_proto(&sold.invoice_id).unwrap())
        .await
        .unwrap()
        .invoice;

    assert_eq!(credits_invoice.total, 1024);
    assert_eq!(
        credits_invoice.line_items[0].kind,
        LineItemKind::Credit,
        "a credit sale issues a Credit line, funded when paid"
    );
    assert_eq!(credits_invoice.subtotal, 1024);
    assert_eq!(credits_invoice.applied_credits, 0);
    assert_eq!(credits_invoice.customer_id.to_string(), created_manual.id);
    // sell credits end

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

    let client = http_client_tracing::client();

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
                "country": "FR"
            },
            "shipping_address": {
               "same_as_billing": true
            },
            "vat_number": "VAT123456",
            "custom_taxes": [,
                {
                    "tax_code": "TAX001",
                    "name": "Standard Tax",
                    "rate": 20.0
                }
            ]
        }))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();

    let created = scrub_customer_json(&mut created, &["id", "invoicing_entity_id"]);

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

    let get_by_alias = scrub_customer_json(&mut get_by_alias, &["id", "invoicing_entity_id"]);

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

    let list = scrub_customer_json(&mut list, &["id", "invoicing_entity_id"]);
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
                "country": "FR"
            },
            "shipping_address": {
               "same_as_billing": true
            },
            "vat_number": "VAT123456",
            "custom_taxes": [,
                {
                    "tax_code": "TAX001",
                    "name": "Standard Tax",
                    "rate": 20.0
                }
            ],
            "invoicing_entity_id": invoicing_entity_id,
        }))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();

    let updated = scrub_customer_json(&mut updated, &["id", "invoicing_entity_id"]);
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
                    if let Some(id_str) = v.as_str()
                        && let Some(pos) = id_str.rfind('_')
                    {
                        let obfuscated = format!("{}{}", &id_str[..=pos], "xxx");
                        *v = serde_json::Value::String(obfuscated);
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
