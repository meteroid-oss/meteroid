//! Recurring invoice consolidation (merging) tests.
//!
//! When several of a customer's subscriptions renew on the same day with the same currency,
//! payment method, auto-advance flag and invoicing entity, their renewal drafts are merged
//! into a single consolidated invoice. The per-subscription drafts are retained as hidden
//! "consolidated children" (so MRR and idempotency stay per-subscription) while the
//! consolidated parent is the one finalized, charged and rendered.

use chrono::NaiveDate;
use rstest::rstest;

use crate::data::ids::*;
use crate::harness::{SubscriptionAssertExt, TestEnv, subscription, test_env};
use meteroid_store::domain::enums::{InvoiceStatusEnum, InvoiceType};

/// Helper: the consolidated parent invoices for a customer (recurring, no subscription_id).
async fn consolidated_parents(
    env: &TestEnv,
    customer_id: common_domain::ids::CustomerId,
) -> Vec<meteroid_store::domain::Invoice> {
    env.get_customer_invoices(customer_id)
        .await
        .into_iter()
        .filter(|i| i.subscription_id.is_none() && i.invoice_type == InvoiceType::Recurring)
        .collect()
}

/// Two subscriptions for the same customer renewing the same day are merged into one invoice.
#[rstest]
#[tokio::test]
async fn test_same_day_renewals_are_consolidated(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_consolidate_recurring_invoices(true).await;
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    // Same customer, same billing day, same currency/payment/auto-advance (builder defaults).
    let sub_a = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_1_LEETCODE_ID) // $35/month
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let sub_b = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Renewal: both fall due the same day and should merge into a single invoice.
    env.process_cycles().await;

    // Exactly one consolidated renewal invoice for the customer.
    let parents = consolidated_parents(&env, CUST_UBER_ID).await;
    assert_eq!(
        parents.len(),
        1,
        "expected a single consolidated renewal invoice, got {}",
        parents.len()
    );
    let parent = &parents[0];
    assert_eq!(parent.status, InvoiceStatusEnum::Finalized);
    assert_eq!(parent.total, 7000, "combined total of both $35 renewals");
    assert_eq!(
        parent.line_items.len(),
        2,
        "consolidated invoice aggregates one line per merged subscription"
    );
    assert_ne!(
        parent.invoice_number, "draft",
        "consolidated parent must be finalized with a real number"
    );

    // The two per-subscription drafts are retained as hidden consolidated children.
    let children = env.get_consolidated_children(parent.id).await;
    assert_eq!(children.len(), 2);

    let child_subs: Vec<_> = children.iter().filter_map(|c| c.subscription_id).collect();
    assert_eq!(child_subs.len(), 2);
    assert!(child_subs.contains(&sub_a) && child_subs.contains(&sub_b));

    for child in &children {
        assert_eq!(child.consolidated_into_invoice_id, Some(parent.id));
        assert_eq!(
            child.status,
            InvoiceStatusEnum::Draft,
            "children stay draft (never finalized on their own)"
        );
        assert_eq!(child.total, 3500);
    }

    // A subscription's own listing surfaces its contribution as a child linked to the
    // consolidated parent (so the cycle isn't shown as unbilled), while customer-level listings
    // show the parent instead.
    let listed_a = env.get_invoices(sub_a).await;
    assert!(
        listed_a
            .iter()
            .any(|i| i.consolidated_into_invoice_id == Some(parent.id)),
        "subscription listing should surface its consolidated contribution, linked to the parent"
    );
    assert!(
        consolidated_parents(&env, CUST_UBER_ID)
            .await
            .iter()
            .all(|p| p.subscription_id.is_none()),
        "the consolidated parent is customer-level (no subscription_id)"
    );
}

/// Subscriptions belonging to different customers are never merged together.
#[rstest]
#[tokio::test]
async fn test_different_customers_are_not_consolidated(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_consolidate_recurring_invoices(true).await;
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let _sub_uber = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let _sub_spotify = subscription()
        .customer(CUST_SPOTIFY_ID)
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    env.process_cycles().await;

    // Neither customer gets a consolidated parent: a single subscription has nothing to merge.
    assert!(
        consolidated_parents(&env, CUST_UBER_ID).await.is_empty(),
        "single-subscription customer must not be consolidated"
    );
    assert!(
        consolidated_parents(&env, CUST_SPOTIFY_ID).await.is_empty(),
        "single-subscription customer must not be consolidated"
    );

    // Every invoice keeps its own subscription_id (no merging happened).
    for customer in [CUST_UBER_ID, CUST_SPOTIFY_ID] {
        assert!(
            env.get_customer_invoices(customer)
                .await
                .iter()
                .all(|i| i.subscription_id.is_some() && i.consolidated_into_invoice_id.is_none()),
        );
    }
}

/// Same customer, but subscriptions billing on different days are not merged
/// (the merge key requires the same invoice date).
#[rstest]
#[tokio::test]
async fn test_different_billing_days_are_not_consolidated(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_consolidate_recurring_invoices(true).await;

    let sub_first = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let sub_mid = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 15).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    env.process_cycles().await;

    // Different billing-day anchors → different renewal dates → no consolidation.
    assert!(
        consolidated_parents(&env, CUST_UBER_ID).await.is_empty(),
        "subscriptions on different billing days must not merge"
    );

    // Each subscription still renews to its own standalone invoice.
    for sub in [sub_first, sub_mid] {
        let invoices = env.get_invoices(sub).await;
        assert!(
            invoices
                .iter()
                .any(|i| i.status == InvoiceStatusEnum::Finalized),
            "each subscription should still produce its own finalized renewal"
        );
        assert!(
            invoices
                .iter()
                .all(|i| i.consolidated_into_invoice_id.is_none())
        );
    }
}

/// When the consolidated parent is paid, every merged member subscription must transition
/// TrialExpired → Active — even though the parent has no subscription_id of its own.
#[rstest]
#[tokio::test]
async fn test_consolidated_payment_activates_all_trial_members(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_consolidate_recurring_invoices(true).await;
    env.seed_payments().await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    // Two OnCheckout free-trial subscriptions for the same customer, ending the same day.
    let mut sub_ids = Vec::new();
    for _ in 0..2 {
        let sub_id = subscription()
            .customer(CUST_UBER_ID)
            .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID) // $49/month
            .start_date(start_date)
            .on_checkout()
            .trial_days(14)
            .auto_charge()
            .create(env.services())
            .await;
        sub_ids.push(sub_id);
    }

    for sub_id in &sub_ids {
        let mut conn = env.conn().await;
        env.services()
            .complete_subscription_checkout_tx(
                &mut conn,
                TENANT_ID,
                *sub_id,
                CUST_UBER_PAYMENT_METHOD_ID,
                0,
                "EUR".to_string(),
                None,
            )
            .await
            .expect("checkout should succeed");
    }

    // Trial ends → both go TrialExpired and their drafts consolidate.
    env.process_cycles().await;
    for sub_id in &sub_ids {
        env.get_subscription(*sub_id)
            .await
            .assert()
            .is_trial_expired();
    }

    let parents = consolidated_parents(&env, CUST_UBER_ID).await;
    assert_eq!(
        parents.len(),
        1,
        "the two trial-end drafts should consolidate"
    );

    env.run_outbox_and_orchestration().await;

    let parent = &consolidated_parents(&env, CUST_UBER_ID).await[0];
    assert_eq!(parent.status, InvoiceStatusEnum::Finalized);
    assert_eq!(parent.total, 9800, "two $49 trial subscriptions merged");

    for sub_id in &sub_ids {
        env.get_subscription(*sub_id).await.assert().is_active();
    }
}

/// With the opt-in flag off (the default), eligible drafts are NOT merged: each renews to
/// its own standalone invoice.
#[rstest]
#[tokio::test]
async fn test_consolidation_off_by_default(#[future] test_env: TestEnv) {
    let env = test_env.await;
    // No `set_consolidate_recurring_invoices(true)` — flag stays off.
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let sub_a = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let sub_b = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    env.process_cycles().await;

    assert!(
        consolidated_parents(&env, CUST_UBER_ID).await.is_empty(),
        "consolidation must not run when the entity flag is off"
    );
    for sub in [sub_a, sub_b] {
        let invoices = env.get_invoices(sub).await;
        assert!(
            invoices
                .iter()
                .any(|i| i.status == InvoiceStatusEnum::Finalized
                    && i.consolidated_into_invoice_id.is_none()),
            "each subscription renews to its own standalone invoice"
        );
    }
}

/// Subscriptions that resolve a different `charge_automatically` are NOT merged together: the
/// candidate query matches the static key, but the per-subscription Rust filter splits them so
/// the consolidated parent has a well-defined charging behavior.
#[rstest]
#[tokio::test]
async fn test_differing_charge_automatically_not_consolidated(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_consolidate_recurring_invoices(true).await;
    // Auto-charging requires a configured payment provider on the customer.
    env.seed_payments().await;
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    // Same customer/plan/day, but one auto-charges and the other does not.
    let sub_auto = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .auto_charge()
        .create(env.services())
        .await;
    let sub_manual = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .no_auto_charge()
        .create(env.services())
        .await;

    env.process_cycles().await;

    // No consolidation: the two subscriptions have incompatible charging behavior.
    assert!(
        consolidated_parents(&env, CUST_UBER_ID).await.is_empty(),
        "subscriptions with different charge_automatically must not merge"
    );
    for sub in [sub_auto, sub_manual] {
        let invoices = env.get_invoices(sub).await;
        assert!(
            invoices
                .iter()
                .any(|i| i.status == InvoiceStatusEnum::Finalized
                    && i.consolidated_into_invoice_id.is_none()),
            "each subscription renews to its own standalone invoice"
        );
    }
}

/// `net_terms` is part of the merge key (it drives the inherited due date), so subscriptions with
/// different net terms are not consolidated together.
#[rstest]
#[tokio::test]
async fn test_differing_net_terms_not_consolidated(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_consolidate_recurring_invoices(true).await;
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let sub_net15 = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .net_terms(15)
        .create(env.services())
        .await;
    let sub_net30 = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .net_terms(30)
        .create(env.services())
        .await;

    env.process_cycles().await;

    assert!(
        consolidated_parents(&env, CUST_UBER_ID).await.is_empty(),
        "subscriptions with different net_terms must not merge"
    );
    for sub in [sub_net15, sub_net30] {
        let invoices = env.get_invoices(sub).await;
        assert!(
            invoices
                .iter()
                .any(|i| i.status == InvoiceStatusEnum::Finalized
                    && i.consolidated_into_invoice_id.is_none()),
            "each subscription renews to its own standalone invoice"
        );
    }
}

/// A customer's prepaid balance is applied to the consolidated parent and NOT carried by the
/// hidden children: each member draft computes a credit against the same balance, so leaving them
/// in place would double-count. Consolidation recomputes the credit once on the parent and clears
/// the children's draft-time credit.
#[rstest]
#[tokio::test]
async fn test_prepaid_balance_credit_recomputed_once(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_consolidate_recurring_invoices(true).await;
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    // Ample prepaid balance so the renewal is fully covered regardless of what the subscriptions'
    // initial-period invoices consumed first. The discriminating signal is that the credit lives
    // on the parent only — the children must carry zero applied_credits.
    env.top_up_balance(CUST_UBER_ID, 1_000_000).await;

    for _ in 0..2 {
        subscription()
            .customer(CUST_UBER_ID)
            .plan_version(PLAN_VERSION_1_LEETCODE_ID)
            .start_date(start_date)
            .on_start()
            .no_trial()
            .create(env.services())
            .await;
    }

    env.process_cycles().await;

    let parents = consolidated_parents(&env, CUST_UBER_ID).await;
    assert_eq!(parents.len(), 1, "the two renewals should consolidate");
    let parent = &parents[0];
    assert_eq!(parent.total, 7000);
    // Credit recomputed once on the merged total; balance is ample so it fully covers it.
    assert_eq!(
        parent.applied_credits, 7000,
        "credit recomputed on the consolidated parent"
    );
    assert_eq!(parent.amount_due, 0, "amount_due = total - applied_credits");

    // The credit lives on the parent only: children carry no (double-counting) credit.
    let children = env.get_consolidated_children(parent.id).await;
    assert_eq!(children.len(), 2);
    for child in &children {
        assert_eq!(
            child.applied_credits, 0,
            "child draft-time credit must be cleared (not summed into the parent twice)"
        );
        assert_eq!(child.amount_due, child.total);
    }
}

/// Voiding a consolidated parent succeeds when no member is awaiting activation; the hidden
/// children remain linked (their audit trail is preserved) and are not themselves voided.
#[rstest]
#[tokio::test]
async fn test_void_consolidated_parent_keeps_children(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_consolidate_recurring_invoices(true).await;
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    for _ in 0..2 {
        subscription()
            .customer(CUST_UBER_ID)
            .plan_version(PLAN_VERSION_1_LEETCODE_ID)
            .start_date(start_date)
            .on_start()
            .no_trial()
            .create(env.services())
            .await;
    }
    env.process_cycles().await;

    let parent = consolidated_parents(&env, CUST_UBER_ID).await[0].clone();
    assert_eq!(parent.status, InvoiceStatusEnum::Finalized);

    // Members are Active (on_start, no trial) so voiding strands nobody → allowed.
    env.try_void_invoice(parent.id)
        .await
        .expect("voiding a consolidated parent with active members should succeed");

    let voided = env.get_detailed_invoice(parent.id).await;
    assert_eq!(voided.invoice.status, InvoiceStatusEnum::Void);

    // Children are untouched and still point at the (now void) parent.
    let children = env.get_consolidated_children(parent.id).await;
    assert_eq!(children.len(), 2);
    for child in &children {
        assert_eq!(child.consolidated_into_invoice_id, Some(parent.id));
        assert_eq!(child.status, InvoiceStatusEnum::Draft);
    }
}

/// Voiding a consolidated parent is BLOCKED while any member subscription is still awaiting this
/// payment to activate (TrialExpired) — otherwise those members would be stranded with no
/// activation path.
#[rstest]
#[tokio::test]
async fn test_void_blocked_while_trial_members_pending(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_consolidate_recurring_invoices(true).await;
    env.seed_payments().await;
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let mut sub_ids = Vec::new();
    for _ in 0..2 {
        let sub_id = subscription()
            .customer(CUST_UBER_ID)
            .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
            .start_date(start_date)
            .on_checkout()
            .trial_days(14)
            .auto_charge()
            .create(env.services())
            .await;
        sub_ids.push(sub_id);
    }
    for sub_id in &sub_ids {
        let mut conn = env.conn().await;
        env.services()
            .complete_subscription_checkout_tx(
                &mut conn,
                TENANT_ID,
                *sub_id,
                CUST_UBER_PAYMENT_METHOD_ID,
                0,
                "EUR".to_string(),
                None,
            )
            .await
            .expect("checkout should succeed");
    }

    // Trial ends → both TrialExpired and drafts consolidate into one finalized (unpaid) parent.
    env.process_cycles().await;
    for sub_id in &sub_ids {
        env.get_subscription(*sub_id)
            .await
            .assert()
            .is_trial_expired();
    }
    let parent = consolidated_parents(&env, CUST_UBER_ID).await[0].clone();
    assert_eq!(parent.status, InvoiceStatusEnum::Finalized);

    // Voiding would strand the trial members → must be rejected.
    let result = env.try_void_invoice(parent.id).await;
    assert!(
        result.is_err(),
        "voiding a consolidated parent with trial-pending members must be rejected"
    );

    // The parent is untouched and members remain awaiting activation.
    assert_eq!(
        env.get_detailed_invoice(parent.id).await.invoice.status,
        InvoiceStatusEnum::Finalized
    );
    for sub_id in &sub_ids {
        env.get_subscription(*sub_id)
            .await
            .assert()
            .is_trial_expired();
    }
}

/// Consolidation still works with a zero grace period: the effective grace is floored to a small
/// minimum so the shared finalize deadline stays in the future until all same-day sibling drafts
/// are created, rather than firing immediately and finalizing each one alone.
#[rstest]
#[tokio::test]
async fn test_zero_grace_still_consolidates(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_consolidate_and_grace(true, 0).await;
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    for _ in 0..2 {
        subscription()
            .customer(CUST_UBER_ID)
            .plan_version(PLAN_VERSION_1_LEETCODE_ID)
            .start_date(start_date)
            .on_start()
            .no_trial()
            .create(env.services())
            .await;
    }

    env.process_cycles().await;

    let parents = consolidated_parents(&env, CUST_UBER_ID).await;
    assert_eq!(
        parents.len(),
        1,
        "same-day renewals must still consolidate with a zero grace period"
    );
    assert_eq!(parents[0].total, 7000);
    assert_eq!(env.get_consolidated_children(parents[0].id).await.len(), 2);
}
