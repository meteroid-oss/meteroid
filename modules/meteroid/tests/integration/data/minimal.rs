use super::ids;
use chrono::{NaiveDate, NaiveDateTime};
use diesel_async::AsyncConnection;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::api_tokens::ApiTokenRowNew;
use diesel_models::enums::{OrganizationUserRole, TenantEnvironmentEnum};
use diesel_models::errors::DatabaseErrorContainer;
use diesel_models::historical_rates_from_usd::HistoricalRatesFromUsdRowNew;
use diesel_models::invoicing_entities::InvoicingEntityRow;
use diesel_models::organization_members::OrganizationMemberRow;
use diesel_models::organizations::OrganizationRowNew;
use diesel_models::tenants::TenantRowNew;
use diesel_models::users::UserRowNew;
use meteroid_store::store::PgPool;
use std::str::FromStr;
use uuid::Uuid;

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
            default_country: "FR".to_string(),
        }.insert(tx).await?;

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
            country: "FR".to_string(),
            accounting_currency: "EUR".to_string(),
            tenant_id: ids::TENANT_ID,
            card_provider_id: None,
            direct_debit_provider_id: None,
            bank_account_id: None,
        }.insert(tx).await?;

        HistoricalRatesFromUsdRowNew::insert_batch(tx, vec![
            HistoricalRatesFromUsdRowNew {
                id: Uuid::new_v4(),
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
                id: Uuid::new_v4(),
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
