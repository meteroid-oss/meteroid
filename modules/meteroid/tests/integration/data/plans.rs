use super::ids;
use common_domain::ids::*;
use diesel_async::AsyncConnection;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::PgConn;
use diesel_models::enums::{
    BillingPeriodEnum as DieselBillingPeriodEnum, FeeTypeEnum as DieselFeeTypeEnum, PlanStatusEnum,
    PlanTypeEnum,
};
use diesel_models::errors::DatabaseErrorContainer;
use diesel_models::plan_component_prices::PlanComponentPriceRowNew;
use diesel_models::plan_versions::PlanVersionRowNew;
use diesel_models::plans::{PlanRowNew, PlanRowPatch};
use diesel_models::price_components::PriceComponentRowNew;
use diesel_models::prices::PriceRowNew;
use diesel_models::products::ProductRowNew;
use meteroid_store::domain::price_components::UsagePricingModel;
use meteroid_store::domain::prices::{FeeStructure, Pricing, UsageModel};
use meteroid_store::domain::{
    BillingPeriodEnum, DowngradePolicy, FeeType, TermRate, UpgradePolicy,
};
use meteroid_store::store::PgPool;
use rust_decimal::Decimal;

pub async fn run_plans_seed(pool: &PgPool) {
    let mut conn = pool
        .get()
        .await
        .expect("couldn't get db connection from pool");

    conn.transaction(|tx| {
        async move {
            // === Shared Product Catalog ===
            seed_product_catalog(tx).await?;

            // === Plans ===

            PlanSeed::new(
                ids::PLAN_LEETCODE_ID,
                "LeetCode",
                ids::PLAN_VERSION_1_LEETCODE_ID,
            )
            .draft(ids::PLAN_VERSION_2_LEETCODE_ID, 2)
            .components(vec![SeedComp::rate(
                ids::COMP_LEETCODE_RATE_ID,
                "Subscription Rate",
                ids::PRODUCT_PLATFORM_FEE_ID,
                ids::PRICE_LEETCODE_RATE_ID,
                DieselBillingPeriodEnum::Monthly,
                Decimal::new(3500, 2),
            )])
            .seed(tx)
            .await?;

            PlanSeed::new(ids::PLAN_NOTION_ID, "Notion", ids::PLAN_VERSION_NOTION_ID)
                .components(vec![SeedComp::slot_multi(
                    ids::COMP_NOTION_SEATS_ID,
                    "Seats",
                    ids::PRODUCT_SEATS_ID,
                    vec![
                        (
                            ids::PRICE_NOTION_SEATS_MONTHLY_ID,
                            DieselBillingPeriodEnum::Monthly,
                            Decimal::new(1000, 2),
                        ),
                        (
                            ids::PRICE_NOTION_SEATS_ANNUAL_ID,
                            DieselBillingPeriodEnum::Annual,
                            Decimal::new(9600, 2),
                        ),
                    ],
                )])
                .seed(tx)
                .await?;

            PlanSeed::new(ids::PLAN_FREE_ID, "Free", ids::PLAN_VERSION_FREE_ID)
                .free()
                .seed(tx)
                .await?;

            PlanSeed::new(
                ids::PLAN_ENTERPRISE_ID,
                "Enterprise",
                ids::PLAN_VERSION_ENTERPRISE_ID,
            )
            .seed(tx)
            .await?;

            PlanSeed::new(
                ids::PLAN_PRO_WITH_TRIAL_ID,
                "Free with Trial",
                ids::PLAN_VERSION_PRO_WITH_TRIAL_ID,
            )
            .free()
            .trial(7, ids::PLAN_ENTERPRISE_ID, true)
            .seed(tx)
            .await?;

            PlanSeed::new(
                ids::PLAN_PAID_FREE_TRIAL_ID,
                "Paid with Free Trial",
                ids::PLAN_VERSION_PAID_FREE_TRIAL_ID,
            )
            .trial(14, ids::PLAN_ENTERPRISE_ID, true)
            .components(vec![SeedComp::rate(
                ids::COMP_PAID_FREE_TRIAL_RATE_ID,
                "Monthly Rate",
                ids::PRODUCT_PLATFORM_FEE_ID,
                ids::PRICE_PAID_FREE_TRIAL_RATE_ID,
                DieselBillingPeriodEnum::Monthly,
                Decimal::new(4900, 2),
            )])
            .seed(tx)
            .await?;

            PlanSeed::new(
                ids::PLAN_PAID_TRIAL_ID,
                "Paid with Paid Trial",
                ids::PLAN_VERSION_PAID_TRIAL_ID,
            )
            .trial(7, ids::PLAN_ENTERPRISE_ID, false)
            .components(vec![SeedComp::rate(
                ids::COMP_PAID_TRIAL_RATE_ID,
                "Monthly Rate",
                ids::PRODUCT_PLATFORM_FEE_ID,
                ids::PRICE_PAID_TRIAL_RATE_ID,
                DieselBillingPeriodEnum::Monthly,
                Decimal::new(9900, 2),
            )])
            .seed(tx)
            .await?;

            PlanSeed::new(
                ids::PLAN_STARTER_ID,
                "Starter",
                ids::PLAN_VERSION_STARTER_ID,
            )
            .components(vec![
                SeedComp::rate(
                    ids::COMP_STARTER_PLATFORM_FEE_ID,
                    "Platform Fee",
                    ids::PRODUCT_PLATFORM_FEE_ID,
                    ids::PRICE_STARTER_PLATFORM_FEE_ID,
                    DieselBillingPeriodEnum::Monthly,
                    Decimal::new(2900, 2),
                ),
                SeedComp::slot(
                    ids::COMP_STARTER_SEATS_ID,
                    "Seats",
                    ids::PRODUCT_SEATS_ID,
                    ids::PRICE_STARTER_SEATS_ID,
                    DieselBillingPeriodEnum::Monthly,
                    Decimal::new(1000, 2),
                ),
            ])
            .seed(tx)
            .await?;

            PlanSeed::new(ids::PLAN_PRO_ID, "Pro", ids::PLAN_VERSION_PRO_ID)
                .draft(ids::PLAN_VERSION_PRO_DRAFT_ID, 2)
                .components(vec![
                    SeedComp::rate(
                        ids::COMP_PRO_PLATFORM_FEE_ID,
                        "Platform Fee",
                        ids::PRODUCT_PLATFORM_FEE_ID,
                        ids::PRICE_PRO_PLATFORM_FEE_ID,
                        DieselBillingPeriodEnum::Monthly,
                        Decimal::new(9900, 2),
                    ),
                    SeedComp::slot(
                        ids::COMP_PRO_SEATS_ID,
                        "Seats",
                        ids::PRODUCT_SEATS_ID,
                        ids::PRICE_PRO_SEATS_ID,
                        DieselBillingPeriodEnum::Monthly,
                        Decimal::new(2500, 2),
                    ),
                ])
                .seed(tx)
                .await?;

            PlanSeed::new(ids::PLAN_USD_ID, "USD Plan", ids::PLAN_VERSION_USD_ID)
                .currency("USD")
                .seed(tx)
                .await?;

            PlanSeed::new(ids::PLAN_USAGE_ID, "Usage Plan", ids::PLAN_VERSION_USAGE_ID)
                .components(vec![
                    SeedComp::rate(
                        ids::COMP_USAGE_RATE_ID,
                        "Platform Fee",
                        ids::PRODUCT_PLATFORM_FEE_ID,
                        ids::PRICE_USAGE_RATE_ID,
                        DieselBillingPeriodEnum::Monthly,
                        Decimal::new(2000, 2),
                    ),
                    SeedComp::usage(
                        ids::COMP_USAGE_BANDWIDTH_ID,
                        "Bandwidth",
                        ids::PRODUCT_BANDWIDTH_ID,
                        ids::METRIC_BANDWIDTH,
                        ids::PRICE_USAGE_BANDWIDTH_ID,
                        DieselBillingPeriodEnum::Monthly,
                        UsagePricingModel::PerUnit {
                            rate: Decimal::new(10, 2),
                        },
                    ),
                ])
                .seed(tx)
                .await?;

            Ok::<(), DatabaseErrorContainer>(())
        }
        .scope_boxed()
    })
    .await
    .unwrap();
}

// ---------------------------------------------------------------------------
// Product catalog
// ---------------------------------------------------------------------------

fn product(
    id: ProductId,
    name: &str,
    fee_type: DieselFeeTypeEnum,
    fee_structure: impl serde::Serialize,
) -> ProductRowNew {
    ProductRowNew {
        id,
        name: name.to_string(),
        description: None,
        created_by: ids::USER_ID,
        tenant_id: ids::TENANT_ID,
        product_family_id: ids::PRODUCT_FAMILY_ID,
        fee_type,
        fee_structure: serde_json::to_value(&fee_structure).unwrap(),
        catalog: true,
    }
}

async fn seed_product_catalog(tx: &mut PgConn) -> Result<(), DatabaseErrorContainer> {
    for p in [
        product(
            ids::PRODUCT_PLATFORM_FEE_ID,
            "Platform Fee",
            DieselFeeTypeEnum::Rate,
            FeeStructure::Rate {},
        ),
        product(
            ids::PRODUCT_SEATS_ID,
            "Seats",
            DieselFeeTypeEnum::Slot,
            FeeStructure::Slot {
                unit_name: "Seats".to_string(),
                upgrade_policy: UpgradePolicy::Prorated,
                downgrade_policy: DowngradePolicy::RemoveAtEndOfPeriod,
            },
        ),
        product(
            ids::PRODUCT_BANDWIDTH_ID,
            "Bandwidth",
            DieselFeeTypeEnum::Usage,
            FeeStructure::Usage {
                metric_id: ids::METRIC_BANDWIDTH,
                model: UsageModel::PerUnit,
            },
        ),
    ] {
        p.insert(tx).await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Plan seed builder
// ---------------------------------------------------------------------------

struct PlanSeed {
    plan_id: PlanId,
    name: &'static str,
    version_id: PlanVersionId,
    ver: i32,
    plan_type: PlanTypeEnum,
    currency: &'static str,
    draft: Option<(PlanVersionId, i32)>,
    trial: Option<(i32, PlanId, bool)>, // (days, trialing_plan_id, is_free)
    components: Vec<SeedComp>,
}

impl PlanSeed {
    fn new(plan_id: PlanId, name: &'static str, version_id: PlanVersionId) -> Self {
        Self {
            plan_id,
            name,
            version_id,
            ver: 1,
            plan_type: PlanTypeEnum::Standard,
            currency: "EUR",
            draft: None,
            trial: None,
            components: vec![],
        }
    }

    fn free(mut self) -> Self {
        self.plan_type = PlanTypeEnum::Free;
        self
    }

    fn currency(mut self, c: &'static str) -> Self {
        self.currency = c;
        self
    }

    fn draft(mut self, id: PlanVersionId, version: i32) -> Self {
        self.draft = Some((id, version));
        self
    }

    fn trial(mut self, days: i32, trialing_plan_id: PlanId, is_free: bool) -> Self {
        self.trial = Some((days, trialing_plan_id, is_free));
        self
    }

    fn components(mut self, c: Vec<SeedComp>) -> Self {
        self.components = c;
        self
    }

    async fn seed(self, tx: &mut PgConn) -> Result<(), DatabaseErrorContainer> {
        PlanRowNew {
            id: self.plan_id,
            name: self.name.to_string(),
            description: None,
            created_by: ids::USER_ID,
            tenant_id: ids::TENANT_ID,
            product_family_id: ids::PRODUCT_FAMILY_ID,
            plan_type: self.plan_type,
            status: PlanStatusEnum::Active,
        }
        .insert(tx)
        .await?;

        let (trial_days, trialing_plan_id, trial_is_free) = match self.trial {
            Some((d, p, f)) => (Some(d), Some(p), f),
            None => (None, None, true),
        };

        PlanVersionRowNew {
            id: self.version_id,
            is_draft_version: false,
            plan_id: self.plan_id,
            version: self.ver,
            trial_duration_days: trial_days,
            tenant_id: ids::TENANT_ID,
            period_start_day: None,
            net_terms: 0,
            currency: self.currency.to_string(),
            billing_cycles: None,
            created_by: ids::USER_ID,
            trialing_plan_id,
            trial_is_free,
            uses_product_pricing: true,
        }
        .insert(tx)
        .await?;

        let mut draft_version_id = None;
        if let Some((draft_id, draft_ver)) = self.draft {
            draft_version_id = Some(Some(draft_id));
            PlanVersionRowNew {
                id: draft_id,
                is_draft_version: true,
                plan_id: self.plan_id,
                version: draft_ver,
                trial_duration_days: trial_days,
                tenant_id: ids::TENANT_ID,
                period_start_day: None,
                net_terms: 0,
                currency: self.currency.to_string(),
                billing_cycles: None,
                created_by: ids::USER_ID,
                trialing_plan_id,
                trial_is_free,
                uses_product_pricing: true,
            }
            .insert(tx)
            .await?;
        }

        PlanRowPatch {
            id: self.plan_id,
            tenant_id: ids::TENANT_ID,
            name: None,
            description: None,
            active_version_id: Some(Some(self.version_id)),
            draft_version_id,
            self_service_rank: None,
        }
        .update(tx)
        .await?;

        let mut pcp_links = Vec::new();
        for comp in &self.components {
            PriceComponentRowNew {
                id: comp.id,
                name: comp.name.to_string(),
                legacy_fee: Some(comp.legacy_fee.clone().try_into().unwrap()),
                plan_version_id: self.version_id,
                product_id: Some(comp.product_id),
                billable_metric_id: comp.billable_metric_id,
            }
            .insert(tx)
            .await?;

            for price in &comp.prices {
                PriceRowNew {
                    id: price.id,
                    product_id: comp.product_id,
                    cadence: price.cadence.clone(),
                    currency: self.currency.to_string(),
                    pricing: serde_json::to_value(&price.pricing).unwrap(),
                    tenant_id: ids::TENANT_ID,
                    created_by: ids::USER_ID,
                    catalog: true,
                }
                .insert(tx)
                .await?;

                pcp_links.push(PlanComponentPriceRowNew {
                    plan_component_id: comp.id,
                    price_id: price.id,
                });
            }
        }
        if !pcp_links.is_empty() {
            PlanComponentPriceRowNew::insert_batch(tx, &pcp_links).await?;
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Component helpers
// ---------------------------------------------------------------------------

struct SeedPrice {
    id: PriceId,
    cadence: DieselBillingPeriodEnum,
    pricing: Pricing,
}

struct SeedComp {
    id: PriceComponentId,
    name: &'static str,
    product_id: ProductId,
    billable_metric_id: Option<BillableMetricId>,
    legacy_fee: FeeType,
    prices: Vec<SeedPrice>,
}

impl SeedComp {
    fn rate(
        id: PriceComponentId,
        name: &'static str,
        product_id: ProductId,
        price_id: PriceId,
        cadence: DieselBillingPeriodEnum,
        amount: Decimal,
    ) -> Self {
        let term: BillingPeriodEnum = cadence.clone().into();
        Self {
            id,
            name,
            product_id,
            billable_metric_id: None,
            legacy_fee: FeeType::Rate {
                rates: vec![TermRate {
                    price: amount,
                    term,
                }],
            },
            prices: vec![SeedPrice {
                id: price_id,
                cadence,
                pricing: Pricing::Rate { rate: amount },
            }],
        }
    }

    fn slot(
        id: PriceComponentId,
        name: &'static str,
        product_id: ProductId,
        price_id: PriceId,
        cadence: DieselBillingPeriodEnum,
        unit_rate: Decimal,
    ) -> Self {
        Self::slot_multi(id, name, product_id, vec![(price_id, cadence, unit_rate)])
    }

    fn slot_multi(
        id: PriceComponentId,
        name: &'static str,
        product_id: ProductId,
        cadences: Vec<(PriceId, DieselBillingPeriodEnum, Decimal)>,
    ) -> Self {
        let rates: Vec<TermRate> = cadences
            .iter()
            .map(|(_, c, r)| TermRate {
                price: *r,
                term: c.clone().into(),
            })
            .collect();

        let prices: Vec<SeedPrice> = cadences
            .into_iter()
            .map(|(pid, c, r)| SeedPrice {
                id: pid,
                cadence: c,
                pricing: Pricing::Slot {
                    unit_rate: r,
                    min_slots: Some(1),
                    max_slots: None,
                },
            })
            .collect();

        Self {
            id,
            name,
            product_id,
            billable_metric_id: None,
            legacy_fee: FeeType::Slot {
                quota: None,
                rates,
                slot_unit_name: name.to_string(),
                minimum_count: Some(1),
                upgrade_policy: UpgradePolicy::Prorated,
                downgrade_policy: DowngradePolicy::RemoveAtEndOfPeriod,
            },
            prices,
        }
    }

    fn usage(
        id: PriceComponentId,
        name: &'static str,
        product_id: ProductId,
        metric_id: BillableMetricId,
        price_id: PriceId,
        cadence: DieselBillingPeriodEnum,
        model: UsagePricingModel,
    ) -> Self {
        let billing_cadence: BillingPeriodEnum = cadence.clone().into();
        Self {
            id,
            name,
            product_id,
            billable_metric_id: Some(metric_id),
            legacy_fee: FeeType::Usage {
                metric_id,
                pricing: model.clone(),
                cadence: billing_cadence,
            },
            prices: vec![SeedPrice {
                id: price_id,
                cadence,
                pricing: Pricing::Usage(model),
            }],
        }
    }
}
