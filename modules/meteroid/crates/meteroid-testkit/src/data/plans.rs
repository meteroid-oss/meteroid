use super::ids;
use common_domain::ids::*;
use diesel_async::AsyncConnection;
use diesel_models::PgConn;
use diesel_models::enums::{
    BillingPeriodEnum as DieselBillingPeriodEnum, PlanStatusEnum, PlanTypeEnum,
};
use diesel_models::errors::DatabaseErrorContainer;
use diesel_models::plan_component_prices::PlanComponentPriceRowNew;
use diesel_models::plan_version_options::PlanVersionOptionRowNew;
use diesel_models::plan_versions::PlanVersionRowNew;
use diesel_models::plans::{PlanRowNew, PlanRowPatch};
use diesel_models::price_components::PriceComponentRowNew;
use diesel_models::prices::PriceRowNew;
use diesel_models::products::ProductRowNew;
use meteroid_store::domain::price_components::CapacityThreshold;
use meteroid_store::domain::prices::{FeeStructure, Pricing, UsageModel};
use meteroid_store::domain::{
    BillingPeriodEnum, BillingType, DowngradePolicy, FeeType, TermRate, UpgradePolicy,
    UsagePricingModel,
};
use meteroid_store::store::PgPool;
use rust_decimal::Decimal;

/// The id of `version_id`'s seeded option of `alias`, derived from both.
pub fn option_id(version_id: PlanVersionId, alias: &str) -> PlanOptionId {
    PlanOptionId::from(uuid::Uuid::new_v5(&version_id.as_uuid(), alias.as_bytes()))
}

/// The option a seeded version starts with, Standard.
pub fn default_option_id(version_id: PlanVersionId) -> PlanOptionId {
    option_id(
        version_id,
        meteroid_store::domain::plan_options::DEFAULT_OPTION_ALIAS,
    )
}

/// Inserts `version_id`'s default option, Standard: every version is sold in at least one.
/// Seeding more components into a version finds the one already there.
pub async fn seed_default_option(
    tx: &mut PgConn,
    tenant_id: TenantId,
    version_id: PlanVersionId,
) -> Result<PlanOptionId, DatabaseErrorContainer> {
    let id = default_option_id(version_id);
    let existing =
        diesel_models::plan_version_options::PlanVersionOptionRow::list_by_plan_version_id(
            tx, tenant_id, version_id,
        )
        .await?;
    if existing.iter().any(|option| option.id == id) {
        return Ok(id);
    }
    PlanVersionOptionRowNew {
        id,
        tenant_id,
        plan_version_id: version_id,
        name: meteroid_store::domain::plan_options::DEFAULT_OPTION_NAME.to_owned(),
        alias: meteroid_store::domain::plan_options::DEFAULT_OPTION_ALIAS.to_owned(),
        position: 0,
        hidden: false,
    }
    .insert(tx)
    .await?;
    Ok(id)
}

/// Inserts an option of `version_id` per `(name, cadence)`, aliased the name in lowercase.
async fn seed_cadence_options(
    tx: &mut PgConn,
    version_id: PlanVersionId,
    options: &[(&'static str, DieselBillingPeriodEnum)],
) -> Result<Vec<(PlanOptionId, BillingPeriodEnum)>, DatabaseErrorContainer> {
    let mut seeded = Vec::with_capacity(options.len());
    for (position, (name, cadence)) in options.iter().enumerate() {
        let alias = name.to_ascii_lowercase();
        let id = option_id(version_id, &alias);
        PlanVersionOptionRowNew {
            id,
            tenant_id: ids::TENANT_ID,
            plan_version_id: version_id,
            name: (*name).to_owned(),
            alias,
            position: position as i32,
            hidden: false,
        }
        .insert(tx)
        .await?;
        seeded.push((id, cadence.clone().into()));
    }
    Ok(seeded)
}

pub async fn run_plans_seed(pool: &PgPool) {
    let mut conn = pool
        .get()
        .await
        .expect("couldn't get db connection from pool");

    conn.transaction(async |tx| {
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

        // Quarterly-billed Rate plan
        PlanSeed::new(
            ids::PLAN_LEETCODE_QUARTERLY_ID,
            "LeetCode Quarterly",
            ids::PLAN_VERSION_LEETCODE_QUARTERLY_ID,
        )
        .components(vec![SeedComp::rate(
            ids::COMP_LEETCODE_QUARTERLY_RATE_ID,
            "Subscription Rate",
            ids::PRODUCT_PLATFORM_FEE_ID,
            ids::PRICE_LEETCODE_QUARTERLY_RATE_ID,
            DieselBillingPeriodEnum::Quarterly,
            Decimal::new(9000, 2),
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

        // Usage Alpha: Rate €10/mo + API Calls at €0.10/unit
        PlanSeed::new(
            ids::PLAN_USAGE_ALPHA_ID,
            "Usage Alpha",
            ids::PLAN_VERSION_USAGE_ALPHA_ID,
        )
        .components(vec![
            SeedComp::rate(
                ids::COMP_USAGE_ALPHA_RATE_ID,
                "Platform Fee",
                ids::PRODUCT_PLATFORM_FEE_ID,
                ids::PRICE_USAGE_ALPHA_RATE_ID,
                DieselBillingPeriodEnum::Monthly,
                Decimal::new(1000, 2),
            ),
            SeedComp::usage(
                ids::COMP_USAGE_ALPHA_API_CALLS_ID,
                "API Calls",
                ids::PRODUCT_API_CALLS_ID,
                ids::METRIC_BANDWIDTH,
                ids::PRICE_USAGE_ALPHA_API_CALLS_ID,
                DieselBillingPeriodEnum::Monthly,
                UsagePricingModel::PerUnit {
                    rate: Decimal::new(10, 2),
                },
            ),
        ])
        .seed(tx)
        .await?;

        // Usage Beta: Rate €20/mo + API Calls at €0.20/unit + DB Storage at €0.50/unit
        PlanSeed::new(
            ids::PLAN_USAGE_BETA_ID,
            "Usage Beta",
            ids::PLAN_VERSION_USAGE_BETA_ID,
        )
        .components(vec![
            SeedComp::rate(
                ids::COMP_USAGE_BETA_RATE_ID,
                "Platform Fee",
                ids::PRODUCT_PLATFORM_FEE_ID,
                ids::PRICE_USAGE_BETA_RATE_ID,
                DieselBillingPeriodEnum::Monthly,
                Decimal::new(2000, 2),
            ),
            SeedComp::usage(
                ids::COMP_USAGE_BETA_API_CALLS_ID,
                "API Calls",
                ids::PRODUCT_API_CALLS_ID,
                ids::METRIC_BANDWIDTH,
                ids::PRICE_USAGE_BETA_API_CALLS_ID,
                DieselBillingPeriodEnum::Monthly,
                UsagePricingModel::PerUnit {
                    rate: Decimal::new(20, 2),
                },
            ),
            SeedComp::usage(
                ids::COMP_USAGE_BETA_DB_STORAGE_ID,
                "DB Storage",
                ids::PRODUCT_DB_STORAGE_ID,
                ids::METRIC_DATABASE_SIZE,
                ids::PRICE_USAGE_BETA_DB_STORAGE_ID,
                DieselBillingPeriodEnum::Monthly,
                UsagePricingModel::PerUnit {
                    rate: Decimal::new(50, 2),
                },
            ),
        ])
        .seed(tx)
        .await?;

        Ok::<(), DatabaseErrorContainer>(())
    })
    .await
    .unwrap();
}

// ---------------------------------------------------------------------------
// Product catalog
// ---------------------------------------------------------------------------

fn product(id: ProductId, name: &str) -> ProductRowNew {
    ProductRowNew {
        id,
        name: name.to_string(),
        description: None,
        tenant_id: ids::TENANT_ID,
        product_family_id: ids::PRODUCT_FAMILY_ID,
        catalog: true,
        tax_category_id: None,
        icon: None,
        color: None,
    }
}

/// How the charges of the shared catalog bill each product: a product holds no structure, its
/// charges do. A seat charge counts the catalog's "Seats" quantity.
pub fn catalog_structure(product_id: ProductId) -> Option<FeeStructure> {
    let structure = match product_id {
        id if id == ids::PRODUCT_PLATFORM_FEE_ID => FeeStructure::Rate {},
        id if id == ids::PRODUCT_SEATS_ID => FeeStructure::Slot {
            unit_name: "Seats".to_string(),
            quantity_id: Some(ids::QUANTITY_SEATS_ID),
            upgrade_policy: UpgradePolicy::Prorated,
            downgrade_policy: DowngradePolicy::RemoveAtEndOfPeriod,
        },
        id if id == ids::PRODUCT_BANDWIDTH_ID || id == ids::PRODUCT_API_CALLS_ID => {
            FeeStructure::Usage {
                metric_id: ids::METRIC_BANDWIDTH,
                model: UsageModel::PerUnit,
            }
        }
        id if id == ids::PRODUCT_DB_STORAGE_ID => FeeStructure::Usage {
            metric_id: ids::METRIC_DATABASE_SIZE,
            model: UsageModel::PerUnit,
        },
        id if id == ids::PRODUCT_CAPACITY_ID => FeeStructure::Capacity {
            metric_id: ids::METRIC_BANDWIDTH,
        },
        id if id == ids::PRODUCT_RECURRING_ARREARS_ID => FeeStructure::ExtraRecurring {
            billing_type: BillingType::Arrears,
        },
        id if id == ids::PRODUCT_ONETIME_SETUP_ID => FeeStructure::OneTime {},
        id if id == ids::PRODUCT_EXTRA_ADVANCE_ID => FeeStructure::ExtraRecurring {
            billing_type: BillingType::Advance,
        },
        _ => return None,
    };
    Some(structure)
}

/// How a fixture's charge of an existing product priced by `pricing` bills it: the catalog's
/// structure, else what the pricing alone tells (a flat fee, a one-time fee, a credit pack).
pub fn charge_structure_of(product_id: ProductId, pricing: &Pricing) -> Option<FeeStructure> {
    catalog_structure(product_id).or(match pricing {
        Pricing::Rate { .. } => Some(FeeStructure::Rate {}),
        Pricing::OneTime { .. } => Some(FeeStructure::OneTime {}),
        Pricing::Credit { .. } => Some(FeeStructure::Credit {}),
        Pricing::ExtraRecurring { .. } => Some(FeeStructure::ExtraRecurring {
            billing_type: BillingType::Advance,
        }),
        Pricing::Slot { .. } | Pricing::Capacity { .. } | Pricing::Usage(_) => None,
    })
}

async fn seed_product_catalog(tx: &mut PgConn) -> Result<(), DatabaseErrorContainer> {
    for p in [
        product(ids::PRODUCT_PLATFORM_FEE_ID, "Platform Fee"),
        product(ids::PRODUCT_SEATS_ID, "Seats"),
        product(ids::PRODUCT_BANDWIDTH_ID, "Bandwidth"),
        product(ids::PRODUCT_API_CALLS_ID, "API Calls"),
        product(ids::PRODUCT_DB_STORAGE_ID, "DB Storage"),
        product(ids::PRODUCT_CAPACITY_ID, "Capacity Bandwidth"),
        product(ids::PRODUCT_RECURRING_ARREARS_ID, "Recurring Arrears"),
        product(ids::PRODUCT_ONETIME_SETUP_ID, "Setup Fee"),
        product(ids::PRODUCT_EXTRA_ADVANCE_ID, "Extra Advance"),
    ] {
        p.insert(tx).await?;
    }
    diesel_models::managed_quantities::ManagedQuantityRow::insert(
        tx,
        &diesel_models::managed_quantities::ManagedQuantityRowNew {
            id: ids::QUANTITY_SEATS_ID,
            tenant_id: ids::TENANT_ID,
            code: "Seats".to_string(),
            name: "Seats".to_string(),
        },
    )
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Plan seed builder
// ---------------------------------------------------------------------------

pub struct PlanSeed {
    plan_id: PlanId,
    name: &'static str,
    version_id: PlanVersionId,
    ver: i32,
    plan_type: PlanTypeEnum,
    currency: &'static str,
    draft: Option<(PlanVersionId, i32)>,
    trial: Option<(i32, PlanId, bool)>, // (days, trialing_plan_id, is_free)
    components: Vec<SeedComp>,
    options: Vec<(&'static str, DieselBillingPeriodEnum)>,
}

impl PlanSeed {
    pub fn new(plan_id: PlanId, name: &'static str, version_id: PlanVersionId) -> Self {
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
            options: vec![],
        }
    }

    /// An option per `(name, cadence)` instead of Standard: a charge priced on several cadences
    /// is priced in each on its cadence, any other the same in every option.
    pub fn options(mut self, options: &[(&'static str, DieselBillingPeriodEnum)]) -> Self {
        self.options = options.to_vec();
        self
    }

    pub fn free(mut self) -> Self {
        self.plan_type = PlanTypeEnum::Free;
        self
    }

    pub fn currency(mut self, c: &'static str) -> Self {
        self.currency = c;
        self
    }

    pub fn draft(mut self, id: PlanVersionId, version: i32) -> Self {
        self.draft = Some((id, version));
        self
    }

    pub fn trial(mut self, days: i32, trialing_plan_id: PlanId, is_free: bool) -> Self {
        self.trial = Some((days, trialing_plan_id, is_free));
        self
    }

    pub fn components(mut self, c: Vec<SeedComp>) -> Self {
        self.components = c;
        self
    }

    pub async fn seed(self, tx: &mut PgConn) -> Result<(), DatabaseErrorContainer> {
        PlanRowNew {
            id: self.plan_id,
            name: self.name.to_string(),
            description: None,
            tenant_id: ids::TENANT_ID,
            product_family_id: ids::PRODUCT_FAMILY_ID,
            plan_type: self.plan_type,
            status: PlanStatusEnum::Active,
        }
        .insert(tx)
        .await?;

        if let Some((days, trial_plan_id, free)) = self.trial {
            // The plan's trial: an offer of its own, as the plan editor writes it (R-153).
            let now = common_clock::now().naive_utc();
            let offer = diesel_models::trial_offers::TrialOfferRow {
                id: common_domain::ids::TrialOfferId::from(self.plan_id.as_uuid()),
                tenant_id: ids::TENANT_ID,
                name: format!("{} trial", self.name),
                code: None,
                duration_days: days,
                free,
                trial_plan_id: Some(trial_plan_id).filter(|id| *id != self.plan_id),
                end_action: "CONVERT".to_string(),
                fallback_plan_version_id: None,
                eligibility_scope: "NONE".to_string(),
                archived_at: None,
                created_at: now,
                updated_at: now,
            }
            .insert(tx)
            .await?;
            diesel_models::trial_offers::TrialOfferRow::set_for_plan(
                tx,
                &ids::TENANT_ID,
                self.plan_id,
                Some(offer.id),
            )
            .await?;
        }

        PlanVersionRowNew {
            id: self.version_id,
            is_draft_version: false,
            plan_id: self.plan_id,
            version: self.ver,
            tenant_id: ids::TENANT_ID,
            period_start_day: None,
            net_terms: 0,
            currency: self.currency.to_string(),
            billing_cycles: None,
            uses_product_pricing: true,
            tax_inclusive: false,
            credit_exhausted_rule: None,
            credit_exhausted_caps: serde_json::json!({}),
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
                tenant_id: ids::TENANT_ID,
                period_start_day: None,
                net_terms: 0,
                currency: self.currency.to_string(),
                billing_cycles: None,
                uses_product_pricing: true,
                tax_inclusive: false,
                credit_exhausted_rule: None,
                credit_exhausted_caps: serde_json::json!({}),
            }
            .insert(tx)
            .await?;
            seed_default_option(tx, ids::TENANT_ID, draft_id).await?;
        }

        PlanRowPatch {
            id: self.plan_id,
            tenant_id: ids::TENANT_ID,
            name: None,
            description: None,
            active_version_id: Some(Some(self.version_id)),
            draft_version_id,
            self_service_rank: None,
            issues_license: None,
        }
        .update(tx)
        .await?;

        Self::seed_components_in(
            tx,
            self.version_id,
            &self.components,
            self.currency,
            &self.options,
        )
        .await?;

        Ok(())
    }

    /// Seed only a new version (+ components) for an already-existing plan.
    pub async fn seed_version(
        tx: &mut PgConn,
        plan_id: PlanId,
        version_id: PlanVersionId,
        version: i32,
        currency: &str,
        components: &[SeedComp],
    ) -> Result<(), DatabaseErrorContainer> {
        PlanVersionRowNew {
            id: version_id,
            is_draft_version: false,
            plan_id,
            version,
            tenant_id: ids::TENANT_ID,
            period_start_day: None,
            net_terms: 0,
            currency: currency.to_string(),
            billing_cycles: None,
            uses_product_pricing: true,
            tax_inclusive: false,
            credit_exhausted_rule: None,
            credit_exhausted_caps: serde_json::json!({}),
        }
        .insert(tx)
        .await?;

        Self::seed_components(tx, version_id, components, currency).await?;

        Ok(())
    }

    pub async fn seed_components(
        tx: &mut PgConn,
        version_id: PlanVersionId,
        components: &[SeedComp],
        currency: &str,
    ) -> Result<(), DatabaseErrorContainer> {
        Self::seed_components_in(tx, version_id, components, currency, &[]).await
    }

    /// `components` in the options `options` seeds (none: Standard).
    pub async fn seed_components_in(
        tx: &mut PgConn,
        version_id: PlanVersionId,
        components: &[SeedComp],
        currency: &str,
        options: &[(&'static str, DieselBillingPeriodEnum)],
    ) -> Result<(), DatabaseErrorContainer> {
        let options = if options.is_empty() {
            vec![(
                seed_default_option(tx, ids::TENANT_ID, version_id).await?,
                BillingPeriodEnum::Monthly,
            )]
        } else {
            seed_cadence_options(tx, version_id, options).await?
        };
        let fixed =
            |pricing: &Pricing| !matches!(pricing, Pricing::Usage(_) | Pricing::OneTime { .. });
        let mut pcp_links = Vec::new();
        for comp in components {
            PriceComponentRowNew {
                id: comp.id,
                name: comp.name.to_string(),
                legacy_fee: (!comp.prices.is_empty())
                    .then(|| comp.legacy_fee.clone().try_into().unwrap()),
                plan_version_id: version_id,
                product_id: Some(comp.product_id),
                billable_metric_id: comp.billable_metric_id,
                // A priced charge bills as its product's catalog structure, else its own.
                fee_structure: comp.structure().map(|structure| {
                    serde_json::to_value(structure).expect("a fee structure serializes")
                }),
            }
            .insert(tx)
            .await?;

            let mut cadences: Vec<BillingPeriodEnum> = comp
                .prices
                .iter()
                .filter(|p| fixed(&p.pricing))
                .map(|p| p.cadence.clone().into())
                .collect();
            cadences.sort();
            cadences.dedup();
            let per_option = options.len() > 1 && cadences.len() > 1;
            for price in &comp.prices {
                PriceRowNew {
                    id: price.id,
                    product_id: comp.product_id,
                    cadence: price.cadence.clone(),
                    currency: currency.to_string(),
                    pricing: serde_json::to_value(&price.pricing).unwrap(),
                    tenant_id: ids::TENANT_ID,
                    catalog: true,
                    credit_unit_id: None,
                    credit_unit_rate: None,
                }
                .insert(tx)
                .await?;

                let own: BillingPeriodEnum = price.cadence.clone().into();
                for (option_id, cadence) in &options {
                    if per_option && fixed(&price.pricing) && *cadence != own {
                        continue;
                    }
                    pcp_links.push(PlanComponentPriceRowNew {
                        plan_component_id: comp.id,
                        price_id: price.id,
                        option_id: *option_id,
                    });
                }
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

pub struct SeedPrice {
    pub id: PriceId,
    pub cadence: DieselBillingPeriodEnum,
    pub pricing: Pricing,
}

pub struct SeedComp {
    pub id: PriceComponentId,
    pub name: &'static str,
    pub product_id: ProductId,
    pub billable_metric_id: Option<BillableMetricId>,
    pub legacy_fee: FeeType,
    pub prices: Vec<SeedPrice>,
    /// How the charge bills its product when not as the catalog does (`with_structure`).
    pub fee_structure: Option<FeeStructure>,
}

impl SeedComp {
    /// How the charge bills its product: none when it includes it with no price; else the
    /// catalog's structure of its product, or the one its v1 fee describes for another product.
    pub fn structure(&self) -> Option<FeeStructure> {
        if self.prices.is_empty() {
            return None;
        }
        if let Some(structure) = &self.fee_structure {
            return Some(structure.clone());
        }
        catalog_structure(self.product_id).or_else(|| {
            Some(meteroid_store::domain::prices::extract_fee_structure(&self.legacy_fee).1)
        })
    }

    /// The charge billing its product as `structure` says: a product charged another way than
    /// the catalog's charges do, or seats counting another quantity.
    pub fn with_structure(mut self, structure: FeeStructure) -> Self {
        self.fee_structure = Some(structure);
        self
    }

    /// A product the plan includes with no price: its features, no fee.
    pub fn included(id: PriceComponentId, name: &'static str, product_id: ProductId) -> Self {
        Self {
            id,
            name,
            product_id,
            billable_metric_id: None,
            legacy_fee: FeeType::Rate { rates: vec![] },
            prices: vec![],
            fee_structure: None,
        }
    }

    pub fn rate(
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
            fee_structure: None,
        }
    }

    pub fn slot(
        id: PriceComponentId,
        name: &'static str,
        product_id: ProductId,
        price_id: PriceId,
        cadence: DieselBillingPeriodEnum,
        unit_rate: Decimal,
    ) -> Self {
        Self::slot_multi(id, name, product_id, vec![(price_id, cadence, unit_rate)])
    }

    pub fn slot_multi(
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
                    model: meteroid_store::domain::price_components::RatingModel::PerUnit {
                        rate: r,
                    },
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
            fee_structure: None,
        }
    }

    pub fn usage(
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
            fee_structure: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn capacity(
        id: PriceComponentId,
        name: &'static str,
        product_id: ProductId,
        metric_id: BillableMetricId,
        price_id: PriceId,
        cadence: DieselBillingPeriodEnum,
        base_rate: Decimal,
        included: u64,
        overage_rate: Decimal,
    ) -> Self {
        let billing_cadence: BillingPeriodEnum = cadence.clone().into();
        Self {
            id,
            name,
            product_id,
            billable_metric_id: Some(metric_id),
            legacy_fee: FeeType::Capacity {
                metric_id,
                thresholds: vec![CapacityThreshold {
                    included_amount: included,
                    price: base_rate,
                    per_unit_overage: overage_rate,
                }],
                cadence: billing_cadence,
            },
            prices: vec![SeedPrice {
                id: price_id,
                cadence,
                pricing: Pricing::Capacity {
                    rate: base_rate,
                    included,
                    overage_rate,
                },
            }],
            fee_structure: None,
        }
    }

    pub fn extra_recurring(
        id: PriceComponentId,
        name: &'static str,
        product_id: ProductId,
        price_id: PriceId,
        cadence: DieselBillingPeriodEnum,
        unit_price: Decimal,
        quantity: u32,
        billing_type: BillingType,
    ) -> Self {
        let billing_cadence: BillingPeriodEnum = cadence.clone().into();
        Self {
            id,
            name,
            product_id,
            billable_metric_id: None,
            legacy_fee: FeeType::ExtraRecurring {
                unit_price,
                quantity,
                billing_type: billing_type.clone(),
                cadence: billing_cadence,
            },
            prices: vec![SeedPrice {
                id: price_id,
                cadence,
                pricing: Pricing::ExtraRecurring {
                    unit_price,
                    quantity,
                    model: None,
                },
            }],
            fee_structure: None,
        }
    }

    pub fn one_time(
        id: PriceComponentId,
        name: &'static str,
        product_id: ProductId,
        price_id: PriceId,
        unit_price: Decimal,
        quantity: u32,
    ) -> Self {
        Self {
            id,
            name,
            product_id,
            billable_metric_id: None,
            legacy_fee: FeeType::OneTime {
                unit_price,
                quantity,
            },
            prices: vec![SeedPrice {
                id: price_id,
                cadence: DieselBillingPeriodEnum::Monthly,
                pricing: Pricing::OneTime {
                    unit_price,
                    quantity,
                    model: None,
                },
            }],
            fee_structure: None,
        }
    }
}
