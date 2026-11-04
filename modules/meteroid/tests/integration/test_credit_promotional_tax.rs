//! Promotional credit under the EU VAT engine: a line the credit takes whole keeps its rate, so it
//! is taxed again when the credit is gone; exempt and reverse-charge customers owe no tax either way.
use crate::data::ids::{INVOICING_ENTITY_ID, PLAN_VERSION_1_LEETCODE_ID, TENANT_ID};
use crate::helpers::credit::{Grant, fund_promotional_grant};
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};
use chrono::NaiveDate;
use common_domain::actor::Actor;
use common_domain::country::CountryCode;
use common_domain::ids::{CreditGrantId, CustomerId};
use diesel_models::enums::CustomerTaxStatusEnum;
use meteroid_store::domain::enums::{InvoicePaymentStatus, InvoiceStatusEnum, InvoiceType};
use meteroid_store::domain::{
    CreateSubscription, InvoiceNew, InvoicingEntityPatch, SubscriptionActivationCondition,
    SubscriptionNew,
};
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::credit::customer_credit_wallets::DebitCustomerCreditGrant;
use meteroid_store::repositories::customers::CustomersInterfaceAuto;
use meteroid_store::repositories::invoicing_entities::{
    InvoicingEntityInterface, InvoicingEntityInterfaceAuto,
};
use meteroid_store::repositories::{InvoiceInterface, SubscriptionInterface};
use meteroid_store::services::invoice_lines::invoice_lines::ComputedInvoiceContent;
use rust_decimal_macros::dec;
use std::str::FromStr;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn test_error(error: impl std::fmt::Debug) -> Box<dyn std::error::Error> {
    format!("{error:?}").into()
}

async fn setup() -> TestResult<MeteroidSetup> {
    crate::helpers::init::logging();
    let database =
        crate::meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;
    let setup = crate::meteroid_it::container::start_meteroid(database, SeedLevel::PLANS).await;
    setup
        .store
        .patch_invoicing_entity(
            Actor::System,
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                tax_resolver: Some(meteroid_store::domain::enums::TaxResolverEnum::MeteroidEuVat),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .map_err(test_error)?;
    Ok(setup)
}

async fn customer(
    setup: &MeteroidSetup,
    country: &str,
    vat_number: &str,
    tax_status: CustomerTaxStatusEnum,
) -> TestResult<CustomerId> {
    use diesel_models::customers::CustomerRowNew;
    use meteroid_store::domain::Address;
    let id = CustomerId::from_proto(uuid::Uuid::new_v4().to_string()).map_err(test_error)?;
    let address = Address {
        line1: Some("1 Rue de la Paix".to_string()),
        line2: None,
        city: Some("Paris".to_string()),
        country: Some(CountryCode::from_str(country).map_err(test_error)?),
        state: None,
        zip_code: Some("75002".to_string()),
    };
    CustomerRowNew {
        id,
        name: format!("{country} business"),
        created_at: Some(chrono::Utc::now().naive_utc()),
        tenant_id: TENANT_ID,
        alias: None,
        currency: "EUR".to_string(),
        invoicing_entity_id: INVOICING_ENTITY_ID,
        billing_address: Some(serde_json::to_value(&address)?),
        shipping_address: None,
        billing_email: None,
        current_payment_method_id: None,
        vat_number: Some(vat_number.to_string()),
        legal_number: None,
        buyer_reference: None,
        custom_taxes: serde_json::json!([]),
        invoicing_emails: vec![],
        phone: None,
        tax_status,
        exemption_reason: None,
        vat_number_format_valid: true,
        connected_account_id: None,
        vat_number_validation_status: None,
        custom_properties: serde_json::Value::Null,
        preferred_locales: vec![],
        customer_type: diesel_models::enums::CustomerTypeEnum::Company,
        first_name: None,
        last_name: None,
    }
    .insert(&mut setup.store.get_conn().await.map_err(test_error)?)
    .await
    .map_err(test_error)?;
    Ok(id)
}

/// The LeetCode plan's second 35.00 period, computed as its draft is, once `promotional` credit
/// is granted (after the first period's invoice, which would draw it).
async fn period_content(
    setup: &MeteroidSetup,
    customer_id: CustomerId,
    promotional: i64,
) -> TestResult<(ComputedInvoiceContent, CreditGrantId)> {
    let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let subscription_id = setup
        .services
        .insert_subscription(
            Actor::System,
            CreateSubscription {
                subscription: SubscriptionNew {
                    customer_id,
                    plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
                    net_terms: None,
                    invoice_memo: None,
                    invoice_threshold: None,
                    start_date: start,
                    end_date: None,
                    billing_start_date: Some(start),
                    activation_condition: SubscriptionActivationCondition::OnStart,
                    trial_duration: None,
                    billing_day_anchor: None,
                    payment_methods_config: None,
                    auto_advance_invoices: true,
                    charge_automatically: false,
                    purchase_order: None,
                    backdate_invoices: false,
                    skip_checkout_session: false,
                    skip_past_invoices: false,
                    usage_filter: None,
                    custom_properties: serde_json::Value::Null,
                },
                price_components: None,
                add_ons: None,
                coupons: None,
                entitlements: vec![],
                minimum_override: None,
                drop_minimum: false,
            },
            TENANT_ID,
        )
        .await
        .map_err(test_error)?
        .id;
    let details = setup
        .store
        .get_subscription_details_with_conn(
            &mut setup.store.get_conn().await.map_err(test_error)?,
            TENANT_ID,
            subscription_id,
        )
        .await
        .map_err(test_error)?;
    let grant = fund_promotional_grant(
        &setup.store,
        TENANT_ID,
        customer_id,
        None,
        Grant::invoice(promotional),
    )
    .await;
    let content = setup
        .services
        .compute_invoice(
            &NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
            &details,
            None,
            InvoiceType::Recurring,
        )
        .await
        .map_err(test_error)?;
    Ok((content, grant))
}

#[tokio::test]
async fn a_line_the_credit_took_whole_is_taxed_again_once_the_credit_is_gone() -> TestResult {
    let setup = setup().await?;
    let customer_id = customer(
        &setup,
        "FR",
        "FR12345678901",
        CustomerTaxStatusEnum::Taxable,
    )
    .await?;
    let (content, promo) = period_content(&setup, customer_id, 3_500).await?;
    assert_eq!(content.promotional_credits, 3_500);
    assert_eq!((content.tax_amount, content.total), (0, 0));
    let line = &content.invoice_lines[0];
    assert_eq!(line.taxable_amount, 0);
    assert_eq!(
        line.tax_rate,
        dec!(0.2),
        "the rate is kept on a line taken whole"
    );
    assert!(!line.tax_details.is_empty());

    // The draft as computed, then its promotional credit is spent elsewhere.
    let stored = setup
        .store
        .find_customer_by_id(customer_id, TENANT_ID)
        .await
        .map_err(test_error)?;
    let entity = setup
        .store
        .get_invoicing_entity(TENANT_ID, Some(INVOICING_ENTITY_ID))
        .await
        .map_err(test_error)?;
    let today = chrono::Utc::now().date_naive();
    let draft = setup
        .store
        .insert_invoice(InvoiceNew {
            status: InvoiceStatusEnum::Draft,
            tenant_id: TENANT_ID,
            customer_id,
            subscription_id: None,
            cycle_index: None,
            currency: "EUR".to_owned(),
            invoice_number: "draft".to_owned(),
            line_items: content.invoice_lines.clone(),
            coupons: Vec::new(),
            data_updated_at: None,
            invoice_date: today,
            billing_period_start: today,
            tax_inclusive: false,
            plan_version_id: None,
            invoice_type: InvoiceType::OneOff,
            finalized_at: None,
            subtotal: content.subtotal,
            subtotal_recurring: content.subtotal_recurring,
            discount: content.discount,
            tax_amount: content.tax_amount,
            total: content.total,
            amount_due: content.amount_due,
            applied_credits: 0,
            promotional_credits: content.promotional_credits,
            net_terms: 0,
            reference: None,
            purchase_order: None,
            memo: None,
            due_at: None,
            plan_name: None,
            customer_details: stored.into(),
            seller_details: entity.into(),
            auto_advance: false,
            payment_status: InvoicePaymentStatus::Unpaid,
            tax_breakdown: content.tax_breakdown.clone(),
            manual: true,
            invoicing_entity_id: INVOICING_ENTITY_ID,
            parent_invoice_id: None,
            consolidated_into_invoice_id: None,
            annexes: Vec::new(),
            custom_properties: serde_json::Value::Null,
        })
        .await
        .map_err(test_error)?;
    setup
        .store
        .debit_customer_credit_grant(DebitCustomerCreditGrant {
            tenant_id: TENANT_ID,
            customer_id,
            grant_id: promo,
            amount_minor: None,
            idempotency_key: uuid::Uuid::now_v7(),
            reason: "Spent elsewhere".to_owned(),
            actor_id: None,
        })
        .await
        .map_err(test_error)?;

    let finalized = setup
        .services
        .finalize_invoice(Actor::System, draft.id, TENANT_ID)
        .await
        .map_err(test_error)?
        .invoice;
    assert_eq!(finalized.promotional_credits, 0);
    assert_eq!(finalized.tax_amount, 700, "20% of 35.00");
    assert_eq!(finalized.total, 4_200);
    assert_eq!(finalized.amount_due, 4_200);
    assert_eq!(finalized.line_items[0].tax_details[0].tax_amount, 700);
    Ok(())
}

#[tokio::test]
async fn exempt_and_reverse_charge_customers_owe_no_tax_on_what_promotion_leaves() -> TestResult {
    let setup = setup().await?;
    for (country, vat_number, status) in [
        ("DE", "DE123456789", CustomerTaxStatusEnum::Taxable),
        ("FR", "FR12345678901", CustomerTaxStatusEnum::Exempt),
    ] {
        let customer_id = customer(&setup, country, vat_number, status).await?;
        let (content, _) = period_content(&setup, customer_id, 1_000).await?;
        assert_eq!(content.promotional_credits, 1_000, "{country}");
        assert_eq!(content.tax_amount, 0, "{country}");
        assert_eq!(content.total, 2_500, "{country}");
        assert_eq!(
            content.subtotal - content.discount - content.promotional_credits + content.tax_amount,
            content.total
        );
    }
    Ok(())
}

/// A period promotional credit pays whole still states its tax position: the French customer's
/// 20% at nothing, the German business's reverse charge with its exemption.
#[tokio::test]
async fn a_period_paid_whole_by_promotion_keeps_its_tax_position() -> TestResult {
    use meteroid_store::domain::TaxExemptionType;
    let setup = setup().await?;
    for (country, vat_number, exemption) in [
        ("FR", "FR12345678901", None),
        ("DE", "DE123456789", Some(TaxExemptionType::ReverseCharge)),
    ] {
        let customer_id =
            customer(&setup, country, vat_number, CustomerTaxStatusEnum::Taxable).await?;
        let (content, _) = period_content(&setup, customer_id, 3_500).await?;
        assert_eq!((content.total, content.tax_amount), (0, 0), "{country}");
        let row = content
            .tax_breakdown
            .first()
            .unwrap_or_else(|| panic!("{country}: the breakdown states the tax position"));
        assert_eq!(row.exemption_type, exemption, "{country}");
        assert_eq!((row.taxable_amount, row.tax_amount), (0, 0), "{country}");
        if exemption.is_none() {
            assert_eq!(row.tax_rate, dec!(0.2));
        }
    }
    Ok(())
}

/// T10: on a parent invoice taxed at 20%, A's paid pack, scoped to A, pays A's usage with its tax
/// (5,000 + 1,000) and nothing of B's (3,000 + 600).
#[tokio::test]
async fn scoped_paid_credit_pays_its_own_lines_tax() -> TestResult {
    use crate::data::ids::{CUST_UBER_ID, METRIC_DATABASE_SIZE};
    use crate::test_credit_promotional::{taxed, taxed_draft};
    use crate::test_customer_credit::finalize;
    use meteroid_store::domain::LineItem;
    use meteroid_store::domain::credit::{CreditApplication, GrantClassification, GrantScope};
    use meteroid_store::repositories::credit::customer_credit_wallets::FundCustomerCreditWallet;
    crate::helpers::init::logging();
    let database =
        crate::meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;
    let setup = crate::meteroid_it::container::start_meteroid(database, SeedLevel::PLANS).await;
    let live = || {
        crate::harness::subscription()
            .start_date(chrono::Utc::now().date_naive())
            .on_start()
            .no_trial()
            .create(&setup.services)
    };
    let (a, b) = (live().await, live().await);
    let wallet = setup
        .store
        .create_customer_credit_wallet(TENANT_ID, CUST_UBER_ID, "EUR")
        .await
        .map_err(test_error)?;
    let pack = setup
        .store
        .fund_scoped_customer_credit_wallet(
            FundCustomerCreditWallet {
                tenant_id: TENANT_ID,
                customer_id: CUST_UBER_ID,
                wallet_id: wallet.id,
                amount_minor: 10_000,
                classification: GrantClassification::Paid,
                priority: 100,
                expires_at: None,
                idempotency_key: uuid::Uuid::now_v7(),
                reason: "A's paid pack".to_owned(),
                reference: None,
                actor_id: None,
                cost: None,
            },
            GrantScope {
                subscription_id: Some(a),
                application: CreditApplication::Usage,
                product_ids: Vec::new(),
            },
        )
        .await
        .map_err(test_error)?
        .grant_id;
    let usage_of = |subscription_id, name: &str, net: i64| LineItem {
        subscription_id: Some(subscription_id),
        metric_id: Some(METRIC_DATABASE_SIZE),
        ..taxed(name, net)
    };
    let parent = taxed_draft(
        &setup,
        vec![usage_of(a, "Usage A", 5_000), usage_of(b, "Usage B", 3_000)],
        false,
    )
    .await
    .map_err(test_error)?;
    let parent = finalize(&setup, parent.id)
        .await
        .map_err(test_error)?
        .invoice;
    assert_eq!(parent.tax_amount, 1_600);
    assert_eq!(parent.applied_credits, 6_000, "A's usage and its tax");
    assert_eq!(parent.amount_due, 3_600, "B's usage and its tax");
    let remaining =
        crate::helpers::credit::grant_remaining(&setup.store, TENANT_ID, CUST_UBER_ID, pack).await;
    assert_eq!(remaining, 4_000);
    Ok(())
}
