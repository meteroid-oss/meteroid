use super::{InvoiceServiceComponents, mapping};
use crate::api::invoices::error::InvoiceApiError;
use crate::api::shared::conversions::ProtoConv;
use crate::api::utils::PaginationExt;
use chrono::{NaiveDate, NaiveTime};
use common_domain::ids::{CustomerId, InvoiceId, SubscriptionId, TenantId};
use common_grpc::middleware::server::auth::RequestExt;
use common_utils::decimals::ToSubunit;
use meteroid_grpc::meteroid::api::invoices::v1::{
    CreateInvoiceRequest, CreateInvoiceResponse, DeleteInvoiceRequest, DeleteInvoiceResponse,
    FinalizeInvoiceRequest, FinalizeInvoiceResponse, GetInvoiceRequest, GetInvoiceResponse,
    Invoice, ListInvoicesRequest, ListInvoicesResponse, NewInvoice, PreviewInvoiceRequest,
    PreviewInvoiceResponse, PreviewNewInvoiceRequest, PreviewNewInvoiceResponse,
    RefreshInvoiceDataRequest, RefreshInvoiceDataResponse, RequestPdfGenerationRequest,
    RequestPdfGenerationResponse, SyncToPennylaneRequest, SyncToPennylaneResponse,
    invoices_service_server::InvoicesService, list_invoices_request::SortBy,
};
use meteroid_store::Store;
use meteroid_store::domain::pgmq::{InvoicePdfRequestEvent, PgmqMessageNew, PgmqQueue};
use meteroid_store::domain::{
    InvoiceNew, InvoicingEntity, LineItem, OrderByRequest, TaxBreakdownItem,
};
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;
use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;
use meteroid_store::repositories::pgmq::PgmqInterface;
use meteroid_store::repositories::{CustomersInterface, InvoiceInterface};
use meteroid_store::utils::local_id::{IdType, LocalId};
use std::collections::HashMap;
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
                mapping::invoices::domain_invoice_with_plan_details_to_server(
                    inv,
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
    async fn preview_invoice_html(
        &self,
        request: Request<PreviewInvoiceRequest>,
    ) -> Result<Response<PreviewInvoiceResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();

        let html = self
            .preview_rendering
            .preview_invoice_by_id(InvoiceId::from_proto(&req.id)?, tenant_id)
            .await
            .map_err(Into::<InvoiceApiError>::into)?;

        let response = PreviewInvoiceResponse { html };

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
                mapping::invoices::domain_invoice_with_plan_details_to_server(
                    inv,
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
                mapping::invoices::domain_invoice_with_plan_details_to_server(
                    inv,
                    self.jwt_secret.clone(),
                )
            })
            .map_err(Into::<InvoiceApiError>::into)?;

        Ok(Response::new(CreateInvoiceResponse {
            invoice: Some(invoice),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn preview_new_invoice_html(
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

        let html = self
            .preview_rendering
            .preview_invoice(new_invoice.into(), inv_entity)
            .await
            .map_err(Into::<InvoiceApiError>::into)?;

        let response = PreviewNewInvoiceResponse { html };

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
                mapping::invoices::domain_invoice_with_plan_details_to_server(
                    inv,
                    self.jwt_secret.clone(),
                )
            })
            .map_err(Into::<InvoiceApiError>::into)?;

        Ok(Response::new(FinalizeInvoiceResponse {
            invoice: Some(finalized),
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
        let quantity = rust_decimal::Decimal::from_proto_ref(&line.quantity)?;
        let tax_rate = rust_decimal::Decimal::from_proto_ref(&line.tax_rate)?;
        let unit_price = rust_decimal::Decimal::from_proto_ref(&line.unit_price)?;
        let start_date = NaiveDate::from_proto_ref(&line.start_date)?;
        let end_date = NaiveDate::from_proto_ref(&line.end_date)?;

        let amount_subtotal = (quantity * unit_price)
            .to_subunit_opt(currency.exponent as u8)
            .unwrap_or(0);

        let item = LineItem {
            local_id: LocalId::generate_for(IdType::Other),
            name: line.product.clone(),
            tax_rate,
            amount_subtotal,
            taxable_amount: amount_subtotal, // Will be updated by distribute_discount
            tax_amount: 0,                   // Will be calculated after discount
            amount_total: 0,                 // Will be calculated after discount
            quantity: Some(quantity),
            unit_price: Some(unit_price),
            start_date,
            end_date,
            sub_lines: vec![],
            is_prorated: false,
            price_component_id: None,
            product_id: None,
            metric_id: None,
            description: None,
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
    for line in lines.iter_mut() {
        let taxable_amount_decimal =
            *rusty_money::Money::from_minor(line.taxable_amount, currency).amount();

        let tax_amount_decimal = taxable_amount_decimal * line.tax_rate;
        line.tax_amount = tax_amount_decimal
            .to_subunit_opt(currency.exponent as u8)
            .unwrap_or(0);
        line.amount_total = line.taxable_amount + line.tax_amount;
    }

    // Calculate invoice totals from line items
    let subtotal: i64 = lines.iter().map(|line| line.amount_subtotal).sum();
    let total_tax_amount: i64 = lines.iter().map(|line| line.tax_amount).sum();
    let total_taxable: i64 = lines.iter().map(|line| line.taxable_amount).sum();
    let total = total_taxable + total_tax_amount;
    let amount_due = total; // Same as total for new invoices

    // Compute tax breakdown by grouping line items by tax rate
    let tax_breakdown = compute_tax_breakdown(&lines);

    let invoicing_entity = store
        .get_invoicing_entity(tenant_id, Some(customer.invoicing_entity_id))
        .await
        .map_err(Into::<InvoiceApiError>::into)?;

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
        reference: None,
        purchase_order: invoice_req.purchase_order.clone(),
        memo: None,
        due_at: due_date.map(|d| d.and_time(NaiveTime::MIN)),
        customer_details: customer.into(),
        seller_details: invoicing_entity.clone().into(),
        auto_advance: false,
        payment_status: meteroid_store::domain::InvoicePaymentStatus::Unpaid,
        tax_breakdown,
        manual: true,
    };
    Ok((invoice_new, invoicing_entity))
}

/// Compute tax breakdown by grouping line items by tax rate
fn compute_tax_breakdown(lines: &[LineItem]) -> Vec<TaxBreakdownItem> {
    let mut tax_groups: HashMap<rust_decimal::Decimal, (u64, u64)> = HashMap::new();

    // Group line items by tax rate and aggregate amounts
    for line in lines {
        if line.tax_amount > 0 || line.taxable_amount > 0 {
            let entry = tax_groups.entry(line.tax_rate).or_insert((0, 0));
            entry.0 += line.taxable_amount as u64; // taxable_amount
            entry.1 += line.tax_amount as u64; // tax_amount
        }
    }

    // Convert groups to TaxBreakdownItem
    tax_groups
        .into_iter()
        .map(
            |(tax_rate, (taxable_amount, tax_amount))| TaxBreakdownItem {
                taxable_amount,
                tax_amount,
                tax_rate,
                name: "Tax".to_string(),
                exemption_type: None,
            },
        )
        .collect()
}
