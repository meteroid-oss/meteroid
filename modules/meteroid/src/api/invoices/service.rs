use super::{InvoiceServiceComponents, mapping};
use crate::api::customers::mapping::customer::DomainAddressWrapper;
use crate::api::invoices::error::InvoiceApiError;
use crate::api::shared::conversions::{AsProtoOpt, FromProtoOpt, ProtoConv};
use crate::api::utils::PaginationExt;
use chrono::{NaiveDate, NaiveTime};
use common_domain::ids::{CustomerId, InvoiceId, SubscriptionId, TenantId};
use common_grpc::middleware::server::auth::RequestExt;
use common_utils::decimals::ToSubunit;
use meteroid_grpc::meteroid::api::invoices::v1::{
    CreateInvoiceRequest, CreateInvoiceResponse, DeleteInvoiceRequest, DeleteInvoiceResponse,
    FinalizeInvoiceRequest, FinalizeInvoiceResponse, GenerateInvoicePaymentTokenRequest,
    GenerateInvoicePaymentTokenResponse, GetInvoiceRequest, GetInvoiceResponse, Invoice,
    ListInvoicesRequest, ListInvoicesResponse, MarkInvoiceAsUncollectibleRequest,
    MarkInvoiceAsUncollectibleResponse, NewInvoice, PreviewInvoiceRequest, PreviewInvoiceResponse,
    PreviewInvoiceUpdateRequest, PreviewInvoiceUpdateResponse, PreviewNewInvoiceRequest,
    PreviewNewInvoiceResponse, RefreshInvoiceDataRequest, RefreshInvoiceDataResponse,
    RequestPdfGenerationRequest, RequestPdfGenerationResponse, SubLineItem as ProtoSubLineItem,
    SyncToPennylaneRequest, SyncToPennylaneResponse, UpdateInvoiceRequest, UpdateInvoiceResponse,
    VoidInvoiceRequest, VoidInvoiceResponse, invoices_service_server::InvoicesService,
    list_invoices_request::SortBy,
};
use meteroid_store::Store;
use meteroid_store::domain::pgmq::{InvoicePdfRequestEvent, PgmqMessageNew, PgmqQueue};
use meteroid_store::domain::{
    InvoiceNew, InvoicingEntity, LineItem, OrderByRequest, UpdateInvoiceParams,
    UpdateLineItemParams,
};
use meteroid_store::repositories::invoices::compute_tax_breakdown;
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;
use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;
use meteroid_store::repositories::pgmq::PgmqInterface;
use meteroid_store::repositories::{CustomersInterface, InvoiceInterface};
use meteroid_store::services::CustomerDetailsUpdate;
use meteroid_store::utils::local_id::{IdType, LocalId};
use tonic::{Request, Response, Status};

#[tonic::async_trait]
impl InvoicesService for InvoiceServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn list_invoices(
        &self,
        request: Request<ListInvoicesRequest>,
    ) -> Result<Response<ListInvoicesResponse>, Status> {
        let tenant_id = request.tenant()?;

        let inner = request.into_inner();

        let customer_id = CustomerId::from_proto_opt(inner.customer_id)?;
        let subscription_id = SubscriptionId::from_proto_opt(inner.subscription_id)?;

        let pagination_req = inner.pagination.into_domain();

        let order_by = match inner.sort_by.try_into() {
            Ok(SortBy::DateAsc) => OrderByRequest::DateAsc,
            Ok(SortBy::DateDesc) => OrderByRequest::DateDesc,
            Ok(SortBy::IdAsc) => OrderByRequest::IdAsc,
            Ok(SortBy::IdDesc) => OrderByRequest::IdDesc,
            Ok(SortBy::NumberAsc) => OrderByRequest::NameAsc,
            Ok(SortBy::NumberDesc) => OrderByRequest::NameDesc,
            Err(_) => OrderByRequest::DateDesc,
        };

        let res = self
            .store
            .list_invoices(
                tenant_id,
                customer_id,
                subscription_id,
                mapping::invoices::status_server_to_domain(inner.status),
                inner.search,
                order_by,
                pagination_req,
            )
            .await
            .map_err(Into::<InvoiceApiError>::into)?;

        let response = ListInvoicesResponse {
            pagination_meta: inner
                .pagination
                .into_response(res.total_pages, res.total_results),
            invoices: res
                .items
                .into_iter()
                .map(mapping::invoices::domain_to_server)
                .collect::<Vec<Invoice>>(),
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
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

        let mut invoice = self
            .store
            .get_detailed_invoice_by_id(tenant_id, invoice_id)
            .await
            .and_then(|inv| {
                mapping::invoices::domain_invoice_with_transactions_to_server(
                    inv.invoice,
                    inv.transactions,
                    self.jwt_secret.clone(),
                )
            })
            .map_err(Into::<InvoiceApiError>::into)?;

        invoice.transactions = transactions;

        let response = GetInvoiceResponse {
            invoice: Some(invoice),
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
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

    // for demo & local use when the worker was not started initially
    #[tracing::instrument(skip_all)]
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

        let pgmq_msg_new: PgmqMessageNew = InvoicePdfRequestEvent::new(invoice.invoice.id, false)
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

    #[tracing::instrument(skip_all)]
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

    #[tracing::instrument(skip_all)]
    async fn finalize_invoice(
        &self,
        request: Request<FinalizeInvoiceRequest>,
    ) -> Result<Response<FinalizeInvoiceResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();

        let invoice_id = InvoiceId::from_proto(&req.id)?;

        let finalized = self
            .services
            .finalize_invoice(invoice_id, tenant_id)
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

    #[tracing::instrument(skip_all)]
    async fn update_invoice(
        &self,
        request: Request<UpdateInvoiceRequest>,
    ) -> Result<Response<UpdateInvoiceResponse>, Status> {
        let tenant_id = request.tenant()?;

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

    #[tracing::instrument(skip_all)]
    async fn preview_invoice_update(
        &self,
        request: Request<PreviewInvoiceUpdateRequest>,
    ) -> Result<Response<PreviewInvoiceUpdateResponse>, Status> {
        let tenant_id = request.tenant()?;

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

    #[tracing::instrument(skip_all)]
    async fn delete_invoice(
        &self,
        request: Request<DeleteInvoiceRequest>,
    ) -> Result<Response<DeleteInvoiceResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let invoice_id = InvoiceId::from_proto(&req.id)?;

        self.store
            .delete_invoice(invoice_id, tenant_id)
            .await
            .map_err(Into::<InvoiceApiError>::into)?;

        Ok(Response::new(DeleteInvoiceResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn void_invoice(
        &self,
        request: Request<VoidInvoiceRequest>,
    ) -> Result<Response<VoidInvoiceResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let invoice_id = InvoiceId::from_proto(&req.id)?;

        let invoice = self
            .store
            .void_invoice(invoice_id, tenant_id)
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

    #[tracing::instrument(skip_all)]
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

    #[tracing::instrument(skip_all)]
    async fn generate_invoice_payment_token(
        &self,
        request: Request<GenerateInvoicePaymentTokenRequest>,
    ) -> Result<Response<GenerateInvoicePaymentTokenResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();
        let invoice_id = InvoiceId::from_proto(req.invoice_id)?;

        // Generate the JWT token for invoice portal access
        let token = meteroid_store::jwt_claims::generate_portal_token(
            &self.jwt_secret,
            tenant_id,
            meteroid_store::jwt_claims::ResourceAccess::Invoice(invoice_id),
        )
        .map_err(Into::<InvoiceApiError>::into)?;

        Ok(Response::new(GenerateInvoicePaymentTokenResponse { token }))
    }

    #[tracing::instrument(skip_all)]
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
        let req = request.into_inner();

        let invoice_id = InvoiceId::from_proto(req.invoice_id)?;
        let amount = rust_decimal::Decimal::from_proto_ref(&req.amount)?;
        let payment_date = chrono::NaiveDateTime::from_proto_opt(req.payment_date)?
            .unwrap_or_else(|| chrono::Utc::now().naive_utc());

        let transaction = self
            .services
            .add_manual_payment_transaction(
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
                invoice_id: transaction.invoice_id.as_proto(),
                amount: rust_decimal::Decimal::from(transaction.amount).as_proto(),
                currency: transaction.currency,
                status: format!("{:?}", transaction.status),
                payment_date: transaction
                    .processed_at
                    .as_proto()
                    .unwrap_or_else(|| chrono::Utc::now().naive_utc().as_proto()),
            },
        ))
    }

    #[tracing::instrument(skip_all)]
    async fn mark_invoice_as_paid(
        &self,
        request: Request<meteroid_grpc::meteroid::api::invoices::v1::MarkInvoiceAsPaidRequest>,
    ) -> Result<
        Response<meteroid_grpc::meteroid::api::invoices::v1::MarkInvoiceAsPaidResponse>,
        Status,
    > {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let invoice_id = InvoiceId::from_proto(req.invoice_id)?;
        let total_amount = rust_decimal::Decimal::from_proto_ref(&req.total_amount)?;
        let payment_date = chrono::NaiveDateTime::from_proto_opt(req.payment_date)?
            .unwrap_or_else(|| chrono::Utc::now().naive_utc());

        let invoice = self
            .services
            .mark_invoice_as_paid(
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

fn parse_as_minor(s: &String, currency: &rusty_money::iso::Currency) -> Result<i64, Status> {
    let decimal = rust_decimal::Decimal::from_proto_ref(s)?;

    let money = rusty_money::Money::from_decimal(decimal, currency);

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
    currency: &rusty_money::iso::Currency,
) -> Result<Option<i64>, Status> {
    str_ref
        .map(|str_val| parse_as_minor(str_val, currency))
        .transpose()
}

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
                local_id: sub.id.clone(),
                name: sub.name.clone(),
                total: sub.total,
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
    let currency = rusty_money::iso::find(currency_str)
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

        // Convert sublines from proto to domain
        let sub_lines = convert_sublines_from_proto(&line.sub_line_items)?;

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

    // Calculate tax amounts after discount is applied
    for line in &mut lines {
        let taxable_amount_decimal =
            *rusty_money::Money::from_minor(line.taxable_amount, currency).amount();

        let tax_amount_decimal = taxable_amount_decimal * line.tax_rate;
        line.tax_amount = tax_amount_decimal
            .to_subunit_opt(currency.exponent as u8)
            .unwrap_or(0);
        line.amount_total = line.taxable_amount + line.tax_amount;
    }

    let subtotal: i64 = lines.iter().map(|line| line.amount_subtotal).sum();
    let total_tax_amount: i64 = lines.iter().map(|line| line.tax_amount).sum();
    let total_taxable: i64 = lines.iter().map(|line| line.taxable_amount).sum();
    let total = total_taxable + total_tax_amount;
    let amount_due = total;

    // Compute tax breakdown by grouping line items by tax rate
    let tax_breakdown = compute_tax_breakdown(&lines);

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
            snapshot_at: chrono::Utc::now().naive_utc(),
            vat_number: custom_details.vat_number,
            email: custom_details.email,
            alias: customer.alias,
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
        currency: invoice_req.currency,
        line_items: lines,
        coupons: vec![],
        data_updated_at: None,
        invoice_date,
        finalized_at: None,
        total,
        amount_due,
        subtotal,
        subtotal_recurring: 0, // no recurring items in manual invoices
        tax_amount: total_tax_amount,
        net_terms: invoicing_entity.net_terms, // todo derive from due_date?
        discount: discount.unwrap_or(0),
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
            use crate::api::customers::mapping::customer::DomainAddressWrapper;
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

    Ok(UpdateInvoiceParams {
        memo: req.memo.map(Some),
        reference: req.reference.map(Some),
        purchase_order: req.purchase_order.map(Some),
        due_date,
        line_items,
        discount,
        customer_details,
        invoicing_entity_id,
    })
}
