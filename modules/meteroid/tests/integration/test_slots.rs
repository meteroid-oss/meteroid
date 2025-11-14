use chrono::{NaiveDate, NaiveTime};
use rust_decimal::Decimal;
use std::str::FromStr;
use std::sync::Arc;

use crate::data::ids::*;
use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use common_domain::ids::{BaseId, PriceComponentId, SubscriptionId};
use diesel_models::subscriptions::SubscriptionRow;
use meteroid_mailer::service::MockMailerService;
use meteroid_store::clients::usage::MockUsageClient;
use meteroid_store::domain::{
    ComponentOverride, CreateSubscription, CreateSubscriptionComponents, SlotUpgradeBillingMode,
    SubscriptionComponentNewInternal, SubscriptionFee, SubscriptionFeeBillingPeriod,
    SubscriptionNew,
};
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::repositories::subscriptions::slots::SubscriptionSlotsInterfaceAuto;
use meteroid_store::store::PgConn;
use meteroid_store::{Services, Store};

#[tokio::test]
async fn test_slot_transactions_comprehensive() {
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

    let store = setup.store.clone();
    let pool = setup.store.pool.clone();
    let services = setup.services.clone();
    let mut conn = pool.get().await.unwrap();

    // Test 1: Optimistic mode - slots activate immediately
    test_optimistic_upgrade(&services, &store).await;

    // Test 2: OnInvoicePaid mode - slots pending until payment
    test_on_invoice_paid_upgrade(&services, &store).await;

    // Test 3: OnCheckout mode - preview only, no transaction
    test_on_checkout_preview(&services, &store, &mut conn).await;

    // Test 4: Downgrade - always deferred to next period
    test_slot_downgrade(&services, &store, &mut conn).await;

    // Test 5: Min/Max limits enforcement
    test_min_max_limits(&services, &store, &mut conn).await;

    // Test 6: Currency precision (JPY, BHD, USD)
    test_currency_precision(&services, &store, &mut conn).await;

    // Test 7: Input validation
    test_input_validation(&services, &store, &mut conn).await;

    // Test 8: Concurrent upgrades (race condition prevention)
    test_concurrent_upgrades(&services, &store, &mut conn).await;

    // Test 9: Temporal slot changes across billing cycles
    test_temporal_slot_changes(&services, &store).await;

    // Test 10: Mixed pending and active transactions
    test_mixed_pending_and_active(&services, &store).await;
}

async fn test_optimistic_upgrade(services: &Services, store: &Store) {
    let unit = "optimistic-test-seats";

    let (subscription_id, slot_component_id) = create_subscription_with_slots(
        services,
        unit,
        Decimal::from_str("10.00").unwrap(), // $10 per slot
        Some(1),                             // min 1 slot
        Some(100),                           // max 100 slots
        10,                                  // initial 10 slots
    )
    .await;

    let result = services
        .update_subscription_slots_for_test(
            TENANT_ID,
            subscription_id,
            slot_component_id,
            5, // Add 5 slots
            SlotUpgradeBillingMode::Optimistic,
            NaiveDate::from_ymd_opt(2024, 1, 15).map(|t| t.and_time(NaiveTime::MIN)),
        )
        .await
        .expect("Failed to upgrade slots");

    // Verify slots are active immediately
    assert!(
        result.slots_active,
        "Slots should be active immediately in Optimistic mode"
    );
    assert_eq!(result.delta_applied, 5);
    assert!(result.invoice_id.is_some(), "Invoice should be created");
    assert!(result.prorated_amount.is_some());

    // Verify invoice was created
    let invoice = store
        .get_invoice_by_id(TENANT_ID, result.invoice_id.unwrap())
        .await
        .expect("Failed to get invoice");

    assert!(invoice.total > 0, "Invoice total should be positive");
    assert!(
        !invoice.line_items.is_empty(),
        "Invoice should have line items"
    );

    // Verify line item has correct slot information
    let line_item = &invoice.line_items[0];
    assert_eq!(line_item.quantity, Some(Decimal::from(5)));
    assert!(line_item.is_prorated, "Line item should be prorated");

    let count = store
        .get_active_slots_value(TENANT_ID, subscription_id, unit.to_string(), None)
        .await
        .expect("Failed to get current slots");

    assert_eq!(count, 15);
}

async fn test_on_invoice_paid_upgrade(services: &Services, store: &Store) {
    let (subscription_id, slot_component_id) = create_subscription_with_slots(
        services,
        "on-invoice-paid-seats",
        Decimal::from_str("15.00").unwrap(),
        Some(1),
        Some(100),
        5,
    )
    .await;

    let result = services
        .update_subscription_slots(
            TENANT_ID,
            subscription_id,
            slot_component_id,
            3, // Add 3 slots
            SlotUpgradeBillingMode::OnInvoicePaid,
        )
        .await
        .expect("Failed to upgrade slots");

    // Verify slots are NOT active yet
    assert!(
        !result.slots_active,
        "Slots should be pending in OnInvoicePaid mode"
    );
    assert_eq!(result.delta_applied, 3);
    assert!(result.invoice_id.is_some(), "Invoice should be created");

    let invoice_id = result.invoice_id.unwrap();

    // Verify invoice was created
    let invoice = store
        .get_invoice_by_id(TENANT_ID, invoice_id)
        .await
        .expect("Failed to get invoice");

    assert!(invoice.total > 0);

    // Simulate payment webhook - activate pending transactions
    let activated = services
        .activate_pending_slot_transactions(
            TENANT_ID,
            invoice_id,
            NaiveDate::from_ymd_opt(2024, 1, 4).map(|t| t.and_time(NaiveTime::MIN)),
        )
        .await
        .expect("Failed to activate pending transactions");

    assert!(
        !activated.is_empty(),
        "Should have activated pending transactions"
    );
    assert_eq!(
        activated[0].0, subscription_id,
        "Should activate for correct subscription"
    );
    let count_after_activation = store
        .get_active_slots_value(
            TENANT_ID,
            subscription_id,
            "on-invoice-paid-seats".to_string(),
            NaiveDate::from_ymd_opt(2024, 1, 30).map(|t| t.and_time(NaiveTime::MIN)),
        )
        .await
        .expect("Failed to get slots after activation");
    assert_eq!(
        count_after_activation, 17,
        "Should have 17 active slots after pending activation"
    );
}

async fn test_on_checkout_preview(services: &Services, _store: &Store, _conn: &mut PgConn) {
    let (subscription_id, slot_component_id) = create_subscription_with_slots(
        services,
        "on-checkout-preview-seats",
        Decimal::from_str("20.00").unwrap(),
        Some(1),
        Some(100),
        8,
    )
    .await;

    let result = services
        .update_subscription_slots(
            TENANT_ID,
            subscription_id,
            slot_component_id,
            2, // Add 2 slots
            SlotUpgradeBillingMode::OnCheckout,
        )
        .await
        .expect("Failed to get checkout preview");

    // Verify no slots activated, no invoice created
    assert!(
        !result.slots_active,
        "Slots should not be active in OnCheckout mode"
    );
    assert_eq!(result.delta_applied, 2);
    assert!(
        result.invoice_id.is_none(),
        "No invoice should be created in preview mode"
    );
    assert!(
        result.prorated_amount.is_some(),
        "Should return prorated amount for preview"
    );
}

async fn test_slot_downgrade(services: &Services, store: &Store, _conn: &mut PgConn) {
    let (subscription_id, slot_component_id) = create_subscription_with_slots(
        services,
        "downgrade-test-seats",
        Decimal::from_str("12.00").unwrap(),
        Some(1),
        Some(100),
        20,
    )
    .await;

    let initial_count = store
        .get_active_slots_value(
            TENANT_ID,
            subscription_id,
            "downgrade-test-seats".to_string(),
            None,
        )
        .await
        .expect("Failed to get current slots");

    // Downgrade by 5 slots
    let result = services
        .update_subscription_slots(
            TENANT_ID,
            subscription_id,
            slot_component_id,
            -5,                                 // Remove 5 slots
            SlotUpgradeBillingMode::Optimistic, // Mode is ignored for downgrades
        )
        .await
        .expect("Failed to downgrade slots");

    // Verify downgrade is deferred (slots still active)
    assert!(
        result.slots_active,
        "Downgrade slots remain active until period end"
    );
    assert_eq!(result.delta_applied, -5);
    assert!(result.invoice_id.is_none(), "No invoice for downgrade");

    // Verify current slots haven't changed yet (deferred)
    let current_count = store
        .get_active_slots_value(
            TENANT_ID,
            subscription_id,
            "downgrade-test-seats".to_string(),
            None,
        )
        .await
        .expect("Failed to get current slots");

    assert_eq!(
        current_count, initial_count,
        "Slots should not change until period end"
    );
}

async fn test_min_max_limits(services: &Services, _store: &Store, _conn: &mut PgConn) {
    let (subscription_id, slot_component_id) = create_subscription_with_slots(
        services,
        "limit-test-seats",
        Decimal::from_str("5.00").unwrap(),
        Some(1),  // min 1 slot
        Some(50), // max 50 slots
        10,
    )
    .await;

    // Test exceeding maximum
    let result = services
        .update_subscription_slots(
            TENANT_ID,
            subscription_id,
            slot_component_id,
            100, // Try to add way too many slots (would result in 110 total)
            SlotUpgradeBillingMode::Optimistic,
        )
        .await;

    assert!(result.is_err(), "Should reject upgrade exceeding max_slots");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("maximum") || err_msg.contains("exceed"),
        "Error should mention maximum limit"
    );

    // Test going below minimum
    let result = services
        .update_subscription_slots(
            TENANT_ID,
            subscription_id,
            slot_component_id,
            -10, // Try to remove all slots (would result in 0 total)
            SlotUpgradeBillingMode::Optimistic,
        )
        .await;

    assert!(result.is_err(), "Should reject downgrade below min_slots");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("minimum") || err_msg.contains("below"),
        "Error should mention minimum limit"
    );
}

async fn test_currency_precision(services: &Services, store: &Store, _conn: &mut PgConn) {
    let (subscription_id, slot_component_id) = create_subscription_with_slots(
        services,
        "precision-test-seats",
        Decimal::from_str("25.50").unwrap(), // Price with cents
        Some(1),
        Some(100),
        5,
    )
    .await;

    let result = services
        .update_subscription_slots(
            TENANT_ID,
            subscription_id,
            slot_component_id,
            1,
            SlotUpgradeBillingMode::Optimistic,
        )
        .await
        .expect("Failed to upgrade slots");

    let invoice = store
        .get_invoice_by_id(TENANT_ID, result.invoice_id.unwrap())
        .await
        .expect("Failed to get invoice");

    // Verify amounts are in subunits (cents for USD)
    assert!(invoice.total > 0, "Total should be positive");
    assert!(invoice.subtotal > 0, "Subtotal should be positive");

    // Verify line item amount is correctly calculated
    let line_item = &invoice.line_items[0];
    assert_eq!(line_item.quantity, Some(Decimal::from(1)));
    assert!(
        line_item.amount_subtotal > 0,
        "Line item subtotal should be positive"
    );
}

async fn test_input_validation(services: &Services, _store: &Store, _conn: &mut PgConn) {
    let (subscription_id, slot_component_id) = create_subscription_with_slots(
        services,
        "validation-test-seats",
        Decimal::from_str("10.00").unwrap(),
        Some(1),
        Some(100),
        10,
    )
    .await;

    // Test delta = 0
    let result = services
        .update_subscription_slots(
            TENANT_ID,
            subscription_id,
            slot_component_id,
            0, // Invalid: zero delta
            SlotUpgradeBillingMode::Optimistic,
        )
        .await;

    assert!(result.is_err(), "Should reject delta=0");
    assert!(
        result.unwrap_err().to_string().contains("zero"),
        "Error should mention zero"
    );

    // Test negative slots (below zero)
    let result = services
        .update_subscription_slots(
            TENANT_ID,
            subscription_id,
            slot_component_id,
            -20, // Would result in -10 slots
            SlotUpgradeBillingMode::Optimistic,
        )
        .await;

    assert!(result.is_err(), "Should reject downgrade below zero");

    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("below zero") || err.contains("negative") || err.contains("below minimum"),
        "Error should mention negative/zero slots or minimum limit, got: {}",
        err
    );
}

async fn test_concurrent_upgrades(services: &Services, store: &Store, _conn: &mut PgConn) {
    let (subscription_id, slot_component_id) = create_subscription_with_slots(
        services,
        "concurrent-test-seats",
        Decimal::from_str("8.00").unwrap(),
        Some(1),
        Some(100),
        20,
    )
    .await;

    let initial_count = store
        .get_active_slots_value(
            TENANT_ID,
            subscription_id,
            "concurrent-test-seats".to_string(),
            None,
        )
        .await
        .expect("Failed to get initial count");

    // Launch 3 concurrent upgrade requests
    let services_clone1 = services.clone();
    let services_clone2 = services.clone();
    let services_clone3 = services.clone();

    let handle1 = tokio::spawn(async move {
        services_clone1
            .update_subscription_slots_for_test(
                TENANT_ID,
                subscription_id,
                slot_component_id,
                1,
                SlotUpgradeBillingMode::Optimistic,
                NaiveDate::from_ymd_opt(2024, 1, 15).map(|t| t.and_time(NaiveTime::MIN)),
            )
            .await
    });

    let handle2 = tokio::spawn(async move {
        services_clone2
            .update_subscription_slots_for_test(
                TENANT_ID,
                subscription_id,
                slot_component_id,
                1,
                SlotUpgradeBillingMode::Optimistic,
                NaiveDate::from_ymd_opt(2024, 1, 15).map(|t| t.and_time(NaiveTime::MIN)),
            )
            .await
    });

    let handle3 = tokio::spawn(async move {
        services_clone3
            .update_subscription_slots_for_test(
                TENANT_ID,
                subscription_id,
                slot_component_id,
                1,
                SlotUpgradeBillingMode::Optimistic,
                NaiveDate::from_ymd_opt(2024, 1, 15).map(|t| t.and_time(NaiveTime::MIN)),
            )
            .await
    });

    let results = tokio::join!(handle1, handle2, handle3);

    assert!(results.0.is_ok(), "First request should succeed");
    assert!(results.1.is_ok(), "Second request should succeed");
    assert!(results.2.is_ok(), "Third request should succeed");

    let _result1 = results.0.unwrap().unwrap();
    let _result2 = results.1.unwrap().unwrap();
    let _result3 = results.2.unwrap().unwrap();

    // Verify final count is correct (initial + 3)
    let final_count = store
        .get_active_slots_value(
            TENANT_ID,
            subscription_id,
            "concurrent-test-seats".to_string(),
            NaiveDate::from_ymd_opt(2024, 1, 15).map(|t| t.and_time(NaiveTime::MIN)),
        )
        .await
        .expect("Failed to get final count");

    assert_eq!(
        final_count,
        initial_count + 3,
        "Final count should be initial + 3 (one from each concurrent request)"
    );
}

async fn test_temporal_slot_changes(services: &Services, store: &Store) {
    let unit = "temporal-test-seats";

    // Create subscription starting on 2024-01-01 with 5 slots
    let (subscription_id, slot_component_id) = create_subscription_with_slots(
        services,
        unit,
        Decimal::from_str("10.00").unwrap(), // $10 per slot
        Some(0),                             // min 0 slots
        Some(100),                           // max 100 slots
        5,                                   // initial 5 slots
    )
    .await;

    let slots = store
        .get_active_slots_value(
            TENANT_ID,
            subscription_id,
            unit.to_string(),
            NaiveDate::from_ymd_opt(2024, 1, 1).map(|t| t.and_time(NaiveTime::MIN)),
        )
        .await
        .expect("Failed to get initial slots");
    assert_eq!(slots, 5, "Should start with 5 slots");

    // Check initial invoice count (should have 1 invoice for initial subscription)
    let mut invoices = get_invoices_for_subscription(store, subscription_id, unit).await;
    assert_eq!(invoices.len(), 1, "Should have 1 initial invoice");

    // Mid-period: Add 2 slots (immediate effect)
    let result = services
        .update_subscription_slots_for_test(
            TENANT_ID,
            subscription_id,
            slot_component_id,
            2,
            SlotUpgradeBillingMode::Optimistic,
            NaiveDate::from_ymd_opt(2024, 1, 15).map(|t| t.and_time(NaiveTime::MIN)),
        )
        .await
        .expect("Failed to add 2 slots");

    assert!(
        result.slots_active,
        "Upgrade should take effect immediately"
    );
    assert_eq!(
        result.new_slot_count, 7,
        "Should have 7 slots after upgrade"
    );
    assert!(
        result.invoice_id.is_some(),
        "Should create prorated invoice"
    );

    // Verify slots are now 7
    let slots = store
        .get_active_slots_value(
            TENANT_ID,
            subscription_id,
            unit.to_string(),
            NaiveDate::from_ymd_opt(2024, 1, 15).map(|t| t.and_time(NaiveTime::MIN)),
        )
        .await
        .expect("Failed to get slots after upgrade");
    assert_eq!(slots, 7, "Should have 7 active slots");

    invoices = get_invoices_for_subscription(store, subscription_id, unit).await;
    assert_eq!(invoices.len(), 2, "Should have 2 invoices after upgrade");

    // Verify the upgrade invoice has correct details
    let upgrade_invoice = &invoices[1]; // Most recent
    let upgrade_line = upgrade_invoice
        .line_items
        .iter()
        .find(|line| line.name.contains(unit))
        .expect("Should have slot line item");
    assert_eq!(
        upgrade_line.quantity,
        Some(Decimal::from(2)),
        "Upgrade invoice should be for 2 slots"
    );
    assert!(
        upgrade_invoice.total > 0,
        "Upgrade invoice should have positive amount"
    );

    // Mid-period: Remove 5 slots (deferred to period end)
    let result = services
        .update_subscription_slots_for_test(
            TENANT_ID,
            subscription_id,
            slot_component_id,
            -5,
            SlotUpgradeBillingMode::Optimistic, // Mode is ignored for downgrades
            NaiveDate::from_ymd_opt(2024, 1, 15).map(|t| t.and_time(NaiveTime::MIN)),
        )
        .await
        .expect("Failed to schedule downgrade");

    assert!(result.slots_active, "Downgrade should be deferred");
    assert_eq!(
        result.delta_applied, -5,
        "Should schedule removal of 5 slots"
    );
    assert!(
        result.invoice_id.is_none(),
        "No invoice created for deferred downgrade"
    );

    // Verify slots are still 7 (downgrade not yet effective)
    let slots = store
        .get_active_slots_value(
            TENANT_ID,
            subscription_id,
            unit.to_string(),
            NaiveDate::from_ymd_opt(2024, 1, 15).map(|t| t.and_time(NaiveTime::MIN)),
        )
        .await
        .expect("Failed to get slots after downgrade schedule");
    assert_eq!(slots, 7, "Should still have 7 slots until period end");

    // Try to remove 5 again, which would end up in a NEGATIVE
    let result = services
        .update_subscription_slots_for_test(
            TENANT_ID,
            subscription_id,
            slot_component_id,
            -5,
            SlotUpgradeBillingMode::Optimistic, // Mode is ignored for downgrades
            NaiveDate::from_ymd_opt(2024, 1, 15).map(|t| t.and_time(NaiveTime::MIN)),
        )
        .await;

    assert!(
        result.is_err(),
        "Should fail to schedule removal of 5 slots again, as it would cause a negative"
    );

    // Check invoice count after downgrade (should still be 2, no new invoice)
    invoices = get_invoices_for_subscription(store, subscription_id, unit).await;
    assert_eq!(
        invoices.len(),
        2,
        "Should still have 2 invoices (no invoice for deferred downgrade)"
    );

    services.get_and_process_cycle_transitions().await.unwrap();
    services.get_and_process_due_events().await.unwrap();

    // Verify the downgrade has taken effect (7 - 5 = 2)
    let slots = store
        .get_active_slots_value(
            TENANT_ID,
            subscription_id,
            unit.to_string(),
            NaiveDate::from_ymd_opt(2024, 2, 1).map(|t| t.and_time(NaiveTime::MIN)),
        )
        .await
        .expect("Failed to get slots after period transition");
    assert_eq!(slots, 2, "Should have 2 slots after downgrade takes effect");

    // Check invoice count after billing cycle (should have 3 invoices now - new period invoice)
    invoices = get_invoices_for_subscription(store, subscription_id, unit).await;
    assert_eq!(
        invoices.len(),
        3,
        "Should have 3 invoices after billing cycle"
    );

    let new_period_invoice = &invoices[2]; // Most recent
    let _new_period_line = new_period_invoice
        .line_items
        .iter()
        .find(|line| line.name.contains(unit))
        .expect("Should have slot line item");
}

async fn test_mixed_pending_and_active(services: &Services, store: &Store) {
    let unit = "mixed-mode-test-seats";

    // Create subscription starting with 10 slots
    let (subscription_id, slot_component_id) = create_subscription_with_slots(
        services,
        unit,
        Decimal::from_str("10.00").unwrap(),
        Some(1),
        Some(100),
        10, // initial 10 slots
    )
    .await;

    // Verify initial count
    let initial_count = store
        .get_active_slots_value(
            TENANT_ID,
            subscription_id,
            unit.to_string(),
            NaiveDate::from_ymd_opt(2024, 1, 1).map(|t| t.and_time(NaiveTime::MIN)),
        )
        .await
        .expect("Failed to get initial slots");
    assert_eq!(initial_count, 10, "Should start with 10 slots");

    // Step 1: Add +1 slot (Optimistic - should activate immediately)
    let result1 = services
        .update_subscription_slots_for_test(
            TENANT_ID,
            subscription_id,
            slot_component_id,
            1,
            SlotUpgradeBillingMode::Optimistic,
            NaiveDate::from_ymd_opt(2024, 1, 2).map(|t| t.and_time(NaiveTime::MIN)),
        )
        .await
        .expect("Failed to add first +1 slot");

    assert!(
        result1.slots_active,
        "First +1 should be active immediately"
    );
    assert_eq!(result1.delta_applied, 1);
    assert_eq!(result1.new_slot_count, 11, "Should now have 11 slots");

    // Verify count is now 11
    let count_after_first = store
        .get_active_slots_value(
            TENANT_ID,
            subscription_id,
            unit.to_string(),
            NaiveDate::from_ymd_opt(2024, 1, 2).map(|t| t.and_time(NaiveTime::MIN)),
        )
        .await
        .expect("Failed to get slots after first upgrade");
    assert_eq!(count_after_first, 11, "Should have 11 active slots");

    // Step 2: Add +5 slots as PENDING (OnInvoicePaid - should NOT activate yet)
    let result2 = services
        .update_subscription_slots_for_test(
            TENANT_ID,
            subscription_id,
            slot_component_id,
            5,
            SlotUpgradeBillingMode::OnInvoicePaid,
            NaiveDate::from_ymd_opt(2024, 1, 3).map(|t| t.and_time(NaiveTime::MIN)),
        )
        .await
        .expect("Failed to add pending +5 slots");

    assert!(!result2.slots_active, "Pending +5 should NOT be active yet");
    assert_eq!(result2.delta_applied, 5);
    assert_eq!(
        result2.new_slot_count, 11,
        "New count should stay 11 (as not activated)"
    );
    let pending_invoice_id = result2
        .invoice_id
        .expect("Should create invoice for pending slots");

    // Verify count is still 11 (pending slots not active yet)
    let count_after_pending = store
        .get_active_slots_value(
            TENANT_ID,
            subscription_id,
            unit.to_string(),
            NaiveDate::from_ymd_opt(2024, 1, 3).map(|t| t.and_time(NaiveTime::MIN)),
        )
        .await
        .expect("Failed to get slots after pending addition");
    assert_eq!(
        count_after_pending, 11,
        "Should still have 11 active slots (pending not activated)"
    );

    // Step 3: Add another +1 slot (Optimistic - should activate immediately)
    let result3 = services
        .update_subscription_slots_for_test(
            TENANT_ID,
            subscription_id,
            slot_component_id,
            1,
            SlotUpgradeBillingMode::Optimistic,
            NaiveDate::from_ymd_opt(2024, 1, 4).map(|t| t.and_time(NaiveTime::MIN)),
        )
        .await
        .expect("Failed to add second +1 slot");

    assert!(
        result3.slots_active,
        "Second +1 should be active immediately"
    );
    assert_eq!(result3.delta_applied, 1);
    assert_eq!(
        result3.new_slot_count, 12,
        "Should now have 12 active slots (11 + 1, pending still not active)"
    );

    // Verify count is now 12 (still without pending)
    let count_after_second = store
        .get_active_slots_value(
            TENANT_ID,
            subscription_id,
            unit.to_string(),
            NaiveDate::from_ymd_opt(2024, 1, 4).map(|t| t.and_time(NaiveTime::MIN)),
        )
        .await
        .expect("Failed to get slots after second upgrade");
    assert_eq!(
        count_after_second, 12,
        "Should have 12 active slots (pending +5 still not active)"
    );

    // Step 4: Activate the pending +5 slots (simulate payment)
    let activated = services
        .activate_pending_slot_transactions(
            TENANT_ID,
            pending_invoice_id,
            NaiveDate::from_ymd_opt(2024, 1, 4).map(|t| t.and_time(NaiveTime::MIN)),
        )
        .await
        .expect("Failed to activate pending transactions");

    assert!(
        !activated.is_empty(),
        "Should have activated pending transactions"
    );
    assert_eq!(
        activated[0].0, subscription_id,
        "Should activate for correct subscription"
    );

    // Verify count is now 17 (12 + 5 pending activated)
    let count_after_activation = store
        .get_active_slots_value(
            TENANT_ID,
            subscription_id,
            unit.to_string(),
            NaiveDate::from_ymd_opt(2024, 1, 4).map(|t| t.and_time(NaiveTime::MIN)),
        )
        .await
        .expect("Failed to get slots after activation");
    assert_eq!(
        count_after_activation, 17,
        "Should have 17 active slots after pending activation"
    );

    // Step 5: Add another +1 slot (Optimistic - should activate immediately)
    let result5 = services
        .update_subscription_slots_for_test(
            TENANT_ID,
            subscription_id,
            slot_component_id,
            1,
            SlotUpgradeBillingMode::Optimistic,
            NaiveDate::from_ymd_opt(2024, 1, 5).map(|t| t.and_time(NaiveTime::MIN)),
        )
        .await
        .expect("Failed to add third +1 slot");

    assert!(
        result5.slots_active,
        "Third +1 should be active immediately"
    );
    assert_eq!(result5.delta_applied, 1);
    assert_eq!(result5.new_slot_count, 18, "Should now have 18 slots");

    // Final verification: count should be initial + 8 (10 + 1 + 5 + 1 + 1 = 18)
    let final_count = store
        .get_active_slots_value(TENANT_ID, subscription_id, unit.to_string(), None)
        .await
        .expect("Failed to get final slots count");
    assert_eq!(
        final_count,
        initial_count + 8,
        "Final count should be initial (10) + 8 = 18"
    );
    assert_eq!(final_count, 18, "Final count should be exactly 18 slots");
}

async fn get_invoices_for_subscription(
    store: &Store,
    subscription_id: SubscriptionId,
    unit: &str,
) -> Vec<meteroid_store::domain::Invoice> {
    use meteroid_store::domain::{OrderByRequest, PaginationRequest};

    let all_invoices = store
        .list_invoices(
            TENANT_ID,
            None,
            Some(subscription_id),
            None,
            None,
            OrderByRequest::DateAsc,
            PaginationRequest {
                page: 0,
                per_page: None,
            },
        )
        .await
        .expect("Failed to get invoices");

    all_invoices
        .items
        .into_iter()
        .filter(|inv_with_customer| {
            inv_with_customer
                .invoice
                .line_items
                .iter()
                .any(|line| line.name.contains(unit))
        })
        .map(|inv_with_customer| inv_with_customer.invoice)
        .collect()
}

async fn list_all_slot_transactions(
    conn: &mut PgConn,
    subscription_id: SubscriptionId,
) -> Vec<diesel_models::slot_transactions::SlotTransactionRow> {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    use diesel_models::schema::slot_transaction;

    slot_transaction::table
        .filter(slot_transaction::subscription_id.eq(subscription_id))
        .order_by(slot_transaction::transaction_at.asc())
        .load::<diesel_models::slot_transactions::SlotTransactionRow>(conn)
        .await
        .expect("Failed to load slot transactions")
}

async fn get_subscription(conn: &mut PgConn, subscription_id: SubscriptionId) -> SubscriptionRow {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    use diesel_models::schema::subscription;

    subscription::table
        .filter(subscription::id.eq(subscription_id))
        .first::<SubscriptionRow>(conn)
        .await
        .expect("Failed to load subscription")
}

async fn create_subscription_with_slots(
    services: &Services,
    unit_name: &str,
    unit_rate: Decimal,
    min_slots: Option<u32>,
    max_slots: Option<u32>,
    initial_slots: u32,
) -> (SubscriptionId, PriceComponentId) {
    let subscription_id = services
        .insert_subscription(
            CreateSubscription {
                subscription: SubscriptionNew {
                    customer_id: CUST_UBER_ID,
                    plan_version_id: PLAN_VERSION_NOTION_ID,
                    created_by: USER_ID,
                    net_terms: None,
                    invoice_memo: None,
                    invoice_threshold: None,
                    start_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                    end_date: None,
                    billing_start_date: None,
                    activation_condition:
                        meteroid_store::domain::SubscriptionActivationCondition::OnStart,
                    trial_duration: None,
                    billing_day_anchor: None,
                    payment_strategy: None,
                    auto_advance_invoices: true,
                    charge_automatically: true,
                    purchase_order: None,
                },
                price_components: Some(CreateSubscriptionComponents {
                    parameterized_components: vec![],
                    overridden_components: vec![ComponentOverride {
                        component_id: COMP_NOTION_SEATS_ID,
                        component: SubscriptionComponentNewInternal {
                            price_component_id: Some(COMP_NOTION_SEATS_ID),
                            product_id: None,
                            name: unit_name.to_string(),
                            period: SubscriptionFeeBillingPeriod::Monthly,
                            fee: SubscriptionFee::Slot {
                                unit: unit_name.to_string(),
                                unit_rate,
                                min_slots,
                                max_slots,
                                initial_slots,
                            },
                            is_override: false,
                        },
                    }],
                    extra_components: vec![],
                    remove_components: vec![],
                }),
                add_ons: None,
                coupons: None,
            },
            TENANT_ID,
        )
        .await
        .expect("Failed to create subscription")
        .id;

    (subscription_id, COMP_NOTION_SEATS_ID)
}
