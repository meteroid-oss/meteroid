use super::CustomerServiceComponents;
use crate::customers::error::CustomerApiError;
use crate::customers::mapping::customer::{
    DomainAddressWrapper, DomainShippingAddressWrapper, ServerCustomerBriefWrapper,
    ServerCustomerWrapper, tax_status_from_server,
};
use crate::customers::mapping::summary as summary_mapping;
use common_domain::ids::{
    AliasOr, BaseId, ConnectedAccountId, ConnectorId, CustomerConnectionId, CustomerId,
    InvoicingEntityId, OrganizationId, TenantId, UserId,
};
use common_domain::rbac::{Permission, RolePermissions};
use common_grpc::middleware::server::auth::RequestExt;
use error_stack::Report;
use meteroid_app::features::customer_signals_gate_open;
use meteroid_grpc::meteroid::api::customers::v1::{
    ArchiveCustomerRequest, ArchiveCustomerResponse, AssignCustomerOwnerRequest,
    AssignCustomerOwnerResponse, CreateCustomerRequest, CreateCustomerResponse, CustomerBrief,
    CustomerOpenSignals, CustomerTaxStatus, DeleteCustomerConnectionRequest,
    DeleteCustomerConnectionResponse, GenerateCustomerPortalTokenRequest,
    GenerateCustomerPortalTokenResponse, GetCustomerByAliasRequest, GetCustomerByAliasResponse,
    GetCustomerByIdRequest, GetCustomerByIdResponse, GetCustomerSummaryRequest,
    GetCustomerSummaryResponse, ListCustomerCountriesRequest, ListCustomerCountriesResponse,
    ListCustomerRequest, ListCustomerResponse, ListCustomersByIdsRequest,
    ListCustomersByIdsResponse, RefreshVatValidationRequest, RefreshVatValidationResponse,
    SyncToHubspotRequest, SyncToHubspotResponse, SyncToPennylaneRequest, SyncToPennylaneResponse,
    UnarchiveCustomerRequest, UnarchiveCustomerResponse, UpdateCustomerRequest,
    UpdateCustomerResponse, UpsertCustomerConnectionRequest, UpsertCustomerConnectionResponse,
    customers_service_server::CustomersService,
};
use meteroid_grpc_mapping::utils::{PaginationExt, caller_permissions, check_bulk_ids};
use meteroid_store::domain::customer_signals::OpenSignals;
use meteroid_store::domain::customer_summary::SummaryParts;
use meteroid_store::domain::{CustomerNew, CustomerPatch};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::CustomersInterface;
use meteroid_store::repositories::connectors::ConnectorsInterface;
use meteroid_store::repositories::customer_connection::CustomerConnectionInterface;
use meteroid_store::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use meteroid_store::repositories::customer_signals::CustomerSignalsInterface;
use meteroid_store::repositories::customers::CustomersInterfaceAuto;
use meteroid_store::repositories::users::UserInterface;
use meteroid_store::services::fx::reporting_fx;
use std::collections::HashMap;
use tonic::{Request, Response, Status};

/// Parses the optional JSON-encoded `custom_properties` string carried by the customer proto.
/// `None` = field omitted (the store leaves custom properties untouched).
fn parse_custom_properties(
    raw: Option<String>,
) -> Result<Option<serde_json::Value>, CustomerApiError> {
    raw.map(|s| {
        serde_json::from_str(&s).map_err(|e| {
            CustomerApiError::InvalidArgument(format!("Invalid custom_properties JSON: {e}"))
        })
    })
    .transpose()
}

impl CustomerServiceComponents {
    /// Open signals of the listed customers, only for callers who may see signals (alerts:read
    /// and the Signals add-on).
    async fn open_signals(
        &self,
        org_id: OrganizationId,
        tenant_id: TenantId,
        permissions: &RolePermissions,
        customer_ids: &[CustomerId],
    ) -> Result<HashMap<CustomerId, OpenSignals>, Status> {
        if customer_ids.is_empty()
            || !permissions.contains(Permission::AlertsRead)
            || !customer_signals_gate_open(
                self.feature_resolver.as_ref(),
                &self.store,
                org_id,
                tenant_id,
            )
            .await
        {
            return Ok(HashMap::new());
        }
        let fx = reporting_fx(&self.store, tenant_id, chrono::Utc::now().date_naive())
            .await
            .map_err(Into::<CustomerApiError>::into)?;
        self.store
            .open_signals_by_customer(tenant_id, customer_ids, &fx)
            .await
            .map_err(|e| CustomerApiError::from(e).into())
    }
}

#[tonic::async_trait]
impl CustomersService for CustomerServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn create_customer(
        &self,
        request: Request<CreateCustomerRequest>,
    ) -> Result<Response<CreateCustomerResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor_typed = request.actor_typed()?;

        let inner = request
            .into_inner()
            .data
            .ok_or(CustomerApiError::MissingArgument("no data".into()))?;

        let customer_new = CustomerNew {
            name: inner.name,
            customer_type: crate::customers::mapping::customer::customer_type_from_server(
                inner.customer_type,
            )?,
            first_name: inner.first_name,
            last_name: inner.last_name,
            invoicing_entity_id: InvoicingEntityId::from_proto_opt(inner.invoicing_entity_id)?,
            alias: inner.alias,
            billing_email: inner.billing_email,
            invoicing_emails: inner.invoicing_emails,
            legal_number: inner.legal_number,
            buyer_reference: inner.buyer_reference,
            phone: inner.phone,
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
            vat_number: inner.vat_number,
            custom_taxes: inner
                .custom_taxes
                .into_iter()
                .map(
                    |t| -> Result<meteroid_store::domain::CustomerTaxRate, CustomerApiError> {
                        Ok(meteroid_store::domain::CustomerTaxRate {
                            tax_code: t.tax_code,
                            name: t.name,
                            rate: t.rate.parse().map_err(|_| {
                                CustomerApiError::InvalidArgument("Invalid tax rate".to_string())
                            })?,
                        })
                    },
                )
                .collect::<Result<Vec<_>, _>>()?,
            tax_status: inner
                .tax_status
                .and_then(|v| CustomerTaxStatus::try_from(v).ok())
                .map(tax_status_from_server)
                .unwrap_or_default(),
            exemption_reason: inner.exemption_reason,
            connected_account_id: ConnectedAccountId::from_proto_opt(inner.connected_account_id)?,
            custom_properties: parse_custom_properties(inner.custom_properties)?
                .unwrap_or(serde_json::Value::Null),
            preferred_locales: validated_locales(inner.preferred_locales)?,
        };

        let customer = self
            .store
            .insert_customer(actor_typed, customer_new, tenant_id)
            .await
            .and_then(ServerCustomerBriefWrapper::try_from)
            .map(|v| v.0)
            .map_err(Into::<CustomerApiError>::into)?;

        Ok(Response::new(CreateCustomerResponse {
            customer: Some(customer),
        }))
    }

    #[tracing::instrument(skip_all, fields(
        customer_id = request.get_ref().customer.as_ref().map(|c| c.id.as_str())
    ))]
    async fn update_customer(
        &self,
        request: Request<UpdateCustomerRequest>,
    ) -> Result<Response<UpdateCustomerResponse>, Status> {
        let actor = request.actor_typed()?;
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
        let custom_properties = parse_custom_properties(customer.custom_properties)?;
        let preferred_locales = validated_locales(customer.preferred_locales.clone())?;

        let _ = self
            .store
            .patch_customer(
                actor,
                tenant_id,
                CustomerPatch {
                    id: CustomerId::from_proto(&customer.id)?,
                    name: customer.name.clone(),
                    customer_type: customer
                        .customer_type
                        .map(crate::customers::mapping::customer::customer_type_from_server)
                        .transpose()?,
                    first_name: customer.first_name.clone(),
                    last_name: customer.last_name.clone(),
                    alias: customer.alias.clone(),
                    billing_email: customer.billing_email.clone(),
                    invoicing_emails: customer.invoicing_emails.map(|v| v.emails),
                    legal_number: customer.legal_number.clone().map(|value| {
                        if value.trim().is_empty() {
                            None
                        } else {
                            Some(value)
                        }
                    }),
                    buyer_reference: customer
                        .buyer_reference
                        .clone()
                        .map(|value| Some(value).filter(|v| !v.trim().is_empty())),
                    phone: customer.phone.clone(),
                    invoicing_entity_id: InvoicingEntityId::from_proto_opt(
                        customer.invoicing_entity_id,
                    )?,
                    currency: customer.currency.clone(),
                    billing_address,
                    shipping_address,
                    vat_number: Some(customer.vat_number),
                    custom_taxes: crate::customers::mapping::customer::custom_taxes_from_grpc(
                        customer.custom_taxes,
                    )
                    .map_err(Into::<Status>::into)?,
                    current_payment_method_id: None,
                    tax_status: customer
                        .tax_status
                        .and_then(|v| CustomerTaxStatus::try_from(v).ok())
                        .map(tax_status_from_server),
                    exemption_reason: customer.exemption_reason.map(|value| {
                        if value.trim().is_empty() {
                            None
                        } else {
                            Some(value)
                        }
                    }),
                    connected_account_id: None,
                    custom_properties,
                    // Always touched, like vat_number above: this RPC carries a full
                    // form, so an empty list means "reset to inherit".
                    preferred_locales: Some(preferred_locales),
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
        let org_id = request.organization()?;
        let permissions = caller_permissions(&request);

        let inner = request.into_inner();

        let pagination_req = inner.pagination.into_domain();

        let filters = meteroid_grpc_mapping::filters::map_filters(inner.filters)?;

        let res = self
            .store
            .list_customers_with_metrics(
                tenant_id,
                pagination_req,
                inner.order_by,
                inner.search,
                inner.archived,
                inner.lifecycle_state,
                inner.country,
                filters,
            )
            .await
            .map_err(Into::<CustomerApiError>::into)?;

        let customer_ids: Vec<CustomerId> = res.items.iter().map(|c| c.customer.id).collect();
        let open_signals = self
            .open_signals(org_id, tenant_id, &permissions, &customer_ids)
            .await?;
        let mut customers = res
            .items
            .into_iter()
            .map(|l| ServerCustomerBriefWrapper::try_from(l).map(|v| v.0))
            .collect::<Vec<Result<CustomerBrief, Report<StoreError>>>>()
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::<CustomerApiError>::into)?;
        for (brief, id) in customers.iter_mut().zip(&customer_ids) {
            brief.open_signals = open_signals.get(id).map(|o| CustomerOpenSignals {
                count: u32::try_from(o.count).unwrap_or(u32::MAX),
                top_category: o.top_category.as_str().to_string(),
                critical: o.critical,
            });
        }

        let response = ListCustomerResponse {
            pagination_meta: inner
                .pagination
                .into_response(res.total_pages, res.total_results),
            customers,
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn list_customer_countries(
        &self,
        request: Request<ListCustomerCountriesRequest>,
    ) -> Result<Response<ListCustomerCountriesResponse>, Status> {
        let tenant_id = request.tenant()?;

        let countries = self
            .store
            .list_customer_countries(tenant_id)
            .await
            .map_err(Into::<CustomerApiError>::into)?;

        Ok(Response::new(ListCustomerCountriesResponse { countries }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_customers_by_ids(
        &self,
        request: Request<ListCustomersByIdsRequest>,
    ) -> Result<Response<ListCustomersByIdsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let ids = request.into_inner().ids;

        check_bulk_ids("ids", ids.len())?;

        let ids = ids
            .iter()
            .map(CustomerId::from_proto)
            .collect::<Result<Vec<_>, _>>()?;

        let customers = self
            .store
            .list_customers_by_ids(tenant_id, ids)
            .await
            .map_err(Into::<CustomerApiError>::into)?;

        let customers = customers
            .into_iter()
            .map(|c| ServerCustomerBriefWrapper::try_from(c).map(|v| v.0))
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::<CustomerApiError>::into)?;

        Ok(Response::new(ListCustomersByIdsResponse { customers }))
    }

    #[tracing::instrument(skip_all, fields(customer_id = %request.get_ref().customer_id))]
    async fn get_customer_summary(
        &self,
        request: Request<GetCustomerSummaryRequest>,
    ) -> Result<Response<GetCustomerSummaryResponse>, Status> {
        let tenant_id = request.tenant()?;
        let permissions = caller_permissions(&request);
        let customer_id = CustomerId::from_proto(&request.into_inner().customer_id)?;

        let parts = SummaryParts {
            invoices: permissions.contains(Permission::InvoicesRead),
            subscriptions: permissions.contains(Permission::SubscriptionsRead),
        };
        let summary = self
            .service
            .get_customer_summary(
                tenant_id,
                customer_id,
                parts,
                chrono::Utc::now().naive_utc(),
            )
            .await
            .map_err(Into::<CustomerApiError>::into)?;

        Ok(Response::new(summary_mapping::summary_to_server(&summary)))
    }

    #[tracing::instrument(skip_all, fields(customer_id = %request.get_ref().id))]
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

        let customer_connections = self
            .store
            .list_connections_by_customer_id(&tenant_id, &customer_id)
            .await
            .map_err(Into::<CustomerApiError>::into)?;

        let connectors = self
            .store
            .list_connectors(None, tenant_id)
            .await
            .map_err(Into::<CustomerApiError>::into)?;

        let payment_methods = self
            .store
            .list_payment_methods_by_customer(&tenant_id, &customer_id)
            .await
            .map_err(Into::<CustomerApiError>::into)?;

        let lifecycle_state = self
            .store
            .derive_customer_lifecycle_state(customer_id, tenant_id)
            .await
            .map_err(Into::<CustomerApiError>::into)?;

        let customer_proto = ServerCustomerWrapper::try_from(customer)
            .map(|mut v| {
                v.0.customer_connections = customer_connections
                    .into_iter()
                    .filter_map(|conn| {
                        // Find the connector to get the provider type and external_company_id
                        connectors
                            .iter()
                            .find(|c| c.id == conn.connector_id)
                            .and_then(|connector| {
                                let external_company_id =
                                    connector.external_company_id().map(str::to_string);
                                crate::customers::mapping::customer::map_customer_connection_to_proto(
                                    conn,
                                    connector.provider().clone(),
                                    external_company_id,
                                )
                            })
                    })
                    .collect();
                v.0.payment_methods = payment_methods
                    .into_iter()
                    .map(crate::customers::mapping::customer_payment_method::domain_to_server)
                    .collect();
                v.0.lifecycle_state = lifecycle_state;
                v.0
            })
            .map_err(Into::<CustomerApiError>::into)?;

        let mut customer_proto = customer_proto;
        // Resolve the owner's display name; best-effort, so a dangling owner (removed
        // from the org but still referenced) degrades to just the id.
        if let Some(owner) = customer_proto.owner.as_mut()
            && let Ok(owner_id) = UserId::from_proto(&owner.user_id)
            && let Ok(user) = self
                .store
                .find_user_by_id_and_tenant(owner_id, tenant_id)
                .await
        {
            let name = [user.first_name.as_deref(), user.last_name.as_deref()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" ");
            owner.display_name = if name.trim().is_empty() {
                user.email
            } else {
                name
            };
        }

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

    #[tracing::instrument(skip_all, fields(customer_id = %request.get_ref().id))]
    async fn archive_customer(
        &self,
        request: Request<ArchiveCustomerRequest>,
    ) -> Result<Response<ArchiveCustomerResponse>, Status> {
        let actor_typed = request.actor_typed()?;
        let tenant_id = request.tenant()?;

        let req = request.into_inner();
        let customer_id = CustomerId::from_proto(&req.id)?;

        self.store
            .archive_customer(actor_typed, tenant_id, AliasOr::Id(customer_id))
            .await
            .map_err(Into::<CustomerApiError>::into)?;

        Ok(Response::new(ArchiveCustomerResponse {}))
    }

    #[tracing::instrument(skip_all, fields(customer_id = %request.get_ref().id))]
    async fn unarchive_customer(
        &self,
        request: Request<UnarchiveCustomerRequest>,
    ) -> Result<Response<UnarchiveCustomerResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;

        let req = request.into_inner();
        let customer_id = CustomerId::from_proto(&req.id)?;

        self.store
            .unarchive_customer(actor, tenant_id, AliasOr::Id(customer_id))
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

    #[tracing::instrument(skip_all, fields(customer_id = %request.get_ref().customer_id))]
    async fn generate_customer_portal_token(
        &self,
        request: Request<GenerateCustomerPortalTokenRequest>,
    ) -> Result<Response<GenerateCustomerPortalTokenResponse>, Status> {
        let tenant_id = request.tenant()?;
        let delegated = request.delegated();

        let req = request.into_inner();
        let customer_id = CustomerId::from_proto(req.customer_id)?;

        // Generate the JWT token for customer portal access
        let token = meteroid_store::jwt_claims::generate_portal_token(
            &self.jwt_secret,
            tenant_id,
            meteroid_store::jwt_claims::ResourceAccess::Customer(customer_id),
            None,
            Some(meteroid_store::jwt_claims::default_customer_portal_token_ttl()),
            delegated,
        )
        .map_err(Into::<CustomerApiError>::into)?;

        Ok(Response::new(GenerateCustomerPortalTokenResponse { token }))
    }

    #[tracing::instrument(skip_all, fields(customer_id = %request.get_ref().customer_id))]
    async fn refresh_vat_validation(
        &self,
        request: Request<RefreshVatValidationRequest>,
    ) -> Result<Response<RefreshVatValidationResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();
        let customer_id = CustomerId::from_proto(&req.customer_id)?;

        let customer = self
            .store
            .request_vat_number_revalidation(tenant_id, customer_id)
            .await
            .and_then(ServerCustomerWrapper::try_from)
            .map(|v| v.0)
            .map_err(Into::<CustomerApiError>::into)?;

        Ok(Response::new(RefreshVatValidationResponse {
            customer: Some(customer),
        }))
    }

    #[tracing::instrument(skip_all, fields(customer_id = %request.get_ref().customer_id))]
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

        let connector = self
            .store
            .get_connector_with_data(connector_id, tenant_id)
            .await
            .map_err(Into::<CustomerApiError>::into)?;

        let external_company_id = connector.external_company_id().map(str::to_string);

        let connection_proto =
            crate::customers::mapping::customer::map_customer_connection_to_proto(
                result,
                connector.provider(),
                external_company_id,
            );

        Ok(Response::new(UpsertCustomerConnectionResponse {
            connection: connection_proto,
        }))
    }

    #[tracing::instrument(skip_all, fields(customer_id = %request.get_ref().customer_id))]
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

    #[tracing::instrument(skip_all)]
    async fn assign_customer_owner(
        &self,
        request: Request<AssignCustomerOwnerRequest>,
    ) -> Result<Response<AssignCustomerOwnerResponse>, Status> {
        let actor = request.actor_typed()?;
        let tenant_id = request.tenant()?;

        let req = request.into_inner();
        check_bulk_ids("customer_ids", req.customer_ids.len())?;

        let customer_ids = req
            .customer_ids
            .iter()
            .map(CustomerId::from_proto)
            .collect::<Result<Vec<_>, _>>()?;

        // Empty / omitted owner = unassign.
        let owner_user_id = req
            .owner_user_id
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(UserId::from_proto)
            .transpose()?;

        let updated = self
            .store
            .assign_customer_owner(actor, tenant_id, customer_ids, owner_user_id)
            .await
            .map_err(Into::<CustomerApiError>::into)?;

        Ok(Response::new(AssignCustomerOwnerResponse {
            updated: updated as u32,
        }))
    }
}

/// Reject a preference list we can serve no entry of at the point of writing it, rather
/// than storing it and letting the level below quietly decide the language, with only a
/// log line to show for it. One servable entry is enough — falling through to it is what
/// the list is for.
fn validated_locales(locales: Vec<String>) -> Result<Vec<String>, CustomerApiError> {
    meteroid_invoicing::lang::reject_unsupported_preferences(&locales).map_err(|tag| {
        CustomerApiError::InvalidArgument(format!(
            "Unsupported preferred_locales '{tag}'; supported: {:?}",
            meteroid_invoicing::lang::SUPPORTED
        ))
    })?;
    Ok(locales)
}
