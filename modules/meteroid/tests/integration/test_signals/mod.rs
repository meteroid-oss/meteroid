//! The customer summary and a subscription's usage.

mod summary;

use std::sync::Arc;

use chrono::{NaiveDate, NaiveDateTime};
use common_domain::actor::Actor;
use common_domain::ids::{
    BaseId, CustomerId, EntitlementEntityId, FeatureId, InvoiceId, PlanVersionId, SubscriptionId,
};
use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use meteroid_store::clients::usage::{
    GroupedUsageData, MockUsageDataParams, MutableMockUsageClient, UsageData,
};
use meteroid_store::domain::customer_summary::{CustomerSummary, RenewalMode, SummaryParts};
use meteroid_store::domain::entitlements::{self, FeatureNew, FeatureType};
use meteroid_store::domain::enums::{InvoicePaymentStatus, InvoiceStatusEnum, InvoiceType};
use meteroid_store::domain::scheduled_events::{ScheduledEventData, ScheduledEventNew};
use meteroid_store::domain::{InvoiceNew, UsagePeriod};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterfaceAuto;
use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;
use meteroid_store::repositories::{
    EntitlementsInterface, InvoiceInterface, SubscriptionInterface,
};
use meteroid_store::services::fx::reporting_fx;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::data::ids::*;
use crate::harness::{TestEnv, subscription, test_env, test_env_with_usage};

fn now() -> NaiveDateTime {
    at(2026, 9, 15, 12)
}

fn at(y: i32, m: u32, d: u32, h: u32) -> NaiveDateTime {
    date(y, m, d).and_hms_opt(h, 0, 0).unwrap()
}

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// A billing-active subscription with a pinned MRR, currency and cycle.
async fn active_subscription(
    env: &TestEnv,
    customer: CustomerId,
    start: NaiveDate,
    currency: &str,
    mrr_cents: i64,
    period: (NaiveDate, NaiveDate),
) -> SubscriptionId {
    let id = subscription()
        .customer(customer)
        .start_date(start)
        .create(env.services())
        .await;

    use diesel_models::enums::SubscriptionStatusEnum;
    use diesel_models::schema::subscription::dsl as s;
    diesel::update(s::subscription.filter(s::id.eq(id)))
        .set((
            s::status.eq(SubscriptionStatusEnum::Active),
            s::currency.eq(currency),
            s::mrr_cents.eq(mrr_cents),
        ))
        .execute(&mut env.conn().await)
        .await
        .unwrap();
    env.set_subscription_period(id, period.0, period.1).await;
    id
}

fn cycle_start() -> NaiveDate {
    date(2026, 9, 1)
}

/// A finalized invoice of `subscription` owing `amount_due`, as the billing pipeline leaves it.
async fn owed_invoice(
    env: &TestEnv,
    subscription: SubscriptionId,
    number: &str,
    (invoice_date, due_on): (NaiveDate, NaiveDate),
    amount_due: i64,
    payment_status: InvoicePaymentStatus,
) -> InvoiceId {
    let sub = env.get_subscription(subscription).await;
    let customer = env.get_customer(sub.customer_id).await;
    let seller = env
        .store()
        .get_invoicing_entity(TENANT_ID, Some(INVOICING_ENTITY_ID))
        .await
        .unwrap();
    env.store()
        .insert_invoice(InvoiceNew {
            status: InvoiceStatusEnum::Finalized,
            tenant_id: TENANT_ID,
            customer_id: sub.customer_id,
            subscription_id: Some(subscription),
            cycle_index: None,
            currency: sub.currency.clone(),
            invoice_number: number.to_string(),
            line_items: vec![crate::test_customer_credit::line(
                "Services", amount_due, false,
            )],
            coupons: vec![],
            data_updated_at: None,
            invoice_date,
            billing_period_start: invoice_date,
            tax_inclusive: false,
            plan_version_id: Some(sub.plan_version_id),
            invoice_type: InvoiceType::Recurring,
            finalized_at: Some(invoice_date.and_hms_opt(0, 0, 0).unwrap()),
            subtotal: amount_due,
            subtotal_recurring: amount_due,
            discount: 0,
            tax_amount: 0,
            total: amount_due,
            amount_due,
            applied_credits: 0,
            promotional_credits: 0,
            net_terms: 10,
            reference: None,
            purchase_order: None,
            memo: None,
            due_at: Some(due_on.and_hms_opt(0, 0, 0).unwrap()),
            plan_name: None,
            customer_details: customer.into(),
            seller_details: seller.into(),
            auto_advance: false,
            payment_status,
            tax_breakdown: vec![],
            manual: false,
            invoicing_entity_id: INVOICING_ENTITY_ID,
            parent_invoice_id: None,
            consolidated_into_invoice_id: None,
            annexes: vec![],
            custom_properties: serde_json::Value::Null,
        })
        .await
        .unwrap()
        .id
}

async fn record_failed_payment(env: &TestEnv, invoice: InvoiceId, at: NaiveDateTime) {
    diesel::sql_query(
        "INSERT INTO payment_transaction (id, tenant_id, invoice_id, amount, currency, status,
                                          payment_type, error_type, created_at)
         SELECT $1, i.tenant_id, i.id, i.amount_due, i.currency, 'FAILED', 'PAYMENT',
                'card_declined', $3 AT TIME ZONE 'UTC'
         FROM invoice i WHERE i.id = $2",
    )
    .bind::<diesel::sql_types::Uuid, _>(uuid::Uuid::now_v7())
    .bind::<diesel::sql_types::Uuid, _>(*invoice)
    .bind::<diesel::sql_types::Timestamp, _>(at)
    .execute(&mut env.conn().await)
    .await
    .unwrap();
}

async fn schedule_retry(
    env: &TestEnv,
    subscription: SubscriptionId,
    invoice: InvoiceId,
    at: NaiveDateTime,
) {
    env.store()
        .schedule_events(
            &mut env.conn().await,
            vec![ScheduledEventNew {
                subscription_id: subscription,
                tenant_id: TENANT_ID,
                scheduled_time: at,
                event_data: ScheduledEventData::RetryPayment {
                    invoice_id: invoice,
                },
                source: "dunning".to_string(),
                created_by: Actor::System,
            }],
        )
        .await
        .unwrap();
}

/// A billing-active subscription on `plan_version` whose current cycle ends on `period_end`.
async fn plan_subscription(
    env: &TestEnv,
    customer: CustomerId,
    plan_version: PlanVersionId,
    mrr_cents: i64,
    period_end: NaiveDate,
) -> SubscriptionId {
    let id = subscription()
        .customer(customer)
        .plan_version(plan_version)
        .start_date(date(2024, 1, 1))
        .create(env.services())
        .await;

    use diesel_models::enums::SubscriptionStatusEnum;
    use diesel_models::schema::subscription::dsl as s;
    diesel::update(s::subscription.filter(s::id.eq(id)))
        .set((
            s::status.eq(SubscriptionStatusEnum::Active),
            s::mrr_cents.eq(mrr_cents),
        ))
        .execute(&mut env.conn().await)
        .await
        .unwrap();
    env.set_subscription_period(id, cycle_start(), period_end)
        .await;
    id
}

fn set_usage(usage: &MutableMockUsageClient, start: NaiveDateTime, end: NaiveDateTime, value: i64) {
    usage.set_usage(
        MockUsageDataParams {
            metric_id: METRIC_BANDWIDTH,
            period_start: start,
            period_end: end,
        },
        UsageData {
            data: vec![GroupedUsageData {
                value: Decimal::from(value),
                dimensions: std::collections::HashMap::new(),
            }],
            period: UsagePeriod { start, end },
        },
    );
}

/// A subscription-level, billing-cycle limit on the bandwidth metric.
async fn cycle_limit(
    env: &TestEnv,
    subscription: SubscriptionId,
    code: &str,
    limit: i64,
    overage_behavior: entitlements::OverageBehavior,
) -> FeatureId {
    cycle_limit_on(
        env,
        EntitlementEntityId::Subscription(subscription),
        code,
        limit,
        overage_behavior,
    )
    .await
}

/// A billing-cycle limit on the bandwidth metric, set on `entity`.
async fn cycle_limit_on(
    env: &TestEnv,
    entity: EntitlementEntityId,
    code: &str,
    limit: i64,
    overage_behavior: entitlements::OverageBehavior,
) -> FeatureId {
    let feature = env
        .store()
        .create_feature(
            Actor::System,
            FeatureNew {
                tenant_id: TENANT_ID,
                product_id: None,
                name: format!("API calls {code}"),
                code: code.to_string(),
                description: None,
                feature_type: FeatureType::Metered {
                    metric_id: METRIC_BANDWIDTH,
                },
                entitlement: None,
            },
        )
        .await
        .unwrap();
    env.store()
        .create_entitlement(entitlements::EntitlementNew {
            tenant_id: TENANT_ID,
            feature_id: feature.id,
            entity,
            value: entitlements::EntitlementValue::Metered {
                limit: Some(Decimal::from(limit)),
                reset_period: entitlements::ResetPeriod::BillingCycle,
                overage_behavior,
                thresholds: vec![],
                enabled: true,
                group_by: None,
            },
        })
        .await
        .unwrap();
    feature.id
}

/// Usage Alpha (API calls at €0.10) for Uber on `[Sep 1, Oct 1)`, limited to 1 000 per cycle.
async fn limited_uber(env: &TestEnv) -> SubscriptionId {
    let sub = plan_subscription(
        env,
        CUST_UBER_ID,
        PLAN_VERSION_USAGE_ALPHA_ID,
        1_000,
        date(2026, 10, 1),
    )
    .await;
    cycle_limit(
        env,
        sub,
        "api_calls",
        1_000,
        entitlements::OverageBehavior::Allow,
    )
    .await;
    sub
}

const ALL_PARTS: SummaryParts = SummaryParts {
    invoices: true,
    subscriptions: true,
};

async fn summary(env: &TestEnv, customer: CustomerId, parts: SummaryParts) -> CustomerSummary {
    env.services()
        .get_customer_summary(TENANT_ID, customer, parts, now())
        .await
        .unwrap()
}

/// Uber owes an overdue invoice whose collection failed, a retry scheduled.
async fn failing_payment(env: &TestEnv, subscription: SubscriptionId) -> InvoiceId {
    diesel::sql_query("UPDATE invoice SET payment_status = 'PAID', amount_due = 0")
        .execute(&mut env.conn().await)
        .await
        .unwrap();
    let invoice = owed_invoice(
        env,
        subscription,
        "OV-1",
        (date(2026, 8, 20), date(2026, 8, 30)),
        30_000,
        InvoicePaymentStatus::Errored,
    )
    .await;
    record_failed_payment(env, invoice, at(2026, 9, 1, 10)).await;
    schedule_retry(env, subscription, invoice, at(2026, 9, 18, 9)).await;
    invoice
}
