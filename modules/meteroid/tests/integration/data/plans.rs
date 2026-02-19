use super::ids;
use diesel_async::AsyncConnection;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::enums::{
    BillingPeriodEnum as DieselBillingPeriodEnum, FeeTypeEnum as DieselFeeTypeEnum,
    PlanStatusEnum, PlanTypeEnum,
};
use diesel_models::errors::DatabaseErrorContainer;
use diesel_models::plan_component_prices::PlanComponentPriceRowNew;
use diesel_models::plan_versions::PlanVersionRowNew;
use diesel_models::plans::{PlanRowNew, PlanRowPatch};
use diesel_models::price_components::PriceComponentRowNew;
use diesel_models::prices::PriceRowNew;
use diesel_models::products::ProductRowNew;
use meteroid_store::domain::prices::{FeeStructure, Pricing};
use meteroid_store::domain::{
    BillingPeriodEnum, DowngradePolicy, FeeType, TermRate, UpgradePolicy,
};
use common_domain::ids::*;
use diesel_models::PgConn;
use meteroid_store::store::PgPool;
use rust_decimal::Decimal;

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
                trial_is_free: true,
                uses_product_pricing: true,
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
                trial_is_free: true,
                uses_product_pricing: true,
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

            ProductRowNew {
                id: ids::PRODUCT_LEETCODE_RATE_ID,
                name: "Subscription Rate".to_string(),
                description: None,
                created_by: ids::USER_ID,
                tenant_id: ids::TENANT_ID,
                product_family_id: ids::PRODUCT_FAMILY_ID,
                fee_type: DieselFeeTypeEnum::Rate,
                fee_structure: serde_json::to_value(&FeeStructure::Rate {}).unwrap(),
            }
            .insert(tx)
            .await?;

            PriceComponentRowNew {
                id: ids::COMP_LEETCODE_RATE_ID,
                name: "Subscription Rate".to_string(),
                legacy_fee: Some(
                    FeeType::Rate {
                        rates: vec![TermRate {
                            price: rust_decimal::Decimal::new(3500, 2),
                            term: BillingPeriodEnum::Monthly,
                        }],
                    }
                    .try_into()
                    .unwrap(),
                ),
                plan_version_id: ids::PLAN_VERSION_1_LEETCODE_ID,
                product_id: Some(ids::PRODUCT_LEETCODE_RATE_ID),
                billable_metric_id: None,
            }
            .insert(tx)
            .await?;

            PriceRowNew {
                id: ids::PRICE_LEETCODE_RATE_ID,
                product_id: ids::PRODUCT_LEETCODE_RATE_ID,
                cadence: DieselBillingPeriodEnum::Monthly,
                currency: "EUR".to_string(),
                pricing: serde_json::to_value(&Pricing::Rate {
                    rate: Decimal::new(3500, 2),
                })
                .unwrap(),
                tenant_id: ids::TENANT_ID,
                created_by: ids::USER_ID,
            }
            .insert(tx)
            .await?;

            PlanComponentPriceRowNew::insert_batch(
                tx,
                &[PlanComponentPriceRowNew {
                    plan_component_id: ids::COMP_LEETCODE_RATE_ID,
                    price_id: ids::PRICE_LEETCODE_RATE_ID,
                }],
            )
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
                trial_is_free: true,
                uses_product_pricing: true,
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

            ProductRowNew {
                id: ids::PRODUCT_NOTION_SEATS_ID,
                name: "Seats".to_string(),
                description: None,
                created_by: ids::USER_ID,
                tenant_id: ids::TENANT_ID,
                product_family_id: ids::PRODUCT_FAMILY_ID,
                fee_type: DieselFeeTypeEnum::Slot,
                fee_structure: serde_json::to_value(&FeeStructure::Slot {
                        unit_name: "Seats".to_string(),
                        min_slots: Some(1),
                        max_slots: None,
                        upgrade_policy: UpgradePolicy::Prorated,
                        downgrade_policy: DowngradePolicy::RemoveAtEndOfPeriod,
                    })
                    .unwrap(),
            }
            .insert(tx)
            .await?;

            PriceComponentRowNew {
                id: ids::COMP_NOTION_SEATS_ID,
                name: "Seats".to_string(),
                legacy_fee: Some(
                    FeeType::Slot {
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
                ),
                plan_version_id: ids::PLAN_VERSION_NOTION_ID,
                product_id: Some(ids::PRODUCT_NOTION_SEATS_ID),
                billable_metric_id: None,
            }
            .insert(tx)
            .await?;

            PriceRowNew {
                id: ids::PRICE_NOTION_SEATS_MONTHLY_ID,
                product_id: ids::PRODUCT_NOTION_SEATS_ID,
                cadence: DieselBillingPeriodEnum::Monthly,
                currency: "EUR".to_string(),
                pricing: serde_json::to_value(&Pricing::Slot {
                    unit_rate: Decimal::new(1000, 2),
                })
                .unwrap(),
                tenant_id: ids::TENANT_ID,
                created_by: ids::USER_ID,
            }
            .insert(tx)
            .await?;

            PriceRowNew {
                id: ids::PRICE_NOTION_SEATS_ANNUAL_ID,
                product_id: ids::PRODUCT_NOTION_SEATS_ID,
                cadence: DieselBillingPeriodEnum::Annual,
                currency: "EUR".to_string(),
                pricing: serde_json::to_value(&Pricing::Slot {
                    unit_rate: Decimal::new(9600, 2),
                })
                .unwrap(),
                tenant_id: ids::TENANT_ID,
                created_by: ids::USER_ID,
            }
            .insert(tx)
            .await?;

            PlanComponentPriceRowNew::insert_batch(
                tx,
                &[
                    PlanComponentPriceRowNew {
                        plan_component_id: ids::COMP_NOTION_SEATS_ID,
                        price_id: ids::PRICE_NOTION_SEATS_MONTHLY_ID,
                    },
                    PlanComponentPriceRowNew {
                        plan_component_id: ids::COMP_NOTION_SEATS_ID,
                        price_id: ids::PRICE_NOTION_SEATS_ANNUAL_ID,
                    },
                ],
            )
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
                trial_is_free: true,
                uses_product_pricing: true,
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

            ProductRowNew {
                id: ids::PRODUCT_SUPABASE_ORG_SLOTS_ID,
                name: "Organization Slots".to_string(),
                description: None,
                created_by: ids::USER_ID,
                tenant_id: ids::TENANT_ID,
                product_family_id: ids::PRODUCT_FAMILY_ID,
                fee_type: DieselFeeTypeEnum::Slot,
                fee_structure: serde_json::to_value(&FeeStructure::Slot {
                        unit_name: "Organization".to_string(),
                        min_slots: Some(1),
                        max_slots: None,
                        upgrade_policy: UpgradePolicy::Prorated,
                        downgrade_policy: DowngradePolicy::RemoveAtEndOfPeriod,
                    })
                    .unwrap(),
            }
            .insert(tx)
            .await?;

            PriceComponentRowNew {
                id: ids::COMP_SUPABASE_ORG_SLOTS_ID,
                name: "Organization Slots".to_string(),
                legacy_fee: Some(
                    FeeType::Slot {
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
                ),
                plan_version_id: ids::PLAN_VERSION_SUPABASE_ID,
                product_id: Some(ids::PRODUCT_SUPABASE_ORG_SLOTS_ID),
                billable_metric_id: None,
            }
            .insert(tx)
            .await?;

            PriceRowNew {
                id: ids::PRICE_SUPABASE_ORG_SLOTS_ID,
                product_id: ids::PRODUCT_SUPABASE_ORG_SLOTS_ID,
                cadence: DieselBillingPeriodEnum::Monthly,
                currency: "EUR".to_string(),
                pricing: serde_json::to_value(&Pricing::Slot {
                    unit_rate: Decimal::new(2500, 2),
                })
                .unwrap(),
                tenant_id: ids::TENANT_ID,
                created_by: ids::USER_ID,
            }
            .insert(tx)
            .await?;

            PlanComponentPriceRowNew::insert_batch(
                tx,
                &[PlanComponentPriceRowNew {
                    plan_component_id: ids::COMP_SUPABASE_ORG_SLOTS_ID,
                    price_id: ids::PRICE_SUPABASE_ORG_SLOTS_ID,
                }],
            )
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
                trial_is_free: true,
                uses_product_pricing: true,
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
                trial_is_free: true,
                uses_product_pricing: true,
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
                uses_product_pricing: true,
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
                uses_product_pricing: true,
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

            ProductRowNew {
                id: ids::PRODUCT_PAID_FREE_TRIAL_RATE_ID,
                name: "Monthly Rate".to_string(),
                description: None,
                created_by: ids::USER_ID,
                tenant_id: ids::TENANT_ID,
                product_family_id: ids::PRODUCT_FAMILY_ID,
                fee_type: DieselFeeTypeEnum::Rate,
                fee_structure: serde_json::to_value(&FeeStructure::Rate {}).unwrap(),
            }
            .insert(tx)
            .await?;

            PriceComponentRowNew {
                id: ids::COMP_PAID_FREE_TRIAL_RATE_ID,
                name: "Monthly Rate".to_string(),
                legacy_fee: Some(
                    FeeType::Rate {
                        rates: vec![TermRate {
                            price: rust_decimal::Decimal::new(4900, 2), // $49/month
                            term: BillingPeriodEnum::Monthly,
                        }],
                    }
                    .try_into()
                    .unwrap(),
                ),
                plan_version_id: ids::PLAN_VERSION_PAID_FREE_TRIAL_ID,
                product_id: Some(ids::PRODUCT_PAID_FREE_TRIAL_RATE_ID),
                billable_metric_id: None,
            }
            .insert(tx)
            .await?;

            PriceRowNew {
                id: ids::PRICE_PAID_FREE_TRIAL_RATE_ID,
                product_id: ids::PRODUCT_PAID_FREE_TRIAL_RATE_ID,
                cadence: DieselBillingPeriodEnum::Monthly,
                currency: "EUR".to_string(),
                pricing: serde_json::to_value(&Pricing::Rate {
                    rate: Decimal::new(4900, 2),
                })
                .unwrap(),
                tenant_id: ids::TENANT_ID,
                created_by: ids::USER_ID,
            }
            .insert(tx)
            .await?;

            PlanComponentPriceRowNew::insert_batch(
                tx,
                &[PlanComponentPriceRowNew {
                    plan_component_id: ids::COMP_PAID_FREE_TRIAL_RATE_ID,
                    price_id: ids::PRICE_PAID_FREE_TRIAL_RATE_ID,
                }],
            )
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
                uses_product_pricing: true,
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

            ProductRowNew {
                id: ids::PRODUCT_PAID_TRIAL_RATE_ID,
                name: "Monthly Rate".to_string(),
                description: None,
                created_by: ids::USER_ID,
                tenant_id: ids::TENANT_ID,
                product_family_id: ids::PRODUCT_FAMILY_ID,
                fee_type: DieselFeeTypeEnum::Rate,
                fee_structure: serde_json::to_value(&FeeStructure::Rate {}).unwrap(),
            }
            .insert(tx)
            .await?;

            PriceComponentRowNew {
                id: ids::COMP_PAID_TRIAL_RATE_ID,
                name: "Monthly Rate".to_string(),
                legacy_fee: Some(
                    FeeType::Rate {
                        rates: vec![TermRate {
                            price: rust_decimal::Decimal::new(9900, 2), // $99/month
                            term: BillingPeriodEnum::Monthly,
                        }],
                    }
                    .try_into()
                    .unwrap(),
                ),
                plan_version_id: ids::PLAN_VERSION_PAID_TRIAL_ID,
                product_id: Some(ids::PRODUCT_PAID_TRIAL_RATE_ID),
                billable_metric_id: None,
            }
            .insert(tx)
            .await?;

            PriceRowNew {
                id: ids::PRICE_PAID_TRIAL_RATE_ID,
                product_id: ids::PRODUCT_PAID_TRIAL_RATE_ID,
                cadence: DieselBillingPeriodEnum::Monthly,
                currency: "EUR".to_string(),
                pricing: serde_json::to_value(&Pricing::Rate {
                    rate: Decimal::new(9900, 2),
                })
                .unwrap(),
                tenant_id: ids::TENANT_ID,
                created_by: ids::USER_ID,
            }
            .insert(tx)
            .await?;

            PlanComponentPriceRowNew::insert_batch(
                tx,
                &[PlanComponentPriceRowNew {
                    plan_component_id: ids::COMP_PAID_TRIAL_RATE_ID,
                    price_id: ids::PRICE_PAID_TRIAL_RATE_ID,
                }],
            )
            .await?;

            // === Product-backed plans (Starter & Pro) ===
            // These plans use products + prices (new model) instead of legacy FeeType-only.

            // Shared products across Starter & Pro
            ProductRowNew {
                id: ids::PRODUCT_PLATFORM_FEE_ID,
                name: "Platform Fee".to_string(),
                description: None,
                created_by: ids::USER_ID,
                tenant_id: ids::TENANT_ID,
                product_family_id: ids::PRODUCT_FAMILY_ID,
                fee_type: DieselFeeTypeEnum::Rate,
                fee_structure: serde_json::to_value(&FeeStructure::Rate {}).unwrap(),
            }
            .insert(tx)
            .await?;

            ProductRowNew {
                id: ids::PRODUCT_SEATS_ID,
                name: "Seats".to_string(),
                description: None,
                created_by: ids::USER_ID,
                tenant_id: ids::TENANT_ID,
                product_family_id: ids::PRODUCT_FAMILY_ID,
                fee_type: DieselFeeTypeEnum::Slot,
                fee_structure: serde_json::to_value(&FeeStructure::Slot {
                    unit_name: "seat".to_string(),
                    min_slots: Some(1),
                    max_slots: Some(100),
                    upgrade_policy: UpgradePolicy::Prorated,
                    downgrade_policy: DowngradePolicy::RemoveAtEndOfPeriod,
                })
                .unwrap(),
            }
            .insert(tx)
            .await?;

            // --- Starter Plan ---
            seed_product_backed_plan(
                tx,
                SeedPlan {
                    plan_id: ids::PLAN_STARTER_ID,
                    plan_name: "Starter",
                    plan_version_id: ids::PLAN_VERSION_STARTER_ID,
                    draft_version: None,
                    currency: "EUR",
                    components: vec![
                        SeedComponent::rate(
                            ids::COMP_STARTER_PLATFORM_FEE_ID,
                            "Platform Fee",
                            ids::PRODUCT_PLATFORM_FEE_ID,
                            ids::PRICE_STARTER_PLATFORM_FEE_ID,
                            DieselBillingPeriodEnum::Monthly,
                            Decimal::new(2900, 2),
                        ),
                        SeedComponent::slot(
                            ids::COMP_STARTER_SEATS_ID,
                            "Seats",
                            ids::PRODUCT_SEATS_ID,
                            ids::PRICE_STARTER_SEATS_ID,
                            DieselBillingPeriodEnum::Monthly,
                            Decimal::new(1000, 2),
                        ),
                    ],
                },
            )
            .await?;

            // --- Pro Plan ---
            seed_product_backed_plan(
                tx,
                SeedPlan {
                    plan_id: ids::PLAN_PRO_ID,
                    plan_name: "Pro",
                    plan_version_id: ids::PLAN_VERSION_PRO_ID,
                    draft_version: Some(SeedDraftVersion {
                        id: ids::PLAN_VERSION_PRO_DRAFT_ID,
                        version: 2,
                    }),
                    currency: "EUR",
                    components: vec![
                        SeedComponent::rate(
                            ids::COMP_PRO_PLATFORM_FEE_ID,
                            "Platform Fee",
                            ids::PRODUCT_PLATFORM_FEE_ID,
                            ids::PRICE_PRO_PLATFORM_FEE_ID,
                            DieselBillingPeriodEnum::Monthly,
                            Decimal::new(9900, 2),
                        ),
                        SeedComponent::slot(
                            ids::COMP_PRO_SEATS_ID,
                            "Seats",
                            ids::PRODUCT_SEATS_ID,
                            ids::PRICE_PRO_SEATS_ID,
                            DieselBillingPeriodEnum::Monthly,
                            Decimal::new(2500, 2),
                        ),
                    ],
                },
            )
            .await?;

            // --- USD Plan (for currency mismatch test) ---

            PlanRowNew {
                id: ids::PLAN_USD_ID,
                name: "USD Plan".to_string(),
                description: Some("Plan in USD for currency mismatch test".to_string()),
                created_by: ids::USER_ID,
                tenant_id: ids::TENANT_ID,
                product_family_id: ids::PRODUCT_FAMILY_ID,
                plan_type: PlanTypeEnum::Standard,
                status: PlanStatusEnum::Active,
            }
            .insert(tx)
            .await?;

            PlanVersionRowNew {
                id: ids::PLAN_VERSION_USD_ID,
                is_draft_version: false,
                plan_id: ids::PLAN_USD_ID,
                version: 1,
                trial_duration_days: None,
                tenant_id: ids::TENANT_ID,
                period_start_day: None,
                net_terms: 0,
                currency: "USD".to_string(),
                billing_cycles: None,
                created_by: ids::USER_ID,
                trialing_plan_id: None,
                trial_is_free: true,
                uses_product_pricing: true,
            }
            .insert(tx)
            .await?;

            PlanRowPatch {
                id: ids::PLAN_USD_ID,
                tenant_id: ids::TENANT_ID,
                name: None,
                description: None,
                active_version_id: Some(Some(ids::PLAN_VERSION_USD_ID)),
                draft_version_id: None,
            }
            .update(tx)
            .await?;

            Ok::<(), DatabaseErrorContainer>(())
        }
        .scope_boxed()
    })
    .await
    .unwrap();
}

// --- Helpers for product-backed plan seeds ---

struct SeedDraftVersion {
    id: PlanVersionId,
    version: i32,
}

struct SeedComponent {
    component_id: PriceComponentId,
    name: &'static str,
    product_id: ProductId,
    price_id: PriceId,
    cadence: DieselBillingPeriodEnum,
    legacy_fee: FeeType,
    pricing: Pricing,
}

impl SeedComponent {
    fn rate(
        component_id: PriceComponentId,
        name: &'static str,
        product_id: ProductId,
        price_id: PriceId,
        cadence: DieselBillingPeriodEnum,
        amount: Decimal,
    ) -> Self {
        let term: BillingPeriodEnum = cadence.clone().into();
        Self {
            component_id,
            name,
            product_id,
            price_id,
            cadence,
            legacy_fee: FeeType::Rate {
                rates: vec![TermRate {
                    price: amount,
                    term,
                }],
            },
            pricing: Pricing::Rate { rate: amount },
        }
    }

    fn slot(
        component_id: PriceComponentId,
        name: &'static str,
        product_id: ProductId,
        price_id: PriceId,
        cadence: DieselBillingPeriodEnum,
        unit_rate: Decimal,
    ) -> Self {
        let term: BillingPeriodEnum = cadence.clone().into();
        Self {
            component_id,
            name,
            product_id,
            price_id,
            cadence,
            legacy_fee: FeeType::Slot {
                quota: None,
                rates: vec![TermRate {
                    price: unit_rate,
                    term,
                }],
                slot_unit_name: name.to_lowercase(),
                minimum_count: Some(1),
                upgrade_policy: UpgradePolicy::Prorated,
                downgrade_policy: DowngradePolicy::RemoveAtEndOfPeriod,
            },
            pricing: Pricing::Slot { unit_rate },
        }
    }
}

struct SeedPlan {
    plan_id: PlanId,
    plan_name: &'static str,
    plan_version_id: PlanVersionId,
    draft_version: Option<SeedDraftVersion>,
    currency: &'static str,
    components: Vec<SeedComponent>,
}

async fn seed_product_backed_plan(
    tx: &mut PgConn,
    plan: SeedPlan,
) -> Result<(), DatabaseErrorContainer> {
    PlanRowNew {
        id: plan.plan_id,
        name: plan.plan_name.to_string(),
        description: Some(format!("{} plan with product-backed pricing", plan.plan_name)),
        created_by: ids::USER_ID,
        tenant_id: ids::TENANT_ID,
        product_family_id: ids::PRODUCT_FAMILY_ID,
        plan_type: PlanTypeEnum::Standard,
        status: PlanStatusEnum::Active,
    }
    .insert(tx)
    .await?;

    PlanVersionRowNew {
        id: plan.plan_version_id,
        is_draft_version: false,
        plan_id: plan.plan_id,
        version: 1,
        trial_duration_days: None,
        tenant_id: ids::TENANT_ID,
        period_start_day: None,
        net_terms: 0,
        currency: plan.currency.to_string(),
        billing_cycles: None,
        created_by: ids::USER_ID,
        trialing_plan_id: None,
        trial_is_free: true,
        uses_product_pricing: true,
    }
    .insert(tx)
    .await?;

    if let Some(draft) = &plan.draft_version {
        PlanVersionRowNew {
            id: draft.id,
            is_draft_version: true,
            plan_id: plan.plan_id,
            version: draft.version,
            trial_duration_days: None,
            tenant_id: ids::TENANT_ID,
            period_start_day: None,
            net_terms: 0,
            currency: plan.currency.to_string(),
            billing_cycles: None,
            created_by: ids::USER_ID,
            trialing_plan_id: None,
            trial_is_free: true,
            uses_product_pricing: true,
        }
        .insert(tx)
        .await?;
    }

    PlanRowPatch {
        id: plan.plan_id,
        tenant_id: ids::TENANT_ID,
        name: None,
        description: None,
        active_version_id: Some(Some(plan.plan_version_id)),
        draft_version_id: plan.draft_version.as_ref().map(|d| Some(d.id)),
    }
    .update(tx)
    .await?;

    let mut pcp_links = Vec::new();

    for comp in &plan.components {
        PriceComponentRowNew {
            id: comp.component_id,
            name: comp.name.to_string(),
            legacy_fee: Some(comp.legacy_fee.clone().try_into().unwrap()),
            plan_version_id: plan.plan_version_id,
            product_id: Some(comp.product_id),
            billable_metric_id: None,
        }
        .insert(tx)
        .await?;

        PriceRowNew {
            id: comp.price_id,
            product_id: comp.product_id,
            cadence: comp.cadence.clone(),
            currency: plan.currency.to_string(),
            pricing: serde_json::to_value(&comp.pricing).unwrap(),
            tenant_id: ids::TENANT_ID,
            created_by: ids::USER_ID,
        }
        .insert(tx)
        .await?;

        pcp_links.push(PlanComponentPriceRowNew {
            plan_component_id: comp.component_id,
            price_id: comp.price_id,
        });
    }

    PlanComponentPriceRowNew::insert_batch(tx, &pcp_links).await?;

    Ok(())
}
