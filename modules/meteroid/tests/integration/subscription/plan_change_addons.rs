//! Add-ons across a plan change (Starter → Pro). An add-on the target plan version offers carries
//! over untouched; one it does not offer ends on the change date, like an amendment removing it:
//! its usage up to then is still billed and the unused part of its advance fee is credited.

use chrono::{NaiveDate, NaiveTime};
use common_domain::ids::{AddOnId, PlanVersionId, SubscriptionId};
use diesel_models::subscription_add_ons::SubscriptionAddOnRow;
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::sync::Arc;

use super::amendment::create_rate_addon;
use crate::data::ids::*;
use crate::harness::{TestEnv, subscription, test_env_with_usage};
use meteroid_store::clients::usage::{
    GroupedUsageData, MockUsageClient, MockUsageDataParams, UsageData,
};
use meteroid_store::domain::enums::{FeeTypeEnum, InvoiceType};
use meteroid_store::domain::plan_version_add_ons::PlanVersionAddOnNew;
use meteroid_store::domain::price_components::{PriceEntry, PriceInput, ProductRef};
use meteroid_store::domain::prices::{FeeStructure, Pricing, UsageModel};
use meteroid_store::domain::subscription_add_ons::{
    CreateSubscriptionAddOn, SubscriptionAddOnCustomization,
};
use meteroid_store::domain::subscription_amendment::{
    AddOnChanges, ComponentChanges, SubscriptionAmendment,
};
use meteroid_store::domain::subscription_changes::PlanChangeMode;
use meteroid_store::domain::{BillingPeriodEnum, Invoice, UsagePeriod, UsagePricingModel};
use meteroid_store::repositories::add_ons::AddOnInterface;
use meteroid_store::repositories::credit_notes::CreditNoteInterface;
use meteroid_store::repositories::plan_version_add_ons::PlanVersionAddOnInterface;

#[derive(Clone, Copy, PartialEq)]
enum Target {
    Offers,
    DoesNotOffer,
}

fn day(month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2025, month, day).unwrap()
}

fn bandwidth(from: NaiveDate, to: NaiveDate, units: i64) -> (MockUsageDataParams, Decimal) {
    (
        MockUsageDataParams {
            metric_id: METRIC_BANDWIDTH,
            period_start: from.and_time(NaiveTime::MIN),
            period_end: to.and_time(NaiveTime::MIN),
        },
        Decimal::new(units, 0),
    )
}

fn usage_mock(entries: Vec<(MockUsageDataParams, Decimal)>) -> Arc<MockUsageClient> {
    let data = entries
        .into_iter()
        .map(|(params, value)| {
            let period = UsagePeriod {
                start: params.period_start,
                end: params.period_end,
            };
            let usage = UsageData {
                data: vec![GroupedUsageData {
                    value,
                    dimensions: HashMap::new(),
                }],
                period,
            };
            (params, usage)
        })
        .collect();
    Arc::new(MockUsageClient { data })
}

/// €0.10 per bandwidth unit, billed in arrears.
async fn usage_add_on(env: &TestEnv) -> AddOnId {
    env.store()
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
        .expect("create usage add-on")
        .id
}

async fn offer_on(env: &TestEnv, target: Target, plan_version_id: PlanVersionId, add_on: AddOnId) {
    if target == Target::DoesNotOffer {
        return;
    }
    env.store()
        .attach_add_on_to_plan_version(PlanVersionAddOnNew {
            plan_version_id,
            add_on_id: add_on,
            price_id: None,
            self_serviceable: None,
            max_instances_per_subscription: None,
            tenant_id: TENANT_ID,
        })
        .await
        .expect("attach add-on to plan version");
}

async fn subscribe_with(env: &TestEnv, add_on: AddOnId, added_on: NaiveDate) -> SubscriptionId {
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges::default(),
                add_on_changes: AddOnChanges {
                    added: vec![CreateSubscriptionAddOn {
                        add_on_id: add_on,
                        customization: SubscriptionAddOnCustomization::None,
                        quantity: 1,
                    }],
                    edited: vec![],
                    removed: vec![],
                },
            },
            added_on,
        )
        .await
        .expect("add the add-on");
    sub_id
}

async fn change_to_pro_on(env: &TestEnv, sub_id: SubscriptionId, on: NaiveDate) {
    env.services()
        .apply_plan_change_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            PLAN_VERSION_PRO_ID,
            vec![],
            on,
        )
        .await
        .expect("immediate plan change");
}

async fn schedule_pro(env: &TestEnv, sub_id: SubscriptionId) {
    env.services()
        .schedule_plan_change(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            PLAN_VERSION_PRO_ID,
            vec![],
        )
        .await
        .expect("schedule plan change");
}

async fn add_on_rows(env: &TestEnv, sub_id: SubscriptionId) -> Vec<SubscriptionAddOnRow> {
    let mut conn = env.conn().await;
    SubscriptionAddOnRow::list_add_on_history_for_period(&mut conn, &sub_id, day(1, 1), day(12, 31))
        .await
        .expect("add-on history")
}

/// The single add-on row: still open, or ended on `ended_on`.
async fn assert_add_on(env: &TestEnv, sub_id: SubscriptionId, ended_on: Option<NaiveDate>) {
    let rows = add_on_rows(env, sub_id).await;
    assert_eq!(rows.len(), 1, "the add-on keeps its one row");
    assert_eq!(rows[0].effective_to, ended_on);
}

async fn renewal(env: &TestEnv, sub_id: SubscriptionId, period_start: NaiveDate) -> Invoice {
    env.get_invoices(sub_id)
        .await
        .into_iter()
        .find(|invoice| {
            invoice.invoice_type == InvoiceType::Recurring
                && invoice.billing_period_start == period_start
        })
        .unwrap_or_else(|| panic!("renewal invoice of {period_start}"))
}

fn bandwidth_billed(invoice: &Invoice) -> i64 {
    invoice
        .line_items
        .iter()
        .filter(|line| line.metric_id == Some(METRIC_BANDWIDTH))
        .map(|line| line.amount_subtotal)
        .sum()
}

/// What credit notes gave back of the invoice lines billing the add-on.
async fn add_on_credited(env: &TestEnv, sub_id: SubscriptionId) -> i64 {
    let mut credited = 0;
    for invoice in env.get_invoices(sub_id).await {
        let add_on_lines: Vec<&str> = invoice
            .line_items
            .iter()
            .filter(|line| line.sub_add_on_id.is_some())
            .map(|line| line.local_id.as_str())
            .collect();
        for note in env
            .store()
            .list_credit_notes_by_invoice_id(TENANT_ID, invoice.id)
            .await
            .expect("credit notes")
        {
            credited -= note
                .line_items
                .iter()
                .filter(|line| add_on_lines.contains(&line.local_id.as_str()))
                .map(|line| line.amount_subtotal)
                .sum::<i64>();
        }
    }
    credited
}

/// Jan 1–15: 100 units, Jan 15–Feb 1: 200 units. Ended on Jan 15, the add-on bills its 100 units
/// at the renewal; carried over, it bills the month's 300.
async fn usage_add_on_across_immediate_change(target: Target) {
    let env = test_env_with_usage(usage_mock(vec![
        bandwidth(day(1, 1), day(1, 15), 100),
        bandwidth(day(1, 1), day(2, 1), 300),
    ]))
    .await;
    let add_on = usage_add_on(&env).await;
    offer_on(&env, target, PLAN_VERSION_PRO_ID, add_on).await;
    let sub_id = subscribe_with(&env, add_on, day(1, 1)).await;

    change_to_pro_on(&env, sub_id, day(1, 15)).await;
    env.process_cycles().await;

    let feb = renewal(&env, sub_id, day(2, 1)).await;
    match target {
        Target::DoesNotOffer => {
            assert_add_on(&env, sub_id, Some(day(1, 15))).await;
            assert_eq!(
                bandwidth_billed(&feb),
                1000,
                "100 units × €0.10 up to the change"
            );
        }
        Target::Offers => {
            assert_add_on(&env, sub_id, None).await;
            assert_eq!(
                bandwidth_billed(&feb),
                3000,
                "300 units × €0.10 over the month"
            );
        }
    }
}

#[tokio::test]
async fn an_immediate_change_bills_the_usage_of_an_add_on_the_target_does_not_offer() {
    usage_add_on_across_immediate_change(Target::DoesNotOffer).await;
}

#[tokio::test]
async fn an_immediate_change_carries_over_a_usage_add_on_the_target_offers() {
    usage_add_on_across_immediate_change(Target::Offers).await;
}

/// January: 300 units, February: 200 units. A switch at the end of January bills January's usage
/// either way; only an add-on the target offers bills February's.
async fn usage_add_on_across_scheduled_change(target: Target) {
    let env = test_env_with_usage(usage_mock(vec![
        bandwidth(day(1, 1), day(2, 1), 300),
        bandwidth(day(2, 1), day(3, 1), 200),
    ]))
    .await;
    let add_on = usage_add_on(&env).await;
    offer_on(&env, target, PLAN_VERSION_PRO_ID, add_on).await;
    let sub_id = subscribe_with(&env, add_on, day(1, 1)).await;

    schedule_pro(&env, sub_id).await;
    env.process_cycles().await;
    let sub = env.get_subscription(sub_id).await;
    assert_eq!(
        sub.plan_version_id, PLAN_VERSION_PRO_ID,
        "switched on Feb 1"
    );
    assert_eq!(
        bandwidth_billed(&renewal(&env, sub_id, day(2, 1)).await),
        3000,
        "January: 300 units × €0.10"
    );

    env.process_cycles().await;
    let march = bandwidth_billed(&renewal(&env, sub_id, day(3, 1)).await);
    match target {
        Target::DoesNotOffer => {
            assert_add_on(&env, sub_id, Some(day(2, 1))).await;
            assert_eq!(march, 0, "the add-on ended with January");
        }
        Target::Offers => {
            assert_add_on(&env, sub_id, None).await;
            assert_eq!(march, 2000, "February: 200 units × €0.10");
        }
    }
}

#[tokio::test]
async fn a_scheduled_change_bills_the_last_period_of_an_add_on_the_target_does_not_offer() {
    usage_add_on_across_scheduled_change(Target::DoesNotOffer).await;
}

#[tokio::test]
async fn a_scheduled_change_carries_over_a_usage_add_on_the_target_offers() {
    usage_add_on_across_scheduled_change(Target::Offers).await;
}

/// A €20/month add-on billed on the February renewal, the plan changed on Feb 15: ended, it gives
/// back its 14 unused days of 28 (€10) and leaves the March renewal; carried over, it keeps both.
async fn advance_add_on_across_immediate_change(target: Target) {
    let env = test_env_with_usage(usage_mock(vec![])).await;
    let add_on = create_rate_addon(&env, "Premium Support", 2000).await;
    offer_on(&env, target, PLAN_VERSION_PRO_ID, add_on).await;
    let sub_id = subscribe_with(&env, add_on, day(1, 16)).await;
    env.process_cycles().await;
    let billed_for_feb = renewal(&env, sub_id, day(2, 1))
        .await
        .line_items
        .iter()
        .any(|line| line.sub_add_on_id.is_some() && line.amount_subtotal == 2000);
    assert!(billed_for_feb, "February bills the add-on in advance");

    change_to_pro_on(&env, sub_id, day(2, 15)).await;
    env.process_cycles().await;

    let billed_for_march = renewal(&env, sub_id, day(3, 1))
        .await
        .line_items
        .iter()
        .any(|line| line.sub_add_on_id.is_some());
    match target {
        Target::DoesNotOffer => {
            assert_add_on(&env, sub_id, Some(day(2, 15))).await;
            assert_eq!(add_on_credited(&env, sub_id).await, 1000, "14 of 28 days");
            assert!(!billed_for_march, "the add-on ended on Feb 15");
        }
        Target::Offers => {
            assert_add_on(&env, sub_id, None).await;
            assert_eq!(add_on_credited(&env, sub_id).await, 0);
            assert!(billed_for_march, "the add-on carries over");
        }
    }
}

#[tokio::test]
async fn an_immediate_change_credits_the_unused_fee_of_an_add_on_the_target_does_not_offer() {
    advance_add_on_across_immediate_change(Target::DoesNotOffer).await;
}

#[tokio::test]
async fn an_immediate_change_carries_over_an_advance_add_on_the_target_offers() {
    advance_add_on_across_immediate_change(Target::Offers).await;
}
