use super::CustomerServiceComponents;
use crate::api::customers::error::CustomerApiError;
use crate::api::customers::mapping::customer::{
    DomainAddressWrapper, DomainShippingAddressWrapper, ServerCustomerBriefWrapper,
    ServerCustomerWrapper,
};
use crate::api::utils::PaginationExt;
use common_domain::ids::{AliasOr, BankAccountId, BaseId, ConnectorId, CustomerConnectionId, CustomerId, InvoicingEntityId};
use common_grpc::middleware::server::auth::RequestExt;
use error_stack::Report;
use meteroid_grpc::meteroid::api::customers::v1::list_customer_request::SortBy;
use meteroid_grpc::meteroid::api::customers::v1::{
    ArchiveCustomerRequest, ArchiveCustomerResponse, BuyCustomerCreditsRequest,
    BuyCustomerCreditsResponse, CreateCustomerRequest, CreateCustomerResponse, CustomerBrief,
    DeleteCustomerConnectionRequest, DeleteCustomerConnectionResponse,
    GenerateCustomerPortalTokenRequest, GenerateCustomerPortalTokenResponse,
    GetCustomerByAliasRequest, GetCustomerByAliasResponse, GetCustomerByIdRequest,
    GetCustomerByIdResponse, ListCustomerRequest, ListCustomerResponse, SyncToHubspotRequest,
    SyncToHubspotResponse, SyncToPennylaneRequest, SyncToPennylaneResponse,
    TopUpCustomerBalanceRequest, TopUpCustomerBalanceResponse, UnarchiveCustomerRequest,
    UnarchiveCustomerResponse,UpdateCustomerRequest, UpdateCustomerResponse, UpsertCustomerConnectionRequest,
    UpsertCustomerConnectionResponse, customers_service_server::CustomersService,
};
use meteroid_store::domain::{
    CustomerBuyCredits, CustomerNew, CustomerPatch, CustomerTopUpBalance, OrderByRequest,
};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::CustomersInterface;
use meteroid_store::repositories::customer_connection::CustomerConnectionInterface;
use meteroid_store::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use meteroid_store::repositories::connectors::ConnectorsInterface;
use tonic::{Request, Response, Status};

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
            bank_account_id: BankAccountId::from_proto_opt(inner.bank_account_id)?,
            vat_number: inner.vat_number,
            custom_taxes: inner
                .custom_taxes
                .into_iter()
                .map(
                    |t| -> Result<meteroid_store::domain::CustomerCustomTax, CustomerApiError> {
                        Ok(meteroid_store::domain::CustomerCustomTax {
                            tax_code: t.tax_code,
                            name: t.name,
                            rate: t.rate.parse().map_err(|_| {
                                CustomerApiError::InvalidArgument("Invalid tax rate".to_string())
                            })?,
                        })
                    },
                )
                .collect::<Result<Vec<_>, _>>()?,
            is_tax_exempt: inner.is_tax_exempt.unwrap_or(false),
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
    async fn update_customer(
        &self,
        request: Request<UpdateCustomerRequest>,
    ) -> Result<Response<UpdateCustomerResponse>, Status> {
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
                    vat_number: Some(customer.vat_number),
                    custom_taxes: crate::api::customers::mapping::customer::custom_taxes_from_grpc(
                        customer.custom_taxes,
                    )
                    .map_err(Into::<Status>::into)?,
                    bank_account_id: Some(BankAccountId::from_proto_opt(customer.bank_account_id)?),
                    current_payment_method_id: None,
                    is_tax_exempt: customer.is_tax_exempt,
                },
            )
            .await
            .map_err(Into::<CustomerApiError>::into)?;

        tracing::info!("Customer updated: {}", customer.id);

        Ok(Response::new(UpdateCustomerResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn list_customers(
        &self,
        request: Request<ListCustomerRequest>,
    ) -> Result<Response<ListCustomerResponse>, Status> {
        let tenant_id = request.tenant()?;

        let inner = request.into_inner();

        let pagination_req = inner.pagination.into_domain();

        let order_by = match inner.sort_by.try_into() {
            Ok(SortBy::DateAsc) => OrderByRequest::DateAsc,
            Ok(SortBy::DateDesc) => OrderByRequest::DateDesc,
            Ok(SortBy::NameAsc) => OrderByRequest::NameAsc,
            Ok(SortBy::NameDesc) => OrderByRequest::NameDesc,
            Err(_) => OrderByRequest::DateDesc,
        };

        let res = self
            .store
            .list_customers(
                tenant_id,
                pagination_req,
                order_by,
                inner.search,
                inner.archived,
            )
            .await
            .map_err(Into::<CustomerApiError>::into)?;

        let response = ListCustomerResponse {
            pagination_meta: inner
                .pagination
                .into_response(res.total_pages, res.total_results),
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
            .map_err(Into::<CustomerApiError>::into)?;

        // Fetch customer connections
        let customer_connections = self
            .store
            .list_connections_by_customer_id(&tenant_id, &customer_id)
            .await
            .map_err(Into::<CustomerApiError>::into)?;

        // Fetch all connectors to map connection IDs to provider types
        let connectors = self
            .store
            .list_connectors(None, tenant_id)
            .await
            .map_err(Into::<CustomerApiError>::into)?;

        // Fetch customer payment methods
        let payment_methods = self
            .store
            .list_payment_methods_by_customer(&tenant_id, &customer_id)
            .await
            .map_err(Into::<CustomerApiError>::into)?;

        let customer_proto = ServerCustomerWrapper::try_from(customer)
            .map(|mut v| {
                // Map customer connections to proto
                v.0.customer_connections = customer_connections
                    .into_iter()
                    .filter_map(|conn| {
                        // Find the connector to get the provider type and external_company_id
                        connectors
                            .iter()
                            .find(|c| c.id == conn.connector_id)
                            .map(|connector| {
                                let external_company_id = match &connector.data {
                                    Some(meteroid_store::domain::connectors::ProviderData::Hubspot(data)) => {
                                        Some(data.external_company_id.clone())
                                    }
                                    Some(meteroid_store::domain::connectors::ProviderData::Pennylane(data)) => {
                                        Some(data.external_company_id.clone())
                                    }
                                    _ => None,
                                };
                                crate::api::customers::mapping::customer::map_customer_connection_to_proto(
                                    conn,
                                    connector.provider.clone(),
                                    external_company_id,
                                )
                            })
                    })
                    .collect();
                // Map payment methods to proto
                v.0.payment_methods = payment_methods
                    .into_iter()
                    .map(crate::api::customers::mapping::customer_payment_method::domain_to_server)
                    .collect();
                v.0
            })
            .map_err(Into::<CustomerApiError>::into)?;

        Ok(Response::new(GetCustomerByIdResponse {
            customer: Some(customer_proto),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_customer_by_alias(
        &self,
        request: Request<GetCustomerByAliasRequest>,
    ) -> Result<Response<GetCustomerByAliasResponse>, Status> {
        let tenant = request.tenant()?;

        let req = request.into_inner();

        let customer = self
            .store
            .find_customer_by_alias(req.alias.clone(), tenant)
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
            .service
            .buy_customer_credits(CustomerBuyCredits {
                created_by: actor,
                tenant_id,
                customer_id,
                cents: req.cents,
                notes: req.notes,
            })
            .await
            .and_then(|inv| {
                crate::api::invoices::mapping::invoices::domain_invoice_with_transactions_to_server(
                    inv.invoice,
                    inv.transactions,
                    self.jwt_secret.clone(),
                )
            })
            .map_err(Into::<CustomerApiError>::into)?;

        Ok(Response::new(BuyCustomerCreditsResponse {
            invoice: Some(invoice),
        }))
    }

    #[tracing::instrument(skip_all)]
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

    #[tracing::instrument(skip_all)]
    async fn unarchive_customer(
        &self,
        request: Request<UnarchiveCustomerRequest>,
    ) -> Result<Response<UnarchiveCustomerResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();
        let customer_id = CustomerId::from_proto(&req.id)?;

        self.store
            .unarchive_customer(tenant_id, AliasOr::Id(customer_id))
            .await
            .map_err(Into::<CustomerApiError>::into)?;

        Ok(Response::new(UnarchiveCustomerResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn sync_to_hubspot(
        &self,
        request: Request<SyncToHubspotRequest>,
    ) -> Result<Response<SyncToHubspotResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();

        let ids = req
            .customer_ids
            .iter()
            .map(CustomerId::from_proto)
            .map(|id| id.map(AliasOr::Id))
            .collect::<Result<Vec<_>, _>>()?;

        self.store
            .sync_customers_to_hubspot(ids, tenant_id)
            .await
            .map_err(Into::<CustomerApiError>::into)?;

        Ok(Response::new(SyncToHubspotResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn sync_to_pennylane(
        &self,
        request: Request<SyncToPennylaneRequest>,
    ) -> Result<Response<SyncToPennylaneResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();

        let ids = req
            .customer_ids
            .iter()
            .map(CustomerId::from_proto)
            .map(|id| id.map(AliasOr::Id))
            .collect::<Result<Vec<_>, _>>()?;

        self.store
            .sync_customers_to_pennylane(ids, tenant_id)
            .await
            .map_err(Into::<CustomerApiError>::into)?;

        Ok(Response::new(SyncToPennylaneResponse {}))
    }


    #[tracing::instrument(skip_all)]
    async fn generate_customer_portal_token(
        &self,
        request: Request<GenerateCustomerPortalTokenRequest>,
    ) -> Result<Response<GenerateCustomerPortalTokenResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();
        let customer_id = CustomerId::from_proto(req.customer_id)?;

        // Generate the JWT token for customer portal access
        let token = meteroid_store::jwt_claims::generate_portal_token(
            &self.jwt_secret,
            tenant_id,
            meteroid_store::jwt_claims::ResourceAccess::Customer(customer_id),
        )
            .map_err(Into::<CustomerApiError>::into)?;

        Ok(Response::new(GenerateCustomerPortalTokenResponse { token }))
    }

    #[tracing::instrument(skip_all)]
    async fn upsert_customer_connection(
        &self,
        request: Request<UpsertCustomerConnectionRequest>,
    ) -> Result<Response<UpsertCustomerConnectionResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();
        let customer_id = CustomerId::from_proto(&req.customer_id)?;
        let connector_id = ConnectorId::from_proto(&req.connector_id)?;

        // Verify connector exists and belongs to tenant
        let _connector = self
            .store
            .get_connector_with_data(connector_id, tenant_id)
            .await
            .map_err(Into::<CustomerApiError>::into)?;

        // Create or update the connection
        let connection = meteroid_store::domain::CustomerConnection {
            id: CustomerConnectionId::new(),
            customer_id,
            connector_id,
            supported_payment_types: None,
            external_customer_id: req.external_customer_id,
        };

        let result = self
            .store
            .upsert_customer_connection(&tenant_id, connection)
            .await
            .map_err(Into::<CustomerApiError>::into)?;

        // Get connector to map the response
        let connector = self
            .store
            .get_connector_with_data(connector_id, tenant_id)
            .await
            .map_err(Into::<CustomerApiError>::into)?;

        let external_company_id = match &connector.data {
            Some(meteroid_store::domain::connectors::ProviderData::Hubspot(data)) => {
                Some(data.external_company_id.clone())
            }
            Some(meteroid_store::domain::connectors::ProviderData::Pennylane(data)) => {
                Some(data.external_company_id.clone())
            }
            _ => None,
        };

        let connection_proto =
            crate::api::customers::mapping::customer::map_customer_connection_to_proto(
                result,
                connector.provider,
                external_company_id,
            );

        Ok(Response::new(UpsertCustomerConnectionResponse {
            connection: Some(connection_proto),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn delete_customer_connection(
        &self,
        request: Request<DeleteCustomerConnectionRequest>,
    ) -> Result<Response<DeleteCustomerConnectionResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();
        let connection_id = CustomerConnectionId::from_proto(&req.connection_id)?;

        self.store
            .delete_customer_connection(&tenant_id, &connection_id)
            .await
            .map_err(Into::<CustomerApiError>::into)?;

        Ok(Response::new(DeleteCustomerConnectionResponse {}))
    }
}
