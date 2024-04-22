use std::env;

use tokio::signal;

use common_logging::init::init_regular_logging;
use error_stack::ResultExt;
use meteroid::seeder::domain;
use meteroid::seeder::errors::SeederError;
use meteroid::seeder::runner;
use meteroid::seeder::utils::slugify;
use meteroid_store::domain::enums::{BillingPeriodEnum, PlanTypeEnum};
use meteroid_store::domain::{DowngradePolicy, UpgradePolicy};
use meteroid_store::Store;
use rust_decimal_macros::dec;

#[tokio::main]
async fn main() -> error_stack::Result<(), SeederError> {
    dotenvy::dotenv().ok();

    init_regular_logging();
    let _exit = signal::ctrl_c();

    let crypt_key = secrecy::SecretString::new("00000000000000000000000000000000".into());

    let store = Store::new(
        env::var("DATABASE_URL").change_context(SeederError::InitializationError)?,
        crypt_key,
    )
    .change_context(SeederError::InitializationError)?;

    let organization_id = uuid::uuid!("018dfa06-2e9b-7c70-a6a9-7a9e4dc7ce70");
    let user_id = uuid::uuid!("018dfa06-2e9c-74b8-a6ea-247967b75a63");

    let rand_tenant_name = format!("Seedtest / {}", rand::random::<u32>());

    log::info!("Creating tenant '{}'", rand_tenant_name);

    let scenario = domain::Scenario {
        tenant: domain::Tenant {
            slug: slugify(&rand_tenant_name),
            name: rand_tenant_name,
            currency: "EUR".to_string(),
        },
        plans: vec![
            domain::Plan {
                name: "Free".to_string(),
                code: "free".to_string(),
                weight: 0.2,
                description: None,
                plan_type: PlanTypeEnum::Free,
                version_details: domain::PlanVersion {
                    trial_duration_days: None,
                    trial_fallback_plan_id: None,
                    period_start_day: None,
                    currency: "EUR".to_string(),
                    billing_cycles: None,
                    billing_periods: vec![],
                    net_terms: 0,
                },
                components: vec![],
                churn_rate: None,
            },
            domain::Plan {
                name: "Hobby".to_string(),
                code: "hobby".to_string(),
                weight: 0.7,
                description: None,
                plan_type: PlanTypeEnum::Standard,
                version_details: domain::PlanVersion {
                    trial_duration_days: None,
                    trial_fallback_plan_id: None,
                    period_start_day: None,
                    currency: "EUR".to_string(),
                    billing_cycles: None,
                    billing_periods: vec![BillingPeriodEnum::Monthly, BillingPeriodEnum::Annual],
                    net_terms: 0,
                },
                components: vec![meteroid_store::domain::PriceComponentNewInternal {
                    name: "Rate".to_string(),
                    product_item_id: None,
                    fee: meteroid_store::domain::FeeType::Rate {
                        rates: vec![
                            meteroid_store::domain::TermRate {
                                term: BillingPeriodEnum::Monthly,
                                price: dec!(35.00),
                            },
                            meteroid_store::domain::TermRate {
                                term: BillingPeriodEnum::Annual,
                                price: dec!(159.00),
                            },
                        ],
                    },
                }],
                churn_rate: Some(0.05),
            },
            domain::Plan {
                name: "Enterprise".to_string(),
                code: "enterprise".to_string(),
                weight: 0.1,
                description: None,
                plan_type: PlanTypeEnum::Standard,
                version_details: domain::PlanVersion {
                    trial_duration_days: None,
                    trial_fallback_plan_id: None,
                    period_start_day: None,
                    currency: "EUR".to_string(),
                    billing_cycles: None,
                    billing_periods: vec![BillingPeriodEnum::Annual],
                    net_terms: 90,
                },
                components: vec![meteroid_store::domain::PriceComponentNewInternal {
                    name: "Seats".to_string(),
                    product_item_id: None,
                    fee: meteroid_store::domain::FeeType::Slot {
                        quota: None,
                        rates: vec![
                            meteroid_store::domain::TermRate {
                                term: BillingPeriodEnum::Monthly,
                                price: dec!(50.00),
                            },
                            meteroid_store::domain::TermRate {
                                term: BillingPeriodEnum::Annual,
                                price: dec!(496.00),
                            },
                        ],
                        slot_unit_name: "Seats".to_string(),
                        minimum_count: Some(5),
                        upgrade_policy: UpgradePolicy::Prorated,
                        downgrade_policy: DowngradePolicy::RemoveAtEndOfPeriod,
                    },
                }],
                churn_rate: Some(0.02),
            },
        ],
        start_date: chrono::NaiveDate::from_ymd_opt(2022, 1, 1).unwrap(),
        end_date: chrono::Utc::now().naive_utc().date(),
        product_family: "Marketing Hub".to_string(),
        name: "Test".to_string(),
        customer_base: domain::CustomerBase {
            dataset_path: None,
            customer_count: Some(50),
            customer_growth_curve: vec![0.1, 0.3, 1.0],
        },
        randomness: domain::Randomness {
            seed: None,
            randomness_factor: 0.5,
        },
    };

    let service = runner::run(store, scenario, organization_id, user_id);

    service.await.change_context(SeederError::StoreError)
}
