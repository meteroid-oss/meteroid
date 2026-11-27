//! The release's migrations apply, revert to exactly the schema they started from (main's), and
//! apply again to exactly the schema they first produced.
use meteroid_testkit::release::{catalog, client, migrate, postgres, revert_last};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const FIRST_RELEASE_MIGRATION: &str = "20261008110347";

#[tokio::test]
async fn release_migrations_revert_exactly_and_reapply_identically() -> TestResult {
    let (_container, url) = postgres("release-migrations").await;
    migrate(&url, "", Some(FIRST_RELEASE_MIGRATION)).await?;
    let client = client(&url).await;
    let before = catalog(&client).await;
    assert!(
        !before
            .iter()
            .any(|l| l.starts_with("table customer_credit_grant "))
    );

    migrate(&url, FIRST_RELEASE_MIGRATION, None).await?;
    let applied = catalog(&client).await;
    for expected in [
        "column usage_fold.grouping ",
        "column usage_fold.window_start ",
        "table usage_monitor_state u {fillfactor=80}",
        "table customer_budget p ",
        "column customer_budget.subscription_id ",
        "table credit_unit p ",
        "table payment_reversal_movement p ",
        "column invoice.store_xid ",
        "index usage_monitor_state usage_monitor_state_heartbeat_idx ",
        "index subscription subscription_live_customer_idx ",
        "constraint subscription subscription_usage_filter_needs_resource true ",
    ] {
        assert!(
            applied.iter().any(|l| l.starts_with(expected)),
            "missing {expected}\n{applied:#?}"
        );
    }
    assert!(
        !applied
            .iter()
            .any(|l| l.starts_with("column usage_fold.period_start "))
    );
    assert!(
        !applied.iter().any(|l| l.starts_with("table usage_limit ")),
        "usage limits were folded into entitlements before the release"
    );
    assert!(
        applied
            .iter()
            .filter(|l| l.starts_with("constraint ") && !before.contains(l))
            .all(|l| l.split(' ').nth(3) == Some("true")),
        "every constraint the release adds is validated"
    );

    loop {
        let reverted = revert_last(&url).await?;
        if reverted.as_str() <= FIRST_RELEASE_MIGRATION {
            break;
        }
    }
    // PostgreSQL cannot drop enum values: the ones the release adds stay, unused.
    let kept = [
        "enum BatchJobTypeEnum 6 SIGNUP_PLAN_BACKFILL",
        "enum CheckoutTypeEnum 5 TOP_UP",
        "enum DeclineKindEnum 9 DO_NOT_RETRY",
        "enum DeclineKindEnum 10 UNCONFIRMED",
        "enum FeeTypeEnum 7 CREDIT",
        "enum MilestoneEnum 17 FIRST_MOLLIE_CONNECTED",
        "enum PaymentTypeEnum 3 TOP_UP",
        "enum PaymentTypeEnum 4 AUTO_TOPUP",
        "enum StoredObjectKindEnum 10 QUOTE_CERTIFICATE_PDF",
        "enum StoredObjectKindEnum 11 QUOTE_PDF",
        "enum StoredObjectKindEnum 12 QUOTE_ATTACHMENT",
    ];
    // Appended after the type's own labels, whatever their count.
    let final_invoice = |l: &str| {
        (l.starts_with("enum InvoiceType ") && l.ends_with(" FINAL"))
            || (l.starts_with("enum InvoicePaymentStatus ") && l.ends_with(" CARRIED"))
    };
    let reverted = catalog(&client).await;
    assert!(kept.iter().all(|value| reverted.iter().any(|l| l == value)));
    assert_eq!(reverted.iter().filter(|l| final_invoice(l)).count(), 2);
    assert_eq!(
        reverted
            .into_iter()
            .filter(|l| !kept.contains(&l.as_str()) && !final_invoice(l))
            .collect::<Vec<_>>(),
        before,
        "the down migrations restore main's schema"
    );

    migrate(&url, FIRST_RELEASE_MIGRATION, None).await?;
    assert_eq!(catalog(&client).await, applied);
    Ok(())
}

/// The release's first migration, its additions and backfills, and its revert.
pub const BILLING_ON_FACTS_UP: &str =
    include_str!("../../migrations/diesel/2026-11-16-171538_billing_on_facts/up.sql");
pub const BILLING_ON_FACTS_DOWN: &str =
    include_str!("../../migrations/diesel/2026-11-16-171538_billing_on_facts/down.sql");
/// The release's last migration's revert: main's scheduled_event, among others.
pub const BILLING_LEGACY_DROPPED_DOWN: &str =
    include_str!("../../migrations/diesel/2026-11-16-171749_billing_legacy_dropped/down.sql");

/// The section `title` of a migration (from its `-- ── title` header to the next one).
pub fn migration_section<'a>(sql: &'a str, title: &str) -> &'a str {
    let header = format!("-- ── {title} ");
    let start = sql.find(&header).expect("the section");
    let rest = &sql[start + header.len()..];
    let end = rest
        .find("\n-- ── ")
        .map_or(sql.len(), |at| start + header.len() + at);
    &sql[start..end]
}
