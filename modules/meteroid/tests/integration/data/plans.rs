use super::ids;
use diesel_async::AsyncConnection;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::enums::{PlanStatusEnum, PlanTypeEnum};
use diesel_models::errors::DatabaseErrorContainer;
use diesel_models::plan_versions::PlanVersionRowNew;
use diesel_models::plans::{PlanRowNew, PlanRowPatch};
use diesel_models::price_components::PriceComponentRowNew;
use meteroid_store::domain::{
    BillingPeriodEnum, DowngradePolicy, FeeType, TermRate, UpgradePolicy,
};
use meteroid_store::store::PgPool;

pub async fn run_plans_seed(pool: &PgPool) {
    let mut conn = pool
        .get()
        .await
        .expect("couldn't get db connection from pool");

    conn.transaction(|tx| {
        async move {
            // leetcode

            PlanRowNew {
                id: ids::PLAN_LEETCODE_ID,
                name: "LeetCode".to_string(),
                description: None,
                created_by: ids::USER_ID,
                tenant_id: ids::TENANT_ID,
                product_family_id: ids::PRODUCT_FAMILY_ID,
                plan_type: PlanTypeEnum::Standard,
                status: PlanStatusEnum::Active,
            }
            .insert(tx)
            .await?;

            PlanVersionRowNew {
                id: ids::PLAN_VERSION_1_LEETCODE_ID,
                is_draft_version: false,
                plan_id: ids::PLAN_LEETCODE_ID,
                version: 1,
                trial_duration_days: None,
                downgrade_plan_id: None,
                tenant_id: ids::TENANT_ID,
                period_start_day: None,
                net_terms: 0,
                currency: "EUR".to_string(),
                billing_cycles: None,
                created_by: ids::USER_ID,
                trialing_plan_id: None,
                action_after_trial: None,
                trial_is_free: false,
            }
            .insert(tx)
            .await?;

            PlanVersionRowNew {
                id: ids::PLAN_VERSION_2_LEETCODE_ID,
                is_draft_version: true,
                plan_id: ids::PLAN_LEETCODE_ID,
                version: 2,
                trial_duration_days: None,
                downgrade_plan_id: None,
                tenant_id: ids::TENANT_ID,
                period_start_day: None,
                net_terms: 0,
                currency: "EUR".to_string(),
                billing_cycles: None,
                created_by: ids::USER_ID,
                trialing_plan_id: None,
                action_after_trial: None,
                trial_is_free: false,
            }
            .insert(tx)
            .await?;

            PlanRowPatch {
                id: ids::PLAN_LEETCODE_ID,
                tenant_id: ids::TENANT_ID,
                name: None,
                description: None,
                active_version_id: Some(Some(ids::PLAN_VERSION_1_LEETCODE_ID)),
                draft_version_id: Some(Some(ids::PLAN_VERSION_2_LEETCODE_ID)),
            }
            .update(tx)
            .await?;

            PriceComponentRowNew {
                id: ids::COMP_LEETCODE_RATE_ID,
                name: "Subscription Rate".to_string(),
                fee: FeeType::Rate {
                    rates: vec![TermRate {
                        price: rust_decimal::Decimal::new(3500, 2),
                        term: BillingPeriodEnum::Monthly,
                    }],
                }
                .try_into()
                .unwrap(),
                plan_version_id: ids::PLAN_VERSION_1_LEETCODE_ID,
                product_id: None,
                billable_metric_id: None,
            }
            .insert(tx)
            .await?;

            // notion

            PlanRowNew {
                id: ids::PLAN_NOTION_ID,
                name: "Notion".to_string(),
                description: None,
                created_by: ids::USER_ID,
                tenant_id: ids::TENANT_ID,
                product_family_id: ids::PRODUCT_FAMILY_ID,
                plan_type: PlanTypeEnum::Standard,
                status: PlanStatusEnum::Active,
            }
            .insert(tx)
            .await?;

            PlanVersionRowNew {
                id: ids::PLAN_VERSION_NOTION_ID,
                is_draft_version: false,
                plan_id: ids::PLAN_NOTION_ID,
                version: 1,
                trial_duration_days: None,
                downgrade_plan_id: None,
                tenant_id: ids::TENANT_ID,
                period_start_day: None,
                net_terms: 0,
                currency: "EUR".to_string(),
                billing_cycles: None,
                created_by: ids::USER_ID,
                trialing_plan_id: None,
                action_after_trial: None,
                trial_is_free: false,
            }
            .insert(tx)
            .await?;

            PlanRowPatch {
                id: ids::PLAN_NOTION_ID,
                tenant_id: ids::TENANT_ID,
                name: None,
                description: None,
                active_version_id: Some(Some(ids::PLAN_VERSION_NOTION_ID)),
                draft_version_id: None,
            }
            .update(tx)
            .await?;

            PriceComponentRowNew {
                id: ids::COMP_NOTION_SEATS_ID,
                name: "Seats".to_string(),
                fee: FeeType::Slot {
                    quota: None,
                    rates: vec![
                        TermRate {
                            price: rust_decimal::Decimal::new(1000, 2),
                            term: BillingPeriodEnum::Monthly,
                        },
                        TermRate {
                            price: rust_decimal::Decimal::new(9600, 2),
                            term: BillingPeriodEnum::Annual,
                        },
                    ],
                    slot_unit_name: "Seats".to_string(),
                    minimum_count: Some(1),
                    upgrade_policy: UpgradePolicy::Prorated,
                    downgrade_policy: DowngradePolicy::RemoveAtEndOfPeriod,
                }
                .try_into()
                .unwrap(),
                plan_version_id: ids::PLAN_VERSION_NOTION_ID,
                product_id: None,
                billable_metric_id: None,
            }
            .insert(tx)
            .await?;

            // supabase

            PlanRowNew {
                id: ids::PLAN_SUPABASE_ID,
                name: "Supabase".to_string(),
                description: None,
                created_by: ids::USER_ID,
                tenant_id: ids::TENANT_ID,
                product_family_id: ids::PRODUCT_FAMILY_ID,
                plan_type: PlanTypeEnum::Standard,
                status: PlanStatusEnum::Active,
            }
            .insert(tx)
            .await?;

            PlanVersionRowNew {
                id: ids::PLAN_VERSION_SUPABASE_ID,
                is_draft_version: false,
                plan_id: ids::PLAN_SUPABASE_ID,
                version: 3,
                trial_duration_days: None,
                downgrade_plan_id: None,
                tenant_id: ids::TENANT_ID,
                period_start_day: None,
                net_terms: 0,
                currency: "EUR".to_string(),
                billing_cycles: None,
                created_by: ids::USER_ID,
                trialing_plan_id: None,
                action_after_trial: None,
                trial_is_free: false,
            }
            .insert(tx)
            .await?;

            PlanRowPatch {
                id: ids::PLAN_SUPABASE_ID,
                tenant_id: ids::TENANT_ID,
                name: None,
                description: None,
                active_version_id: Some(Some(ids::PLAN_VERSION_SUPABASE_ID)),
                draft_version_id: None,
            }
            .update(tx)
            .await?;

            PriceComponentRowNew {
                id: ids::COMP_SUPABASE_ORG_SLOTS_ID,
                name: "Organization Slots".to_string(),
                fee: FeeType::Slot {
                    quota: None,
                    rates: vec![TermRate {
                        price: rust_decimal::Decimal::new(2500, 2),
                        term: BillingPeriodEnum::Monthly,
                    }],
                    slot_unit_name: "Organization".to_string(),
                    minimum_count: Some(1),
                    upgrade_policy: UpgradePolicy::Prorated,
                    downgrade_policy: DowngradePolicy::RemoveAtEndOfPeriod,
                }
                .try_into()
                .unwrap(),
                plan_version_id: ids::PLAN_VERSION_SUPABASE_ID,
                product_id: None,
                billable_metric_id: None,
            }
            .insert(tx)
            .await?;

            Ok::<(), DatabaseErrorContainer>(())
        }
        .scope_boxed()
    })
    .await
    .unwrap();
}
