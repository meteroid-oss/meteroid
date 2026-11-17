//! A database at main's schema with plan changes and amendments pending as scheduled events,
//! migrated by this binary in one run: each is written as the facts it becomes and moved to
//! `subscription_change`, whatever its tenant's status, and `scheduled_event` is dropped; all in
//! one transaction, which a failure rolls back to main's schema.
use chrono::Months;
use meteroid::migrations;
use meteroid_store::store::{PgConfig, diesel_make_pg_pool};
use meteroid_testkit::release::{
    AtMain, FIRST_AFTER_MAIN, at_main, catalog, pending_unresolvable, server,
};
use tokio_postgres::Client;
use uuid::Uuid;

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pending_changes_left_by_main_are_written_as_facts_in_one_deploy() -> TestResult {
    let (_container, base_url) = server("pending-changes").await;
    let AtMain {
        url,
        db,
        next,
        active,
        archived,
        sandbox,
        unresolvable,
    } = at_main(&base_url).await?;

    migrations::run(&diesel_make_pg_pool(PgConfig::new(url.clone()))?).await?;

    let table = db
        .query_one(
            "SELECT to_regclass('scheduled_event') IS NULL AS dropped",
            &[],
        )
        .await?;
    assert!(
        table.get::<_, bool>("dropped"),
        "scheduled_event is dropped"
    );
    let moved: Vec<String> = db
        .query(
            "SELECT id::text AS id FROM subscription_change
             WHERE written_facts IS NOT NULL AND withdrawn_at IS NULL ORDER BY id",
            &[],
        )
        .await?
        .iter()
        .map(|row| row.get("id"))
        .collect();
    let mut expected = [active.amendment, archived.amendment, sandbox.amendment];
    expected.sort();
    let expected: Vec<String> = expected.iter().map(Uuid::to_string).collect();
    assert_eq!(
        moved, expected,
        "each pending amendment, the archived and sandbox tenants' too"
    );
    for seeded in [&active, &archived, &sandbox] {
        let closed = db
            .query_one(
                &format!(
                    "SELECT effective_to::text AS effective_to FROM subscription_component
                     WHERE id = '{}'",
                    seeded.component
                ),
                &[],
            )
            .await?;
        assert_eq!(
            closed.get::<_, Option<String>>("effective_to"),
            Some(next.to_string()),
            "the amendment's fact: its component closes at the next period"
        );
    }
    let unwritten = db
        .query_one(
            &format!(
                "SELECT count(*) AS n FROM subscription_change WHERE id IN ('{}', '{}')
                   AND written_facts IS NULL AND withdrawn_at IS NOT NULL
                   AND withdrawn_reason LIKE 'previous release CANCELED: not written as facts: %'",
                unresolvable[0], unresolvable[1]
            ),
            &[],
        )
        .await?;
    assert_eq!(
        unwritten.get::<_, i64>("n"),
        2,
        "cancelled, kept as withdrawn history"
    );
    Ok(())
}

/// A change of a live tenant (active, in production) that cannot be written is never cancelled:
/// the release refuses, naming it, and nothing of it stays.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_live_tenants_unwritable_change_fails_the_release() -> TestResult {
    let (_container, base_url) = server("pending-changes").await;
    let at = at_main(&base_url).await?;
    let live = pending_unresolvable(&at.db, at.active.subscription, at.next).await;
    let before = catalog(&at.db).await;

    let failed = migrations::run(&diesel_make_pg_pool(PgConfig::new(at.url.clone()))?)
        .await
        .expect_err("the release refuses");
    let error = failed.to_string();
    assert!(
        error.contains(&live.to_string()) && error.contains("ComponentRowNotOpen"),
        "names the change and why: {error}"
    );
    assert!(
        error.contains("cancel each listed change") && !error.contains("pre-release"),
        "says what to do: {error}"
    );
    for cancelled_elsewhere in at.unresolvable {
        assert!(
            !error.contains(&cancelled_elsewhere.to_string()),
            "a tenant not live is not listed"
        );
    }

    assert_eq!(catalog(&at.db).await, before, "main's schema, as it was");
    assert_eq!(pending(&at.db).await?, 6, "every change still pending");
    Ok(())
}

async fn pending(db: &Client) -> Result<i64, tokio_postgres::Error> {
    db.query_one(
        "SELECT count(*) AS n FROM scheduled_event WHERE status = 'PENDING'",
        &[],
    )
    .await
    .map(|row| row.get::<_, i64>("n"))
}

/// The release fails at its last migration, after the facts backfill and the changes written as
/// facts (a view reads a column it drops): nothing of it stays, the pending changes and main's
/// schema untouched.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_release_failing_halfway_leaves_mains_schema() -> TestResult {
    let (_container, base_url) = server("pending-changes").await;
    let at = at_main(&base_url).await?;
    at.db
        .batch_execute(
            "CREATE VIEW subscription_cycles AS SELECT id, cycle_index FROM subscription",
        )
        .await?;
    let before = catalog(&at.db).await;
    assert_eq!(pending(&at.db).await?, 5);

    let failed = migrations::run(&diesel_make_pg_pool(PgConfig::new(at.url.clone()))?).await;
    assert!(failed.is_err(), "the release refuses");

    assert_eq!(catalog(&at.db).await, before, "main's schema, as it was");
    assert_eq!(pending(&at.db).await?, 5, "the changes still pending");
    let release = at
        .db
        .query_one(
            &format!(
                "SELECT count(*) AS n FROM __diesel_schema_migrations
                 WHERE version >= '{FIRST_AFTER_MAIN}'"
            ),
            &[],
        )
        .await?;
    assert_eq!(
        release.get::<_, i64>("n"),
        0,
        "no migration of the release recorded"
    );
    let facts = at
        .db
        .query_one(
            "SELECT count(*) AS n FROM subscription_component WHERE effective_to IS NOT NULL",
            &[],
        )
        .await?;
    assert_eq!(facts.get::<_, i64>("n"), 0, "no fact written");
    Ok(())
}

/// Main's `CheckoutSessionRow` columns, as its every read and write of a session names them.
const MAIN_CHECKOUT_SESSION_COLUMNS: &str = "id, tenant_id, customer_id, plan_version_id,
    billing_start_date, billing_day_anchor, net_terms, trial_duration_days, end_date,
    auto_advance_invoices, charge_automatically, invoice_memo, invoice_threshold, purchase_order,
    components, add_ons, coupon_code, coupon_ids, status, created_at, expires_at, completed_at,
    subscription_id, metadata, checkout_type, payment_methods_config, change_date, success_url,
    cancel_url, pre_payment_coupon_code, pre_payment_coupon_ids";

/// What a pod of main still running after the release commits would write with nothing left to
/// stop it: a self-serve checkout charging the card before failing to record the subscription,
/// and a matrix price edit re-rating every day billed from the price. Each fails first.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pod_of_main_fails_before_charging_a_checkout_or_rerating_a_price() -> TestResult {
    use tokio_postgres::error::SqlState;

    let (_container, base_url) = server("pending-changes").await;
    let at = at_main(&base_url).await?;
    let session = Uuid::now_v7();
    at.db
        .batch_execute(&format!(
            "INSERT INTO checkout_session (id, tenant_id, customer_id, auto_advance_invoices,
                 charge_automatically, coupon_ids, status, created_at, checkout_type)
             VALUES ('{session}', '{tenant}', '{customer}', true, true, '{{}}', 'CREATED', now(),
                     'SELF_SERVE')",
            tenant = at.active.tenant,
            customer = at.active.customer,
        ))
        .await?;

    migrations::run(&diesel_make_pg_pool(PgConfig::new(at.url.clone()))?).await?;

    // complete_checkout reads the session first; set_payment_coupons, still before the charge.
    let read = at
        .db
        .query(
            &format!(
                "SELECT {MAIN_CHECKOUT_SESSION_COLUMNS} FROM checkout_session
                 WHERE id = '{session}' AND tenant_id = '{}' FOR NO KEY UPDATE",
                at.active.tenant
            ),
            &[],
        )
        .await
        .expect_err("main's read of the session fails");
    assert_eq!(read.code(), Some(&SqlState::UNDEFINED_COLUMN));
    let coupons = at
        .db
        .query(
            &format!(
                "UPDATE checkout_session
                 SET pre_payment_coupon_code = CASE WHEN pre_payment_coupon_ids IS NULL
                         THEN coupon_code ELSE pre_payment_coupon_code END,
                     pre_payment_coupon_ids = COALESCE(pre_payment_coupon_ids, coupon_ids),
                     coupon_code = NULL, coupon_ids = '{{}}'
                 WHERE id = '{session}' AND tenant_id = '{}'
                 RETURNING {MAIN_CHECKOUT_SESSION_COLUMNS}",
                at.active.tenant
            ),
            &[],
        )
        .await
        .expect_err("main's coupons for the payment fail");
    assert_eq!(coupons.code(), Some(&SqlState::UNDEFINED_COLUMN));

    // update_matrix_prices on main rewrites the row's rates.
    let rerated = at
        .db
        .execute(
            &format!(
                "UPDATE price SET pricing = '{{\"rate\": \"12.00\", \"type\": \"Rate\"}}'
                 WHERE id = '{}' AND tenant_id = '{}'",
                at.active.price, at.active.tenant
            ),
            &[],
        )
        .await
        .expect_err("rates are never rewritten");
    assert_eq!(rerated.code(), Some(&SqlState::CHECK_VIOLATION));
    let archived = at
        .db
        .execute(
            &format!(
                "UPDATE price SET archived_at = now() WHERE id = '{}'",
                at.active.price
            ),
            &[],
        )
        .await?;
    assert_eq!(archived, 1, "archiving a price is no rate change");
    Ok(())
}

/// Main's dead events on a live tenant, failed, completed or cancelled: a cancellation, a retry of
/// a finalized unpaid invoice, the finalization of a draft. After the release none is a fact:
/// no ending, no deadline on either invoice.
/// Rules: R-016, R-080
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mains_dead_events_become_no_fact() -> TestResult {
    let (_container, base_url) = server("pending-changes").await;
    let at = at_main(&base_url).await?;
    let active = &at.active;
    let (finalized, draft) = (Uuid::now_v7(), Uuid::now_v7());
    let mut rows = Vec::new();
    for (invoice, status, period) in [
        (finalized, "FINALIZED", "CURRENT_DATE - 31"),
        (draft, "DRAFT", "CURRENT_DATE"),
    ] {
        rows.push(format!(
            "('{invoice}', now(), '{tenant}', '{customer}', '{subscription}', 'EUR', '[]',
              {period}, 1000, 30, 'INV-{invoice}', 0, 1000, '{{}}', 1000, 1000, '{{}}',
              '{entity}', {period}, '{status}')",
            tenant = active.tenant,
            customer = active.customer,
            subscription = active.subscription,
            entity = active.entity,
        ));
    }
    at.db
        .batch_execute(&format!(
            "INSERT INTO invoice (id, created_at, tenant_id, customer_id, subscription_id, currency,
                 line_items, invoice_date, total, net_terms, invoice_number, tax_amount,
                 subtotal_recurring, customer_details, amount_due, subtotal, seller_details,
                 invoicing_entity_id, billing_period_start, status)
             VALUES {}",
            rows.join(", ")
        ))
        .await?;
    let mut events = Vec::new();
    for status in ["FAILED", "COMPLETED", "CANCELED"] {
        for (kind, data) in [
            (
                "CANCEL_SUBSCRIPTION",
                r#"{"CancelSubscription": {"reason": null}}"#.to_string(),
            ),
            (
                "RETRY_PAYMENT",
                format!(r#"{{"RetryPayment": {{"invoice_id": "{finalized}"}}}}"#),
            ),
            (
                "FINALIZE_INVOICE",
                format!(r#"{{"FinalizeInvoice": {{"invoice_id": "{draft}"}}}}"#),
            ),
        ] {
            events.push(format!(
                "('{}', '{}', '{}', '{kind}', now() - interval '20 days', 0, '{data}', now(),
                  '{status}', 'api')",
                Uuid::now_v7(),
                active.subscription,
                active.tenant,
            ));
        }
    }
    at.db
        .batch_execute(&format!(
            "INSERT INTO scheduled_event (id, subscription_id, tenant_id, event_type,
                 scheduled_time, priority, event_data, created_at, status, source)
             VALUES {}",
            events.join(", ")
        ))
        .await?;

    migrations::run(&diesel_make_pg_pool(PgConfig::new(at.url.clone()))?).await?;

    let endings = at
        .db
        .query_one(
            &format!(
                "SELECT count(*) AS n FROM subscription_ending WHERE subscription_id = '{}'",
                active.subscription
            ),
            &[],
        )
        .await?;
    assert_eq!(endings.get::<_, i64>("n"), 0, "no ending");
    let deadlines = at
        .db
        .query_one(
            &format!(
                "SELECT count(*) AS n FROM invoice WHERE id IN ('{finalized}', '{draft}')
                   AND (finalize_after IS NOT NULL OR next_payment_attempt_at IS NOT NULL)"
            ),
            &[],
        )
        .await?;
    assert_eq!(deadlines.get::<_, i64>("n"), 0, "nothing due");
    Ok(())
}

/// Main's applied, cancelled and failed amendments of a live tenant are kept as the change history,
/// withdrawn, never as changes still to come (F27); a deadline main backed off is carried at its
/// next attempt, not its first schedule (F28).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mains_change_history_and_backed_off_deadlines_are_kept() -> TestResult {
    let (_container, base_url) = server("pending-changes").await;
    let at = at_main(&base_url).await?;
    let active = &at.active;
    let mut history = Vec::new();
    for (status, error) in [
        ("COMPLETED", "NULL"),
        ("CANCELED", "'withdrawn'"),
        ("FAILED", "'boom'"),
    ] {
        let id = Uuid::now_v7();
        at.db
            .batch_execute(&format!(
                "INSERT INTO scheduled_event (id, subscription_id, tenant_id, event_type,
                     scheduled_time, priority, event_data, created_at, status, error, source)
                 VALUES ('{id}', '{sub}', '{tenant}', 'APPLY_AMENDMENT', now() - interval '40 days',
                     0, '{{\"ApplyAmendment\": {{\"component_close\": [], \"component_insert\": [],
                       \"addon_close\": [], \"addon_insert\": []}}}}', now() - interval '50 days',
                     '{status}', {error}, 'api')",
                sub = active.subscription,
                tenant = active.tenant,
            ))
            .await?;
        history.push(id);
    }
    let (draft, finalized) = (Uuid::now_v7(), Uuid::now_v7());
    let mut rows = Vec::new();
    for (invoice, status, period) in [
        (finalized, "FINALIZED", "CURRENT_DATE - 31"),
        (draft, "DRAFT", "CURRENT_DATE"),
    ] {
        rows.push(format!(
            "('{invoice}', now(), '{tenant}', '{customer}', '{subscription}', 'EUR', '[]',
              {period}, 1000, 30, 'INV-{invoice}', 0, 1000, '{{}}', 1000, 1000, '{{}}',
              '{entity}', {period}, '{status}')",
            tenant = active.tenant,
            customer = active.customer,
            subscription = active.subscription,
            entity = active.entity,
        ));
    }
    at.db
        .batch_execute(&format!(
            "INSERT INTO invoice (id, created_at, tenant_id, customer_id, subscription_id, currency,
                 line_items, invoice_date, total, net_terms, invoice_number, tax_amount,
                 subtotal_recurring, customer_details, amount_due, subtotal, seller_details,
                 invoicing_entity_id, billing_period_start, status)
             VALUES {};
             INSERT INTO scheduled_event (id, subscription_id, tenant_id, event_type,
                 scheduled_time, next_attempt_at, priority, event_data, created_at, status, source)
             VALUES (gen_random_uuid(), '{sub}', '{tenant}', 'FINALIZE_INVOICE',
                     '2030-01-01 00:00', '2030-01-02 14:00', 0,
                     '{{\"FinalizeInvoice\": {{\"invoice_id\": \"{draft}\"}}}}', now(),
                     'PENDING', 'billing'),
                    (gen_random_uuid(), '{sub}', '{tenant}', 'RETRY_PAYMENT',
                     '2030-01-01 00:00', '2030-01-03 09:00', 0,
                     '{{\"RetryPayment\": {{\"invoice_id\": \"{finalized}\"}}}}', now(),
                     'PENDING', 'dunning')",
            rows.join(", "),
            sub = active.subscription,
            tenant = active.tenant,
        ))
        .await?;

    migrations::run(&diesel_make_pg_pool(PgConfig::new(at.url.clone()))?).await?;

    let kept: Vec<(String, Option<String>, bool)> = at
        .db
        .query(
            &format!(
                "SELECT id::text AS id, withdrawn_reason, withdrawn_at IS NOT NULL AS withdrawn
                 FROM subscription_change WHERE id IN ('{}', '{}', '{}') ORDER BY id",
                history[0], history[1], history[2]
            ),
            &[],
        )
        .await?
        .iter()
        .map(|row| {
            (
                row.get("id"),
                row.get("withdrawn_reason"),
                row.get("withdrawn"),
            )
        })
        .collect();
    assert_eq!(
        kept,
        vec![
            (
                history[0].to_string(),
                Some("previous release COMPLETED".to_string()),
                true
            ),
            (
                history[1].to_string(),
                Some("previous release CANCELED: withdrawn".to_string()),
                true
            ),
            (
                history[2].to_string(),
                Some("previous release FAILED: boom".to_string()),
                true
            ),
        ],
        "the history, withdrawn"
    );
    let deadlines = at
        .db
        .query_one(
            &format!(
                "SELECT (SELECT finalize_after::text FROM invoice WHERE id = '{draft}') AS finalize,
                        (SELECT next_payment_attempt_at::text FROM invoice WHERE id = '{finalized}')
                            AS attempt"
            ),
            &[],
        )
        .await?;
    assert_eq!(
        (
            deadlines.get::<_, Option<String>>("finalize"),
            deadlines.get::<_, Option<String>>("attempt")
        ),
        (
            Some("2030-01-02 14:00:00".to_string()),
            Some("2030-01-03 09:00:00".to_string())
        ),
        "each at its next attempt"
    );
    Ok(())
}

/// An import was billed elsewhere through its import period, that period's advance included. One
/// main never renewed settles through its current period and owes that period's arrears; one whose
/// renewal is still a draft settles through that renewal's period start, its arrears owed from the
/// import; one renewed on main keeps the watermark of its issued invoice.
/// Rules: R-094
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_import_not_renewed_on_main_bills_from_its_next_renewal() -> TestResult {
    let (_container, base_url) = server("pending-changes").await;
    let at = at_main(&base_url).await?;
    let period_start = at.next - Months::new(1);
    let imported = period_start + chrono::Days::new(4);
    for seeded in [&at.active, &at.archived, &at.sandbox] {
        at.db
            .batch_execute(&format!(
                "UPDATE subscription SET imported_at = '{imported} 10:00:00' WHERE id = '{}'",
                seeded.subscription
            ))
            .await?;
    }
    let mut rows = Vec::new();
    for (seeded, status) in [(&at.archived, "DRAFT"), (&at.sandbox, "FINALIZED")] {
        rows.push(format!(
            "('{}', now(), '{}', '{}', '{}', 'EUR', '[]', '{next}', 1000, 30, 'INV-{}', 0, 1000,
              '{{}}', 1000, 1000, '{{}}', '{}', '{next}', '{status}', 'RECURRING')",
            Uuid::now_v7(),
            seeded.tenant,
            seeded.customer,
            seeded.subscription,
            Uuid::now_v7(),
            seeded.entity,
            next = at.next,
        ));
    }
    at.db
        .batch_execute(&format!(
            "INSERT INTO invoice (id, created_at, tenant_id, customer_id, subscription_id, currency,
                 line_items, invoice_date, total, net_terms, invoice_number, tax_amount,
                 subtotal_recurring, customer_details, amount_due, subtotal, seller_details,
                 invoicing_entity_id, billing_period_start, status, invoice_type)
             VALUES {}",
            rows.join(", ")
        ))
        .await?;

    migrations::run(&diesel_make_pg_pool(PgConfig::new(at.url.clone()))?).await?;

    let mut watermarks = Vec::new();
    for seeded in [&at.active, &at.archived, &at.sandbox] {
        let row = at
            .db
            .query_one(
                &format!(
                    "SELECT reconcile_from::text AS settled,
                            reconcile_arrears_from::text AS arrears
                     FROM subscription WHERE id = '{}'",
                    seeded.subscription
                ),
                &[],
            )
            .await?;
        watermarks.push((
            row.get::<_, Option<String>>("settled"),
            row.get::<_, Option<String>>("arrears"),
        ));
    }
    assert_eq!(
        watermarks,
        vec![
            (Some(at.next.to_string()), Some(period_start.to_string())),
            (
                Some(at.next.to_string()),
                Some((imported - chrono::Days::new(1)).to_string())
            ),
            (Some(at.next.to_string()), None),
        ]
    );
    Ok(())
}

/// A paused or suspended subscription of a live tenant is never resumed by the release: it
/// fails, naming it, and main's schema stays. One of a tenant not live becomes ACTIVE, billed
/// again, as before.
/// Rules: R-122
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_live_tenants_paused_subscription_fails_the_release() -> TestResult {
    let (_container, base_url) = server("pending-changes").await;
    let at = at_main(&base_url).await?;
    for (seeded, status) in [(&at.active, "PAUSED"), (&at.sandbox, "SUSPENDED")] {
        at.db
            .batch_execute(&format!(
                "DELETE FROM scheduled_event WHERE subscription_id = '{sub}';
                 UPDATE subscription SET status = '{status}' WHERE id = '{sub}'",
                sub = seeded.subscription
            ))
            .await?;
    }
    let before = catalog(&at.db).await;

    let failed = migrations::run(&diesel_make_pg_pool(PgConfig::new(at.url.clone()))?)
        .await
        .expect_err("the release refuses");
    let error = format!("{failed:?}");
    assert!(
        error.contains(&at.active.subscription.to_string()),
        "names the subscription: {error}"
    );
    assert!(
        error.contains("cancel or resume"),
        "says what to do: {error}"
    );
    assert!(
        !error.contains(&at.sandbox.subscription.to_string()),
        "a tenant not live is not listed"
    );
    assert_eq!(catalog(&at.db).await, before, "main's schema, as it was");

    // Resumed on its own, the release goes through and the sandbox's is ACTIVE.
    at.db
        .batch_execute(&format!(
            "UPDATE subscription SET status = 'ACTIVE' WHERE id = '{}'",
            at.active.subscription
        ))
        .await?;
    migrations::run(&diesel_make_pg_pool(PgConfig::new(at.url.clone()))?).await?;
    let status = at
        .db
        .query_one(
            &format!(
                "SELECT status::text AS status FROM subscription WHERE id = '{}'",
                at.sandbox.subscription
            ),
            &[],
        )
        .await?;
    assert_eq!(status.get::<_, String>("status"), "ACTIVE");
    Ok(())
}
