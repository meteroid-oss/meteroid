use super::mapping::map_transaction_to_rest;
use super::model::{RecordInvoicePaymentRequest, ReverseInvoicePaymentRequest, Transaction};
use crate::AppState;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::{Extension, Json};
use common_domain::ids::{InvoiceId, PaymentTransactionId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use meteroid_app::errors::RestApiError;
use meteroid_app::rest::writes::{fingerprint, idempotency_key, store_error};
use meteroid_rest_common::error::RestErrorResponse;
use meteroid_store::domain::credit::ApiIdempotency;
use meteroid_store::services::{
    ManualInvoicePayment, ManualInvoicePaymentKind, ReverseManualPaymentCommand,
};

/// Record an invoice payment
///
/// Record a partial or full payment received outside Meteroid against a finalized invoice.
/// Paying the remaining balance marks the invoice paid and emits an `invoice.paid` webhook.
/// Retrying with the same key and request returns the same transaction without recording another payment.
#[utoipa::path(
    post,
    tag = "Invoices",
    path = "/api/v1/invoices/{invoice_id}/payments",
    params(
        ("invoice_id" = InvoiceId, Path, description = "Invoice ID"),
        ("Idempotency-Key" = String, Header, description = "Required. Reuse it only to retry the same request"),
    ),
    request_body = RecordInvoicePaymentRequest,
    responses(
        (status = 201, description = "Recorded payment", body = Transaction),
        (status = 400, description = "Invalid request or invoice cannot accept the payment", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 403, description = "Forbidden", body = RestErrorResponse),
        (status = 404, description = "Invoice not found", body = RestErrorResponse),
        (status = 409, description = "Idempotency-Key reused with another request", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
pub async fn record_invoice_payment(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(invoice_id): Path<InvoiceId>,
    headers: HeaderMap,
    Json(request): Json<RecordInvoicePaymentRequest>,
) -> Result<(StatusCode, Json<Transaction>), RestApiError> {
    record_payment(
        &state,
        &auth,
        invoice_id,
        &headers,
        request,
        ManualInvoicePaymentKind::Record,
    )
    .await
}

/// Mark an invoice as paid
///
/// Record an external payment for the entire remaining balance of a finalized invoice and emit an
/// `invoice.paid` webhook. The amount must match the remaining balance.
/// Retrying with the same key and request returns the same transaction without recording another payment.
#[utoipa::path(
    post,
    tag = "Invoices",
    path = "/api/v1/invoices/{invoice_id}/mark-paid",
    params(
        ("invoice_id" = InvoiceId, Path, description = "Invoice ID"),
        ("Idempotency-Key" = String, Header, description = "Required. Reuse it only to retry the same request"),
    ),
    request_body = RecordInvoicePaymentRequest,
    responses(
        (status = 201, description = "Recorded payment of the remaining balance", body = Transaction),
        (status = 400, description = "Invalid request or invoice cannot be marked paid", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 403, description = "Forbidden", body = RestErrorResponse),
        (status = 404, description = "Invoice not found", body = RestErrorResponse),
        (status = 409, description = "Idempotency-Key reused with another request", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
pub async fn mark_invoice_paid(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(invoice_id): Path<InvoiceId>,
    headers: HeaderMap,
    Json(request): Json<RecordInvoicePaymentRequest>,
) -> Result<(StatusCode, Json<Transaction>), RestApiError> {
    record_payment(
        &state,
        &auth,
        invoice_id,
        &headers,
        request,
        ManualInvoicePaymentKind::MarkPaid,
    )
    .await
}

async fn record_payment(
    state: &AppState,
    auth: &AuthorizedAsTenant,
    invoice_id: InvoiceId,
    headers: &HeaderMap,
    request: RecordInvoicePaymentRequest,
    kind: ManualInvoicePaymentKind,
) -> Result<(StatusCode, Json<Transaction>), RestApiError> {
    let operation = match kind {
        ManualInvoicePaymentKind::Record => "invoice_payment.record",
        ManualInvoicePaymentKind::MarkPaid => "invoice.mark_paid",
    };
    let identity = ApiIdempotency::new(
        operation,
        idempotency_key(headers)?,
        fingerprint(&(invoice_id, &request))?,
    )?;
    let payment = state
        .services
        .record_manual_invoice_payment(
            auth.as_actor(),
            auth.tenant_id,
            invoice_id,
            ManualInvoicePayment {
                amount: request.amount,
                payment_date: request
                    .payment_date
                    .unwrap_or_else(|| common_domain::clock::now().fixed_offset()),
                reference: request.reference,
            },
            kind,
            &identity,
        )
        .await
        .map_err(store_error)?;
    Ok((StatusCode::CREATED, Json(map_transaction_to_rest(payment))))
}

/// Reverse an invoice payment
///
/// Record the full reversal of a settled manual payment belonging to this invoice. A failed
/// collection restores the amount due. A dispute or chargeback leaves the invoice paid and
/// records the loss. A reversal emits the `payment.reversed` webhook.
/// Retrying with the same key and request returns the transaction without applying the reversal again.
#[utoipa::path(
    post,
    tag = "Invoices",
    path = "/api/v1/invoices/{invoice_id}/payments/{transaction_id}/reverse",
    params(
        ("invoice_id" = InvoiceId, Path, description = "Invoice ID"),
        ("transaction_id" = PaymentTransactionId, Path, description = "Payment transaction ID"),
        ("Idempotency-Key" = String, Header, description = "Required. Reuse it only to retry the same request"),
    ),
    request_body = ReverseInvoicePaymentRequest,
    responses(
        (status = 200, description = "Reversed payment", body = Transaction),
        (status = 400, description = "Invalid request or payment cannot be reversed", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 403, description = "Forbidden", body = RestErrorResponse),
        (status = 404, description = "Payment not found on the invoice", body = RestErrorResponse),
        (status = 409, description = "Idempotency-Key reused with another request", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
pub async fn reverse_invoice_payment(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path((invoice_id, transaction_id)): Path<(InvoiceId, PaymentTransactionId)>,
    headers: HeaderMap,
    Json(request): Json<ReverseInvoicePaymentRequest>,
) -> Result<Json<Transaction>, RestApiError> {
    // ApiDateTime serializes to milliseconds, while a reversal preserves microseconds.
    let reversed_at = request.reversed_at.map(|at| at.naive_utc());
    let identity = ApiIdempotency::new(
        "invoice_payment.reverse",
        idempotency_key(&headers)?,
        fingerprint(&(
            invoice_id,
            transaction_id,
            &request.kind,
            &request.reason,
            reversed_at,
        ))?,
    )?;
    let payment = state
        .services
        .reverse_invoice_payment(
            auth.as_actor(),
            auth.tenant_id,
            invoice_id,
            ReverseManualPaymentCommand {
                transaction_id,
                kind: request.kind.into(),
                reason: request.reason,
                reversed_at: reversed_at.unwrap_or_else(|| common_domain::clock::now().naive_utc()),
            },
            &identity,
        )
        .await
        .map_err(store_error)?;
    Ok(Json(map_transaction_to_rest(payment)))
}

/// Mark an invoice as uncollectible
///
/// Transition a finalized invoice to uncollectible and stop its payment reminders.
#[utoipa::path(
    post,
    tag = "Invoices",
    path = "/api/v1/invoices/{invoice_id}/mark-uncollectible",
    params(
        ("invoice_id" = InvoiceId, Path, description = "Invoice ID"),
        ("Idempotency-Key" = String, Header, description = "Required. Reuse it only to retry the same request"),
    ),
    responses(
        (status = 204, description = "Invoice marked uncollectible"),
        (status = 400, description = "Invalid request or invoice cannot be marked uncollectible", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 403, description = "Forbidden", body = RestErrorResponse),
        (status = 404, description = "Invoice not found", body = RestErrorResponse),
        (status = 409, description = "Idempotency-Key reused with another request", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
pub async fn mark_invoice_uncollectible(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(invoice_id): Path<InvoiceId>,
    headers: HeaderMap,
) -> Result<StatusCode, RestApiError> {
    let identity = ApiIdempotency::new(
        "invoice.mark_uncollectible",
        idempotency_key(&headers)?,
        fingerprint(&invoice_id)?,
    )?;
    state
        .store
        .mark_invoice_as_uncollectible_idempotent(invoice_id, auth.tenant_id, &identity)
        .await
        .map_err(store_error)?;
    Ok(StatusCode::NO_CONTENT)
}
