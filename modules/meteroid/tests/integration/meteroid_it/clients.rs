use std::str::FromStr;

use http::{HeaderName, HeaderValue};
use tonic::transport::Channel;
use tower_http::auth::{AddAuthorization, AddAuthorizationLayer};
use tower_http::set_header::{SetRequestHeader, SetRequestHeaderLayer};

use common_grpc::middleware::common::auth::INTERNAL_API_CONTEXT_HEADER;
use meteroid_grpc::meteroid::api::addons::v1::add_ons_service_client::AddOnsServiceClient;
use meteroid_grpc::meteroid::api::apitokens::v1::api_tokens_service_client::ApiTokensServiceClient;
use meteroid_grpc::meteroid::api::bankaccounts::v1::bank_accounts_service_client::BankAccountsServiceClient;
use meteroid_grpc::meteroid::api::billablemetrics::v1::billable_metrics_service_client::BillableMetricsServiceClient;
use meteroid_grpc::meteroid::api::components::v1::price_components_service_client::PriceComponentsServiceClient;
use meteroid_grpc::meteroid::api::connectors::v1::connectors_service_client::ConnectorsServiceClient;
use meteroid_grpc::meteroid::api::coupons::v1::coupons_service_client::CouponsServiceClient;
use meteroid_grpc::meteroid::api::customers::v1::customers_service_client::CustomersServiceClient;
use meteroid_grpc::meteroid::api::instance::v1::instance_service_client::InstanceServiceClient;
use meteroid_grpc::meteroid::api::invoicingentities::v1::invoicing_entities_service_client::InvoicingEntitiesServiceClient;
use meteroid_grpc::meteroid::api::organizations::v1::organizations_service_client::OrganizationsServiceClient;
use meteroid_grpc::meteroid::api::plans::v1::plans_service_client::PlansServiceClient;
use meteroid_grpc::meteroid::api::productfamilies::v1::product_families_service_client::ProductFamiliesServiceClient;
use meteroid_grpc::meteroid::api::products::v1::products_service_client::ProductsServiceClient;
use meteroid_grpc::meteroid::api::quotes::v1::quotes_service_client::QuotesServiceClient;
use meteroid_grpc::meteroid::api::schedules::v1::schedules_service_client::SchedulesServiceClient;
use meteroid_grpc::meteroid::api::stats::v1::stats_service_client::StatsServiceClient;
use meteroid_grpc::meteroid::api::subscriptions::v1::subscriptions_service_client::SubscriptionsServiceClient;
use meteroid_grpc::meteroid::api::tenants::v1::tenants_service_client::TenantsServiceClient;
use meteroid_grpc::meteroid::api::users::v1::users_service_client::UsersServiceClient;
use meteroid_grpc::meteroid::api::webhooks::out::v1::webhooks_service_client::WebhooksServiceClient;

pub type TestLayeredClientService = AddAuthorization<SetRequestHeader<Channel, HeaderValue>>;

pub struct AllClients {
    pub add_ons: AddOnsServiceClient<TestLayeredClientService>,
    pub api_tokens: ApiTokensServiceClient<TestLayeredClientService>,
    pub bank_accounts: BankAccountsServiceClient<TestLayeredClientService>,
    pub connectors: ConnectorsServiceClient<TestLayeredClientService>,
    pub coupons: CouponsServiceClient<TestLayeredClientService>,
    pub customers: CustomersServiceClient<TestLayeredClientService>,
    pub metrics: BillableMetricsServiceClient<TestLayeredClientService>,
    pub instance: InstanceServiceClient<TestLayeredClientService>,
    pub invoicing_entities: InvoicingEntitiesServiceClient<TestLayeredClientService>,
    pub plans: PlansServiceClient<TestLayeredClientService>,
    pub price_components: PriceComponentsServiceClient<TestLayeredClientService>,
    pub product_families: ProductFamiliesServiceClient<TestLayeredClientService>,
    pub products: ProductsServiceClient<TestLayeredClientService>,
    pub subscriptions: SubscriptionsServiceClient<TestLayeredClientService>,
    pub schedules: SchedulesServiceClient<TestLayeredClientService>,
    pub tenants: TenantsServiceClient<TestLayeredClientService>,
    pub users: UsersServiceClient<TestLayeredClientService>,
    pub webhooks_out: WebhooksServiceClient<TestLayeredClientService>,
    pub stats: StatsServiceClient<TestLayeredClientService>,
    pub organizations: OrganizationsServiceClient<TestLayeredClientService>,
    pub quotes: QuotesServiceClient<TestLayeredClientService>,
}

impl AllClients {
    pub fn from_channel(
        channel: Channel,
        bearer_token: &str,
        org_slug: &str,
        tenant_slug: &str,
    ) -> AllClients {
        let service =
            Self::build_layered_client_service(channel, bearer_token, org_slug, tenant_slug);

        Self {
            add_ons: AddOnsServiceClient::new(service.clone()),
            api_tokens: ApiTokensServiceClient::new(service.clone()),
            bank_accounts: BankAccountsServiceClient::new(service.clone()),
            connectors: ConnectorsServiceClient::new(service.clone()),
            coupons: CouponsServiceClient::new(service.clone()),
            customers: CustomersServiceClient::new(service.clone()),
            metrics: BillableMetricsServiceClient::new(service.clone()),
            instance: InstanceServiceClient::new(service.clone()),
            invoicing_entities: InvoicingEntitiesServiceClient::new(service.clone()),
            plans: PlansServiceClient::new(service.clone()),
            price_components: PriceComponentsServiceClient::new(service.clone()),
            product_families: ProductFamiliesServiceClient::new(service.clone()),
            products: ProductsServiceClient::new(service.clone()),
            schedules: SchedulesServiceClient::new(service.clone()),
            subscriptions: SubscriptionsServiceClient::new(service.clone()),
            tenants: TenantsServiceClient::new(service.clone()),
            users: UsersServiceClient::new(service.clone()),
            webhooks_out: WebhooksServiceClient::new(service.clone()),
            stats: StatsServiceClient::new(service.clone()),
            organizations: OrganizationsServiceClient::new(service.clone()),
            quotes: QuotesServiceClient::new(service.clone()),
        }
    }

    pub fn build_layered_client_service(
        channel: Channel,
        bearer_token: &str,
        org_slug: &str,
        tenant_slug: &str,
    ) -> TestLayeredClientService {
        // if we have a tenant slug then we could resolve role via tenant
        // otherwise don't set it

        let header_name = if tenant_slug.is_empty() {
            "_fake"
        } else {
            INTERNAL_API_CONTEXT_HEADER
        };

        tower::ServiceBuilder::new()
            .layer(AddAuthorizationLayer::bearer(bearer_token))
            .layer(SetRequestHeaderLayer::if_not_present(
                HeaderName::from_str(header_name).unwrap(),
                HeaderValue::from_str(format!("{}/{}", org_slug, tenant_slug).as_str()).unwrap(),
            ))
            .service(channel)
    }
}
