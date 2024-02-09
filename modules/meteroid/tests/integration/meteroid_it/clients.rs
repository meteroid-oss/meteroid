use std::str::FromStr;

use http::{HeaderName, HeaderValue};
use tonic::transport::Channel;
use tower_http::auth::{AddAuthorization, AddAuthorizationLayer};
use tower_http::set_header::{SetRequestHeader, SetRequestHeaderLayer};

use common_grpc::middleware::common::auth::TENANT_SLUG_HEADER;
use meteroid_grpc::meteroid::api::apitokens::v1::api_tokens_service_client::ApiTokensServiceClient;
use meteroid_grpc::meteroid::api::billablemetrics::v1::billable_metrics_service_client::BillableMetricsServiceClient;
use meteroid_grpc::meteroid::api::components::v1::price_components_service_client::PriceComponentsServiceClient;
use meteroid_grpc::meteroid::api::customers::v1::customers_service_client::CustomersServiceClient;
use meteroid_grpc::meteroid::api::instance::v1::instance_service_client::InstanceServiceClient;
use meteroid_grpc::meteroid::api::plans::v1::plans_service_client::PlansServiceClient;
use meteroid_grpc::meteroid::api::productfamilies::v1::product_families_service_client::ProductFamiliesServiceClient;
use meteroid_grpc::meteroid::api::products::v1::products_service_client::ProductsServiceClient;
use meteroid_grpc::meteroid::api::schedules::v1::schedules_service_client::SchedulesServiceClient;
use meteroid_grpc::meteroid::api::subscriptions::v1::subscriptions_service_client::SubscriptionsServiceClient;
use meteroid_grpc::meteroid::api::tenants::v1::tenants_service_client::TenantsServiceClient;
use meteroid_grpc::meteroid::api::users::v1::users_service_client::UsersServiceClient;
use meteroid_grpc::meteroid::api::webhooks::out::v1::webhooks_service_client::WebhooksServiceClient;

pub type TestLayeredClientService = AddAuthorization<SetRequestHeader<Channel, HeaderValue>>;

pub struct AllClients {
    pub api_tokens: ApiTokensServiceClient<TestLayeredClientService>,
    pub customers: CustomersServiceClient<TestLayeredClientService>,
    pub metrics: BillableMetricsServiceClient<TestLayeredClientService>,
    pub instance: InstanceServiceClient<TestLayeredClientService>,
    pub plans: PlansServiceClient<TestLayeredClientService>,
    pub price_components: PriceComponentsServiceClient<TestLayeredClientService>,
    pub product_families: ProductFamiliesServiceClient<TestLayeredClientService>,
    pub products: ProductsServiceClient<TestLayeredClientService>,
    pub subscriptions: SubscriptionsServiceClient<TestLayeredClientService>,
    pub schedules: SchedulesServiceClient<TestLayeredClientService>,
    pub tenants: TenantsServiceClient<TestLayeredClientService>,
    pub users: UsersServiceClient<TestLayeredClientService>,
    pub webhooks_out: WebhooksServiceClient<TestLayeredClientService>,
}

impl AllClients {
    pub fn from_channel(channel: Channel, bearer_token: &str, tenant_slug: &str) -> AllClients {
        let service = Self::build_layered_client_service(channel, bearer_token, tenant_slug);

        Self {
            api_tokens: ApiTokensServiceClient::new(service.clone()),
            customers: CustomersServiceClient::new(service.clone()),
            metrics: BillableMetricsServiceClient::new(service.clone()),
            instance: InstanceServiceClient::new(service.clone()),
            plans: PlansServiceClient::new(service.clone()),
            price_components: PriceComponentsServiceClient::new(service.clone()),
            product_families: ProductFamiliesServiceClient::new(service.clone()),
            products: ProductsServiceClient::new(service.clone()),
            schedules: SchedulesServiceClient::new(service.clone()),
            subscriptions: SubscriptionsServiceClient::new(service.clone()),
            tenants: TenantsServiceClient::new(service.clone()),
            users: UsersServiceClient::new(service.clone()),
            webhooks_out: WebhooksServiceClient::new(service.clone()),
        }
    }

    pub fn build_layered_client_service(
        channel: Channel,
        bearer_token: &str,
        tenant_slug: &str,
    ) -> TestLayeredClientService {
        // if we have a tenant slug then we could resolve role via tenant
        // otherwise don't set it

        let header_name = if tenant_slug.is_empty() {
            "_fake"
        } else {
            TENANT_SLUG_HEADER
        };

        tower::ServiceBuilder::new()
            .layer(AddAuthorizationLayer::bearer(bearer_token))
            .layer(SetRequestHeaderLayer::if_not_present(
                HeaderName::from_str(header_name).unwrap(),
                HeaderValue::from_str(tenant_slug).unwrap(),
            ))
            .service(channel)
    }
}
