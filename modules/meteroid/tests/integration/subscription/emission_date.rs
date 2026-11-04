//! Emission-date stamping at finalization.
//!
//! A draft anchors on `billing_period_start`; finalizing it stamps `invoice_date` with the
//! actual emission day and rebases `due_at` on it, so a draft that outlived its period (grace,
//! catch-up billing of a past start date) is not issued already overdue. Backdated
//! subscriptions (historical imports) keep their period dates.

use chrono::NaiveDate;
use rstest::rstest;

use crate::data::ids::*;
use crate::harness::{InvoicesAssertExt, TestEnv, subscription, test_env};
use meteroid_store::domain::UpdateInvoiceParams;

/// Catch-up billing of a past period finalizes with today's date and J+net_terms due date.
#[rstest]
#[tokio::test]
async fn test_finalization_stamps_emission_date(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Fires the scheduled finalization of the cycle-0 invoice (its period is long past).
    env.process_cycles().await;

    let today = chrono::Utc::now().date_naive();
    let invoice = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .find(|i| i.billing_period_start == start_date)
        .expect("invoice for the first period");

    assert!(!invoice.can_edit(), "invoice should be finalized");
    assert_eq!(
        invoice.invoice_date, today,
        "finalized invoice carries the emission date, not the period start"
    );
    let due = invoice.due_at.expect("due date").date();
    assert_eq!(
        due,
        today + chrono::Duration::days(i64::from(invoice.net_terms)),
        "due date is rebased on the emission date"
    );
}

/// Historical imports (`backdate_invoices`) keep period dating end to end.
#[rstest]
#[tokio::test]
async fn test_backdated_subscription_keeps_period_dates(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .backdate_invoices()
        .create(env.services())
        .await;

    env.process_cycles().await;

    let invoice = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .find(|i| i.billing_period_start == start_date)
        .expect("invoice for the first period");

    assert!(!invoice.can_edit(), "invoice should be finalized");
    assert_eq!(
        invoice.invoice_date, start_date,
        "backdated invoice keeps its period date"
    );
    assert_eq!(
        invoice.finalized_at.expect("finalized_at").date(),
        start_date,
        "finalized_at is backdated too"
    );
}

/// An operator-set due date on the draft survives the emission-date stamping.
#[rstest]
#[tokio::test]
async fn test_operator_due_date_survives_finalization(#[future] test_env: TestEnv) {
    let env = test_env.await;

    // A far-future grace keeps the renewal invoice in draft (the creation invoice
    // finalizes inline under Immediate billing, so the renewal is the draft we edit).
    env.set_consolidate_and_grace(false, 300_000).await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let period_2_start = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    env.process_cycles().await;

    let draft = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .find(|i| i.billing_period_start == period_2_start)
        .expect("invoice for the second period");
    assert!(draft.can_edit(), "invoice should still be a draft");

    let operator_due = NaiveDate::from_ymd_opt(2030, 5, 5).unwrap();
    env.services()
        .update_draft_invoice(
            draft.id,
            TENANT_ID,
            UpdateInvoiceParams {
                due_date: Some(Some(operator_due)),
                ..Default::default()
            },
        )
        .await
        .expect("update draft due date");

    env.manual_finalize_invoice(draft.id)
        .await
        .expect("finalize draft");

    let today = chrono::Utc::now().date_naive();
    let invoice = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .find(|i| i.billing_period_start == period_2_start)
        .expect("invoice for the second period");

    assert!(!invoice.can_edit(), "invoice should be finalized");
    assert_eq!(invoice.invoice_date, today);
    assert_eq!(
        invoice.due_at.expect("due date").date(),
        operator_due,
        "operator-set due date is kept"
    );
}

/// Several past periods finalized the same day must coexist: the duplicate-billing
/// backstop keys on the billing period, not the (now shared) emission date.
#[rstest]
#[tokio::test]
async fn test_catch_up_billing_finalizes_all_periods_same_day(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Three cycles of catch-up billing, all finalizing today.
    env.process_cycles().await;
    env.process_cycles().await;
    env.process_cycles().await;

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(4);

    let today = chrono::Utc::now().date_naive();
    let finalized: Vec<_> = (0..invoices.len())
        .filter(|i| !invoices[*i].can_edit())
        .collect();
    assert!(
        finalized.len() >= 3,
        "several periods finalized the same day"
    );
    for i in finalized {
        invoices.assert().invoice_at(i).has_invoice_date(today);
    }

    // Each finalized invoice settles a distinct period.
    let mut periods: Vec<_> = invoices.iter().map(|i| i.billing_period_start).collect();
    periods.sort();
    periods.dedup();
    assert_eq!(periods.len(), invoices.len(), "one invoice per period");
}
