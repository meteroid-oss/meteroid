use super::mapping;
use crate::creditnotes::error::CreditNoteApiError;
use crate::invoices::mapping as invoice_mapping;
use common_domain::ids::{CreditNoteId, CustomerId, InvoiceId, TenantId};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::creditnotes::v1::{
    CreateCreditNoteRequest, CreateCreditNoteResponse, CreditType, DeleteDraftCreditNoteRequest,
    DeleteDraftCreditNoteResponse, FinalizeCreditNoteRequest, FinalizeCreditNoteResponse,
    GetCreditNoteRequest, GetCreditNoteResponse, ListCreditNoteRefundsRequest,
    ListCreditNoteRefundsResponse, ListCreditNotesByCustomerIdRequest,
    ListCreditNotesByCustomerIdResponse, ListCreditNotesByInvoiceIdRequest,
    ListCreditNotesByInvoiceIdResponse, ListCreditNotesRequest, ListCreditNotesResponse,
    PreviewCreditNoteSvgRequest, PreviewCreditNoteSvgResponse,
    RequestCreditNotePdfGenerationRequest, RequestCreditNotePdfGenerationResponse,
    SyncCreditNoteToPennylaneRequest, SyncCreditNoteToPennylaneResponse, VoidCreditNoteRequest,
    VoidCreditNoteResponse, credit_notes_service_server::CreditNotesService,
};
use meteroid_grpc_mapping::utils::PaginationExt;
use meteroid_store::domain::{CreditNote as DomainCreditNote, CreditNoteStatus};
use meteroid_store::repositories::CreditNoteInterface;
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::repositories::credit_notes::{
    CreateCreditNoteParams, CreditLineItem, CreditType as DomainCreditType,
};
use meteroid_store::repositories::customers::CustomersInterfaceAuto;
use meteroid_store::repositories::payment_transactions::CreditNoteRefundSummary;
use tonic::{Request, Response, Status};

use crate::creditnotes::CreditNoteServiceComponents;

impl CreditNoteServiceComponents {
    /// The refund figures the credit note page shows. Only a finalized Refund credit note can be
    /// drawn on, so every other one reports nothing and costs no query.
    async fn refund_summary(
        &self,
        tenant_id: TenantId,
        credit_note: &DomainCreditNote,
    ) -> Result<Option<CreditNoteRefundSummary>, Status> {
        if credit_note.credit_type != DomainCreditType::Refund
            || credit_note.status != CreditNoteStatus::Finalized
        {
            return Ok(None);
        }

        let summary = self
            .store
            .get_credit_note_refunds(tenant_id, credit_note.id)
            .await
            .map_err(Into::<CreditNoteApiError>::into)?;

        Ok(Some(summary))
    }
}

#[tonic::async_trait]
impl CreditNotesService for CreditNoteServiceComponents {
    #[tracing::instrument(skip_all, fields(
        customer_id = request.get_ref().customer_id.as_deref(),
        invoice_id = request.get_ref().invoice_id.as_deref(),
    ))]
    async fn list_credit_notes(
        &self,
        request: Request<ListCreditNotesRequest>,
    ) -> Result<Response<ListCreditNotesResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let customer_id = CustomerId::from_proto_opt(inner.customer_id)?;
        let invoice_id = InvoiceId::from_proto_opt(inner.invoice_id)?;
        let status = mapping::credit_note_status_server_to_domain(inner.status);

        let pagination_req = inner.pagination.clone().into_domain();

        let res = self
            .store
            .list_credit_notes(
                tenant_id,
                customer_id,
                invoice_id,
                status,
                inner.search,
                meteroid_grpc_mapping::filters::map_filters(inner.filters)?,
                inner.order_by,
                pagination_req,
            )
            .await
            .map_err(Into::<CreditNoteApiError>::into)?;

        let credit_notes: Vec<_> = res
            .items
            .into_iter()
            .map(|cn| mapping::domain_to_server(cn.clone(), cn.customer_details.name.clone()))
            .collect();

        let response = ListCreditNotesResponse {
            pagination_meta: inner
                .pagination
                .into_response(res.total_pages, res.total_results),
            credit_notes,
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn get_credit_note(
        &self,
        request: Request<GetCreditNoteRequest>,
    ) -> Result<Response<GetCreditNoteResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let credit_note_id = CreditNoteId::from_proto(&req.id)?;

        let credit_note = self
            .store
            .get_credit_note_by_id(tenant_id, credit_note_id)
            .await
            .map_err(Into::<CreditNoteApiError>::into)?;

        let customer = self
            .store
            .find_customer_by_id(credit_note.customer_id, tenant_id)
            .await
            .map_err(Into::<CreditNoteApiError>::into)?;

        let refunds = self.refund_summary(tenant_id, &credit_note).await?;
        let invoice = self
            .store
            .get_invoice_by_id(tenant_id, credit_note.invoice_id)
            .await
            .map_err(Into::<CreditNoteApiError>::into)?;
        let promotional_credit_restored = self
            .store
            .promotional_credit_restored(&credit_note)
            .await
            .map_err(Into::<CreditNoteApiError>::into)?;

        let detailed = meteroid_store::domain::DetailedCreditNote {
            credit_note,
            customer,
        };

        let mut credit_note = mapping::detailed_domain_to_server(
            detailed,
            Some(&invoice.line_items),
            refunds.as_ref(),
            &self.jwt_secret,
        )?;
        credit_note.promotional_credit_restored_cents = promotional_credit_restored;
        let response = GetCreditNoteResponse {
            credit_note: Some(credit_note),
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all, fields(invoice_id = %request.get_ref().invoice_id))]
    async fn list_credit_notes_by_invoice_id(
        &self,
        request: Request<ListCreditNotesByInvoiceIdRequest>,
    ) -> Result<Response<ListCreditNotesByInvoiceIdResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let invoice_id = InvoiceId::from_proto(&req.invoice_id)?;

        let credit_notes = self
            .store
            .list_credit_notes_by_invoice_id(tenant_id, invoice_id)
            .await
            .map_err(Into::<CreditNoteApiError>::into)?;

        let invoice = self
            .store
            .get_invoice_by_id(tenant_id, invoice_id)
            .await
            .map_err(Into::<CreditNoteApiError>::into)?;

        let mut detailed_credit_notes = vec![];

        for cn in credit_notes {
            let customer = self
                .store
                .find_customer_by_id(cn.customer_id, tenant_id)
                .await
                .map_err(Into::<CreditNoteApiError>::into)?;

            let detailed = meteroid_store::domain::DetailedCreditNote {
                credit_note: cn,
                customer,
            };

            detailed_credit_notes.push(mapping::detailed_domain_to_server(
                detailed,
                Some(&invoice.line_items),
                None,
                &self.jwt_secret,
            )?);
        }

        let response = ListCreditNotesByInvoiceIdResponse {
            credit_notes: detailed_credit_notes,
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all, fields(customer_id = %request.get_ref().customer_id))]
    async fn list_credit_notes_by_customer_id(
        &self,
        request: Request<ListCreditNotesByCustomerIdRequest>,
    ) -> Result<Response<ListCreditNotesByCustomerIdResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let customer_id = CustomerId::from_proto(&req.customer_id)?;

        let credit_notes = self
            .store
            .list_credit_notes_by_customer_id(tenant_id, customer_id)
            .await
            .map_err(Into::<CreditNoteApiError>::into)?;

        let customer = self
            .store
            .find_customer_by_id(customer_id, tenant_id)
            .await
            .map_err(Into::<CreditNoteApiError>::into)?;

        // The credited invoices' lines state a tax-inclusive note's net promotional share.
        let invoice_ids = credit_notes
            .iter()
            .map(|cn| cn.invoice_id)
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        let invoices: std::collections::HashMap<_, _> = self
            .store
            .list_invoices_by_ids(invoice_ids)
            .await
            .map_err(Into::<CreditNoteApiError>::into)?
            .into_iter()
            .map(|invoice| (invoice.id, invoice.line_items))
            .collect();

        let mut detailed_credit_notes = vec![];

        for cn in credit_notes {
            let invoice_lines = invoices.get(&cn.invoice_id).map(Vec::as_slice);
            let detailed = meteroid_store::domain::DetailedCreditNote {
                credit_note: cn,
                customer: customer.clone(),
            };

            detailed_credit_notes.push(mapping::detailed_domain_to_server(
                detailed,
                invoice_lines,
                None,
                &self.jwt_secret,
            )?);
        }

        let response = ListCreditNotesByCustomerIdResponse {
            credit_notes: detailed_credit_notes,
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn create_credit_note(
        &self,
        request: Request<CreateCreditNoteRequest>,
    ) -> Result<Response<CreateCreditNoteResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let req = request.into_inner();

        let credit_note_req = req
            .credit_note
            .ok_or_else(|| CreditNoteApiError::MissingArgument("credit_note".to_string()))?;

        let invoice_id = InvoiceId::from_proto(&credit_note_req.invoice_id)?;

        let line_items: Vec<CreditLineItem> = credit_note_req
            .line_items
            .iter()
            .map(mapping::credit_line_item_server_to_domain)
            .collect::<Result<Vec<_>, _>>()?;

        // Convert credit type
        let credit_type = match CreditType::try_from(credit_note_req.credit_type)
            .map_err(|_| CreditNoteApiError::InvalidArgument("Invalid credit type".to_string()))?
        {
            CreditType::CreditToBalance => DomainCreditType::CreditToBalance,
            CreditType::Refund => DomainCreditType::Refund,
            CreditType::DebtCancellation => DomainCreditType::DebtCancellation,
        };

        let custom_properties = credit_note_req
            .custom_properties
            .map(|s| serde_json::from_str::<serde_json::Value>(&s))
            .transpose()
            .map_err(|e| {
                CreditNoteApiError::InvalidArgument(format!("Invalid custom_properties JSON: {e}"))
            })?;

        let params = CreateCreditNoteParams {
            invoice_id,
            line_items,
            reason: credit_note_req.reason,
            memo: credit_note_req.memo,
            credit_type,
            custom_properties,
        };

        if req.reissue_as_draft && !req.finalize {
            return Err(Status::invalid_argument(
                "reissue_as_draft requires finalize=true",
            ));
        }

        let (created, corrected_invoice_id) = if req.finalize && req.reissue_as_draft {
            let (cn, inv) = self
                .services
                .create_and_finalize_credit_note_with_reissue(
                    actor.clone(),
                    tenant_id,
                    params,
                    true,
                )
                .await
                .map_err(Into::<CreditNoteApiError>::into)?;
            (cn, inv.map(|i| i.id.as_proto()))
        } else if req.finalize {
            let cn = self
                .store
                .create_and_finalize_credit_note(actor.clone(), tenant_id, params)
                .await
                .map_err(Into::<CreditNoteApiError>::into)?;
            (cn, None)
        } else {
            let cn = self
                .store
                .create_credit_note(actor.clone(), tenant_id, params)
                .await
                .map_err(Into::<CreditNoteApiError>::into)?;
            (cn, None)
        };

        let customer = self
            .store
            .find_customer_by_id(created.customer_id, tenant_id)
            .await
            .map_err(Into::<CreditNoteApiError>::into)?;

        let refunds = self.refund_summary(tenant_id, &created).await?;

        let invoice = self
            .store
            .get_invoice_by_id(tenant_id, created.invoice_id)
            .await
            .map_err(Into::<CreditNoteApiError>::into)?;

        let detailed = meteroid_store::domain::DetailedCreditNote {
            credit_note: created,
            customer,
        };

        let response = CreateCreditNoteResponse {
            credit_note: Some(mapping::detailed_domain_to_server(
                detailed,
                Some(&invoice.line_items),
                refunds.as_ref(),
                &self.jwt_secret,
            )?),
            corrected_invoice_id,
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn finalize_credit_note(
        &self,
        request: Request<FinalizeCreditNoteRequest>,
    ) -> Result<Response<FinalizeCreditNoteResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let req = request.into_inner();

        let credit_note_id = CreditNoteId::from_proto(&req.id)?;

        let finalized = self
            .store
            .finalize_credit_note(actor, tenant_id, credit_note_id)
            .await
            .map_err(Into::<CreditNoteApiError>::into)?;

        let customer = self
            .store
            .find_customer_by_id(finalized.customer_id, tenant_id)
            .await
            .map_err(Into::<CreditNoteApiError>::into)?;

        let refunds = self.refund_summary(tenant_id, &finalized).await?;

        let invoice = self
            .store
            .get_invoice_by_id(tenant_id, finalized.invoice_id)
            .await
            .map_err(Into::<CreditNoteApiError>::into)?;

        let detailed = meteroid_store::domain::DetailedCreditNote {
            credit_note: finalized,
            customer,
        };

        let response = FinalizeCreditNoteResponse {
            credit_note: Some(mapping::detailed_domain_to_server(
                detailed,
                Some(&invoice.line_items),
                refunds.as_ref(),
                &self.jwt_secret,
            )?),
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn void_credit_note(
        &self,
        request: Request<VoidCreditNoteRequest>,
    ) -> Result<Response<VoidCreditNoteResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let req = request.into_inner();

        let credit_note_id = CreditNoteId::from_proto(&req.id)?;

        let voided = self
            .store
            .void_credit_note(actor, tenant_id, credit_note_id)
            .await
            .map_err(Into::<CreditNoteApiError>::into)?;

        let customer = self
            .store
            .find_customer_by_id(voided.customer_id, tenant_id)
            .await
            .map_err(Into::<CreditNoteApiError>::into)?;

        let refunds = self.refund_summary(tenant_id, &voided).await?;

        let invoice = self
            .store
            .get_invoice_by_id(tenant_id, voided.invoice_id)
            .await
            .map_err(Into::<CreditNoteApiError>::into)?;

        let detailed = meteroid_store::domain::DetailedCreditNote {
            credit_note: voided,
            customer,
        };

        let response = VoidCreditNoteResponse {
            credit_note: Some(mapping::detailed_domain_to_server(
                detailed,
                Some(&invoice.line_items),
                refunds.as_ref(),
                &self.jwt_secret,
            )?),
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn delete_draft_credit_note(
        &self,
        request: Request<DeleteDraftCreditNoteRequest>,
    ) -> Result<Response<DeleteDraftCreditNoteResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let credit_note_id = CreditNoteId::from_proto(&req.id)?;

        self.store
            .delete_draft_credit_note(tenant_id, credit_note_id)
            .await
            .map_err(Into::<CreditNoteApiError>::into)?;

        Ok(Response::new(DeleteDraftCreditNoteResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn preview_credit_note_svg(
        &self,
        request: Request<PreviewCreditNoteSvgRequest>,
    ) -> Result<Response<PreviewCreditNoteSvgResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let credit_note_id = CreditNoteId::from_proto(&req.id)?;

        let svgs = self
            .preview_rendering
            .preview_credit_note_by_id(credit_note_id, tenant_id)
            .await
            .map_err(Into::<CreditNoteApiError>::into)?;

        let response = PreviewCreditNoteSvgResponse { svgs };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn request_credit_note_pdf_generation(
        &self,
        request: Request<RequestCreditNotePdfGenerationRequest>,
    ) -> Result<Response<RequestCreditNotePdfGenerationResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let credit_note_id = CreditNoteId::from_proto(&req.id)?;

        // Verify the credit note exists and belongs to the tenant
        let _credit_note = self
            .store
            .get_credit_note_by_id(tenant_id, credit_note_id)
            .await
            .map_err(Into::<CreditNoteApiError>::into)?;

        // TODO: Queue the PDF generation request via outbox event
        // For now, this is a placeholder - the actual PDF generation
        // will be triggered by the finalize_credit_note flow

        Ok(Response::new(RequestCreditNotePdfGenerationResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn list_credit_note_refunds(
        &self,
        request: Request<ListCreditNoteRefundsRequest>,
    ) -> Result<Response<ListCreditNoteRefundsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let credit_note_id = CreditNoteId::from_proto(&req.credit_note_id)?;

        let summary = self
            .store
            .get_credit_note_refunds(tenant_id, credit_note_id)
            .await
            .map_err(Into::<CreditNoteApiError>::into)?;

        Ok(Response::new(ListCreditNoteRefundsResponse {
            refunds: summary
                .refunds
                .into_iter()
                .map(invoice_mapping::transactions::domain_to_server)
                .collect(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn sync_credit_note_to_pennylane(
        &self,
        request: Request<SyncCreditNoteToPennylaneRequest>,
    ) -> Result<Response<SyncCreditNoteToPennylaneResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let ids = req
            .credit_note_ids
            .iter()
            .map(CreditNoteId::from_proto)
            .collect::<Result<Vec<_>, _>>()?;

        self.store
            .sync_credit_notes_to_pennylane(ids, tenant_id)
            .await
            .map_err(Into::<CreditNoteApiError>::into)?;

        Ok(Response::new(SyncCreditNoteToPennylaneResponse {}))
    }
}
