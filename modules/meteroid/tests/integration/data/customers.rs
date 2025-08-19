use super::ids;
use chrono::NaiveDateTime;
use diesel_async::AsyncConnection;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::customers::CustomerRowNew;
use diesel_models::errors::DatabaseErrorContainer;
use meteroid_store::store::PgPool;
use std::str::FromStr;

pub async fn run_customers_seed(pool: &PgPool) {
    let mut conn = pool
        .get()
        .await
        .expect("couldn't get db connection from pool");

    conn.transaction(|tx| {
        async move {
            CustomerRowNew {
                id: ids::CUST_SPOTIFY_ID,
                name: "Spotify".to_string(),
                created_at: NaiveDateTime::from_str("2023-12-04T10:28:39").ok(),
                created_by: ids::USER_ID,
                tenant_id: ids::TENANT_ID,
                alias: Some("spotify".to_string()),
                balance_value_cents: 0,
                currency: "EUR".to_string(),
                invoicing_entity_id: ids::INVOICING_ENTITY_ID,
                billing_address: None,
                shipping_address: None,
                billing_email: None,
                bank_account_id: None,
                current_payment_method_id: None,
                card_provider_id: None,
                direct_debit_provider_id: None,
                vat_number: None,
                custom_tax_rate: None,
                invoicing_emails: vec![],
                phone: None,
                is_tax_exempt: false,
                vat_number_format_valid: false,
            }
            .insert(tx)
            .await?;

            CustomerRowNew {
                id: ids::CUST_UBER_ID,
                name: "Uber".to_string(),
                created_at: NaiveDateTime::from_str("2023-12-04T10:29:07").ok(),
                created_by: ids::USER_ID,
                tenant_id: ids::TENANT_ID,
                alias: Some("uber".to_string()),
                balance_value_cents: 0,
                currency: "EUR".to_string(),
                invoicing_entity_id: ids::INVOICING_ENTITY_ID,
                billing_address: None,
                shipping_address: None,
                billing_email: None,
                bank_account_id: None,
                current_payment_method_id: None,
                card_provider_id: None,
                direct_debit_provider_id: None,
                vat_number: None,
                custom_tax_rate: None,
                invoicing_emails: vec![],
                phone: None,
                is_tax_exempt: false,
                vat_number_format_valid: false,
            }
            .insert(tx)
            .await?;

            CustomerRowNew {
                id: ids::CUST_COMODO_ID,
                name: "Comodo".to_string(),
                created_at: NaiveDateTime::from_str("2023-12-04T10:32:34").ok(),
                created_by: ids::USER_ID,
                tenant_id: ids::TENANT_ID,
                alias: Some("comodo".to_string()),
                balance_value_cents: 0,
                currency: "EUR".to_string(),
                invoicing_entity_id: ids::INVOICING_ENTITY_ID,
                billing_address: None,
                shipping_address: None,
                billing_email: None,
                bank_account_id: None,
                current_payment_method_id: None,
                card_provider_id: None,
                direct_debit_provider_id: None,
                vat_number: None,
                custom_tax_rate: None,
                invoicing_emails: vec![],
                phone: None,
                is_tax_exempt: false,
                vat_number_format_valid: false,
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
