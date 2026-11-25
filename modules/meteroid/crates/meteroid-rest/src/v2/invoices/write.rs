//! Invoices written by hand through v2 (ADR-0013 §12), on the store services the dashboard uses:
//! a one-off invoice created (`create_manual_invoice`, issued by `finalize_invoice` when asked),
//! and a draft's lines, memo, purchase order and due date edited (`update_draft_invoice`). The
//! store prices every line; v2 only names what is kept and checks what is sent at its pointer.
use super::model::{CreateInvoice, Invoice, InvoiceLineInput, NewInvoiceStatus, UpdateInvoice};
use super::read::{self, invoice_of};
use crate::AppState;
use crate::v2::extract::{Body, Path};
use crate::v2::idempotency::{IDEMPOTENCY_KEY_HEADER, committed, committed_write};
use crate::v2::ids;
use crate::v2::problem::{Problem, ProblemCode};
use axum::Extension;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use chrono::NaiveDate;
use common_domain::ids::{InvoiceId, TenantId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::{HeaderMap, StatusCode};
use meteroid_store::constants::Currencies;
use meteroid_store::domain::{
    InvoiceStatusEnum, LineItem, UpdateInvoiceParams, UpdateLineItemParams,
};
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::services::{ManualInvoice, ManualInvoiceLine};
use rust_decimal::Decimal;

fn blank(text: Option<String>) -> Option<String> {
    text.map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
}

/// A tax rate is a fraction from 0, below 1.
fn tax_rate(rate: Decimal, path: String) -> Result<Decimal, Problem> {
    if rate < Decimal::ZERO || rate >= Decimal::ONE {
        return Err(Problem::field(
            path,
            "invalid",
            "A tax rate is a fraction from 0 to below 1: 0.2 for 20%",
        ));
    }
    Ok(rate)
}

/// A line's days, `end_date` on or after `start_date`.
fn days(start: NaiveDate, end: NaiveDate, index: usize) -> Result<(NaiveDate, NaiveDate), Problem> {
    if end < start {
        return Err(Problem::field(
            format!("/lines/{index}/end_date"),
            "invalid",
            "A line ends on or after its start",
        ));
    }
    Ok((start, end))
}

/// A quantity from 0, a price from 0: a line written by hand bills, a credit note gives back.
fn amounts(line: &InvoiceLineInput, index: usize) -> Result<(), Problem> {
    if line
        .quantity
        .is_some_and(|quantity| quantity < Decimal::ZERO)
    {
        return Err(Problem::field(
            format!("/lines/{index}/quantity"),
            "invalid",
            "A quantity is 0 or more",
        ));
    }
    if line.unit_price.is_some_and(|price| price < Decimal::ZERO) {
        return Err(Problem::field(
            format!("/lines/{index}/unit_price"),
            "invalid",
            "A unit price is 0 or more: what is given back is a credit note",
        ));
    }
    Ok(())
}

/// A line not on the document yet: its name, quantity and price sent.
struct NewLine {
    name: String,
    description: Option<String>,
    quantity: Decimal,
    unit_price: Decimal,
    tax_rate: Decimal,
    start_date: NaiveDate,
    end_date: NaiveDate,
}

fn new_line(line: &InvoiceLineInput, index: usize, on: NaiveDate) -> Result<NewLine, Problem> {
    let at = |field: &str| format!("/lines/{index}/{field}");
    let name = blank(line.name.clone())
        .ok_or_else(|| Problem::field(at("name"), "required", "A line has a name"))?;
    let quantity = line
        .quantity
        .ok_or_else(|| Problem::field(at("quantity"), "required", "A new line has a quantity"))?;
    let unit_price = line.unit_price.ok_or_else(|| {
        Problem::field(at("unit_price"), "required", "A new line has a unit price")
    })?;
    amounts(line, index)?;
    let start = line.start_date.unwrap_or(on);
    let (start_date, end_date) = days(start, line.end_date.unwrap_or(start), index)?;
    Ok(NewLine {
        name,
        description: blank(line.description.clone()),
        quantity,
        unit_price,
        tax_rate: tax_rate(line.tax_rate.unwrap_or_default(), at("tax_rate"))?,
        start_date,
        end_date,
    })
}

/// The draft's lines as asked, for the store's edit: a line named by its `id` kept with what is
/// sent laid over what it holds (on a tax-inclusive draft, the figures it was quoted in, as the
/// store reads a line it keeps), a line without one added.
fn draft_lines(
    asked: &[InvoiceLineInput],
    stored: &[LineItem],
    tax_inclusive: bool,
    on: NaiveDate,
) -> Result<Vec<UpdateLineItemParams>, Problem> {
    asked
        .iter()
        .enumerate()
        .map(|(index, line)| {
            let at = |field: &str| format!("/lines/{index}/{field}");
            let Some(id) = &line.id else {
                let new = new_line(line, index, on)?;
                return Ok(UpdateLineItemParams {
                    id: None,
                    name: new.name,
                    start_date: new.start_date,
                    end_date: new.end_date,
                    quantity: Some(new.quantity),
                    unit_price: Some(new.unit_price),
                    tax_rate: new.tax_rate,
                    description: new.description,
                    sub_lines: Vec::new(),
                });
            };
            let kept = stored
                .iter()
                .find(|candidate| candidate.local_id == *id)
                .ok_or_else(|| {
                    Problem::field(
                        at("id"),
                        "not_found",
                        format!("The draft has no line '{id}'"),
                    )
                })?;
            let kept = if tax_inclusive {
                kept.clone().into_tax_inclusive_quote()
            } else {
                kept.clone()
            };
            amounts(line, index)?;
            let priced_by_sub_lines = !kept.sub_lines.is_empty();
            if priced_by_sub_lines && (line.quantity.is_some() || line.unit_price.is_some()) {
                return Err(Problem::field(
                    at("quantity"),
                    "not_expressible",
                    "The line is priced by its sub-lines (tiers, packages): it keeps its quantity \
                     and price, or is removed",
                ));
            }
            let (start_date, end_date) = days(
                line.start_date.unwrap_or(kept.start_date),
                line.end_date.unwrap_or(kept.end_date),
                index,
            )?;
            Ok(UpdateLineItemParams {
                id: Some(kept.local_id.clone()),
                name: match &line.name {
                    Some(_) => blank(line.name.clone()).ok_or_else(|| {
                        Problem::field(at("name"), "required", "A line has a name")
                    })?,
                    None => kept.name.clone(),
                },
                start_date,
                end_date,
                quantity: if priced_by_sub_lines {
                    None
                } else {
                    line.quantity.or(kept.quantity)
                },
                unit_price: if priced_by_sub_lines {
                    None
                } else {
                    line.unit_price.or(kept.unit_price)
                },
                tax_rate: match line.tax_rate {
                    Some(rate) => tax_rate(rate, at("tax_rate"))?,
                    None => kept.tax_rate,
                },
                description: match &line.description {
                    Some(_) => blank(line.description.clone()),
                    None => kept.description.clone(),
                },
                sub_lines: kept.sub_lines.clone(),
            })
        })
        .collect()
}

/// The invoice written, as it reads; by its id alone when v2 cannot read it back (the write is
/// made all the same: a write committed never answers an error).
pub(super) async fn written(
    state: &AppState,
    tenant_id: TenantId,
    status: StatusCode,
    id: InvoiceId,
) -> Response {
    committed_write(
        status,
        read::invoice(state, tenant_id, id).await,
        || serde_json::json!({"id": id.to_string()}),
    )
}

/// Create a one-off invoice
///
/// An invoice written by hand to a customer, beside any subscription: its lines priced as
/// `quantity × unit_price`, each taxed at its `tax_rate`, totalled by the store as the dashboard's
/// editor prices them. A draft unless `status: finalized`, which issues it at once. An
/// `Idempotency-Key` is required: a retry never bills twice.
#[utoipa::path(
    post,
    tag = "Invoices",
    path = "/api/v2/invoices",
    request_body = CreateInvoice,
    responses(
        (status = 201, description = "Created", body = Invoice),
        (status = 409, description = "Created as a draft, but not issued: see `detail`", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused, or no Idempotency-Key", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn create_invoice(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Body(body): Body<CreateInvoice>,
) -> Result<Response, Problem> {
    if !headers.contains_key(IDEMPOTENCY_KEY_HEADER) {
        return Err(Problem::key_required(
            "An invoice is created with an Idempotency-Key: a retry must never bill twice",
        ));
    }
    let tenant_id = auth.tenant_id;
    let invoice_date = body
        .invoice_date
        .unwrap_or_else(|| common_clock::now().date_naive());
    if body
        .due_date
        .is_some_and(|due_date| due_date < invoice_date)
    {
        return Err(Problem::field(
            "/due_date",
            "invalid",
            "An invoice is due on or after its date",
        ));
    }
    if body.lines.is_empty() {
        return Err(Problem::field(
            "/lines",
            "required",
            "An invoice bills a line at least",
        ));
    }
    let mut lines = Vec::with_capacity(body.lines.len());
    for (index, line) in body.lines.iter().enumerate() {
        if line.id.is_some() {
            return Err(Problem::field(
                format!("/lines/{index}/id"),
                "invalid",
                "A new invoice's lines have no id",
            ));
        }
        let new = new_line(line, index, invoice_date)?;
        lines.push(ManualInvoiceLine {
            name: new.name,
            description: new.description,
            quantity: Some(new.quantity),
            unit_price: Some(new.unit_price),
            tax_rate: new.tax_rate,
            start_date: new.start_date,
            end_date: new.end_date,
            sub_lines: Vec::new(),
        });
    }
    let customer = ids::customer(&state, tenant_id, &body.customer)
        .await
        .map_err(|problem| {
            if problem.status == 404 {
                Problem::field(
                    "/customer",
                    "not_found",
                    format!("customer {} not found", body.customer.trim()),
                )
            } else {
                problem
            }
        })?;
    let currency = match body.currency.as_deref().map(str::trim) {
        Some(currency) => {
            let currency = currency.to_ascii_uppercase();
            if Currencies::resolve_currency_precision(&currency).is_none() {
                return Err(Problem::field(
                    "/currency",
                    "invalid",
                    "An ISO 4217 currency code, like EUR",
                ));
            }
            currency
        }
        None => customer.currency.clone(),
    };

    let created = state
        .services
        .create_manual_invoice(
            tenant_id,
            ManualInvoice {
                customer_id: customer.id,
                currency,
                invoice_date,
                due_date: body.due_date,
                lines,
                discount: None,
                tax_inclusive: false,
                reference: None,
                purchase_order: blank(body.purchase_order.clone()),
                memo: blank(body.memo.clone()),
                customer_details: None,
                annexes: Vec::new(),
                custom_properties: body
                    .custom_properties
                    .clone()
                    .unwrap_or(serde_json::Value::Null),
            },
        )
        .await?;
    // Written: from here the request answers what it did, whatever is read back.
    if body.status == NewInvoiceStatus::Finalized
        && let Err(error) = state
            .services
            .finalize_invoice(auth.as_actor(), created.id, tenant_id)
            .await
    {
        let refused = Problem::from(error);
        let mut problem = Problem::conflict(
            ProblemCode::Conflict,
            format!(
                "Invoice {} was created as a draft but not issued: {}. Issue it with POST \
                 /api/v2/invoices/{}/finalize",
                created.id,
                refused.detail.clone().unwrap_or_default(),
                created.id,
            ),
        );
        problem.errors = refused.errors;
        return Ok(committed(problem.into_response()));
    }
    Ok(written(&state, tenant_id, StatusCode::CREATED, created.id).await)
}

/// Update an invoice
///
/// A JSON merge patch. Its custom properties change at any status: they are the integrator's own
/// workflow data (an export, a filing), never the invoice's figures. A draft's lines, memo,
/// purchase order and due date change as the dashboard's editor changes them: `lines` is the
/// draft's whole list, each kept line by its `id`, priced again as `quantity × unit_price`; the
/// draft then finalizes from these lines, no longer priced again from the usage. Refused on an
/// issued invoice, while a payment of the draft is in progress, and on a draft billing usage in
/// credits.
#[utoipa::path(
    patch,
    tag = "Invoices",
    path = "/api/v2/invoices/{id}",
    params(("id" = String, Path, description = "Invoice id")),
    request_body(content = UpdateInvoice, content_type = "application/merge-patch+json"),
    responses(
        (status = 200, description = "Updated", body = Invoice),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "Not a draft, for what only a draft changes", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn update_invoice(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<UpdateInvoice>,
) -> Result<Response, Problem> {
    let tenant_id = auth.tenant_id;
    let invoice = invoice_of(&state, tenant_id, &id).await?;
    if !body.edits_draft() {
        if let Some(properties) = body.custom_properties {
            state
                .store
                .patch_invoice_custom_properties(tenant_id, invoice.id, properties)
                .await
                .map_err(|error| Problem::at("/custom_properties", error))?;
        }
        return Ok(written(&state, tenant_id, StatusCode::OK, invoice.id).await);
    }
    if invoice.status != InvoiceStatusEnum::Draft {
        return Err(Problem::conflict(
            ProblemCode::Conflict,
            "Only a draft's lines, memo, purchase order and due date change: an issued invoice \
             is corrected by a credit note (POST /api/v2/credit-notes)",
        ));
    }
    if let Some(Some(due_date)) = body.due_date
        && due_date < invoice.invoice_date
    {
        return Err(Problem::field(
            "/due_date",
            "invalid",
            "An invoice is due on or after its date",
        ));
    }
    let line_items = match &body.lines {
        Some(lines) => {
            if lines.is_empty() {
                return Err(Problem::field(
                    "/lines",
                    "required",
                    "A draft bills a line at least",
                ));
            }
            Some(draft_lines(
                lines,
                &invoice.line_items,
                invoice.tax_inclusive,
                invoice.invoice_date,
            )?)
        }
        None => None,
    };
    let params = UpdateInvoiceParams {
        memo: body.memo.map(blank),
        reference: None,
        purchase_order: body.purchase_order.map(blank),
        due_date: body.due_date,
        line_items,
        discount: None,
        customer_details: None,
        invoicing_entity_id: None,
        annexes: None,
        custom_properties: body.custom_properties,
    };
    state
        .services
        .update_draft_invoice(invoice.id, tenant_id, params)
        .await?;
    Ok(written(&state, tenant_id, StatusCode::OK, invoice.id).await)
}
