use common_domain::ids::{AliasOr, CustomerId, InvoicingEntityId};
use common_grpc::middleware::server::auth::RequestExt;
use error_stack::Report;
use meteroid_grpc::meteroid::api::customers::v1::list_customer_request::SortBy;
use meteroid_grpc::meteroid::api::customers::v1::{
    ArchiveCustomerRequest, ArchiveCustomerResponse, BuyCustomerCreditsRequest,
    BuyCustomerCreditsResponse, CreateCustomerRequest, CreateCustomerResponse, CustomerBrief,
    GetCustomerByAliasRequest, GetCustomerByAliasResponse, GetCustomerByIdRequest,
    GetCustomerByIdResponse, ListCustomerRequest, ListCustomerResponse, PatchCustomerRequest,
    PatchCustomerResponse, TopUpCustomerBalanceRequest, TopUpCustomerBalanceResponse,
    customers_service_server::CustomersService,
};
use meteroid_store::domain;
use meteroid_store::domain::{
    CustomerBuyCredits, CustomerNew, CustomerPatch, CustomerTopUpBalance, OrderByRequest,
};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::CustomersInterface;
use tonic::{Request, Response, Status};

use crate::api::customers::error::CustomerApiError;
use crate::api::customers::mapping::customer::{
    DomainAddressWrapper, DomainShippingAddressWrapper, ServerCustomerBriefWrapper,
    ServerCustomerWrapper,
};
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

        let customer_new = CustomerNew {
            name: inner.name,
            created_by: actor,
            invoicing_entity_id: InvoicingEntityId::from_proto_opt(inner.invoicing_entity_id)?,
            alias: inner.alias,
            billing_email: inner.billing_email,
            invoicing_emails: inner.invoicing_emails,
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
            // TODO
            bank_account_id: None,
            vat_number: None,
            custom_vat_rate: None,
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

        let billing_address = customer
            .billing_address
            .map(DomainAddressWrapper::try_from)
            .transpose()?
            .map(|v| v.0);
        let shipping_address = customer
            .shipping_address
            .map(DomainShippingAddressWrapper::try_from)
            .transpose()?
            .map(|v| v.0);

        let _ = self
            .store
            .patch_customer(
                actor,
                tenant_id,
                CustomerPatch {
                    id: CustomerId::from_proto(&customer.id)?,
                    name: customer.name.clone(),
                    alias: customer.alias.clone(),
                    billing_email: customer.billing_email.clone(),
                    invoicing_emails: customer.invoicing_emails.map(|v| v.emails),
                    phone: customer.phone.clone(),
                    balance_value_cents: customer.balance_value_cents,
                    invoicing_entity_id: InvoicingEntityId::from_proto_opt(
                        customer.invoicing_entity_id,
                    )?,
                    currency: customer.currency.clone(),
                    billing_address,
                    shipping_address,
                    // TODO
                    vat_number: None,
                    custom_vat_rate: None,
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
        let customer_id = CustomerId::from_proto(&req.id)?;

        let customer = self
            .store
            .find_customer_by_id(customer_id, tenant_id)
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
        let customer_id = CustomerId::from_proto(&req.customer_id)?;

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
        let customer_id = CustomerId::from_proto(&req.customer_id)?;

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

    async fn archive_customer(
        &self,
        request: Request<ArchiveCustomerRequest>,
    ) -> Result<Response<ArchiveCustomerResponse>, Status> {
        let actor = request.actor()?;
        let tenant_id = request.tenant()?;

        let req = request.into_inner();
        let customer_id = CustomerId::from_proto(&req.id)?;

        self.store
            .archive_customer(actor, tenant_id, AliasOr::Id(customer_id))
            .await
            .map_err(Into::<CustomerApiError>::into)?;

        Ok(Response::new(ArchiveCustomerResponse {}))
    }
}
