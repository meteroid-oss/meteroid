use error_stack::Report;
use tonic::{Request, Response, Status};

use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::customers::v1::{
    customers_service_server::CustomersService, CreateCustomerRequest, CreateCustomerResponse,
    Customer, CustomerList, GetCustomerByAliasRequest, GetCustomerRequest, ListCustomerRequest,
    ListCustomerResponse, PatchCustomerRequest, PatchCustomerResponse,
};
use meteroid_store::domain;
use meteroid_store::domain::{CustomerNew, CustomerPatch, OrderByRequest};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::CustomersInterface;

use crate::api::customers::error::CustomerApiError;
use crate::api::utils::parse_uuid;
use crate::api::utils::PaginationExt;

use super::{mapping, CustomerServiceComponents};

#[tonic::async_trait]
impl CustomersService for CustomerServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn create_customer(
        &self,
        request: tonic::Request<CreateCustomerRequest>,
    ) -> std::result::Result<tonic::Response<CreateCustomerResponse>, tonic::Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor()?;

        let inner = request.into_inner();

        let billing_config = inner
            .billing_config
            .ok_or_else(|| CustomerApiError::MissingArgument("billing_config".to_string()))
            .and_then(mapping::customer::billing_config_server_to_domain)?;

        let customer = self
            .store
            .insert_customer(CustomerNew {
                name: inner.name,
                created_by: actor,
                tenant_id: tenant_id,
                billing_config: Some(billing_config),
                alias: inner.alias,
                email: inner.email,
                invoicing_email: None,
                phone: None,
                balance_value_cents: 0,
                balance_currency: "EUR".to_string(),
                billing_address: None,
                shipping_address: None,
                created_at: None,
            })
            .await
            .and_then(mapping::customer::list_db_to_server)
            .map_err(Into::<crate::api::customers::error::CustomerApiError>::into)?;

        Ok(Response::new(CreateCustomerResponse {
            customer: Some(customer),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn patch_customer(
        &self,
        request: tonic::Request<PatchCustomerRequest>,
    ) -> std::result::Result<tonic::Response<PatchCustomerResponse>, tonic::Status> {
        let actor = request.actor()?;
        let tenant_id = request.tenant()?;

        let customer = request
            .into_inner()
            .customer
            .ok_or(CustomerApiError::MissingArgument(
                "customer payload missing".to_string(),
            ))?;

        let _ = self
            .store
            .patch_customer(
                actor,
                tenant_id,
                CustomerPatch {
                    id: parse_uuid(&customer.id, "id")?,
                    name: customer.name.clone(),
                    alias: customer.alias.clone(),
                    email: customer.email.clone(),
                    invoicing_email: customer.invoicing_email.clone(),
                    phone: customer.phone.clone(),
                    balance_value_cents: customer.balance_value_cents,
                    balance_currency: customer.balance_currency.clone(),
                    billing_address: customer
                        .billing_address
                        .map(|s| serde_json::to_value(s).unwrap()),
                    shipping_address: customer
                        .shipping_address
                        .map(|s| serde_json::to_value(s).unwrap()),
                },
            )
            .await
            .map_err(Into::<crate::api::customers::error::CustomerApiError>::into)?;

        Ok(Response::new(PatchCustomerResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn list_customers(
        &self,
        request: tonic::Request<ListCustomerRequest>,
    ) -> std::result::Result<tonic::Response<ListCustomerResponse>, tonic::Status> {
        let tenant_id = request.tenant()?;

        let inner = request.into_inner();

        let pagination_req = domain::PaginationRequest {
            page: inner.pagination.as_ref().map(|p| p.offset).unwrap_or(0),
            per_page: inner.pagination.as_ref().map(|p| p.limit),
        };

        let order_by = match inner.sort_by.try_into() {
            Ok(meteroid_grpc::meteroid::api::customers::v1::list_customer_request::SortBy::DateAsc) => OrderByRequest::DateAsc,
            Ok(meteroid_grpc::meteroid::api::customers::v1::list_customer_request::SortBy::DateDesc) => OrderByRequest::DateDesc,
            Ok(meteroid_grpc::meteroid::api::customers::v1::list_customer_request::SortBy::NameAsc) => OrderByRequest::NameAsc,
            Ok(meteroid_grpc::meteroid::api::customers::v1::list_customer_request::SortBy::NameDesc) => OrderByRequest::NameDesc,
            Err(_) => OrderByRequest::DateDesc,
        };

        let res = self
            .store
            .list_customers(tenant_id, pagination_req, order_by, inner.search)
            .await
            .map_err(Into::<crate::api::customers::error::CustomerApiError>::into)?;

        let response = ListCustomerResponse {
            pagination_meta: inner.pagination.into_response(res.total_results as u32),
            customers: res
                .items
                .into_iter()
                .map(|l| crate::api::customers::mapping::customer::list_db_to_server(l))
                .collect::<Vec<Result<CustomerList, Report<StoreError>>>>()
                .into_iter()
                .collect::<Result<Vec<_>, _>>()
                .map_err(Into::<crate::api::customers::error::CustomerApiError>::into)?,
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn get_customer(
        &self,
        request: Request<GetCustomerRequest>,
    ) -> Result<Response<Customer>, Status> {
        let req = request.into_inner();
        let customer_id = parse_uuid(&req.id, "id")?;

        let customer = self
            .store
            .find_customer_by_id(customer_id.clone())
            .await
            .and_then(mapping::customer::domain_to_server)
            .map_err(Into::<crate::api::customers::error::CustomerApiError>::into)?;

        Ok(Response::new(customer))
    }

    #[tracing::instrument(skip_all)]
    async fn get_customer_by_alias(
        &self,
        request: Request<GetCustomerByAliasRequest>,
    ) -> Result<Response<Customer>, Status> {
        let req = request.into_inner();

        let customer = self
            .store
            .find_customer_by_alias(req.alias.clone())
            .await
            .and_then(mapping::customer::domain_to_server)
            .map_err(Into::<crate::api::customers::error::CustomerApiError>::into)?;

        Ok(Response::new(customer))
    }
}
