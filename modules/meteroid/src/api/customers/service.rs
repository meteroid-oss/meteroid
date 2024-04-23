use common_eventbus::Event;
use cornucopia_async::Params;
use tonic::{Request, Response, Status};

use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::customers::v1::{
    customers_service_server::CustomersService, list_customer_request::SortBy,
    CreateCustomerRequest, CreateCustomerResponse, Customer, GetCustomerByAliasRequest,
    GetCustomerRequest, ListCustomerRequest, ListCustomerResponse, PatchCustomerRequest,
    PatchCustomerResponse,
};
use meteroid_repository as db;

use crate::api::customers::error::CustomerApiError;
use crate::api::utils::PaginationExt;
use crate::api::utils::{parse_uuid, uuid_gen};

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
        let connection = self.get_connection().await?;

        let serialized_config = inner
            .billing_config
            .ok_or_else(|| CustomerApiError::MissingArgument("billing_config".to_string()))
            .and_then(|billing_config| {
                serde_json::to_value(&billing_config).map_err(|e| {
                    CustomerApiError::SerializationError(
                        "failed to serialize billing_config".to_string(),
                        e,
                    )
                })
            })?;

        let params = db::customers::CreateCustomerParams {
            id: uuid_gen::v7(),
            name: inner.name,
            alias: inner.alias,
            email: inner.email,
            tenant_id,
            created_by: actor,
            billing_config: serialized_config,
        };

        let customer = db::customers::create_customer()
            .params(&connection, &params)
            .one()
            .await
            .map_err(|e| {
                CustomerApiError::DatabaseError("Failed to create customer".to_string(), e)
            })?;

        let _ = self
            .eventbus
            .publish(Event::customer_created(actor, customer.id, tenant_id))
            .await;

        let rs = mapping::customer::create_db_to_server(customer).map_err(|e| {
            CustomerApiError::MappingError("failed to map db customer to proto".to_string(), e)
        })?;

        Ok(Response::new(CreateCustomerResponse { customer: Some(rs) }))
    }

    #[tracing::instrument(skip_all)]
    async fn patch_customer(
        &self,
        request: tonic::Request<PatchCustomerRequest>,
    ) -> std::result::Result<tonic::Response<PatchCustomerResponse>, tonic::Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor()?;
        let connection = self.get_connection().await?;

        let customer = request
            .into_inner()
            .customer
            .ok_or(CustomerApiError::MissingArgument(
                "customer payload missing".to_string(),
            ))?;

        let saved_customer = db::customers::get_customer_by_id()
            .bind(&connection, &parse_uuid(&customer.id, "id")?)
            .one()
            .await
            .map_err(|e| {
                CustomerApiError::DatabaseError("failed to get customer".to_string(), e)
            })?;

        let params = db::customers::PatchCustomerParams {
            id: parse_uuid(&customer.id, "id")?,
            name: customer.name.unwrap_or(saved_customer.name),
            alias: customer.alias.or(saved_customer.alias),
            email: customer.email.or(saved_customer.email),
            invoicing_email: customer.invoicing_email.or(saved_customer.invoicing_email),
            phone: customer.phone.or(saved_customer.phone),
            balance_value_cents: customer
                .balance_value_cents
                .unwrap_or(saved_customer.balance_value_cents),
            balance_currency: customer
                .balance_currency
                .unwrap_or(saved_customer.balance_currency),
            billing_address: customer
                .billing_address
                .map(|s| serde_json::to_value(s).unwrap())
                .or(saved_customer.billing_address),
            shipping_address: customer
                .shipping_address
                .map(|s| serde_json::to_value(s).unwrap())
                .or(saved_customer.shipping_address),
        };

        db::customers::patch_customer()
            .params(&connection, &params)
            .await
            .map_err(|e| {
                CustomerApiError::DatabaseError("Failed to patch customer".to_string(), e)
            })?;

        let _ = self
            .eventbus
            .publish(Event::customer_patched(actor, saved_customer.id, tenant_id))
            .await;

        Ok(Response::new(PatchCustomerResponse {}))
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
                CustomerApiError::DatabaseError("Failed to list customers".to_string(), e)
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
                CustomerApiError::DatabaseError("failed to get customer".to_string(), e)
            })?;

        let rs = mapping::customer::db_to_server(customer).map_err(|e| {
            CustomerApiError::MappingError("failed to map db customer to proto".to_string(), e)
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
                CustomerApiError::DatabaseError("failed to get customer by alias".to_string(), e)
            })?;

        let rs = mapping::customer::db_to_server(customer).map_err(|e| {
            CustomerApiError::MappingError("failed to map db customer to proto".to_string(), e)
        })?;

        Ok(Response::new(rs))
    }
}
