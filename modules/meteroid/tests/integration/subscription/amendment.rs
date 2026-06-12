//! Subscription amendment integration tests.
//!
//! Manual/sales-led changes to a live subscription: add/remove/edit components
//! and add-ons, with Immediate (proration + adjustment invoice) and EndOfPeriod
//! (scheduled) modes. Uses the product-backed Starter plan (EUR).

use chrono::{NaiveDate, NaiveTime};
use rstest::rstest;
use std::collections::HashMap;
use std::sync::Arc;

use crate::data::ids::*;
use crate::harness::{
    InvoicesAssertExt, SubscriptionAssertExt, TestEnv, subscription, test_env, test_env_with_usage,
};
use diesel_models::subscription_add_ons::SubscriptionAddOnRow;
use meteroid_store::clients::usage::{
    GroupedUsageData, MockUsageClient, MockUsageDataParams, UsageData,
};
use meteroid_store::domain::enums::FeeTypeEnum;
use meteroid_store::domain::enums::{
    BillingType, CreditType, InvoicePaymentStatus, InvoiceStatusEnum, InvoiceType,
};
use meteroid_store::domain::price_components::{PriceEntry, PriceInput, ProductRef};
use meteroid_store::domain::prices::{FeeStructure, Pricing, UsageModel};
use meteroid_store::domain::subscription_add_ons::{
    CreateSubscriptionAddOn, SubscriptionAddOnCustomization,
};
use meteroid_store::domain::subscription_amendment::{
    AddExtraComponent, AddOnChanges, ComponentChanges, EditComponent, EditSubscriptionAddOn,
    SubscriptionAmendment,
};
use meteroid_store::domain::subscription_changes::{ChangeDirection, PlanChangeMode};
use meteroid_store::domain::{BillingPeriodEnum, UsagePeriod, UsagePricingModel};
use meteroid_store::repositories::add_ons::AddOnInterface;
use meteroid_store::repositories::credit_notes::CreditNoteInterface;
use rust_decimal::Decimal;

/// Create a catalog add-on (new product) with a monthly EUR Rate price.
async fn create_rate_addon(
    env: &TestEnv,
    name: &str,
    rate_cents: i64,
) -> common_domain::ids::AddOnId {
    env.store()
        .create_add_on_from_ref(
            name.to_string(),
            ProductRef::New {
                name: format!("{name} Product"),
                fee_type: FeeTypeEnum::Rate,
                fee_structure: FeeStructure::Rate {},
            },
            PriceEntry::New(PriceInput {
                cadence: BillingPeriodEnum::Monthly,
                currency: "EUR".to_string(),
                pricing: Pricing::Rate {
                    rate: Decimal::new(rate_cents, 2),
                },
            }),
            None,
            true,
            Some(10),
            TENANT_ID,
            PRODUCT_FAMILY_ID,
            vec![],
        )
        .await
        .expect("create_add_on_from_ref failed")
        .id
}

/// Build a MockUsageClient keyed on (metric_id, period_start, period_end).
fn build_usage_mock(entries: Vec<(MockUsageDataParams, Decimal)>) -> Arc<MockUsageClient> {
    let mut data = HashMap::new();
    for (params, value) in entries {
        let period = UsagePeriod {
            start: params.period_start,
            end: params.period_end,
        };
        data.insert(
            params,
            UsageData {
                data: vec![GroupedUsageData {
                    value,
                    dimensions: HashMap::new(),
                }],
                period,
            },
        );
    }
    Arc::new(MockUsageClient { data })
}

fn add_addon_amendment(
    mode: PlanChangeMode,
    add_on_id: common_domain::ids::AddOnId,
    quantity: i32,
) -> SubscriptionAmendment {
    SubscriptionAmendment {
        apply_mode: mode,
        component_changes: ComponentChanges::default(),
        add_on_changes: AddOnChanges {
            added: vec![CreateSubscriptionAddOn {
                add_on_id,
                customization: SubscriptionAddOnCustomization::None,
                quantity,
            }],
            edited: vec![],
            removed: vec![],
        },
    }
}

/// Build an ad-hoc extra Rate component (new product + new monthly EUR price).
fn extra_rate_component(name: &str, rate_cents: i64) -> AddExtraComponent {
    AddExtraComponent {
        name: name.to_string(),
        product: ProductRef::New {
            name: format!("{name} Product"),
            fee_type: FeeTypeEnum::Rate,
            fee_structure: FeeStructure::Rate {},
        },
        price_entry: PriceEntry::New(PriceInput {
            cadence: BillingPeriodEnum::Monthly,
            currency: "EUR".to_string(),
            pricing: Pricing::Rate {
                rate: Decimal::new(rate_cents, 2),
            },
        }),
    }
}

/// Build an ad-hoc extra fixed-rate arrears component (billed at period end).
fn extra_arrears_component(name: &str, rate_cents: i64) -> AddExtraComponent {
    AddExtraComponent {
        name: name.to_string(),
        product: ProductRef::New {
            name: format!("{name} Product"),
            fee_type: FeeTypeEnum::ExtraRecurring,
            fee_structure: FeeStructure::ExtraRecurring {
                billing_type: BillingType::Arrears,
            },
        },
        price_entry: PriceEntry::New(PriceInput {
            cadence: BillingPeriodEnum::Monthly,
            currency: "EUR".to_string(),
            pricing: Pricing::ExtraRecurring {
                unit_price: Decimal::new(rate_cents, 2),
                quantity: 1,
            },
        }),
    }
}

/// Build an ad-hoc one-time component (billed in full, once).
fn extra_onetime_component(name: &str, rate_cents: i64) -> AddExtraComponent {
    AddExtraComponent {
        name: name.to_string(),
        product: ProductRef::New {
            name: format!("{name} Product"),
            fee_type: FeeTypeEnum::OneTime,
            fee_structure: FeeStructure::OneTime {},
        },
        price_entry: PriceEntry::New(PriceInput {
            // One-time fees have no cadence; Monthly is an inert placeholder.
            cadence: BillingPeriodEnum::Monthly,
            currency: "EUR".to_string(),
            pricing: Pricing::OneTime {
                unit_price: Decimal::new(rate_cents, 2),
                quantity: 1,
            },
        }),
    }
}

fn amendment_add_component(mode: PlanChangeMode, comp: AddExtraComponent) -> SubscriptionAmendment {
    SubscriptionAmendment {
        apply_mode: mode,
        component_changes: ComponentChanges {
            edited: vec![],
            added: vec![comp],
            removed: vec![],
        },
        add_on_changes: AddOnChanges::default(),
    }
}

// =============================================================================
// PREVIEW
// =============================================================================

/// Preview of an amendment that adds a €50/mo component: reports the added line,
/// upgrade direction, and (for Immediate) a proration summary.
/// Exact prorated amounts are asserted by the apply tests below.
#[rstest]
#[tokio::test]
async fn test_preview_amendment_add_component(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // End-of-period preview: structural, no proration.
    let eop = env
        .services()
        .preview_amendment(
            sub_id,
            TENANT_ID,
            amendment_add_component(
                PlanChangeMode::EndOfPeriod,
                extra_rate_component("Support", 5000),
            ),
        )
        .await
        .expect("preview_amendment (eop) failed");
    assert_eq!(eop.preview.component_added.len(), 1);
    assert!(eop.preview.component_removed.is_empty());
    assert_eq!(eop.change_direction, ChangeDirection::Upgrade);
    assert!(
        eop.proration.is_none(),
        "end-of-period preview has no proration"
    );

    // Immediate preview: a proration summary is produced (amount asserted in apply tests).
    let imm = env
        .services()
        .preview_amendment(
            sub_id,
            TENANT_ID,
            amendment_add_component(
                PlanChangeMode::Immediate,
                extra_rate_component("Support", 5000),
            ),
        )
        .await
        .expect("preview_amendment (immediate) failed");
    assert_eq!(imm.preview.component_added.len(), 1);
    assert_eq!(imm.change_direction, ChangeDirection::Upgrade);
    assert!(
        imm.proration.is_some(),
        "immediate preview has a proration summary"
    );
}

/// Immediate preview of adding a €50/mo component reports: an MRR increase of
/// 5000, a prorated adjustment invoice (less than a full month) with the new
/// component line, and a next-cycle invoice that bills the component in full.
#[rstest]
#[tokio::test]
async fn test_preview_amendment_invoices_and_mrr(#[future] test_env: TestEnv) {
    let env = test_env.await;
    // Immediate preview prorates against "now", so the current period must contain
    // today: start the subscription 10 days ago.
    let start_date = chrono::Utc::now().naive_utc().date() - chrono::Duration::days(10);

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let mrr_before = env.get_subscription(sub_id).await.mrr_cents as i64;

    let preview = env
        .services()
        .preview_amendment(
            sub_id,
            TENANT_ID,
            amendment_add_component(
                PlanChangeMode::Immediate,
                extra_rate_component("Support", 5000),
            ),
        )
        .await
        .expect("preview_amendment failed");

    // MRR rises by the added component's monthly rate.
    assert_eq!(preview.mrr_before_cents, mrr_before);
    assert_eq!(preview.mrr_after_cents, mrr_before + 5000);

    // Adjustment invoice: a prorated charge for the remainder of the period.
    let adjustment = preview
        .adjustment_invoice
        .expect("immediate preview has an adjustment invoice");
    assert!(
        adjustment
            .invoice_lines
            .iter()
            .any(|l| l.name.contains("Support")),
        "adjustment invoice should include the added component"
    );
    assert!(
        adjustment.total > 0 && adjustment.total <= 5000,
        "adjustment is prorated (at most a full month), got {}",
        adjustment.total
    );

    // Next renewal invoice bills the added component in full.
    let next = preview
        .next_invoice
        .expect("preview has a next-cycle invoice");
    assert!(
        next.invoice_lines
            .iter()
            .any(|l| l.name.contains("Support")),
        "next invoice should include the added component"
    );
}

/// Immediate preview of a downgrade (component removal) on a finalized current-period
/// invoice produces a credit-note preview with negative line amounts. Regression for
/// "proration shows a credit but no credit-note preview appears".
#[rstest]
#[tokio::test]
async fn test_preview_amendment_credit_note(#[future] test_env: TestEnv) {
    let env = test_env.await;
    // The current period must contain today (the preview prorates against "now")
    // and its invoice must be finalized for a credit note to be issuable.
    let start_date = chrono::Utc::now().naive_utc().date() - chrono::Duration::days(10);

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Finalize the current-period invoice so there is something to credit against.
    env.run_outbox_and_orchestration().await;

    let platform = env
        .get_subscription_components(sub_id)
        .await
        .into_iter()
        .find(|c| c.name == "Platform Fee")
        .expect("Platform Fee component");

    let preview = env
        .services()
        .preview_amendment(
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges {
                    edited: vec![],
                    added: vec![],
                    removed: vec![platform.id],
                },
                add_on_changes: AddOnChanges::default(),
            },
        )
        .await
        .expect("preview_amendment failed");

    // The credit side is non-empty…
    let proration = preview.proration.expect("immediate preview has proration");
    assert!(
        proration.credits_total_cents < 0,
        "a removal should produce a credit, got {}",
        proration.credits_total_cents
    );

    // …so a credit-note preview must be present, with negative (credit) amounts.
    let credit_note = preview
        .credit_note
        .expect("a downgrade against a finalized invoice has a credit-note preview");
    assert!(
        credit_note.total < 0,
        "credit note total should be negative, got {}",
        credit_note.total
    );
    assert!(
        credit_note
            .invoice_lines
            .iter()
            .any(|l| l.name.contains("Platform Fee")),
        "credit note should reference the removed Platform Fee line"
    );
}

// =============================================================================
// APPLY IMMEDIATE — ADD COMPONENT
// =============================================================================

/// Apply immediate add of a €50/mo component at Jan 16: prorated adjustment
/// invoice for 16/31 of 5000 = 2581, MRR increases by exactly 5000.
#[rstest]
#[tokio::test]
async fn test_apply_amendment_add_component_immediate(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let change_date = NaiveDate::from_ymd_opt(2024, 1, 16).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let mrr_before = env.get_subscription(sub_id).await.mrr_cents;

    let amendment = amendment_add_component(
        PlanChangeMode::Immediate,
        extra_rate_component("Support", 5000),
    );

    let result = env
        .services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            amendment,
            change_date,
        )
        .await
        .expect("apply_amendment_immediate_at failed");

    assert!(
        result.adjustment_invoice_id.is_some(),
        "should create adjustment invoice for the added component"
    );

    // Active components: Platform Fee, Seats, + new Support = 3.
    let components = env.get_subscription_components(sub_id).await;
    assert_eq!(components.len(), 3, "should have 3 active components");
    assert!(components.iter().any(|c| c.name == "Support"));
    let support = components.iter().find(|c| c.name == "Support").unwrap();
    assert_eq!(support.effective_from, change_date);
    assert!(support.effective_to.is_none());

    // MRR increases by exactly the new component's monthly rate.
    let mrr_after = env.get_subscription(sub_id).await.mrr_cents;
    assert_eq!(mrr_after, mrr_before + 5000);

    // Initial Starter invoice + adjustment invoice.
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);
    invoices
        .assert()
        .invoice_at(1)
        .with_context("add-component adjustment")
        .has_status(meteroid_store::domain::enums::InvoiceStatusEnum::Finalized)
        .check_prorated(true)
        .has_total(2581);
}

// =============================================================================
// APPLY IMMEDIATE — REMOVE COMPONENT
// =============================================================================

/// Apply immediate removal of the €29/mo Platform Fee at Jan 16: the component
/// is closed (effective_to = change_date), MRR drops by 2900, and a credit
/// adjustment invoice is produced.
#[rstest]
#[tokio::test]
async fn test_apply_amendment_remove_component_immediate(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let change_date = NaiveDate::from_ymd_opt(2024, 1, 16).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let mrr_before = env.get_subscription(sub_id).await.mrr_cents;

    let platform = env
        .get_subscription_components(sub_id)
        .await
        .into_iter()
        .find(|c| c.name == "Platform Fee")
        .expect("Platform Fee component");

    let amendment = SubscriptionAmendment {
        apply_mode: PlanChangeMode::Immediate,
        component_changes: ComponentChanges {
            edited: vec![],
            added: vec![],
            removed: vec![platform.id],
        },
        add_on_changes: AddOnChanges::default(),
    };

    env.services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            amendment,
            change_date,
        )
        .await
        .expect("apply_amendment_immediate_at failed");

    // Only the Seats component remains active.
    let active = env.get_subscription_components(sub_id).await;
    assert_eq!(active.len(), 1, "only Seats should remain active");
    assert_eq!(active[0].name, "Seats");

    // The closed Platform Fee row carries effective_to = change_date.
    let history = env
        .get_all_subscription_components(
            sub_id,
            start_date,
            change_date + chrono::Duration::days(1),
        )
        .await;
    let closed = history
        .iter()
        .find(|c| c.id == platform.id)
        .expect("closed Platform Fee row");
    assert_eq!(closed.effective_to, Some(change_date));

    // MRR drops by the removed component's monthly rate.
    let mrr_after = env.get_subscription(sub_id).await.mrr_cents;
    assert_eq!(mrr_after, mrr_before - 2900);
}

// =============================================================================
// SCHEDULE & CANCEL (END OF PERIOD)
// =============================================================================

/// EndOfPeriod amendment schedules an event at current_period_end and can be cancelled.
#[rstest]
#[tokio::test]
async fn test_schedule_and_cancel_amendment(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let amendment = amendment_add_component(
        PlanChangeMode::EndOfPeriod,
        extra_rate_component("Support", 5000),
    );

    let event = env
        .services()
        .schedule_amendment(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            amendment,
        )
        .await
        .expect("schedule_amendment failed");

    assert_eq!(event.subscription_id, sub_id);
    let sub = env.get_subscription(sub_id).await;
    assert_eq!(
        event.scheduled_time,
        sub.current_period_end
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
    );

    // No new components yet (only scheduled).
    assert_eq!(env.get_subscription_components(sub_id).await.len(), 2);

    // Cancel succeeds, and a second cancel reports nothing pending.
    env.services()
        .cancel_amendment(common_domain::actor::Actor::System, sub_id, TENANT_ID)
        .await
        .expect("cancel_amendment failed");
    assert!(
        env.services()
            .cancel_amendment(common_domain::actor::Actor::System, sub_id, TENANT_ID)
            .await
            .is_err(),
        "second cancel should error (no pending amendment)"
    );
}

// =============================================================================
// APPLY IMMEDIATE — ADD ADD-ON (headline)
// =============================================================================

/// Attach a catalog add-on (€20/mo) to a live subscription mid-period: the
/// add-on becomes active with effective_from = change_date, MRR increases by
/// 2000, and a prorated adjustment invoice is produced.
#[rstest]
#[tokio::test]
async fn test_apply_amendment_add_addon_immediate(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let change_date = NaiveDate::from_ymd_opt(2024, 1, 16).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Create a catalog add-on (new product + €20/mo EUR price).
    let add_on = env
        .store()
        .create_add_on_from_ref(
            "Premium Support".to_string(),
            ProductRef::New {
                name: "Premium Support Product".to_string(),
                fee_type: FeeTypeEnum::Rate,
                fee_structure: FeeStructure::Rate {},
            },
            PriceEntry::New(PriceInput {
                cadence: BillingPeriodEnum::Monthly,
                currency: "EUR".to_string(),
                pricing: Pricing::Rate {
                    rate: Decimal::new(2000, 2),
                },
            }),
            None,
            true,
            Some(5),
            TENANT_ID,
            PRODUCT_FAMILY_ID,
            vec![],
        )
        .await
        .expect("create_add_on_from_ref failed");

    let mrr_before = env.get_subscription(sub_id).await.mrr_cents;

    let amendment = SubscriptionAmendment {
        apply_mode: PlanChangeMode::Immediate,
        component_changes: ComponentChanges::default(),
        add_on_changes: AddOnChanges {
            added: vec![CreateSubscriptionAddOn {
                add_on_id: add_on.id,
                customization: SubscriptionAddOnCustomization::None,
                quantity: 1,
            }],
            edited: vec![],
            removed: vec![],
        },
    };

    let result = env
        .services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            amendment,
            change_date,
        )
        .await
        .expect("apply_amendment_immediate_at failed");

    assert!(result.adjustment_invoice_id.is_some());

    // The add-on is active with the right temporal bounds.
    let mut conn = env.conn().await;
    let active_addons =
        SubscriptionAddOnRow::list_by_subscription_id_active(&mut conn, &TENANT_ID, &sub_id)
            .await
            .expect("list active add-ons");
    assert_eq!(active_addons.len(), 1);
    assert_eq!(active_addons[0].add_on_id, add_on.id);
    assert_eq!(active_addons[0].effective_from, change_date);
    assert!(active_addons[0].effective_to.is_none());

    // MRR increases by the add-on's monthly rate.
    let mrr_after = env.get_subscription(sub_id).await.mrr_cents;
    assert_eq!(mrr_after, mrr_before + 2000);
}

/// Adding a multi-instance add-on (quantity 3) immediately mid-period must show the
/// real instance count on the prorated adjustment-invoice line, with the effective
/// unit price reconciling to the line total (no `1 × total`, no repeating decimal).
#[rstest]
#[tokio::test]
async fn test_apply_amendment_add_multi_instance_addon_shows_count(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let change_date = NaiveDate::from_ymd_opt(2024, 1, 16).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // €20/mo add-on, up to 5 instances.
    let add_on = env
        .store()
        .create_add_on_from_ref(
            "Premium Support".to_string(),
            ProductRef::New {
                name: "Premium Support Product".to_string(),
                fee_type: FeeTypeEnum::Rate,
                fee_structure: FeeStructure::Rate {},
            },
            PriceEntry::New(PriceInput {
                cadence: BillingPeriodEnum::Monthly,
                currency: "EUR".to_string(),
                pricing: Pricing::Rate {
                    rate: Decimal::new(2000, 2),
                },
            }),
            None,
            true,
            Some(5),
            TENANT_ID,
            PRODUCT_FAMILY_ID,
            vec![],
        )
        .await
        .expect("create_add_on_from_ref failed");

    let amendment = SubscriptionAmendment {
        apply_mode: PlanChangeMode::Immediate,
        component_changes: ComponentChanges::default(),
        add_on_changes: AddOnChanges {
            added: vec![CreateSubscriptionAddOn {
                add_on_id: add_on.id,
                customization: SubscriptionAddOnCustomization::None,
                quantity: 3,
            }],
            edited: vec![],
            removed: vec![],
        },
    };

    let result = env
        .services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            amendment,
            change_date,
        )
        .await
        .expect("apply_amendment_immediate_at failed");
    assert!(result.adjustment_invoice_id.is_some());

    // The prorated charge line for the add-on.
    let invoices = env.get_invoices(sub_id).await;
    let line = invoices
        .iter()
        .flat_map(|i| &i.line_items)
        .find(|l| l.name.contains("Premium Support"))
        .expect("add-on charge line on the adjustment invoice");

    assert!(line.amount_subtotal > 0, "charge line should be positive");
    assert!(line.is_prorated, "mid-period add-on charge is prorated");
    assert_eq!(
        line.quantity,
        Some(Decimal::from(3)),
        "shows the real instance count (3), not 1"
    );
    let q = line.quantity.unwrap();
    let p = line.unit_price.expect("effective unit price");
    assert_eq!(
        (q * p).round_dp(2),
        Decimal::new(line.amount_subtotal, 2),
        "qty × unit_price reconciles to the line subtotal"
    );
}

// =============================================================================
// END-OF-PERIOD EXECUTION (#1)
// =============================================================================

/// A scheduled (EndOfPeriod) amendment is applied at current_period_end by the
/// cycle worker, and the renewal invoice reflects the amended component set.
/// Starter €39/mo; after adding a €50/mo component the renewal is €89/mo.
#[rstest]
#[tokio::test]
async fn test_end_of_period_amendment_executes_at_period_end(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let feb1 = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    let mar1 = NaiveDate::from_ymd_opt(2024, 3, 1).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Cycle 0 invoice: Starter €39.
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices.assert().invoice_at(0).has_total(3900);

    // Schedule an end-of-period amendment adding a €50/mo component.
    env.services()
        .schedule_amendment(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            amendment_add_component(
                PlanChangeMode::EndOfPeriod,
                extra_rate_component("Support", 5000),
            ),
        )
        .await
        .expect("schedule_amendment failed");

    // Not yet applied.
    assert_eq!(env.get_subscription_components(sub_id).await.len(), 2);

    // Cycle 1: the worker applies the amendment and renews in one pass.
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_cycle_index(1)
        .has_period_start(feb1)
        .has_period_end(mar1);

    let components = env.get_subscription_components(sub_id).await;
    assert_eq!(components.len(), 3, "Support should be active after apply");
    let support = components.iter().find(|c| c.name == "Support").unwrap();
    assert_eq!(support.effective_from, feb1);

    // Renewal invoice reflects the amended set: €39 + €50 = €89.
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);
    invoices
        .assert()
        .invoice_at(1)
        .with_context("cycle 1 - amended renewal")
        .is_finalized_unpaid()
        .has_total(8900)
        .has_period(feb1, mar1);
}

/// Regression: an ARREARS component added immediately mid-period must be prorated
/// (not billed in full) on the next invoice. Adding €30/mo arrears at Jan 16 of a
/// [Jan 1, Feb 1] period charges nothing immediately (arrears are excluded from the
/// adjustment invoice) and bills 16/31 of €30 = €15.48 at the Feb 1 renewal, on top
/// of the €39 advance for the new period.
#[rstest]
#[tokio::test]
async fn test_add_arrears_component_immediate_prorated_at_renewal(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let jan1 = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let jan16 = NaiveDate::from_ymd_opt(2024, 1, 16).unwrap();
    let feb1 = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    let mar1 = NaiveDate::from_ymd_opt(2024, 3, 1).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(jan1)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Add a €30/mo arrears component at Jan 16 (immediate).
    let result = env
        .services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            amendment_add_component(
                PlanChangeMode::Immediate,
                extra_arrears_component("Overage Fee", 3000),
            ),
            jan16,
        )
        .await
        .expect("add arrears component failed");

    // Arrears are billed at period end, so nothing is charged immediately.
    assert!(
        result.adjustment_invoice_id.is_none(),
        "an arrears add must not produce an immediate adjustment invoice"
    );

    // Cycle 1 renewal [Feb 1, Mar 1]: €39 advance + 16/31 of €30 arrears for
    // [Jan 16, Feb 1] = 1548 cents → €54.48 total.
    env.process_cycles().await;

    let invoices = env.get_invoices(sub_id).await;
    let renewal = invoices
        .iter()
        .find(|i| {
            i.line_items
                .iter()
                .any(|l| l.start_date == feb1 && l.end_date == mar1)
        })
        .expect("cycle 1 renewal invoice");

    let arrears_line = renewal
        .line_items
        .iter()
        .find(|l| l.name.contains("Overage Fee"))
        .expect("renewal must bill the arrears component");
    assert_eq!(
        arrears_line.amount_subtotal, 1548,
        "arrears must be prorated to 16/31 of €30, not billed in full"
    );
    assert!(
        arrears_line.is_prorated,
        "the mid-period arrears charge is prorated"
    );
    assert_eq!(arrears_line.start_date, jan16);
    assert_eq!(arrears_line.end_date, feb1);
    assert_eq!(renewal.total, 5448, "€39 advance + €15.48 prorated arrears");
}

/// Regression: a ONE-TIME component scheduled end-of-period must appear on the next
/// period's invoice (billed in full, once) and not be silently dropped because
/// `applies_this_period` excludes one-time fees on cycles > 0.
#[rstest]
#[tokio::test]
async fn test_end_of_period_onetime_component_billed_once(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let jan1 = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let feb1 = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    let mar1 = NaiveDate::from_ymd_opt(2024, 3, 1).unwrap();
    let apr1 = NaiveDate::from_ymd_opt(2024, 4, 1).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(jan1)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Schedule an end-of-period amendment adding a €100 one-time component.
    env.services()
        .schedule_amendment(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            amendment_add_component(
                PlanChangeMode::EndOfPeriod,
                extra_onetime_component("Onboarding", 10000),
            ),
        )
        .await
        .expect("schedule_amendment failed");

    // Cycle 1 renewal [Feb 1, Mar 1]: Starter €39 + one-time €100 = €139.
    env.process_cycles().await;

    let invoices = env.get_invoices(sub_id).await;
    let cycle1 = invoices
        .iter()
        .find(|i| {
            i.line_items
                .iter()
                .any(|l| l.start_date == feb1 && l.end_date == mar1)
        })
        .expect("cycle 1 renewal invoice");

    let onetime_line = cycle1
        .line_items
        .iter()
        .find(|l| l.name.contains("Onboarding"))
        .expect("renewal must bill the one-time component");
    assert_eq!(
        onetime_line.amount_subtotal, 10000,
        "one-time fee is billed in full"
    );
    assert!(
        !onetime_line.is_prorated,
        "one-time fees are never prorated"
    );
    assert_eq!(cycle1.total, 13900, "€39 recurring + €100 one-time");

    // Cycle 2 renewal [Mar 1, Apr 1] must NOT bill the one-time fee again.
    env.process_cycles().await;
    let invoices = env.get_invoices(sub_id).await;
    let cycle2 = invoices
        .iter()
        .find(|i| {
            i.line_items
                .iter()
                .any(|l| l.start_date == mar1 && l.end_date == apr1)
        })
        .expect("cycle 2 renewal invoice");
    assert!(
        !cycle2
            .line_items
            .iter()
            .any(|l| l.name.contains("Onboarding")),
        "a one-time fee must be billed exactly once, not re-billed on later cycles"
    );
    assert_eq!(cycle2.total, 3900, "Starter only on the following renewal");
}

/// Regression for the preview: the next-cycle invoice preview must reflect both an
/// arrears component added immediately (prorated) and a one-time component scheduled
/// end-of-period (billed in full).
#[rstest]
#[tokio::test]
async fn test_preview_next_invoice_includes_arrears_and_onetime(#[future] test_env: TestEnv) {
    let env = test_env.await;
    // Immediate preview prorates against "now", so the current period must contain today.
    let start_date = chrono::Utc::now().naive_utc().date() - chrono::Duration::days(10);

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Immediate arrears add: prorated arrears must show on the next-cycle invoice.
    let arrears_preview = env
        .services()
        .preview_amendment(
            sub_id,
            TENANT_ID,
            amendment_add_component(
                PlanChangeMode::Immediate,
                extra_arrears_component("Overage Fee", 3000),
            ),
        )
        .await
        .expect("preview arrears failed");

    let next = arrears_preview
        .next_invoice
        .expect("preview has a next-cycle invoice");
    let arrears_line = next
        .invoice_lines
        .iter()
        .find(|l| l.name.contains("Overage Fee"))
        .expect("next invoice preview must include the arrears component");
    assert!(
        arrears_line.amount_subtotal > 0 && arrears_line.amount_subtotal < 3000,
        "arrears must be prorated (less than a full €30 month), got {}",
        arrears_line.amount_subtotal
    );

    // The proration summary must surface the deferred arrears charge in
    // arrears_charge_cents (not charges_total_cents, which tracks only the
    // immediate adjustment invoice).
    let summary = arrears_preview
        .proration
        .expect("immediate preview has a proration summary");
    assert_eq!(
        summary.credits_total_cents, 0,
        "adding an arrears component credits nothing"
    );
    assert_eq!(
        summary.charges_total_cents, 0,
        "no immediate adjustment invoice for a pure-arrears add"
    );
    assert_eq!(
        summary.net_amount_cents, 0,
        "net immediate adjustment is zero for a pure-arrears add"
    );
    assert!(
        summary.arrears_charge_cents > 0 && summary.arrears_charge_cents < 3000,
        "deferred arrears charge must be prorated (less than €30), got {}",
        summary.arrears_charge_cents
    );
    // It matches the prorated line on the next renewal invoice.
    assert_eq!(
        summary.arrears_charge_cents, arrears_line.amount_subtotal,
        "deferred arrears charge must match the next-invoice prorated arrears line"
    );

    // End-of-period one-time add: full charge on the next-cycle invoice preview.
    let onetime_preview = env
        .services()
        .preview_amendment(
            sub_id,
            TENANT_ID,
            amendment_add_component(
                PlanChangeMode::EndOfPeriod,
                extra_onetime_component("Onboarding", 10000),
            ),
        )
        .await
        .expect("preview one-time failed");

    let next = onetime_preview
        .next_invoice
        .expect("preview has a next-cycle invoice");
    let onetime_line = next
        .invoice_lines
        .iter()
        .find(|l| l.name.contains("Onboarding"))
        .expect("next invoice preview must include the end-of-period one-time component");
    assert_eq!(
        onetime_line.amount_subtotal, 10000,
        "one-time fee is previewed in full"
    );
}

// =============================================================================
// ADD-ON BILLING ACROSS A RENEWAL (#2)
// =============================================================================

/// An advance-billed add-on removed mid-cycle stops billing at the next renewal.
/// This is the headline correctness guarantee of the temporal add-on columns.
#[rstest]
#[tokio::test]
async fn test_remove_advance_addon_not_billed_at_renewal(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let feb1 = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    let feb16 = NaiveDate::from_ymd_opt(2024, 2, 16).unwrap();
    let mar1 = NaiveDate::from_ymd_opt(2024, 3, 1).unwrap();
    let apr1 = NaiveDate::from_ymd_opt(2024, 4, 1).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let add_on_id = create_rate_addon(&env, "Premium Support", 2000).await;

    // Add the add-on at Jan 16 (immediate, prorated).
    env.services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            add_addon_amendment(PlanChangeMode::Immediate, add_on_id, 1),
            NaiveDate::from_ymd_opt(2024, 1, 16).unwrap(),
        )
        .await
        .expect("add add-on failed");

    // Cycle 1 renewal [Feb 1, Mar 1] bills the add-on (€39 + €20 = €59).
    env.process_cycles().await;
    let invoices = env.get_invoices(sub_id).await;
    let cycle1 = invoices
        .iter()
        .find(|i| {
            i.line_items
                .iter()
                .any(|l| l.start_date == feb1 && l.end_date == mar1)
        })
        .expect("cycle 1 renewal invoice");
    assert_eq!(cycle1.total, 5900, "renewal with add-on should be €59");
    assert!(
        cycle1
            .line_items
            .iter()
            .any(|l| l.name.contains("Premium Support")),
        "active add-on should be billed on the renewal"
    );

    // Remove the add-on at Feb 16.
    env.services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges::default(),
                add_on_changes: AddOnChanges {
                    added: vec![],
                    edited: vec![],
                    removed: vec![active_addon_id(&env, sub_id).await],
                },
            },
            feb16,
        )
        .await
        .expect("remove add-on failed");

    // Cycle 2 renewal [Mar 1, Apr 1] must NOT bill the removed add-on.
    env.process_cycles().await;
    let invoices = env.get_invoices(sub_id).await;
    let cycle2 = invoices
        .iter()
        .find(|i| {
            i.line_items
                .iter()
                .any(|l| l.start_date == mar1 && l.end_date == apr1)
        })
        .expect("cycle 2 renewal invoice");
    assert_eq!(
        cycle2.total, 3900,
        "renewal after removal should be Starter only"
    );
    assert!(
        !cycle2
            .line_items
            .iter()
            .any(|l| l.name.contains("Premium Support")),
        "removed add-on must not appear on the renewal invoice"
    );
}

/// An arrears (usage) add-on removed mid-cycle still bills its final partial
/// segment at the next renewal, via `historical_addon_lines`.
#[tokio::test]
async fn test_remove_arrears_addon_final_segment_billed() {
    let jan1 = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
    let jan15 = NaiveDate::from_ymd_opt(2025, 1, 15).unwrap();
    let feb1 = NaiveDate::from_ymd_opt(2025, 2, 1).unwrap();

    // Usage for the add-on's active window [Jan 1, Jan 15]: 100 units.
    let usage_client = build_usage_mock(vec![(
        MockUsageDataParams {
            metric_id: METRIC_BANDWIDTH,
            period_start: jan1.and_time(NaiveTime::MIN),
            period_end: jan15.and_time(NaiveTime::MIN),
        },
        Decimal::new(100, 0),
    )]);
    let env = test_env_with_usage(usage_client).await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(jan1)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Create a usage add-on (€0.10/unit on METRIC_BANDWIDTH).
    let add_on_id = env
        .store()
        .create_add_on_from_ref(
            "Metered API".to_string(),
            ProductRef::New {
                name: "Metered API Product".to_string(),
                fee_type: FeeTypeEnum::Usage,
                fee_structure: FeeStructure::Usage {
                    metric_id: METRIC_BANDWIDTH,
                    model: UsageModel::PerUnit,
                },
            },
            PriceEntry::New(PriceInput {
                cadence: BillingPeriodEnum::Monthly,
                currency: "EUR".to_string(),
                pricing: Pricing::Usage(UsagePricingModel::PerUnit {
                    rate: Decimal::new(10, 2),
                }),
            }),
            None,
            true,
            None,
            TENANT_ID,
            PRODUCT_FAMILY_ID,
            vec![],
        )
        .await
        .expect("create usage add-on failed")
        .id;

    // Add at Jan 1, remove at Jan 15 (usage is excluded from proration → no adjustment).
    env.services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            add_addon_amendment(PlanChangeMode::Immediate, add_on_id, 1),
            jan1,
        )
        .await
        .expect("add usage add-on failed");

    env.services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges::default(),
                add_on_changes: AddOnChanges {
                    added: vec![],
                    edited: vec![],
                    removed: vec![active_addon_id(&env, sub_id).await],
                },
            },
            jan15,
        )
        .await
        .expect("remove usage add-on failed");

    // Renewal at Feb 1 bills cycle-0 arrears: the closed usage add-on's final
    // segment [Jan 1, Jan 15] = 100 × €0.10 = €10 via historical_addon_lines.
    env.process_cycles().await;

    let invoices = env.get_invoices(sub_id).await;
    let usage_line = invoices
        .iter()
        .flat_map(|i| &i.line_items)
        .find(|l| l.metric_id == Some(METRIC_BANDWIDTH))
        .expect("renewal should contain the historical usage add-on line");
    assert_eq!(usage_line.amount_total, 1000, "100 units × €0.10 = €10");
    let _ = feb1;
}

// =============================================================================
// ADD-ON QUANTITY CHANGE (#3)
// =============================================================================

/// Changing an add-on's quantity closes the old row and inserts a new one with
/// the new quantity; MRR tracks the delta.
#[rstest]
#[tokio::test]
async fn test_amendment_change_addon_quantity(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let jan10 = NaiveDate::from_ymd_opt(2024, 1, 10).unwrap();
    let jan16 = NaiveDate::from_ymd_opt(2024, 1, 16).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let add_on_id = create_rate_addon(&env, "Seats Pack", 2000).await;

    env.services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            add_addon_amendment(PlanChangeMode::Immediate, add_on_id, 1),
            jan10,
        )
        .await
        .expect("add add-on failed");

    let mrr_after_add = env.get_subscription(sub_id).await.mrr_cents;
    let sao_id = active_addon_id(&env, sub_id).await;

    // Bump quantity 1 → 3.
    env.services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges::default(),
                add_on_changes: AddOnChanges {
                    added: vec![],
                    edited: vec![EditSubscriptionAddOn {
                        subscription_add_on_id: sao_id,
                        quantity: Some(3),
                        customization: None,
                    }],
                    removed: vec![],
                },
            },
            jan16,
        )
        .await
        .expect("change quantity failed");

    // One active add-on row, now quantity 3.
    let mut conn = env.conn().await;
    let active =
        SubscriptionAddOnRow::list_by_subscription_id_active(&mut conn, &TENANT_ID, &sub_id)
            .await
            .unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].quantity, 3);
    assert_ne!(active[0].id, sao_id, "should be a new (re-inserted) row");

    // MRR: 1 unit (€20) → 3 units (€60), i.e. +€40 over the post-add MRR.
    let mrr_after_change = env.get_subscription(sub_id).await.mrr_cents;
    assert_eq!(mrr_after_change, mrr_after_add + 4000);
}

/// A multi-quantity add-on must bill for all instances at renewal — quantity is
/// stored separately from the per-unit fee, so the invoice path scales by it.
/// Regression guard: previously the renewal billed a single instance.
#[rstest]
#[tokio::test]
async fn test_addon_quantity_billed_at_renewal(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let jan16 = NaiveDate::from_ymd_opt(2024, 1, 16).unwrap();
    let feb1 = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    let mar1 = NaiveDate::from_ymd_opt(2024, 3, 1).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Add a €20/mo add-on with quantity 2 at Jan 16.
    let add_on_id = create_rate_addon(&env, "Premium Support", 2000).await;
    env.services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            add_addon_amendment(PlanChangeMode::Immediate, add_on_id, 2),
            jan16,
        )
        .await
        .expect("add add-on failed");

    // Cycle 1 renewal [Feb 1, Mar 1] bills the add-on for both instances:
    // Starter €39 + 2 × €20 = €79.
    env.process_cycles().await;
    let invoices = env.get_invoices(sub_id).await;
    let cycle1 = invoices
        .iter()
        .find(|i| {
            i.line_items
                .iter()
                .any(|l| l.start_date == feb1 && l.end_date == mar1)
        })
        .expect("cycle 1 renewal invoice");

    let addon_line = cycle1
        .line_items
        .iter()
        .find(|l| l.name.contains("Premium Support"))
        .expect("add-on line on the renewal invoice");
    assert_eq!(
        addon_line.amount_subtotal, 4000,
        "add-on should bill 2 × €20 = €40 at renewal, got {}",
        addon_line.amount_subtotal
    );
    assert_eq!(
        cycle1.total,
        3900 + 4000,
        "renewal total = Starter €39 + 2×€20"
    );
}

// =============================================================================
// COMPONENT PRICE EDIT (#4)
// =============================================================================

/// Overriding a component's price closes the old row and inserts an override
/// row with the new fee; MRR tracks the delta.
#[rstest]
#[tokio::test]
async fn test_amendment_edit_component_price(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let change_date = NaiveDate::from_ymd_opt(2024, 1, 16).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let mrr_before = env.get_subscription(sub_id).await.mrr_cents;
    let platform = env
        .get_subscription_components(sub_id)
        .await
        .into_iter()
        .find(|c| c.name == "Platform Fee")
        .expect("Platform Fee component");

    // Override Platform Fee €29 → €99.
    let result = env
        .services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges {
                    edited: vec![EditComponent {
                        subscription_component_id: platform.id,
                        name: None,
                        price_entry: PriceEntry::New(PriceInput {
                            cadence: BillingPeriodEnum::Monthly,
                            currency: "EUR".to_string(),
                            pricing: Pricing::Rate {
                                rate: Decimal::new(9900, 2),
                            },
                        }),
                    }],
                    added: vec![],
                    removed: vec![],
                },
                add_on_changes: AddOnChanges::default(),
            },
            change_date,
        )
        .await
        .expect("edit component failed");

    assert!(result.adjustment_invoice_id.is_some());

    // Still 2 active components; the Platform Fee row is replaced and is_override.
    let components = env.get_subscription_components(sub_id).await;
    assert_eq!(components.len(), 2);
    let platform_new = components
        .iter()
        .find(|c| c.name == "Platform Fee")
        .expect("overridden Platform Fee");
    assert_ne!(platform_new.id, platform.id, "should be a new override row");
    assert_eq!(platform_new.effective_from, change_date);

    // MRR rises by €99 − €29 = €70.
    let mrr_after = env.get_subscription(sub_id).await.mrr_cents;
    assert_eq!(mrr_after, mrr_before + 7000);
}

// =============================================================================
// SLOT COMPONENT (#5)
// =============================================================================

/// Removing a slot-fee component closes it and drops MRR by the slot amount.
#[rstest]
#[tokio::test]
async fn test_amendment_remove_slot_component(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let change_date = NaiveDate::from_ymd_opt(2024, 1, 16).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let mrr_before = env.get_subscription(sub_id).await.mrr_cents;
    let seats = env
        .get_subscription_components(sub_id)
        .await
        .into_iter()
        .find(|c| c.name == "Seats")
        .expect("Seats (slot) component");

    env.services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges {
                    edited: vec![],
                    added: vec![],
                    removed: vec![seats.id],
                },
                add_on_changes: AddOnChanges::default(),
            },
            change_date,
        )
        .await
        .expect("remove slot component failed");

    let active = env.get_subscription_components(sub_id).await;
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].name, "Platform Fee");

    // Seats was €10 × 1 slot = €10/mo.
    let mrr_after = env.get_subscription(sub_id).await.mrr_cents;
    assert_eq!(mrr_after, mrr_before - 1000);
}

// =============================================================================
// USAGE TEMPORAL SPLIT ON AMENDMENT (#6)
// =============================================================================

/// Editing a usage component's rate mid-period must bill usage on a temporal
/// split: the pre-change window at the old rate and the post-change window at
/// the new rate (via `historical_lines`), never the whole period at one rate.
///
/// Usage Alpha: Rate €10/mo + "API Calls" on METRIC_BANDWIDTH @ €0.10/unit.
/// Feb 15: edit API Calls €0.10 → €0.20.
/// Mar 1 renewal: Rate €10 + [Feb 1,Feb 15]@€0.10 (50u=€5) + [Feb 15,Mar 1]@€0.20 (200u=€40) = €55.
#[tokio::test]
async fn test_amendment_edit_usage_component_temporal_split() {
    let jan1 = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
    let feb1 = NaiveDate::from_ymd_opt(2025, 2, 1).unwrap();
    let feb15 = NaiveDate::from_ymd_opt(2025, 2, 15).unwrap();
    let mar1 = NaiveDate::from_ymd_opt(2025, 3, 1).unwrap();
    let apr1 = NaiveDate::from_ymd_opt(2025, 4, 1).unwrap();

    let usage_client = build_usage_mock(vec![
        // Cycle 1 arrear: full Jan window.
        (
            MockUsageDataParams {
                metric_id: METRIC_BANDWIDTH,
                period_start: jan1.and_time(NaiveTime::MIN),
                period_end: feb1.and_time(NaiveTime::MIN),
            },
            Decimal::new(1000, 0),
        ),
        // Old rate window [Feb 1, Feb 15].
        (
            MockUsageDataParams {
                metric_id: METRIC_BANDWIDTH,
                period_start: feb1.and_time(NaiveTime::MIN),
                period_end: feb15.and_time(NaiveTime::MIN),
            },
            Decimal::new(50, 0),
        ),
        // New rate window [Feb 15, Mar 1].
        (
            MockUsageDataParams {
                metric_id: METRIC_BANDWIDTH,
                period_start: feb15.and_time(NaiveTime::MIN),
                period_end: mar1.and_time(NaiveTime::MIN),
            },
            Decimal::new(200, 0),
        ),
    ]);
    let env = test_env_with_usage(usage_client).await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_ALPHA_ID)
        .start_date(jan1)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Cycle 1 renewal so cycle-0 usage arrears are billed and we advance into [Feb 1, Mar 1].
    env.process_cycles().await;

    // Feb 15: edit the "API Calls" usage component rate €0.10 → €0.20.
    let api_calls = env
        .get_subscription_components(sub_id)
        .await
        .into_iter()
        .find(|c| c.name == "API Calls")
        .expect("API Calls usage component");

    env.services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges {
                    edited: vec![EditComponent {
                        subscription_component_id: api_calls.id,
                        name: None,
                        price_entry: PriceEntry::New(PriceInput {
                            cadence: BillingPeriodEnum::Monthly,
                            currency: "EUR".to_string(),
                            pricing: Pricing::Usage(UsagePricingModel::PerUnit {
                                rate: Decimal::new(20, 2),
                            }),
                        }),
                    }],
                    added: vec![],
                    removed: vec![],
                },
                add_on_changes: AddOnChanges::default(),
            },
            feb15,
        )
        .await
        .expect("edit usage component failed");

    // Cycle 2 renewal [Mar 1, Apr 1]: Rate €10 + temporal-split usage.
    env.process_cycles().await;

    let invoices = env.get_invoices(sub_id).await;
    let cycle2 = invoices
        .iter()
        .find(|i| {
            i.line_items
                .iter()
                .any(|l| l.start_date == mar1 && l.end_date == apr1)
        })
        .expect("cycle 2 renewal invoice");

    // €10 rate + €5 (old window) + €40 (new window) = €55.
    assert_eq!(
        cycle2.total, 5500,
        "usage must be split: old rate for [Feb 1,Feb 15], new rate for [Feb 15,Mar 1]"
    );

    // Two distinct usage segments for the same metric (the temporal split).
    let usage_lines: Vec<_> = cycle2
        .line_items
        .iter()
        .filter(|l| l.metric_id == Some(METRIC_BANDWIDTH))
        .collect();
    assert_eq!(
        usage_lines.len(),
        2,
        "expected an old-rate and a new-rate usage segment, got {usage_lines:?}"
    );
}

// =============================================================================
// VALIDATION (#7)
// =============================================================================

/// An amendment is rejected while a plan change is already scheduled.
#[rstest]
#[tokio::test]
async fn test_amendment_rejected_when_plan_change_pending(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    env.services()
        .schedule_plan_change(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            PLAN_VERSION_PRO_ID,
            vec![],
        )
        .await
        .expect("schedule_plan_change failed");

    let err = env
        .services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            amendment_add_component(
                PlanChangeMode::Immediate,
                extra_rate_component("Support", 5000),
            ),
            NaiveDate::from_ymd_opt(2024, 1, 16).unwrap(),
        )
        .await;
    assert!(
        err.is_err(),
        "amendment should be rejected while a plan change is pending"
    );
}

/// An add-on with quantity < 1 is rejected.
#[rstest]
#[tokio::test]
async fn test_amendment_addon_quantity_must_be_positive(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let add_on_id = create_rate_addon(&env, "Premium Support", 2000).await;

    let err = env
        .services()
        .preview_amendment(
            sub_id,
            TENANT_ID,
            add_addon_amendment(PlanChangeMode::Immediate, add_on_id, 0),
        )
        .await;
    assert!(err.is_err(), "quantity 0 should be rejected");
}

// =============================================================================
// AUDIT TRAIL
// =============================================================================

/// Applying, scheduling, and cancelling amendments record audit activities
/// with the acting actor.
#[rstest]
#[tokio::test]
async fn test_amendment_records_audit_activities(#[future] test_env: TestEnv) {
    use common_domain::ids::BaseId;
    use diesel_models::entity_activity::EntityActivityRow;
    use diesel_models::enums::ActorTypeEnum;

    let env = test_env.await;
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Immediate apply → subscription.amended.
    env.services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            amendment_add_component(
                PlanChangeMode::Immediate,
                extra_rate_component("Support", 5000),
            ),
            NaiveDate::from_ymd_opt(2024, 1, 16).unwrap(),
        )
        .await
        .expect("apply failed");

    // Schedule → subscription.amendment_scheduled, then cancel → subscription.amendment_cancelled.
    env.services()
        .schedule_amendment(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            amendment_add_component(
                PlanChangeMode::EndOfPeriod,
                extra_rate_component("Extra", 1000),
            ),
        )
        .await
        .expect("schedule failed");
    env.services()
        .cancel_amendment(common_domain::actor::Actor::System, sub_id, TENANT_ID)
        .await
        .expect("cancel failed");

    let mut conn = env.conn().await;
    let activities = EntityActivityRow::list_by_entity(
        &mut conn,
        TENANT_ID,
        "subscription",
        sub_id.as_uuid(),
        None,
        50,
    )
    .await
    .expect("list activities");

    for expected in [
        "subscription.amended",
        "subscription.amendment_scheduled",
        "subscription.amendment_cancelled",
    ] {
        let found = activities.iter().find(|a| a.activity_type == expected);
        assert!(
            found.is_some(),
            "expected audit activity '{expected}', got {:?}",
            activities
                .iter()
                .map(|a| &a.activity_type)
                .collect::<Vec<_>>()
        );
        assert_eq!(found.unwrap().actor_type, ActorTypeEnum::System);
    }
}

// =============================================================================
// CREDIT NOTES (credit side of an immediate amendment)
// =============================================================================

/// Removing an advance-billed component mid-period on an *unpaid* invoice issues a
/// DebtCancellation credit note for the unused portion, reducing what is owed.
#[rstest]
#[tokio::test]
async fn test_amendment_removal_unpaid_issues_debt_cancellation(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let start_date = NaiveDate::from_ymd_opt(2024, 4, 1).unwrap();
    let change_date = NaiveDate::from_ymd_opt(2024, 4, 16).unwrap(); // 15 of 30 days remaining

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Finalize the initial invoice; without auto-charge it stays unpaid.
    env.run_outbox_and_orchestration().await;

    let recurring = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .find(|i| i.invoice_type == InvoiceType::Recurring && i.invoice_date == start_date)
        .expect("recurring invoice for the first period");
    assert_eq!(recurring.status, InvoiceStatusEnum::Finalized);
    assert_eq!(recurring.payment_status, InvoicePaymentStatus::Unpaid);

    let platform = env
        .get_subscription_components(sub_id)
        .await
        .into_iter()
        .find(|c| c.name == "Platform Fee")
        .expect("Platform Fee component");
    let platform_line = recurring
        .line_items
        .iter()
        .find(|l| l.sub_component_id == Some(platform.id))
        .expect("Platform Fee line on the original invoice");
    let expected_credit = platform_line.amount_subtotal / 2; // half the period unused

    env.services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges {
                    edited: vec![],
                    added: vec![],
                    removed: vec![platform.id],
                },
                add_on_changes: AddOnChanges::default(),
            },
            change_date,
        )
        .await
        .expect("apply amendment failed");

    let credit_notes = env
        .store()
        .list_credit_notes_by_invoice_id(TENANT_ID, recurring.id)
        .await
        .expect("list credit notes");

    assert_eq!(credit_notes.len(), 1, "exactly one credit note expected");
    let cn = &credit_notes[0];
    assert_eq!(cn.credit_type, CreditType::DebtCancellation);
    assert_eq!(cn.subtotal, -expected_credit); // credit notes store negative subtotals
    // DebtCancellation reduces invoice debt, not the customer balance.
    assert_eq!(cn.credited_amount_cents, 0);
    // Genuine mid-period proration (factor 0.5) → the credit line is flagged prorated.
    assert!(
        cn.line_items.iter().any(|l| l.is_prorated),
        "a partial-period credit should be flagged prorated"
    );
}

/// Removing a component at the exact period start credits 100% (proration factor 1.0).
/// That is a full credit, not a prorated one, so the credit-note line must NOT be
/// flagged prorated — the marker only appears when proration actually reduces an amount.
#[rstest]
#[tokio::test]
async fn test_amendment_removal_at_period_start_is_not_prorated(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let start_date = NaiveDate::from_ymd_opt(2024, 4, 1).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    env.run_outbox_and_orchestration().await;

    let recurring = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .find(|i| i.invoice_type == InvoiceType::Recurring && i.invoice_date == start_date)
        .expect("recurring invoice for the first period");

    let platform = env
        .get_subscription_components(sub_id)
        .await
        .into_iter()
        .find(|c| c.name == "Platform Fee")
        .expect("Platform Fee component");
    let full = recurring
        .line_items
        .iter()
        .find(|l| l.sub_component_id == Some(platform.id))
        .expect("Platform Fee line on the original invoice")
        .amount_subtotal;

    // Remove at the exact period start → 100% unused → full credit (factor 1.0).
    env.services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges {
                    edited: vec![],
                    added: vec![],
                    removed: vec![platform.id],
                },
                add_on_changes: AddOnChanges::default(),
            },
            start_date,
        )
        .await
        .expect("apply amendment failed");

    let credit_notes = env
        .store()
        .list_credit_notes_by_invoice_id(TENANT_ID, recurring.id)
        .await
        .expect("list credit notes");
    assert_eq!(credit_notes.len(), 1, "exactly one credit note expected");
    let cn = &credit_notes[0];

    assert_eq!(cn.subtotal, -full, "credits the full period amount (100%)");
    let line = cn
        .line_items
        .iter()
        .find(|l| l.name.contains("Platform Fee"))
        .expect("Platform Fee credit line");
    assert!(
        !line.is_prorated,
        "a 100% credit (factor 1.0) must not be flagged prorated"
    );
}

/// Removing an advance-billed component mid-period on a *paid* invoice issues a
/// CreditToBalance credit note for the unused portion, crediting the customer balance.
#[rstest]
#[tokio::test]
async fn test_amendment_removal_paid_issues_credit_to_balance(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    let start_date = NaiveDate::from_ymd_opt(2024, 4, 1).unwrap();
    let change_date = NaiveDate::from_ymd_opt(2024, 4, 16).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .auto_charge()
        .create(env.services())
        .await;

    // Finalize and pay the initial invoice.
    env.run_outbox_and_orchestration().await;

    let recurring = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .find(|i| i.invoice_type == InvoiceType::Recurring && i.invoice_date == start_date)
        .expect("recurring invoice for the first period");
    assert_eq!(recurring.payment_status, InvoicePaymentStatus::Paid);

    let platform = env
        .get_subscription_components(sub_id)
        .await
        .into_iter()
        .find(|c| c.name == "Platform Fee")
        .expect("Platform Fee component");
    let platform_line = recurring
        .line_items
        .iter()
        .find(|l| l.sub_component_id == Some(platform.id))
        .expect("Platform Fee line on the original invoice");
    let expected_credit = platform_line.amount_subtotal / 2;

    let balance_before = env.get_customer(CUST_UBER_ID).await.balance_value_cents;

    env.services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges {
                    edited: vec![],
                    added: vec![],
                    removed: vec![platform.id],
                },
                add_on_changes: AddOnChanges::default(),
            },
            change_date,
        )
        .await
        .expect("apply amendment failed");

    let credit_notes = env
        .store()
        .list_credit_notes_by_invoice_id(TENANT_ID, recurring.id)
        .await
        .expect("list credit notes");

    assert_eq!(credit_notes.len(), 1, "exactly one credit note expected");
    let cn = &credit_notes[0];
    assert_eq!(cn.credit_type, CreditType::CreditToBalance);
    assert_eq!(cn.subtotal, -expected_credit); // credit notes store negative subtotals
    assert!(cn.credited_amount_cents > 0);

    // The credit lands on the customer balance (same currency, so 1:1).
    let balance_after = env.get_customer(CUST_UBER_ID).await.balance_value_cents;
    assert_eq!(balance_after - balance_before, cn.credited_amount_cents);
}

/// Add an add-on immediately, then remove it within the *same* period. The add-on
/// was billed (prorated) on an `Adjustment` invoice — never on the period's
/// recurring invoice — so the credit for the unused portion must land on that
/// adjustment invoice. Regression test: previously the credit search only looked at
/// the recurring invoice, so no credit note was issued and the customer kept paying
/// for an add-on they no longer had.
#[rstest]
#[tokio::test]
async fn test_amendment_add_then_remove_addon_credits_adjustment_invoice(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let add_date = NaiveDate::from_ymd_opt(2024, 1, 16).unwrap(); // 16 of 31 days remaining
    let remove_date = NaiveDate::from_ymd_opt(2024, 1, 20).unwrap(); // 12 of 31 days remaining

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Finalize the initial recurring invoice (unpaid). It bills the Starter plan
    // only — the add-on does not exist yet.
    env.run_outbox_and_orchestration().await;

    let add_on_id = create_rate_addon(&env, "Premium Support", 2000).await;

    // Add the add-on at Jan 16 → a prorated charge on a finalized adjustment invoice.
    let add = env
        .services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            add_addon_amendment(PlanChangeMode::Immediate, add_on_id, 1),
            add_date,
        )
        .await
        .expect("add add-on failed");
    let adjustment_id = add
        .adjustment_invoice_id
        .expect("adding an add-on mid-period produces an adjustment invoice");

    let adjustment = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .find(|i| i.id == adjustment_id)
        .expect("adjustment invoice");
    assert_eq!(adjustment.invoice_type, InvoiceType::Adjustment);
    // €20/mo * 16/31 days = €10.32 charged for the add-on, on its own line.
    let billed = adjustment
        .line_items
        .iter()
        .find(|l| l.sub_add_on_id.is_some())
        .expect("add-on line on the adjustment invoice");
    assert_eq!(billed.amount_subtotal, 1032);

    // Remove the add-on at Jan 20 → credit the unused 12/31 of what was billed.
    let removal = env
        .services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges::default(),
                add_on_changes: AddOnChanges {
                    added: vec![],
                    edited: vec![],
                    removed: vec![active_addon_id(&env, sub_id).await],
                },
            },
            remove_date,
        )
        .await
        .expect("remove add-on failed");

    // Exactly one credit note, and it targets the adjustment invoice that billed
    // the add-on — not the recurring invoice.
    assert_eq!(
        removal.credit_note_ids.len(),
        1,
        "removing the just-added add-on must issue a credit note"
    );

    let credit_notes = env
        .store()
        .list_credit_notes_by_invoice_id(TENANT_ID, adjustment_id)
        .await
        .expect("list credit notes");
    assert_eq!(
        credit_notes.len(),
        1,
        "the credit note is issued against the adjustment invoice"
    );
    let cn = &credit_notes[0];
    // Adjustment invoice is unpaid → debt cancellation reduces what is owed.
    assert_eq!(cn.credit_type, CreditType::DebtCancellation);
    // Unused 12 of 31 days of the €20/mo add-on: 2000 * 12/31 = 774 (rounded).
    assert_eq!(cn.subtotal, -774);

    // The recurring invoice never billed the add-on, so it is not credited.
    let recurring = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .find(|i| i.invoice_type == InvoiceType::Recurring && i.invoice_date == start_date)
        .expect("recurring invoice");
    let recurring_cns = env
        .store()
        .list_credit_notes_by_invoice_id(TENANT_ID, recurring.id)
        .await
        .expect("list recurring credit notes");
    assert!(
        recurring_cns.is_empty(),
        "recurring invoice must not be credited for an add-on it never billed"
    );
}

/// Fetch the single active subscription_add_on id (test helper).
async fn active_addon_id(
    env: &TestEnv,
    sub_id: common_domain::ids::SubscriptionId,
) -> common_domain::ids::SubscriptionAddOnId {
    let mut conn = env.conn().await;
    let active =
        SubscriptionAddOnRow::list_by_subscription_id_active(&mut conn, &TENANT_ID, &sub_id)
            .await
            .expect("list active add-ons");
    assert_eq!(active.len(), 1, "expected exactly one active add-on");
    active[0].id
}
