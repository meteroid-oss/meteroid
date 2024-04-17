use chrono::{Datelike, Days, NaiveDate};
use error_stack::ResultExt;
use fake::Fake;
use meteroid_store::domain::enums::{
    BillingPeriodEnum, InvoiceStatusEnum, InvoiceType, InvoicingProviderEnum, PlanStatusEnum,
    PlanTypeEnum, TenantEnvironmentEnum,
};

use meteroid_store::domain as store_domain;
use meteroid_store::repositories::*;
use uuid::Uuid;

use super::errors::SeederError;
use super::growth::generate_smooth_growth;
use super::utils::slugify;
use meteroid_store::Store;

use fake::faker::company::en::CompanyName;
use fake::faker::internet::en::SafeEmail;

use rand::prelude::*;
use rand_chacha::ChaCha8Rng;

use crate::compute::calculate_period_range;
use crate::compute::clients::slots::MockSlotClient;
use crate::compute::clients::usage::MockUsageClient;
use crate::compute::InvoiceEngine;

use chrono::Utc;

use nanoid::nanoid;

use meteroid_store::domain::TenantContext;
use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;
use std::collections::HashMap;
use std::sync::Arc;

pub async fn run(
    store: Store,
    scenario: super::domain::Scenario,
    organization_id: Uuid,
    user_id: Uuid,
) -> error_stack::Result<(), SeederError> {
    // create an org, tenant, user (if standalone mode)
    // const setup_res = setup();

    let now = Utc::now().naive_utc().date();

    let mut rng = match scenario.randomness.seed {
        Some(seed) => ChaCha8Rng::seed_from_u64(seed),
        None => ChaCha8Rng::from_entropy(),
    };

    let tenant = store
        .insert_tenant(store_domain::TenantNew {
            name: scenario.tenant.name,
            slug: scenario.tenant.slug,
            organization_id: organization_id.clone(),
            currency: scenario.tenant.currency,
            environment: Some(TenantEnvironmentEnum::Sandbox),
        })
        .await
        .change_context(SeederError::TempError)?;

    log::info!("Created tenant '{}'", &tenant.name);

    let product_family = store
        .insert_product_family(store_domain::ProductFamilyNew {
            external_id: slugify(&scenario.product_family),
            name: scenario.product_family,
            tenant_id: tenant.id,
        })
        .await
        .change_context(SeederError::TempError)?;

    log::info!("Created product family '{}'", &product_family.name);

    let mut created_plans = vec![];

    // create the plans
    for plan in scenario.plans.clone() {
        let created = store
            .insert_plan(store_domain::FullPlanNew {
                plan: store_domain::PlanNew {
                    external_id: slugify(&plan.name),
                    name: plan.name,
                    plan_type: plan.plan_type,
                    status: PlanStatusEnum::Active,
                    tenant_id: tenant.id,
                    product_family_id: product_family.id,
                    description: plan.description,
                    created_by: user_id,
                },
                version: store_domain::PlanVersionNewInternal {
                    is_draft_version: false,
                    trial_duration_days: plan.version_details.trial_duration_days,
                    trial_fallback_plan_id: plan.version_details.trial_fallback_plan_id,
                    period_start_day: plan.version_details.period_start_day,
                    net_terms: plan.version_details.net_terms,
                    currency: plan.version_details.currency,
                    billing_cycles: plan.version_details.billing_cycles,
                    billing_periods: plan.version_details.billing_periods,
                },
                price_components: plan
                    .components
                    .into_iter()
                    .map(|component| store_domain::PriceComponentNewInternal {
                        name: component.name.clone(),
                        fee: component.fee.clone(),
                        product_item_id: component.product_item_id.clone(),
                    })
                    .collect::<Vec<_>>(),
            })
            .await
            .change_context(SeederError::TempError)?;

        log::info!("Created plan '{}'", &created.plan.name);
        created_plans.push(created);
    }

    // create the customers
    let customer_smooth_growth = generate_smooth_growth(
        scenario.start_date,
        scenario.end_date,
        scenario.customer_base.customer_count.unwrap_or(10),
        scenario.customer_base.customer_growth_curve,
        scenario.randomness.randomness_factor,
    );

    log::info!(
        "Expecting {:?} customers",
        scenario.customer_base.customer_count
    );
    log::info!("Creating {} customers", customer_smooth_growth.len());

    let mut customers_to_create = vec![];
    // we turn that into a vec of customers
    for (date, customer_count, _) in customer_smooth_growth {
        (0..customer_count).for_each(|_| {
            let company_name: String = CompanyName().fake();

            log::info!("Creating customer '{}'", &company_name);

            let alias = format!("{}-{}", slugify(&company_name), nanoid!(5));
            customers_to_create.push(store_domain::CustomerNew {
                tenant_id: tenant.id,
                billing_config: None,
                email: SafeEmail().fake(),
                invoicing_email: None,
                phone: None,
                balance_value_cents: 0,
                balance_currency: "EUR".to_string(),
                billing_address: None, // TODO
                created_by: user_id,
                created_at: date.and_hms_opt(0, 0, 0),
                alias: Some(alias),
                name: company_name.to_string(),
                shipping_address: None,
            });
        });
    }

    let created_customers = store
        .insert_customer_batch(customers_to_create)
        .await
        .change_context(SeederError::TempError)?;

    let mut subscriptions_to_create = vec![];

    for customer in created_customers {
        // for now, the customer lifecycle is defined only by a single subscription.

        let _plan = &scenario
            .plans
            .choose_weighted(&mut rng, |f| f.weight)
            .unwrap();

        // we pick a plan at random => TODO plan prob matrix
        let store_domain::FullPlan {
            plan,
            version,
            price_components,
        } = created_plans
            .iter()
            .find(|f| f.plan.name == _plan.name)
            .unwrap();

        let customer_created_at_date = customer.created_at.date();
        let trial_start_date = version
            .trial_duration_days
            .map(|_| customer_created_at_date);

        // TODO billing start date to None for free plans
        // if paid plan
        let billing_start_date = customer_created_at_date
            .checked_add_days(Days::new(version.trial_duration_days.unwrap_or(0) as u64))
            .unwrap_or(customer_created_at_date);

        let activated_at = if plan.plan_type != PlanTypeEnum::Free {
            billing_start_date.and_hms_opt(0, 0, 0)
        } else {
            None
        };

        log::info!("Creating subscription for plan '{}'", plan.name);

        let billing_end_date = None;

        let subscription = store_domain::SubscriptionNew {
            customer_id: customer.id,
            currency: "EUR".to_string(), // TODO
            billing_day: version.period_start_day.unwrap_or(1),
            tenant_id: tenant.id,
            trial_start_date,
            billing_start_date,
            billing_end_date,
            plan_version_id: version.id,
            created_by: user_id,
            net_terms: version.net_terms,
            invoice_memo: None,
            invoice_threshold: None,
            activated_at,
        };

        let mut parameterized_components = vec![];
        // here we decide wether we need to provide parameters or not
        for component in price_components.clone() {
            match &component.fee {
                store_domain::FeeType::Rate { rates } => {
                    if rates.len() > 1 {
                        // Multiple rates, requires parameterization
                        let billing_period = rates[rng.gen_range(0..rates.len())].term.clone();
                        parameterized_components.push(store_domain::ComponentParameterization {
                            component_id: component.id,
                            parameters: store_domain::ComponentParameters {
                                billing_period: Some(billing_period),
                                initial_slot_count: None,
                                committed_capacity: None,
                            },
                        });
                    }
                }
                store_domain::FeeType::Slot {
                    rates,
                    minimum_count,
                    ..
                } => {
                    // Slot-based pricing, requires parameterization
                    let billing_period = rates[rng.gen_range(0..rates.len())].term.clone();
                    let initial_slots = rng.gen_range(minimum_count.clone().unwrap_or(1)..=100); // Generate a random number of initial slots (adjust the range as needed)
                    parameterized_components.push(store_domain::ComponentParameterization {
                        component_id: component.id,
                        parameters: store_domain::ComponentParameters {
                            billing_period: Some(billing_period),
                            initial_slot_count: Some(initial_slots),
                            committed_capacity: None,
                        },
                    });
                }
                store_domain::FeeType::Capacity { thresholds, .. } => {
                    if thresholds.len() > 1 {
                        // Multiple capacity thresholds, requires parameterization
                        let committed_capacity =
                            thresholds[rng.gen_range(0..thresholds.len())].included_amount;
                        parameterized_components.push(store_domain::ComponentParameterization {
                            component_id: component.id,
                            parameters: store_domain::ComponentParameters {
                                billing_period: None,
                                initial_slot_count: None,
                                committed_capacity: Some(committed_capacity),
                            },
                        });
                    }
                }
                _ => {}
            }
        }

        let create_subscription_components = if parameterized_components.is_empty() {
            None
        } else {
            Some(store_domain::CreateSubscriptionComponents {
                parameterized_components,
                overridden_components: vec![],
                extra_components: vec![],
                remove_components: vec![],
            })
        };

        let params = store_domain::CreateSubscription {
            subscription,
            price_components: create_subscription_components,
        };

        subscriptions_to_create.push(params);

        // let created_subscription = store.insert_subscription(subscription).await?;
    }

    let created_subscriptions = store
        .insert_subscription_batch(subscriptions_to_create)
        .await
        .change_context(SeederError::TempError)?;

    let created_plan_hashmap = created_plans
        .into_iter()
        .map(|plan| (plan.version.id, plan))
        .collect::<std::collections::HashMap<_, _>>();

    for subscription in created_subscriptions {
        // TODO batch

        log::info!(
            "Created subscription components for plan version '{}'",
            subscription.plan_version_id.to_string()
        );

        // we get the plan
        let plan = created_plan_hashmap
            .get(&subscription.plan_version_id)
            .unwrap();

        log::info!("plan.plan_type '{:?}'", &plan.plan.plan_type);

        if plan.plan.plan_type == PlanTypeEnum::Free {
            continue;
        }

        log::info!("price_components '{}'", plan.price_components.len());

        let churn_rate = scenario
            .plans
            .iter()
            .find(|p| p.name == plan.plan.name)
            .and_then(|c| c.churn_rate);

        // Add some variations (cancellations, reactivations, upgrades, downgrades, switch, trial conversions TODO)
        // CHURN START
        match churn_rate {
            Some(churn_rate) => {
                if plan.plan.plan_type != PlanTypeEnum::Free {
                    let months_since_start = (now.year() - subscription.billing_start_date.year())
                        * 12
                        + now.month() as i32
                        - subscription.billing_start_date.month() as i32;

                    let churn_probability = 1.0 - (1.0 - churn_rate).powi(months_since_start);

                    if rng.gen::<f64>() < churn_probability {
                        let end_month = rng.gen_range(0..=months_since_start);
                        let end_date = subscription.billing_start_date
                            + chrono::Duration::days(end_month as i64 * 30);

                        if end_date < now {
                            store
                                .cancel_subscription(
                                    subscription.id,
                                    Some("Not used anymore".to_string()),
                                    CancellationEffectiveAt::Date(end_date),
                                    TenantContext {
                                        tenant_id: tenant.id,
                                        actor: user_id,
                                    },
                                )
                                .await
                                .change_context(SeederError::TempError)?;
                        }
                    }
                }
            }
            None => {}
        }
        // CHURN END

        // create the invoices for the whole subscription lifecycle

        // basically we get all price components, for each we outline the billing period, then we can figure out the dates of all invoices and the components that should be included

        // TODO for ALL billing periods of the subscription
        let invoice_dates = calculate_period_end_dates(
            subscription.billing_start_date,
            subscription.billing_end_date,
            subscription.billing_day as u32,
            &BillingPeriodEnum::Monthly,
        );

        let invoice_engine = InvoiceEngine::new(
            Arc::new(MockUsageClient {
                data: HashMap::new(),
            }),
            Arc::new(MockSlotClient {
                data: HashMap::new(),
            }),
        );

        // TODO don't refetch the details, we should have everything, or at the least do it in a batch
        let details = store
            .get_subscription_details(subscription.tenant_id, subscription.id)
            .await
            .change_context(SeederError::TempError)?;

        let subscription_details = details;

        let mut invoices_to_create = vec![];

        for invoice_date in invoice_dates {
            // we get all components that need to be invoiced for this date
            let invoice_lines = invoice_engine
                .compute_dated_invoice_lines(&invoice_date, subscription_details.clone())
                .await
                .change_context(SeederError::TempError)?;

            if invoice_lines.is_empty() {
                continue;
            }

            let amount_cents = invoice_lines.iter().map(|c| c.total).sum();

            if amount_cents == 0 {
                continue;
            }

            let is_last_invoice = &invoice_date > &now;

            // we create the invoice
            let invoice = store_domain::InvoiceNew {
                tenant_id: tenant.id,
                customer_id: subscription.customer_id,
                subscription_id: subscription.id,
                amount_cents: Some(amount_cents),
                plan_version_id: Some(subscription.plan_version_id),
                invoice_type: InvoiceType::Recurring,
                currency: "EUR".to_string(),
                days_until_due: None,
                external_invoice_id: None,
                invoice_id: None,
                invoicing_provider: InvoicingProviderEnum::Stripe,
                line_items: serde_json::to_value(invoice_lines).unwrap(),
                issued: false,
                issue_attempts: 0,
                last_issue_attempt_at: None,
                last_issue_error: None,
                data_updated_at: None,
                status: if is_last_invoice {
                    InvoiceStatusEnum::Draft
                } else {
                    InvoiceStatusEnum::Finalized
                },
                external_status: None,
                invoice_date,
                finalized_at: if is_last_invoice {
                    None
                } else {
                    invoice_date.and_hms_opt(0, 0, 0)
                },
            };

            invoices_to_create.push(invoice);
        }

        store
            .insert_invoice_batch(invoices_to_create)
            .await
            .change_context(SeederError::TempError)?;
    }

    Ok(())
}

fn calculate_period_end_dates(
    billing_start_date: NaiveDate,
    billing_end_date: Option<NaiveDate>,
    billing_day: u32,
    billing_period: &BillingPeriodEnum,
) -> Vec<NaiveDate> {
    let mut end_dates = Vec::new();
    let mut period_index = 0;
    let end = billing_end_date.unwrap_or_else(|| Utc::now().naive_utc().date());

    // TODO check that. We add the billing_start_date, but that's maybe just if there is an advance fee
    end_dates.push(billing_start_date);
    loop {
        let period = calculate_period_range(
            billing_start_date,
            billing_day,
            period_index,
            billing_period,
        );
        end_dates.push(period.end);
        if period.end >= end {
            break;
        }
        period_index += 1;
    }

    end_dates
}
