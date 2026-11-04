use super::*;

#[tokio::test]
async fn customer_summary_and_subscription_usage() {
    let usage = MutableMockUsageClient::new();
    let env = test_env_with_usage(Arc::new(usage.clone())).await;
    let limited = limited_uber(&env).await;
    let addon = active_subscription(
        &env,
        CUST_UBER_ID,
        date(2025, 6, 1),
        "EUR",
        2_000,
        (date(2026, 9, 10), date(2026, 10, 10)),
    )
    .await;
    set_usage(&usage, cycle_start().into(), now(), 850);
    let invoice = failing_payment(&env, limited).await;

    let uber = summary(&env, CUST_UBER_ID, ALL_PARTS).await;
    assert_eq!(uber.reporting_currency, "EUR");
    assert_eq!(uber.mrr_cents, 3_000);
    let receivables = uber.receivables.as_ref().expect("invoices:read");
    let totals = |t: &[meteroid_store::domain::customer_summary::MoneyTotal]| {
        t.iter()
            .map(|m| (m.currency.clone(), m.amount_cents, m.invoice_count))
            .collect::<Vec<_>>()
    };
    assert_eq!(totals(&receivables.open), vec![("EUR".into(), 30_000, 1)]);
    assert_eq!(
        totals(&receivables.overdue),
        vec![("EUR".into(), 30_000, 1)]
    );
    assert_eq!(receivables.last_failure_invoice_id, Some(invoice));
    assert_eq!(
        receivables.last_failure_reason.as_deref(),
        Some("card_declined")
    );
    assert_eq!(receivables.next_retry_at, Some(at(2026, 9, 18, 9)));
    let term = uber.term.as_ref().expect("subscriptions:read");
    assert_eq!(
        (term.subscription_id, term.date, term.mode),
        (limited, date(2026, 10, 1), RenewalMode::Renewing),
        "the earliest renewal, monthly included"
    );
    assert_eq!(term.cancellation_event_id, None);

    let limited_usage = env
        .services()
        .get_subscription_usage(TENANT_ID, limited, now())
        .await
        .unwrap();
    let cycle = limited_usage.cycle.as_ref().unwrap();
    assert_eq!(
        (cycle.period_start, cycle.period_end, cycle.days_left),
        (cycle_start(), Some(date(2026, 10, 1)), Some(16))
    );
    assert!(cycle.elapsed_pct > 40.0 && cycle.elapsed_pct < 60.0);
    assert_eq!(limited_usage.usage.len(), 1);
    let bar = &limited_usage.usage[0];
    assert_eq!(
        (bar.consumed, bar.limit, bar.pct),
        (Decimal::from(850), Some(Decimal::from(1_000)), Some(85.0))
    );
    assert_eq!(
        (
            bar.reset.as_str(),
            bar.warning_threshold_pct,
            bar.overage_behavior.as_str()
        ),
        ("billing_cycle", 80, "allow")
    );
    assert_eq!(bar.period_start, Some(cycle_start()));
    assert_eq!(bar.resets_at, Some(at(2026, 10, 1, 0)));
    assert_eq!(bar.projected, Some("1758.6207".parse().unwrap()));

    let addon_usage = env
        .services()
        .get_subscription_usage(TENANT_ID, addon, now())
        .await
        .unwrap();
    assert!(addon_usage.usage.is_empty(), "no limited entitlement");
    assert_eq!(addon_usage.cycle.unwrap().days_left, Some(25));

    let other_tenant = env
        .services()
        .get_subscription_usage(TENANT_ID, SubscriptionId::new(), now())
        .await
        .unwrap_err();
    assert!(matches!(
        other_tenant.current_context(),
        StoreError::ValueNotFound(_)
    ));
}

#[rstest]
#[tokio::test]
async fn customer_summary_leaves_out_what_the_caller_may_not_see(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let sub = active_subscription(
        &env,
        CUST_UBER_ID,
        date(2024, 1, 1),
        "EUR",
        10_000,
        (cycle_start(), date(2026, 10, 1)),
    )
    .await;
    failing_payment(&env, sub).await;

    let full = summary(&env, CUST_UBER_ID, ALL_PARTS).await;
    assert!(full.receivables.is_some() && full.term.is_some());

    let no_invoices = summary(
        &env,
        CUST_UBER_ID,
        SummaryParts {
            invoices: false,
            ..ALL_PARTS
        },
    )
    .await;
    assert!(no_invoices.receivables.is_none());
    assert!(no_invoices.term.is_some());
    assert_eq!(no_invoices.mrr_cents, full.mrr_cents);

    let no_subscriptions = summary(
        &env,
        CUST_UBER_ID,
        SummaryParts {
            subscriptions: false,
            ..ALL_PARTS
        },
    )
    .await;
    assert!(no_subscriptions.term.is_none());
    assert!(no_subscriptions.receivables.is_some());

    let missing = env
        .services()
        .get_customer_summary(TENANT_ID, CustomerId::new(), ALL_PARTS, now())
        .await
        .unwrap_err();
    assert!(matches!(
        missing.current_context(),
        StoreError::ValueNotFound(_)
    ));
}

#[rstest]
#[tokio::test]
async fn customer_term_names_the_pending_cancellation(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let cancelled = plan_subscription(
        &env,
        CUST_UBER_ID,
        PLAN_VERSION_1_LEETCODE_ID,
        3_500,
        date(2026, 11, 1),
    )
    .await;
    env.services()
        .cancel_subscription(
            Actor::System,
            cancelled,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(date(2026, 10, 5)),
        )
        .await
        .unwrap();

    let term = summary(&env, CUST_UBER_ID, ALL_PARTS)
        .await
        .term
        .expect("an ending subscription");
    assert_eq!(
        (term.subscription_id, term.date, term.mode),
        (cancelled, date(2026, 10, 5), RenewalMode::Ending)
    );
    let event = term
        .cancellation_event_id
        .expect("the pending cancellation");
    use diesel_models::schema::scheduled_event::dsl as se;
    let event_type: diesel_models::enums::ScheduledEventTypeEnum = se::scheduled_event
        .filter(se::id.eq(event))
        .select(se::event_type)
        .first(&mut env.conn().await)
        .await
        .unwrap();
    assert_eq!(
        event_type,
        diesel_models::enums::ScheduledEventTypeEnum::CancelSubscription
    );
}

#[rstest]
#[tokio::test]
async fn customer_mrr_is_summed_per_currency_then_converted(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let quarter = (date(2026, 7, 1), date(2026, 10, 1));
    for _ in 0..2 {
        active_subscription(&env, CUST_UBER_ID, date(2024, 1, 1), "USD", 5_001, quarter).await;
    }

    let fx = reporting_fx(env.store(), TENANT_ID, now().date())
        .await
        .unwrap();
    let uber = summary(&env, CUST_UBER_ID, ALL_PARTS).await;
    assert_eq!(uber.reporting_currency, "EUR");
    assert_eq!(
        uber.mrr_cents,
        fx.convert(10_002, "USD").unwrap(),
        "summed per currency, then converted"
    );
    assert_ne!(
        uber.mrr_cents,
        2 * fx.convert(5_001, "USD").unwrap(),
        "the fixture tells both roundings apart"
    );
}
