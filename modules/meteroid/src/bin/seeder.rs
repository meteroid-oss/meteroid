use std::collections::{BTreeMap, HashMap};
use std::env;
use std::sync::Arc;

use tokio::signal;

use common_logging::init::init_regular_logging;
use common_utils::rng::UPPER_ALPHANUMERIC;
use error_stack::ResultExt;
use meteroid::eventbus::create_eventbus_noop;
use meteroid::seeder::domain;
use meteroid::seeder::errors::SeederError;
use meteroid::seeder::runner;
use meteroid::seeder::utils::slugify;
use meteroid_mailer::config::MailerConfig;
use meteroid_oauth::config::OauthConfig;
use meteroid_store::compute::clients::usage::MockUsageClient;
use meteroid_store::domain::enums::{BillingPeriodEnum, PlanTypeEnum};
use meteroid_store::domain::historical_rates::HistoricalRatesFromUsdNew;
use meteroid_store::domain::{DowngradePolicy, UpgradePolicy};
use meteroid_store::repositories::historical_rates::HistoricalRatesInterface;
use meteroid_store::store::StoreConfig;
use meteroid_store::Store;
use rust_decimal_macros::dec;
use secrecy::SecretString;
use stripe_client::client::StripeClient;
use tap::TapFallible;

#[tokio::main]
async fn main() -> error_stack::Result<(), SeederError> {
    dotenvy::dotenv().ok();

    init_regular_logging();
    let _exit = signal::ctrl_c();

    let store = Store::new(StoreConfig {
        database_url: env::var("DATABASE_URL").change_context(SeederError::InitializationError)?,
        crypt_key: env::var("SECRETS_CRYPT_KEY")
            .map(SecretString::new)
            .change_context(SeederError::InitializationError)?,
        jwt_secret: env::var("JWT_SECRET")
            .map(SecretString::new)
            .change_context(SeederError::InitializationError)?,
        multi_organization_enabled: false,
        public_url: "http://localhost:8080".to_owned(),
        eventbus: create_eventbus_noop().await,
        usage_client: Arc::new(MockUsageClient {
            data: HashMap::new(),
        }),
        svix: None,
        mailer: meteroid_mailer::service::mailer_service(MailerConfig::dummy()),
        stripe: Arc::new(StripeClient::new()),
        oauth: meteroid_oauth::service::OauthServices::new(OauthConfig::dummy()),
    })
    .change_context(SeederError::InitializationError)?;

    let organization_id = env::var("SEEDER_ORGANIZATION_ID")
        .map(|s| uuid::Uuid::parse_str(&s))
        .change_context(SeederError::InitializationError)?
        .change_context(SeederError::InitializationError)?;

    let tenant_currency = "EUR".to_string();
    let tenant_country = "FR".to_string();

    let now = chrono::Utc::now().naive_utc();

    store
        .create_historical_rate_from_usd(HistoricalRatesFromUsdNew {
            date: now
                .date()
                .checked_sub_months(chrono::Months::new(5 * 12))
                .unwrap(),
            rates: BTreeMap::from([(tenant_currency.clone(), 0.92)]),
        })
        .await
        .change_context(SeederError::InitializationError)?;

    let user_id = uuid::uuid!("00000000-0000-0000-0000-000000000000");

    let tenant_name = format!("seed-{}", nanoid::nanoid!(6, &UPPER_ALPHANUMERIC));

    log::info!("Creating tenant '{}'", tenant_name);

    let scenario = domain::Scenario {
        metrics: Vec::new(),
        tenant: domain::Tenant {
            slug: slugify(&tenant_name),
            name: tenant_name,
            currency: tenant_currency.clone(),
            country: tenant_country.to_string(),
        },
        plans: vec![
            domain::Plan {
                name: "Free".to_string(),
                code: "free".to_string(),
                weight: 0.2,
                description: None,
                plan_type: PlanTypeEnum::Free,
                version_details: domain::PlanVersion {
                    trial: None,
                    period_start_day: None,
                    currency: tenant_currency.clone(),
                    billing_cycles: None,
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
                    trial: None,
                    period_start_day: None,
                    currency: tenant_currency.clone(),
                    billing_cycles: None,
                    net_terms: 0,
                },
                components: vec![meteroid_store::domain::PriceComponentNewInternal {
                    name: "Rate".to_string(),
                    product_id: None,
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
                    trial: None,
                    period_start_day: None,
                    currency: tenant_currency.clone(),
                    billing_cycles: None,
                    net_terms: 90,
                },
                components: vec![meteroid_store::domain::PriceComponentNewInternal {
                    name: "Seats".to_string(),
                    product_id: None,
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
            customer_count: Some(5),
            customer_growth_curve: vec![0.1, 0.3, 1.0],
        },
        randomness: domain::Randomness {
            seed: None,
            randomness_factor: 0.5,
        },
    };

    let service = runner::run(store, scenario, organization_id, user_id);

    service
        .await
        .change_context(SeederError::StoreError)
        .tap_err(|e| log::error!("Error: {:?}", e))
}
