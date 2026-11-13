use super::{InvoiceServiceComponents, mapping};
use crate::customers::mapping::customer::DomainAddressWrapper;
use crate::invoices::error::InvoiceApiError;
use chrono::{NaiveDate, NaiveTime};
use common_domain::ids::{
    CreditNoteId, CustomerId, InvoiceId, PaymentTransactionId, SubscriptionId, TenantId,
};
use common_grpc::middleware::server::auth::RequestExt;
use common_utils::decimals::ToSubunit;
use meteroid_app::conversions::{AsProtoOpt, FromProtoOpt, ProtoConv};
use meteroid_app::features::gates;
use meteroid_grpc::meteroid::api::invoices::v1::{
    CheckEinvoicingReadinessRequest, CheckEinvoicingReadinessResponse,
    CreateCorrectedInvoiceRequest, CreateCorrectedInvoiceResponse, CreateInvoiceRequest,
    CreateInvoiceResponse, DeleteInvoiceRequest, DeleteInvoiceResponse, FinalizeInvoiceRequest,
    FinalizeInvoiceResponse, GenerateInvoicePaymentTokenRequest,
    GenerateInvoicePaymentTokenResponse, GetInvoiceRequest, GetInvoiceResponse, Invoice,
    ListInvoicesRequest, ListInvoicesResponse, MarkInvoiceAsUncollectibleRequest,
    MarkInvoiceAsUncollectibleResponse, NewInvoice, PreviewInvoiceRequest, PreviewInvoiceResponse,
    PreviewInvoiceUpdateRequest, PreviewInvoiceUpdateResponse, PreviewNewInvoiceRequest,
    PreviewNewInvoiceResponse, RefreshInvoiceDataRequest, RefreshInvoiceDataResponse,
    RegenerateInvoiceRequest, RegenerateInvoiceResponse, RequestPdfGenerationRequest,
    RequestPdfGenerationResponse, SubLineItem as ProtoSubLineItem, SyncToPennylaneRequest,
    SyncToPennylaneResponse, UpdateInvoiceRequest, UpdateInvoiceResponse, VoidInvoiceRequest,
    VoidInvoiceResponse, invoices_service_server::InvoicesService,
};
use meteroid_grpc_mapping::utils::PaginationExt;
use meteroid_store::Store;
use meteroid_store::domain::pgmq::{InvoicePdfRequestEvent, PgmqMessageNew, PgmqQueue};
use meteroid_store::domain::{
    ChargeType, InvoiceNew, InvoicingEntity, LineItem, UpdateInvoiceParams, UpdateLineItemParams,
};
use meteroid_store::repositories::CreditNoteInterface;
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::repositories::customers::CustomersInterfaceAuto;
use meteroid_store::repositories::invoices::apply_tax_rates_to_priced_lines;
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterfaceAuto;
use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;
use meteroid_store::repositories::pgmq::PgmqInterface;
use meteroid_store::repositories::portal_tokens::PortalTokensInterface;
use meteroid_store::services::{
    CustomerDetailsUpdate, OfflineRefundCommand, RefundCommand, ReverseManualPaymentCommand,
};
use meteroid_store::utils::local_id::{IdType, LocalId};
use tonic::{Request, Response, Status};

impl InvoiceServiceComponents {
    /// Gates the invoice-annex surface behind the per-organization entitlement, matching
    /// `AnnexTemplatesService`. Reads stay open — an invoice issued while entitled keeps
    /// rendering its appendix; only editing and regenerating are gated.
    async fn ensure_annexes_entitled<T>(&self, request: &Request<T>) -> Result<(), Status> {
        let organization_id = request.organization()?;
        if !self
            .feature_resolver
            .resolve(organization_id)
            .await
            .is_enabled(gates::INVOICE_ANNEXES)
        {
            return Err(Status::permission_denied(
                "Invoice annexes are not enabled for this organization.",
            ));
        }
        Ok(())
    }

    /// Refund amounts arrive as decimals in major units, like manual payments, and are read in
    /// the parent transaction's currency — a refund never converts.
    async fn parse_transaction_amount(
        &self,
        tenant_id: TenantId,
        transaction_id: PaymentTransactionId,
        amount: &String,
    ) -> Result<i64, Status> {
        let transaction = self
            .store
            .get_payment_tx_by_id(tenant_id, transaction_id)
            .await
            .map_err(Into::<InvoiceApiError>::into)?;

        let currency = meteroid_money::iso::find(&transaction.currency).ok_or_else(|| {
            InvoiceApiError::InvalidArgument(format!("Unknown currency {}", transaction.currency))
        })?;

        parse_as_minor(amount, currency)
    }
}

#[tonic::async_trait]
impl InvoicesService for InvoiceServiceComponents {
    #[tracing::instrument(skip_all, fields(
        customer_id = request.get_ref().customer_id.as_deref(),
        subscription_id = request.get_ref().subscription_id.as_deref(),
    ))]
    async fn list_invoices(
        &self,
        request: Request<ListInvoicesRequest>,
    ) -> Result<Response<ListInvoicesResponse>, Status> {
        let tenant_id = request.tenant()?;

        let inner = request.into_inner();

        let customer_id = CustomerId::from_proto_opt(inner.customer_id)?;
        let subscription_id = SubscriptionId::from_proto_opt(inner.subscription_id)?;

        let pagination_req = inner.pagination.into_domain();

        let res = self
            .store
            .list_invoices(
                tenant_id,
                customer_id,
                subscription_id,
                mapping::invoices::status_server_to_domain(inner.status),
                mapping::invoices::payment_status_server_to_domain(inner.payment_status),
                mapping::invoices::einvoicing_status_server_to_domain(inner.einvoicing_status),
                inner.overdue.unwrap_or(false),
                inner.search,
                meteroid_grpc_mapping::filters::map_filters(inner.filters)?,
                inner.order_by,
                pagination_req,
            )
            .await
            .map_err(Into::<InvoiceApiError>::into)?;

        // The list shows the stable billing-period date ("Invoice date") and does not project
        // draft emission dates — that's a detail-view concern.
        let items = res.items;

        let settled = self
            .store
            .invoices_settled_by_credit_notes(
                tenant_id,
                &items.iter().map(|item| &item.invoice).collect::<Vec<_>>(),
            )
            .await
            .map_err(Into::<InvoiceApiError>::into)?;

        let mut invoices: Vec<Invoice> = items
            .into_iter()
            .map(|item| Invoice {
                settled_by_credit_notes: settled.contains(&item.invoice.id),
                ..mapping::invoices::domain_to_server(item)
            })
            .collect();

        // For merged children in this page (subscription-scoped listings), resolve the consolidated
        // parent's invoice number for display ("Merged into INV-…").
        let parent_ids: Vec<InvoiceId> = invoices
            .iter()
            .filter_map(|i| i.consolidated_into_invoice_id.as_deref())
            .filter_map(|s| InvoiceId::from_proto(s).ok())
            .collect();
        if !parent_ids.is_empty() {
            let numbers: std::collections::HashMap<String, String> = self
                .store
                .list_invoices_by_ids(parent_ids)
                .await
                .map_err(Into::<InvoiceApiError>::into)?
                .into_iter()
                .map(|p| (p.id.to_string(), p.invoice_number))
                .collect();
            for inv in invoices.iter_mut() {
                if let Some(pid) = &inv.consolidated_into_invoice_id {
                    inv.consolidated_into_invoice_number = numbers.get(pid).cloned();
                }
            }
        }

        let response = ListInvoicesResponse {
            pagination_meta: inner
                .pagination
                .into_response(res.total_pages, res.total_results),
            invoices,
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all, fields(invoice_id = %request.get_ref().id))]
    async fn get_invoice(
        &self,
        request: Request<GetInvoiceRequest>,
    ) -> Result<Response<GetInvoiceResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();

        let invoice_id = InvoiceId::from_proto(&req.id)?;

        let transactions = self
            .store
            .list_payment_tx_by_invoice_id(tenant_id, invoice_id)
            .await
            .map_err(Into::<InvoiceApiError>::into)?
            .into_iter()
            .map(mapping::transactions::domain_with_method_to_server)
            .collect::<Vec<_>>();

        let mut detailed = self
            .store
            .get_detailed_invoice_by_id(tenant_id, invoice_id)
            .await
            .map_err(Into::<InvoiceApiError>::into)?;

        // Drafts display the dates finalization would stamp; `dates_projected` tells the UI
        // whether these are projected or the real stored values, so it can label them honestly.
        let dates_projected = self
            .store
            .project_draft_emission_dates(&mut detailed.invoice)
            .await
            .map_err(Into::<InvoiceApiError>::into)?;
        let regenerable = self
            .services
            .can_regenerate_invoice(tenant_id, &detailed.invoice)
            .await
            .map_err(Into::<InvoiceApiError>::into)?;
        let settled_by_credit_notes = self
            .store
            .invoices_settled_by_credit_notes(tenant_id, &[&detailed.invoice])
            .await
            .map_err(Into::<InvoiceApiError>::into)?
            .contains(&invoice_id);

        let mut invoice = mapping::invoices::domain_invoice_with_transactions_to_server(
            detailed.invoice,
            detailed.transactions,
            self.jwt_secret.clone(),
        )
        .map_err(Into::<InvoiceApiError>::into)?;
        invoice.dates_projected = dates_projected;
        invoice.regenerable = regenerable;
        invoice.settled_by_credit_notes = settled_by_credit_notes;

        invoice.transactions = transactions;

        invoice.child_invoice_id = self
            .store
            .find_child_invoice_id(tenant_id, invoice_id)
            .await
            .map_err(Into::<InvoiceApiError>::into)?
            .map(|id| id.as_proto());

        // For a consolidated parent, surface the per-subscription children so the UI can show
        // which subscription/plan contributed which amount. Empty for ordinary invoices.
        invoice.consolidated_children = self
            .store
            .list_consolidated_children(tenant_id, invoice_id)
            .await
            .map_err(Into::<InvoiceApiError>::into)?
            .iter()
            .map(mapping::invoices::domain_consolidated_child_to_server)
            .collect();

        // For a child, resolve the consolidated parent's invoice number for display
        // ("Merged into INV-…"). The parent is finalized by the time the child is viewed.
        if let Some(parent_id) = &invoice.consolidated_into_invoice_id
            && let Ok(parent_id) = InvoiceId::from_proto(parent_id)
            && let Ok(parent) = self.store.get_invoice_by_id(tenant_id, parent_id).await
        {
            invoice.consolidated_into_invoice_number = Some(parent.invoice_number);
        }

        let response = GetInvoiceResponse {
            invoice: Some(invoice),
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all, fields(invoice_id = %request.get_ref().id))]
    async fn preview_invoice_svg(
        &self,
        request: Request<PreviewInvoiceRequest>,
    ) -> Result<Response<PreviewInvoiceResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();

        let svgs = self
            .preview_rendering
            .preview_invoice_by_id(InvoiceId::from_proto(&req.id)?, tenant_id)
            .await
            .map_err(Into::<InvoiceApiError>::into)?;

        let response = PreviewInvoiceResponse { svgs };

        Ok(Response::new(response))
    }

    /// The e-invoicing gate, run as a dry run instead of at finalization.
    ///
    /// Meaningful for drafts: the report is recomputed from the live customer and
    /// entity records, so a finding here can still be fixed on this invoice —
    /// the window the post-finalization report on `GetInvoice` says is closed.
    /// A separate RPC rather than a `GetInvoice` field because it costs the full
    /// document assembly (payment resolution included) and its failure must not
    /// take the invoice page down with it.
    #[tracing::instrument(skip_all, fields(invoice_id = %request.get_ref().id))]
    async fn check_einvoicing_readiness(
        &self,
        request: Request<CheckEinvoicingReadinessRequest>,
    ) -> Result<Response<CheckEinvoicingReadinessResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let report = self
            .preview_rendering
            .check_einvoicing_readiness(InvoiceId::from_proto(&req.id)?, tenant_id)
            .await
            .map_err(Into::<InvoiceApiError>::into)?;

        Ok(Response::new(CheckEinvoicingReadinessResponse {
            readiness: report
                .as_ref()
                .map(mapping::invoices::einvoicing_readiness_to_server),
        }))
    }

    // for demo & local use when the worker was not started initially
    #[tracing::instrument(skip_all, fields(invoice_id = %request.get_ref().id))]
    async fn request_pdf_generation(
        &self,
        request: Request<RequestPdfGenerationRequest>,
    ) -> Result<Response<RequestPdfGenerationResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let invoice = self
            .store
            .get_detailed_invoice_by_id(tenant_id, InvoiceId::from_proto(&req.id)?)
            .await
            .map_err(Into::<InvoiceApiError>::into)?;

        let pgmq_msg_new: PgmqMessageNew =
            InvoicePdfRequestEvent::new(tenant_id, invoice.invoice.id, false)
                .try_into()
                .map_err(Into::<InvoiceApiError>::into)?;

        // check if already generated ?
        self.store
            .pgmq_send_batch(PgmqQueue::InvoicePdfRequest, vec![pgmq_msg_new])
            .await
            .map_err(Into::<InvoiceApiError>::into)?;

        let response = RequestPdfGenerationResponse {};

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all, fields(invoice_id = %request.get_ref().id))]
    async fn refresh_invoice_data(
        &self,
        request: Request<RefreshInvoiceDataRequest>,
    ) -> Result<Response<RefreshInvoiceDataResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();

        let invoice = self
            .services
            .refresh_invoice_data(InvoiceId::from_proto(&req.id)?, tenant_id)
            .await
            .and_then(|inv| {
                mapping::invoices::domain_invoice_with_transactions_to_server(
                    inv.invoice,
                    inv.transactions,
                    self.jwt_secret.clone(),
                )
            })
            .map_err(Into::<InvoiceApiError>::into)?;

        let response = RefreshInvoiceDataResponse {
            invoice: Some(invoice),
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn sync_to_pennylane(
        &self,
        request: Request<SyncToPennylaneRequest>,
    ) -> Result<Response<SyncToPennylaneResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();

        let ids = req
            .invoice_ids
            .iter()
            .map(InvoiceId::from_proto)
            .collect::<Result<Vec<_>, _>>()?;

        self.store
            .sync_invoices_to_pennylane(ids, tenant_id)
            .await
            .map_err(Into::<InvoiceApiError>::into)?;

        Ok(Response::new(SyncToPennylaneResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn create_invoice(
        &self,
        request: Request<CreateInvoiceRequest>,
    ) -> Result<Response<CreateInvoiceResponse>, Status> {
        let tenant_id = request.tenant()?;

        if request
            .get_ref()
            .invoice
            .as_ref()
            .is_some_and(|i| !i.annexes.is_empty())
        {
            self.ensure_annexes_entitled(&request).await?;
        }

        let req = request.into_inner();

        let invoice_req = req
            .invoice
            .ok_or_else(|| InvoiceApiError::InputError("Missing invoice data".to_string()))?;

        let (new_invoice, _) = to_domain_invoice_new(tenant_id, invoice_req, &self.store).await?;

        let inserted = self
            .store
            .insert_invoice(new_invoice)
            .await
            .map_err(Into::<InvoiceApiError>::into)?;

        let invoice = self
            .store
            .get_detailed_invoice_by_id(tenant_id, inserted.id)
            .await
            .and_then(|inv| {
                mapping::invoices::domain_invoice_with_transactions_to_server(
                    inv.invoice,
                    inv.transactions,
                    self.jwt_secret.clone(),
                )
            })
            .map_err(Into::<InvoiceApiError>::into)?;

        Ok(Response::new(CreateInvoiceResponse {
            invoice: Some(invoice),
        }))
    }

    #[tracing::instrument(skip_all, fields(invoice_id = %request.get_ref().parent_invoice_id))]
    async fn create_corrected_invoice(
        &self,
        request: Request<CreateCorrectedInvoiceRequest>,
    ) -> Result<Response<CreateCorrectedInvoiceResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let parent_invoice_id = InvoiceId::from_proto(&req.parent_invoice_id)?;

        let created = self
            .services
            .create_corrected_invoice_from(tenant_id, parent_invoice_id)
            .await
            .map_err(Into::<InvoiceApiError>::into)?;

        let invoice = self
            .store
            .get_detailed_invoice_by_id(tenant_id, created.id)
            .await
            .and_then(|inv| {
                mapping::invoices::domain_invoice_with_transactions_to_server(
                    inv.invoice,
                    inv.transactions,
                    self.jwt_secret.clone(),
                )
            })
            .map_err(Into::<InvoiceApiError>::into)?;

        Ok(Response::new(CreateCorrectedInvoiceResponse {
            invoice: Some(invoice),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn preview_new_invoice_svg(
        &self,
        request: Request<PreviewNewInvoiceRequest>,
    ) -> Result<Response<PreviewNewInvoiceResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();

        let invoice_req = req
            .invoice
            .ok_or_else(|| InvoiceApiError::InputError("Missing invoice data".to_string()))?;

        let (new_invoice, inv_entity) =
            to_domain_invoice_new(tenant_id, invoice_req, &self.store).await?;

        let svgs = self
            .preview_rendering
            .preview_invoice(new_invoice.into(), inv_entity)
            .await
            .map_err(Into::<InvoiceApiError>::into)?;

        let response = PreviewNewInvoiceResponse { svgs };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all, fields(invoice_id = %request.get_ref().id))]
    async fn finalize_invoice(
        &self,
        request: Request<FinalizeInvoiceRequest>,
    ) -> Result<Response<FinalizeInvoiceResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;

        let req = request.into_inner();

        let invoice_id = InvoiceId::from_proto(&req.id)?;

        let finalized = self
            .services
            .finalize_invoice(actor, invoice_id, tenant_id)
            .await
            .and_then(|inv| {
                mapping::invoices::domain_invoice_with_transactions_to_server(
                    inv.invoice,
                    inv.transactions,
                    self.jwt_secret.clone(),
                )
            })
            .map_err(Into::<InvoiceApiError>::into)?;

        Ok(Response::new(FinalizeInvoiceResponse {
            invoice: Some(finalized),
        }))
    }

    #[tracing::instrument(skip_all, fields(invoice_id = %request.get_ref().id))]
    async fn update_invoice(
        &self,
        request: Request<UpdateInvoiceRequest>,
    ) -> Result<Response<UpdateInvoiceResponse>, Status> {
        let tenant_id = request.tenant()?;

        if request.get_ref().annexes.is_some() {
            self.ensure_annexes_entitled(&request).await?;
        }

        let req = request.into_inner();

        let invoice_id = InvoiceId::from_proto(&req.id)?;

        let params = to_update_invoice_params(req)?;

        let updated = self
            .services
            .update_draft_invoice(invoice_id, tenant_id, params)
            .await
            .and_then(|inv| {
                mapping::invoices::domain_invoice_with_transactions_to_server(
                    inv.invoice,
                    inv.transactions,
                    self.jwt_secret.clone(),
                )
            })
            .map_err(Into::<InvoiceApiError>::into)?;

        Ok(Response::new(UpdateInvoiceResponse {
            invoice: Some(updated),
        }))
    }

    #[tracing::instrument(skip_all, fields(
        invoice_id = request.get_ref().update_request.as_ref().map(|u| u.id.as_str())
    ))]
    async fn preview_invoice_update(
        &self,
        request: Request<PreviewInvoiceUpdateRequest>,
    ) -> Result<Response<PreviewInvoiceUpdateResponse>, Status> {
        let tenant_id = request.tenant()?;

        if request
            .get_ref()
            .update_request
            .as_ref()
            .is_some_and(|r| r.annexes.is_some())
        {
            self.ensure_annexes_entitled(&request).await?;
        }

        let req = request.into_inner();
        let update_req = req
            .update_request
            .ok_or_else(|| InvoiceApiError::InputError("Missing update_request".to_string()))?;

        let invoice_id = InvoiceId::from_proto(&update_req.id)?;

        let params = to_update_invoice_params(update_req)?;

        let preview = self
            .services
            .preview_draft_invoice_update(invoice_id, tenant_id, params)
            .await
            .and_then(|inv| {
                mapping::invoices::domain_invoice_with_transactions_to_server(
                    inv,
                    Vec::new(), // no transaction in draft
                    self.jwt_secret.clone(),
                )
            })
            .map_err(Into::<InvoiceApiError>::into)?;

        Ok(Response::new(PreviewInvoiceUpdateResponse {
            preview: Some(preview),
        }))
    }

    #[tracing::instrument(skip_all, fields(invoice_id = %request.get_ref().id))]
    async fn delete_invoice(
        &self,
        request: Request<DeleteInvoiceRequest>,
    ) -> Result<Response<DeleteInvoiceResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let req = request.into_inner();

        let invoice_id = InvoiceId::from_proto(&req.id)?;

        self.store
            .delete_invoice(actor, invoice_id, tenant_id)
            .await
            .map_err(Into::<InvoiceApiError>::into)?;

        Ok(Response::new(DeleteInvoiceResponse {}))
    }

    #[tracing::instrument(skip_all, fields(invoice_id = %request.get_ref().id))]
    async fn void_invoice(
        &self,
        request: Request<VoidInvoiceRequest>,
    ) -> Result<Response<VoidInvoiceResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let req = request.into_inner();

        let invoice_id = InvoiceId::from_proto(&req.id)?;

        let invoice = self
            .store
            .void_invoice(actor, invoice_id, tenant_id)
            .await
            .and_then(|inv| {
                mapping::invoices::domain_invoice_with_transactions_to_server(
                    inv.invoice,
                    inv.transactions,
                    self.jwt_secret.clone(),
                )
            })
            .map_err(Into::<InvoiceApiError>::into)?;

        Ok(Response::new(VoidInvoiceResponse {
            invoice: Some(invoice),
        }))
    }

    #[tracing::instrument(skip_all, fields(invoice_id = %request.get_ref().id))]
    async fn regenerate_invoice(
        &self,
        request: Request<RegenerateInvoiceRequest>,
    ) -> Result<Response<RegenerateInvoiceResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let req = request.into_inner();

        let invoice_id = InvoiceId::from_proto(&req.id)?;

        let invoice = self
            .services
            .regenerate_invoice(actor, tenant_id, invoice_id)
            .await
            .and_then(|inv| {
                mapping::invoices::domain_invoice_with_transactions_to_server(
                    inv.invoice,
                    inv.transactions,
                    self.jwt_secret.clone(),
                )
            })
            .map_err(Into::<InvoiceApiError>::into)?;

        Ok(Response::new(RegenerateInvoiceResponse {
            invoice: Some(invoice),
        }))
    }

    #[tracing::instrument(skip_all, fields(invoice_id = %request.get_ref().id))]
    async fn mark_invoice_as_uncollectible(
        &self,
        request: Request<MarkInvoiceAsUncollectibleRequest>,
    ) -> Result<Response<MarkInvoiceAsUncollectibleResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let invoice_id = InvoiceId::from_proto(&req.id)?;

        let invoice = self
            .store
            .mark_invoice_as_uncollectible(invoice_id, tenant_id)
            .await
            .and_then(|inv| {
                mapping::invoices::domain_invoice_with_transactions_to_server(
                    inv.invoice,
                    inv.transactions,
                    self.jwt_secret.clone(),
                )
            })
            .map_err(Into::<InvoiceApiError>::into)?;

        Ok(Response::new(MarkInvoiceAsUncollectibleResponse {
            invoice: Some(invoice),
        }))
    }

    #[tracing::instrument(skip_all, fields(invoice_id = %request.get_ref().invoice_id))]
    async fn generate_invoice_payment_token(
        &self,
        request: Request<GenerateInvoicePaymentTokenRequest>,
    ) -> Result<Response<GenerateInvoicePaymentTokenResponse>, Status> {
        let tenant_id = request.tenant()?;
        let delegated = request.delegated();

        let req = request.into_inner();
        let invoice_id = InvoiceId::from_proto(req.invoice_id)?;

        // A consolidated child is billed via its parent; don't mint a portal payment link for it.
        let invoice = self
            .store
            .get_invoice_by_id(tenant_id, invoice_id)
            .await
            .map_err(Into::<InvoiceApiError>::into)?;
        if invoice.consolidated_into_invoice_id.is_some() {
            return Err(Status::failed_precondition(
                "Cannot generate a payment link for an invoice merged into a consolidated parent",
            ));
        }

        let token = self
            .store
            .invoice_portal_token(tenant_id, invoice_id, delegated)
            .await
            .map_err(Into::<InvoiceApiError>::into)?;

        Ok(Response::new(GenerateInvoicePaymentTokenResponse { token }))
    }

    /// Test gateway only: the connector behind the transaction must be the mock, so a real
    /// provider's transaction can never be settled from here.
    #[tracing::instrument(skip_all)]
    async fn simulate_test_gateway_event(
        &self,
        request: Request<
            meteroid_grpc::meteroid::api::invoices::v1::SimulateTestGatewayEventRequest,
        >,
    ) -> Result<
        Response<meteroid_grpc::meteroid::api::invoices::v1::SimulateTestGatewayEventResponse>,
        Status,
    > {
        use meteroid_app::payment_webhooks::test_gateway::{
            self, TestGatewayError, TestGatewayEvent,
        };
        use meteroid_grpc::meteroid::api::invoices::v1::simulate_test_gateway_event_request::TestGatewayEvent as ProtoEvent;
        use meteroid_store::repositories::connectors::ConnectorsInterface;
        use meteroid_store::repositories::customer_connection::CustomerConnectionInterface;
        use meteroid_store::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
        use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;

        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let event = match ProtoEvent::try_from(req.event) {
            Ok(ProtoEvent::Succeeded) => TestGatewayEvent::Succeeded,
            Ok(ProtoEvent::Failed) => TestGatewayEvent::Failed,
            Ok(ProtoEvent::Refunded) => TestGatewayEvent::Refunded,
            Ok(ProtoEvent::ChargedBack) => TestGatewayEvent::ChargedBack,
            Err(_) => {
                return Err(InvoiceApiError::InvalidArgument(
                    "unknown test gateway event".to_string(),
                )
                .into());
            }
        };
        let transaction_id = PaymentTransactionId::from_proto(req.transaction_id)?;

        let tx = self
            .store
            .get_payment_tx_by_id(tenant_id, transaction_id)
            .await
            .map_err(Into::<InvoiceApiError>::into)?;
        let payment_method_id = tx.payment_method_id.ok_or_else(|| {
            InvoiceApiError::InvalidArgument("this transaction has no payment method".to_string())
        })?;
        let method = self
            .store
            .get_payment_method_by_id(&tenant_id, &payment_method_id)
            .await
            .map_err(Into::<InvoiceApiError>::into)?;
        let connection = self
            .store
            .get_connection_by_id(&tenant_id, &method.connection_id)
            .await
            .map_err(Into::<InvoiceApiError>::into)?;
        let connector = self
            .store
            .get_connector_with_data(connection.connector_id, tenant_id)
            .await
            .map_err(Into::<InvoiceApiError>::into)?;

        test_gateway::simulate(&self.store, &self.services, &connector, &tx, event)
            .await
            .map_err(|report| match report.current_context() {
                TestGatewayError::NotTestGateway | TestGatewayError::InvalidState(..) => {
                    InvoiceApiError::InvalidArgument(report.current_context().to_string())
                }
                TestGatewayError::Dispatch => InvoiceApiError::StoreError(
                    "simulated test gateway event failed".to_string(),
                    Box::new(report.into_error()),
                ),
            })?;

        Ok(Response::new(
            meteroid_grpc::meteroid::api::invoices::v1::SimulateTestGatewayEventResponse {},
        ))
    }

    #[tracing::instrument(skip_all, fields(invoice_id = %request.get_ref().invoice_id))]
    async fn add_manual_payment_transaction(
        &self,
        request: Request<
            meteroid_grpc::meteroid::api::invoices::v1::AddManualPaymentTransactionRequest,
        >,
    ) -> Result<
        Response<meteroid_grpc::meteroid::api::invoices::v1::AddManualPaymentTransactionResponse>,
        Status,
    > {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let req = request.into_inner();

        let invoice_id = InvoiceId::from_proto(req.invoice_id)?;
        let amount = rust_decimal::Decimal::from_proto_ref(&req.amount)?;
        let payment_date = manual_payment_date(req.payment_date)?;

        let transaction = self
            .services
            .add_manual_payment_transaction(
                actor,
                tenant_id,
                invoice_id,
                amount,
                payment_date,
                req.reference,
            )
            .await
            .map_err(Into::<InvoiceApiError>::into)?;

        Ok(Response::new(
            meteroid_grpc::meteroid::api::invoices::v1::AddManualPaymentTransactionResponse {
                transaction_id: transaction.id.as_proto(),
                invoice_id: transaction
                    .invoice_id
                    .ok_or_else(|| {
                        Status::internal("Manual payment transaction missing invoice_id")
                    })?
                    .as_proto(),
                amount: rust_decimal::Decimal::from(transaction.amount).as_proto(),
                currency: transaction.currency,
                status: format!("{:?}", transaction.status),
                payment_date: transaction
                    .processed_at
                    .as_proto()
                    .unwrap_or_else(|| common_domain::clock::now().naive_utc().as_proto()),
            },
        ))
    }

    #[tracing::instrument(skip_all)]
    async fn refund_transaction(
        &self,
        request: Request<meteroid_grpc::meteroid::api::invoices::v1::RefundTransactionRequest>,
    ) -> Result<
        Response<meteroid_grpc::meteroid::api::invoices::v1::RefundTransactionResponse>,
        Status,
    > {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let req = request.into_inner();

        let transaction_id = PaymentTransactionId::from_proto(req.transaction_id)?;
        let amount = self
            .parse_transaction_amount(tenant_id, transaction_id, &req.amount)
            .await?;

        let refund = self
            .services
            .refund_transaction(
                actor,
                tenant_id,
                RefundCommand {
                    parent_transaction_id: transaction_id,
                    amount,
                    credit_note_id: CreditNoteId::from_proto_opt(req.credit_note_id)?,
                    reason: req.reason,
                },
            )
            .await
            .map_err(Into::<InvoiceApiError>::into)?;

        Ok(Response::new(
            meteroid_grpc::meteroid::api::invoices::v1::RefundTransactionResponse {
                transaction: Some(mapping::transactions::domain_to_server(refund)),
            },
        ))
    }

    #[tracing::instrument(skip_all)]
    async fn record_offline_refund(
        &self,
        request: Request<meteroid_grpc::meteroid::api::invoices::v1::RecordOfflineRefundRequest>,
    ) -> Result<
        Response<meteroid_grpc::meteroid::api::invoices::v1::RecordOfflineRefundResponse>,
        Status,
    > {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let req = request.into_inner();

        let transaction_id = PaymentTransactionId::from_proto(req.transaction_id)?;
        let amount = self
            .parse_transaction_amount(tenant_id, transaction_id, &req.amount)
            .await?;
        let refund_date = chrono::NaiveDateTime::from_proto_opt(req.refund_date)?
            .unwrap_or_else(|| common_domain::clock::now().naive_utc());

        let refund = self
            .services
            .record_offline_refund(
                actor,
                tenant_id,
                OfflineRefundCommand {
                    parent_transaction_id: transaction_id,
                    amount,
                    credit_note_id: CreditNoteId::from_proto_opt(req.credit_note_id)?,
                    refund_date,
                    reference: req.reference,
                    reason: req.reason,
                },
            )
            .await
            .map_err(Into::<InvoiceApiError>::into)?;

        Ok(Response::new(
            meteroid_grpc::meteroid::api::invoices::v1::RecordOfflineRefundResponse {
                transaction: Some(mapping::transactions::domain_to_server(refund)),
            },
        ))
    }

    #[tracing::instrument(skip_all)]
    async fn reverse_manual_payment(
        &self,
        request: Request<meteroid_grpc::meteroid::api::invoices::v1::ReverseManualPaymentRequest>,
    ) -> Result<
        Response<meteroid_grpc::meteroid::api::invoices::v1::ReverseManualPaymentResponse>,
        Status,
    > {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let req = request.into_inner();

        let kind = mapping::transactions::reversal_kind_server_to_domain(req.kind)
            .ok_or_else(|| InvoiceApiError::InvalidArgument("unknown reversal kind".to_string()))?;
        let reversed_at = chrono::NaiveDateTime::from_proto_opt(req.reversed_at)?
            .unwrap_or_else(|| common_domain::clock::now().naive_utc());

        let reversed = self
            .services
            .reverse_manual_payment(
                actor,
                tenant_id,
                ReverseManualPaymentCommand {
                    transaction_id: PaymentTransactionId::from_proto(req.transaction_id)?,
                    kind,
                    reason: req.reason,
                    reversed_at,
                },
            )
            .await
            .map_err(Into::<InvoiceApiError>::into)?;

        Ok(Response::new(
            meteroid_grpc::meteroid::api::invoices::v1::ReverseManualPaymentResponse {
                transaction: Some(mapping::transactions::domain_to_server(reversed)),
            },
        ))
    }

    #[tracing::instrument(skip_all, fields(invoice_id = %request.get_ref().invoice_id))]
    async fn mark_invoice_as_paid(
        &self,
        request: Request<meteroid_grpc::meteroid::api::invoices::v1::MarkInvoiceAsPaidRequest>,
    ) -> Result<
        Response<meteroid_grpc::meteroid::api::invoices::v1::MarkInvoiceAsPaidResponse>,
        Status,
    > {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let req = request.into_inner();

        let invoice_id = InvoiceId::from_proto(req.invoice_id)?;
        let total_amount = rust_decimal::Decimal::from_proto_ref(&req.total_amount)?;
        let payment_date = manual_payment_date(req.payment_date)?;

        let invoice = self
            .services
            .mark_invoice_as_paid(
                actor,
                tenant_id,
                invoice_id,
                total_amount,
                payment_date,
                req.reference,
            )
            .await
            .map_err(Into::<InvoiceApiError>::into)?;

        let proto_invoice = mapping::invoices::domain_invoice_with_transactions_to_server(
            invoice.invoice,
            invoice.transactions,
            self.jwt_secret.clone(),
        )
        .map_err(Into::<InvoiceApiError>::into)?;

        Ok(Response::new(
            meteroid_grpc::meteroid::api::invoices::v1::MarkInvoiceAsPaidResponse {
                invoice: Some(proto_invoice),
            },
        ))
    }
}

fn parse_as_minor(s: &String, currency: &meteroid_money::iso::Currency) -> Result<i64, Status> {
    let decimal = rust_decimal::Decimal::from_proto_ref(s)?;

    let money = meteroid_money::Money::from_decimal(decimal, currency);

    let minor_units = money
        .amount()
        .to_subunit_opt(currency.exponent as u8)
        .ok_or_else(|| {
            InvoiceApiError::InputError("Decimal to subunit conversion failed".to_string())
        })?;

    Ok(minor_units)
}

fn parse_as_minor_opt(
    str_ref: Option<&String>,
    currency: &meteroid_money::iso::Currency,
) -> Result<Option<i64>, Status> {
    str_ref
        .map(|str_val| parse_as_minor(str_val, currency))
        .transpose()
}

/// Converts proto sublines to domain. Does NOT compute `total` — callers must
/// recompute from quantity * unit_price using the invoice currency's precision
/// (the client-supplied `total` is ignored; backend is authoritative).
fn convert_sublines_from_proto(
    proto_sublines: &[ProtoSubLineItem],
) -> Result<Vec<meteroid_store::domain::invoice_lines::SubLineItem>, Status> {
    use meteroid_store::domain::invoice_lines::SubLineAttributes;

    proto_sublines
        .iter()
        .map(|sub| {
            let quantity_dec =
                rust_decimal::Decimal::from_proto_ref(&sub.quantity).map_err(|e| {
                    InvoiceApiError::InputError(format!("Invalid subline quantity: {}", e))
                })?;
            let unit_price_dec =
                rust_decimal::Decimal::from_proto_ref(&sub.unit_price).map_err(|e| {
                    InvoiceApiError::InputError(format!("Invalid subline unit price: {}", e))
                })?;

            let attributes = sub.subline_attributes.as_ref().and_then(|attr| {
                use meteroid_grpc::meteroid::api::invoices::v1::sub_line_item::SublineAttributes;
                match attr {
                    SublineAttributes::Tiered(t) => Some(SubLineAttributes::Tiered {
                        first_unit: t.first_unit,
                        last_unit: t.last_unit,
                        flat_cap: rust_decimal::Decimal::from_proto_opt(t.flat_cap.clone())
                            .ok()
                            .flatten(),
                        flat_fee: rust_decimal::Decimal::from_proto_opt(t.flat_fee.clone())
                            .ok()
                            .flatten(),
                    }),
                    SublineAttributes::Volume(v) => Some(SubLineAttributes::Volume {
                        first_unit: v.first_unit,
                        last_unit: v.last_unit,
                        flat_cap: rust_decimal::Decimal::from_proto_opt(v.flat_cap.clone())
                            .ok()
                            .flatten(),
                        flat_fee: rust_decimal::Decimal::from_proto_opt(v.flat_fee.clone())
                            .ok()
                            .flatten(),
                    }),
                    SublineAttributes::Matrix(m) => Some(SubLineAttributes::Matrix {
                        dimension1_key: m.dimension1_key.clone(),
                        dimension1_value: m.dimension1_value.clone(),
                        dimension2_key: m.dimension2_key.clone(),
                        dimension2_value: m.dimension2_value.clone(),
                    }),
                    SublineAttributes::Package(p) => {
                        let raw = rust_decimal::Decimal::from_proto_opt(Some(p.raw_usage.clone()))
                            .ok()
                            .flatten();
                        raw.map(|r| SubLineAttributes::Package { raw_usage: r })
                    }
                }
            });

            Ok(meteroid_store::domain::invoice_lines::SubLineItem {
                local_id: if sub.id.is_empty() {
                    LocalId::generate_for(IdType::Other)
                } else {
                    sub.id.clone()
                },
                name: sub.name.clone(),
                total: 0, // computed by caller using invoice currency precision
                quantity: quantity_dec,
                unit_price: unit_price_dec,
                attributes,
            })
        })
        .collect()
}

async fn to_domain_invoice_new(
    tenant_id: TenantId,
    invoice_req: NewInvoice,
    store: &Store,
) -> Result<(InvoiceNew, InvoicingEntity), Status> {
    let customer_id = CustomerId::from_proto(invoice_req.customer_id.as_str())?;
    let invoice_date = NaiveDate::from_proto_ref(&invoice_req.invoice_date)?;
    let due_date = invoice_req
        .due_date
        .as_ref()
        .map(NaiveDate::from_proto_ref)
        .transpose()?;
    let currency_str = invoice_req.currency.as_str();
    let currency = meteroid_money::iso::find(currency_str)
        .ok_or_else(|| InvoiceApiError::InputError("Invalid currency".to_string()))?;

    let discount = parse_as_minor_opt(invoice_req.discount.as_ref(), currency)?;

    let customer = store
        .find_customer_by_id(customer_id, tenant_id)
        .await
        .map_err(Into::<InvoiceApiError>::into)?;

    // First create line items with initial calculations (before discount)
    let mut lines = vec![];
    for line in &invoice_req.line_items {
        let tax_rate = rust_decimal::Decimal::from_proto_ref(&line.tax_rate)?;
        let start_date = NaiveDate::from_proto_ref(&line.start_date)?;
        let end_date = NaiveDate::from_proto_ref(&line.end_date)?;

        // Handle optional quantity and unit_price (can be null for items with sublines)
        let quantity = line
            .quantity
            .clone()
            .map(rust_decimal::Decimal::from_proto)
            .transpose()?;
        let unit_price = line
            .unit_price
            .clone()
            .map(rust_decimal::Decimal::from_proto)
            .transpose()?;

        // Convert sublines from proto to domain; recompute each total from qty * unit_price
        // using the invoice currency precision (backend is authoritative on totals).
        let mut sub_lines = convert_sublines_from_proto(&line.sub_line_items)?;
        for sl in &mut sub_lines {
            sl.total = (sl.quantity * sl.unit_price)
                .to_subunit_opt(currency.exponent as u8)
                .unwrap_or(0);
        }

        // Calculate amount_subtotal
        // If we have sublines, sum their totals; otherwise calculate from quantity * unit_price
        let amount_subtotal = if !sub_lines.is_empty() {
            sub_lines.iter().map(|s| s.total).sum()
        } else if let (Some(q), Some(p)) = (quantity, unit_price) {
            (q * p).to_subunit_opt(currency.exponent as u8).unwrap_or(0)
        } else {
            0
        };

        let item = LineItem {
            local_id: LocalId::generate_for(IdType::Other),
            name: line.product.clone(),
            // Manual one-off invoice line with no subscription component.
            charge_type: Some(ChargeType::OneTime),
            tax_rate,
            tax_details: vec![], // Will be calculated after discount
            amount_subtotal,
            taxable_amount: amount_subtotal, // Will be updated by distribute_discount
            tax_amount: 0,                   // Will be calculated after discount
            amount_total: 0,                 // Will be calculated after discount
            quantity,
            unit_price,
            start_date,
            end_date,
            sub_lines,
            is_prorated: false,
            price_component_id: None,
            sub_component_id: None,
            sub_add_on_id: None,
            product_id: None,
            metric_id: None,
            description: line.description.clone(),
            group_by_dimensions: None,
            kind: meteroid_store::domain::LineItemKind::Regular,
            credit_grant_amount: None,
            credit_grant_expiry: None,
            credit_grant_included: false,
            taxed_at_purchase: false,
            tax_inclusive_quote: None,
            credit_unit: None,
            credit_grant_unit: None,
            credit_grant_rollover: None,
            promotional_credit: None,
            subscription_id: None,
            usage_ends_at: None,
        };

        lines.push(item);
    }

    // Apply discount proportionally across line items
    if let Some(discount_amount) = discount {
        lines = meteroid_store::services::invoice_lines::discount::distribute_discount(
            lines,
            discount_amount as u64,
        );
    }

    // Tax after discount, rounded once per rate over the whole base like the tax engine.
    // When the operator entered tax-included prices, the tax is carved out of them and the
    // lines come back net, so the total stays the figure that was typed.
    let tax_inclusive = invoice_req.tax_inclusive;
    let (tax_breakdown, discount_tax) = apply_tax_rates_to_priced_lines(&mut lines, tax_inclusive);

    let subtotal: i64 = lines.iter().map(|line| line.amount_subtotal).sum();
    let total_tax_amount: i64 = lines.iter().map(|line| line.tax_amount).sum();
    let total_taxable: i64 = lines.iter().map(|line| line.taxable_amount).sum();
    let total = total_taxable + total_tax_amount;
    let amount_due = total;

    let invoicing_entity = store
        .get_invoicing_entity(tenant_id, Some(customer.invoicing_entity_id))
        .await
        .map_err(Into::<InvoiceApiError>::into)?;

    // Use custom customer details if provided, otherwise use customer from DB
    let customer_details = if let Some(custom_details) = invoice_req.customer_details {
        let billing_address = custom_details
            .billing_address
            .map(DomainAddressWrapper::try_from)
            .transpose()
            .map_err(|_| InvoiceApiError::InputError("Invalid address".to_string()))?
            .map(|w| w.0);

        meteroid_store::domain::InlineCustomer {
            id: customer_id,
            name: custom_details.name,
            billing_address,
            snapshot_at: common_domain::clock::now().naive_utc(),
            vat_number: custom_details.vat_number,
            legal_number: customer.legal_number.clone(),
            buyer_reference: customer.buyer_reference.clone(),
            email: custom_details.email,
            alias: customer.alias,
            preferred_locales: customer.preferred_locales.clone(),
        }
    } else {
        customer.into()
    };

    let invoice_new = InvoiceNew {
        status: meteroid_store::domain::InvoiceStatusEnum::Draft,
        tenant_id,
        customer_id,
        subscription_id: None,
        plan_name: None,
        plan_version_id: None,
        invoice_type: meteroid_store::domain::InvoiceType::OneOff,
        // Manual one-off: not a subscription period, so no cycle to pin.
        cycle_index: None,
        currency: invoice_req.currency,
        line_items: lines,
        coupons: vec![],
        data_updated_at: None,
        invoice_date,
        billing_period_start: invoice_date,
        tax_inclusive,
        finalized_at: None,
        total,
        amount_due,
        applied_credits: 0, // manual invoices don't use customer credits ?
        promotional_credits: 0,
        subtotal,
        subtotal_recurring: 0, // no recurring items in manual invoices
        tax_amount: total_tax_amount,
        net_terms: invoicing_entity.net_terms, // todo derive from due_date?
        discount: discount.unwrap_or(0) - discount_tax,
        invoice_number: "draft".to_string(),
        reference: invoice_req.reference,
        purchase_order: invoice_req.purchase_order.clone(),
        memo: invoice_req.memo,
        due_at: due_date.map(|d| d.and_time(NaiveTime::MIN)),
        customer_details,
        seller_details: invoicing_entity.clone().into(),
        auto_advance: false,
        payment_status: meteroid_store::domain::InvoicePaymentStatus::Unpaid,
        tax_breakdown,
        manual: true,
        invoicing_entity_id: invoicing_entity.id,
        parent_invoice_id: None,
        consolidated_into_invoice_id: None,
        annexes: mapping::invoices::server_annexes_to_domain(invoice_req.annexes)?,
        custom_properties: invoice_req
            .custom_properties
            .map(|s| serde_json::from_str::<serde_json::Value>(&s))
            .transpose()
            .map_err(|e| {
                InvoiceApiError::InputError(format!("Invalid custom_properties JSON: {e}"))
            })?
            .unwrap_or(serde_json::Value::Null),
    };
    Ok((invoice_new, invoicing_entity))
}

fn to_update_invoice_params(req: UpdateInvoiceRequest) -> Result<UpdateInvoiceParams, Status> {
    let line_items = if let Some(line_items_wrapper) = req.line_items {
        let mut items = vec![];
        for line in line_items_wrapper.items {
            // Handle optional quantity and unit_price (can be None for items with sublines)
            let quantity = match &line.quantity {
                Some(q) if !q.is_empty() => Some(rust_decimal::Decimal::from_proto_ref(q)?),
                _ => None,
            };
            let unit_price = match &line.unit_price {
                Some(p) if !p.is_empty() => Some(rust_decimal::Decimal::from_proto_ref(p)?),
                _ => None,
            };
            let tax_rate = rust_decimal::Decimal::from_proto_ref(&line.tax_rate)?;
            let start_date = NaiveDate::from_proto_ref(&line.start_date)?;
            let end_date = NaiveDate::from_proto_ref(&line.end_date)?;

            let sub_lines = convert_sublines_from_proto(&line.sub_line_items)?;

            items.push(UpdateLineItemParams {
                id: line.id,
                name: line.name,
                start_date,
                end_date,
                quantity,
                unit_price,
                tax_rate,
                description: line.description,
                sub_lines,
            });
        }
        Some(items)
    } else {
        None
    };

    let discount = req.discount;

    let customer_details = req.customer_details.map(|cd| {
        if cd.refresh_from_customer {
            CustomerDetailsUpdate::RefreshFromCustomer
        } else {
            use crate::customers::mapping::customer::DomainAddressWrapper;
            CustomerDetailsUpdate::InlineEdit {
                name: cd.name,
                billing_address: cd
                    .billing_address
                    .and_then(|addr| DomainAddressWrapper::try_from(addr).ok().map(|w| w.0)),
                vat_number: cd.vat_number,
            }
        }
    });

    let invoicing_entity_id = req
        .invoicing_entity_id
        .as_ref()
        .map(common_domain::ids::InvoicingEntityId::from_proto)
        .transpose()?;

    let due_date = req
        .due_date
        .as_ref()
        .map(|d| NaiveDate::from_proto_ref(d).map(Some))
        .transpose()?;

    let custom_properties = req
        .custom_properties
        .as_ref()
        .map(|raw| {
            serde_json::from_str::<serde_json::Value>(raw).map_err(|e| {
                Status::invalid_argument(format!("Invalid custom_properties JSON: {e}"))
            })
        })
        .transpose()?;

    let annexes = req
        .annexes
        .map(|a| mapping::invoices::server_annexes_to_domain(a.items))
        .transpose()?;

    Ok(UpdateInvoiceParams {
        memo: req.memo.map(Some),
        reference: req.reference.map(Some),
        purchase_order: req.purchase_order.map(Some),
        due_date,
        line_items,
        discount,
        customer_details,
        invoicing_entity_id,
        annexes,
        custom_properties,
    })
}

/// A manual payment's date as the dashboard sends it, RFC 3339 with the operator's UTC offset, so
/// "today" is the operator's. Without an offset (an older dashboard) it is read as UTC; absent, now.
fn manual_payment_date(
    proto: Option<String>,
) -> Result<chrono::DateTime<chrono::FixedOffset>, Status> {
    let Some(value) = proto else {
        return Ok(common_domain::clock::now().fixed_offset());
    };
    chrono::DateTime::parse_from_rfc3339(&value)
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(&value, "%Y-%m-%dT%H:%M:%S")
                .map(|naive| naive.and_utc().fixed_offset())
        })
        .map_err(|e| Status::invalid_argument(format!("Invalid payment date: {e}")))
}
