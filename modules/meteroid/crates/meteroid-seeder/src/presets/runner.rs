use chrono::Days;
use meteroid_store::domain::enums::{
    PlanStatusEnum, SubscriptionActivationCondition, TenantEnvironmentEnum,
};
use std::collections::HashMap;
use std::sync::Arc;

use meteroid_store::repositories::{
    CustomersInterface, PlansInterface, ProductFamilyInterface, QuotesInterface, TenantInterface,
};
use meteroid_store::{Services, domain as store_domain};

use meteroid_store::Store;

use chrono::Utc;

use crate::presets::scenarios;
use crate::presets::scenarios::domain::{FeatureKind, MinimumSpec, UsageProfile};
use crate::presets::scenarios::timeline::Clock;
use crate::presets::{account_credit, credit_traffic, credits, preconditions, replay, usage_wait};
use crate::usage;
use crate::utils::slugify;
use chrono::{Datelike, NaiveDate, NaiveTime};
use common_domain::actor::Actor;
use common_domain::ids::{
    AddOnId, BaseId, CustomerId, FeatureId, OrganizationId, PlanVersionId, ProductId, QuoteId,
    TenantId,
};
use common_utils::decimals::ToSubunit;
use metering_grpc::meteroid::metering::v1::Event;
use meteroid_store::StoreResult;
use meteroid_store::clients::usage::UsageClient;
use meteroid_store::domain::entitlements::{
    EntitlementSpec, EntitlementValue, FeatureNew, FeatureType, OverageBehavior, ResetPeriod,
};
use meteroid_store::domain::plan_minimums::CreateMinimumScope;
use meteroid_store::domain::{BillingPeriodEnum, FullPlan, Period, Tenant};
use meteroid_store::errors::StoreError;
use meteroid_store::hooks::{AnalyticsHook, SeededSandbox, no_analytics_hook};
use meteroid_store::repositories::CreditUnitsInterface;
use meteroid_store::repositories::add_ons::AddOnInterface;
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;
use meteroid_store::repositories::entitlements::EntitlementsInterface;
use meteroid_store::repositories::invoicing_entities::{
    InvoicingEntityInterface, InvoicingEntityInterfaceAuto,
};
use meteroid_store::repositories::sandbox::SandboxInterface;
use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;
use meteroid_store::utils::periods::find_period_containing_date;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
pub async fn run_preset(
    store: &Store,
    services: &Services,
    scenario: scenarios::domain::Scenario,
    organization_id: OrganizationId,
    tenant_name: Option<String>,
    email_delivery_mode: Option<store_domain::enums::EmailDeliveryMode>,
    analytics: Arc<dyn AnalyticsHook>,
) -> StoreResult<Tenant> {
    // TODO tenant archiving. Make sure all apis & more important, processors, do not run

    let now = Utc::now().naive_utc();
    // Refused before the tenant exists, so a sandbox never starts seeding what it cannot finish.
    preconditions::check_scenario(&scenario, now)?;
    let (tenant, invoicing_entity) = create_sandbox_tenant(
        store,
        &scenario,
        organization_id,
        tenant_name,
        email_delivery_mode,
    )
    .await?;

    // Seed the catalog, customers, subscriptions, usage, quotes and demo extras in the
    // background so tenant creation returns immediately; the FE shows a loader driven
    // by the seeding-status signal until it completes.
    store.start_sandbox_seed(tenant.id).await?;
    let store = store.clone();
    let services = services.clone();
    let seeded_tenant = tenant.clone();
    // Seeded history ends where the live worker's traffic starts.
    let as_of = credit_traffic::seed_cutoff(tenant.created_at);
    tokio::spawn(async move {
        let _ = seed_and_record(
            &store,
            &services,
            &seeded_tenant,
            invoicing_entity,
            scenario,
            now.date(),
            as_of,
            analytics.as_ref(),
        )
        .await;
    });

    Ok(tenant)
}

/// `run_preset` seeded in the caller's task, with its history ending at `as_of`, a past moment:
/// returns once seeding has finished, or with why it stopped.
pub async fn run_preset_at(
    store: &Store,
    services: &Services,
    scenario: scenarios::domain::Scenario,
    organization_id: OrganizationId,
    tenant_name: Option<String>,
    as_of: chrono::NaiveDateTime,
) -> StoreResult<Tenant> {
    preconditions::check_scenario(&scenario, as_of)?;
    let as_of = credit_traffic::seed_cutoff(as_of);
    let (tenant, invoicing_entity) =
        create_sandbox_tenant(store, &scenario, organization_id, tenant_name, None).await?;
    store.start_sandbox_seed(tenant.id).await?;
    seed_and_record(
        store,
        services,
        &tenant,
        invoicing_entity,
        scenario,
        as_of.date(),
        as_of,
        no_analytics_hook().as_ref(),
    )
    .await?;
    Ok(tenant)
}

async fn create_sandbox_tenant(
    store: &Store,
    scenario: &scenarios::domain::Scenario,
    organization_id: OrganizationId,
    tenant_name: Option<String>,
    email_delivery_mode: Option<store_domain::enums::EmailDeliveryMode>,
) -> StoreResult<(Tenant, store_domain::InvoicingEntity)> {
    let tenant = store
        .insert_tenant(
            store_domain::TenantNew {
                name: tenant_name.unwrap_or_else(|| scenario.name.clone()),
                environment: TenantEnvironmentEnum::Sandbox,
                email_delivery_mode,
                invoicing_entity: None,
            },
            organization_id,
        )
        .await?;

    log::info!("Created tenant '{}'", &tenant.name);

    // Collect all unique currencies from plans and customers in the scenario
    let mut scenario_currencies: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    for plan in &scenario.plans {
        scenario_currencies.insert(plan.currency.clone());
    }
    for customer in &scenario.customers {
        scenario_currencies.insert(customer.currency.clone());
    }

    // Add scenario currencies to the tenant's available currencies
    let mut available_currencies: Vec<String> = tenant
        .available_currencies
        .iter()
        .filter_map(|c| c.clone())
        .collect();
    for currency in &scenario_currencies {
        if !available_currencies.contains(currency) {
            available_currencies.push(currency.clone());
        }
    }
    store
        .update_tenant_available_currencies(tenant.id, available_currencies)
        .await?;
    let tenant = match &scenario.reporting_currency {
        Some(currency) if *currency != tenant.reporting_currency => {
            store
                .update_tenant(
                    Actor::System,
                    store_domain::TenantUpdate {
                        reporting_currency: Some(currency.clone()),
                        ..Default::default()
                    },
                    organization_id,
                    tenant.id,
                )
                .await?
        }
        _ => tenant,
    };

    let mut invoicing_entity = store.get_invoicing_entity(tenant.id, None).await?;

    // Update invoicing entity with organization details if provided, if it was not configured
    if let Some(ref org) = scenario.organization
        && invoicing_entity.address_line1.is_none()
        && invoicing_entity.vat_number.is_none()
    {
        let patch = store_domain::InvoicingEntityPatch {
            id: invoicing_entity.id,
            vat_number: org.vat_number.clone(),
            address_line1: org.address_line1.clone(),
            city: org.city.clone(),
            zip_code: org.zip_code.clone(),
            invoice_footer_info: org.invoice_footer_info.clone(),
            invoice_footer_legal: org.invoice_footer_legal.clone(),
            invoicing_language: Some(org.invoicing_language.clone()),
            // Drops the FR statutory en-US translation seeded on create, which would
            // otherwise override the scenario's legal footer on English invoices.
            invoice_footer_translations: Some(Default::default()),
            country: org
                .country
                .and_then(common_domain::country::CountryCode::parse_as_opt),
            state: org.state.map(str::to_string),
            tax_resolver: org.tax_resolver.clone(),
            ..Default::default()
        };

        invoicing_entity = store
            .patch_invoicing_entity(Actor::System, patch, tenant.id)
            .await?;
        log::info!("Updated invoicing entity with organization details");
    }
    Ok((tenant, invoicing_entity))
}

/// Seeds the tenant and records how seeding ended; a failure shows the user what is missing while
/// its detail stays in the logs.
#[allow(clippy::too_many_arguments)]
async fn seed_and_record(
    store: &Store,
    services: &Services,
    tenant: &Tenant,
    invoicing_entity: store_domain::InvoicingEntity,
    scenario: scenarios::domain::Scenario,
    now: NaiveDate,
    as_of: chrono::NaiveDateTime,
    analytics: &dyn AnalyticsHook,
) -> StoreResult<()> {
    let mut stage = SeedStage::Catalog;
    let outcome = seed_sandbox_data(
        store,
        services,
        tenant,
        invoicing_entity,
        scenario,
        now,
        as_of,
        analytics,
        &mut stage,
    )
    .await;
    let failure = match &outcome {
        Ok(()) => None,
        Err(e) => {
            log::error!(
                "sandbox seeding failed for tenant {} while creating {stage:?}: {e:?}",
                tenant.id
            );
            Some(stage.failure_reason())
        }
    };
    if let Err(e) = store.finish_sandbox_seed(tenant.id, failure).await {
        log::error!(
            "sandbox seeding status not recorded for tenant {}: {e:?}",
            tenant.id
        );
    }
    outcome
}

/// Where seeding stands, so a failure tells the user what is missing.
#[derive(Clone, Copy, Debug)]
enum SeedStage {
    Catalog,
    Customers,
    Usage,
    BillingHistory,
    Subscriptions,
}

impl SeedStage {
    fn failure_reason(self) -> String {
        let what = match self {
            SeedStage::Catalog => "the sample products and plans",
            SeedStage::Customers => "the sample customers",
            SeedStage::Usage => "the sample usage",
            SeedStage::BillingHistory => "the sample billing history",
            SeedStage::Subscriptions => "the sample subscriptions",
        };
        format!("Seeding stopped while creating {what}. Create a new sandbox to try again.")
    }
}

/// Heavy sandbox seeding, run in the background: catalog, customers, subscriptions,
/// usage backfill, quotes and alerts.
#[allow(clippy::too_many_arguments)]
async fn seed_sandbox_data(
    store: &Store,
    services: &Services,
    tenant: &Tenant,
    invoicing_entity: store_domain::InvoicingEntity,
    scenario: scenarios::domain::Scenario,
    now: NaiveDate,
    as_of: chrono::NaiveDateTime,
    analytics: &dyn AnalyticsHook,
    stage: &mut SeedStage,
) -> StoreResult<()> {
    let product_family = store.find_default_product_family(tenant.id).await?;
    let staged = staged_usage(&scenario, now);

    let mut scenario_currencies: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    for plan in &scenario.plans {
        scenario_currencies.insert(plan.currency.clone());
    }
    for customer in &scenario.customers {
        scenario_currencies.insert(customer.currency.clone());
    }

    let mut credit_units =
        credits::create_credit_units(store, tenant.id, &scenario.credit_units).await?;
    credits::sell_credit_units(
        store,
        tenant.id,
        product_family.id,
        &scenario.credit_units,
        &mut credit_units,
    )
    .await?;
    let unit_ids = credits::unit_ids(&credit_units);
    let mut created_metrics = vec![];

    for metric in scenario.metrics.clone() {
        let unit_label = store_domain::MetricUnitLabel::parse(
            metric.unit_label.as_ref().map(|label| label.singular),
            metric.unit_label.as_ref().map(|label| label.plural),
        )
        .map_err(StoreError::InvalidArgument)?;
        let created = store
            .insert_billable_metric(
                Actor::System,
                store_domain::BillableMetricNew {
                    tenant_id: tenant.id,
                    name: metric.name,
                    code: metric.code,
                    aggregation_type: metric.aggregation_type,
                    aggregation_key: metric.aggregation_key,
                    unit_conversion_factor: metric.unit_conversion_factor,
                    unit_conversion_rounding: metric.unit_conversion_rounding,
                    segmentation_matrix: metric.segmentation_matrix,
                    filters: None,
                    usage_group_key: metric.usage_group_key,
                    unit_label,
                    description: None,
                    product_family_id: product_family.id,
                    product_id: None,
                },
            )
            .await?;

        log::info!("Created metric '{}'", &created.name);
        created_metrics.push(created);
    }

    let mut feature_ids: HashMap<String, FeatureId> = HashMap::new();
    for feature in &scenario.features {
        let (feature_type, entitlement) = match &feature.kind {
            FeatureKind::Metered {
                metric_code,
                unlimited_by_default,
            } => {
                let metric = created_metrics
                    .iter()
                    .find(|m| &m.code == metric_code)
                    .ok_or(StoreError::ValueNotFound(format!(
                        "Metric was not found {metric_code}"
                    )))?;
                let unlimited = unlimited_by_default.then_some(EntitlementValue::Metered {
                    limit: None,
                    reset_period: ResetPeriod::BillingCycle,
                    overage_behavior: OverageBehavior::Block {
                        grace_period_pct: None,
                    },
                    thresholds: vec![],
                    enabled: true,
                    group_by: None,
                });
                (FeatureType::metered(metric.id), unlimited)
            }
            FeatureKind::Boolean { default_enabled } => (
                FeatureType::Boolean,
                Some(EntitlementValue::Boolean {
                    enabled: *default_enabled,
                }),
            ),
        };
        let created = store
            .create_feature(
                Actor::System,
                FeatureNew {
                    tenant_id: tenant.id,
                    product_id: None,
                    name: feature.name.clone(),
                    code: feature.code.clone(),
                    description: None,
                    feature_type,
                    entitlement,
                },
            )
            .await?;
        feature_ids.insert(feature.code.clone(), created.id);
    }

    let mut created_plans = vec![];
    let mut product_by_name: HashMap<String, common_domain::ids::ProductId> = HashMap::new();
    let mut revisions: Vec<PlanRevision> = vec![];

    for plan in scenario.plans.clone() {
        let currency = plan.currency.clone();
        let revision = plan.revision.clone();
        let net_terms = plan.payment.net_terms as i32;
        let entitlements = plan
            .entitlements
            .iter()
            .map(|e| entitlement_spec(e, &feature_ids))
            .collect::<StoreResult<Vec<_>>>()?;

        let mut components: Vec<store_domain::PriceComponentNewInternal> = plan
            .components
            .iter()
            .map(|component| component.to_domain(&created_metrics, &currency, &unit_ids))
            .collect::<Result<Vec<_>, _>>()?;

        // Reuse existing products when a product with the same name was already created
        for comp in &mut components {
            if let store_domain::price_components::ProductRef::New { name, .. } = &comp.product_ref
                && let Some(&pid) = product_by_name.get(name)
            {
                comp.product_ref = store_domain::price_components::ProductRef::Existing(pid);
            }
        }

        let created = store
            .insert_plan(
                Actor::System,
                store_domain::FullPlanNew {
                    plan: store_domain::PlanNew {
                        name: plan.name,
                        plan_type: plan.plan_type,
                        status: PlanStatusEnum::Active,
                        tenant_id: tenant.id,
                        product_family_id: product_family.id,
                        description: None,
                    },
                    version: store_domain::PlanVersionNewInternal {
                        // A minimum is only set on a draft, published right after.
                        is_draft_version: plan.minimum.is_some(),
                        trial: None, // TODO
                        period_start_day: None,
                        net_terms,
                        currency: Some(plan.currency.clone()),
                        billing_cycles: None, // TODO drop
                        entitlements: entitlements.clone(),
                        tax_inclusive: false,
                        credit_exhausted_rule: plan.credit_exhausted_rule.clone(),
                    },
                    price_components: components,
                },
            )
            .await?;

        // Track products by name for reuse in subsequent plans. `insert_plan`
        // returns an empty `products` map, so derive the mapping from the created
        // components, which each carry their `product_id`.
        for comp in &created.price_components {
            if let Some(pid) = comp.product_id {
                product_by_name.entry(comp.name.clone()).or_insert(pid);
            }
        }

        if let Some(minimum) = plan.minimum {
            store
                .set_plan_minimum(
                    created.version.id,
                    tenant.id,
                    subunits(minimum, &currency)?,
                    store_domain::plan_minimums::MinimumScope::AllComponents,
                )
                .await?;
            store
                .publish_plan_version(Actor::System, created.version.id, tenant.id)
                .await?;
        }

        if let Some(rank) = plan.self_service_rank {
            store
                .patch_published_plan(
                    Actor::System,
                    store_domain::PlanPatch {
                        id: created.plan.id,
                        tenant_id: tenant.id,
                        name: None,
                        description: None,
                        active_version_id: None,
                        self_service_rank: Some(Some(rank)),
                        issues_license: None,
                    },
                )
                .await?;
        }

        log::info!("Created plan '{}'", &created.plan.name);
        if let Some(revision) = revision {
            let mut components = revision
                .iter()
                .map(|component| component.to_domain(&created_metrics, &currency, &unit_ids))
                .collect::<Result<Vec<_>, _>>()?;
            for comp in &mut components {
                if let store_domain::price_components::ProductRef::New { name, .. } =
                    &comp.product_ref
                    && let Some(&pid) = product_by_name.get(name)
                {
                    comp.product_ref = store_domain::price_components::ProductRef::Existing(pid);
                }
            }
            revisions.push(PlanRevision {
                credit_exhausted_rule: created.version.credit_exhausted_rule.clone(),
                plan_id: created.plan.id,
                name: created.plan.name.clone(),
                first_version_id: created.version.id,
                currency: currency.clone(),
                net_terms,
                entitlements,
                components,
            });
        }
        created_plans.push(created);
    }

    let plan_map: HashMap<String, FullPlan> =
        HashMap::from_iter(created_plans.into_iter().map(|c| (c.plan.name.clone(), c)));

    // Add-on catalog: one entry per (add-on, currency) so a subscription can attach
    // an add-on priced in its own currency.
    let mut add_on_ids: HashMap<(String, String), AddOnId> = HashMap::new();
    if !scenario.add_ons.is_empty() {
        let currencies: Vec<String> = scenario_currencies.iter().cloned().collect();
        for add_on in &scenario.add_ons {
            for currency in &currencies {
                let created = store
                    .create_add_on_from_ref(
                        add_on.name.clone(),
                        store_domain::price_components::ProductRef::New {
                            name: add_on.name.clone(),
                            fee_type: store_domain::enums::FeeTypeEnum::Rate,
                            fee_structure: store_domain::prices::FeeStructure::Rate {},
                        },
                        store_domain::price_components::PriceEntry::New(
                            store_domain::price_components::PriceInput {
                                cadence: add_on.cadence,
                                currency: currency.clone(),
                                pricing: store_domain::prices::Pricing::Rate { rate: add_on.price },
                                credit_unit: None,
                            },
                        ),
                        None,
                        false,
                        None,
                        tenant.id,
                        product_family.id,
                        vec![],
                    )
                    .await?;
                add_on_ids.insert((add_on.name.clone(), currency.clone()), created.id);
            }
        }
        log::info!("Created {} add-on catalog entries", add_on_ids.len());
    }

    *stage = SeedStage::Customers;
    let mut customers_to_create = vec![];

    for customer in scenario.customers.clone() {
        let earliest_start = match &customer.history {
            Some(history) => {
                let clock = Clock::new(as_of, history.cycle_age_days);
                history
                    .events
                    .iter()
                    .map(|e| clock.resolve(e.at).date())
                    .min()
                    .unwrap_or(now)
            }
            None => customer
                .subscriptions
                .iter()
                .map(|s| s.start_date)
                .min()
                .unwrap_or(now),
        };
        let created_at = earliest_start.checked_sub_days(Days::new(1)).unwrap_or(now);
        let custom_taxes = if customer.tax_rates.is_empty() {
            country_vat_taxes(&customer.billing_address)
        } else {
            customer.tax_rates.clone()
        };

        customers_to_create.push(store_domain::CustomerNew {
            invoicing_entity_id: Some(invoicing_entity.id),
            billing_email: Some(customer.email.clone()),
            invoicing_emails: customer.invoicing_emails.clone(),
            phone: customer.phone.clone(),
            currency: customer.currency,
            billing_address: customer.billing_address.clone(),
            force_created_date: created_at.and_hms_opt(0, 0, 0),
            vat_number: customer.vat_number.clone(),
            legal_number: None,
            buyer_reference: None,
            alias: customer
                .alias
                .clone()
                .or_else(|| Some(slugify(&customer.name))),
            name: customer.name,
            customer_type: store_domain::enums::CustomerType::Company,
            first_name: None,
            last_name: None,
            shipping_address: customer.shipping_address.clone(),
            custom_taxes,
            tax_status: meteroid_store::domain::CustomerTaxStatus::Taxable,
            exemption_reason: None,
            connected_account_id: None,
            custom_properties: serde_json::Value::Null,
            preferred_locales: Vec::new(),
        });
    }

    let created_customers = store
        .insert_customer_batch(Actor::System, customers_to_create, tenant.id)
        .await?;

    let customer_map: HashMap<String, CustomerId> = HashMap::from_iter(
        created_customers
            .into_iter()
            .map(|c| (c.name.clone(), c.id)),
    );

    // Capture each customer's active window before `scenario.customers` is consumed below. A
    // customer starting later has no usage yet.
    let customer_windows: HashMap<String, (NaiveDate, NaiveDate)> = scenario
        .customers
        .iter()
        .map(|c| (c.name.clone(), customer_active_window(c, now)))
        .filter(|(_, (start, _))| *start <= now)
        .collect();

    let quote_specs = scenario.quotes.clone();

    *stage = SeedStage::Usage;
    // Ingest synthetic usage BEFORE creating subscriptions: the backdated
    // subscriptions' catch-up bills arrears usage from a live ClickHouse query, so
    // the events must already be materialized or every historical usage line
    // computes to zero. Awaited (with a bounded wait), not spawned.
    if let Some(program) = &scenario.credits {
        let metric = created_metrics
            .iter()
            .find(|m| m.code == program.usage_metric_code)
            .ok_or(StoreError::ValueNotFound(format!(
                "Metric was not found {}",
                program.usage_metric_code
            )))?;
        let connector_id = credits::connect_gateway(store, tenant.id, &invoicing_entity).await?;
        credits::connect_bank_account(store, tenant.id, &invoicing_entity, &scenario.plans).await?;
        let usage = credits::history_usage(
            &scenario.customers,
            &scenario.plans,
            &scenario.credit_units,
            &customer_map,
            as_of,
        )?;
        credits::ingest_history_usage(services, tenant, metric, &usage).await?;
        *stage = SeedStage::BillingHistory;
        replay::replay_history(replay::HistoryReplay {
            store,
            services,
            tenant_id: tenant.id,
            scenario: &scenario,
            plans: &plan_map,
            credit_units: &credit_units,
            connector_id,
            customers: &customer_map,
            usage: &usage,
        })
        .await?;
    } else {
        backfill_usage(
            services,
            tenant,
            &created_metrics,
            &customer_map,
            &customer_windows,
            &staged,
            now,
        )
        .await;
    }

    *stage = SeedStage::Subscriptions;
    let mut subscriptions_to_create = vec![];
    // Subscriptions that should churn are created open-ended and then cancelled
    // with a backdated effective date after insertion, so they emit a real Churn
    // MRR movement. A plain `end_date` only completes them, without any churn.
    let mut cancellations: HashMap<(CustomerId, PlanVersionId, NaiveDate), NaiveDate> =
        HashMap::new();

    for customer in &scenario.customers {
        let customer_id = customer_map
            .get(&customer.name)
            .ok_or(StoreError::ValueNotFound(format!(
                "Customer was not found : {}",
                &customer.name
            )))?;

        for sub in &customer.subscriptions {
            let plan = plan_map
                .get(&sub.plan_name)
                .ok_or(StoreError::ValueNotFound(format!(
                    "Plan was not found : {}",
                    &sub.plan_name
                )))?;

            if let Some(end) = sub.end_date {
                cancellations.insert((*customer_id, plan.version.id, sub.start_date), end);
            }

            let subscription = store_domain::SubscriptionNew {
                customer_id: *customer_id,
                activation_condition: SubscriptionActivationCondition::OnStart,
                trial_duration: sub.trial_days,
                billing_day_anchor: None,
                plan_version_id: plan.version.id,
                net_terms: None,
                invoice_memo: None,
                invoice_threshold: None,
                start_date: sub.start_date,
                end_date: None,
                payment_methods_config: None,
                billing_start_date: None,
                auto_advance_invoices: true,
                charge_automatically: false,
                purchase_order: None,
                backdate_invoices: true,
                skip_checkout_session: false,
                skip_past_invoices: false,
                usage_filter: None,
                custom_properties: serde_json::Value::Null,
            };

            let parameterized_components = sub
                .seats
                .and_then(|seats| {
                    plan.price_components
                        .iter()
                        .find(|c| c.name.eq_ignore_ascii_case("Seats"))
                        .map(
                            |c| store_domain::subscription_components::ComponentParameterization {
                                component_id: c.id,
                                parameters:
                                    store_domain::subscription_components::ComponentParameters {
                                        initial_slot_count: Some(seats),
                                        billing_period: None,
                                        committed_capacity: None,
                                    },
                            },
                        )
                })
                .into_iter()
                .collect::<Vec<_>>();
            let overridden_components = sub
                .overrides
                .iter()
                .map(|component| {
                    component_override(
                        component,
                        plan,
                        &created_metrics,
                        &customer.currency,
                        &unit_ids,
                    )
                })
                .collect::<StoreResult<Vec<_>>>()?;
            let price_components = (!parameterized_components.is_empty()
                || !overridden_components.is_empty())
            .then_some(
                store_domain::subscription_components::CreateSubscriptionComponents {
                    parameterized_components,
                    overridden_components,
                    extra_components: vec![],
                    remove_components: vec![],
                },
            );

            let add_ons = {
                let items: Vec<store_domain::subscription_add_ons::CreateSubscriptionAddOn> = sub
                    .add_ons
                    .iter()
                    .filter_map(|r| {
                        add_on_ids
                            .get(&(r.name.clone(), customer.currency.clone()))
                            .map(|id| store_domain::subscription_add_ons::CreateSubscriptionAddOn {
                                add_on_id: *id,
                                customization:
                                    store_domain::subscription_add_ons::SubscriptionAddOnCustomization::None,
                                quantity: r.quantity as i32,
                            })
                    })
                    .collect();
                (!items.is_empty()).then_some(
                    store_domain::subscription_add_ons::CreateSubscriptionAddOns { add_ons: items },
                )
            };

            let minimum_override = sub
                .minimum
                .as_ref()
                .map(|m| minimum_override(m, &customer.currency, &product_by_name))
                .transpose()?;

            subscriptions_to_create.push(store_domain::CreateSubscription {
                subscription,
                price_components,
                add_ons,
                coupons: None,
                entitlements: vec![],
                minimum_override,
                drop_minimum: false,
            });
        }
    }

    let created_subs = services
        .insert_subscription_batch(Actor::System, subscriptions_to_create, tenant.id)
        .await?;

    // Cancel the scenario's ended subscriptions on their end date: a backdated one emits
    // a real Churn MRR movement, a future one stays a pending scheduled cancellation.
    for cs in &created_subs {
        if let Some(end) = cancellations.get(&(cs.customer_id, cs.plan_version_id, cs.start_date)) {
            services
                .cancel_subscription(
                    Actor::System,
                    cs.id,
                    tenant.id,
                    Some("Churned (sandbox seed)".to_string()),
                    CancellationEffectiveAt::Date(*end),
                )
                .await?;
        }
    }
    // After the subscriptions, whose first invoice would otherwise count on credit dated after it.
    if let Err(e) = account_credit::seed_account_credit(&account_credit::AccountCredit {
        store,
        services,
        tenant_id: tenant.id,
        scenario: &scenario,
        credit_units: &credit_units,
        customers: &customer_map,
        as_of,
    })
    .await
    {
        log::warn!(
            "sandbox seed: customer credit failed for tenant {}: {e:?}",
            tenant.id
        );
    }
    // Signed ahead of their start and cancelled with notice, as a real book reads: a target's
    // past days then show what billing knew before it happened.
    if let Err(e) = store.backdate_sandbox_signatures(tenant.id).await {
        log::warn!(
            "sandbox seed: backdating signatures failed for tenant {}: {e:?}",
            tenant.id
        );
    }

    // Quotes and the edition's demo extras are best effort: a failure must not fail
    // tenant creation.
    if let Err(e) = seed_quotes(
        store,
        services,
        tenant.id,
        &quote_specs,
        &scenario.customers,
        &customer_map,
        &plan_map,
    )
    .await
    {
        log::warn!(
            "sandbox seed: quotes failed for tenant {}: {e:?}",
            tenant.id
        );
    }
    analytics
        .sandbox_seeded(
            store,
            &SeededSandbox {
                tenant_id: tenant.id,
                plans: &plan_map,
                metrics: &created_metrics,
            },
        )
        .await;
    // Revisions come last, so the subscriptions and quotes keep the version they were made on.
    if let Err(e) = publish_revisions(store, tenant.id, revisions, &add_on_ids).await {
        log::warn!(
            "sandbox seed: plan revisions failed for tenant {}: {e:?}",
            tenant.id
        );
    }

    // The seeded history is back-dated, so analytics catch up on it now rather than never.
    // `from` is clamped so the re-derive never rewinds more than ~366 days.
    let earliest = customer_windows
        .values()
        .map(|(start, _)| *start)
        .min()
        .unwrap_or(now);
    let floor = now.checked_sub_days(Days::new(366)).unwrap_or(now);
    analytics
        .tenant_history_written(store, tenant.id, earliest.max(floor))
        .await;

    // Targets are seeded by the sandbox warmup, once analytics has the tenant's history: they are
    // set against the actuals of past months.

    // Past invoices are generated and settled asynchronously by the billing and
    // sandbox-maintenance workers, not here.

    Ok(())
}

/// The plan component named like `component`, priced as `component` for one subscription.
fn component_override(
    component: &scenarios::domain::PriceComponent,
    plan: &FullPlan,
    metrics: &[store_domain::BillableMetric],
    currency: &str,
    credit_units: &HashMap<String, common_domain::ids::CreditUnitId>,
) -> StoreResult<store_domain::subscription_components::ComponentOverride> {
    let replaced = plan
        .price_components
        .iter()
        .find(|c| c.name == component.name)
        .ok_or_else(|| {
            StoreError::ValueNotFound(format!(
                "Plan {} has no component {}",
                plan.plan.name, component.name
            ))
        })?;
    let price_entry = component
        .to_domain(metrics, currency, credit_units)?
        .prices
        .into_iter()
        .next()
        .ok_or_else(|| {
            StoreError::InvalidArgument(format!("{} override has no price", component.name))
        })?;
    Ok(store_domain::subscription_components::ComponentOverride {
        component_id: replaced.id,
        name: component.name.clone(),
        price_entry,
    })
}

/// A destination-country VAT line for a customer, applied via the default Manual
/// tax resolver. Returns empty (no tax) for non-VAT countries like the US.
fn country_vat_taxes(
    billing_address: &Option<store_domain::Address>,
) -> Vec<store_domain::CustomerTaxRate> {
    let Some(country) = billing_address.as_ref().and_then(|a| a.country.as_ref()) else {
        return vec![];
    };
    let rate = match country.to_string().as_str() {
        "FR" => dec!(0.20),
        "DE" => dec!(0.19),
        "GB" => dec!(0.20),
        "ES" => dec!(0.21),
        "IT" => dec!(0.22),
        "NL" => dec!(0.21),
        "BE" => dec!(0.21),
        "IE" => dec!(0.23),
        "AT" => dec!(0.20),
        "SE" => dec!(0.25),
        "FI" => dec!(0.24),
        "DK" => dec!(0.25),
        "PL" => dec!(0.23),
        "PT" => dec!(0.23),
        _ => return vec![],
    };
    vec![store_domain::CustomerTaxRate {
        tax_code: "VAT".to_string(),
        name: "VAT".to_string(),
        rate,
    }]
}

/// A customer's usage window: earliest subscription start to the latest end
/// (capped at today), or today if any subscription is still open-ended.
fn customer_active_window(
    customer: &scenarios::domain::Customer,
    today: NaiveDate,
) -> (NaiveDate, NaiveDate) {
    let start = customer
        .subscriptions
        .iter()
        .map(|s| s.start_date)
        .min()
        .unwrap_or(today);
    let end = if customer.subscriptions.iter().any(|s| s.end_date.is_none()) {
        today
    } else {
        customer
            .subscriptions
            .iter()
            .filter_map(|s| s.end_date)
            .max()
            .unwrap_or(today)
            .min(today)
    };
    (start, end.max(start))
}

/// Synthetic usage backfill. Generation is synchronous and fast; ingestion is
/// awaited (not spawned) and followed by a bounded wait for the events to become
/// queryable in ClickHouse. The subscriptions created next bill their backdated
/// arrears usage from a live query, so if the events aren't materialized yet every
/// historical usage line silently computes to zero. Degrades gracefully: if the
/// metering pipeline is unavailable we give up after the cap and let usage stay
/// empty rather than blocking tenant creation indefinitely.
async fn backfill_usage(
    services: &Services,
    tenant: &Tenant,
    metrics: &[store_domain::BillableMetric],
    customer_map: &HashMap<String, CustomerId>,
    windows: &HashMap<String, (NaiveDate, NaiveDate)>,
    staged: &HashMap<String, StagedUsage>,
    today: NaiveDate,
) {
    use store_domain::BillingMetricAggregateEnum as Agg;
    let tenant_id = tenant.id;

    let mut events: Vec<Contribution> = Vec::new();
    for metric in metrics {
        let value_key = metric
            .aggregation_key
            .clone()
            .unwrap_or_else(|| "value".to_string());
        let is_gauge = matches!(
            metric.aggregation_type,
            Agg::Max | Agg::Min | Agg::Mean | Agg::Latest
        );
        let stream = usage::stream_for(&metric.code, &value_key, is_gauge, 18);
        for (name, id) in customer_map {
            let Some((start, end)) = windows.get(name) else {
                continue;
            };
            let customer = usage::UsageCustomer {
                customer_id: *id,
                scale: usage::scale_for(*id),
                start: *start,
                end: (*end).min(today),
                seed: usage::seed_for(*id, &stream.metric_code),
            };
            events.extend(
                customer_events(&stream, &customer, staged.get(name), today)
                    .into_iter()
                    .map(|e| Contribution {
                        customer_id: e.customer_id,
                        day: e.timestamp.date(),
                        value: match metric.aggregation_type {
                            Agg::Count => Decimal::ONE,
                            _ => e
                                .properties
                                .iter()
                                .find(|(k, _)| *k == value_key)
                                .and_then(|(_, v)| v.parse().ok())
                                .unwrap_or_default(),
                        },
                        event: usage::to_ingest_event(e),
                    }),
            );
        }
    }

    if events.is_empty() {
        return;
    }

    let client = services.usage_clients();
    log::info!(
        "sandbox usage backfill: generated {} events for tenant {tenant_id}",
        events.len()
    );
    let ingested: Vec<Event> = events.iter().map(|c| c.event.clone()).collect();
    usage::ingest_all(client.clone(), tenant_id, tenant.keep_last_events, ingested).await;
    // Best-effort, as the backfill is: a missing metering pipeline must not block the tenant.
    if let Err(short) = await_usage_materialized(
        client.as_ref(),
        tenant_id,
        &expected_totals(metrics, &events, today),
        usage_wait::DEMO_USAGE_WAIT,
    )
    .await
    {
        log::warn!(
            "sandbox usage backfill: usage not fully queryable for tenant {tenant_id}; \
             historical usage lines may be short: {}",
            short.join("; ")
        );
    }
}

/// A generated event with what it adds to its metric's total.
struct Contribution {
    customer_id: CustomerId,
    day: NaiveDate,
    value: Decimal,
    event: Event,
}

/// Per customer and summable metric, what its generated events add up to.
fn expected_totals(
    metrics: &[store_domain::BillableMetric],
    events: &[Contribution],
    today: NaiveDate,
) -> Vec<ExpectedUsage> {
    use store_domain::BillingMetricAggregateEnum as Agg;

    let mut totals: HashMap<(CustomerId, String), (Decimal, NaiveDate)> = HashMap::new();
    for c in events {
        let entry = totals
            .entry((c.customer_id, c.event.code.clone()))
            .or_insert((Decimal::ZERO, c.day));
        entry.0 += c.value;
        entry.1 = entry.1.min(c.day);
    }
    totals
        .into_iter()
        .filter_map(|((customer_id, code), (total, first))| {
            let metric = metrics.iter().find(|m| {
                m.code == code
                    && m.segmentation_matrix.is_none()
                    && m.filters.is_none()
                    && matches!(m.aggregation_type, Agg::Sum | Agg::Count)
            })?;
            Some(ExpectedUsage {
                customer_id,
                metric: metric.clone(),
                period: store_domain::UsagePeriod {
                    start: first.and_time(NaiveTime::MIN),
                    end: (today + Days::new(1)).and_time(NaiveTime::MIN),
                },
                total,
            })
        })
        .collect()
}

/// A staged customer's usage, resolved against its current billing cycle.
#[derive(Clone, Debug, PartialEq)]
enum StagedUsage {
    None,
    /// Generated `metric_code` events stop at the cycle start; the cycle carries `target` units.
    Target {
        metric_code: String,
        cycle: Period,
        target: Decimal,
    },
    /// Keeps every `keep_every`-th generated event inside the cycle.
    Collapse {
        cycle: Period,
        keep_every: usize,
    },
}

fn staged_usage(
    scenario: &scenarios::domain::Scenario,
    today: NaiveDate,
) -> HashMap<String, StagedUsage> {
    scenario
        .customers
        .iter()
        .filter(|c| c.has_staged_usage())
        .filter_map(|c| {
            let resolved = resolve_staged_usage(scenario, c, today);
            if resolved.is_none() {
                log::warn!(
                    "sandbox seed: staged usage of '{}' does not resolve; generating it instead",
                    c.name
                );
            }
            resolved.map(|r| (c.name.clone(), r))
        })
        .collect()
}

fn resolve_staged_usage(
    scenario: &scenarios::domain::Scenario,
    customer: &scenarios::domain::Customer,
    today: NaiveDate,
) -> Option<StagedUsage> {
    let current_cycle = || {
        let sub = customer.current_subscription(today)?;
        Some(find_period_containing_date(
            sub.start_date,
            today,
            &BillingPeriodEnum::Monthly,
            sub.start_date.day(),
        ))
    };
    match &customer.usage {
        UsageProfile::Generated => None,
        UsageProfile::None => Some(StagedUsage::None),
        UsageProfile::CycleUsage {
            metric_code,
            pct_of_limit,
        } => {
            let plan_name = &customer.current_subscription(today)?.plan_name;
            let plan = scenario.plans.iter().find(|p| &p.name == plan_name)?;
            let limit = scenario
                .features
                .iter()
                .filter(|f| f.metric_code() == Some(metric_code.as_str()))
                .find_map(|f| plan.entitlement(&f.code))?
                .limit()?;
            Some(StagedUsage::Target {
                metric_code: metric_code.clone(),
                cycle: current_cycle()?,
                target: limit * Decimal::from(*pct_of_limit) / Decimal::ONE_HUNDRED,
            })
        }
        UsageProfile::Collapse { keep_ratio } => Some(StagedUsage::Collapse {
            cycle: current_cycle()?,
            keep_every: (1.0 / keep_ratio).round().max(1.0) as usize,
        }),
    }
}

/// One customer's events for a metric stream, reshaped by its staged usage.
fn customer_events(
    stream: &usage::UsageStream,
    customer: &usage::UsageCustomer,
    staged: Option<&StagedUsage>,
    today: NaiveDate,
) -> Vec<usage::GeneratedEvent> {
    let generated = || usage::generate_stream(stream, std::slice::from_ref(customer));
    match staged {
        None => generated(),
        Some(StagedUsage::None) => vec![],
        Some(StagedUsage::Target {
            metric_code,
            cycle,
            target,
        }) if *metric_code == stream.metric_code => {
            let mut events: Vec<_> = generated()
                .into_iter()
                .filter(|e| e.timestamp.date() < cycle.start)
                .collect();
            events.extend(cycle_target_events(
                stream,
                customer,
                cycle.start,
                *target,
                today,
            ));
            events
        }
        Some(StagedUsage::Target { .. }) => generated(),
        Some(StagedUsage::Collapse { cycle, keep_every }) => {
            let mut in_cycle = 0usize;
            generated()
                .into_iter()
                .filter(|e| {
                    if e.timestamp.date() < cycle.start {
                        return true;
                    }
                    in_cycle += 1;
                    (in_cycle - 1).is_multiple_of(*keep_every)
                })
                .collect()
        }
    }
}

/// One midday event per elapsed day of the cycle, summing exactly to `target`; a cycle
/// starting today gets a single event at midnight so it is already counted.
fn cycle_target_events(
    stream: &usage::UsageStream,
    customer: &usage::UsageCustomer,
    cycle_start: NaiveDate,
    target: Decimal,
    today: NaiveDate,
) -> Vec<usage::GeneratedEvent> {
    let elapsed: Vec<NaiveDate> = cycle_start.iter_days().take_while(|d| *d < today).collect();
    let (days, time) = if elapsed.is_empty() {
        (vec![cycle_start], NaiveTime::MIN)
    } else {
        (
            elapsed,
            NaiveTime::from_hms_opt(12, 0, 0).expect("valid midday"),
        )
    };
    let per_day = (target / Decimal::from(days.len())).floor();
    let last = days.len() - 1;
    days.into_iter()
        .enumerate()
        .map(|(i, day)| {
            let value = if i == last {
                target - per_day * Decimal::from(last)
            } else {
                per_day
            };
            usage::GeneratedEvent {
                id: format!(
                    "sbx-{}-{:016x}-cycle-{i}",
                    stream.metric_code, customer.seed
                ),
                code: stream.metric_code.clone(),
                customer_id: customer.customer_id,
                timestamp: day.and_time(time),
                properties: vec![(
                    stream.value_key().to_string(),
                    value.normalize().to_string(),
                )],
            }
        })
        .collect()
}

fn entitlement_spec(
    entitlement: &scenarios::domain::PlanEntitlementSpec,
    feature_ids: &HashMap<String, FeatureId>,
) -> StoreResult<EntitlementSpec> {
    use scenarios::domain::PlanEntitlementSpec;

    let feature_id =
        feature_ids
            .get(entitlement.feature_code())
            .copied()
            .ok_or(StoreError::ValueNotFound(format!(
                "Feature was not found {}",
                entitlement.feature_code()
            )))?;
    let value = match entitlement {
        PlanEntitlementSpec::Metered {
            limit,
            warning_pct,
            block,
            ..
        } => EntitlementValue::Metered {
            limit: Some(*limit),
            reset_period: ResetPeriod::BillingCycle,
            overage_behavior: match block {
                Some(block) => OverageBehavior::Block {
                    grace_period_pct: block.grace_pct,
                },
                None => OverageBehavior::Allow,
            },
            thresholds: vec![*warning_pct],
            enabled: true,
            group_by: None,
        },
        PlanEntitlementSpec::Boolean { enabled, .. } => {
            EntitlementValue::Boolean { enabled: *enabled }
        }
    };
    Ok(EntitlementSpec { feature_id, value })
}

fn subunits(amount: Decimal, currency: &str) -> StoreResult<i64> {
    let precision = meteroid_money::iso::find(currency)
        .map(|c| c.exponent as u8)
        .ok_or(StoreError::InvalidArgument(format!(
            "unknown currency {currency}"
        )))?;
    amount
        .to_subunit_opt(precision)
        .ok_or(StoreError::InvalidDecimal.into())
}

/// Scoped to the named product when the catalog has it, else to every component.
fn minimum_override(
    minimum: &MinimumSpec,
    currency: &str,
    product_by_name: &HashMap<String, ProductId>,
) -> StoreResult<(i64, CreateMinimumScope)> {
    let precision = meteroid_money::iso::find(currency)
        .map(|c| c.exponent as u8)
        .unwrap_or(2);
    let amount = minimum
        .amount
        .to_subunit_opt(precision)
        .ok_or(StoreError::InvalidDecimal)?;
    let scope = match minimum
        .product_name
        .as_ref()
        .and_then(|name| product_by_name.get(name))
    {
        Some(product_id) => CreateMinimumScope::Products {
            product_ids: vec![*product_id],
            extra_product_names: vec![],
        },
        None => CreateMinimumScope::AllComponents,
    };
    Ok((amount, scope))
}

/// What a customer's ingested usage of a metric must add up to over a period.
pub(super) struct ExpectedUsage {
    pub customer_id: CustomerId,
    pub metric: store_domain::BillableMetric,
    pub period: store_domain::UsagePeriod,
    pub total: Decimal,
}

/// Waits, at most `within`, until every expected total is queryable, so the billing that
/// follows reads all of the ingested usage. Returns what is still short.
pub(super) async fn await_usage_materialized(
    client: &dyn UsageClient,
    tenant_id: TenantId,
    expected: &[ExpectedUsage],
    within: std::time::Duration,
) -> Result<(), Vec<String>> {
    usage_wait::await_totals(
        expected,
        within,
        |want| want.total,
        |want| format!("{} {}", want.customer_id, want.metric.code),
        |want| {
            client.fetch_total_usage(
                &tenant_id,
                &want.customer_id,
                &want.metric,
                want.period.clone(),
            )
        },
    )
    .await
}

/// A plan version published after the scenario's subscriptions.
struct PlanRevision {
    credit_exhausted_rule: Option<meteroid_store::domain::credit_exhausted::CreditExhaustedRule>,
    plan_id: common_domain::ids::PlanId,
    name: String,
    first_version_id: PlanVersionId,
    currency: String,
    net_terms: i32,
    entitlements: Vec<meteroid_store::domain::entitlements::EntitlementSpec>,
    components: Vec<store_domain::PriceComponentNewInternal>,
}

/// Publishes each revision as its plan's active version; add-ons of its currency stay attachable.
async fn publish_revisions(
    store: &Store,
    tenant_id: TenantId,
    revisions: Vec<PlanRevision>,
    add_on_ids: &HashMap<(String, String), AddOnId>,
) -> StoreResult<()> {
    for revision in revisions {
        let attachments = add_on_ids
            .iter()
            .filter(|((_, currency), _)| *currency == revision.currency)
            .map(
                |(_, add_on_id)| store_domain::plan_version_add_ons::PlanVersionAddOnNew {
                    plan_version_id: revision.first_version_id,
                    add_on_id: *add_on_id,
                    price_id: None,
                    self_serviceable: None,
                    max_instances_per_subscription: None,
                    tenant_id,
                },
            )
            .collect();
        store
            .replace_plan_version(
                Actor::System,
                revision.plan_id,
                tenant_id,
                revision.name.clone(),
                None,
                store_domain::PlanVersionNewInternal {
                    is_draft_version: false,
                    trial: None,
                    period_start_day: None,
                    net_terms: revision.net_terms,
                    currency: Some(revision.currency),
                    billing_cycles: None,
                    entitlements: revision.entitlements,
                    tax_inclusive: false,
                    credit_exhausted_rule: revision.credit_exhausted_rule,
                },
                revision.components,
                attachments,
                true,
                None,
            )
            .await?;
        log::info!("Published a new version of plan '{}'", revision.name);
    }
    Ok(())
}

/// Seed quotes from the scenario: publish each, and for `convert` specs also accept
/// and convert it into a subscription. The plan is re-fetched fully hydrated because
/// `insert_plan` returns an empty products map, which the fee resolution needs.
async fn seed_quotes(
    store: &Store,
    services: &Services,
    tenant_id: TenantId,
    quotes: &[scenarios::domain::QuoteSpec],
    customers: &[scenarios::domain::Customer],
    customer_map: &HashMap<String, CustomerId>,
    plan_map: &HashMap<String, FullPlan>,
) -> StoreResult<()> {
    use meteroid_store::domain::quotes::{QuoteNew, QuotePriceComponentNew, RecipientDetails};

    for (i, spec) in quotes.iter().enumerate() {
        let Some(&customer_id) = customer_map.get(&spec.customer) else {
            log::warn!(
                "sandbox seed: quote references unknown customer '{}'",
                spec.customer
            );
            continue;
        };
        let Some(plan) = plan_map.get(&spec.plan_name) else {
            log::warn!(
                "sandbox seed: quote references unknown plan '{}'",
                spec.plan_name
            );
            continue;
        };

        let full = store
            .get_full_plan(
                plan.plan.id,
                tenant_id,
                store_domain::plans::PlanVersionFilter::Active,
            )
            .await?;

        // Address the quote to the customer so it reads as a sendable document.
        let recipients = customers
            .iter()
            .find(|c| c.name == spec.customer)
            .map(|c| {
                vec![RecipientDetails {
                    name: c.name.clone(),
                    email: c.email.clone(),
                }]
            })
            .unwrap_or_default();

        let units = store
            .get_credit_units_by_ids(
                tenant_id,
                full.price_components
                    .iter()
                    .flat_map(|pc| pc.credit_unit_ids())
                    .collect(),
            )
            .await?;
        let quote_id = QuoteId::new();
        let components: Vec<QuotePriceComponentNew> = full
            .price_components
            .iter()
            .filter_map(|pc| {
                let resolved = pc
                    .resolve_subscription_fee(&full.products, &units, None)
                    .ok()?;
                Some(QuotePriceComponentNew {
                    name: pc.name.clone(),
                    quote_id,
                    price_component_id: Some(pc.id),
                    product_id: pc.product_id,
                    period: resolved.period,
                    fee: resolved.fee,
                    is_override: false,
                    price_id: resolved.price_id,
                    example_usage_quantity: None,
                })
            })
            .collect();

        let quote = QuoteNew {
            id: quote_id,
            status: store_domain::enums::QuoteStatusEnum::Draft,
            tenant_id,
            customer_id,
            plan_version_id: full.version.id,
            currency: full.version.currency.clone(),
            quote_number: format!("Q-SEED-{:04}", i + 1),
            trial_duration_days: None,
            billing_start_date: None,
            billing_end_date: None,
            billing_day_anchor: None,
            activation_condition: SubscriptionActivationCondition::OnStart,
            valid_until: None,
            expires_at: None,
            internal_notes: None,
            cover_image: None,
            overview: None,
            terms_and_services: None,
            net_terms: 30,
            attachments: vec![],
            pdf_document_id: None,
            sharing_key: Some(uuid::Uuid::new_v4().to_string()),
            recipients,
            auto_advance_invoices: true,
            charge_automatically: false,
            invoice_memo: None,
            invoice_threshold: None,
            create_subscription_on_acceptance: false,
            payment_methods_config: None,
            custom_properties: serde_json::Value::Null,
            entitlements: vec![],
        };

        store
            .insert_quote_with_details(quote, components, vec![], vec![], vec![], Actor::System)
            .await?;
        store
            .publish_quote(Actor::System, quote_id, tenant_id)
            .await?;

        if spec.convert {
            store
                .accept_quote(Actor::System, quote_id, tenant_id)
                .await?;
            services
                .convert_quote_to_subscription(Actor::System, tenant_id, quote_id)
                .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::scenarios::pulse::pulse_scenario;
    use chrono::NaiveDateTime;
    use meteroid_store::domain::customer_summary::usage_pct;
    use meteroid_store::utils::datetime::elapsed_fraction;
    use rust_decimal::prelude::ToPrimitive;
    use rust_decimal_macros::dec;

    const TODAY: (i32, u32, u32) = (2026, 9, 23);

    fn date(ymd: (i32, u32, u32)) -> NaiveDate {
        NaiveDate::from_ymd_opt(ymd.0, ymd.1, ymd.2).unwrap()
    }

    fn customer_id(name: &str) -> CustomerId {
        let hash = name
            .bytes()
            .fold(0u128, |h, b| h.wrapping_mul(31).wrapping_add(u128::from(b)));
        CustomerId::from_const(uuid::Uuid::from_u128(hash))
    }

    struct Staged {
        staged: StagedUsage,
        generated: Vec<usage::GeneratedEvent>,
        events: Vec<usage::GeneratedEvent>,
        start: NaiveDate,
    }

    fn staged_customer(name: &str, today: NaiveDate) -> Staged {
        let scenario = pulse_scenario(today);
        let staged = staged_usage(&scenario, today);
        let customer = scenario.customers.iter().find(|c| c.name == name).unwrap();
        let (start, end) = customer_active_window(customer, today);
        let id = customer_id(name);
        let stream = usage::stream_for("ai_actions", "value", false, 18);
        let usage_customer = usage::UsageCustomer {
            customer_id: id,
            scale: usage::scale_for(id),
            start,
            end,
            seed: usage::seed_for(id, &stream.metric_code),
        };
        Staged {
            staged: staged.get(name).cloned().unwrap(),
            generated: usage::generate_stream(&stream, std::slice::from_ref(&usage_customer)),
            events: customer_events(&stream, &usage_customer, staged.get(name), today),
            start,
        }
    }

    fn total(events: &[usage::GeneratedEvent], from: NaiveDate, to: NaiveDateTime) -> Decimal {
        events
            .iter()
            .filter(|e| e.timestamp.date() >= from && e.timestamp < to)
            .map(|e| e.properties[0].1.parse::<Decimal>().unwrap())
            .sum()
    }

    fn cycle(staged: &StagedUsage) -> Period {
        match staged {
            StagedUsage::Target { cycle, .. } | StagedUsage::Collapse { cycle, .. } => {
                cycle.clone()
            }
            StagedUsage::None => panic!("no cycle"),
        }
    }

    #[test]
    fn cycle_usage_events_hit_target() {
        for today in [date(TODAY), date((2026, 3, 1)), date((2026, 12, 31))] {
            for (name, pct) in [("Cedar Foundry", 92), ("Batchline GmbH", 118)] {
                let s = staged_customer(name, today);
                let StagedUsage::Target { cycle, target, .. } = &s.staged else {
                    panic!("{name} is not a cycle target");
                };
                assert_eq!(*target, dec!(20000) * Decimal::from(pct) / dec!(100));
                assert!(cycle.start <= today && today < cycle.end, "{name} {today}");

                let (before, inside): (Vec<_>, Vec<_>) = s
                    .events
                    .iter()
                    .cloned()
                    .partition(|e| e.timestamp.date() < cycle.start);
                assert!(!inside.is_empty());
                assert!(inside.iter().all(|e| e.timestamp.date() <= today));
                let end_of_today = today.succ_opt().unwrap().and_time(NaiveTime::MIN);
                assert_eq!(total(&inside, cycle.start, end_of_today), *target);
                let generated_before: Vec<_> = s
                    .generated
                    .iter()
                    .filter(|e| e.timestamp.date() < cycle.start)
                    .cloned()
                    .collect();
                assert_eq!(
                    before, generated_before,
                    "{name}: history kept as generated"
                );
                assert!(before.iter().all(|e| e.timestamp.date() >= s.start));
            }
        }
    }

    #[test]
    fn usage_collapse_keeps_one_in_five() {
        let s = staged_customer("Pinewood Interactive", date(TODAY));
        let StagedUsage::Collapse { cycle, keep_every } = &s.staged else {
            panic!("Pinewood is not a collapse");
        };
        assert_eq!(*keep_every, 5);
        let in_cycle = |events: &[usage::GeneratedEvent]| {
            events
                .iter()
                .filter(|e| e.timestamp.date() >= cycle.start)
                .count()
        };
        let generated = in_cycle(&s.generated);
        assert!(generated >= 50);
        assert_eq!(in_cycle(&s.events), generated.div_ceil(5));
        assert_eq!(
            s.events.len() - in_cycle(&s.events),
            s.generated.len() - generated
        );
    }

    #[test]
    fn staged_usage_classifies_as_intended() {
        let today = date(TODAY);
        let now = today.and_time(NaiveTime::MIN);
        let limit = dec!(20000);

        // The limit_burndown signal warns from 80% of the limit by default.
        let consumed_pct = |name: &str| {
            let s = staged_customer(name, today);
            usage_pct(total(&s.events, cycle(&s.staged).start, now), limit).unwrap()
        };
        let cedar = consumed_pct("Cedar Foundry");
        assert!((80.0..100.0).contains(&cedar), "approaching, got {cedar}");
        let batchline = consumed_pct("Batchline GmbH");
        assert!(batchline >= 100.0, "over, got {batchline}");

        // usage_collapse by default: past 25% of the cycle, below half the previous cycle's pace.
        let s = staged_customer("Pinewood Interactive", today);
        let current = cycle(&s.staged);
        let previous = find_period_containing_date(
            s.start,
            current.start.pred_opt().unwrap(),
            &BillingPeriodEnum::Monthly,
            s.start.day(),
        );
        assert!(previous.end == current.start && previous.start >= s.start);
        let daily_rate = |from: NaiveDate, to: NaiveDateTime| {
            let days = (to - from.and_time(NaiveTime::MIN)).num_seconds() as f64 / 86_400.0;
            total(&s.events, from, to).to_f64().unwrap() / days
        };
        let previous_rate = daily_rate(previous.start, previous.end.and_time(NaiveTime::MIN));
        let ratio = daily_rate(current.start, now) / previous_rate;
        assert!(previous_rate >= 1.0, "previous rate {previous_rate}");
        assert!(ratio < 0.5, "ratio {ratio}");
        assert!(elapsed_fraction(current.start, current.end, now) >= 0.25);
    }
}
