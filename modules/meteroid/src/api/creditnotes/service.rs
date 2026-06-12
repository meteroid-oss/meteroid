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
use meteroid_store::repositories::CreditNoteInterface;
use meteroid_store::repositories::credit_notes::{
    CreateCreditNoteParams, CreditLineItem, CreditSubLineItem, CreditType as DomainCreditType,
};
use meteroid_store::repositories::customers::CustomersInterfaceAuto;
use std::str::FromStr;
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

        let res = self
            .store
            .list_credit_notes(
                tenant_id,
                customer_id,
                invoice_id,
                status,
                inner.search,
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

        let detailed = meteroid_store::domain::DetailedCreditNote {
            credit_note,
            customer,
        };

        let response = GetCreditNoteResponse {
            credit_note: Some(mapping::detailed_domain_to_server(
                detailed,
                &self.jwt_secret,
            )?),
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

            detailed_credit_notes.push(mapping::detailed_domain_to_server(
                detailed,
                &self.jwt_secret,
            )?);
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

            detailed_credit_notes.push(mapping::detailed_domain_to_server(
                detailed,
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
        let req = request.into_inner();

        let credit_note_req = req
            .credit_note
            .ok_or_else(|| CreditNoteApiError::MissingArgument("credit_note".to_string()))?;

        let invoice_id = InvoiceId::from_proto(&credit_note_req.invoice_id)?;

        // Convert line items: either per-subline overrides or a line-level quantity.
        let line_items: Vec<CreditLineItem> = credit_note_req
            .line_items
            .iter()
            .map(|li| {
                let local_id = li.line_item_local_id.clone();
                if !li.sub_lines.is_empty() {
                    if li.quantity.is_some() {
                        return Err(CreditNoteApiError::InvalidArgument(format!(
                            "Line '{}': quantity and sub_lines are mutually exclusive",
                            local_id
                        )));
                    }
                    let sub_lines = li
                        .sub_lines
                        .iter()
                        .map(|sl| {
                            let quantity =
                                rust_decimal::Decimal::from_str(&sl.quantity).map_err(|e| {
                                    CreditNoteApiError::InvalidArgument(format!(
                                        "Invalid sub-line quantity: {}",
                                        e
                                    ))
                                })?;
                            let unit_price = sl
                                .unit_price
                                .as_ref()
                                .map(|s| rust_decimal::Decimal::from_str(s))
                                .transpose()
                                .map_err(|e| {
                                    CreditNoteApiError::InvalidArgument(format!(
                                        "Invalid sub-line unit_price: {}",
                                        e
                                    ))
                                })?;
                            Ok::<_, CreditNoteApiError>(CreditSubLineItem {
                                local_id: sl.sub_line_local_id.clone(),
                                quantity,
                                unit_price,
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(CreditLineItem::SubLines {
                        local_id,
                        sub_lines,
                    })
                } else {
                    let qty_str = li.quantity.as_ref().ok_or_else(|| {
                        CreditNoteApiError::InvalidArgument(format!(
                            "Line '{}': quantity or sub_lines is required",
                            local_id
                        ))
                    })?;
                    let quantity = rust_decimal::Decimal::from_str(qty_str).map_err(|e| {
                        CreditNoteApiError::InvalidArgument(format!("Invalid quantity: {}", e))
                    })?;
                    let unit_price = li
                        .unit_price
                        .as_ref()
                        .map(|s| rust_decimal::Decimal::from_str(s))
                        .transpose()
                        .map_err(|e| {
                            CreditNoteApiError::InvalidArgument(format!(
                                "Invalid unit_price: {}",
                                e
                            ))
                        })?;
                    Ok(CreditLineItem::Line {
                        local_id,
                        quantity,
                        unit_price,
                    })
                }
            })
            .collect::<Result<Vec<_>, _>>()?;

        // Convert credit type
        let credit_type = match CreditType::try_from(credit_note_req.credit_type)
            .map_err(|_| CreditNoteApiError::InvalidArgument("Invalid credit type".to_string()))?
        {
            CreditType::CreditToBalance => DomainCreditType::CreditToBalance,
            CreditType::Refund => DomainCreditType::Refund,
            CreditType::DebtCancellation => DomainCreditType::DebtCancellation,
        };

        let params = CreateCreditNoteParams {
            invoice_id,
            line_items,
            reason: credit_note_req.reason,
            memo: credit_note_req.memo,
            credit_type,
        };

        if req.reissue_as_draft && !req.finalize {
            return Err(Status::invalid_argument(
                "reissue_as_draft requires finalize=true",
            ));
        }

        let (created, corrected_invoice_id) = if req.finalize && req.reissue_as_draft {
            let (cn, inv) = self
                .services
                .create_and_finalize_credit_note_with_reissue(tenant_id, params, true)
                .await
                .map_err(Into::<CreditNoteApiError>::into)?;
            (cn, inv.map(|i| i.id.as_proto()))
        } else if req.finalize {
            let cn = self
                .store
                .create_and_finalize_credit_note(tenant_id, params)
                .await
                .map_err(Into::<CreditNoteApiError>::into)?;
            (cn, None)
        } else {
            let cn = self
                .store
                .create_credit_note(tenant_id, params)
                .await
                .map_err(Into::<CreditNoteApiError>::into)?;
            (cn, None)
        };

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
            credit_note: Some(mapping::detailed_domain_to_server(
                detailed,
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
            credit_note: Some(mapping::detailed_domain_to_server(
                detailed,
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
            credit_note: Some(mapping::detailed_domain_to_server(
                detailed,
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
}
