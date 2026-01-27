use super::ids;
use chrono::{NaiveDate, NaiveDateTime};
use common_domain::country::CountryCode;
use diesel_async::AsyncConnection;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::api_tokens::ApiTokenRowNew;
use diesel_models::connectors::ConnectorRowNew;
use diesel_models::enums::{
    ConnectorProviderEnum, ConnectorTypeEnum, OrganizationUserRole, TenantEnvironmentEnum,
};
use diesel_models::errors::DatabaseErrorContainer;
use diesel_models::historical_rates_from_usd::HistoricalRatesFromUsdRowNew;
use diesel_models::invoicing_entities::{InvoicingEntityRow, InvoicingEntityRowProvidersPatch};
use diesel_models::organization_members::OrganizationMemberRow;
use diesel_models::organizations::{OrganizationRow, OrganizationRowNew};
use diesel_models::tenants::TenantRowNew;
use diesel_models::users::UserRowNew;
use meteroid_store::store::PgPool;
use std::str::FromStr;

pub async fn run_minimal_seed(pool: &PgPool) {
    let mut conn = pool
        .get()
        .await
        .expect("couldn't get db connection from pool");

    conn.transaction(|tx| async move {

        // create organization
        OrganizationRowNew {
            id: ids::ORGANIZATION_ID,
            trade_name: "Local Org".to_string(),
            slug: "TESTORG".to_string(),
            default_country: CountryCode::from_str("FR").expect("failed to parse country code"),
        }.insert(tx).await?;

        OrganizationRow::update_invite_link(
            tx,
            ids::ORGANIZATION_ID,
            &"fake-invite-link".to_string(),
        ).await?;

        // create user
        UserRowNew {
            id: ids::USER_ID,
            email: "demo-user@meteroid.dev".to_string(),
            password_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$dawIX5+sybNHqfFoNvHFhw$uhtWJd50wiFDV8nR10RNZI4OCrOAJ1kiNZQF0OUSoGE".to_string()),
        }.insert(tx).await?;

        // create organization member
        OrganizationMemberRow {
            user_id: ids::USER_ID,
            organization_id: ids::ORGANIZATION_ID,
            role: OrganizationUserRole::Admin,
        }.insert(tx).await?;

        TenantRowNew {
            id: ids::TENANT_ID,
            name: "Sandbox".to_string(),
            slug: "testslug".to_string(),
            organization_id: ids::ORGANIZATION_ID,
            reporting_currency: "EUR".to_string(),
            environment: TenantEnvironmentEnum::Development,
            available_currencies: vec![Some("EUR".to_string()), Some("USD".to_string())],
            disable_emails: true
        }.insert(tx).await?;

        // create api token
        ApiTokenRowNew {
            id: ids::API_TOKEN_ID,
            name: "token-pD_".to_string(),
            created_at: NaiveDateTime::from_str("2024-01-03T00:00:00").expect("failed to parse api token date"),
            created_by: ids::USER_ID,
            tenant_id: ids::TENANT_ID,
            hash: "$argon2id$v=19$m=19456,t=2,p=1$98CkbdqB8KNdlqryCBIx+g$nhTanF/4QsVnpPFvPHzshLPOGd7btYxXfq2UWB0xkiU".to_string(),
            hint: "pv_sand_9XzH...AbBG".to_string(),
        }.insert(tx).await?;

        // create invoicing entity
        InvoicingEntityRow {
            id: ids::INVOICING_ENTITY_ID,
            is_default: true,
            legal_name: "ACME_UK".to_string(),
            invoice_number_pattern: "INV-{number}".to_string(),
            next_invoice_number: 1,
            next_credit_note_number: 1,
            grace_period_hours: 12,
            net_terms: 30,
            invoice_footer_info: Some("hello".to_string()),
            invoice_footer_legal: Some("world".to_string()),
            logo_attachment_id: None,
            brand_color: None,
            address_line1: None,
            address_line2: None,
            zip_code: None,
            state: None,
            city: None,
            vat_number: None,
            country: CountryCode::from_str("FR").expect("failed to parse country code"),
            accounting_currency: "EUR".to_string(),
            tenant_id: ids::TENANT_ID,
            card_provider_id: None,
            direct_debit_provider_id: None,
            bank_account_id: None,
            tax_resolver: diesel_models::enums::TaxResolverEnum::None,
        }.insert(tx).await?;

        HistoricalRatesFromUsdRowNew::insert_batch(tx, vec![
            HistoricalRatesFromUsdRowNew {
                id: ids::HISTORICAL_RATE_2024_01_01_ID,
                date: NaiveDate::from_str("2024-01-01").expect("failed to parse historical rates date"),
                rates: serde_json::json!({
                "AUD": 1.468645,
                "BRL": 4.8539,
                "CAD": 1.324436,
                "CHF": 0.841915,
                "CNY": 7.0786,
                "COP": 3887.87175,
                "EUR": 0.906074,
                "GBP": 0.78569,
                "HKD": 7.81035,
                "JPY": 141.115,
                "KRW": 1280.64,
                "MXN": 16.9664,
                "NZD": 1.583713,
                "SEK": 10.074633,
                "USD": 1
            }),
                updated_at: chrono::Utc::now().naive_utc(),
            },
            HistoricalRatesFromUsdRowNew {
                id: ids::HISTORICAL_RATE_2010_01_01_ID,
                date: NaiveDate::from_str("2010-01-01").expect("failed to parse historical rates date"),
                rates: serde_json::json!({
                "AUD": 1.108609,
                "BRL": 1.741616,
                "CAD": 1.048367,
                "CHF": 1.0338,
                "CNY": 6.828759,
                "COP": 2044.171135,
                "EUR": 0.697253,
                "GBP": 0.618224,
                "HKD": 7.754729,
                "JPY": 92.910732,
                "KRW": 1160.640163,
                "MXN": 13.108757,
                "NZD": 1.377768,
                "SEK": 7.138645,
                "USD": 1
            }),
                updated_at: chrono::Utc::now().naive_utc(),
            },
        ]).await?;

        Ok::<(), DatabaseErrorContainer>(())
    } .scope_boxed()).await.unwrap();
}

use diesel_models::customer_connection::CustomerConnectionRow;
use diesel_models::customer_payment_methods::CustomerPaymentMethodRowNew;
use diesel_models::customers::CustomerRowPatch;
use diesel_models::enums::PaymentMethodTypeEnum;

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

/// Creates a customer connection or returns the existing one if it already exists.
pub async fn get_or_create_customer_connection(
    pool: &PgPool,
    customer_id: common_domain::ids::CustomerId,
    connection_id: common_domain::ids::CustomerConnectionId,
    external_customer_id: &str,
) -> common_domain::ids::CustomerConnectionId {
    let mut conn = pool
        .get()
        .await
        .expect("couldn't get db connection from pool");

    // Check if connection already exists (e.g., created during subscription with auto_charge)
    let existing = CustomerConnectionRow::list_connections_by_customer_id(
        &mut conn,
        &ids::TENANT_ID,
        &customer_id,
    )
    .await
    .expect("Failed to list connections")
    .into_iter()
    .find(|c| c.connector_id == ids::MOCK_CONNECTOR_ID);

    if let Some(existing_conn) = existing {
        return existing_conn.id;
    }

    // Create new connection
    conn.transaction(|tx| {
        async move {
            CustomerConnectionRow {
                id: connection_id,
                customer_id,
                connector_id: ids::MOCK_CONNECTOR_ID,
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

pub async fn run_customer_payment_methods_seed(pool: &PgPool) {
    let uber_connection_id = get_or_create_customer_connection(
        pool,
        ids::CUST_UBER_ID,
        ids::CUST_UBER_CONNECTION_ID,
        "mock_cus_uber",
    )
    .await;

    let spotify_connection_id = get_or_create_customer_connection(
        pool,
        ids::CUST_SPOTIFY_ID,
        ids::CUST_SPOTIFY_CONNECTION_ID,
        "mock_cus_spotify",
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

    create_customer_payment_method(
        pool,
        ids::CUST_SPOTIFY_ID,
        spotify_connection_id,
        ids::CUST_SPOTIFY_PAYMENT_METHOD_ID,
        "mock_pm_spotify_card",
    )
    .await;

    // Set the payment methods as the default for each customer
    set_customer_default_payment_method(pool, ids::CUST_UBER_ID, ids::CUST_UBER_PAYMENT_METHOD_ID)
        .await;
    set_customer_default_payment_method(
        pool,
        ids::CUST_SPOTIFY_ID,
        ids::CUST_SPOTIFY_PAYMENT_METHOD_ID,
    )
    .await;
}

async fn set_customer_default_payment_method(
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
        bank_account_id: None,
        is_tax_exempt: None,
        custom_taxes: None,
        vat_number_format_valid: None,
    }
    .update(&mut conn, ids::TENANT_ID)
    .await
    .expect("Failed to set customer default payment method");
}
