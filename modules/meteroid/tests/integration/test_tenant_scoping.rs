//! A tenant must not read or change another tenant's rows by passing their ids.
//!
//! The seeded tenant (`TENANT_ID`) owns the targeted rows; a second tenant created in each test
//! plays the caller.

use chrono::Utc;
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, CustomerId, QuoteId, TenantId};
use error_stack::Report;
use rstest::rstest;

use meteroid_store::domain::accounting::{ProductAccounting, TaxRate, TaxRateNew};
use meteroid_store::domain::coupons::CouponStatusPatch;
use meteroid_store::domain::entitlements::{FeatureNew, FeatureType, FeatureUpdate};
use meteroid_store::domain::enums::{CustomerType, QuoteStatusEnum, TenantEnvironmentEnum};
use meteroid_store::domain::quotes::QuoteNew;
use meteroid_store::domain::tenants::TenantNew;
use meteroid_store::domain::{
    CustomerNew, CustomerTaxStatus, InvoicingEntityProvidersPatch, SubscriptionActivationCondition,
};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::accounting::AccountingInterface;
use meteroid_store::repositories::coupons::{CouponInterface, CouponInterfaceAuto};
use meteroid_store::repositories::entitlements::EntitlementsInterface;
use meteroid_store::repositories::invoicing_entities::{
    InvoicingEntityInterface, InvoicingEntityInterfaceAuto,
};
use meteroid_store::repositories::quotes::QuotesInterface;
use meteroid_store::repositories::{CustomersInterface, TenantInterface};

use crate::data::ids::*;
use crate::harness::{TestEnv, subscription, test_env};

/// A second tenant in the seeded organization, with its default invoicing entity.
async fn other_tenant(env: &TestEnv) -> (TenantId, common_domain::ids::InvoicingEntityId) {
    let tenant = env
        .store()
        .insert_tenant(
            TenantNew {
                name: "Other".to_string(),
                environment: TenantEnvironmentEnum::Development,
                email_delivery_mode: None,
                invoicing_entity: None,
            },
            ORGANIZATION_ID,
        )
        .await
        .expect("other tenant");
    let entity = env
        .store()
        .get_invoicing_entity(tenant.id, None)
        .await
        .expect("other tenant invoicing entity");
    (tenant.id, entity.id)
}

fn is_not_found<T: std::fmt::Debug>(res: &Result<T, Report<StoreError>>) -> bool {
    matches!(
        res.as_ref().map_err(|e| e.current_context()),
        Err(StoreError::ValueNotFound(_))
    )
}

fn is_invalid_argument<T: std::fmt::Debug>(res: &Result<T, Report<StoreError>>) -> bool {
    matches!(
        res.as_ref().map_err(|e| e.current_context()),
        Err(StoreError::InvalidArgument(_))
    )
}

#[rstest]
#[tokio::test]
async fn test_update_tax_rate_of_other_tenant_is_not_found(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let store = env.store();
    let (other, other_entity) = other_tenant(&env).await;

    let victim = store
        .insert_custom_tax(
            TENANT_ID,
            TaxRateNew {
                invoicing_entity_id: INVOICING_ENTITY_ID,
                name: "Victim tax".to_string(),
                tax_code: "VICTIM".to_string(),
                tax_category_id: None,
                rules: vec![],
            },
        )
        .await
        .expect("victim tax");

    let res = store
        .update_custom_tax(
            other,
            TaxRate {
                id: victim.id,
                invoicing_entity_id: other_entity,
                name: "Hijacked".to_string(),
                tax_code: "HIJACKED".to_string(),
                tax_category_id: None,
                rules: vec![],
            },
        )
        .await;
    assert!(is_not_found(&res), "expected NotFound, got {res:?}");

    let taxes = store
        .list_custom_taxes_by_invoicing_entity_id(TENANT_ID, INVOICING_ENTITY_ID)
        .await
        .expect("list victim taxes");
    let unchanged = taxes
        .iter()
        .find(|t| t.id == victim.id)
        .expect("victim tax");
    assert_eq!(unchanged.name, "Victim tax");
    assert_eq!(unchanged.tax_code, "VICTIM");

    // The owner can still update it.
    let updated = store
        .update_custom_tax(
            TENANT_ID,
            TaxRate {
                name: "Renamed".to_string(),
                ..unchanged.clone()
            },
        )
        .await
        .expect("owner update");
    assert_eq!(updated.name, "Renamed");
}

#[rstest]
#[tokio::test]
async fn test_coupon_status_of_other_tenant_is_not_found(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let store = env.store();
    let (other, _) = other_tenant(&env).await;

    let coupon_id = env.create_fixed_coupon("VICTIM", 10, "EUR").await;

    let res = store
        .update_coupon_status(
            Actor::User { id: USER_ID },
            CouponStatusPatch {
                id: coupon_id,
                tenant_id: other,
                archived_at: Some(Some(Utc::now().naive_utc())),
                disabled: Some(true),
            },
        )
        .await;
    assert!(is_not_found(&res), "expected NotFound, got {res:?}");

    let coupon = store
        .get_coupon_by_id(TENANT_ID, coupon_id)
        .await
        .expect("victim coupon");
    assert!(coupon.archived_at.is_none());
    assert!(!coupon.disabled);
}

#[rstest]
#[tokio::test]
async fn test_invoicing_entity_rejects_bank_account_of_other_tenant(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let store = env.store();
    let (other, other_entity) = other_tenant(&env).await;
    crate::data::payment::run_bank_account_seed(env.pool()).await;

    let res = store
        .patch_invoicing_entity_providers(
            Actor::User { id: USER_ID },
            InvoicingEntityProvidersPatch {
                id: other_entity,
                card_provider_id: None,
                direct_debit_provider_id: None,
                bank_account_id: Some(Some(TEST_BANK_ACCOUNT_ID)),
            },
            other,
        )
        .await;
    assert!(res.is_err(), "expected an error, got {res:?}");

    let providers = store
        .resolve_providers_by_id(other, other_entity)
        .await
        .expect("providers");
    assert!(providers.bank_account.is_none());
}

fn quote_new(tenant_id: TenantId, customer_id: CustomerId) -> QuoteNew {
    QuoteNew {
        id: QuoteId::new(),
        status: QuoteStatusEnum::Draft,
        tenant_id,
        customer_id,
        plan_version_id: PLAN_VERSION_NOTION_ID,
        currency: "EUR".to_string(),
        quote_number: "Q-CROSS-0001".to_string(),
        trial_duration_days: None,
        billing_start_date: None,
        billing_end_date: None,
        billing_day_anchor: None,
        activation_condition: SubscriptionActivationCondition::OnStart,
        valid_until: None,
        expires_at: None,
        internal_notes: None,
        cover_image: None,
        overview: None,
        terms_and_services: None,
        net_terms: 30,
        attachments: vec![],
        pdf_document_id: None,
        sharing_key: None,
        recipients: vec![],
        auto_advance_invoices: true,
        charge_automatically: false,
        invoice_memo: None,
        invoice_threshold: None,
        create_subscription_on_acceptance: false,
        payment_methods_config: None,
        custom_properties: serde_json::Value::Null,
        entitlements: vec![],
    }
}

#[rstest]
#[tokio::test]
async fn test_quote_rejects_customer_of_other_tenant(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let store = env.store();
    let (other, _) = other_tenant(&env).await;

    let quote = quote_new(other, CUST_SPOTIFY_ID);
    let quote_id = quote.id;
    let res = store
        .insert_quote_with_details(quote, vec![], vec![], vec![], vec![], Actor::System)
        .await;
    assert!(
        is_invalid_argument(&res),
        "expected InvalidArgument, got {res:?}"
    );

    let res = store.get_quote_by_id(other, quote_id).await;
    assert!(res.is_err(), "the quote must not have been stored");
}

#[rstest]
#[tokio::test]
async fn test_subscription_rejects_plan_version_of_other_tenant(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let (other, _) = other_tenant(&env).await;

    let customer = env
        .store()
        .insert_customer(
            Actor::System,
            CustomerNew {
                name: "Other customer".to_string(),
                customer_type: CustomerType::Company,
                first_name: None,
                last_name: None,
                alias: None,
                billing_email: None,
                invoicing_emails: vec![],
                phone: None,
                currency: "EUR".to_string(),
                billing_address: None,
                shipping_address: None,
                invoicing_entity_id: None,
                force_created_date: None,
                tax_status: CustomerTaxStatus::Taxable,
                exemption_reason: None,
                vat_number: None,
                legal_number: None,
                buyer_reference: None,
                custom_taxes: vec![],
                connected_account_id: None,
                custom_properties: serde_json::Value::Null,
                preferred_locales: vec![],
            },
            other,
        )
        .await
        .expect("other customer");

    let request = subscription()
        .customer(customer.id)
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .manual()
        .request();
    let res = env
        .services()
        .insert_subscription(Actor::User { id: USER_ID }, request, other)
        .await;
    assert!(is_not_found(&res), "expected NotFound, got {res:?}");
}

#[rstest]
#[tokio::test]
async fn test_feature_rejects_product_of_other_tenant(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let store = env.store();
    let (other, _) = other_tenant(&env).await;
    let feature = |product_id| FeatureNew {
        tenant_id: other,
        product_id,
        name: "Feature".to_string(),
        code: "feature".to_string(),
        description: None,
        feature_type: FeatureType::Boolean,
        entitlement: None,
    };

    let res = store
        .create_feature(Actor::System, feature(Some(PRODUCT_SEATS_ID)))
        .await;
    assert!(
        is_invalid_argument(&res),
        "expected InvalidArgument, got {res:?}"
    );

    let created = store
        .create_feature(Actor::System, feature(None))
        .await
        .expect("feature without product");
    let res = store
        .update_feature(
            created.id,
            other,
            FeatureUpdate {
                name: None,
                description: None,
                product_id: Some(Some(PRODUCT_SEATS_ID)),
                config_options: None,
            },
        )
        .await;
    assert!(
        is_invalid_argument(&res),
        "expected InvalidArgument, got {res:?}"
    );

    let unchanged = store.get_feature(created.id, other).await.expect("feature");
    assert!(unchanged.product.is_none());
}

#[rstest]
#[tokio::test]
async fn test_product_accounting_of_other_tenant_is_not_returned(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let store = env.store();
    let (other, _) = other_tenant(&env).await;

    store
        .upsert_product_accounting(
            TENANT_ID,
            ProductAccounting {
                product_id: PRODUCT_SEATS_ID,
                invoicing_entity_id: INVOICING_ENTITY_ID,
                product_code: Some("SEATS".to_string()),
                ledger_account_code: Some("7000".to_string()),
            },
        )
        .await
        .expect("victim product accounting");

    let mut conn = env.conn().await;
    let foreign = store
        .list_product_tax_configuration_by_product_ids_and_invoicing_entity_id_grouped(
            &mut conn,
            other,
            vec![PRODUCT_SEATS_ID],
            INVOICING_ENTITY_ID,
        )
        .await
        .expect("list as other tenant");
    assert!(foreign.is_empty());

    let own = store
        .list_product_tax_configuration_by_product_ids_and_invoicing_entity_id_grouped(
            &mut conn,
            TENANT_ID,
            vec![PRODUCT_SEATS_ID],
            INVOICING_ENTITY_ID,
        )
        .await
        .expect("list as owner");
    assert_eq!(own.len(), 1);
}
