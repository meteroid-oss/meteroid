use chrono::NaiveDate;
use std::sync::Arc;

use crate::data::ids::*;
use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use meteroid_mailer::service::MockMailerService;
use meteroid_store::Services;
use meteroid_store::clients::usage::MockUsageClient;
use meteroid_store::domain::{
    CreateSubscription, InvoicingEntityPatch, SubscriptionActivationCondition, SubscriptionNew,
};
use meteroid_store::repositories::SubscriptionInterface;
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;
use meteroid_store::store::PgConn;

#[tokio::test]
async fn test_compute_invoice_scenarios() {
    helpers::init::logging();
    let (_postgres_container, postgres_connection_string) =
        meteroid_it::container::start_postgres().await;

    let mock_mailer = Arc::new(MockMailerService::new());
    let setup = meteroid_it::container::start_meteroid_with_clients(
        postgres_connection_string,
        SeedLevel::PLANS,
        Arc::new(MockUsageClient::noop()),
        mock_mailer.clone(),
    )
    .await;

    let services = setup.services.clone();
    let store = setup.store.clone();
    let mut conn = setup.store.pool.get().await.unwrap();

    test_compute_invoice_basic(&services, &store, &mut conn).await;
    test_compute_invoice_with_eu_vat(&services, &store, &mut conn).await;
}

async fn test_compute_invoice_basic(
    services: &Services,
    store: &meteroid_store::Store,
    conn: &mut PgConn,
) {
    // Create a subscription using existing pattern from billing tests
    let subscription_id = services
        .insert_subscription(
            CreateSubscription {
                subscription: SubscriptionNew {
                    customer_id: CUST_UBER_ID,
                    plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
                    created_by: USER_ID,
                    net_terms: None,
                    invoice_memo: None,
                    invoice_threshold: None,
                    start_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                    end_date: None,
                    billing_start_date: Some(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()),
                    activation_condition: SubscriptionActivationCondition::OnStart,
                    trial_duration: None,
                    billing_day_anchor: None,
                    payment_strategy: None,
                    auto_advance_invoices: true,
                    charge_automatically: true,
                },
                price_components: None,
                add_ons: None,
                coupons: None,
            },
            TENANT_ID,
        )
        .await
        .unwrap()
        .id;

    // Get subscription details using the store interface directly
    let subscription_details = store
        .get_subscription_details_with_conn(conn, TENANT_ID, subscription_id)
        .await
        .unwrap();

    // Test compute_invoice with this subscription
    let invoice_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let result = services
        .compute_invoice(&invoice_date, &subscription_details, None)
        .await
        .unwrap();

    // Basic assertions
    assert!(result.subtotal > 0, "Subtotal should be positive");
    assert_eq!(
        result.tax_amount, 0,
        "Tax amount should be zero for no tax scenario"
    );
    assert_eq!(
        result.total, result.subtotal,
        "Total should be equal to subtotal"
    );
    assert_eq!(
        result.invoice_lines.len(),
        1,
        "Should have one invoice line"
    );
    assert_eq!(
        result.tax_breakdown.len(),
        0,
        "Should have no tax breakdown"
    );

    // Test business logic
    assert_eq!(
        result.total,
        result.subtotal + result.tax_amount - result.discount + result.applied_credits.abs(),
        "Total calculation should be consistent"
    );

    // Test with a different date
    let full_month = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    let result2 = services
        .compute_invoice(&full_month, &subscription_details, None)
        .await
        .unwrap();

    assert_eq!(
        result2.subtotal, 3500,
        "Subtotal should be the full price component price"
    );
    assert_eq!(
        result2.total, result2.subtotal,
        "Total should be equal to subtotal"
    );

    // Test with prepaid amount
    let prepaid_amount = 1000u64; // 10.00 in cents
    let result3 = services
        .compute_invoice(&invoice_date, &subscription_details, Some(prepaid_amount))
        .await
        .unwrap();

    // Amount due should be reduced by prepaid amount
    assert_eq!(
        result3.amount_due,
        result.amount_due - prepaid_amount as i64,
        "Amount due should be reduced by prepaid amount"
    );

    assert_eq!(
        result3.subtotal, result.subtotal,
        "Subtotal should remain the same after applying prepaid amount"
    );

    assert_eq!(
        result3.total, result.total,
        "Total should remain the same after applying prepaid amount"
    );
}

/// Test compute_invoice with EU VAT tax resolver (France B2B scenario)
async fn test_compute_invoice_with_eu_vat(
    services: &Services,
    store: &meteroid_store::Store,
    conn: &mut PgConn,
) {
    store
        .patch_invoicing_entity(
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                tax_resolver: Some(
                    meteroid_store::domain::enums::TaxResolverEnum::MeteroidEuVat.into(),
                ),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .unwrap();

    // Create a French customer with business address (B2B scenario)
    let french_customer_id = create_french_b2b_customer(services, conn).await;

    // Create a subscription for the French customer
    let subscription_id = services
        .insert_subscription(
            CreateSubscription {
                subscription: SubscriptionNew {
                    customer_id: french_customer_id,
                    plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
                    created_by: USER_ID,
                    net_terms: None,
                    invoice_memo: None,
                    invoice_threshold: None,
                    start_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                    end_date: None,
                    billing_start_date: Some(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()),
                    activation_condition: SubscriptionActivationCondition::OnStart,
                    trial_duration: None,
                    billing_day_anchor: None,
                    payment_strategy: None,
                    auto_advance_invoices: true,
                    charge_automatically: true,
                },
                price_components: None,
                add_ons: None,
                coupons: None,
            },
            TENANT_ID,
        )
        .await
        .unwrap()
        .id;

    // Get subscription details
    let subscription_details = store
        .get_subscription_details_with_conn(conn, TENANT_ID, subscription_id)
        .await
        .unwrap();

    // Test compute_invoice with EU VAT
    let invoice_date = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    let result = services
        .compute_invoice(&invoice_date, &subscription_details, None)
        .await
        .unwrap();

    // we expect a subtotal of 3500, and a 20% tax rate

    assert_eq!(
        result.subtotal, 3500,
        "Subtotal should be 3500 cents for LeetCode plan"
    );
    assert_eq!(
        result.tax_breakdown,
        vec![meteroid_store::domain::TaxBreakdownItem {
            name: "VAT".to_string(),
            tax_rate: rust_decimal::Decimal::new(20, 2),
            taxable_amount: 3500,
        }],
        "Tax breakdown should contain VAT at 20%"
    );

    assert_eq!(
        result.tax_amount, 700,
        "Tax amount should be 700 cents (20% of 3500)"
    );
    assert_eq!(
        result.total, 4200,
        "Total should be 4200 cents (3500 + 700 tax)"
    );
}

async fn create_french_b2b_customer(
    _services: &Services,
    conn: &mut PgConn,
) -> common_domain::ids::CustomerId {
    use diesel_models::customers::CustomerRowNew;
    use meteroid_store::domain::Address;

    let customer_id =
        common_domain::ids::CustomerId::from_proto(&uuid::Uuid::new_v4().to_string()).unwrap();

    // French business address
    let french_address = Address {
        line1: Some("123 Avenue des Champs-Élysées".to_string()),
        line2: None,
        city: Some("Paris".to_string()),
        country: Some("FR".to_string()),
        state: Some("Île-de-France".to_string()),
        zip_code: Some("75008".to_string()),
    };

    let customer_row = CustomerRowNew {
        id: customer_id,
        name: "French Business Customer SAS".to_string(),
        created_at: Some(chrono::Utc::now().naive_utc()),
        created_by: USER_ID,
        tenant_id: TENANT_ID,
        alias: None,
        balance_value_cents: 0,
        currency: "EUR".to_string(),
        invoicing_entity_id: INVOICING_ENTITY_ID,
        billing_address: Some(serde_json::to_value(&french_address).unwrap()),
        shipping_address: None,
        billing_email: None,
        bank_account_id: None,
        current_payment_method_id: None,
        card_provider_id: None,
        direct_debit_provider_id: None,
        vat_number: Some("FR12345678901".to_string()),
        custom_tax_rate: None,
        invoicing_emails: vec![],
        phone: None,
        is_tax_exempt: false,
        vat_number_format_valid: true,
    };

    customer_row.insert(conn).await.unwrap();

    customer_id
}
