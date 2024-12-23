use error_stack::Report;
use tonic::{Request, Response, Status};
use uuid::Uuid;

use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::customers::v1::list_customer_request::SortBy;
use meteroid_grpc::meteroid::api::customers::v1::{
    customers_service_server::CustomersService, BuyCustomerCreditsRequest,
    BuyCustomerCreditsResponse, CreateCustomerRequest, CreateCustomerResponse, CustomerBrief,
    GetCustomerByAliasRequest, GetCustomerByAliasResponse, GetCustomerByIdRequest,
    GetCustomerByIdResponse, ListCustomerRequest, ListCustomerResponse, PatchCustomerRequest,
    PatchCustomerResponse, TopUpCustomerBalanceRequest, TopUpCustomerBalanceResponse,
};
use meteroid_store::domain;
use meteroid_store::domain::{
    CustomerBuyCredits, CustomerNew, CustomerPatch, CustomerTopUpBalance, Identity, OrderByRequest,
};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::CustomersInterface;

use crate::api::customers::error::CustomerApiError;
use crate::api::customers::mapping::customer::{
    DomainAddressWrapper, DomainBillingConfigWrapper, DomainShippingAddressWrapper,
    ServerCustomerBriefWrapper, ServerCustomerWrapper,
};
use crate::api::shared::conversions::FromProtoOpt;
use crate::api::utils::parse_uuid;
use crate::api::utils::PaginationExt;

use super::CustomerServiceComponents;

#[tonic::async_trait]
impl CustomersService for CustomerServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn create_customer(
        &self,
        request: Request<CreateCustomerRequest>,
    ) -> Result<Response<CreateCustomerResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor()?;

        let inner = request
            .into_inner()
            .data
            .ok_or(CustomerApiError::MissingArgument("no data".into()))?;

        let billing_config = match inner.billing_config {
            Some(b) => DomainBillingConfigWrapper::try_from(b)?.0,
            None => domain::BillingConfig::Manual,
        };

        let customer_new = CustomerNew {
            name: inner.name,
            created_by: actor,
            invoicing_entity_id: Uuid::from_proto_opt(inner.invoicing_entity_id)?
                .map(Identity::UUID),
            billing_config,
            alias: inner.alias,
            email: inner.email,
            invoicing_email: inner.invoicing_email,
            phone: inner.phone,
            balance_value_cents: 0,
            currency: inner.currency,
            billing_address: inner
                .billing_address
                .map(DomainAddressWrapper::try_from)
                .transpose()?
                .map(|v| v.0),
            shipping_address: inner
                .shipping_address
                .map(DomainShippingAddressWrapper::try_from)
                .transpose()?
                .map(|v| v.0),
            force_created_date: None,
        };

        let customer = self
            .store
            .insert_customer(customer_new, tenant_id)
            .await
            .and_then(ServerCustomerBriefWrapper::try_from)
            .map(|v| v.0)
            .map_err(Into::<CustomerApiError>::into)?;

        Ok(Response::new(CreateCustomerResponse {
            customer: Some(customer),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn patch_customer(
        &self,
        request: Request<PatchCustomerRequest>,
    ) -> Result<Response<PatchCustomerResponse>, Status> {
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
                    invoicing_entity_id: Uuid::from_proto_opt(customer.invoicing_entity_id)?,
                    currency: customer.currency.clone(),
                    billing_address: customer
                        .billing_address
                        .map(|s| serde_json::to_value(s).unwrap()),
                    shipping_address: customer
                        .shipping_address
                        .map(|s| serde_json::to_value(s).unwrap()),
                },
            )
            .await
            .map_err(Into::<CustomerApiError>::into)?;

        Ok(Response::new(PatchCustomerResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn list_customers(
        &self,
        request: Request<ListCustomerRequest>,
    ) -> Result<Response<ListCustomerResponse>, Status> {
        let tenant_id = request.tenant()?;

        let inner = request.into_inner();

        let pagination_req = domain::PaginationRequest {
            page: inner.pagination.as_ref().map(|p| p.offset).unwrap_or(0),
            per_page: inner.pagination.as_ref().map(|p| p.limit),
        };

        let order_by = match inner.sort_by.try_into() {
            Ok(SortBy::DateAsc) => OrderByRequest::DateAsc,
            Ok(SortBy::DateDesc) => OrderByRequest::DateDesc,
            Ok(SortBy::NameAsc) => OrderByRequest::NameAsc,
            Ok(SortBy::NameDesc) => OrderByRequest::NameDesc,
            Err(_) => OrderByRequest::DateDesc,
        };

        let res = self
            .store
            .list_customers(tenant_id, pagination_req, order_by, inner.search)
            .await
            .map_err(Into::<CustomerApiError>::into)?;

        let response = ListCustomerResponse {
            pagination_meta: inner.pagination.into_response(res.total_results as u32),
            customers: res
                .items
                .into_iter()
                .map(|l| ServerCustomerBriefWrapper::try_from(l).map(|v| v.0))
                .collect::<Vec<Result<CustomerBrief, Report<StoreError>>>>()
                .into_iter()
                .collect::<Result<Vec<_>, _>>()
                .map_err(Into::<CustomerApiError>::into)?,
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn get_customer_by_id(
        &self,
        request: Request<GetCustomerByIdRequest>,
    ) -> Result<Response<GetCustomerByIdResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();
        let customer_id = parse_uuid(&req.id, "id")?;

        let customer = self
            .store
            .find_customer_by_id(Identity::UUID(customer_id), tenant_id)
            .await
            .and_then(ServerCustomerWrapper::try_from)
            .map(|v| v.0)
            .map_err(Into::<CustomerApiError>::into)?;

        Ok(Response::new(GetCustomerByIdResponse {
            customer: Some(customer),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_customer_by_alias(
        &self,
        request: Request<GetCustomerByAliasRequest>,
    ) -> Result<Response<GetCustomerByAliasResponse>, Status> {
        let req = request.into_inner();

        let customer = self
            .store
            .find_customer_by_alias(req.alias.clone())
            .await
            .and_then(ServerCustomerWrapper::try_from)
            .map(|v| v.0)
            .map_err(Into::<CustomerApiError>::into)?;

        Ok(Response::new(GetCustomerByAliasResponse {
            customer: Some(customer),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn top_up_customer_balance(
        &self,
        request: Request<TopUpCustomerBalanceRequest>,
    ) -> Result<Response<TopUpCustomerBalanceResponse>, Status> {
        let actor = request.actor()?;
        let tenant_id = request.tenant()?;

        let req = request.into_inner();
        let customer_id = parse_uuid(&req.customer_id, "customer_id")?;

        let customer = self
            .store
            .top_up_customer_balance(CustomerTopUpBalance {
                created_by: actor,
                tenant_id,
                customer_id,
                cents: req.cents,
                notes: req.notes,
            })
            .await
            .and_then(ServerCustomerWrapper::try_from)
            .map(|v| v.0)
            .map_err(Into::<CustomerApiError>::into)?;

        Ok(Response::new(TopUpCustomerBalanceResponse {
            customer: Some(customer),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn buy_customer_credits(
        &self,
        request: Request<BuyCustomerCreditsRequest>,
    ) -> Result<Response<BuyCustomerCreditsResponse>, Status> {
        let actor = request.actor()?;
        let tenant_id = request.tenant()?;

        let req = request.into_inner();
        let customer_id = parse_uuid(&req.customer_id, "customer_id")?;

        let invoice = self
            .store
            .buy_customer_credits(CustomerBuyCredits {
                created_by: actor,
                tenant_id,
                customer_id,
                cents: req.cents,
                notes: req.notes,
            })
            .await
            .and_then(|inv| {
                crate::api::invoices::mapping::invoices::domain_invoice_with_plan_details_to_server(
                    inv,
                    self.jwt_secret.clone(),
                )
            })
            .map_err(Into::<CustomerApiError>::into)?;

        Ok(Response::new(BuyCustomerCreditsResponse {
            invoice: Some(invoice),
        }))
    }
}
