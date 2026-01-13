use super::mapping;
use crate::api::creditnotes::error::CreditNoteApiError;
use crate::api::utils::PaginationExt;
use common_domain::ids::{CreditNoteId, CustomerId, InvoiceId};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::creditnotes::v1::{
    CreateCreditNoteRequest, CreateCreditNoteResponse, CreditType, DeleteDraftCreditNoteRequest,
    DeleteDraftCreditNoteResponse, FinalizeCreditNoteRequest, FinalizeCreditNoteResponse,
    GetCreditNoteRequest, GetCreditNoteResponse, ListCreditNotesByCustomerIdRequest,
    ListCreditNotesByCustomerIdResponse, ListCreditNotesByInvoiceIdRequest,
    ListCreditNotesByInvoiceIdResponse, ListCreditNotesRequest, ListCreditNotesResponse,
    PreviewCreditNoteSvgRequest, PreviewCreditNoteSvgResponse,
    RequestCreditNotePdfGenerationRequest, RequestCreditNotePdfGenerationResponse,
    VoidCreditNoteRequest, VoidCreditNoteResponse, credit_notes_service_server::CreditNotesService,
};
use meteroid_store::domain::OrderByRequest;
use meteroid_store::repositories::CreditNoteInterface;
use meteroid_store::repositories::credit_notes::{
    CreateCreditNoteParams, CreditLineItem, CreditType as DomainCreditType,
};
use meteroid_store::repositories::customers::CustomersInterfaceAuto;
use tonic::{Request, Response, Status};

use crate::api::creditnotes::CreditNoteServiceComponents;

#[tonic::async_trait]
impl CreditNotesService for CreditNoteServiceComponents {
    #[tracing::instrument(skip_all)]
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

        let order_by = match inner.sort_by.try_into() {
            Ok(meteroid_grpc::meteroid::api::creditnotes::v1::list_credit_notes_request::SortBy::DateDesc) => OrderByRequest::DateDesc,
            Ok(meteroid_grpc::meteroid::api::creditnotes::v1::list_credit_notes_request::SortBy::DateAsc) => OrderByRequest::DateAsc,
            Ok(meteroid_grpc::meteroid::api::creditnotes::v1::list_credit_notes_request::SortBy::IdDesc) => OrderByRequest::IdDesc,
            Ok(meteroid_grpc::meteroid::api::creditnotes::v1::list_credit_notes_request::SortBy::IdAsc) => OrderByRequest::IdAsc,
            Ok(meteroid_grpc::meteroid::api::creditnotes::v1::list_credit_notes_request::SortBy::NumberAsc) => OrderByRequest::NameAsc,
            Ok(meteroid_grpc::meteroid::api::creditnotes::v1::list_credit_notes_request::SortBy::NumberDesc) => OrderByRequest::NameDesc,
            Err(_) => OrderByRequest::DateDesc,
        };

        let res = self
            .store
            .list_credit_notes(
                tenant_id,
                customer_id,
                invoice_id,
                status,
                inner.search,
                order_by,
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

        let detailed = meteroid_store::domain::DetailedCreditNote {
            credit_note,
            customer,
        };

        let response = GetCreditNoteResponse {
            credit_note: Some(mapping::detailed_domain_to_server(detailed)?),
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
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

            detailed_credit_notes.push(mapping::detailed_domain_to_server(detailed)?);
        }

        let response = ListCreditNotesByInvoiceIdResponse {
            credit_notes: detailed_credit_notes,
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
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

        let mut detailed_credit_notes = vec![];

        for cn in credit_notes {
            let detailed = meteroid_store::domain::DetailedCreditNote {
                credit_note: cn,
                customer: customer.clone(),
            };

            detailed_credit_notes.push(mapping::detailed_domain_to_server(detailed)?);
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
        let req = request.into_inner();

        let credit_note_req = req
            .credit_note
            .ok_or_else(|| CreditNoteApiError::MissingArgument("credit_note".to_string()))?;

        let invoice_id = InvoiceId::from_proto(&credit_note_req.invoice_id)?;

        // Convert line items with optional amounts
        let line_items: Vec<CreditLineItem> = credit_note_req
            .line_items
            .iter()
            .map(|li| CreditLineItem {
                local_id: li.line_item_local_id.clone(),
                amount: li.amount,
            })
            .collect();

        // Convert credit type
        let credit_type = match CreditType::try_from(credit_note_req.credit_type)
            .map_err(|_| CreditNoteApiError::InvalidArgument("Invalid credit type".to_string()))?
        {
            CreditType::CreditToBalance => DomainCreditType::CreditToBalance,
            CreditType::Refund => DomainCreditType::Refund,
        };

        let params = CreateCreditNoteParams {
            invoice_id,
            line_items,
            reason: credit_note_req.reason,
            memo: credit_note_req.memo,
            credit_type,
        };

        let created = self
            .store
            .create_credit_note(tenant_id, params)
            .await
            .map_err(Into::<CreditNoteApiError>::into)?;

        let customer = self
            .store
            .find_customer_by_id(created.customer_id, tenant_id)
            .await
            .map_err(Into::<CreditNoteApiError>::into)?;

        let detailed = meteroid_store::domain::DetailedCreditNote {
            credit_note: created,
            customer,
        };

        let response = CreateCreditNoteResponse {
            credit_note: Some(mapping::detailed_domain_to_server(detailed)?),
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn finalize_credit_note(
        &self,
        request: Request<FinalizeCreditNoteRequest>,
    ) -> Result<Response<FinalizeCreditNoteResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let credit_note_id = CreditNoteId::from_proto(&req.id)?;

        let finalized = self
            .store
            .finalize_credit_note(tenant_id, credit_note_id)
            .await
            .map_err(Into::<CreditNoteApiError>::into)?;

        let customer = self
            .store
            .find_customer_by_id(finalized.customer_id, tenant_id)
            .await
            .map_err(Into::<CreditNoteApiError>::into)?;

        let detailed = meteroid_store::domain::DetailedCreditNote {
            credit_note: finalized,
            customer,
        };

        let response = FinalizeCreditNoteResponse {
            credit_note: Some(mapping::detailed_domain_to_server(detailed)?),
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn void_credit_note(
        &self,
        request: Request<VoidCreditNoteRequest>,
    ) -> Result<Response<VoidCreditNoteResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let credit_note_id = CreditNoteId::from_proto(&req.id)?;

        let voided = self
            .store
            .void_credit_note(tenant_id, credit_note_id)
            .await
            .map_err(Into::<CreditNoteApiError>::into)?;

        let customer = self
            .store
            .find_customer_by_id(voided.customer_id, tenant_id)
            .await
            .map_err(Into::<CreditNoteApiError>::into)?;

        let detailed = meteroid_store::domain::DetailedCreditNote {
            credit_note: voided,
            customer,
        };

        let response = VoidCreditNoteResponse {
            credit_note: Some(mapping::detailed_domain_to_server(detailed)?),
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
}
