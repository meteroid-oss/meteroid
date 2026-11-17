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

    // Their checkouts never completed: unsettled, each awaits the payment of its first invoice,
    // collected at the trial's end from the customer's card on file.

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
    env.fund_invoice_credit(CUST_UBER_ID, 1_000_000).await;

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
    // Their checkouts never completed: unsettled, each awaits the payment of its first invoice,
    // collected at the trial's end from the customer's card on file.

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

/// Minimum-commitment × consolidation: the floor is a per-subscription concept, computed during
/// each member's pre-merge refresh, so the consolidated parent must carry ONE shortfall line per
/// member — never a merged or double-counted floor. $50 floor on the $35 LeetCode plan → each
/// renewal is a $35 line + a $15 top-up; two merged members = 4 lines, $100 total.
#[rstest]
#[tokio::test]
async fn test_consolidated_invoice_includes_per_subscription_minimum(#[future] test_env: TestEnv) {
    use diesel_async::RunQueryDsl;
    use diesel_models::plan_version_adjustments::PlanVersionAdjustmentRowNew;
    use meteroid_store::domain::LineItemKind;

    let env = test_env.await;
    env.set_consolidate_recurring_invoices(true).await;
    let mut conn = env.conn().await;

    diesel::insert_into(diesel_models::schema::plan_version_adjustment::table)
        .values(&PlanVersionAdjustmentRowNew {
            kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
            plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
            amount_subunit: 5_000, // $50 floor
            includes_all_components: true,
        })
        .execute(&mut conn)
        .await
        .unwrap();

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
    assert_eq!(parents.len(), 1, "one consolidated renewal invoice");
    let parent = &parents[0];

    let shortfall_count = parent
        .line_items
        .iter()
        .filter(|l| l.kind == LineItemKind::MinimumShortfall)
        .count();
    assert_eq!(
        shortfall_count, 2,
        "one per-subscription shortfall line, not merged or double-counted"
    );
    assert_eq!(
        parent.total, 10_000,
        "2 × ($35 recurring + $15 floor top-up)"
    );
    assert_eq!(
        parent.line_items.len(),
        4,
        "2 recurring + 2 shortfall lines aggregated from the two members"
    );
}

/// A merged draft is never issued, so a credit note cannot target it: the consolidated parent
/// carries its lines under the same local ids and takes the notes, which is why budget spend reads
/// only the parents' notes. Before the explicit guard, the refusal came from the draft status alone.
#[rstest]
#[tokio::test]
async fn test_credit_notes_target_the_consolidated_parent_never_a_member(
    #[future] test_env: TestEnv,
) {
    use common_domain::actor::Actor;
    use meteroid_store::repositories::CreditNoteInterface;
    use meteroid_store::repositories::credit_notes::{
        CreateCreditNoteParams, CreditLineItem, CreditType,
    };

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
    let children = env.get_consolidated_children(parent.id).await;
    assert_eq!(children.len(), 2);

    let params = |invoice_id, line_items| CreateCreditNoteParams {
        invoice_id,
        line_items,
        reason: None,
        memo: None,
        credit_type: CreditType::DebtCancellation,
        custom_properties: None,
    };
    for child in &children {
        let refused = env
            .store()
            .create_credit_note(Actor::System, TENANT_ID, params(child.id, vec![]))
            .await
            .expect_err("a consolidated member takes no credit note");
        assert!(
            format!("{refused:?}").contains("merged into a consolidated parent"),
            "refused as a consolidated member, not merely as a draft: {refused:?}"
        );
    }

    let member_line = &children[0].line_items[0];
    let note = env
        .store()
        .create_credit_note(
            Actor::System,
            TENANT_ID,
            params(
                parent.id,
                vec![CreditLineItem::Amount {
                    local_id: member_line.local_id.clone(),
                    amount_cents: 1_000,
                }],
            ),
        )
        .await
        .expect("the parent credits a merged member's line by its local id");
    assert_eq!(note.invoice_id, parent.id);
    assert_eq!(note.line_items.len(), 1);
    assert_eq!(note.line_items[0].local_id, member_line.local_id);
    assert_eq!(note.line_items[0].amount_subtotal, -1_000);
}

/// Two 35.00 renewals merged into one invoice draw a 50.00 promotional grant once: the drafts
/// merged into it carry their shares of what it drew, never each its own estimate against the
/// same grant.
#[rstest]
#[tokio::test]
async fn merged_drafts_carry_their_shares_of_the_credit_the_consolidated_invoice_drew(
    #[future] test_env: TestEnv,
) {
    use crate::helpers::credit::{Grant, fund_promotional_grant};
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
    // After the first invoices: only the merged renewal draws it.
    fund_promotional_grant(
        env.store(),
        TENANT_ID,
        CUST_UBER_ID,
        None,
        Grant::invoice(5_000),
    )
    .await;
    env.process_cycles().await;

    let parents = consolidated_parents(&env, CUST_UBER_ID).await;
    assert_eq!(parents.len(), 1);
    let parent = &parents[0];
    let children = env.get_consolidated_children(parent.id).await;
    assert_eq!(children.len(), 2);
    assert!(parent.promotional_credits > 0);
    assert_eq!(
        children
            .iter()
            .map(|child| child.promotional_credits)
            .sum::<i64>(),
        parent.promotional_credits
    );
    assert_eq!(
        children
            .iter()
            .map(|child| child.applied_credits)
            .sum::<i64>(),
        parent.applied_credits
    );
    assert_eq!(
        children.iter().map(|child| child.total).sum::<i64>(),
        parent.total
    );
}

/// A yearly 365.00 support fee in arrears added to the first subscription on Jan 1, removed on
/// Mar 15: its year [Jan 1 2024, Jan 1 2025) has 366 days, and [Jan 1, Mar 15) bills
/// 36500 × 74/366 = 7379.78 → 7380 once, on the Apr 1 consolidated renewal, the first invoice
/// from its end (R-011). That renewal is voided, then the 35.00 rate raised to 65.00 on Apr 16:
/// what May 1 bills of those arrears, and what the raise bills.
async fn yearly_arrears_billed_after_a_voided_consolidated_renewal(env: &TestEnv) -> (i64, i64) {
    use meteroid_store::domain::enums::{BillingPeriodEnum, BillingType, FeeTypeEnum};
    use meteroid_store::domain::price_components::{PriceEntry, PriceInput, ProductRef};
    use meteroid_store::domain::prices::{FeeStructure, Pricing};
    use meteroid_store::domain::subscription_amendment::{
        AddExtraComponent, AddOnChanges, ComponentChanges, SubscriptionAmendment,
    };
    use meteroid_store::domain::subscription_changes::PlanChangeMode;
    use rust_decimal::Decimal;

    env.set_consolidate_recurring_invoices(true).await;
    let day = |m, d| NaiveDate::from_ymd_opt(2024, m, d).unwrap();
    let mut subs = Vec::new();
    for _ in 0..2 {
        subs.push(
            subscription()
                .customer(CUST_UBER_ID)
                .plan_version(PLAN_VERSION_1_LEETCODE_ID)
                .start_date(day(1, 1))
                .on_start()
                .no_trial()
                .create(env.services())
                .await,
        );
    }
    let amend = |component_changes| SubscriptionAmendment {
        apply_mode: PlanChangeMode::Immediate,
        component_changes,
        add_on_changes: AddOnChanges::default(),
    };
    env.services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            subs[0],
            TENANT_ID,
            amend(ComponentChanges {
                edited: vec![],
                added: vec![AddExtraComponent {
                    name: "Yearly Support".to_string(),
                    product: ProductRef::New {
                        name: "Yearly Support Product".to_string(),
                        fee_type: FeeTypeEnum::ExtraRecurring,
                        fee_structure: FeeStructure::ExtraRecurring {
                            billing_type: BillingType::Arrears,
                        },
                    },
                    price_entry: PriceEntry::New(PriceInput {
                        cadence: BillingPeriodEnum::Annual,
                        currency: "EUR".to_string(),
                        pricing: Pricing::ExtraRecurring {
                            unit_price: Decimal::new(36_500, 2),
                            quantity: 1,
                        },
                        credit_unit: None,
                    }),
                }],
                removed: vec![],
            }),
            day(1, 1),
        )
        .await
        .expect("add the yearly support fee");
    env.process_cycles().await;
    env.process_cycles().await;
    let support = env
        .get_subscription_components(subs[0])
        .await
        .into_iter()
        .find(|c| c.name == "Yearly Support")
        .expect("yearly support")
        .id;
    env.services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            subs[0],
            TENANT_ID,
            amend(ComponentChanges {
                edited: vec![],
                added: vec![],
                removed: vec![support],
            }),
            day(3, 15),
        )
        .await
        .expect("remove the yearly support fee");
    env.process_cycles().await;

    let support_lines = |invoices: &[meteroid_store::domain::Invoice], on: NaiveDate| -> i64 {
        invoices
            .iter()
            .filter(|i| i.billing_period_start == on && i.status != InvoiceStatusEnum::Void)
            .filter(|i| i.consolidated_into_invoice_id.is_none())
            .flat_map(|i| &i.line_items)
            .filter(|l| l.name == "Yearly Support")
            .map(|l| l.amount_subtotal)
            .sum()
    };
    let april: Vec<_> = env
        .get_customer_invoices(CUST_UBER_ID)
        .await
        .into_iter()
        .filter(|i| i.billing_period_start == day(4, 1) && i.status == InvoiceStatusEnum::Finalized)
        .filter(|i| i.consolidated_into_invoice_id.is_none())
        .collect();
    assert_eq!(support_lines(&april, day(4, 1)), 7380);
    let billed = april
        .iter()
        .find(|i| i.line_items.iter().any(|l| l.name == "Yearly Support"))
        .expect("the invoice billing April's renewal")
        .id;
    env.try_void_invoice(billed).await.expect("void April");
    let rate = env
        .get_subscription_components(subs[0])
        .await
        .into_iter()
        .find(|c| c.name == "Subscription Rate")
        .expect("the rate")
        .id;
    env.services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            subs[0],
            TENANT_ID,
            amend(ComponentChanges {
                edited: vec![
                    meteroid_store::domain::subscription_amendment::EditComponent {
                        subscription_component_id: rate,
                        name: None,
                        price_entry: PriceEntry::New(PriceInput {
                            cadence: BillingPeriodEnum::Monthly,
                            currency: "EUR".to_string(),
                            pricing: Pricing::Rate {
                                rate: Decimal::new(6_500, 2),
                            },
                            credit_unit: None,
                        }),
                    },
                ],
                added: vec![],
                removed: vec![],
            }),
            day(4, 16),
        )
        .await
        .expect("raise the rate");

    env.process_cycles().await;
    let may: Vec<_> = env.get_customer_invoices(CUST_UBER_ID).await;
    let raise = may
        .iter()
        .filter(|i| {
            i.subscription_id == Some(subs[0])
                && i.invoice_type == meteroid_store::domain::enums::InvoiceType::Adjustment
        })
        .flat_map(|i| &i.line_items)
        .filter(|l| l.name.starts_with("Subscription Rate"))
        .map(|l| l.amount_subtotal)
        .sum();
    (support_lines(&may, day(5, 1)), raise)
}

/// The facts bill those arrears once, on the voided renewal, never on a later one: May 1 bills
/// none of them, the void's merged draft still counting. The raise owes April's 3500 − 1750 =
/// 1750 and 6500 × 15/30 = 3250, 5000 less the voided 3500: 1500, never the voided rate again.
/// Rules: R-011, R-084, R-096
#[rstest]
#[tokio::test]
async fn test_arrears_of_a_voided_consolidated_renewal_are_billed_once(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    assert_eq!(
        yearly_arrears_billed_after_a_voided_consolidated_renewal(&env).await,
        (0, 1_500)
    );
}

/// The reconcile watermark's backfill reads a consolidated renewal as issued once its parent is:
/// February's children stay drafts, merged into a finalized parent, so both subscriptions were
/// billed by February's start.
#[rstest]
#[tokio::test]
async fn the_reconcile_watermark_counts_a_consolidated_renewal_as_issued(
    #[future] test_env: TestEnv,
) {
    use diesel::prelude::*;
    use diesel_async::{RunQueryDsl, SimpleAsyncConnection};
    use diesel_models::schema::subscription::dsl as s;

    let env = test_env.await;
    env.set_consolidate_recurring_invoices(true).await;
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let mut subs = Vec::new();
    for _ in 0..2 {
        subs.push(
            subscription()
                .customer(CUST_UBER_ID)
                .plan_version(PLAN_VERSION_1_LEETCODE_ID)
                .start_date(start_date)
                .on_start()
                .no_trial()
                .create(env.services())
                .await,
        );
    }
    env.process_cycles().await;
    assert_eq!(consolidated_parents(&env, CUST_UBER_ID).await.len(), 1);

    let migration = crate::release_migrations::migration_section(
        crate::release_migrations::BILLING_ON_FACTS_UP,
        "Reconcile watermark",
    );
    let backfill = &migration[migration.find("UPDATE subscription").expect("the backfill")..];
    let mut conn = env.conn().await;
    diesel::update(s::subscription.filter(s::id.eq_any(&subs)))
        .set(s::reconcile_from.eq(None::<NaiveDate>))
        .execute(&mut conn)
        .await
        .unwrap();
    conn.batch_execute(backfill)
        .await
        .expect("the backfill runs");
    let watermarks: Vec<Option<NaiveDate>> = s::subscription
        .filter(s::id.eq_any(&subs))
        .select(s::reconcile_from)
        .load(&mut conn)
        .await
        .unwrap();
    assert_eq!(
        watermarks,
        vec![Some(NaiveDate::from_ymd_opt(2024, 2, 1).unwrap()); 2]
    );
}

/// Each line of the credit notes on `invoice_id`: its local id and amount.
async fn note_lines(
    env: &TestEnv,
    invoice_id: common_domain::ids::InvoiceId,
) -> Vec<(String, i64)> {
    use meteroid_store::repositories::CreditNoteInterface;
    let mut lines = Vec::new();
    for note in env
        .store()
        .list_credit_notes_by_invoice_id(TENANT_ID, invoice_id)
        .await
        .expect("credit notes")
    {
        lines.extend(
            note.line_items
                .iter()
                .map(|line| (line.local_id.clone(), line.amount_subtotal)),
        );
    }
    lines
}

/// Two 35.00 monthly subscriptions from Jan 1, 2024, renewals merged; on Mar 5 one is cancelled
/// from Jan 20: January gives back 3500 × 12/31 = 1355, each merged renewal its 3500 whole.
/// Rules: R-016, R-077, R-080
#[rstest]
#[tokio::test]
async fn a_backdated_cancellation_gives_back_the_periods_its_merged_renewals_billed(
    #[future] test_env: TestEnv,
) {
    use common_domain::actor::Actor;
    use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;

    let env = test_env.await;
    let day = |m: u32, d: u32| NaiveDate::from_ymd_opt(2024, m, d).unwrap();
    env.set_now(day(1, 1).and_hms_opt(1, 0, 0).unwrap().and_utc());
    env.set_consolidate_recurring_invoices(true).await;
    let mut subs = Vec::new();
    for _ in 0..2 {
        subs.push(
            subscription()
                .customer(CUST_UBER_ID)
                .plan_version(PLAN_VERSION_1_LEETCODE_ID)
                .start_date(day(1, 1))
                .on_start()
                .no_trial()
                .create(env.services())
                .await,
        );
    }
    let (cancelled, kept) = (subs[0], subs[1]);
    env.run_until(day(3, 5)).await;
    let parents = consolidated_parents(&env, CUST_UBER_ID).await;
    assert_eq!(parents.len(), 2, "the Feb 1 and Mar 1 renewals, merged");

    env.services()
        .cancel_subscription(
            Actor::System,
            cancelled,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(day(1, 20)),
        )
        .await
        .expect("cancelled from Jan 20");
    env.process_cycles().await;
    env.run_outbox_and_orchestration().await;

    for parent in &parents {
        let children = env.get_consolidated_children(parent.id).await;
        let member = |sub| {
            children
                .iter()
                .find(|child| child.subscription_id == Some(sub))
                .expect("a merged draft of each subscription")
        };
        assert_eq!(member(cancelled).status, InvoiceStatusEnum::Draft);
        assert_eq!(member(kept).status, InvoiceStatusEnum::Draft);
        let line = &member(cancelled).line_items[0];
        assert_eq!(
            (
                parent.billing_period_start,
                note_lines(&env, parent.id).await
            ),
            (
                parent.billing_period_start,
                vec![(line.local_id.clone(), -3500)]
            ),
            "the cancelled subscription's merged line comes back whole, the other's stays"
        );
    }
    let own: Vec<_> = env
        .get_invoices(cancelled)
        .await
        .into_iter()
        .filter(|invoice| {
            invoice.subscription_id == Some(cancelled)
                && invoice.consolidated_into_invoice_id.is_none()
        })
        .collect();
    let opening = own
        .iter()
        .find(|invoice| invoice.billing_period_start == day(1, 1))
        .expect("January's invoice");
    assert_eq!(
        note_lines(&env, opening.id).await,
        vec![(opening.line_items[0].local_id.clone(), -1355)]
    );
    assert!(
        own.iter()
            .all(|invoice| invoice.billing_period_start == day(1, 1) || invoice.total == 0),
        "nothing else billed: {own:?}"
    );
}
