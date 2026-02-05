//! Payment methods configuration tests.
//!
//! These tests verify that the `payment_methods_config` on a subscription correctly
//! determines what checkout options are available when a customer pays an invoice.
//!
//! Key concept: When a customer lands on the pay_online page for an invoice,
//! the system calls `resolve_subscription_payment_methods()` to determine which
//! payment options (card, direct debit, bank transfer) are available.
//!
//! Tests cover:
//! - Inherit vs Override config resolution
//! - Config changes affecting resolved checkout options
//! - External subscription (all online payments disabled)
//! - Auto-charge behavior with and without payment methods on file

use chrono::NaiveDate;
use rstest::rstest;

use crate::data::ids::*;
use crate::harness::{InvoicesAssertExt, SubscriptionAssertExt, TestEnv, subscription, test_env};
use meteroid_store::domain::SubscriptionPatch;
use meteroid_store::domain::subscriptions::PaymentMethodsConfig;
use meteroid_store::repositories::SubscriptionInterface;
use meteroid_store::repositories::customers::CustomersInterfaceAuto;

// =============================================================================
// PAYMENT METHOD RESOLUTION: INHERIT VS OVERRIDE
// =============================================================================

/// Verify that payment method resolution respects the subscription's config.
///
/// This test creates subscriptions with different configs and verifies that
/// `resolve_subscription_payment_methods()` returns the correct checkout options.
///
/// - Inherit config: Uses invoicing entity's providers (card available)
/// - External config: No online payment methods available
/// - Card-only config: Only card available, no direct debit
#[rstest]
#[tokio::test]
async fn test_resolution_inherit_vs_override_vs_external(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    // Get customer for resolution tests
    let customer = env
        .store()
        .find_customer_by_id(CUST_UBER_ID, TENANT_ID)
        .await
        .expect("Failed to get customer");

    // === Test 1: Inherit (None) - should use invoicing entity's card provider ===
    let resolved_inherit = env
        .services()
        .resolve_subscription_payment_methods(TENANT_ID, None, &customer)
        .await
        .expect("Failed to resolve inherit config");

    assert!(
        resolved_inherit.card_connection_id.is_some(),
        "Inherit should resolve card connection from invoicing entity"
    );
    assert!(
        resolved_inherit.card_enabled,
        "Inherit should have card enabled"
    );

    // === Test 2: External (all disabled) - should have no online payment ===
    let external_config = PaymentMethodsConfig::external();
    let resolved_external = env
        .services()
        .resolve_subscription_payment_methods(TENANT_ID, Some(&external_config), &customer)
        .await
        .expect("Failed to resolve external config");

    assert!(
        resolved_external.card_connection_id.is_none(),
        "External should have no card connection"
    );
    assert!(
        resolved_external.direct_debit_connection_id.is_none(),
        "External should have no DD connection"
    );
    assert!(
        !resolved_external.card_enabled,
        "External should have card disabled"
    );
    assert!(
        !resolved_external.direct_debit_enabled,
        "External should have DD disabled"
    );
    assert!(
        !resolved_external.has_online_payment(),
        "External should have no online payment options"
    );

    // === Test 3: Card-only online - should have card but not DD ===
    let card_only_config = PaymentMethodsConfig::online_specific(true, false);
    let resolved_card = env
        .services()
        .resolve_subscription_payment_methods(TENANT_ID, Some(&card_only_config), &customer)
        .await
        .expect("Failed to resolve card-only config");

    assert!(
        resolved_card.card_connection_id.is_some(),
        "Card-only should have card connection"
    );
    assert!(
        resolved_card.card_enabled,
        "Card-only should have card enabled"
    );
    assert!(
        !resolved_card.direct_debit_enabled,
        "Card-only should have DD disabled"
    );
    assert!(
        resolved_card.has_online_payment(),
        "Card-only should have online payment available"
    );
}

// =============================================================================
// SUBSCRIPTION CONFIG CHANGE AFFECTS RESOLUTION
// =============================================================================

/// Test that changing a subscription's payment_methods_config affects what
/// checkout options are resolved for that subscription's invoices.
///
/// This simulates the real-world scenario:
/// 1. Merchant creates subscription with Inherit (card available)
/// 2. Customer's invoices have card checkout option
/// 3. Merchant changes subscription to External (manual payment)
/// 4. Customer's new invoices have NO card checkout option
/// 5. Merchant re-enables card payments
/// 6. Customer's invoices have card checkout option again
#[rstest]
#[tokio::test]
async fn test_config_change_affects_resolved_checkout_options(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    // Get customer for resolution tests
    let customer = env
        .store()
        .find_customer_by_id(CUST_UBER_ID, TENANT_ID)
        .await
        .expect("Failed to get customer");

    // === Phase 1: Create subscription with Inherit (default - card available) ===
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .no_auto_charge()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active();

    // Resolve payment methods - should have card available
    let config_1: Option<PaymentMethodsConfig> = sub
        .payment_methods_config
        .as_ref()
        .map(|v| serde_json::from_value(v.clone()).expect("parse config"));
    let resolved_1 = env
        .services()
        .resolve_subscription_payment_methods(TENANT_ID, config_1.as_ref(), &customer)
        .await
        .expect("Failed to resolve phase 1");

    assert!(
        resolved_1.card_connection_id.is_some(),
        "Phase 1: Customer should see card checkout option"
    );

    // === Phase 2: Change subscription to External (no online payment) ===
    let patch = SubscriptionPatch {
        id: sub_id,
        charge_automatically: None,
        auto_advance_invoices: None,
        net_terms: None,
        invoice_memo: None,
        purchase_order: None,
        payment_methods_config: Some(Some(PaymentMethodsConfig::external())),
    };

    let updated_sub = env
        .store()
        .patch_subscription(TENANT_ID, patch)
        .await
        .expect("Failed to patch subscription to external");

    // Verify config changed
    assert!(
        matches!(
            updated_sub.payment_methods_config,
            Some(PaymentMethodsConfig::External)
        ),
        "Config should now be external"
    );

    // Resolve payment methods with new config - should have NO card
    let resolved_2 = env
        .services()
        .resolve_subscription_payment_methods(
            TENANT_ID,
            updated_sub.payment_methods_config.as_ref(),
            &customer,
        )
        .await
        .expect("Failed to resolve phase 2");

    assert!(
        resolved_2.card_connection_id.is_none(),
        "Phase 2: After external config, customer should NOT see card checkout option"
    );
    assert!(
        !resolved_2.has_online_payment(),
        "Phase 2: No online payment should be available"
    );

    // === Phase 3: Re-enable card payments ===
    let patch = SubscriptionPatch {
        id: sub_id,
        charge_automatically: None,
        auto_advance_invoices: None,
        net_terms: None,
        invoice_memo: None,
        purchase_order: None,
        payment_methods_config: Some(Some(PaymentMethodsConfig::online_specific(true, false))),
    };

    let updated_sub = env
        .store()
        .patch_subscription(TENANT_ID, patch)
        .await
        .expect("Failed to re-enable card");

    // Resolve payment methods - should have card again
    let resolved_3 = env
        .services()
        .resolve_subscription_payment_methods(
            TENANT_ID,
            updated_sub.payment_methods_config.as_ref(),
            &customer,
        )
        .await
        .expect("Failed to resolve phase 3");

    assert!(
        resolved_3.card_connection_id.is_some(),
        "Phase 3: After re-enabling card, customer should see card checkout option"
    );
    assert!(resolved_3.card_enabled, "Phase 3: Card should be enabled");
}

// =============================================================================
// EXTERNAL SUBSCRIPTION: NO ONLINE CHECKOUT, INVOICES FINALIZED UNPAID
// =============================================================================

/// Test that creating a subscription with External config means:
/// 1. No online payment checkout options are available
/// 2. Invoices are finalized but remain unpaid (trust-based billing)
/// 3. Customer cannot pay online (they'd pay via bank transfer or other means)
#[rstest]
#[tokio::test]
async fn test_external_subscription_no_online_checkout(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    // Create External subscription (all online payments disabled)
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID) // $35/month
        .start_date(start_date)
        .on_start()
        .no_trial()
        .no_auto_charge()
        .external_payment() // Key: external payment config
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active().has_cycle_index(0);

    // Get customer
    let customer = env
        .store()
        .find_customer_by_id(CUST_UBER_ID, TENANT_ID)
        .await
        .expect("Failed to get customer");

    // Resolve payment methods - should have NO online options
    let config: Option<PaymentMethodsConfig> = sub
        .payment_methods_config
        .as_ref()
        .map(|v| serde_json::from_value(v.clone()).expect("parse config"));
    let resolved = env
        .services()
        .resolve_subscription_payment_methods(TENANT_ID, config.as_ref(), &customer)
        .await
        .expect("Failed to resolve");

    assert!(
        !resolved.has_online_payment(),
        "External subscription should have no online payment"
    );
    assert!(!resolved.card_enabled, "Card should be disabled");
    assert!(
        !resolved.direct_debit_enabled,
        "Direct debit should be disabled"
    );
    assert!(
        !resolved.bank_transfer_enabled,
        "Bank transfer should be disabled (not configured)"
    );

    // Invoice should be created but unpaid (trust-based billing)
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .is_finalized_unpaid()
        .has_total(3500);

    // Process renewal - more invoices, still unpaid
    env.process_cycles().await;

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);
    invoices.assert().invoice_at(1).is_finalized_unpaid();
}

// =============================================================================
// MIGRATE EXTERNAL → CARD: ENABLE ONLINE PAYMENTS FOR EXISTING SUBSCRIPTION
// =============================================================================

/// Test the migration scenario: subscription started as External, merchant
/// wants to enable card payments so customer can pay outstanding invoices online.
///
/// Flow:
/// 1. Create External subscription - no online checkout
/// 2. Invoice is finalized unpaid
/// 3. Merchant updates subscription to enable card
/// 4. NOW customer can pay via online checkout
/// 5. Customer pays, invoice marked paid
#[rstest]
#[tokio::test]
async fn test_migrate_external_to_card_enables_checkout(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    // Get customer
    let customer = env
        .store()
        .find_customer_by_id(CUST_UBER_ID, TENANT_ID)
        .await
        .expect("Failed to get customer");

    // === Phase 1: Create External subscription ===
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID) // $35/month
        .start_date(start_date)
        .on_start()
        .no_trial()
        .no_auto_charge()
        .external_payment()
        .create(env.services())
        .await;

    // Verify no online checkout available
    let sub = env.get_subscription(sub_id).await;
    let config: Option<PaymentMethodsConfig> = sub
        .payment_methods_config
        .as_ref()
        .map(|v| serde_json::from_value(v.clone()).expect("parse"));
    let resolved_before = env
        .services()
        .resolve_subscription_payment_methods(TENANT_ID, config.as_ref(), &customer)
        .await
        .expect("resolve before");

    assert!(
        !resolved_before.has_online_payment(),
        "Before migration: No online checkout should be available"
    );

    // Invoice created, finalized unpaid
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices.assert().invoice_at(0).is_finalized_unpaid();

    // === Phase 2: Merchant enables card payments ===
    let patch = SubscriptionPatch {
        id: sub_id,
        charge_automatically: None,
        auto_advance_invoices: None,
        net_terms: None,
        invoice_memo: None,
        purchase_order: None,
        payment_methods_config: Some(Some(PaymentMethodsConfig::online_specific(true, false))),
    };

    let updated_sub = env
        .store()
        .patch_subscription(TENANT_ID, patch)
        .await
        .expect("Failed to enable card");

    // Verify card checkout NOW available
    let resolved_after = env
        .services()
        .resolve_subscription_payment_methods(
            TENANT_ID,
            updated_sub.payment_methods_config.as_ref(),
            &customer,
        )
        .await
        .expect("resolve after");

    assert!(
        resolved_after.card_connection_id.is_some(),
        "After enabling card: Customer should have card checkout option"
    );
    assert!(resolved_after.card_enabled, "Card should be enabled");

    // === Phase 3: Customer pays via card checkout ===
    let invoice_id = invoices[0].id;
    env.services()
        .complete_invoice_payment(TENANT_ID, invoice_id, CUST_UBER_PAYMENT_METHOD_ID)
        .await
        .expect("Failed to pay invoice");
    env.run_outbox_and_orchestration().await;

    // Invoice should now be paid
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().invoice_at(0).is_finalized_paid();
}

// =============================================================================
// CHARGE AUTOMATICALLY WITHOUT CARD ON FILE
// =============================================================================

/// Test that charge_automatically=true without a card on file:
/// 1. Invoice is finalized but unpaid (can't auto-charge without PM)
/// 2. Customer can still pay via pay_online link
/// 3. After customer adds card and pays, future renewals have card available
#[rstest]
#[tokio::test]
async fn test_charge_auto_without_card_invoice_unpaid(#[future] test_env: TestEnv) {
    let env = test_env.await;
    // Seed provider but NOT customer payment methods
    env.seed_mock_payment_provider(false).await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    // Create subscription with charge_auto (customer has no card yet)
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID) // $35/month
        .start_date(start_date)
        .on_start()
        .no_trial()
        .auto_charge() // Want to auto-charge
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active().has_cycle_index(0);

    // Invoice finalized but unpaid (no card to charge)
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .is_finalized_unpaid()
        .has_total(3500);

    // Process renewal - still unpaid
    env.process_cycles().await;

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);
    invoices.assert().invoice_at(1).is_finalized_unpaid();

    // Get customer
    let customer = env
        .store()
        .find_customer_by_id(CUST_UBER_ID, TENANT_ID)
        .await
        .expect("Failed to get customer");

    // Customer should still have card checkout available (subscription has connection)
    let sub = env.get_subscription(sub_id).await;
    let config: Option<PaymentMethodsConfig> = sub
        .payment_methods_config
        .as_ref()
        .map(|v| serde_json::from_value(v.clone()).expect("parse"));
    let resolved = env
        .services()
        .resolve_subscription_payment_methods(TENANT_ID, config.as_ref(), &customer)
        .await
        .expect("resolve");

    assert!(
        resolved.card_connection_id.is_some(),
        "Even without card on file, checkout option should be available"
    );

    // Customer adds card and pays
    env.seed_customer_payment_methods().await;

    // Pay both invoices
    for invoice in &invoices {
        env.services()
            .complete_invoice_payment(TENANT_ID, invoice.id, CUST_UBER_PAYMENT_METHOD_ID)
            .await
            .expect("Failed to pay");
    }
    env.run_outbox_and_orchestration().await;

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().invoice_at(0).is_finalized_paid();
    invoices.assert().invoice_at(1).is_finalized_paid();
}

// =============================================================================
// INHERIT CONFIG USES INVOICING ENTITY'S CURRENT PROVIDERS
// =============================================================================

/// Test that Inherit config resolves to the invoicing entity's current providers.
/// This verifies that if the invoicing entity's provider changes, Inherit
/// subscriptions automatically use the new provider.
///
/// Note: This test verifies the Inherit behavior. Changing the invoicing entity's
/// provider mid-subscription would be tested separately.
#[rstest]
#[tokio::test]
async fn test_inherit_uses_invoicing_entity_providers(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    // Create subscription with implicit Inherit (no payment_methods_config specified)
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .no_auto_charge()
        // Note: no .payment_methods_config() or .external_payment() called
        .create(env.services())
        .await;

    let _sub = env.get_subscription(sub_id).await;

    // The subscription inherits payment config from invoicing entity which
    // is set up by seed_payments -> seed_mock_payment_provider to use
    // MOCK_CONNECTOR_ID for cards

    // Get customer and verify resolution
    let customer = env
        .store()
        .find_customer_by_id(CUST_UBER_ID, TENANT_ID)
        .await
        .expect("customer");

    let resolved = env
        .services()
        .resolve_subscription_payment_methods(TENANT_ID, None, &customer) // None = Inherit
        .await
        .expect("resolve");

    assert!(
        resolved.card_connection_id.is_some(),
        "Inherit should resolve card from invoicing entity"
    );
    assert!(resolved.card_enabled);

    // Invoice created with card checkout available
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
}

// =============================================================================
// CHARGE AUTOMATICALLY VALIDATION: INVALID CONFIGURATIONS
// =============================================================================

/// Test that patching a subscription to set charge_automatically=true
/// fails when payment_methods_config is External (not Online).
///
/// Rule: charge_automatically requires payment_methods_config to be Online
#[rstest]
#[tokio::test]
async fn test_charge_auto_validation_rejects_external_config(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    // Create subscription with External payment config
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .no_auto_charge()
        .external_payment()
        .create(env.services())
        .await;

    // Try to patch to enable charge_automatically - should fail
    let patch = SubscriptionPatch {
        id: sub_id,
        charge_automatically: Some(true), // Try to enable
        auto_advance_invoices: None,
        net_terms: None,
        invoice_memo: None,
        purchase_order: None,
        payment_methods_config: None, // Keep External
    };

    let result = env.store().patch_subscription(TENANT_ID, patch).await;

    assert!(
        result.is_err(),
        "Should reject charge_automatically=true when payment_methods_config is External"
    );

    let err_msg = format!("{:?}", result.unwrap_err());
    assert!(
        err_msg.contains("payment_methods_config")
            || err_msg.contains("charge_automatically")
            || err_msg.contains("Online"),
        "Error should mention payment_methods_config or charge_automatically: {}",
        err_msg
    );
}

/// Test that patching a subscription to set charge_automatically=true
/// fails when the invoicing entity has no payment provider configured.
///
/// Rule: charge_automatically requires the invoicing entity to have
/// a card or direct debit provider
#[rstest]
#[tokio::test]
async fn test_charge_auto_validation_rejects_no_provider(#[future] test_env: TestEnv) {
    let env = test_env.await;
    // Note: NOT calling seed_payments(), so no payment provider is configured

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    // Create subscription without auto-charge (should succeed without provider)
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .no_auto_charge() // No auto-charge initially
        .create(env.services())
        .await;

    // Try to patch to enable charge_automatically - should fail (no provider)
    let patch = SubscriptionPatch {
        id: sub_id,
        charge_automatically: Some(true), // Try to enable
        auto_advance_invoices: None,
        net_terms: None,
        invoice_memo: None,
        purchase_order: None,
        payment_methods_config: None, // Keep default (Online)
    };

    let result = env.store().patch_subscription(TENANT_ID, patch).await;

    assert!(
        result.is_err(),
        "Should reject charge_automatically=true when no payment provider is configured"
    );

    let err_msg = format!("{:?}", result.unwrap_err());
    assert!(
        err_msg.contains("payment provider") || err_msg.contains("invoicing entity"),
        "Error should mention payment provider or invoicing entity: {}",
        err_msg
    );
}

/// Test that patching a subscription's payment_methods_config from Online to External
/// fails if charge_automatically is already true.
///
/// Rule: cannot switch to External while charge_automatically=true
#[rstest]
#[tokio::test]
async fn test_charge_auto_validation_rejects_config_change_to_external(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.seed_payments().await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    // Create subscription WITH auto-charge (Online config is default)
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .auto_charge() // Auto-charge enabled
        .create(env.services())
        .await;

    // Verify subscription has charge_automatically=true
    let sub = env.get_subscription(sub_id).await;
    assert!(
        sub.charge_automatically,
        "Subscription should have charge_automatically=true"
    );

    // Try to patch payment_methods_config to External while keeping charge_auto=true
    let patch = SubscriptionPatch {
        id: sub_id,
        charge_automatically: None, // Keep existing (true)
        auto_advance_invoices: None,
        net_terms: None,
        invoice_memo: None,
        purchase_order: None,
        payment_methods_config: Some(Some(PaymentMethodsConfig::external())), // Try to change to External
    };

    let result = env.store().patch_subscription(TENANT_ID, patch).await;

    assert!(
        result.is_err(),
        "Should reject changing to External while charge_automatically=true"
    );
}

/// Test that setting charge_automatically=true with Online config succeeds
/// when the invoicing entity has a payment provider configured.
#[rstest]
#[tokio::test]
async fn test_charge_auto_validation_accepts_valid_config(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    // Create subscription without auto-charge initially
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .no_auto_charge()
        .create(env.services())
        .await;

    // Patch to enable charge_automatically with Online config - should succeed
    let patch = SubscriptionPatch {
        id: sub_id,
        charge_automatically: Some(true),
        auto_advance_invoices: None,
        net_terms: None,
        invoice_memo: None,
        purchase_order: None,
        payment_methods_config: Some(Some(PaymentMethodsConfig::online())), // Explicit Online
    };

    let result = env.store().patch_subscription(TENANT_ID, patch).await;

    assert!(
        result.is_ok(),
        "Should accept charge_automatically=true with Online config and provider: {:?}",
        result.err()
    );

    let updated = result.unwrap();
    assert!(updated.charge_automatically);
}

// =============================================================================
// OVERRIDE CONFIG: CARD ENABLED VS DISABLED
// =============================================================================

/// Test that Override config correctly enables/disables payment methods.
/// The invoicing entity has the provider configured, but Override controls
/// whether each method is enabled.
#[rstest]
#[tokio::test]
async fn test_override_enables_disables_methods(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let customer = env
        .store()
        .find_customer_by_id(CUST_UBER_ID, TENANT_ID)
        .await
        .expect("customer");

    // Test with card enabled - should resolve to invoicing entity's provider
    let config_enabled = PaymentMethodsConfig::online_specific(true, false);

    let resolved_enabled = env
        .services()
        .resolve_subscription_payment_methods(TENANT_ID, Some(&config_enabled), &customer)
        .await
        .expect("resolve enabled config");

    assert!(
        resolved_enabled.card_connection_id.is_some(),
        "Card enabled should resolve card connection from invoicing entity"
    );
    assert!(resolved_enabled.card_enabled);

    // Test with card disabled (online but both card and DD disabled) - should have no card connection
    let config_disabled = PaymentMethodsConfig::online_specific(false, false);

    let resolved_disabled = env
        .services()
        .resolve_subscription_payment_methods(TENANT_ID, Some(&config_disabled), &customer)
        .await
        .expect("resolve disabled config");

    assert!(
        resolved_disabled.card_connection_id.is_none(),
        "Card disabled should result in no card connection"
    );
    assert!(!resolved_disabled.card_enabled);
}

// =============================================================================
// has_resolved_payment_method ASSERTION: TRUE AND FALSE CASES
// =============================================================================

/// Test the `has_resolved_payment_method` assertion helper with both true and false cases.
///
/// This verifies that the assertion correctly uses the payment resolution service
/// to check if a subscription has the correct payment methods available based on
/// its config and the invoicing entity's providers.
#[rstest]
#[tokio::test]
async fn test_has_resolved_payment_method_assertion(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    // === Test 1: Online config (default) with provider → should have payment method ===
    let sub_id_online = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .no_auto_charge()
        .create(env.services())
        .await;

    let sub_online = env.get_subscription(sub_id_online).await;
    sub_online
        .assert()
        .is_active()
        .has_resolved_payment_method(&env, true)
        .await;

    // === Test 2: External config → should NOT have payment method ===
    let sub_id_external = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .no_auto_charge()
        .external_payment() // External = no online payment
        .create(env.services())
        .await;

    let sub_external = env.get_subscription(sub_id_external).await;
    sub_external
        .assert()
        .is_active()
        .has_resolved_payment_method(&env, false) // No payment method for External
        .await;
}

/// Test that `has_resolved_payment_method` returns false when no provider is configured.
///
/// Even with Online config (default), if the invoicing entity has no payment provider,
/// resolution returns no payment methods.
#[rstest]
#[tokio::test]
async fn test_has_resolved_payment_method_no_provider(#[future] test_env: TestEnv) {
    let env = test_env.await;
    // Note: NOT calling seed_payments(), so no payment provider is configured

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .no_auto_charge()
        // Default Online config, but no provider configured
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_resolved_payment_method(&env, false) // No provider = no payment method
        .await;
}

// =============================================================================
// DIRECT DEBIT ONLY CONFIG
// =============================================================================

/// Test that a subscription with direct debit only config resolves DD but not card.
///
/// Flow:
/// 1. Configure invoicing entity with DD provider
/// 2. Create subscription with card=false, direct_debit=true
/// 3. Verify resolution has DD connection but no card connection
#[rstest]
#[tokio::test]
async fn test_direct_debit_only_config(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_direct_debit_provider().await;

    let customer = env
        .store()
        .find_customer_by_id(CUST_UBER_ID, TENANT_ID)
        .await
        .expect("Failed to get customer");

    // Direct debit only config
    let dd_only_config = PaymentMethodsConfig::online_specific(false, true);
    let resolved = env
        .services()
        .resolve_subscription_payment_methods(TENANT_ID, Some(&dd_only_config), &customer)
        .await
        .expect("Failed to resolve DD-only config");

    assert!(
        resolved.direct_debit_connection_id.is_some(),
        "DD-only config should resolve direct debit connection"
    );
    assert!(
        resolved.direct_debit_enabled,
        "Direct debit should be enabled"
    );
    assert!(
        resolved.card_connection_id.is_none(),
        "DD-only config should NOT have card connection"
    );
    assert!(!resolved.card_enabled, "Card should be disabled");
    assert!(
        resolved.has_online_payment(),
        "DD-only should have online payment available"
    );
}

// =============================================================================
// BOTH CARD AND DIRECT DEBIT ENABLED
// =============================================================================

/// Test that a subscription with both card and DD enabled resolves both.
/// Also verifies connection reuse when providers are the same.
///
/// Flow:
/// 1. Configure invoicing entity with same provider for card and DD
/// 2. Create subscription with card=true, direct_debit=true
/// 3. Verify both connections are resolved
/// 4. Verify connection IDs are the same (provider reuse)
#[rstest]
#[tokio::test]
async fn test_both_card_and_dd_enabled(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_card_and_direct_debit_same_provider().await;

    let customer = env
        .store()
        .find_customer_by_id(CUST_UBER_ID, TENANT_ID)
        .await
        .expect("Failed to get customer");

    // Both card and DD enabled (inherit all)
    let both_enabled = PaymentMethodsConfig::online();
    let resolved = env
        .services()
        .resolve_subscription_payment_methods(TENANT_ID, Some(&both_enabled), &customer)
        .await
        .expect("Failed to resolve both enabled config");

    assert!(
        resolved.card_connection_id.is_some(),
        "Both enabled should resolve card connection"
    );
    assert!(resolved.card_enabled, "Card should be enabled");
    assert!(
        resolved.direct_debit_connection_id.is_some(),
        "Both enabled should resolve DD connection"
    );
    assert!(resolved.direct_debit_enabled, "DD should be enabled");

    // Same provider should reuse the same connection
    assert_eq!(
        resolved.card_connection_id, resolved.direct_debit_connection_id,
        "Same provider should reuse connection for card and DD"
    );
}

// =============================================================================
// BANK TRANSFER CONFIG
// =============================================================================

/// Test that BankTransfer config resolves bank_account but NOT card/DD.
/// Verifies mutual exclusivity: bank transfer cannot coexist with card/DD.
#[rstest]
#[tokio::test]
async fn test_bank_transfer_config(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_bank_account().await;

    let customer = env
        .store()
        .find_customer_by_id(CUST_UBER_ID, TENANT_ID)
        .await
        .expect("Failed to get customer");

    // Bank transfer config (inherits from invoicing entity)
    let bank_transfer_config = PaymentMethodsConfig::bank_transfer();
    let resolved = env
        .services()
        .resolve_subscription_payment_methods(TENANT_ID, Some(&bank_transfer_config), &customer)
        .await
        .expect("Failed to resolve bank transfer config");

    assert!(
        resolved.bank_account_id.is_some(),
        "Bank transfer should resolve bank_account_id"
    );
    assert!(
        resolved.bank_transfer_enabled,
        "Bank transfer should be enabled"
    );
    assert!(
        resolved.card_connection_id.is_none(),
        "Bank transfer should NOT have card connection"
    );
    assert!(!resolved.card_enabled, "Card should be disabled");
    assert!(
        resolved.direct_debit_connection_id.is_none(),
        "Bank transfer should NOT have DD connection"
    );
    assert!(!resolved.direct_debit_enabled, "DD should be disabled");
    assert!(
        !resolved.has_online_payment(),
        "Bank transfer should NOT count as online payment (uses has_any_payment_method instead)"
    );
    assert!(
        resolved.has_any_payment_method(),
        "Bank transfer should have a payment method available"
    );
}

/// Test that BankTransfer config without bank account configured resolves nothing.
#[rstest]
#[tokio::test]
async fn test_bank_transfer_config_no_account(#[future] test_env: TestEnv) {
    let env = test_env.await;
    // Note: NOT seeding bank account

    let customer = env
        .store()
        .find_customer_by_id(CUST_UBER_ID, TENANT_ID)
        .await
        .expect("Failed to get customer");

    // Bank transfer config but no bank account configured
    let bank_transfer_config = PaymentMethodsConfig::bank_transfer();
    let resolved = env
        .services()
        .resolve_subscription_payment_methods(TENANT_ID, Some(&bank_transfer_config), &customer)
        .await
        .expect("Failed to resolve bank transfer config");

    assert!(
        resolved.bank_account_id.is_none(),
        "No bank account configured should result in None"
    );
    assert!(
        !resolved.bank_transfer_enabled,
        "Bank transfer should be disabled when no account"
    );
    assert!(
        !resolved.has_any_payment_method(),
        "Should have no payment method available"
    );
}

// =============================================================================
// AUTO-CHARGE EXECUTION WITH PAYMENT METHOD ON FILE
// =============================================================================

/// Test the full auto-charge flow with a payment method on file.
///
/// Flow:
/// 1. Create OnStart subscription with auto_charge=true
/// 2. Seed customer payment method
/// 3. Invoice is generated at subscription creation
/// 4. Run outbox/orchestration pipeline
/// 5. Verify invoice gets paid automatically
#[rstest]
#[tokio::test]
async fn test_auto_charge_execution_with_payment_method(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    // Create subscription with auto-charge enabled
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID) // $35/month
        .start_date(start_date)
        .on_start()
        .no_trial()
        .auto_charge()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active().has_cycle_index(0);

    // Invoice should be created
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices.assert().invoice_at(0).has_total(3500);

    // Before orchestration, invoice should be finalized but unpaid
    invoices.assert().invoice_at(0).is_finalized_unpaid();

    // Run the full billing pipeline (outbox dispatch, orchestration, payment processing)
    env.run_outbox_and_orchestration().await;

    // After orchestration, invoice should be paid
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().invoice_at(0).is_finalized_paid();
}

/// Test that auto-charge processes multiple invoices across renewals.
#[rstest]
#[tokio::test]
async fn test_auto_charge_renewal_invoices(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    // Create subscription with auto-charge enabled
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .auto_charge()
        .create(env.services())
        .await;

    // Process first invoice
    env.run_outbox_and_orchestration().await;

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices.assert().invoice_at(0).is_finalized_paid();

    // Process renewal to generate second invoice
    env.process_cycles().await;

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);
    invoices.assert().invoice_at(1).is_finalized_unpaid(); // New invoice not yet processed

    // Run orchestration for the renewal invoice
    env.run_outbox_and_orchestration().await;

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().invoice_at(1).is_finalized_paid();

    // Verify subscription advanced to cycle 1
    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active().has_cycle_index(1);
}
