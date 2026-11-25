use chrono::{NaiveDate, NaiveTime};
use rust_decimal::Decimal;
use std::str::FromStr;
use std::sync::Arc;

use crate::data::ids::*;
use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use common_domain::ids::{PriceComponentId, SubscriptionId};
use meteroid_store::clients::usage::MockUsageClient;
use meteroid_store::domain::enums::BillingPeriodEnum;
use meteroid_store::domain::price_components::{PriceEntry, PriceInput, RatingModel, TierRow};
use meteroid_store::domain::prices::Pricing;
use meteroid_store::domain::slot_transactions::UpdateSlotsResult;
use meteroid_store::domain::{
    ComponentOverride, ComponentParameterization, ComponentParameters, CreateSubscription,
    CreateSubscriptionComponents, SlotUpgradeBillingMode, SubscriptionNew,
};
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::repositories::subscriptions::slots::SubscriptionSlotsInterfaceAuto;
use meteroid_store::store::PgConn;
use meteroid_store::{Services, Store};
use meteroid_testkit::mailer::MockMailerService;

/// The slot unit name comes from the Product's FeeStructure, not the component override name.
const SLOT_UNIT: &str = "Seats";

#[tokio::test]
async fn test_slot_transactions_comprehensive() {
    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;

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
}

// separated to keep subscriptions below the batching threshold (cycle & event runners)
#[tokio::test]
async fn test_slot_transactions_comprehensive_next() {
    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;

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

/// A metered feature's limit counted per seat is the entitlement's times the seats held.
/// Rules: R-133
#[tokio::test]
async fn a_feature_limit_counts_per_seat_held() {
    use meteroid_store::domain::entitlements::{
        EntitlementValue, FeatureNew, FeatureType, OverageBehavior, ResetPeriod,
        ResolvedEntitlementValue,
    };
    use meteroid_store::repositories::ManagedQuantitiesInterface;
    use meteroid_store::repositories::entitlements::{EntitlementsInterface, ResolveTarget};

    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;
    let setup = meteroid_it::container::start_meteroid_with_clients(
        postgres_connection_string,
        SeedLevel::PLANS,
        Arc::new(MockUsageClient::noop()),
        Arc::new(MockMailerService::new()),
    )
    .await;
    let (services, store) = (setup.services.clone(), setup.store.clone());
    let (subscription_id, _) = create_subscription_with_slots(
        &services,
        Decimal::from_str("12.00").unwrap(),
        Some(1),
        Some(100),
        20,
    )
    .await;
    let feature = store
        .create_feature(
            common_domain::actor::Actor::System,
            FeatureNew {
                tenant_id: TENANT_ID,
                product_id: Some(PRODUCT_SEATS_ID),
                name: "API calls".to_string(),
                code: "api_calls".to_string(),
                description: None,
                feature_type: FeatureType::metered(METRIC_BANDWIDTH),
                entitlement: Some(EntitlementValue::Metered {
                    limit: Some(Decimal::from(1000)),
                    reset_period: ResetPeriod::BillingCycle,
                    overage_behavior: OverageBehavior::Allow,
                    thresholds: vec![],
                    enabled: true,
                    group_by: None,
                }),
            },
        )
        .await
        .expect("a metered feature of the seat product")
        .id;
    let seats = store
        .list_managed_quantities(TENANT_ID)
        .await
        .expect("quantities")
        .into_iter()
        .find(|quantity| quantity.code == SLOT_UNIT)
        .expect("the seats' quantity");

    let limit = || async {
        let mut conn = store.get_conn().await.expect("conn");
        store
            .resolve_entitlements_for_entity(
                &mut conn,
                TENANT_ID,
                ResolveTarget::Subscription(subscription_id),
            )
            .await
            .expect("entitlements")
            .into_iter()
            .find(|entitlement| entitlement.feature.id == feature)
            .and_then(|entitlement| match entitlement.value {
                ResolvedEntitlementValue::Metered { limit, .. } => limit,
                _ => None,
            })
    };
    assert_eq!(limit().await, Some(Decimal::from(1000)));

    store
        .set_feature_limit_quantity(TENANT_ID, feature, Some(seats.id))
        .await
        .expect("counted per seat");
    assert_eq!(
        limit().await,
        Some(Decimal::from(20_000)),
        "1000 for each of 20 seats"
    );
    let listed = store
        .list_managed_quantities(TENANT_ID)
        .await
        .expect("quantities")
        .into_iter()
        .find(|quantity| quantity.id == seats.id)
        .expect("the seats' quantity");
    assert_eq!(listed.per_unit_features, vec![feature]);

    store
        .set_feature_limit_quantity(TENANT_ID, feature, None)
        .await
        .expect("counted per subscription again");
    assert_eq!(limit().await, Some(Decimal::from(1000)));
}

/// A seat quantity's policies decide when a change takes effect: a decrease at once is out of
/// force from its day, an increase for the next period from the renewal; a change to a count
/// is its difference with the seats held.
/// Rules: R-108
#[tokio::test]
async fn seat_policies_date_a_change_and_a_target_count_is_reached() {
    use meteroid_store::domain::managed_quantities::{
        ManagedQuantityPatch, QuantityDecrease, QuantityIncrease, SeatChange,
    };
    use meteroid_store::repositories::ManagedQuantitiesInterface;

    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;
    let setup = meteroid_it::container::start_meteroid_with_clients(
        postgres_connection_string,
        SeedLevel::PLANS,
        Arc::new(MockUsageClient::noop()),
        Arc::new(MockMailerService::new()),
    )
    .await;
    let (services, store) = (setup.services.clone(), setup.store.clone());

    let (subscription_id, slot_component_id) = create_subscription_with_slots(
        &services,
        Decimal::from_str("12.00").unwrap(),
        Some(1),
        Some(100),
        20,
    )
    .await;
    let seats = store
        .list_managed_quantities(TENANT_ID)
        .await
        .expect("quantities")
        .into_iter()
        .find(|quantity| quantity.code == SLOT_UNIT)
        .expect("the seats' quantity, registered with their ledger");
    store
        .update_managed_quantity(ManagedQuantityPatch {
            tenant_id: TENANT_ID,
            id: seats.id,
            name: None,
            on_increase: Some(QuantityIncrease::NextPeriod),
            on_decrease: Some(QuantityDecrease::Immediately),
        })
        .await
        .expect("policies");

    let day = |day: u32, month: u32| {
        NaiveDate::from_ymd_opt(2024, month, day)
            .unwrap()
            .and_time(NaiveTime::MIN)
    };
    let count_on =
        |at| store.get_active_slots_value(TENANT_ID, subscription_id, QUANTITY_SEATS_ID, Some(at));

    let removed = services
        .update_subscription_slots_for_test(
            TENANT_ID,
            subscription_id,
            slot_component_id,
            -5,
            SlotUpgradeBillingMode::Optimistic,
            Some(day(16, 1)),
        )
        .await
        .expect("an immediate decrease");
    assert_eq!(removed.new_slot_count, 15);
    assert_eq!(count_on(day(15, 1)).await.unwrap(), 20);
    assert_eq!(
        count_on(day(16, 1)).await.unwrap(),
        15,
        "out of force from its day"
    );
    assert_eq!(
        january_seats_net(&store, subscription_id).await,
        24_000 - 3_097,
        "5 × 12.00 × 16/31 given back"
    );

    let added = services
        .update_subscription_slots_for_test(
            TENANT_ID,
            subscription_id,
            slot_component_id,
            SeatChange::To(18),
            SlotUpgradeBillingMode::Optimistic,
            Some(day(16, 1)),
        )
        .await
        .expect("an increase to a count, for the next period");
    assert_eq!(added.delta_applied, 3);
    assert!(
        added.invoice_id.is_none(),
        "nothing billed before the renewal"
    );
    assert_eq!(count_on(day(31, 1)).await.unwrap(), 15);
    assert_eq!(count_on(day(1, 2)).await.unwrap(), 18);

    let back = services
        .update_subscription_slots_for_test(
            TENANT_ID,
            subscription_id,
            slot_component_id,
            SeatChange::To(15),
            SlotUpgradeBillingMode::Optimistic,
            Some(day(16, 1)),
        )
        .await
        .expect("a count replaces the change waiting for the renewal");
    assert_eq!(back.delta_applied, 0);
    assert_eq!(count_on(day(1, 2)).await.unwrap(), 15);

    let unchanged = services
        .update_subscription_slots_for_test(
            TENANT_ID,
            subscription_id,
            slot_component_id,
            SeatChange::To(15),
            SlotUpgradeBillingMode::Optimistic,
            Some(day(16, 1)),
        )
        .await;
    assert!(unchanged.is_err(), "a count already held changes nothing");
}

/// What the seat lines of January bill, net of the notes crediting them, in cents.
async fn january_seats_net(store: &Store, subscription_id: SubscriptionId) -> i64 {
    use meteroid_store::repositories::CreditNoteInterface;
    let january = |start: NaiveDate| start < NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    let mut net = 0;
    for invoice in get_invoices_for_subscription(store, subscription_id, SLOT_UNIT).await {
        net += invoice
            .line_items
            .iter()
            .filter(|line| line.name.contains(SLOT_UNIT) && january(line.start_date))
            .map(|line| line.amount_subtotal)
            .sum::<i64>();
        for note in store
            .list_credit_notes_by_invoice_id(TENANT_ID, invoice.id)
            .await
            .expect("credit notes")
        {
            net -= note
                .line_items
                .iter()
                .filter(|line| january(line.start_date))
                .map(|line| line.amount_subtotal.abs())
                .sum::<i64>();
        }
    }
    net
}

/// Seats priced by graduated tiers, 10.00 for the first ten and 8.00 after: five more on
/// Jan 15 cost what the five next seats cost, 5 × 8.00 × 17/31, not five at the first tier.
#[tokio::test]
async fn tiered_seats_bought_mid_period_cost_their_tier() {
    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;
    let setup = meteroid_it::container::start_meteroid_with_clients(
        postgres_connection_string,
        SeedLevel::PLANS,
        Arc::new(MockUsageClient::noop()),
        Arc::new(MockMailerService::new()),
    )
    .await;
    let (services, store) = (setup.services.clone(), setup.store.clone());
    let tier = |first_unit, rate: &str| TierRow {
        first_unit,
        rate: Decimal::from_str(rate).unwrap(),
        flat_fee: None,
        flat_cap: None,
    };
    let (subscription_id, slot_component_id) = create_subscription_with_seat_model(
        &services,
        RatingModel::Tiered {
            tiers: vec![tier(0, "10.00"), tier(10, "8.00")],
        },
        Some(1),
        Some(100),
        10,
    )
    .await;

    let result = services
        .update_subscription_slots_for_test(
            TENANT_ID,
            subscription_id,
            slot_component_id,
            5,
            SlotUpgradeBillingMode::Optimistic,
            NaiveDate::from_ymd_opt(2024, 1, 15).map(|t| t.and_time(NaiveTime::MIN)),
        )
        .await
        .expect("five more seats");
    let invoice = store
        .get_invoice_by_id(
            TENANT_ID,
            result.invoice_id.expect("the purchase's invoice"),
        )
        .await
        .expect("invoice");
    let charged: i64 = invoice.line_items.iter().map(|l| l.amount_subtotal).sum();
    assert!(
        (2193..=2194).contains(&charged),
        "5 seats at the second tier for 17 of 31 days, got {charged}"
    );
}

/// Three seats free, then 39.00 each: going from 2 to 3 seats costs nothing, and a fourth bought
/// after costs one seat at 39.00 for the days left, as a fifth with it costs two.
#[tokio::test]
async fn seats_bought_past_free_ones_cost_their_tier_after_a_free_purchase() {
    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;
    let setup = meteroid_it::container::start_meteroid_with_clients(
        postgres_connection_string,
        SeedLevel::PLANS,
        Arc::new(MockUsageClient::noop()),
        Arc::new(MockMailerService::new()),
    )
    .await;
    let (services, store) = (setup.services.clone(), setup.store.clone());
    let tier = |first_unit, rate: &str| TierRow {
        first_unit,
        rate: Decimal::from_str(rate).unwrap(),
        flat_fee: None,
        flat_cap: None,
    };
    let (subscription_id, slot_component_id) = create_subscription_with_seat_model(
        &services,
        RatingModel::Tiered {
            tiers: vec![tier(0, "0"), tier(3, "39.00")],
        },
        Some(1),
        Some(100),
        2,
    )
    .await;
    let charged = |result: &UpdateSlotsResult| {
        let store = store.clone();
        let invoice_id = result.invoice_id;
        async move {
            match invoice_id {
                None => 0,
                Some(id) => store
                    .get_invoice_by_id(TENANT_ID, id)
                    .await
                    .expect("invoice")
                    .line_items
                    .iter()
                    .map(|l| l.amount_subtotal)
                    .sum::<i64>(),
            }
        }
    };
    let buy = |delta: i32, day: u32| {
        services.update_subscription_slots_for_test(
            TENANT_ID,
            subscription_id,
            slot_component_id,
            delta,
            SlotUpgradeBillingMode::Optimistic,
            NaiveDate::from_ymd_opt(2024, 1, day).map(|t| t.and_time(NaiveTime::MIN)),
        )
    };

    let third = buy(1, 10).await.expect("a third seat");
    assert_eq!(charged(&third).await, 0, "the third seat is free");
    let fourth = buy(1, 12).await.expect("a fourth seat");
    assert_eq!(
        charged(&fourth).await,
        2516,
        "one seat at 39.00 for 20 of 31 days"
    );
}

/// The creation preview prices seats at a custom price with the count sent: 4 × 7.00, not the
/// plan's 10.00 a seat.
#[tokio::test]
async fn the_creation_preview_bills_custom_priced_seats_for_their_count() {
    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;
    let setup = meteroid_it::container::start_meteroid_with_clients(
        postgres_connection_string,
        SeedLevel::PLANS,
        Arc::new(MockUsageClient::noop()),
        Arc::new(MockMailerService::new()),
    )
    .await;
    let rate = Decimal::from_str("7.00").unwrap();
    let request = seats_at_custom_price(
        chrono::Utc::now().date_naive(),
        RatingModel::PerUnit { rate },
        None,
        None,
        4,
    );

    let (content, details) = setup
        .services
        .preview_create_subscription(&request, TENANT_ID)
        .await
        .expect("preview");

    let seats: Vec<_> = details
        .price_components
        .iter()
        .filter(|c| c.price_component_id == Some(COMP_NOTION_SEATS_ID))
        .collect();
    assert_eq!(seats.len(), 1, "{seats:?}");
    assert!(
        matches!(
            &seats[0].fee,
            meteroid_store::domain::SubscriptionFee::Slot {
                model: RatingModel::PerUnit { rate: r },
                initial_slots: 4,
                ..
            } if *r == rate
        ),
        "{:?}",
        seats[0].fee
    );

    assert_eq!(
        content
            .invoice_lines
            .iter()
            .map(|l| (l.quantity, l.amount_subtotal))
            .collect::<Vec<_>>(),
        vec![(Some(Decimal::from(4)), 2800)],
        "{:?}",
        content.invoice_lines
    );
}

async fn test_optimistic_upgrade(services: &Services, store: &Store) {
    let (subscription_id, slot_component_id) = create_subscription_with_slots(
        services,
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

    // Exact prorated amount. The subscription period is 2024-01-01 .. 2024-02-01
    // (31 days); the +5 seat upgrade lands on 2024-01-15, leaving 17 days. The
    // correct prorated charge is 5 × €10 × 17/31 = €27.42 (2742 minor units). The
    // pre-fix code multiplied by `delta` a second time and would have charged
    // €137.10 (5² × €10 × 17/31) — this assertion catches that regression.
    assert_eq!(
        line_item.end_date,
        NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
        "test assumes a January monthly billing period"
    );
    assert_eq!(
        line_item.amount_subtotal, 2742,
        "Prorated seat charge must be delta×rate×factor, not delta²×rate×factor"
    );

    let count = store
        .get_active_slots_value(TENANT_ID, subscription_id, QUANTITY_SEATS_ID, None)
        .await
        .expect("Failed to get current slots");

    assert_eq!(count, 15);
}

async fn test_on_invoice_paid_upgrade(services: &Services, store: &Store) {
    let (subscription_id, slot_component_id) = create_subscription_with_slots(
        services,
        Decimal::from_str("15.00").unwrap(),
        Some(1),
        Some(100),
        5,
    )
    .await;

    let result = services
        .update_subscription_slots_for_test(
            TENANT_ID,
            subscription_id,
            slot_component_id,
            3, // Add 3 slots
            SlotUpgradeBillingMode::OnInvoicePaid,
            NaiveDate::from_ymd_opt(2024, 1, 2).map(|t| t.and_time(NaiveTime::MIN)),
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
            QUANTITY_SEATS_ID,
            NaiveDate::from_ymd_opt(2024, 1, 30).map(|t| t.and_time(NaiveTime::MIN)),
        )
        .await
        .expect("Failed to get slots after activation");
    assert_eq!(
        count_after_activation, 8,
        "Should have 8 active slots after pending activation"
    );
}

async fn test_on_checkout_preview(services: &Services, _store: &Store, _conn: &mut PgConn) {
    let (subscription_id, slot_component_id) = create_subscription_with_slots(
        services,
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
            common_domain::actor::Actor::System,
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
        Decimal::from_str("12.00").unwrap(),
        Some(1),
        Some(100),
        20,
    )
    .await;

    let initial_count = store
        .get_active_slots_value(TENANT_ID, subscription_id, QUANTITY_SEATS_ID, None)
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
            common_domain::actor::Actor::System,
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

    // Deferred to the period's end, Feb 1, 2024: held until then, removed from it (R-108).
    let count_on = |day: u32, month: u32| {
        store.get_active_slots_value(
            TENANT_ID,
            subscription_id,
            QUANTITY_SEATS_ID,
            NaiveDate::from_ymd_opt(2024, month, day).map(|d| d.and_time(NaiveTime::MIN)),
        )
    };
    assert_eq!(
        count_on(31, 1).await.expect("Failed to get current slots"),
        initial_count,
        "Slots should not change until period end"
    );
    assert_eq!(
        count_on(1, 2)
            .await
            .expect("Failed to get slots at the period end"),
        initial_count - 5
    );
}

async fn test_min_max_limits(services: &Services, _store: &Store, _conn: &mut PgConn) {
    let (subscription_id, slot_component_id) = create_subscription_with_slots(
        services,
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
            common_domain::actor::Actor::System,
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
            common_domain::actor::Actor::System,
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
        Decimal::from_str("25.50").unwrap(), // Price with cents
        Some(1),
        Some(100),
        5,
    )
    .await;

    let result = services
        .update_subscription_slots_for_test(
            TENANT_ID,
            subscription_id,
            slot_component_id,
            1,
            SlotUpgradeBillingMode::Optimistic,
            NaiveDate::from_ymd_opt(2024, 1, 15).map(|t| t.and_time(NaiveTime::MIN)),
        )
        .await
        .expect("Failed to upgrade slots");

    let invoice = store
        .get_invoice_by_id(TENANT_ID, result.invoice_id.unwrap())
        .await
        .expect("Failed to get invoice");

    // Five 25.50 seats, a sixth from Jan 15, 17 of 31 days left: 15300 × 17/31 (8390.32 → 8390)
    // − 12750 × 17/31 (6991.94 → 6992) = 1398 minor units (R-079).
    let line_item = &invoice.line_items[0];
    assert_eq!(line_item.quantity, Some(Decimal::from(1)));
    assert_eq!(line_item.amount_subtotal, 1398);
    assert_eq!(invoice.subtotal, 1398);
}

async fn test_input_validation(services: &Services, _store: &Store, _conn: &mut PgConn) {
    let (subscription_id, slot_component_id) = create_subscription_with_slots(
        services,
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
            common_domain::actor::Actor::System,
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
            common_domain::actor::Actor::System,
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
        Decimal::from_str("8.00").unwrap(),
        Some(1),
        Some(100),
        20,
    )
    .await;

    let initial_count = store
        .get_active_slots_value(TENANT_ID, subscription_id, QUANTITY_SEATS_ID, None)
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
            QUANTITY_SEATS_ID,
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
    // Create subscription starting on 2024-01-01 with 5 slots
    let (subscription_id, slot_component_id) = create_subscription_with_slots(
        services,
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
            QUANTITY_SEATS_ID,
            NaiveDate::from_ymd_opt(2024, 1, 1).map(|t| t.and_time(NaiveTime::MIN)),
        )
        .await
        .expect("Failed to get initial slots");
    assert_eq!(slots, 5, "Should start with 5 slots");

    // Check initial invoice count (should have 1 invoice for initial subscription)
    let mut invoices = get_invoices_for_subscription(store, subscription_id, SLOT_UNIT).await;
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
            QUANTITY_SEATS_ID,
            NaiveDate::from_ymd_opt(2024, 1, 15).map(|t| t.and_time(NaiveTime::MIN)),
        )
        .await
        .expect("Failed to get slots after upgrade");
    assert_eq!(slots, 7, "Should have 7 active slots");

    invoices = get_invoices_for_subscription(store, subscription_id, SLOT_UNIT).await;
    assert_eq!(invoices.len(), 2, "Should have 2 invoices after upgrade");

    // Verify the upgrade invoice has correct details
    let upgrade_invoice = &invoices[1]; // Most recent
    let upgrade_line = upgrade_invoice
        .line_items
        .iter()
        .find(|line| line.name.contains(SLOT_UNIT))
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
            QUANTITY_SEATS_ID,
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
    invoices = get_invoices_for_subscription(store, subscription_id, SLOT_UNIT).await;
    assert_eq!(
        invoices.len(),
        2,
        "Should still have 2 invoices (no invoice for deferred downgrade)"
    );

    services.run_subscription_steps().await.unwrap();
    services.run_invoice_deadlines().await.unwrap();

    // Verify the downgrade has taken effect (7 - 5 = 2)
    let slots = store
        .get_active_slots_value(
            TENANT_ID,
            subscription_id,
            QUANTITY_SEATS_ID,
            NaiveDate::from_ymd_opt(2024, 2, 1).map(|t| t.and_time(NaiveTime::MIN)),
        )
        .await
        .expect("Failed to get slots after period transition");
    assert_eq!(slots, 2, "Should have 2 slots after downgrade takes effect");

    // Check invoice count after billing cycle (should have 3 invoices now - new period invoice)
    invoices = get_invoices_for_subscription(store, subscription_id, SLOT_UNIT).await;
    assert_eq!(
        invoices.len(),
        3,
        "Should have 3 invoices after billing cycle"
    );

    let new_period_invoice = &invoices[2]; // Most recent
    let _new_period_line = new_period_invoice
        .line_items
        .iter()
        .find(|line| line.name.contains(SLOT_UNIT))
        .expect("Should have slot line item");
}

async fn test_mixed_pending_and_active(services: &Services, store: &Store) {
    // Create subscription starting with 10 slots
    let (subscription_id, slot_component_id) = create_subscription_with_slots(
        services,
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
            QUANTITY_SEATS_ID,
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
            QUANTITY_SEATS_ID,
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
            QUANTITY_SEATS_ID,
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
            QUANTITY_SEATS_ID,
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
            QUANTITY_SEATS_ID,
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
        .get_active_slots_value(TENANT_ID, subscription_id, QUANTITY_SEATS_ID, None)
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
    _unit: &str,
) -> Vec<meteroid_store::domain::Invoice> {
    use meteroid_store::domain::PaginationRequest;

    let all_invoices = store
        .list_invoices(
            TENANT_ID,
            None,
            Some(subscription_id),
            None,
            None,
            None,
            false,
            None,
            vec![],
            Some("created_at.asc".to_string()),
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
                .any(|line| line.name.contains(SLOT_UNIT))
        })
        .map(|inv_with_customer| inv_with_customer.invoice)
        .collect()
}

async fn create_subscription_with_slots(
    services: &Services,
    unit_rate: Decimal,
    min_slots: Option<u32>,
    max_slots: Option<u32>,
    initial_slots: u32,
) -> (SubscriptionId, PriceComponentId) {
    create_subscription_with_seat_model(
        services,
        RatingModel::PerUnit { rate: unit_rate },
        min_slots,
        max_slots,
        initial_slots,
    )
    .await
}

async fn create_subscription_with_seat_model(
    services: &Services,
    model: RatingModel,
    min_slots: Option<u32>,
    max_slots: Option<u32>,
    initial_slots: u32,
) -> (SubscriptionId, PriceComponentId) {
    let subscription_id = services
        .insert_subscription(
            common_domain::actor::Actor::System,
            seats_at_custom_price(
                NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                model,
                min_slots,
                max_slots,
                initial_slots,
            ),
            TENANT_ID,
        )
        .await
        .expect("Failed to create subscription")
        .id;

    (subscription_id, COMP_NOTION_SEATS_ID)
}

/// Notion's seats at a custom price, with a seat count.
fn seats_at_custom_price(
    start_date: NaiveDate,
    model: RatingModel,
    min_slots: Option<u32>,
    max_slots: Option<u32>,
    initial_slots: u32,
) -> CreateSubscription {
    CreateSubscription {
        option: None,
        subscription: SubscriptionNew {
            customer_id: CUST_UBER_ID,
            plan_version_id: PLAN_VERSION_NOTION_ID,
            net_terms: None,
            invoice_memo: None,
            invoice_threshold: None,
            start_date,
            end_date: None,
            billing_start_date: None,
            activation_condition: meteroid_store::domain::SubscriptionActivationCondition::OnStart,
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
            external_id: None,
        },
        price_components: Some(CreateSubscriptionComponents {
            parameterized_components: vec![ComponentParameterization {
                component_id: COMP_NOTION_SEATS_ID,
                parameters: ComponentParameters {
                    initial_slot_count: Some(initial_slots),
                    billing_period: None,
                    committed_capacity: None,
                },
            }],
            overridden_components: vec![ComponentOverride {
                component_id: COMP_NOTION_SEATS_ID,
                name: "Seats".to_string(),
                price_entry: PriceEntry::New(PriceInput {
                    cadence: BillingPeriodEnum::Monthly,
                    currency: "EUR".to_string(),
                    pricing: Pricing::Slot {
                        model,
                        min_slots,
                        max_slots,
                    },
                    credit_unit: None,
                }),
            }],
            extra_components: vec![],
            remove_components: vec![],
        }),
        add_ons: None,
        coupons: None,
        entitlements: vec![],
        minimum_override: None,
        drop_minimum: false,
    }
}
