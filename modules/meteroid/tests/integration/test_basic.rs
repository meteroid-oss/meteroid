use rust_decimal::Decimal;
use testcontainers::clients::Cli;
use meteroid::api::shared::conversions::ProtoConv;

use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use meteroid::db::get_connection;
use meteroid_grpc::meteroid::api;
use meteroid_grpc::meteroid::api::customers::v1::CustomerBillingConfig;
use meteroid_grpc::meteroid::api::plans::v1::PlanType;

use meteroid_grpc::meteroid::api::users::v1::UserRole;

#[tokio::test]
async fn test_main() {
    // Generic setup
    helpers::init::logging();
    let docker = Cli::default();
    let (_postgres_container, postgres_connection_string) =
        meteroid_it::container::start_postgres(&docker);
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;

    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    assert_eq!(auth.user.unwrap().role, UserRole::Admin as i32);

    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "",
    );

    let tenant = clients
        .tenants
        .clone()
        .create_tenant(tonic::Request::new(api::tenants::v1::CreateTenantRequest {
            name: "Test - usage".to_string(),
            slug: "test-usage".to_string(),
            currency: "USD".to_string(),
        }))
        .await
        .unwrap()
        .into_inner()
        .tenant
        .unwrap();

    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        tenant.slug.as_str(),
    );

    let product_family = clients
        .product_families
        .clone()
        .create_product_family(tonic::Request::new(
            api::productfamilies::v1::CreateProductFamilyRequest {
                name: "Test - usage".to_string(),
                external_id: "test-usage".to_string(),
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .product_family
        .unwrap();

    let plan = clients
        .plans
        .clone()
        .create_draft_plan(tonic::Request::new(
            api::plans::v1::CreateDraftPlanRequest {
                name: "Test - usage".to_string(),
                external_id: "test-usage-plan".to_string(),
                description: None,
                product_family_external_id: product_family.external_id,
                plan_type: PlanType::Standard as i32,
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .plan
        .unwrap();

    let plan_version = plan.current_version.unwrap();

    let price_component = clients
        .price_components
        .clone()
        .create_price_component(tonic::Request::new(
            api::components::v1_2::CreatePriceComponentRequest {
                plan_version_id: plan_version.clone().id,
                name: "One Time".to_string(),
                fee: Some(api::components::v1_2::Fee {
                    fee_type: Some(api::components::v1_2::fee::FeeType::OneTime(
                        api::components::v1_2::fee::OneTimeFee {
                            unit_price: Decimal::new(100, 2).to_string(),
                            quantity: 1,
                        },
                    )),
                }),
                product_item_id: None,
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .component
        .unwrap();

    let _published_plan = clients
        .plans
        .clone()
        .publish_plan_version(tonic::Request::new(
            api::plans::v1::PublishPlanVersionRequest {
                plan_id: plan.plan.unwrap().id,
                plan_version_id: plan_version.clone().id,
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .plan_version
        .unwrap();

    let customer = clients
        .customers
        .clone()
        .create_customer(tonic::Request::new(
            api::customers::v1::CreateCustomerRequest {
                name: "Customer A".to_string(),
                email: Some("customer@domain.com".to_string()),
                alias: None,
                billing_config: Some(CustomerBillingConfig::default()),
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .customer
        .unwrap();

    let now = chrono::offset::Local::now().date_naive();

    let subscription = clients
        .subscriptions
        .clone()
        .create_subscription(tonic::Request::new(
            api::subscriptions::v1_2::CreateSubscriptionRequest {
                subscription: Some(api::subscriptions::v1_2::CreateSubscription {
                    plan_version_id: plan_version.clone().id,
                    billing_start_date: now.as_proto(),
                    billing_day: 1,
                    customer_id: customer.id.clone(),
                    currency: "USD".to_string(),
                    ..Default::default()
                })
            },
        ))
        .await
        .unwrap()
        .into_inner();

    let db_subscription = meteroid_repository::subscriptions::get_subscription_by_id()
        .bind(
            &get_connection(&setup.pool).await.unwrap(),
            &uuid::Uuid::parse_str(subscription.subscription.clone().unwrap().id.as_str()).unwrap(),
            &uuid::Uuid::parse_str(tenant.id.as_str()).unwrap(),
        )
        .one()
        .await
        .unwrap();

    let tenant_billing = clients
        .tenants
        .clone()
        .configure_tenant_billing(tonic::Request::new(
            api::tenants::v1::ConfigureTenantBillingRequest {
                billing_config: Some(api::tenants::v1::TenantBillingConfiguration {
                    billing_config_oneof: Some(
                        api::tenants::v1::tenant_billing_configuration::BillingConfigOneof::Stripe(
                            api::tenants::v1::tenant_billing_configuration::Stripe {
                                api_secret: "sk_test_123".to_string(),
                                webhook_secret: "whsec_123".to_string(),
                            },
                        ),
                    ),
                }),
            },
        ))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        tenant_billing
            .billing_config
            .unwrap()
            .billing_config_oneof
            .unwrap(),
        api::tenants::v1::tenant_billing_configuration::BillingConfigOneof::Stripe(
            api::tenants::v1::tenant_billing_configuration::Stripe {
                api_secret: "sk_test_123".to_string(),
                webhook_secret: "whsec_123".to_string(),
            }
        )
    );

    // check DB state
    assert_eq!(
        db_subscription.customer_id.clone().to_string(),
        customer.id.clone()
    );
    assert_eq!(db_subscription.billing_day, 1);
    assert_eq!(db_subscription.plan_version_id.to_string(), plan_version.id);

    let db_invoices = meteroid_repository::invoices::get_invoices_to_issue()
        .bind(&get_connection(&setup.pool).await.unwrap(), &1)
        .all()
        .await
        .unwrap();

    assert_eq!(db_invoices.len(), 1);

    let db_invoice = db_invoices.get(0).unwrap();

    assert_eq!(db_invoice.tenant_id.to_string(), tenant.id);
    assert_eq!(db_invoice.customer_id.clone().to_string(), customer.id);
    assert_eq!(
        db_invoice.subscription_id.to_string(),
        subscription.subscription.clone().unwrap().id
    );

    // teardown
    meteroid_it::container::terminate_meteroid(setup.token, setup.join_handle).await
}
