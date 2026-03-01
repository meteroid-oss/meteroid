use chrono::NaiveDate;
use std::sync::Arc;

use crate::data::ids::*;
use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use common_domain::ids::SubscriptionId;
use diesel_models::enums::{CycleActionEnum, SubscriptionEventType, SubscriptionStatusEnum};
use diesel_models::subscription_events::SubscriptionEventRow;
use diesel_models::subscriptions::SubscriptionRow;
use meteroid_mailer::service::MockMailerService;
use meteroid_store::clients::usage::MockUsageClient;
use meteroid_store::domain::enums::InvoiceStatusEnum;
use meteroid_store::domain::{
    CreateSubscription, CustomerNew, Invoice, OrderByRequest, PaginationRequest,
    PaymentMethodsConfig, SubscriptionActivationCondition, SubscriptionNew,
};
use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;
use meteroid_store::repositories::{CustomersInterface, InvoiceInterface};
use meteroid_store::store::PgConn;
use meteroid_store::{Services, Store};

#[tokio::test]
async fn test_lifecycle_billing() {
    helpers::init::logging();
    let postgres_connection_string = meteroid_it::container::create_test_database().await;

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

    // Monthly subscription with billing day anchor
    test_monthly_subscription_with_billing_anchor(&services, &store, &mut conn).await;

    // Anniversary billing (no anchor)
    test_anniversary_billing(&services, &store, &mut conn).await;

    // Subscription with end date
    test_subscription_with_end_date(&services, &store, &mut conn).await;

    // Subscription cancellation
    test_subscription_cancellation(&services, &store, &mut conn).await;

    // Race condition / locking
    test_subscription_cancellation_race_condition(&services, &store, &mut conn).await;

    test_issuing(&services, &store, mock_mailer.clone(), &mut conn).await;

    // MRR overflow regression tests
    test_mrr_cancellation_after_multiple_cycles(&services, &store, &mut conn).await;
    test_mrr_cancellation_at_period_boundary(&services, &store, &mut conn).await;

    // TODO next tests :
    // - ubb
    // - late payments, retries etc
}

async fn test_issuing(
    services: &Services,
    store: &Store,
    _mock_mailer: Arc<MockMailerService>,
    conn: &mut PgConn,
) {
    log::info!(">>> Testing issuing");

    // we insert a customer with an invoicing email
    let _inserted = store
        .insert_customer(
            CustomerNew {
                name: "".to_string(),
                alias: None,
                billing_email: None,
                invoicing_emails: vec!["mock@meteroid.com".to_string()],
                phone: None,
                balance_value_cents: 0,
                currency: "EUR".to_string(),
                billing_address: None,
                shipping_address: None,
                created_by: Default::default(),
                invoicing_entity_id: None,
                force_created_date: None,
                is_tax_exempt: false,
                vat_number: None,
                custom_taxes: vec![],
            },
            TENANT_ID,
        )
        .await
        .unwrap();

    // we insert a subscription
    let _subscription_id = create_subscription(
        services,
        SubscriptionParams {
            start_date: NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(),
            ..Default::default()
        },
    )
    .await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 31).unwrap();
    let expected_total = 3500;

    let invoice_dates = [start_date, NaiveDate::from_ymd_opt(2024, 2, 29).unwrap()];

    let subscription_id = create_subscription(
        services,
        SubscriptionParams {
            start_date,
            ..Default::default()
        },
    )
    .await;

    let subscription = get_subscription_row(conn, subscription_id).await;
    assert_subscription_state(&subscription, 0, &invoice_dates, expected_total);

    let invoices = get_invoices(store, subscription_id).await;
    assert_eq!(invoices.len(), 1);
    assert_full_invoice(&invoices[0], invoice_dates[0], expected_total);

    // we expect the fisrt invoice to be issued

    services.get_and_process_cycle_transitions().await.unwrap();
    services.get_and_process_due_events().await.unwrap();

    // we expect the second invoice to be issued
}

async fn test_monthly_subscription_with_billing_anchor(
    services: &Services,
    store: &Store,
    conn: &mut PgConn,
) {
    log::info!(">>> Testing monthly subscription with billing day anchor");

    let start_date = NaiveDate::from_ymd_opt(2024, 3, 1).unwrap();
    let billing_day = 15u16;
    let expected_total = 3500;

    let invoice_dates = [
        start_date,
        NaiveDate::from_ymd_opt(2024, 3, 15).unwrap(),
        NaiveDate::from_ymd_opt(2024, 4, 15).unwrap(),
        NaiveDate::from_ymd_opt(2024, 5, 15).unwrap(),
        NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(),
        NaiveDate::from_ymd_opt(2024, 7, 15).unwrap(),
    ];

    let subscription_id = create_subscription(
        services,
        SubscriptionParams {
            start_date,
            billing_day_anchor: Some(billing_day),
            ..Default::default()
        },
    )
    .await;

    // Test first cycle (prorated)
    let subscription = get_subscription_row(conn, subscription_id).await;
    assert_subscription_state(&subscription, 0, &invoice_dates, expected_total);

    let invoices = get_invoices(store, subscription_id).await;
    assert_eq!(invoices.len(), 1);
    assert_prorated_invoice(&invoices[0], invoice_dates[0], expected_total);

    // Test subsequent cycles
    for cycle in 1..=2 {
        log::info!(">>> Cycle {}", cycle + 1);

        services.get_and_process_cycle_transitions().await.unwrap();
        if cycle == 1 {
            // Check draft state after transition but before processing due events
            let invoices = get_invoices(store, subscription_id).await;
            assert_eq!(invoices[1].status, InvoiceStatusEnum::Draft);
        }

        services.get_and_process_due_events().await.unwrap();

        let subscription = get_subscription_row(conn, subscription_id).await;
        assert_subscription_state(&subscription, cycle, &invoice_dates, expected_total);

        let invoices = get_invoices(store, subscription_id).await;
        assert_eq!(invoices.len(), cycle + 1);
        assert_full_invoice(&invoices[cycle], invoice_dates[cycle], expected_total);
    }
}

async fn test_anniversary_billing(services: &Services, store: &Store, conn: &mut PgConn) {
    log::info!(">>> Testing anniversary billing");

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 31).unwrap();
    let expected_total = 3500;

    let invoice_dates = [
        start_date,
        NaiveDate::from_ymd_opt(2024, 2, 29).unwrap(),
        NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
        NaiveDate::from_ymd_opt(2024, 4, 30).unwrap(),
    ];

    let subscription_id = create_subscription(
        services,
        SubscriptionParams {
            start_date,
            ..Default::default()
        },
    )
    .await;

    // Test cycles without proration
    for cycle in 0..=2 {
        if cycle > 0 {
            services.get_and_process_cycle_transitions().await.unwrap();
            services.get_and_process_due_events().await.unwrap();
        }

        let subscription = get_subscription_row(conn, subscription_id).await;
        assert_subscription_state(&subscription, cycle, &invoice_dates, expected_total);

        let invoices = get_invoices(store, subscription_id).await;
        assert_eq!(invoices.len(), cycle + 1);
        assert_full_invoice(&invoices[cycle], invoice_dates[cycle], expected_total);
    }
}

async fn test_subscription_with_end_date(services: &Services, store: &Store, conn: &mut PgConn) {
    log::info!(">>> Testing subscription with end date");

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 31).unwrap();
    let end_date = NaiveDate::from_ymd_opt(2024, 3, 10).unwrap();
    let expected_total = 3500;

    let invoice_dates = [start_date, NaiveDate::from_ymd_opt(2024, 2, 29).unwrap()];

    let subscription_id = create_subscription(
        services,
        SubscriptionParams {
            start_date,
            end_date: Some(end_date),
            ..Default::default()
        },
    )
    .await;

    // First cycle
    let subscription = get_subscription_row(conn, subscription_id).await;
    assert_period(&subscription, invoice_dates[0], invoice_dates[1]);
    assert_eq!(subscription.status, SubscriptionStatusEnum::Active);
    assert_eq!(
        subscription.next_cycle_action.unwrap(),
        CycleActionEnum::RenewSubscription
    );
    assert_eq!(subscription.mrr_cents, expected_total);

    let invoices = get_invoices(store, subscription_id).await;
    assert_eq!(invoices.len(), 1);
    assert_full_invoice(&invoices[0], invoice_dates[0], expected_total);

    // Second cycle - should set up for end
    services.get_and_process_cycle_transitions().await.unwrap();
    services.get_and_process_due_events().await.unwrap();

    let subscription = get_subscription_row(conn, subscription_id).await;
    assert_period(&subscription, invoice_dates[1], end_date);
    assert_eq!(subscription.status, SubscriptionStatusEnum::Active);
    assert_eq!(
        subscription.next_cycle_action.unwrap(),
        CycleActionEnum::EndSubscription
    );
    assert_eq!(subscription.mrr_cents, expected_total);

    let invoices = get_invoices(store, subscription_id).await;
    assert_eq!(invoices.len(), 2);
    assert_full_invoice(&invoices[1], invoice_dates[1], expected_total);

    // Final cycle - subscription ends
    services.get_and_process_cycle_transitions().await.unwrap();
    services.get_and_process_due_events().await.unwrap();

    let subscription = get_subscription_row(conn, subscription_id).await;
    assert_eq!(subscription.status, SubscriptionStatusEnum::Completed);
    assert!(subscription.next_cycle_action.is_none());
    assert_eq!(subscription.mrr_cents, expected_total);

    let invoices = get_invoices(store, subscription_id).await;
    assert_eq!(invoices.len(), 2); // No new invoice after completion
}

async fn test_subscription_cancellation(services: &Services, store: &Store, conn: &mut PgConn) {
    log::info!(">>> Testing subscription cancellation");

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 31).unwrap();
    let cancel_date = NaiveDate::from_ymd_opt(2024, 3, 10).unwrap();
    let expected_total = 3500;

    let invoice_dates = [start_date, NaiveDate::from_ymd_opt(2024, 2, 29).unwrap()];

    let subscription_id = create_subscription(
        services,
        SubscriptionParams {
            start_date,
            ..Default::default()
        },
    )
    .await;

    // Schedule cancellation. Events are not processed until it is in current or previous cycle(s)
    services
        .cancel_subscription(
            subscription_id,
            TENANT_ID,
            Some("no reason".to_string()),
            CancellationEffectiveAt::Date(cancel_date),
            USER_ID,
        )
        .await
        .unwrap();

    // First cycle
    let subscription = get_subscription_row(conn, subscription_id).await;
    assert_eq!(subscription.status, SubscriptionStatusEnum::Active);
    assert_period(&subscription, invoice_dates[0], invoice_dates[1]);
    assert_eq!(subscription.mrr_cents, expected_total);

    let invoices = get_invoices(store, subscription_id).await;
    assert_eq!(invoices.len(), 1);
    assert_full_invoice(&invoices[0], invoice_dates[0], expected_total);

    // this should do nothing
    services.get_and_process_due_events().await.unwrap();
    let subscription = get_subscription_row(conn, subscription_id).await;
    assert_eq!(subscription.status, SubscriptionStatusEnum::Active);

    // Process second cycle which includes cancellation
    services.get_and_process_cycle_transitions().await.unwrap();

    // Before processing due events, subscription should still be active but ready for renewal
    let subscription = get_subscription_row(conn, subscription_id).await;
    assert_period(
        &subscription,
        invoice_dates[1],
        NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
    );
    assert_eq!(subscription.status, SubscriptionStatusEnum::Active);
    assert_eq!(
        subscription.next_cycle_action.unwrap(),
        CycleActionEnum::RenewSubscription
    );
    assert_eq!(subscription.mrr_cents, expected_total);

    let invoices = get_invoices(store, subscription_id).await;
    assert_eq!(invoices.len(), 2);

    // Process due events - this finalizes the invoice and processes the cancellation
    services.get_and_process_due_events().await.unwrap();

    let subscription = get_subscription_row(conn, subscription_id).await;
    assert_eq!(subscription.status, SubscriptionStatusEnum::Cancelled);
    assert!(subscription.next_cycle_action.is_none());
    assert_eq!(subscription.current_period_start, cancel_date);
    assert!(subscription.current_period_end.is_none());
    assert_eq!(subscription.mrr_cents, 0);

    let invoices = get_invoices(store, subscription_id).await;
    assert_eq!(invoices.len(), 2);
    assert!(
        invoices
            .iter()
            .all(|i| i.status == InvoiceStatusEnum::Finalized)
    );
    assert_full_invoice(&invoices[1], invoice_dates[1], expected_total);

    // Ensure no further changes after cancellation
    services.get_and_process_cycle_transitions().await.unwrap();
    services.get_and_process_due_events().await.unwrap();

    let invoices = get_invoices(store, subscription_id).await;
    assert_eq!(invoices.len(), 2);
    let subscription = get_subscription_row(conn, subscription_id).await;
    assert_eq!(subscription.status, SubscriptionStatusEnum::Cancelled);
}

async fn test_subscription_cancellation_race_condition(
    services: &Services,
    store: &Store,
    conn: &mut PgConn,
) {
    log::info!(">>> Testing subscription cancellation race condition");

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 31).unwrap();
    let cancel_date = NaiveDate::from_ymd_opt(2024, 2, 29).unwrap();
    let expected_total = 3500;

    let invoice_dates = [start_date, NaiveDate::from_ymd_opt(2024, 2, 29).unwrap()];

    let subscription_id = create_subscription(
        services,
        SubscriptionParams {
            start_date,
            ..Default::default()
        },
    )
    .await;

    let subscription = get_subscription_row(conn, subscription_id).await;
    assert_period(&subscription, invoice_dates[0], invoice_dates[1]);
    assert_eq!(subscription.status, SubscriptionStatusEnum::Active);
    assert_eq!(subscription.mrr_cents, expected_total);

    let invoices = get_invoices(store, subscription_id).await;
    assert_eq!(invoices.len(), 1);
    assert_full_invoice(&invoices[0], invoice_dates[0], expected_total);

    // Schedule cancellation
    services
        .cancel_subscription(
            subscription_id,
            TENANT_ID,
            Some("no reason".to_string()),
            CancellationEffectiveAt::Date(cancel_date),
            USER_ID,
        )
        .await
        .unwrap();

    let (events_processed, _) = tokio::try_join! {
        services.get_and_process_due_events(), // does nothing
        services.get_and_process_cycle_transitions(), // actually process the cancellation
    }
    .unwrap();

    assert_eq!(events_processed, 0); // if this starts failing, it can be because another test was changed & has an event on that date. If so, just isolate this test

    let subscription = get_subscription_row(conn, subscription_id).await;
    assert_eq!(subscription.status, SubscriptionStatusEnum::Cancelled);

    tokio::try_join! {
        services.get_and_process_due_events(), // does nothing
        services.get_and_process_cycle_transitions(), // does nothing
    }
    .unwrap();

    let invoices = get_invoices(store, subscription_id).await;
    assert_eq!(invoices.len(), 1);
    assert_full_invoice(&invoices[0], invoice_dates[0], expected_total);

    let subscription = get_subscription_row(conn, subscription_id).await;
    assert_eq!(subscription.status, SubscriptionStatusEnum::Cancelled);
    assert!(subscription.next_cycle_action.is_none());
    assert_eq!(subscription.current_period_start, cancel_date);
    assert!(subscription.current_period_end.is_none());
    assert_eq!(subscription.mrr_cents, 0);
}

/// Regression test for MRR overflow after cancellation.
///
/// The bug: create_churn_mrr_log in terminate.rs unconditionally applies
/// update_subscription_mrr_delta. If process_mrr (invoices.rs) also processes the
/// Cancelled subscription event (via an invoice inserted at the cancel date), the delta
/// is applied twice, making mrr_cents negative. The negative i64 then wraps to a huge
/// u64 (billions/quintillions) when converted to the domain Subscription type.
///
/// This test verifies:
/// 1. mrr_cents is exactly 0 after cancellation (no double-counting)
/// 2. mrr_cents is never negative (would cause u64 overflow)
/// 3. The Cancelled subscription event is properly linked to the BI MRR movement log
/// 4. Idempotency: repeated lifecycle processing doesn't change mrr_cents
async fn test_mrr_cancellation_after_multiple_cycles(
    services: &Services,
    store: &Store,
    conn: &mut PgConn,
) {
    log::info!(">>> Testing MRR cancellation after multiple billing cycles (overflow regression)");

    let start_date = NaiveDate::from_ymd_opt(2024, 5, 1).unwrap();
    let expected_mrr = 3500i64;

    let subscription_id = create_subscription(
        services,
        SubscriptionParams {
            start_date,
            ..Default::default()
        },
    )
    .await;

    // Verify initial MRR is positive and correct
    let subscription = get_subscription_row(conn, subscription_id).await;
    assert_eq!(subscription.mrr_cents, expected_mrr);
    assert!(
        subscription.mrr_cents >= 0,
        "MRR must never be negative (would overflow to u64::MAX)"
    );

    // Verify the Created event was linked to a BI MRR movement log via process_mrr
    let created_event = SubscriptionEventRow::fetch_by_subscription_id_and_event_type(
        conn,
        subscription_id,
        SubscriptionEventType::Created,
        start_date,
    )
    .await
    .unwrap();
    // process_mrr does NOT set bi_mrr_movement_log_id — this is a known gap
    // The Created event's MRR delta is applied by process_mrr on invoice insertion
    assert!(created_event.is_some());

    // Run through 3 billing cycles to build up state
    for cycle in 1..=3 {
        services.get_and_process_cycle_transitions().await.unwrap();
        services.get_and_process_due_events().await.unwrap();

        let subscription = get_subscription_row(conn, subscription_id).await;
        assert_eq!(subscription.status, SubscriptionStatusEnum::Active);
        assert_eq!(
            subscription.mrr_cents, expected_mrr,
            "MRR should remain stable across billing cycles (cycle {})",
            cycle
        );
        assert!(
            subscription.mrr_cents >= 0,
            "MRR must never be negative at cycle {}",
            cycle
        );
    }

    let invoices = get_invoices(store, subscription_id).await;
    assert_eq!(invoices.len(), 4); // initial + 3 renewals

    // Cancel mid-period with a specific date
    let cancel_date = NaiveDate::from_ymd_opt(2024, 8, 15).unwrap();
    services
        .cancel_subscription(
            subscription_id,
            TENANT_ID,
            Some("MRR overflow test".to_string()),
            CancellationEffectiveAt::Date(cancel_date),
            USER_ID,
        )
        .await
        .unwrap();

    // MRR should still be positive while cancellation is pending
    let subscription = get_subscription_row(conn, subscription_id).await;
    assert_eq!(subscription.mrr_cents, expected_mrr);

    // Process the cancellation through the lifecycle
    services.get_and_process_cycle_transitions().await.unwrap();
    services.get_and_process_due_events().await.unwrap();

    // CRITICAL: mrr_cents must be exactly 0, not negative (which would overflow to u64::MAX)
    let subscription = get_subscription_row(conn, subscription_id).await;
    assert_eq!(subscription.status, SubscriptionStatusEnum::Cancelled);
    assert_eq!(
        subscription.mrr_cents, 0,
        "MRR must be exactly 0 after cancellation, got {} (negative would overflow to billions as u64)",
        subscription.mrr_cents
    );
    assert!(
        subscription.mrr_cents >= 0,
        "MRR is negative ({}), would overflow to {} as u64",
        subscription.mrr_cents,
        subscription.mrr_cents as u64
    );

    // Verify the Cancelled event was linked to the BI MRR movement log by create_churn_mrr_log
    let cancelled_event = SubscriptionEventRow::fetch_by_subscription_id_and_event_type(
        conn,
        subscription_id,
        SubscriptionEventType::Cancelled,
        cancel_date,
    )
    .await
    .unwrap();
    assert!(
        cancelled_event.is_some(),
        "Cancelled subscription event should exist"
    );
    let cancelled_event = cancelled_event.unwrap();
    assert_eq!(cancelled_event.mrr_delta, Some(-expected_mrr));
    assert!(
        cancelled_event.bi_mrr_movement_log_id.is_some(),
        "Cancelled event should be linked to a BI MRR movement log (idempotency marker)"
    );

    // Idempotency: repeated processing must not change mrr_cents
    services.get_and_process_cycle_transitions().await.unwrap();
    services.get_and_process_due_events().await.unwrap();

    let subscription = get_subscription_row(conn, subscription_id).await;
    assert_eq!(subscription.status, SubscriptionStatusEnum::Cancelled);
    assert_eq!(
        subscription.mrr_cents, 0,
        "MRR must remain 0 after repeated lifecycle processing"
    );
}

/// Tests MRR handling when cancellation falls exactly on a period boundary.
///
/// This is an edge case because the cycle transition checks for scheduled events
/// at new_period_start. If the cancel date matches the period boundary, the cycle
/// transition processes the cancellation inline (without creating a renewal invoice first).
///
/// If the order of operations were reversed (bill first, check events second),
/// process_mrr would pick up the Cancelled event from the renewal invoice and
/// create_churn_mrr_log would apply the delta again — causing the double-counting bug.
async fn test_mrr_cancellation_at_period_boundary(
    services: &Services,
    store: &Store,
    conn: &mut PgConn,
) {
    log::info!(">>> Testing MRR cancellation at period boundary (overflow regression)");

    let start_date = NaiveDate::from_ymd_opt(2024, 6, 1).unwrap();
    let expected_mrr = 3500i64;

    let subscription_id = create_subscription(
        services,
        SubscriptionParams {
            start_date,
            ..Default::default()
        },
    )
    .await;

    // Run one billing cycle
    services.get_and_process_cycle_transitions().await.unwrap();
    services.get_and_process_due_events().await.unwrap();

    let subscription = get_subscription_row(conn, subscription_id).await;
    assert_eq!(subscription.mrr_cents, expected_mrr);
    let period_end = subscription.current_period_end.unwrap();

    // Cancel exactly at the period boundary.
    // We use Date(period_end) instead of EndOfBillingPeriod because the latter
    // computes the period relative to the real wall-clock date, which mismatches
    // our synthetic 2024 test dates.
    services
        .cancel_subscription(
            subscription_id,
            TENANT_ID,
            Some("period boundary test".to_string()),
            CancellationEffectiveAt::Date(period_end),
            USER_ID,
        )
        .await
        .unwrap();

    // Process: cycle transition should find the cancel event at the next period start
    // and handle it inline (without billing first)
    services.get_and_process_cycle_transitions().await.unwrap();
    services.get_and_process_due_events().await.unwrap();

    let subscription = get_subscription_row(conn, subscription_id).await;
    assert_eq!(subscription.status, SubscriptionStatusEnum::Cancelled);
    assert_eq!(
        subscription.mrr_cents,
        0,
        "MRR must be 0 after period-boundary cancellation, got {} (negative={}, as u64={})",
        subscription.mrr_cents,
        subscription.mrr_cents < 0,
        subscription.mrr_cents as u64
    );
    assert!(
        subscription.mrr_cents >= 0,
        "MRR went negative ({}) — double-counting bug in create_churn_mrr_log",
        subscription.mrr_cents
    );

    // Verify the cancel event's applies_to matches the period end
    let cancelled_event = SubscriptionEventRow::fetch_by_subscription_id_and_event_type(
        conn,
        subscription_id,
        SubscriptionEventType::Cancelled,
        period_end,
    )
    .await
    .unwrap();
    assert!(
        cancelled_event.is_some(),
        "Cancelled event should exist at period end date {}",
        period_end
    );
    let cancelled_event = cancelled_event.unwrap();
    assert_eq!(cancelled_event.mrr_delta, Some(-expected_mrr));
    assert!(
        cancelled_event.bi_mrr_movement_log_id.is_some(),
        "Cancelled event must be linked to BI MRR movement log"
    );

    // No further invoices should be created
    let invoices = get_invoices(store, subscription_id).await;
    assert!(
        invoices.len() <= 3,
        "No extra invoices should be created during termination"
    );

    // Idempotency
    services.get_and_process_cycle_transitions().await.unwrap();
    services.get_and_process_due_events().await.unwrap();

    let subscription = get_subscription_row(conn, subscription_id).await;
    assert_eq!(subscription.mrr_cents, 0);
}

// Helper functions
fn assert_subscription_state(
    subscription: &SubscriptionRow,
    cycle_idx: usize,
    invoice_dates: &[NaiveDate],
    expected_total: i64,
) {
    assert_eq!(subscription.status, SubscriptionStatusEnum::Active);
    assert_eq!(
        subscription.next_cycle_action.clone().unwrap(),
        CycleActionEnum::RenewSubscription
    );
    assert_period(
        subscription,
        invoice_dates[cycle_idx],
        invoice_dates[cycle_idx + 1],
    );
    assert_eq!(subscription.mrr_cents, expected_total);
}

fn assert_period(subscription: &SubscriptionRow, start: NaiveDate, end: NaiveDate) {
    assert_eq!(subscription.current_period_start, start);
    assert_eq!(subscription.current_period_end.unwrap(), end);
}

fn assert_prorated_invoice(invoice: &Invoice, invoice_date: NaiveDate, expected_total: i64) {
    assert_eq!(invoice.status, InvoiceStatusEnum::Finalized);
    assert_eq!(invoice.amount_due, invoice.total);
    assert_eq!(invoice.invoice_date, invoice_date);

    let is_prorated = invoice.line_items.iter().any(|line| line.is_prorated);
    assert!(is_prorated);
    assert!(invoice.total < expected_total && invoice.total > 0);
}

fn assert_full_invoice(invoice: &Invoice, invoice_date: NaiveDate, expected_total: i64) {
    assert_eq!(invoice.status, InvoiceStatusEnum::Finalized);
    assert_eq!(invoice.amount_due, invoice.total);
    assert_eq!(invoice.invoice_date, invoice_date);
    assert_eq!(invoice.total, expected_total);

    let is_prorated = invoice.line_items.iter().any(|line| line.is_prorated);
    assert!(!is_prorated);
}

async fn get_subscription_row(
    conn: &mut PgConn,
    subscription_id: SubscriptionId,
) -> SubscriptionRow {
    SubscriptionRow::get_subscription_by_id(conn, &TENANT_ID, subscription_id)
        .await
        .unwrap()
        .subscription
}

async fn get_invoices(store: &Store, subscription_id: SubscriptionId) -> Vec<Invoice> {
    store
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
        .unwrap()
        .items
        .into_iter()
        .map(|i| i.invoice)
        .collect()
}

async fn create_subscription(services: &Services, params: SubscriptionParams) -> SubscriptionId {
    services
        .insert_subscription(
            CreateSubscription {
                subscription: SubscriptionNew {
                    customer_id: CUST_UBER_ID,
                    plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
                    created_by: USER_ID,
                    net_terms: None,
                    invoice_memo: None,
                    invoice_threshold: None,
                    start_date: params.start_date,
                    end_date: params.end_date,
                    billing_start_date: params.billing_start_date,
                    activation_condition: params.activation_condition,
                    trial_duration: params.trial_duration,
                    billing_day_anchor: params.billing_day_anchor,
                    payment_methods_config: params.payment_methods_config,
                    auto_advance_invoices: true,
                    charge_automatically: false,
                    purchase_order: None,
                    backdate_invoices: false,
                    skip_checkout_session: false,
                    skip_past_invoices: false,
                },
                price_components: None,
                add_ons: None,
                coupons: None,
            },
            TENANT_ID,
        )
        .await
        .unwrap()
        .id
}

struct SubscriptionParams {
    pub start_date: chrono::NaiveDate,
    pub end_date: Option<chrono::NaiveDate>,
    pub billing_day_anchor: Option<u16>,
    pub activation_condition: SubscriptionActivationCondition,
    pub trial_duration: Option<u32>,
    pub payment_methods_config: Option<PaymentMethodsConfig>,
    pub billing_start_date: Option<NaiveDate>,
}

impl Default for SubscriptionParams {
    fn default() -> Self {
        Self {
            start_date: chrono::Utc::now().naive_utc().date(),
            end_date: None,
            billing_day_anchor: None,
            activation_condition: SubscriptionActivationCondition::OnStart,
            trial_duration: None,
            payment_methods_config: None,
            billing_start_date: None,
        }
    }
}
