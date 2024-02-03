use common_grpc::middleware::server::auth::RequestExt;
use cornucopia_async::Params;
use meteroid_repository as db;
use std::sync::Arc;
use tonic::{Request, Response, Status};

use crate::{
    api::services::utils::{parse_uuid, uuid_gen},
    db::DbService,
};

use crate::api::services::utils::PaginationExt;
use meteroid_grpc::meteroid::api::customers::v1::{
    customers_service_server::CustomersService, list_customer_request::SortBy,
    CreateCustomerRequest, CreateCustomerResponse, Customer, GetCustomerByAliasRequest,
    GetCustomerRequest, ListCustomerRequest, ListCustomerResponse,
};

use super::mapping;

#[tonic::async_trait]
impl CustomersService for DbService {
    #[tracing::instrument(skip_all)]
    async fn create_customer(
        &self,
        request: tonic::Request<CreateCustomerRequest>,
    ) -> std::result::Result<tonic::Response<CreateCustomerResponse>, tonic::Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor()?;

        let inner = request.into_inner();
        let connection = self.get_connection().await?;

        let serialized_config = inner
            .billing_config
            .ok_or_else(|| Status::invalid_argument("Missing billing_config"))
            .and_then(|billing_config| {
                serde_json::to_value(&billing_config).map_err(|e| {
                    Status::invalid_argument(format!("Failed to serialize billing_config: {}", e))
                })
            })?;

        let params = db::customers::CreateCustomerParams {
            id: uuid_gen::v7(),
            name: inner.name,
            alias: inner.alias,
            tenant_id,
            created_by: actor,
            billing_config: serialized_config,
        };

        let customer = db::customers::create_customer()
            .params(&connection, &params)
            .one()
            .await
            .map_err(|e| {
                Status::internal("Failed to create customer")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let rs = mapping::customer::db_to_server(customer).map_err(|e| {
            Status::internal("Failed to map db customer to proto")
                .set_source(Arc::new(e))
                .clone()
        })?;

        Ok(Response::new(CreateCustomerResponse { customer: Some(rs) }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_customers(
        &self,
        request: tonic::Request<ListCustomerRequest>,
    ) -> std::result::Result<tonic::Response<ListCustomerResponse>, tonic::Status> {
        let tenant_id = request.tenant()?;

        let inner = request.into_inner();
        let connection = self.get_connection().await?;

        let params = db::customers::ListCustomersParams {
            tenant_id,
            limit: inner.pagination.limit(),
            offset: inner.pagination.offset(),
            order_by: match inner.sort_by.try_into() {
                Ok(SortBy::DateAsc) => "DATE_ASC",
                Ok(SortBy::DateDesc) => "DATE_DESC",
                Ok(SortBy::NameAsc) => "NAME_ASC",
                Ok(SortBy::NameDesc) => "NAME_DESC",
                Err(_) => "DATE_DESC",
            },
            search: inner.search,
        };

        let customers = db::customers::list_customers()
            .params(&connection, &params)
            .all()
            .await
            .map_err(|e| {
                Status::internal("Failed to list customers")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let total = customers.first().map(|c| c.total_count).unwrap_or(0);

        let customers = customers
            .into_iter()
            .map(|c| mapping::customer::list_db_to_server(c).unwrap())
            .collect();

        Ok(Response::new(ListCustomerResponse {
            customers,
            pagination_meta: inner.pagination.into_response(total as u32),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_customer(
        &self,
        request: Request<GetCustomerRequest>,
    ) -> Result<Response<Customer>, Status> {
        let req = request.into_inner();
        let connection = self.get_connection().await?;
        let id = parse_uuid(&req.id, "id")?;

        let customer = db::customers::get_customer_by_id()
            .bind(&connection, &id)
            .one()
            .await
            .map_err(|e| {
                Status::internal("Failed to get customer")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let rs = mapping::customer::db_to_server(customer).map_err(|e| {
            Status::internal("Failed to map db customer to proto")
                .set_source(Arc::new(e))
                .clone()
        })?;

        Ok(Response::new(rs))
    }

    #[tracing::instrument(skip_all)]
    async fn get_customer_by_alias(
        &self,
        request: Request<GetCustomerByAliasRequest>,
    ) -> Result<Response<Customer>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();
        let connection = self.get_connection().await?;

        let customer = db::customers::get_customer_by_alias()
            .bind(&connection, &tenant_id, &req.alias)
            .one()
            .await
            .map_err(|e| {
                Status::internal("Failed to get customer by alias")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let rs = mapping::customer::db_to_server(customer).map_err(|e| {
            Status::internal("Failed to map db customer to proto")
                .set_source(Arc::new(e))
                .clone()
        })?;

        Ok(Response::new(rs))
    }
}
