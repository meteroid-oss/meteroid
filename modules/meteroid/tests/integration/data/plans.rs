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
                tenant_id: ids::TENANT_ID,
                period_start_day: None,
                net_terms: 0,
                currency: "EUR".to_string(),
                billing_cycles: None,
                created_by: ids::USER_ID,
                trialing_plan_id: None,
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
                tenant_id: ids::TENANT_ID,
                period_start_day: None,
                net_terms: 0,
                currency: "EUR".to_string(),
                billing_cycles: None,
                created_by: ids::USER_ID,
                trialing_plan_id: None,
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
                tenant_id: ids::TENANT_ID,
                period_start_day: None,
                net_terms: 0,
                currency: "EUR".to_string(),
                billing_cycles: None,
                created_by: ids::USER_ID,
                trialing_plan_id: None,
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
                tenant_id: ids::TENANT_ID,
                period_start_day: None,
                net_terms: 0,
                currency: "EUR".to_string(),
                billing_cycles: None,
                created_by: ids::USER_ID,
                trialing_plan_id: None,
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

            // === Trial-related plans ===

            // Free plan (for downgrade tests)
            PlanRowNew {
                id: ids::PLAN_FREE_ID,
                name: "Free".to_string(),
                description: Some("Free tier plan".to_string()),
                created_by: ids::USER_ID,
                tenant_id: ids::TENANT_ID,
                product_family_id: ids::PRODUCT_FAMILY_ID,
                plan_type: PlanTypeEnum::Free,
                status: PlanStatusEnum::Active,
            }
            .insert(tx)
            .await?;

            PlanVersionRowNew {
                id: ids::PLAN_VERSION_FREE_ID,
                is_draft_version: false,
                plan_id: ids::PLAN_FREE_ID,
                version: 1,
                trial_duration_days: None,
                tenant_id: ids::TENANT_ID,
                period_start_day: None,
                net_terms: 0,
                currency: "EUR".to_string(),
                billing_cycles: None,
                created_by: ids::USER_ID,
                trialing_plan_id: None,
                trial_is_free: false,
            }
            .insert(tx)
            .await?;

            PlanRowPatch {
                id: ids::PLAN_FREE_ID,
                tenant_id: ids::TENANT_ID,
                name: None,
                description: None,
                active_version_id: Some(Some(ids::PLAN_VERSION_FREE_ID)),
                draft_version_id: None,
            }
            .update(tx)
            .await?;

            // Enterprise plan (for trialing_plan tests)
            PlanRowNew {
                id: ids::PLAN_ENTERPRISE_ID,
                name: "Enterprise".to_string(),
                description: Some("Enterprise tier plan".to_string()),
                created_by: ids::USER_ID,
                tenant_id: ids::TENANT_ID,
                product_family_id: ids::PRODUCT_FAMILY_ID,
                plan_type: PlanTypeEnum::Standard,
                status: PlanStatusEnum::Active,
            }
            .insert(tx)
            .await?;

            PlanVersionRowNew {
                id: ids::PLAN_VERSION_ENTERPRISE_ID,
                is_draft_version: false,
                plan_id: ids::PLAN_ENTERPRISE_ID,
                version: 1,
                trial_duration_days: None,
                tenant_id: ids::TENANT_ID,
                period_start_day: None,
                net_terms: 0,
                currency: "EUR".to_string(),
                billing_cycles: None,
                created_by: ids::USER_ID,
                trialing_plan_id: None,
                trial_is_free: false,
            }
            .insert(tx)
            .await?;

            PlanRowPatch {
                id: ids::PLAN_ENTERPRISE_ID,
                tenant_id: ids::TENANT_ID,
                name: None,
                description: None,
                active_version_id: Some(Some(ids::PLAN_VERSION_ENTERPRISE_ID)),
                draft_version_id: None,
            }
            .update(tx)
            .await?;

            // Free plan with trial configuration
            // This plan has:
            // - 7 day trial
            // - trialing_plan_id pointing to Enterprise (during trial, use Enterprise features)
            // After trial ends, subscription continues on Free plan
            PlanRowNew {
                id: ids::PLAN_PRO_WITH_TRIAL_ID,
                name: "Free with Trial".to_string(),
                description: Some("Free plan with trial period".to_string()),
                created_by: ids::USER_ID,
                tenant_id: ids::TENANT_ID,
                product_family_id: ids::PRODUCT_FAMILY_ID,
                plan_type: PlanTypeEnum::Free,
                status: PlanStatusEnum::Active,
            }
            .insert(tx)
            .await?;

            PlanVersionRowNew {
                id: ids::PLAN_VERSION_PRO_WITH_TRIAL_ID,
                is_draft_version: false,
                plan_id: ids::PLAN_PRO_WITH_TRIAL_ID,
                version: 1,
                trial_duration_days: Some(7),
                tenant_id: ids::TENANT_ID,
                period_start_day: None,
                net_terms: 0,
                currency: "EUR".to_string(),
                billing_cycles: None,
                created_by: ids::USER_ID,
                trialing_plan_id: Some(ids::PLAN_ENTERPRISE_ID),
                trial_is_free: true,
            }
            .insert(tx)
            .await?;

            PlanRowPatch {
                id: ids::PLAN_PRO_WITH_TRIAL_ID,
                tenant_id: ids::TENANT_ID,
                name: None,
                description: None,
                active_version_id: Some(Some(ids::PLAN_VERSION_PRO_WITH_TRIAL_ID)),
                draft_version_id: None,
            }
            .update(tx)
            .await?;

            // === Paid plan with FREE trial ===
            // Standard plan type with trial_is_free = true
            // After trial ends without payment method: TrialExpired
            // After trial ends with payment method: Active + invoice
            PlanRowNew {
                id: ids::PLAN_PAID_FREE_TRIAL_ID,
                name: "Paid with Free Trial".to_string(),
                description: Some("Paid plan with 14-day free trial".to_string()),
                created_by: ids::USER_ID,
                tenant_id: ids::TENANT_ID,
                product_family_id: ids::PRODUCT_FAMILY_ID,
                plan_type: PlanTypeEnum::Standard,
                status: PlanStatusEnum::Active,
            }
            .insert(tx)
            .await?;

            PlanVersionRowNew {
                id: ids::PLAN_VERSION_PAID_FREE_TRIAL_ID,
                is_draft_version: false,
                plan_id: ids::PLAN_PAID_FREE_TRIAL_ID,
                version: 1,
                trial_duration_days: Some(14),
                tenant_id: ids::TENANT_ID,
                period_start_day: None,
                net_terms: 0,
                currency: "EUR".to_string(),
                billing_cycles: None,
                created_by: ids::USER_ID,
                trialing_plan_id: Some(ids::PLAN_ENTERPRISE_ID), // Enterprise features during trial
                trial_is_free: true,                             // Free trial
            }
            .insert(tx)
            .await?;

            PlanRowPatch {
                id: ids::PLAN_PAID_FREE_TRIAL_ID,
                tenant_id: ids::TENANT_ID,
                name: None,
                description: None,
                active_version_id: Some(Some(ids::PLAN_VERSION_PAID_FREE_TRIAL_ID)),
                draft_version_id: None,
            }
            .update(tx)
            .await?;

            PriceComponentRowNew {
                id: ids::COMP_PAID_FREE_TRIAL_RATE_ID,
                name: "Monthly Rate".to_string(),
                fee: FeeType::Rate {
                    rates: vec![TermRate {
                        price: rust_decimal::Decimal::new(4900, 2), // $49/month
                        term: BillingPeriodEnum::Monthly,
                    }],
                }
                .try_into()
                .unwrap(),
                plan_version_id: ids::PLAN_VERSION_PAID_FREE_TRIAL_ID,
                product_id: None,
                billable_metric_id: None,
            }
            .insert(tx)
            .await?;

            // === Paid plan with PAID trial ===
            // Standard plan type with trial_is_free = false
            // Bills immediately during trial but gives Enterprise features
            PlanRowNew {
                id: ids::PLAN_PAID_TRIAL_ID,
                name: "Paid with Paid Trial".to_string(),
                description: Some(
                    "Paid plan with 7-day paid trial (Enterprise features)".to_string(),
                ),
                created_by: ids::USER_ID,
                tenant_id: ids::TENANT_ID,
                product_family_id: ids::PRODUCT_FAMILY_ID,
                plan_type: PlanTypeEnum::Standard,
                status: PlanStatusEnum::Active,
            }
            .insert(tx)
            .await?;

            PlanVersionRowNew {
                id: ids::PLAN_VERSION_PAID_TRIAL_ID,
                is_draft_version: false,
                plan_id: ids::PLAN_PAID_TRIAL_ID,
                version: 1,
                trial_duration_days: Some(7),
                tenant_id: ids::TENANT_ID,
                period_start_day: None,
                net_terms: 0,
                currency: "EUR".to_string(),
                billing_cycles: None,
                created_by: ids::USER_ID,
                trialing_plan_id: Some(ids::PLAN_ENTERPRISE_ID), // Enterprise features during trial
                trial_is_free: false,                            // Paid trial - bill immediately
            }
            .insert(tx)
            .await?;

            PlanRowPatch {
                id: ids::PLAN_PAID_TRIAL_ID,
                tenant_id: ids::TENANT_ID,
                name: None,
                description: None,
                active_version_id: Some(Some(ids::PLAN_VERSION_PAID_TRIAL_ID)),
                draft_version_id: None,
            }
            .update(tx)
            .await?;

            PriceComponentRowNew {
                id: ids::COMP_PAID_TRIAL_RATE_ID,
                name: "Monthly Rate".to_string(),
                fee: FeeType::Rate {
                    rates: vec![TermRate {
                        price: rust_decimal::Decimal::new(9900, 2), // $99/month
                        term: BillingPeriodEnum::Monthly,
                    }],
                }
                .try_into()
                .unwrap(),
                plan_version_id: ids::PLAN_VERSION_PAID_TRIAL_ID,
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
