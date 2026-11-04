//! A minimum-commitment shortfall is taxed (OQ17): an all-components minimum at the default rate,
//! a product-scoped one by its products' tax settings, split pro rata to what each counted.
use std::str::FromStr;

use chrono::NaiveDate;
use common_domain::country::CountryCode;
use common_domain::ids::{BaseId, CustomerId};
use diesel_async::RunQueryDsl;
use diesel_models::plan_version_adjustments::PlanVersionAdjustmentRowNew;
use meteroid_store::domain::enums::{InvoiceType, TaxResolverEnum};
use meteroid_store::domain::plan_minimums::MinimumScope;
use meteroid_store::domain::{
    Address, CustomerTaxRate, InvoicingEntityPatch, LineItemKind, SubscriptionMinimum,
};
use meteroid_store::repositories::SubscriptionInterface;
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;
use rust_decimal_macros::dec;

use crate::data::ids::*;
use crate::harness::{TestEnv, subscription, test_env};
use crate::helpers::credit::{Grant, fund_grant, fund_promotional_grant};

async fn tax_resolver(env: &TestEnv, resolver: TaxResolverEnum) {
    env.store()
        .patch_invoicing_entity(
            common_domain::actor::Actor::System,
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                tax_resolver: Some(resolver),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .expect("tax resolver");
}

async fn customer(
    env: &TestEnv,
    country: &str,
    state: Option<&str>,
    custom_taxes: Vec<CustomerTaxRate>,
) -> CustomerId {
    let customer_id = CustomerId::new();
    diesel_models::customers::CustomerRowNew {
        id: customer_id,
        name: format!("Customer {country}"),
        created_at: Some(chrono::Utc::now().naive_utc()),
        tenant_id: TENANT_ID,
        alias: None,
        currency: "EUR".to_string(),
        invoicing_entity_id: INVOICING_ENTITY_ID,
        billing_address: Some(
            serde_json::to_value(Address {
                line1: Some("1 Main Street".to_string()),
                line2: None,
                city: Some("Columbus".to_string()),
                country: Some(CountryCode::from_str(country).expect("country")),
                state: state.map(str::to_owned),
                zip_code: Some("43004".to_string()),
            })
            .expect("address"),
        ),
        shipping_address: None,
        billing_email: None,
        current_payment_method_id: None,
        vat_number: (country == "FR").then(|| "FR12345678901".to_string()),
        legal_number: None,
        buyer_reference: None,
        custom_taxes: serde_json::to_value(custom_taxes).expect("taxes"),
        invoicing_emails: vec![],
        phone: None,
        tax_status: diesel_models::enums::CustomerTaxStatusEnum::Taxable,
        exemption_reason: None,
        vat_number_format_valid: country == "FR",
        connected_account_id: None,
        vat_number_validation_status: None,
        custom_properties: serde_json::Value::Null,
        preferred_locales: vec![],
        customer_type: diesel_models::enums::CustomerTypeEnum::Company,
        first_name: None,
        last_name: None,
    }
    .insert(&mut env.conn().await)
    .await
    .expect("customer");
    customer_id
}

/// S15, Ohio variant, in hundredths: LeetCode bills 35.00 for January under a 50.00 all-components
/// minimum, so February's invoice trues January up by 15.00 beside February's 35.00; 5.00 of
/// promotional credit comes off before tax and 10.00 of paid credit after it. Ohio's 8% is the
/// default rate, levied on 45.00: 3.60.
#[rstest::rstest]
#[tokio::test]
async fn s15_an_all_components_shortfall_is_taxed_at_the_default_rate(#[future] test_env: TestEnv) {
    let env = test_env.await;
    tax_resolver(&env, TaxResolverEnum::Manual).await;
    diesel::insert_into(diesel_models::schema::plan_version_adjustment::table)
        .values(&PlanVersionAdjustmentRowNew {
            kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
            plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
            amount_subunit: 5_000,
            includes_all_components: true,
        })
        .execute(&mut env.conn().await)
        .await
        .expect("minimum");
    let ohio = customer(
        &env,
        "US",
        Some("OH"),
        vec![CustomerTaxRate {
            tax_code: "SALES_TAX".to_string(),
            name: "Ohio sales tax".to_string(),
            rate: dec!(0.08),
        }],
    )
    .await;
    let subscription_id = subscription()
        .customer(ohio)
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let first = env.get_invoices(subscription_id).await;
    assert_eq!(first.len(), 1);
    assert!(
        !first[0]
            .line_items
            .iter()
            .any(|line| line.kind == LineItemKind::MinimumShortfall),
        "no period has completed on the first invoice"
    );
    // January's invoice is issued before the credit exists, so February's draws all of it.
    env.process_due_events().await;

    fund_promotional_grant(env.store(), TENANT_ID, ohio, None, Grant::invoice(500)).await;
    fund_grant(env.store(), TENANT_ID, ohio, None, Grant::invoice(1_000)).await;
    env.process_cycles().await;

    let invoices = env.get_invoices(subscription_id).await;
    let invoice = invoices
        .iter()
        .find(|invoice| {
            invoice.billing_period_start == NaiveDate::from_ymd_opt(2024, 2, 1).unwrap()
        })
        .expect("February's invoice");
    let shortfall: Vec<_> = invoice
        .line_items
        .iter()
        .filter(|line| line.kind == LineItemKind::MinimumShortfall)
        .map(|line| {
            (
                line.amount_subtotal,
                line.product_id,
                line.tax_rate,
                line.start_date,
                line.end_date,
            )
        })
        .collect();
    assert_eq!(
        shortfall,
        vec![(
            1_500,
            None,
            dec!(0.08),
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 2, 1).unwrap()
        )]
    );
    assert_eq!(
        (
            invoice.subtotal,
            invoice.promotional_credits,
            invoice.tax_amount,
            invoice.total,
            invoice.applied_credits,
            invoice.amount_due
        ),
        (5_000, 500, 360, 4_860, 1_000, 3_860)
    );
    crate::helpers::credit::assert_ledger_balances(env.store(), TENANT_ID, ohio).await;
}

/// A 100.00 minimum scoped to Starter's Platform Fee (29.00, a 10% category) and Seats (10.00,
/// standard 20% VAT) leaves January 61.00 short on February's invoice. It is split by what each
/// counted, 45.36 and 15.64, each taxed with its product: 10% on 74.36 (7.44), 20% on 25.64 (5.13).
#[rstest::rstest]
#[tokio::test]
async fn a_product_scoped_shortfall_is_taxed_by_its_products(#[future] test_env: TestEnv) {
    use meteroid_store::domain::accounting::{TaxRateNew, TaxRateRule};
    use meteroid_store::repositories::accounting::AccountingInterface;
    use meteroid_store::repositories::products::{ProductInterface, ProductUpdate};
    use meteroid_store::repositories::tax_categories::TaxCategoryInterface;

    let env = test_env.await;
    tax_resolver(&env, TaxResolverEnum::MeteroidEuVat).await;
    let reduced = env
        .store()
        .list_tax_categories(TENANT_ID)
        .await
        .expect("categories")
        .into_iter()
        .find(|category| category.key == "digital_services")
        .expect("built-in category");
    env.store()
        .update_product(
            common_domain::actor::Actor::System,
            ProductUpdate {
                id: PRODUCT_PLATFORM_FEE_ID,
                tenant_id: TENANT_ID,
                name: None,
                description: None,
                fee_type: None,
                fee_structure: None,
                tax_category_id: Some(Some(reduced.id)),
            },
        )
        .await
        .expect("platform fee category");
    env.store()
        .insert_custom_tax(
            TENANT_ID,
            TaxRateNew {
                invoicing_entity_id: INVOICING_ENTITY_ID,
                name: "Reduced rate".to_string(),
                tax_code: "REDUCED_FR".to_string(),
                tax_category_id: Some(reduced.id),
                rules: vec![TaxRateRule {
                    country: Some(CountryCode::from_str("FR").expect("FR")),
                    region: None,
                    rate: dec!(0.10),
                }],
            },
        )
        .await
        .expect("reduced rate");
    let french = customer(&env, "FR", None, vec![]).await;
    let subscription_id = subscription()
        .customer(french)
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let mut conn = env.conn().await;
    let mut details = env
        .store()
        .get_subscription_details_with_conn(&mut conn, TENANT_ID, subscription_id)
        .await
        .expect("details");
    details.minimum = Some(SubscriptionMinimum {
        subscription_id,
        amount_subunit: 10_000,
        scope: MinimumScope::Products(vec![PRODUCT_PLATFORM_FEE_ID, PRODUCT_SEATS_ID]),
        is_override: true,
    });
    // February's renewal: it closes January, billed by the invoice the subscription opened with.
    details.subscription.cycle_index = Some(1);
    let invoice = env
        .services()
        .compute_invoice(
            &NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
            &details,
            None,
            InvoiceType::Recurring,
        )
        .await
        .expect("invoice");

    let mut shortfall: Vec<_> = invoice
        .invoice_lines
        .iter()
        .filter(|line| line.kind == LineItemKind::MinimumShortfall)
        .map(|line| {
            (
                line.product_id,
                line.amount_subtotal,
                line.tax_rate,
                line.tax_amount,
                line.start_date,
            )
        })
        .collect();
    shortfall.sort_by_key(|(_, amount, _, _, _)| -amount);
    let january = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    assert_eq!(
        shortfall,
        vec![
            (
                Some(PRODUCT_PLATFORM_FEE_ID),
                4_536,
                dec!(0.10),
                454,
                january
            ),
            (Some(PRODUCT_SEATS_ID), 1_564, dec!(0.20), 313, january),
        ]
    );
    assert_eq!(
        (invoice.subtotal, invoice.tax_amount, invoice.total),
        (10_000, 744 + 513, 11_257)
    );
}
