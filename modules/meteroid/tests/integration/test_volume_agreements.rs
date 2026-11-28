//! Volume agreements (ADR-0021): members' tiers counted on the volume of all of them.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{NaiveDate, NaiveTime, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{
    BaseId, CustomerId, PlanId, PlanVersionId, PriceComponentId, PriceId, SubscriptionId,
    VolumeAgreementId, VolumeAgreementMemberId,
};
use diesel_async::RunQueryDsl;
use diesel_models::enums::BillingPeriodEnum as DieselBillingPeriodEnum;
use diesel_models::volume_agreements::{
    VolumeAgreementBasisRow, VolumeAgreementMemberRow, VolumeAgreementMetricRow, VolumeAgreementRow,
};
use meteroid_store::clients::usage::{
    GroupedUsageData, MockUsageClient, MockUsageDataParams, UsageData,
};
use meteroid_store::domain::enums::InvoiceStatusEnum;
use meteroid_store::domain::price_components::{TierRow, UsagePricingModel};
use meteroid_store::domain::{InvoiceType, UsagePeriod};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use uuid::Uuid;

use crate::data::ids::*;
use crate::data::plans::{PlanSeed, SeedComp};
use crate::harness::{TestEnv, subscription, test_env_with_seed_and_usage};
use crate::meteroid_it::container::SeedLevel;

fn jan(day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 1, day).unwrap()
}

fn feb(day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 2, day).unwrap()
}

/// `units` SMS in January for every customer: the mock reads usage per metric and window.
async fn env_with_january_usage(units: Decimal) -> TestEnv {
    let (start, end) = (
        jan(1).and_time(NaiveTime::MIN),
        feb(1).and_time(NaiveTime::MIN),
    );
    let data = HashMap::from([(
        MockUsageDataParams {
            metric_id: METRIC_BANDWIDTH,
            period_start: start,
            period_end: end,
        },
        UsageData {
            data: vec![GroupedUsageData {
                value: units,
                dimensions: HashMap::new(),
            }],
            period: UsagePeriod { start, end },
        },
    )]);
    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient { data })).await;
    env.set_now(jan(1).and_hms_opt(1, 0, 0).unwrap().and_utc());
    env
}

/// SMS at 0.07 up to 500,000, 0.06 from it, the whole volume at its bracket.
async fn sms_plan(env: &TestEnv) -> PlanVersionId {
    let version_id = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), "SMS", version_id)
        .components(vec![SeedComp::usage(
            PriceComponentId::new(),
            "SMS",
            PRODUCT_BANDWIDTH_ID,
            METRIC_BANDWIDTH,
            PriceId::new(),
            DieselBillingPeriodEnum::Monthly,
            UsagePricingModel::Volume {
                tiers: vec![
                    TierRow {
                        first_unit: 0,
                        rate: dec!(0.07),
                        flat_fee: None,
                        flat_cap: None,
                    },
                    TierRow {
                        first_unit: 500_000,
                        rate: dec!(0.06),
                        flat_fee: None,
                        flat_cap: None,
                    },
                ],
                block_size: None,
            },
        )])
        .seed(&mut env.conn().await)
        .await
        .unwrap();
    version_id
}

async fn subscribe(env: &TestEnv, customer: CustomerId, plan: PlanVersionId) -> SubscriptionId {
    subscription()
        .customer(customer)
        .plan_version(plan)
        .start_date(jan(1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await
}

async fn agreement(env: &TestEnv, members: &[SubscriptionId]) -> VolumeAgreementId {
    let mut conn = env.conn().await;
    let id = VolumeAgreementId::new();
    diesel::insert_into(diesel_models::schema::volume_agreement::table)
        .values(&VolumeAgreementRow {
            id,
            tenant_id: TENANT_ID,
            name: "Group deal".to_string(),
            currency: "EUR".to_string(),
            cadence: DieselBillingPeriodEnum::Monthly,
            billing_day_anchor: 1,
            start_date: jan(1),
            end_date: None,
            owner_customer_id: None,
            archived_at: None,
            created_at: Utc::now(),
            created_by: Uuid::nil(),
            updated_at: None,
        })
        .execute(&mut conn)
        .await
        .unwrap();
    diesel::insert_into(diesel_models::schema::volume_agreement_metric::table)
        .values(&VolumeAgreementMetricRow {
            agreement_id: id,
            tenant_id: TENANT_ID,
            billable_metric_id: METRIC_BANDWIDTH,
        })
        .execute(&mut conn)
        .await
        .unwrap();
    let rows: Vec<VolumeAgreementMemberRow> = members
        .iter()
        .map(|subscription_id| VolumeAgreementMemberRow {
            id: VolumeAgreementMemberId::new(),
            tenant_id: TENANT_ID,
            agreement_id: id,
            subscription_id: *subscription_id,
            from_date: jan(1),
            until_date: None,
            created_at: Utc::now(),
        })
        .collect();
    diesel::insert_into(diesel_models::schema::volume_agreement_member::table)
        .values(&rows)
        .execute(&mut conn)
        .await
        .unwrap();
    id
}

/// What the subscription's invoice closing January bills for its SMS, and its status.
async fn january_sms(env: &TestEnv, subscription_id: SubscriptionId) -> (i64, InvoiceStatusEnum) {
    let invoices = env.get_invoices(subscription_id).await;
    let invoice = invoices
        .iter()
        .find(|i| {
            i.invoice_type == InvoiceType::Recurring
                && i.line_items
                    .iter()
                    .any(|l| l.metric_id == Some(METRIC_BANDWIDTH))
        })
        .unwrap_or_else(|| {
            panic!(
                "no February invoice: {:?}",
                invoices
                    .iter()
                    .map(|i| (i.invoice_date, i.invoice_type.clone(), i.status))
                    .collect::<Vec<_>>()
            )
        });
    let line = invoice
        .line_items
        .iter()
        .find(|l| l.metric_id == Some(METRIC_BANDWIDTH))
        .expect("the SMS line");
    (line.amount_subtotal, invoice.status)
}

/// Two accounts of 400,000 SMS under one agreement are each billed at the 800,000 bracket:
/// 24,000.00 each, not 28,000.00 as the account alone is. The first finalization closes the window.
/// Rules: R-180
#[tokio::test]
async fn members_are_priced_at_the_bracket_of_their_combined_volume() {
    let env = env_with_january_usage(dec!(400000)).await;
    let plan = sms_plan(&env).await;
    let mut members = Vec::new();
    for customer in [CUST_SPOTIFY_ID, CUST_UBER_ID] {
        members.push(subscribe(&env, customer, plan).await);
    }
    let alone = subscribe(&env, CUST_COMODO_ID, plan).await;
    let agreement_id = agreement(&env, &members).await;

    env.run_until(feb(3)).await;

    for member in &members {
        assert_eq!(
            january_sms(&env, *member).await,
            (2_400_000, InvoiceStatusEnum::Finalized)
        );
    }
    assert_eq!(january_sms(&env, alone).await.0, 2_800_000);

    let bases = VolumeAgreementBasisRow::of_window(&mut env.conn().await, agreement_id, jan(1))
        .await
        .unwrap();
    assert_eq!(bases.len(), 1);
    assert_eq!(bases[0].quantity, dec!(800000));
    assert_eq!(bases[0].period_end, feb(1));
}

/// A member's invoice waits for the window's close: its end plus the longest grace of the members'
/// entities, refused by hand before it.
#[tokio::test]
async fn a_member_invoice_is_finalized_once_its_window_closes() {
    let env = env_with_january_usage(dec!(400000)).await;
    let plan = sms_plan(&env).await;
    let a = subscribe(&env, CUST_SPOTIFY_ID, plan).await;
    let b = subscribe(&env, CUST_UBER_ID, plan).await;
    agreement(&env, &[a, b]).await;

    env.run_until(feb(1)).await;
    let (amount, status) = january_sms(&env, a).await;
    assert_eq!(amount, 2_400_000, "priced live at the 800,000 bracket");
    assert_eq!(status, InvoiceStatusEnum::Draft);

    let draft = env
        .get_invoices(a)
        .await
        .into_iter()
        .find(|i| i.invoice_date == feb(1))
        .unwrap();
    let refused = env
        .services()
        .finalize_invoice(Actor::System, draft.id, TENANT_ID)
        .await
        .expect_err("finalized before the close");
    assert!(
        format!("{refused:?}").contains("waits for its volume agreement"),
        "{refused:?}"
    );
}
