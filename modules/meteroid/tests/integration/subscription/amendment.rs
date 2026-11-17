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
use common_domain::ids::{SubscriptionId, SubscriptionPriceComponentId};
use diesel_models::enums::SubscriptionStatusEnum;
use diesel_models::subscription_add_ons::SubscriptionAddOnRow;
use meteroid_store::clients::usage::{
    GroupedUsageData, MockUsageClient, MockUsageDataParams, MutableMockUsageClient, UsageData,
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
use meteroid_store::domain::{
    BillingPeriodEnum, SlotUpgradeBillingMode, UsagePeriod, UsagePricingModel,
};
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::repositories::add_ons::AddOnInterface;
use meteroid_store::repositories::credit_notes::CreditNoteInterface;
use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;
use meteroid_store::repositories::subscriptions::slots::SubscriptionSlotsInterfaceAuto;
use rust_decimal::Decimal;

/// Create a catalog add-on (new product) with a monthly EUR Rate price.
pub(super) async fn create_rate_addon(
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
                credit_unit: None,
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
            credit_unit: None,
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
            credit_unit: None,
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
            credit_unit: None,
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

/// On Jan 10 a €50/mo component is added at once from Jan 16, a day still to come: the
/// subscription's details list the components in force today, Platform Fee and Seats, until Jan 16
/// lists Support with them. The change bills 16/31 of 5000 = 2581, as on its day.
#[rstest]
#[tokio::test]
async fn a_change_dated_later_shows_in_the_details_from_its_date(#[future] test_env: TestEnv) {
    use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;

    let env = test_env.await;
    let day = |d: u32| NaiveDate::from_ymd_opt(2024, 1, d).unwrap();
    env.set_now(day(1).and_hms_opt(9, 0, 0).unwrap().and_utc());
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(day(1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.set_now(day(10).and_hms_opt(9, 0, 0).unwrap().and_utc());
    env.services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            amendment_add_component(
                PlanChangeMode::Immediate,
                extra_rate_component("Support", 5000),
            ),
            day(16),
        )
        .await
        .expect("a change from Jan 16");
    let listed = async || {
        let mut names: Vec<String> = env
            .store()
            .get_subscription_details(TENANT_ID, sub_id)
            .await
            .expect("details")
            .price_components
            .into_iter()
            .map(|component| component.name)
            .collect();
        names.sort();
        names
    };
    assert_eq!(listed().await, ["Platform Fee", "Seats"]);

    env.set_now(day(16).and_hms_opt(9, 0, 0).unwrap().and_utc());
    assert_eq!(listed().await, ["Platform Fee", "Seats", "Support"]);
    let invoices = env.get_invoices(sub_id).await;
    invoices
        .assert()
        .has_count(2)
        .invoice_at(1)
        .with_context("the change's adjustment")
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
                credit_unit: None,
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
        SubscriptionAddOnRow::list_by_subscription_id_active(&mut conn, &TENANT_ID, &sub_id, None)
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
                credit_unit: None,
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

/// Regression: a fixed-rate in-arrears fee must prorate to the elapsed fraction when the
/// subscription is cancelled mid-period (15/31 of €31 = €15.00), not bill the full period.
#[rstest]
#[tokio::test]
async fn test_mid_period_cancel_prorates_fixed_arrears(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let jan1 = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let jan16 = NaiveDate::from_ymd_opt(2024, 1, 16).unwrap();

    // The cancellation is requested on the day it ends from.
    env.set_now(jan16.and_hms_opt(9, 0, 0).unwrap().and_utc());

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(jan1)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // A €31/mo fixed arrears component active for the whole first period.
    env.services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            amendment_add_component(
                PlanChangeMode::Immediate,
                extra_arrears_component("Overage Fee", 3100),
            ),
            jan1,
        )
        .await
        .expect("add arrears component failed");

    // Cancel effective Jan 16 — mid first period.
    env.services()
        .cancel_subscription(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            Some("mid-period cancel".to_string()),
            CancellationEffectiveAt::Date(jan16),
        )
        .await
        .expect("cancel_subscription failed");

    let sub = env.get_subscription(sub_id).await;
    assert_eq!(sub.status, SubscriptionStatusEnum::Cancelled);

    let invoices = env.get_invoices(sub_id).await;
    let terminal = invoices
        .iter()
        .find(|i| i.billing_period_start == jan16 && i.invoice_type == InvoiceType::Recurring)
        .expect("a terminal recurring invoice of the cancellation's day");

    let arrears_line = terminal
        .line_items
        .iter()
        .find(|l| l.name.contains("Overage Fee"))
        .expect("the terminal invoice must bill the fixed arrears component");
    assert_eq!(
        arrears_line.amount_subtotal, 1500,
        "fixed arrears must prorate to 15/31 of €31 on a mid-period cancel, not bill in full (3100)"
    );
    assert!(
        arrears_line.is_prorated,
        "the mid-period cancellation arrears charge is prorated"
    );
    assert_eq!(arrears_line.start_date, jan1);
    assert_eq!(arrears_line.end_date, jan16);
    env.assert_corpus("fix383_mid_period_cancel_fixed_arrears")
        .await;
}

/// Regression: when a fixed arrears component is BOTH added mid-period and the subscription is
/// cancelled mid-period, the temporal-bound proration and the cancellation proration compose
/// (multiply) rather than clobber. Component effective Jan 10, cancel Jan 20 of a Jan 1–Feb 1
/// period → window [Jan 10, Jan 20] = 10 of 31 days: 19/31 × 10/19 = 10/31 of €31 = €10.00.
#[rstest]
#[tokio::test]
async fn test_add_then_cancel_mid_period_composes_arrears_proration(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let jan1 = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let jan10 = NaiveDate::from_ymd_opt(2024, 1, 10).unwrap();
    let jan20 = NaiveDate::from_ymd_opt(2024, 1, 20).unwrap();

    // The cancellation is requested on the day it ends from.
    env.set_now(jan20.and_hms_opt(9, 0, 0).unwrap().and_utc());

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(jan1)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Add the €31/mo fixed arrears component mid-period (Jan 10).
    env.services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            amendment_add_component(
                PlanChangeMode::Immediate,
                extra_arrears_component("Overage Fee", 3100),
            ),
            jan10,
        )
        .await
        .expect("add arrears component failed");

    // Cancel mid-period (Jan 20), before the component's first full period ever renews.
    env.services()
        .cancel_subscription(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            Some("mid-period cancel".to_string()),
            CancellationEffectiveAt::Date(jan20),
        )
        .await
        .expect("cancel_subscription failed");

    let invoices = env.get_invoices(sub_id).await;
    let terminal = invoices
        .iter()
        .find(|i| i.billing_period_start == jan20 && i.invoice_type == InvoiceType::Recurring)
        .expect("a terminal recurring invoice of the cancellation's day");

    let arrears_line = terminal
        .line_items
        .iter()
        .find(|l| l.name.contains("Overage Fee"))
        .expect("the terminal invoice must bill the fixed arrears component");
    assert_eq!(
        arrears_line.amount_subtotal, 1000,
        "composed proration must be 10/31 of €31 = €10.00 (19/31 × 10/19), not 15/31 or full"
    );
    assert!(arrears_line.is_prorated);
    assert_eq!(arrears_line.start_date, jan10);
    assert_eq!(arrears_line.end_date, jan20);
    env.assert_corpus("fix383_add_then_cancel_composed_arrears")
        .await;
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
                credit_unit: None,
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

/// Late-import variant of the above: the closed usage add-on's final segment must still be billed
/// when its usage is imported *after* the empty renewal draft exists. With the add-on removed the
/// subscription is advance-only, so `can_accrue_arrears()` over its currently-active components is
/// false — the refresh would short-circuit on the draft's advance-only lines and drop the late
/// usage, unless it also consults the closed add-on (`can_accrue_arrears_in_period`).
#[tokio::test]
async fn test_remove_arrears_addon_late_usage_still_billed() {
    let jan1 = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
    let jan15 = NaiveDate::from_ymd_opt(2025, 1, 15).unwrap();

    // The usage client starts empty: the segment's usage lands only during the grace period,
    // after the renewal draft has already been created without a usage line.
    let usage = MutableMockUsageClient::new();
    let env = test_env_with_usage(Arc::new(usage.clone())).await;
    // Grace period so the renewal draft's finalization runs in a later step, after the late import.
    env.set_consolidate_and_grace(false, 720).await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(jan1)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

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
                credit_unit: None,
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

    // Add at Jan 1, remove at Jan 15 → the subscription is advance-only from Jan 15 on, with the
    // usage add-on left as a closed segment [Jan 1, Jan 15].
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

    // Renewal at Feb 1 creates the draft (Starter advance lines, no usage line — usage not imported
    // yet); its finalization is deferred by the grace period.
    env.process_cycle_transitions().await;

    // The segment's usage is imported only now, after the draft exists.
    usage.set_usage(
        MockUsageDataParams {
            metric_id: METRIC_BANDWIDTH,
            period_start: jan1.and_time(NaiveTime::MIN),
            period_end: jan15.and_time(NaiveTime::MIN),
        },
        UsageData {
            data: vec![GroupedUsageData {
                value: Decimal::new(100, 0),
                dimensions: HashMap::new(),
            }],
            period: UsagePeriod {
                start: jan1.and_time(NaiveTime::MIN),
                end: jan15.and_time(NaiveTime::MIN),
            },
        },
    );

    // Finalize the deferred draft: the refresh must recompute and bill the closed segment.
    env.process_due_events().await;

    let invoices = env.get_invoices(sub_id).await;
    let usage_line = invoices
        .iter()
        .flat_map(|i| &i.line_items)
        .find(|l| l.metric_id == Some(METRIC_BANDWIDTH))
        .expect("late-imported usage for the closed add-on segment must be billed, not dropped");
    assert_eq!(usage_line.amount_total, 1000, "100 units × €0.10 = €10");
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
        SubscriptionAddOnRow::list_by_subscription_id_active(&mut conn, &TENANT_ID, &sub_id, None)
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
                            credit_unit: None,
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

/// Starter with 5 seats (1 default + 4 added), then the Seats price is edited €10 → €12.
fn edit_seats_price_amendment(
    mode: PlanChangeMode,
    seats_id: SubscriptionPriceComponentId,
) -> SubscriptionAmendment {
    SubscriptionAmendment {
        apply_mode: mode,
        component_changes: ComponentChanges {
            edited: vec![EditComponent {
                subscription_component_id: seats_id,
                name: None,
                price_entry: PriceEntry::New(PriceInput {
                    cadence: BillingPeriodEnum::Monthly,
                    currency: "EUR".to_string(),
                    credit_unit: None,
                    pricing: Pricing::Slot {
                        unit_rate: Decimal::new(1200, 2),
                        min_slots: None,
                        max_slots: None,
                    },
                }),
            }],
            added: vec![],
            removed: vec![],
        },
        add_on_changes: AddOnChanges::default(),
    }
}

async fn starter_with_five_seats(env: &TestEnv) -> (SubscriptionId, SubscriptionPriceComponentId) {
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    env.services()
        .update_subscription_slots_for_test(
            TENANT_ID,
            sub_id,
            COMP_STARTER_SEATS_ID,
            4,
            SlotUpgradeBillingMode::Optimistic,
            NaiveDate::from_ymd_opt(2024, 1, 2).map(|d| d.and_time(NaiveTime::MIN)),
        )
        .await
        .expect("add 4 seats");

    let seats = env
        .get_subscription_components(sub_id)
        .await
        .into_iter()
        .find(|c| c.name == "Seats")
        .expect("Seats (slot) component");
    (sub_id, seats.id)
}

async fn active_seats(env: &TestEnv, sub_id: SubscriptionId, at: NaiveDate) -> u32 {
    env.store()
        .get_active_slots_value(
            TENANT_ID,
            sub_id,
            "Seats".to_string(),
            Some(at.and_time(NaiveTime::MIN)),
        )
        .await
        .expect("get active seats")
}

/// Repricing a Slot component must keep the seat count: the override must not seed
/// a fresh slot-ledger checkpoint at min_slots.
///
/// Immediate edit on Jan 16 (16/31 left): credit 5 × €10, charge 5 × €12.
/// Feb 1 renewal: Platform €29 + Seats 5 × €12 = €89.
#[rstest]
#[tokio::test]
async fn test_amendment_edit_slot_component_price_keeps_seats(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let change_date = NaiveDate::from_ymd_opt(2024, 1, 16).unwrap();
    let feb1 = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();

    let (sub_id, seats_id) = starter_with_five_seats(&env).await;
    assert_eq!(active_seats(&env, sub_id, change_date).await, 5);

    let result = env
        .services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            edit_seats_price_amendment(PlanChangeMode::Immediate, seats_id),
            change_date,
        )
        .await
        .expect("edit slot component failed");

    assert_eq!(active_seats(&env, sub_id, change_date).await, 5);

    // Both sides prorate the live 5 seats: charge 6000 × 16/31 = 3097, credit
    // 5000 × 16/31 = 2581, netted into one adjustment line of 516.
    let adjustment = env
        .store()
        .get_invoice_by_id(TENANT_ID, result.adjustment_invoice_id.expect("adjustment"))
        .await
        .expect("adjustment invoice");
    assert_eq!(adjustment.subtotal, 516);

    // MRR: Platform €29 + Seats 5 × €12.
    env.get_subscription(sub_id).await.assert().has_mrr(8900);

    env.process_cycles().await;

    assert_eq!(active_seats(&env, sub_id, feb1).await, 5);
    let invoices = env.get_invoices(sub_id).await;
    invoices
        .assert()
        .invoice_at(invoices.len() - 1)
        .with_context("renewal bills 5 seats at the new price")
        .has_period(feb1, NaiveDate::from_ymd_opt(2024, 3, 1).unwrap())
        .has_total(8900);
}

/// End-of-period variant: the override applied at Feb 1 keeps the 5 seats and the
/// renewal bills Platform €29 + Seats 5 × €12 = €89.
#[rstest]
#[tokio::test]
async fn test_amendment_edit_slot_component_price_keeps_seats_end_of_period(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let feb1 = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();

    let (sub_id, seats_id) = starter_with_five_seats(&env).await;

    env.services()
        .schedule_amendment(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            edit_seats_price_amendment(PlanChangeMode::EndOfPeriod, seats_id),
        )
        .await
        .expect("schedule_amendment failed");

    env.process_cycles().await;

    let seats = env
        .get_subscription_components(sub_id)
        .await
        .into_iter()
        .find(|c| c.name == "Seats")
        .expect("Seats (slot) component");
    assert_ne!(seats.id, seats_id, "override should be applied");
    assert_eq!(active_seats(&env, sub_id, feb1).await, 5);

    let invoices = env.get_invoices(sub_id).await;
    invoices
        .assert()
        .invoice_at(invoices.len() - 1)
        .with_context("renewal bills 5 seats at the new price")
        .has_period(feb1, NaiveDate::from_ymd_opt(2024, 3, 1).unwrap())
        .has_total(8900);
    env.get_subscription(sub_id).await.assert().has_mrr(8900);
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
                            credit_unit: None,
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
        .find(|i| i.invoice_type == InvoiceType::Recurring && i.billing_period_start == start_date)
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
        .find(|i| i.invoice_type == InvoiceType::Recurring && i.billing_period_start == start_date)
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
        .find(|i| i.invoice_type == InvoiceType::Recurring && i.billing_period_start == start_date)
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

    let balance_before = env.invoice_credit_balance_minor(CUST_UBER_ID).await;

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
    let balance_after = env.invoice_credit_balance_minor(CUST_UBER_ID).await;
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
        .find(|i| i.invoice_type == InvoiceType::Recurring && i.billing_period_start == start_date)
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
    env.assert_corpus("fix169_add_on_removed_same_period").await;
}

/// Fetch the single active subscription_add_on id (test helper).
async fn active_addon_id(
    env: &TestEnv,
    sub_id: common_domain::ids::SubscriptionId,
) -> common_domain::ids::SubscriptionAddOnId {
    let mut conn = env.conn().await;
    let active =
        SubscriptionAddOnRow::list_by_subscription_id_active(&mut conn, &TENANT_ID, &sub_id, None)
            .await
            .expect("list active add-ons");
    assert_eq!(active.len(), 1, "expected exactly one active add-on");
    active[0].id
}

/// Removing a component mid-period credits only what the customer was charged for its unused
/// part: the coupon's share stays off the credit, the promotional credit the line took goes back
/// to its grant with the tax it saved, and the preview shows exactly the note that is issued.
#[rstest]
#[tokio::test]
async fn test_amendment_removal_nets_the_coupon_and_returns_promotional_credit(
    #[future] test_env: TestEnv,
) {
    use crate::helpers::credit::{Grant, fund_promotional_grant, grant_remaining};
    let env = test_env.await;
    env.seed_payments().await;
    let coupon = env.create_percentage_coupon("AMEND20", 20).await;
    let promo = fund_promotional_grant(
        env.store(),
        TENANT_ID,
        CUST_UBER_ID,
        None,
        Grant::invoice(1_000),
    )
    .await;
    let today = chrono::Utc::now().naive_utc().date();
    let start_date = today - chrono::Duration::days(10);

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .auto_charge()
        .coupon(coupon)
        .create(env.services())
        .await;
    env.run_outbox_and_orchestration().await;

    let recurring = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .find(|i| i.invoice_type == InvoiceType::Recurring && i.billing_period_start == start_date)
        .expect("recurring invoice for the first period");
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
    let taken = platform_line.promotional_credit.unwrap_or(0);
    assert!(taken > 0, "the promotional credit took part of the line");
    assert!(
        platform_line.taxable_amount < platform_line.amount_subtotal - taken,
        "and the coupon another part"
    );

    let amendment = SubscriptionAmendment {
        apply_mode: PlanChangeMode::Immediate,
        component_changes: ComponentChanges {
            edited: vec![],
            added: vec![],
            removed: vec![platform.id],
        },
        add_on_changes: AddOnChanges::default(),
    };
    let preview = env
        .services()
        .preview_amendment(sub_id, TENANT_ID, amendment.clone())
        .await
        .expect("preview_amendment failed")
        .credit_note
        .expect("a credit-note preview");
    let before = grant_remaining(env.store(), TENANT_ID, CUST_UBER_ID, promo).await;

    env.services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            amendment,
            today,
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
    let line = &cn.line_items[0];
    let returned = -line
        .promotional_credit
        .expect("the note returns promotional credit");
    assert!(returned > 0 && returned <= taken);
    assert!(
        line.taxable_amount > line.amount_subtotal,
        "the credit is net of the coupon and the promotional credit"
    );
    assert_eq!(line.amount_total, line.taxable_amount + line.tax_amount);
    assert_eq!(cn.total, line.amount_total);
    assert_eq!(
        grant_remaining(env.store(), TENANT_ID, CUST_UBER_ID, promo).await - before,
        returned,
        "the promotional share goes back to its grant"
    );

    assert_eq!(preview.subtotal, cn.subtotal, "the preview is the note");
    assert_eq!(preview.tax_amount, cn.tax_amount);
    assert_eq!(preview.total, cn.total);
    assert_eq!(preview.promotional_credits, -returned);
    assert_eq!(
        preview.subtotal - preview.discount - preview.promotional_credits + preview.tax_amount,
        preview.total
    );
}

/// Under a minimum commitment a decrease waits for the period's end: removing a €29 fee
/// immediately is refused, in preview and apply; at the period's end it applies. Increases apply
/// at once and count toward the period's true-up (test_plan_minimum).
#[rstest]
#[tokio::test]
async fn test_immediate_amendment_under_a_minimum_commitment_is_refused(
    #[future] test_env: TestEnv,
) {
    use diesel_async::RunQueryDsl;
    use diesel_models::plan_version_adjustments::PlanVersionAdjustmentRowNew;

    let env = test_env.await;
    diesel::insert_into(diesel_models::schema::plan_version_adjustment::table)
        .values(&PlanVersionAdjustmentRowNew {
            kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
            plan_version_id: PLAN_VERSION_STARTER_ID,
            amount_subunit: 10_000,
            includes_all_components: true,
        })
        .execute(&mut env.conn().await)
        .await
        .expect("seed the minimum");
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let platform = env
        .get_subscription_components(sub_id)
        .await
        .into_iter()
        .find(|c| c.name == "Platform Fee")
        .expect("Platform Fee component");
    let removal = |apply_mode| SubscriptionAmendment {
        apply_mode,
        component_changes: ComponentChanges {
            edited: vec![],
            added: vec![],
            removed: vec![platform.id],
        },
        add_on_changes: AddOnChanges::default(),
    };

    let refused = env
        .services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            removal(PlanChangeMode::Immediate),
            NaiveDate::from_ymd_opt(2024, 1, 16).unwrap(),
        )
        .await
        .expect_err("refused under a minimum commitment");
    assert!(
        format!("{refused:?}").contains("minimum commitment"),
        "{refused:?}"
    );
    let previewed = env
        .services()
        .preview_amendment(sub_id, TENANT_ID, removal(PlanChangeMode::Immediate))
        .await
        .expect_err("the immediate preview is refused too");
    assert!(format!("{previewed:?}").contains("minimum commitment"));

    assert!(
        env.get_subscription_components(sub_id)
            .await
            .iter()
            .any(|c| c.id == platform.id),
        "the fee stays"
    );
    assert!(
        env.store()
            .list_credit_notes_by_customer_id(TENANT_ID, CUST_UBER_ID)
            .await
            .expect("credit notes")
            .is_empty(),
        "nothing is credited"
    );
    assert_eq!(env.get_invoices(sub_id).await.len(), 1);

    env.services()
        .schedule_amendment(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            removal(PlanChangeMode::EndOfPeriod),
        )
        .await
        .expect("an amendment at the period's end is accepted");
}

// =============================================================================
// OVERRIDES THEN AN ENDING, IN ONE PERIOD (R-078, R-080)
// =============================================================================

fn edit_platform_amendment(
    component: SubscriptionPriceComponentId,
    rate_cents: i64,
) -> SubscriptionAmendment {
    SubscriptionAmendment {
        apply_mode: PlanChangeMode::Immediate,
        component_changes: ComponentChanges {
            edited: vec![EditComponent {
                subscription_component_id: component,
                name: None,
                price_entry: PriceEntry::New(PriceInput {
                    cadence: BillingPeriodEnum::Monthly,
                    currency: "EUR".to_string(),
                    pricing: Pricing::Rate {
                        rate: Decimal::new(rate_cents, 2),
                    },
                    credit_unit: None,
                }),
            }],
            added: vec![],
            removed: vec![],
        },
        add_on_changes: AddOnChanges::default(),
    }
}

fn remove_component_amendment(component: SubscriptionPriceComponentId) -> SubscriptionAmendment {
    SubscriptionAmendment {
        apply_mode: PlanChangeMode::Immediate,
        component_changes: ComponentChanges {
            edited: vec![],
            added: vec![],
            removed: vec![component],
        },
        add_on_changes: AddOnChanges::default(),
    }
}

async fn component_named(
    env: &TestEnv,
    sub_id: SubscriptionId,
    name: &str,
) -> SubscriptionPriceComponentId {
    env.get_subscription_components(sub_id)
        .await
        .into_iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("{name} component"))
        .id
}

async fn amend(
    env: &TestEnv,
    sub_id: SubscriptionId,
    amendment: SubscriptionAmendment,
    on: NaiveDate,
) {
    env.services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            amendment,
            on,
        )
        .await
        .expect("amendment applied");
}

/// What the subscription's credit notes took off lines whose name starts with `name`.
async fn credited(env: &TestEnv, sub_id: SubscriptionId, name: &str) -> Vec<i64> {
    let mut credited = Vec::new();
    for invoice in env.get_invoices(sub_id).await {
        for note in env
            .store()
            .list_credit_notes_by_invoice_id(TENANT_ID, invoice.id)
            .await
            .expect("credit notes")
        {
            credited.extend(
                note.line_items
                    .iter()
                    .filter(|line| line.name.starts_with(name))
                    .map(|line| line.amount_subtotal),
            );
        }
    }
    credited.sort();
    credited
}

async fn starter_from_january(env: &TestEnv) -> SubscriptionId {
    subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await
}

fn january(day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 1, day).unwrap()
}

/// 29.00 billed for January, lowered to 14.50 on the 11th (9.83 back: 21 days at 14.50 less
/// 21 days at 29.00), removed on the 21st: 5.15 back, the 11 days left at 14.50. The 10 days
/// at 29.00 before the override stay billed.
#[rstest]
#[tokio::test]
async fn an_override_down_then_a_removal_give_back_the_unused_time_of_each_price(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let sub_id = starter_from_january(&env).await;
    let platform = component_named(&env, sub_id, "Platform Fee").await;
    amend(
        &env,
        sub_id,
        edit_platform_amendment(platform, 1450),
        january(11),
    )
    .await;
    let platform = component_named(&env, sub_id, "Platform Fee").await;
    amend(
        &env,
        sub_id,
        remove_component_amendment(platform),
        january(21),
    )
    .await;

    assert_eq!(
        credited(&env, sub_id, "Platform Fee").await,
        vec![-983, -515]
    );
}

/// 29.00 billed for January, raised to 58.00 on the 11th (19.64 more), removed on the 21st:
/// 20.58 back, the 11 days left at 58.00, from the adjustment line first.
#[rstest]
#[tokio::test]
async fn an_override_up_then_a_removal_give_back_the_unused_time_of_the_new_price(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let sub_id = starter_from_january(&env).await;
    let platform = component_named(&env, sub_id, "Platform Fee").await;
    amend(
        &env,
        sub_id,
        edit_platform_amendment(platform, 5800),
        january(11),
    )
    .await;
    let platform = component_named(&env, sub_id, "Platform Fee").await;
    amend(
        &env,
        sub_id,
        remove_component_amendment(platform),
        january(21),
    )
    .await;

    assert_eq!(
        credited(&env, sub_id, "Platform Fee").await,
        vec![-1964, -94]
    );
}

/// The same override up, then the subscription cancelled on the 21st: the same 20.58 back, each
/// line its own 11 days left.
#[rstest]
#[tokio::test]
async fn an_override_up_then_a_cancellation_give_back_the_unused_time_of_the_new_price(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let sub_id = starter_from_january(&env).await;
    let platform = component_named(&env, sub_id, "Platform Fee").await;
    amend(
        &env,
        sub_id,
        edit_platform_amendment(platform, 5800),
        january(11),
    )
    .await;
    env.services()
        .cancel_subscription(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(january(21)),
        )
        .await
        .expect("cancellation scheduled");
    env.process_due_events().await;
    env.process_due_events().await;

    assert_eq!(
        credited(&env, sub_id, "Platform Fee").await,
        vec![-1029, -1029]
    );
}

/// A one-time fee added on the 11th is billed once; removing it on the 21st gives nothing back.
#[rstest]
#[tokio::test]
async fn removing_a_one_time_fee_gives_nothing_back(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let sub_id = starter_from_january(&env).await;
    amend(
        &env,
        sub_id,
        amendment_add_component(
            PlanChangeMode::Immediate,
            extra_onetime_component("Setup", 20_000),
        ),
        january(11),
    )
    .await;
    let setup = component_named(&env, sub_id, "Setup").await;
    amend(&env, sub_id, remove_component_amendment(setup), january(21)).await;

    assert_eq!(credited(&env, sub_id, "").await, Vec::<i64>::new());
}

/// A one-time fee of 200.00 added on Jan 5 during a free trial to Jan 15 bills on the trial's
/// end, beside the plan's 49.00; a plan change to Pro that same day ends its row on the day it
/// bills and gives none of it back (it gave back all 20000).
/// Rules: R-093
#[rstest]
#[tokio::test]
async fn a_plan_change_on_the_day_a_one_time_fee_bills_gives_nothing_of_it_back(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.set_now(january(1).and_hms_opt(1, 0, 0).unwrap().and_utc());
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .start_date(january(1))
        .on_start()
        .trial_days(14)
        .no_auto_charge()
        .create(env.services())
        .await;
    env.run_until(january(5)).await;
    amend(
        &env,
        sub_id,
        amendment_add_component(
            PlanChangeMode::Immediate,
            extra_onetime_component("Setup", 20_000),
        ),
        january(5),
    )
    .await;
    env.run_until(january(15)).await;
    env.services()
        .apply_plan_change_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            PLAN_VERSION_PRO_ID,
            vec![],
            january(15),
        )
        .await
        .expect("the plan change");
    env.run_until(january(16)).await;

    let billed: Vec<(String, NaiveDate, i64)> = env
        .get_invoices(sub_id)
        .await
        .iter()
        .flat_map(|invoice| &invoice.line_items)
        .filter(|line| line.name.starts_with("Setup"))
        .map(|line| (line.name.clone(), line.start_date, line.amount_subtotal))
        .collect();
    assert_eq!(billed, vec![("Setup".to_owned(), january(15), 20_000)]);
    assert_eq!(credited(&env, sub_id, "Setup").await, Vec::<i64>::new());
}

/// A one-time fee of 99.00 added on Jan 1 during a free trial to Jan 15, the subscription then
/// cancelled at its period's end, the trial's: the fee lived before the ending and bills 9900 on
/// Jan 15, where no window was left to bill it and nothing was billed.
/// Rules: R-093, R-016
#[rstest]
#[tokio::test]
async fn a_one_time_fee_added_in_a_free_trial_cancelled_at_its_end_bills(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.set_now(january(1).and_hms_opt(1, 0, 0).unwrap().and_utc());
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .start_date(january(1))
        .on_start()
        .trial_days(14)
        .no_auto_charge()
        .create(env.services())
        .await;
    amend(
        &env,
        sub_id,
        amendment_add_component(
            PlanChangeMode::Immediate,
            extra_onetime_component("Setup", 9_900),
        ),
        january(1),
    )
    .await;
    env.services()
        .cancel_subscription(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::EndOfBillingPeriod,
        )
        .await
        .expect("the cancellation");
    env.run_until(january(16)).await;

    let billed: Vec<(String, NaiveDate, i64)> = env
        .get_invoices(sub_id)
        .await
        .iter()
        .flat_map(|invoice| &invoice.line_items)
        .map(|line| (line.name.clone(), line.start_date, line.amount_subtotal))
        .collect();
    assert_eq!(billed, vec![("Setup".to_owned(), january(15), 9_900)]);
    assert_eq!(
        env.get_subscription(sub_id).await.status,
        SubscriptionStatusEnum::Cancelled
    );
}

/// A one-time fee of 99.00 added on Jan 1 during a free trial to Jan 15, then a plan change to
/// Pro on Jan 4 ending the trial: the fee bills 9900 on the change's opening invoice, where the
/// change ended its row before billing started and nothing billed it.
/// Rules: R-093, R-011
#[rstest]
#[tokio::test]
async fn a_one_time_fee_added_in_a_free_trial_a_change_ends_bills_on_its_opening_invoice(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.set_now(january(1).and_hms_opt(1, 0, 0).unwrap().and_utc());
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .start_date(january(1))
        .on_start()
        .trial_days(14)
        .no_auto_charge()
        .create(env.services())
        .await;
    amend(
        &env,
        sub_id,
        amendment_add_component(
            PlanChangeMode::Immediate,
            extra_onetime_component("Setup", 9_900),
        ),
        january(1),
    )
    .await;
    env.run_until(january(4)).await;
    env.services()
        .apply_plan_change_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            PLAN_VERSION_PRO_ID,
            vec![],
            january(4),
        )
        .await
        .expect("the plan change");
    env.run_until(january(5)).await;

    let billed: Vec<(String, NaiveDate, i64)> = env
        .get_invoices(sub_id)
        .await
        .iter()
        .flat_map(|invoice| &invoice.line_items)
        .filter(|line| line.name.starts_with("Setup"))
        .map(|line| (line.name.clone(), line.start_date, line.amount_subtotal))
        .collect();
    assert_eq!(billed, vec![("Setup".to_owned(), january(4), 9_900)]);
}

/// A yearly 120.00 plan with a free trial to Jan 15, an end-of-period amendment adding a monthly
/// 30.00 support fee scheduled on Jan 5, in the trial: it takes effect at the trial's end, on an
/// active subscription, which renews on the shortest cadence of its fees, Jan 15 to Feb 15,
/// where it renewed yearly to Jan 15, 2025.
/// Rules: R-084
#[rstest]
#[tokio::test]
async fn an_amendment_scheduled_in_a_free_trial_realigns_the_period_at_its_end(
    #[future] test_env: TestEnv,
) {
    use common_domain::ids::{BaseId, PlanId, PlanVersionId, PriceComponentId, PriceId};
    use diesel_models::enums::BillingPeriodEnum as DieselBillingPeriodEnum;
    let env = test_env.await;
    let (plan_id, version_id) = (PlanId::new(), PlanVersionId::new());
    crate::data::plans::PlanSeed::new(plan_id, "Yearly trial", version_id)
        .trial(14, plan_id, true)
        .components(vec![crate::data::plans::SeedComp::rate(
            PriceComponentId::new(),
            "Yearly Rate",
            PRODUCT_PLATFORM_FEE_ID,
            PriceId::new(),
            DieselBillingPeriodEnum::Annual,
            Decimal::new(12_000, 2),
        )])
        .seed(&mut env.conn().await)
        .await
        .expect("the plan");
    env.set_now(january(1).and_hms_opt(1, 0, 0).unwrap().and_utc());
    let sub_id = subscription()
        .plan_version(version_id)
        .start_date(january(1))
        .on_start()
        .trial_days(14)
        .no_auto_charge()
        .create(env.services())
        .await;
    env.run_until(january(5)).await;
    env.services()
        .schedule_amendment(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            amendment_add_component(
                PlanChangeMode::EndOfPeriod,
                extra_rate_component("Support", 3_000),
            ),
        )
        .await
        .expect("the amendment scheduled");
    env.run_until(january(16)).await;

    let sub = env.get_subscription(sub_id).await;
    assert_eq!(
        (sub.current_period_start, sub.current_period_end),
        (
            january(15),
            Some(NaiveDate::from_ymd_opt(2024, 2, 15).unwrap())
        )
    );
    let billed: Vec<(String, NaiveDate, NaiveDate, i64)> = env
        .get_invoices(sub_id)
        .await
        .iter()
        .flat_map(|invoice| &invoice.line_items)
        .map(|line| {
            (
                line.name.clone(),
                line.start_date,
                line.end_date,
                line.amount_subtotal,
            )
        })
        .collect();
    assert!(
        billed.contains(&(
            "Support".to_owned(),
            january(15),
            NaiveDate::from_ymd_opt(2024, 2, 15).unwrap(),
            3_000
        )),
        "{billed:?}"
    );
}

/// A subscription starting January 10 that renews on the 1st bills its first period, January 10
/// to February 1, as 22 of 31 days: a fixed fee billed in arrears too (R-084), as its advance fees
/// are. Before the timeline it billed that fee's whole month.
#[rstest]
#[tokio::test]
async fn a_first_partial_period_prorates_a_fixed_arrears_fee(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(january(10))
        .billing_day_anchor(1)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    amend(
        &env,
        sub_id,
        amendment_add_component(
            PlanChangeMode::Immediate,
            extra_arrears_component("Overage Fee", 3100),
        ),
        january(10),
    )
    .await;
    env.process_cycles().await;

    let arrears = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .flat_map(|invoice| invoice.line_items)
        .find(|line| line.name.contains("Overage Fee"))
        .expect("the arrears fee is billed at the first renewal");
    assert_eq!(
        (
            arrears.start_date,
            arrears.end_date,
            arrears.amount_subtotal
        ),
        (
            january(10),
            NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
            2200
        )
    );
}

/// Starter monthly from Jan 1, 2024 with a 366.00 yearly add-on bought that day, its line billed
/// by the previous release before the watermark on Mar 1. Raised to two on Mar 15: the year's line
/// settles what was due before the watermark, and the raise, 36600 × 292/366 = 29200, is billed.
/// Rules: R-094
#[rstest]
#[tokio::test]
async fn an_increase_after_the_watermark_on_a_charge_billed_before_it_is_billed(
    #[future] test_env: TestEnv,
) {
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_async::RunQueryDsl;
    use diesel_models::schema::subscription::dsl as s;

    let env = test_env.await;
    let day = |m, d| NaiveDate::from_ymd_opt(2024, m, d).unwrap();
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let yearly = env
        .store()
        .create_add_on_from_ref(
            "Yearly Support".to_string(),
            ProductRef::New {
                name: "Yearly Support Product".to_string(),
                fee_type: FeeTypeEnum::Rate,
                fee_structure: FeeStructure::Rate {},
            },
            PriceEntry::New(PriceInput {
                cadence: BillingPeriodEnum::Annual,
                currency: "EUR".to_string(),
                pricing: Pricing::Rate {
                    rate: Decimal::new(36_600, 2),
                },
                credit_unit: None,
            }),
            None,
            true,
            Some(10),
            TENANT_ID,
            PRODUCT_FAMILY_ID,
            vec![],
        )
        .await
        .expect("a yearly add-on")
        .id;
    env.services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            add_addon_amendment(PlanChangeMode::Immediate, yearly, 1),
            day(1, 1),
        )
        .await
        .expect("the yearly add-on");
    env.process_cycles().await;
    env.process_cycles().await;
    diesel::update(s::subscription.filter(s::id.eq(sub_id)))
        .set(s::reconcile_from.eq(Some(day(3, 1))))
        .execute(&mut env.conn().await)
        .await
        .expect("the watermark");
    let add_on = {
        let mut conn = env.conn().await;
        SubscriptionAddOnRow::list_by_subscription_id_active(&mut conn, &TENANT_ID, &sub_id, None)
            .await
            .expect("active add-ons")[0]
            .id
    };

    let raised = env
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
                    edited: vec![EditSubscriptionAddOn {
                        subscription_add_on_id: add_on,
                        quantity: Some(2),
                        customization: None,
                    }],
                    removed: vec![],
                },
            },
            day(3, 15),
        )
        .await
        .expect("the raise");
    let adjustment = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .find(|invoice| Some(invoice.id) == raised.adjustment_invoice_id)
        .expect("an adjustment invoice");
    assert_eq!(
        adjustment
            .line_items
            .iter()
            .map(|line| (line.start_date, line.end_date, line.amount_subtotal))
            .collect::<Vec<_>>(),
        vec![(
            day(3, 15),
            NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            29_200
        )]
    );
    assert!(raised.credit_note_ids.is_empty());
}

/// Starter monthly from Jan 1, 2023 with a 365.00 yearly add-on bought that day, the watermark on
/// Feb 1. Lowered on Feb 1 to 255.50 (10020 back from Jan 1's line) and raised at 01:00 to 293.82:
/// the note dated from the watermark counts there, so the raise bills 3832 × 334/365 = 3507 (R-094).
#[rstest]
#[tokio::test]
async fn a_raise_after_a_lowering_past_the_watermark_bills_what_the_note_took_back(
    #[future] test_env: TestEnv,
) {
    use chrono::{TimeZone, Utc};
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_async::RunQueryDsl;
    use diesel_models::schema::subscription::dsl as s;

    let env = test_env.await;
    let day = |m, d| NaiveDate::from_ymd_opt(2023, m, d).unwrap();
    let year_end = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    env.set_now(Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap());
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let annual = |cents: i64| {
        PriceEntry::New(PriceInput {
            cadence: BillingPeriodEnum::Annual,
            currency: "EUR".to_string(),
            pricing: Pricing::Rate {
                rate: Decimal::new(cents, 2),
            },
            credit_unit: None,
        })
    };
    let yearly = env
        .store()
        .create_add_on_from_ref(
            "Yearly Support".to_string(),
            ProductRef::New {
                name: "Yearly Support Product".to_string(),
                fee_type: FeeTypeEnum::Rate,
                fee_structure: FeeStructure::Rate {},
            },
            annual(36_500),
            None,
            true,
            Some(10),
            TENANT_ID,
            PRODUCT_FAMILY_ID,
            vec![],
        )
        .await
        .expect("a yearly add-on")
        .id;
    env.services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            add_addon_amendment(PlanChangeMode::Immediate, yearly, 1),
            day(1, 1),
        )
        .await
        .expect("the yearly add-on");
    env.set_now(Utc.with_ymd_and_hms(2023, 2, 1, 0, 0, 0).unwrap());
    env.process_cycles().await;
    diesel::update(s::subscription.filter(s::id.eq(sub_id)))
        .set(s::reconcile_from.eq(Some(day(2, 1))))
        .execute(&mut env.conn().await)
        .await
        .expect("the watermark");
    let reprice = |cents: i64, hour: u32| {
        let env = &env;
        async move {
            env.set_now(Utc.with_ymd_and_hms(2023, 2, 1, hour, 0, 0).unwrap());
            let add_on = {
                let mut conn = env.conn().await;
                SubscriptionAddOnRow::list_by_subscription_id_active(
                    &mut conn, &TENANT_ID, &sub_id, None,
                )
                .await
                .expect("active add-ons")[0]
                    .id
            };
            env.services()
                .apply_amendment_immediate(
                    common_domain::actor::Actor::System,
                    sub_id,
                    TENANT_ID,
                    SubscriptionAmendment {
                        apply_mode: PlanChangeMode::Immediate,
                        component_changes: ComponentChanges::default(),
                        add_on_changes: AddOnChanges {
                            added: vec![],
                            edited: vec![EditSubscriptionAddOn {
                                subscription_add_on_id: add_on,
                                quantity: None,
                                customization: Some(
                                    SubscriptionAddOnCustomization::PriceOverride {
                                        name: None,
                                        price_entry: annual(cents),
                                    },
                                ),
                            }],
                            removed: vec![],
                        },
                    },
                )
                .await
                .expect("the reprice")
        }
    };

    let lowered = reprice(25_550, 0).await;
    let notes: Vec<i64> = {
        let mut totals = Vec::new();
        for id in &lowered.credit_note_ids {
            totals.push(
                env.store()
                    .get_credit_note_by_id(TENANT_ID, *id)
                    .await
                    .expect("the note")
                    .subtotal,
            );
        }
        totals
    };
    assert_eq!(notes, vec![-10_020]);
    let raised = reprice(29_382, 1).await;
    let adjustment = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .find(|invoice| Some(invoice.id) == raised.adjustment_invoice_id)
        .expect("an adjustment invoice");
    assert_eq!(
        adjustment
            .line_items
            .iter()
            .map(|line| (line.start_date, line.end_date, line.amount_subtotal))
            .collect::<Vec<_>>(),
        vec![(day(2, 1), year_end, 3_507)]
    );
    assert!(raised.credit_note_ids.is_empty());
}

/// Starter from Jan 1, 2024, its first invoice voided at 17:00, then a one-time 99.00 added at
/// 18:00 the same day: the void forgave January, never the fee added after it. Jan 1 bills 9900.
/// Rules: R-096, R-093
#[rstest]
#[tokio::test]
async fn a_one_time_fee_added_after_a_void_on_its_day_is_billed(#[future] test_env: TestEnv) {
    use chrono::{TimeZone, Utc};

    let env = test_env.await;
    let at = |h| Utc.with_ymd_and_hms(2024, 1, 1, h, 0, 0).unwrap();
    env.set_now(at(12));
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let first = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .find(|invoice| invoice.status == InvoiceStatusEnum::Finalized)
        .expect("the first invoice");
    env.store()
        .void_invoice_at(
            common_domain::actor::Actor::System,
            first.id,
            TENANT_ID,
            at(17),
        )
        .await
        .expect("the void");
    env.set_now(at(18));
    env.services()
        .apply_amendment_immediate(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            amendment_add_component(
                PlanChangeMode::Immediate,
                extra_onetime_component("Setup", 9900),
            ),
        )
        .await
        .expect("a one-time 99.00");

    assert_eq!(
        super::derived_state::documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
            "Recurring 2024-01-01: Seats 2024-01-01..2024-02-01 1000",
            "note: Platform Fee 2024-01-01..2024-02-01 -2900",
            "note: Seats 2024-01-01..2024-02-01 -1000",
            "Adjustment 2024-01-01: Setup (one-time) 2024-01-01..2024-01-01 9900",
        ]
    );
}

/// Starter from Jan 1, 2024 with a 366.00 yearly support fee in arrears, cancelled at 15:00 on
/// Feb 1 from that day, after Feb 1's renewal was written: the year's 31 days the ending cuts,
/// 36600 × 31/366 = 3100, go on a final invoice issued that day, and the renewal keeps its lines.
/// Rules: R-016
#[rstest]
#[tokio::test]
async fn a_cancellation_decided_on_its_renewal_day_bills_what_it_cuts_on_a_final_invoice(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.set_now(january(1).and_hms_opt(1, 0, 0).unwrap().and_utc());
    let sub_id = starter_from_january(&env).await;
    let mut support = extra_arrears_component("Annual Support", 36_600);
    if let PriceEntry::New(price) = &mut support.price_entry {
        price.cadence = BillingPeriodEnum::Annual;
    }
    amend(
        &env,
        sub_id,
        amendment_add_component(PlanChangeMode::Immediate, support),
        january(1),
    )
    .await;
    let feb1 = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    env.run_until(feb1).await;
    let documents = |invoices: Vec<meteroid_store::domain::Invoice>| {
        invoices
            .into_iter()
            .map(|invoice| {
                let lines: Vec<(String, NaiveDate, NaiveDate, i64)> = invoice
                    .line_items
                    .iter()
                    .map(|line| {
                        (
                            line.name.clone(),
                            line.start_date,
                            line.end_date,
                            line.amount_subtotal,
                        )
                    })
                    .collect();
                (
                    invoice.id,
                    invoice.invoice_type,
                    invoice.invoice_date,
                    lines,
                    invoice.total,
                )
            })
            .collect::<Vec<_>>()
    };
    let before = documents(env.get_invoices(sub_id).await);
    assert!(
        before
            .iter()
            .any(|doc| doc.1 == InvoiceType::Recurring && doc.2 == feb1),
        "Feb 1's renewal written in the morning: {before:?}"
    );

    env.set_now(feb1.and_hms_opt(15, 0, 0).unwrap().and_utc());
    env.services()
        .cancel_subscription(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(feb1),
        )
        .await
        .expect("the cancellation");
    env.process_cycles().await;

    let after = documents(env.get_invoices(sub_id).await);
    let (kept, written): (Vec<_>, Vec<_>) = after
        .into_iter()
        .partition(|doc| before.iter().any(|earlier| earlier.0 == doc.0));
    assert_eq!(
        kept, before,
        "the documents written before keep their lines"
    );
    assert_eq!(
        written
            .iter()
            .map(|doc| (doc.1.clone(), doc.2, doc.3.clone(), doc.4))
            .collect::<Vec<_>>(),
        vec![(
            InvoiceType::Final,
            feb1,
            vec![("Annual Support".to_owned(), january(1), feb1, 3_100)],
            3_100
        )]
    );
    assert!(
        !before
            .iter()
            .flat_map(|doc| &doc.3)
            .any(|line| line.0.starts_with("Annual Support")),
        "no document billed the fee before: {before:?}"
    );
    assert_eq!(
        credited(&env, sub_id, "Annual Support").await,
        Vec::<i64>::new()
    );
}

/// The yearly 120.00 plan with a free trial to Jan 15, previewed at 10:00 on Jan 15 before the
/// trial's end is stepped, with an immediate amendment adding a monthly 30.00 support fee: the
/// preview renews monthly, Jan 15 to Feb 15, as applying it does, where it read the stored
/// trial and kept the year.
/// Rules: R-084
#[rstest]
#[tokio::test]
async fn an_immediate_amendment_previewed_at_a_free_trial_s_end_realigns_the_period(
    #[future] test_env: TestEnv,
) {
    use common_domain::ids::{BaseId, PlanId, PlanVersionId, PriceComponentId, PriceId};
    use diesel_models::enums::BillingPeriodEnum as DieselBillingPeriodEnum;
    let env = test_env.await;
    let (plan_id, version_id) = (PlanId::new(), PlanVersionId::new());
    crate::data::plans::PlanSeed::new(plan_id, "Yearly trial", version_id)
        .trial(14, plan_id, true)
        .components(vec![crate::data::plans::SeedComp::rate(
            PriceComponentId::new(),
            "Yearly Rate",
            PRODUCT_PLATFORM_FEE_ID,
            PriceId::new(),
            DieselBillingPeriodEnum::Annual,
            Decimal::new(12_000, 2),
        )])
        .seed(&mut env.conn().await)
        .await
        .expect("the plan");
    env.set_now(january(1).and_hms_opt(1, 0, 0).unwrap().and_utc());
    let sub_id = subscription()
        .plan_version(version_id)
        .start_date(january(1))
        .on_start()
        .trial_days(14)
        .no_auto_charge()
        .create(env.services())
        .await;
    env.run_until(january(14)).await;
    env.set_now(january(15).and_hms_opt(10, 0, 0).unwrap().and_utc());
    assert_eq!(
        env.get_subscription(sub_id).await.status,
        SubscriptionStatusEnum::TrialActive,
        "the trial's end is not stepped yet"
    );
    let support = || {
        amendment_add_component(
            PlanChangeMode::Immediate,
            extra_rate_component("Support", 3_000),
        )
    };
    let previewed = env
        .services()
        .preview_amendment(sub_id, TENANT_ID, support())
        .await
        .expect("the preview")
        .subscription_after;
    let feb15 = NaiveDate::from_ymd_opt(2024, 2, 15).unwrap();
    assert_eq!(
        (
            previewed.period,
            previewed.current_period_start,
            previewed.current_period_end
        ),
        (BillingPeriodEnum::Monthly, january(15), Some(feb15))
    );

    amend(&env, sub_id, support(), january(15)).await;
    env.process_cycles().await;
    let sub = env.get_subscription(sub_id).await;
    assert_eq!(
        (sub.current_period_start, sub.current_period_end),
        (january(15), Some(feb15))
    );
}
