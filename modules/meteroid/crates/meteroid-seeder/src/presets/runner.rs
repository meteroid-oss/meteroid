use chrono::Days;
use meteroid_store::domain::enums::{
    PlanStatusEnum, SubscriptionActivationCondition, TenantEnvironmentEnum,
};
use std::collections::HashMap;

use meteroid_store::repositories::{
    CustomersInterface, PlansInterface, ProductFamilyInterface, TenantInterface,
};
use meteroid_store::{Services, domain as store_domain};

use meteroid_store::Store;

use chrono::Utc;

use crate::presets::scenarios;
use crate::utils::slugify;
use common_domain::ids::{CustomerId, OrganizationId};
use meteroid_store::StoreResult;
use meteroid_store::domain::{FullPlan, Tenant};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;
use meteroid_store::repositories::invoicing_entities::{
    InvoicingEntityInterface, InvoicingEntityInterfaceAuto,
};
use uuid::Uuid;

pub async fn run_preset(
    store: &Store,
    services: &Services,
    scenario: scenarios::domain::Scenario,
    organization_id: OrganizationId,
    user_id: Uuid,
    tenant_name: Option<String>,
    disable_emails: Option<bool>,
) -> StoreResult<Tenant> {
    // TODO tenant archiving. Make sure all apis & more important, processors, do not run

    let now = Utc::now().naive_utc().date();

    let tenant = store
        .insert_tenant(
            store_domain::TenantNew {
                name: tenant_name.unwrap_or(scenario.name),
                environment: TenantEnvironmentEnum::Sandbox,
                disable_emails,
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
    for currency in scenario_currencies {
        if !available_currencies.contains(&currency) {
            available_currencies.push(currency);
        }
    }
    store
        .update_tenant_available_currencies(tenant.id, available_currencies)
        .await?;

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
            ..Default::default()
        };

        invoicing_entity = store.patch_invoicing_entity(patch, tenant.id).await?;
        log::info!("Updated invoicing entity with organization details");
    }

    let product_family = store.find_default_product_family(tenant.id).await?;

    let mut created_metrics = vec![];

    for metric in scenario.metrics {
        let created = store
            .insert_billable_metric(store_domain::BillableMetricNew {
                tenant_id: tenant.id,
                name: metric.name,
                code: metric.code,
                aggregation_type: metric.aggregation_type,
                aggregation_key: metric.aggregation_key,
                unit_conversion_factor: metric.unit_conversion_factor,
                unit_conversion_rounding: metric.unit_conversion_rounding,
                segmentation_matrix: metric.segmentation_matrix,
                usage_group_key: metric.usage_group_key,
                description: None,
                created_by: user_id,
                product_family_id: product_family.id,
                product_id: None,
            })
            .await?;

        log::info!("Created metric '{}'", &created.name);
        created_metrics.push(created);
    }

    let mut created_plans = vec![];

    for plan in scenario.plans {
        let created = store
            .insert_plan(store_domain::FullPlanNew {
                plan: store_domain::PlanNew {
                    name: plan.name,
                    plan_type: plan.plan_type,
                    status: PlanStatusEnum::Active,
                    tenant_id: tenant.id,
                    product_family_id: product_family.id,
                    description: None,
                    created_by: user_id,
                },
                version: store_domain::PlanVersionNewInternal {
                    is_draft_version: false,
                    trial: None, // TODO
                    period_start_day: None,
                    net_terms: 30,
                    currency: Some(plan.currency),
                    billing_cycles: None, // TODO drop
                },
                price_components: plan
                    .components
                    .into_iter()
                    .map(|component| component.to_domain(&created_metrics))
                    .collect::<Result<Vec<_>, _>>()?,
            })
            .await?;

        log::info!("Created plan '{}'", &created.plan.name);
        created_plans.push(created);
    }

    let plan_map: HashMap<String, FullPlan> =
        HashMap::from_iter(created_plans.into_iter().map(|c| (c.plan.name.clone(), c)));

    let mut customers_to_create = vec![];

    for customer in scenario.customers.clone() {
        let created_at = customer
            .subscription
            .start_date
            .checked_sub_days(Days::new(1))
            .unwrap_or(now);

        customers_to_create.push(store_domain::CustomerNew {
            invoicing_entity_id: Some(invoicing_entity.id),
            billing_email: Some(customer.email.clone()),
            invoicing_emails: customer.invoicing_emails.clone(),
            phone: customer.phone.clone(),
            balance_value_cents: 0,
            currency: customer.currency,
            billing_address: customer.billing_address.clone(),
            created_by: user_id,
            force_created_date: created_at.and_hms_opt(0, 0, 0),
            vat_number: customer.vat_number.clone(),
            alias: customer
                .alias
                .clone()
                .or_else(|| Some(slugify(&customer.name))),
            name: customer.name,
            shipping_address: customer.shipping_address.clone(),
            custom_taxes: vec![],
            is_tax_exempt: false,
        });
    }

    let created_customers = store
        .insert_customer_batch(customers_to_create, tenant.id)
        .await?;

    let customer_map: HashMap<String, CustomerId> = HashMap::from_iter(
        created_customers
            .into_iter()
            .map(|c| (c.name.clone(), c.id)),
    );

    let mut subscriptions_to_create = vec![];

    for customer in scenario.customers {
        let customer_id = customer_map
            .get(&customer.name)
            .ok_or(StoreError::ValueNotFound(format!(
                "Customer was not found : {}",
                &customer.name
            )))?;

        let plan =
            plan_map
                .get(&customer.subscription.plan_name)
                .ok_or(StoreError::ValueNotFound(format!(
                    "Plan was not found : {}",
                    &customer.subscription.plan_name
                )))?;

        let subscription = store_domain::SubscriptionNew {
            customer_id: *customer_id,
            activation_condition: SubscriptionActivationCondition::OnStart,
            trial_duration: None,
            billing_day_anchor: None,
            plan_version_id: plan.version.id,
            created_by: user_id,
            net_terms: None,
            invoice_memo: None,
            invoice_threshold: None,
            start_date: customer.subscription.start_date,
            end_date: None,
            payment_methods_config: None,
            billing_start_date: None,
            auto_advance_invoices: true,
            charge_automatically: false,
            purchase_order: None,
            backdate_invoices: true,
            skip_checkout_session: false,
        };

        subscriptions_to_create.push(store_domain::CreateSubscription {
            subscription,
            price_components: None, // TODO parameters
            add_ons: None,
            coupons: None,
        });
    }

    services
        .insert_subscription_batch(subscriptions_to_create, tenant.id)
        .await?;

    // in random seeder we did generate invocies manually.
    // Here we will instead rely on the worker

    // TODO mark past invoices as paid a few days after issues
    // + allow marking a last invoice as failed or overdue ex: last_status

    Ok(tenant)
}
