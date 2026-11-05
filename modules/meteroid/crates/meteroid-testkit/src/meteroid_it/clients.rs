use std::str::FromStr;

use http::{HeaderName, HeaderValue};
use tonic::transport::Channel;
use tower_http::auth::{AddAuthorization, AddAuthorizationLayer};
use tower_http::set_header::{SetRequestHeader, SetRequestHeaderLayer};

use common_grpc::middleware::common::auth::INTERNAL_API_CONTEXT_HEADER;
use meteroid_grpc::meteroid::api::addons::v1::add_ons_service_client::AddOnsServiceClient;
use meteroid_grpc::meteroid::api::alerts::v1::alerts_service_client::AlertsServiceClient;
use meteroid_grpc::meteroid::api::analytics::v1::analytics_service_client::AnalyticsServiceClient;
use meteroid_grpc::meteroid::api::apitokens::v1::api_tokens_service_client::ApiTokensServiceClient;
use meteroid_grpc::meteroid::api::bankaccounts::v1::bank_accounts_service_client::BankAccountsServiceClient;
use meteroid_grpc::meteroid::api::billablemetrics::v1::billable_metrics_service_client::BillableMetricsServiceClient;
use meteroid_grpc::meteroid::api::components::v1::price_components_service_client::PriceComponentsServiceClient;
use meteroid_grpc::meteroid::api::connectors::v1::connectors_service_client::ConnectorsServiceClient;
use meteroid_grpc::meteroid::api::coupons::v1::coupons_service_client::CouponsServiceClient;
use meteroid_grpc::meteroid::api::creditunits::v1::credit_units_service_client::CreditUnitsServiceClient;
use meteroid_grpc::meteroid::api::customeremails::v1::customer_emails_service_client::CustomerEmailsServiceClient;
use meteroid_grpc::meteroid::api::customers::v1::customers_service_client::CustomersServiceClient;
use meteroid_grpc::meteroid::api::entitlements::v1::entitlements_service_client::EntitlementsServiceClient;
use meteroid_grpc::meteroid::api::eventinsights::v1::event_insights_service_client::EventInsightsServiceClient;
use meteroid_grpc::meteroid::api::instance::v1::instance_service_client::InstanceServiceClient;
use meteroid_grpc::meteroid::api::invoices::v1::invoices_service_client::InvoicesServiceClient;
use meteroid_grpc::meteroid::api::invoicingentities::v1::invoicing_entities_service_client::InvoicingEntitiesServiceClient;
use meteroid_grpc::meteroid::api::listviews::v1::list_views_service_client::ListViewsServiceClient;
use meteroid_grpc::meteroid::api::organizations::v1::organizations_service_client::OrganizationsServiceClient;
use meteroid_grpc::meteroid::api::paymentreminders::v1::payment_reminders_service_client::PaymentRemindersServiceClient;
use meteroid_grpc::meteroid::api::plans::v1::plans_service_client::PlansServiceClient;
use meteroid_grpc::meteroid::api::productfamilies::v1::product_families_service_client::ProductFamiliesServiceClient;
use meteroid_grpc::meteroid::api::products::v1::products_service_client::ProductsServiceClient;
use meteroid_grpc::meteroid::api::quotes::v1::quotes_service_client::QuotesServiceClient;
use meteroid_grpc::meteroid::api::schedules::v1::schedules_service_client::SchedulesServiceClient;
use meteroid_grpc::meteroid::api::signals::v1::signals_service_client::SignalsServiceClient;
use meteroid_grpc::meteroid::api::stats::v1::stats_service_client::StatsServiceClient;
use meteroid_grpc::meteroid::api::subscriptions::v1::subscriptions_service_client::SubscriptionsServiceClient;
use meteroid_grpc::meteroid::api::targets::v1::targets_service_client::TargetsServiceClient;
use meteroid_grpc::meteroid::api::tenants::v1::tenants_service_client::TenantsServiceClient;
use meteroid_grpc::meteroid::api::users::v1::users_service_client::UsersServiceClient;
use meteroid_grpc::meteroid::api::wallets::v1::wallets_service_client::WalletsServiceClient;
use meteroid_grpc::meteroid::api::webhooks::out::v1::webhooks_service_client::WebhooksServiceClient;

pub type TestLayeredClientService = AddAuthorization<SetRequestHeader<Channel, HeaderValue>>;

#[allow(dead_code)]
pub struct AllClients {
    pub add_ons: AddOnsServiceClient<TestLayeredClientService>,
    pub alerts: AlertsServiceClient<TestLayeredClientService>,
    pub entitlements: EntitlementsServiceClient<TestLayeredClientService>,
    pub api_tokens: ApiTokensServiceClient<TestLayeredClientService>,
    pub bank_accounts: BankAccountsServiceClient<TestLayeredClientService>,
    pub connectors: ConnectorsServiceClient<TestLayeredClientService>,
    pub coupons: CouponsServiceClient<TestLayeredClientService>,
    pub credit_units: CreditUnitsServiceClient<TestLayeredClientService>,
    pub customers: CustomersServiceClient<TestLayeredClientService>,
    pub customer_emails: CustomerEmailsServiceClient<TestLayeredClientService>,
    pub metrics: BillableMetricsServiceClient<TestLayeredClientService>,
    pub instance: InstanceServiceClient<TestLayeredClientService>,
    pub invoices: InvoicesServiceClient<TestLayeredClientService>,
    pub invoicing_entities: InvoicingEntitiesServiceClient<TestLayeredClientService>,
    pub list_views: ListViewsServiceClient<TestLayeredClientService>,
    pub payment_reminders: PaymentRemindersServiceClient<TestLayeredClientService>,
    pub plans: PlansServiceClient<TestLayeredClientService>,
    pub price_components: PriceComponentsServiceClient<TestLayeredClientService>,
    pub product_families: ProductFamiliesServiceClient<TestLayeredClientService>,
    pub products: ProductsServiceClient<TestLayeredClientService>,
    pub subscriptions: SubscriptionsServiceClient<TestLayeredClientService>,
    pub schedules: SchedulesServiceClient<TestLayeredClientService>,
    pub signals: SignalsServiceClient<TestLayeredClientService>,
    pub targets: TargetsServiceClient<TestLayeredClientService>,
    pub tenants: TenantsServiceClient<TestLayeredClientService>,
    pub users: UsersServiceClient<TestLayeredClientService>,
    pub wallets: WalletsServiceClient<TestLayeredClientService>,
    pub webhooks_out: WebhooksServiceClient<TestLayeredClientService>,
    pub stats: StatsServiceClient<TestLayeredClientService>,
    pub analytics: AnalyticsServiceClient<TestLayeredClientService>,
    pub event_insights: EventInsightsServiceClient<TestLayeredClientService>,
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
            alerts: AlertsServiceClient::new(service.clone()),
            entitlements: EntitlementsServiceClient::new(service.clone()),
            api_tokens: ApiTokensServiceClient::new(service.clone()),
            bank_accounts: BankAccountsServiceClient::new(service.clone()),
            connectors: ConnectorsServiceClient::new(service.clone()),
            coupons: CouponsServiceClient::new(service.clone()),
            credit_units: CreditUnitsServiceClient::new(service.clone()),
            customers: CustomersServiceClient::new(service.clone()),
            customer_emails: CustomerEmailsServiceClient::new(service.clone()),
            metrics: BillableMetricsServiceClient::new(service.clone()),
            instance: InstanceServiceClient::new(service.clone()),
            invoices: InvoicesServiceClient::new(service.clone()),
            invoicing_entities: InvoicingEntitiesServiceClient::new(service.clone()),
            list_views: ListViewsServiceClient::new(service.clone()),
            payment_reminders: PaymentRemindersServiceClient::new(service.clone()),
            plans: PlansServiceClient::new(service.clone()),
            price_components: PriceComponentsServiceClient::new(service.clone()),
            product_families: ProductFamiliesServiceClient::new(service.clone()),
            products: ProductsServiceClient::new(service.clone()),
            schedules: SchedulesServiceClient::new(service.clone()),
            signals: SignalsServiceClient::new(service.clone()),
            targets: TargetsServiceClient::new(service.clone()),
            subscriptions: SubscriptionsServiceClient::new(service.clone()),
            tenants: TenantsServiceClient::new(service.clone()),
            users: UsersServiceClient::new(service.clone()),
            wallets: WalletsServiceClient::new(service.clone()),
            webhooks_out: WebhooksServiceClient::new(service.clone()),
            stats: StatsServiceClient::new(service.clone()),
            analytics: AnalyticsServiceClient::new(service.clone()),
            event_insights: EventInsightsServiceClient::new(service.clone()),
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
