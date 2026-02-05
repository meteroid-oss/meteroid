//! Payment seed data for integration tests.
//!
//! This module contains seed functions that create payment-related test data.
//! For TestEnv helper methods, see `harness/payments.rs`.

use diesel_async::AsyncConnection;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::connectors::ConnectorRowNew;
use diesel_models::customer_connection::CustomerConnectionRow;
use diesel_models::customer_payment_methods::CustomerPaymentMethodRowNew;
use diesel_models::customers::CustomerRowPatch;
use diesel_models::enums::{ConnectorProviderEnum, ConnectorTypeEnum, PaymentMethodTypeEnum};
use diesel_models::errors::DatabaseErrorContainer;
use diesel_models::invoicing_entities::InvoicingEntityRowProvidersPatch;
use meteroid_store::store::PgPool;

use crate::data::ids;

// =============================================================================
// Connector Seeds
// =============================================================================

/// Seeds a mock payment provider connector for testing payment flows.
pub async fn run_mock_payment_provider_seed(pool: &PgPool, fail_payment_intent: bool) {
    let mut conn = pool
        .get()
        .await
        .expect("couldn't get db connection from pool");

    conn.transaction(|tx| {
        async move {
            let mock_data = serde_json::json!({
                "Mock": {
                    "fail_payment_intent": fail_payment_intent,
                    "fail_setup_intent": false
                }
            });

            ConnectorRowNew {
                id: ids::MOCK_CONNECTOR_ID,
                tenant_id: ids::TENANT_ID,
                alias: "mock-payment-provider".to_string(),
                connector_type: ConnectorTypeEnum::PaymentProvider,
                provider: ConnectorProviderEnum::Mock,
                data: Some(mock_data),
                sensitive: None,
            }
            .insert(tx)
            .await?;

            InvoicingEntityRowProvidersPatch {
                id: ids::INVOICING_ENTITY_ID,
                card_provider_id: Some(Some(ids::MOCK_CONNECTOR_ID)),
                direct_debit_provider_id: None,
                bank_account_id: None,
            }
            .patch_invoicing_entity_providers(tx, ids::TENANT_ID)
            .await?;

            Ok::<(), DatabaseErrorContainer>(())
        }
        .scope_boxed()
    })
    .await
    .unwrap();
}

/// Seeds a second mock payment provider connector for testing provider switching.
pub async fn run_mock_payment_provider_2_seed(pool: &PgPool) {
    let mut conn = pool
        .get()
        .await
        .expect("couldn't get db connection from pool");

    conn.transaction(|tx| {
        async move {
            let mock_data = serde_json::json!({
                "Mock": {
                    "fail_payment_intent": false,
                    "fail_setup_intent": false
                }
            });

            ConnectorRowNew {
                id: ids::MOCK_CONNECTOR_2_ID,
                tenant_id: ids::TENANT_ID,
                alias: "mock-payment-provider-2".to_string(),
                connector_type: ConnectorTypeEnum::PaymentProvider,
                provider: ConnectorProviderEnum::Mock,
                data: Some(mock_data),
                sensitive: None,
            }
            .insert(tx)
            .await?;

            Ok::<(), DatabaseErrorContainer>(())
        }
        .scope_boxed()
    })
    .await
    .unwrap();
}

// =============================================================================
// Customer Payment Methods Seeds
// =============================================================================

/// Seeds customer payment methods for Uber and Spotify using the primary provider.
pub async fn run_customer_payment_methods_seed(pool: &PgPool) {
    // Uber
    let uber_connection_id = get_or_create_customer_connection(
        pool,
        ids::CUST_UBER_ID,
        ids::CUST_UBER_CONNECTION_ID,
        ids::MOCK_CONNECTOR_ID,
        "mock_cus_uber",
    )
    .await;

    create_customer_payment_method(
        pool,
        ids::CUST_UBER_ID,
        uber_connection_id,
        ids::CUST_UBER_PAYMENT_METHOD_ID,
        "mock_pm_uber_card",
    )
    .await;

    set_customer_default_payment_method(pool, ids::CUST_UBER_ID, ids::CUST_UBER_PAYMENT_METHOD_ID)
        .await;

    // Spotify
    let spotify_connection_id = get_or_create_customer_connection(
        pool,
        ids::CUST_SPOTIFY_ID,
        ids::CUST_SPOTIFY_CONNECTION_ID,
        ids::MOCK_CONNECTOR_ID,
        "mock_cus_spotify",
    )
    .await;

    create_customer_payment_method(
        pool,
        ids::CUST_SPOTIFY_ID,
        spotify_connection_id,
        ids::CUST_SPOTIFY_PAYMENT_METHOD_ID,
        "mock_pm_spotify_card",
    )
    .await;

    set_customer_default_payment_method(
        pool,
        ids::CUST_SPOTIFY_ID,
        ids::CUST_SPOTIFY_PAYMENT_METHOD_ID,
    )
    .await;
}

/// Seeds customer payment methods for Uber using the secondary provider.
pub async fn run_customer_payment_methods_provider_2_seed(pool: &PgPool) {
    let uber_connection_id = get_or_create_customer_connection(
        pool,
        ids::CUST_UBER_ID,
        ids::CUST_UBER_CONNECTION_2_ID,
        ids::MOCK_CONNECTOR_2_ID,
        "mock_cus_uber_provider2",
    )
    .await;

    create_customer_payment_method(
        pool,
        ids::CUST_UBER_ID,
        uber_connection_id,
        ids::CUST_UBER_PAYMENT_METHOD_2_ID,
        "mock_pm_uber_card_provider2",
    )
    .await;
}

// =============================================================================
// Helper Functions (used by seeds and available for ad-hoc test setup)
// =============================================================================

/// Creates a customer connection to a provider, or returns the existing one.
pub async fn get_or_create_customer_connection(
    pool: &PgPool,
    customer_id: common_domain::ids::CustomerId,
    connection_id: common_domain::ids::CustomerConnectionId,
    connector_id: common_domain::ids::ConnectorId,
    external_customer_id: &str,
) -> common_domain::ids::CustomerConnectionId {
    let mut conn = pool
        .get()
        .await
        .expect("couldn't get db connection from pool");

    // Check if connection already exists
    let existing = CustomerConnectionRow::list_connections_by_customer_id(
        &mut conn,
        &ids::TENANT_ID,
        &customer_id,
    )
    .await
    .expect("Failed to list connections")
    .into_iter()
    .find(|c| c.connector_id == connector_id);

    if let Some(existing_conn) = existing {
        return existing_conn.id;
    }

    // Create new connection
    conn.transaction(|tx| {
        async move {
            CustomerConnectionRow {
                id: connection_id,
                customer_id,
                connector_id,
                supported_payment_types: Some(vec![
                    Some(PaymentMethodTypeEnum::Card),
                    Some(PaymentMethodTypeEnum::DirectDebitSepa),
                ]),
                external_customer_id: external_customer_id.to_string(),
            }
            .insert(tx)
            .await?;

            Ok::<(), DatabaseErrorContainer>(())
        }
        .scope_boxed()
    })
    .await
    .unwrap();

    connection_id
}

/// Creates a customer payment method.
pub async fn create_customer_payment_method(
    pool: &PgPool,
    customer_id: common_domain::ids::CustomerId,
    connection_id: common_domain::ids::CustomerConnectionId,
    payment_method_id: common_domain::ids::CustomerPaymentMethodId,
    external_payment_method_id: &str,
) {
    let mut conn = pool
        .get()
        .await
        .expect("couldn't get db connection from pool");

    conn.transaction(|tx| {
        async move {
            CustomerPaymentMethodRowNew {
                id: payment_method_id,
                tenant_id: ids::TENANT_ID,
                customer_id,
                connection_id,
                external_payment_method_id: external_payment_method_id.to_string(),
                payment_method_type: PaymentMethodTypeEnum::Card,
                account_number_hint: None,
                card_brand: Some("mock_visa".to_string()),
                card_last4: Some("4242".to_string()),
                card_exp_month: Some(12),
                card_exp_year: Some(2030),
            }
            .upsert(tx)
            .await?;

            Ok::<(), DatabaseErrorContainer>(())
        }
        .scope_boxed()
    })
    .await
    .unwrap();
}

/// Sets the default payment method for a customer.
pub async fn set_customer_default_payment_method(
    pool: &PgPool,
    customer_id: common_domain::ids::CustomerId,
    payment_method_id: common_domain::ids::CustomerPaymentMethodId,
) {
    let mut conn = pool
        .get()
        .await
        .expect("couldn't get db connection from pool");

    CustomerRowPatch {
        id: customer_id,
        current_payment_method_id: Some(Some(payment_method_id)),
        name: None,
        alias: None,
        billing_email: None,
        invoicing_emails: None,
        phone: None,
        balance_value_cents: None,
        currency: None,
        billing_address: None,
        shipping_address: None,
        invoicing_entity_id: None,
        vat_number: None,
        is_tax_exempt: None,
        custom_taxes: None,
        vat_number_format_valid: None,
    }
    .update(&mut conn, ids::TENANT_ID)
    .await
    .expect("Failed to set customer default payment method");
}

// =============================================================================
// Direct Debit and Bank Transfer Seeds
// =============================================================================

/// Seeds a mock provider as direct debit only (no card).
pub async fn run_direct_debit_provider_seed(pool: &PgPool) {
    let mut conn = pool
        .get()
        .await
        .expect("couldn't get db connection from pool");

    conn.transaction(|tx| {
        async move {
            let mock_data = serde_json::json!({
                "Mock": {
                    "fail_payment_intent": false,
                    "fail_setup_intent": false
                }
            });

            ConnectorRowNew {
                id: ids::MOCK_CONNECTOR_ID,
                tenant_id: ids::TENANT_ID,
                alias: "mock-dd-provider".to_string(),
                connector_type: ConnectorTypeEnum::PaymentProvider,
                provider: ConnectorProviderEnum::Mock,
                data: Some(mock_data),
                sensitive: None,
            }
            .insert(tx)
            .await?;

            // Set only direct_debit_provider, NOT card_provider
            InvoicingEntityRowProvidersPatch {
                id: ids::INVOICING_ENTITY_ID,
                card_provider_id: None,
                direct_debit_provider_id: Some(Some(ids::MOCK_CONNECTOR_ID)),
                bank_account_id: None,
            }
            .patch_invoicing_entity_providers(tx, ids::TENANT_ID)
            .await?;

            Ok::<(), DatabaseErrorContainer>(())
        }
        .scope_boxed()
    })
    .await
    .unwrap();
}

/// Seeds same mock provider for both card and direct debit.
pub async fn run_card_and_dd_same_provider_seed(pool: &PgPool) {
    let mut conn = pool
        .get()
        .await
        .expect("couldn't get db connection from pool");

    conn.transaction(|tx| {
        async move {
            let mock_data = serde_json::json!({
                "Mock": {
                    "fail_payment_intent": false,
                    "fail_setup_intent": false
                }
            });

            ConnectorRowNew {
                id: ids::MOCK_CONNECTOR_ID,
                tenant_id: ids::TENANT_ID,
                alias: "mock-card-and-dd-provider".to_string(),
                connector_type: ConnectorTypeEnum::PaymentProvider,
                provider: ConnectorProviderEnum::Mock,
                data: Some(mock_data),
                sensitive: None,
            }
            .insert(tx)
            .await?;

            // Same provider for both card and DD
            InvoicingEntityRowProvidersPatch {
                id: ids::INVOICING_ENTITY_ID,
                card_provider_id: Some(Some(ids::MOCK_CONNECTOR_ID)),
                direct_debit_provider_id: Some(Some(ids::MOCK_CONNECTOR_ID)),
                bank_account_id: None,
            }
            .patch_invoicing_entity_providers(tx, ids::TENANT_ID)
            .await?;

            Ok::<(), DatabaseErrorContainer>(())
        }
        .scope_boxed()
    })
    .await
    .unwrap();
}

/// Seeds a bank account for bank transfer testing.
pub async fn run_bank_account_seed(pool: &PgPool) {
    use common_domain::country::CountryCode;
    use diesel_models::bank_accounts::BankAccountRowNew;
    use diesel_models::enums::BankAccountFormat;

    let mut conn = pool
        .get()
        .await
        .expect("couldn't get db connection from pool");

    conn.transaction(|tx| {
        async move {
            BankAccountRowNew {
                id: ids::TEST_BANK_ACCOUNT_ID,
                tenant_id: ids::TENANT_ID,
                created_by: ids::USER_ID,
                currency: "EUR".to_string(),
                country: CountryCode::default(), // FR
                bank_name: "Test Bank".to_string(),
                format: BankAccountFormat::IbanBicSwift,
                account_numbers: "FR7630006000011234567890189".to_string(),
            }
            .insert(tx)
            .await?;

            InvoicingEntityRowProvidersPatch {
                id: ids::INVOICING_ENTITY_ID,
                card_provider_id: None,
                direct_debit_provider_id: None,
                bank_account_id: Some(Some(ids::TEST_BANK_ACCOUNT_ID)),
            }
            .patch_invoicing_entity_providers(tx, ids::TENANT_ID)
            .await?;

            Ok::<(), DatabaseErrorContainer>(())
        }
        .scope_boxed()
    })
    .await
    .unwrap();
}
