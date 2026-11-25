//! What can be done to an invoice through v2, each on the store service the dashboard and API v1
//! run it on, each answering the invoice as it then reads (by its id alone when it cannot be read
//! back: the action is done all the same): a draft refreshed or finalized, an issued invoice
//! voided or written off, a payment received outside Meteroid recorded or reversed. Creating and
//! editing one is `write.rs`'s.
//!
//! The payment writes also claim their own key in the store (as v1's do), so a retry the
//! idempotency middleware did not keep a response for still records one payment: they take the
//! request's `Idempotency-Key`, required where money is recorded.
use super::model::{Invoice, MarkPaid, RecordPayment, ReversePayment};
use super::read::invoice_of;
use super::write::written;
use crate::AppState;
use crate::v2::extract::{Body, Path};
use crate::v2::idempotency::IDEMPOTENCY_KEY_HEADER;
use crate::v2::ids;
use crate::v2::problem::{Problem, store_message};
use axum::Extension;
use axum::extract::State;
use axum::response::Response;
use common_domain::ids::{InvoiceId, PaymentTransactionId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::{HeaderMap, StatusCode};
use meteroid_store::domain::credit::ApiIdempotency;
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::services::{
    ManualInvoicePayment, ManualInvoicePaymentKind, ReverseManualPaymentCommand,
};

/// The store's claim for a payment write: the request's key, or (where one is not required) a
/// key of its own, so a request sent without one is never taken for another.
fn identity(
    headers: &HeaderMap,
    operation: &'static str,
    request: &impl serde::Serialize,
    required: bool,
) -> Result<ApiIdempotency, Problem> {
    let key = match headers
        .get(IDEMPOTENCY_KEY_HEADER)
        .map(|value| value.to_str())
    {
        Some(Ok(key)) => key.to_owned(),
        Some(Err(_)) => {
            return Err(Problem::field(
                IDEMPOTENCY_KEY_HEADER,
                "invalid",
                "An Idempotency-Key is visible ASCII",
            ));
        }
        None if required => {
            return Err(Problem::key_required(
                "A payment is recorded with an Idempotency-Key: a retry must never record it twice",
            ));
        }
        None => uuid::Uuid::now_v7().to_string(),
    };
    let fingerprint = meteroid_app::rest::writes::fingerprint(request)?;
    ApiIdempotency::new(operation, &key, fingerprint).map_err(|error| {
        Problem::field(
            IDEMPOTENCY_KEY_HEADER,
            "invalid",
            store_message(error.current_context()),
        )
    })
}

fn invoice_id(raw: &str) -> Result<InvoiceId, Problem> {
    ids::plain_id(raw, "invoice")
}

/// Refresh a draft
///
/// Prices the draft again against the usage, credits, coupons and tax as they are now (drafts are
/// also refreshed in the background; this forces it, after late events for instance). Refused
/// while a payment of it is in progress, or once it is merged into a consolidated invoice.
#[utoipa::path(
    post,
    tag = "Invoices",
    path = "/api/v2/invoices/{id}/refresh",
    params(("id" = String, Path, description = "Invoice id")),
    responses(
        (status = 200, description = "Refreshed", body = Invoice),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "Not a draft that can be refreshed", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn refresh_invoice(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, Problem> {
    let id = invoice_id(&id)?;
    state
        .services
        .refresh_invoice_data(id, auth.tenant_id)
        .await?;
    Ok(written(&state, auth.tenant_id, StatusCode::OK, id).await)
}

/// Finalize a draft
///
/// Issues it now: priced a last time, numbered, dated, then sent and collected as its
/// subscription or customer says. Its PDF follows shortly (`files.pdf`).
#[utoipa::path(
    post,
    tag = "Invoices",
    path = "/api/v2/invoices/{id}/finalize",
    params(("id" = String, Path, description = "Invoice id")),
    responses(
        (status = 200, description = "Issued", body = Invoice),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "Not a draft that can be issued", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn finalize_invoice(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, Problem> {
    let id = invoice_id(&id)?;
    state
        .services
        .finalize_invoice(auth.as_actor(), id, auth.tenant_id)
        .await?;
    Ok(written(&state, auth.tenant_id, StatusCode::OK, id).await)
}

/// Void an invoice
///
/// Cancels an issued invoice: nothing more is collected on it, and what it billed is given back
/// as the store voids it (credit it granted lapses).
#[utoipa::path(
    post,
    tag = "Invoices",
    path = "/api/v2/invoices/{id}/void",
    params(("id" = String, Path, description = "Invoice id")),
    responses(
        (status = 200, description = "Voided", body = Invoice),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "The invoice's state refuses it", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn void_invoice(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, Problem> {
    let id = invoice_id(&id)?;
    state
        .store
        .void_invoice(auth.as_actor(), id, auth.tenant_id)
        .await?;
    Ok(written(&state, auth.tenant_id, StatusCode::OK, id).await)
}

/// Mark an invoice uncollectible
///
/// Writes off an issued invoice and stops its reminders. Marking it again changes nothing.
#[utoipa::path(
    post,
    tag = "Invoices",
    path = "/api/v2/invoices/{id}/mark-uncollectible",
    params(("id" = String, Path, description = "Invoice id")),
    responses(
        (status = 200, description = "Written off", body = Invoice),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "Not an issued invoice", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn mark_invoice_uncollectible(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, Problem> {
    let id = invoice_id(&id)?;
    let identity = identity(&headers, "v2.invoice.mark_uncollectible", &id, false)?;
    state
        .store
        .mark_invoice_as_uncollectible_idempotent(id, auth.tenant_id, &identity)
        .await?;
    Ok(written(&state, auth.tenant_id, StatusCode::OK, id).await)
}

/// Mark an invoice paid
///
/// Records the whole amount due as received outside Meteroid (a transfer, a check): the invoice
/// is paid. A part of it is recorded with `POST …/payments`.
#[utoipa::path(
    post,
    tag = "Invoices",
    path = "/api/v2/invoices/{id}/mark-paid",
    params(("id" = String, Path, description = "Invoice id")),
    request_body = MarkPaid,
    responses(
        (status = 200, description = "Paid", body = Invoice),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "The key was used for another payment", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "Not an issued invoice with an amount due, or a date more than a day ahead", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn mark_invoice_paid(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Body(body): Body<MarkPaid>,
) -> Result<Response, Problem> {
    let invoice = invoice_of(&state, auth.tenant_id, &id).await?;
    let identity = identity(
        &headers,
        "v2.invoice.mark_paid",
        &(invoice.id, &body),
        false,
    )?;
    state
        .services
        .record_manual_invoice_payment(
            auth.as_actor(),
            auth.tenant_id,
            invoice.id,
            ManualInvoicePayment {
                amount: invoice.amount_due,
                paid_at: body
                    .paid_at
                    .map(|at| at.to_utc())
                    .unwrap_or_else(common_clock::now),
                reference: body.reference,
            },
            ManualInvoicePaymentKind::MarkPaid,
            &identity,
        )
        .await?;
    Ok(written(&state, auth.tenant_id, StatusCode::OK, invoice.id).await)
}

/// Record a payment
///
/// Records a payment received outside Meteroid, in part or in whole: paying what is due makes
/// the invoice paid. An `Idempotency-Key` is required: a retry never records it twice.
#[utoipa::path(
    post,
    tag = "Invoices",
    path = "/api/v2/invoices/{id}/payments",
    params(("id" = String, Path, description = "Invoice id")),
    request_body = RecordPayment,
    responses(
        (status = 200, description = "Recorded: the invoice with its payments", body = Invoice),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "The key was used for another payment", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "An amount or a date refused, an invoice not issued, or no Idempotency-Key", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn record_invoice_payment(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Body(body): Body<RecordPayment>,
) -> Result<Response, Problem> {
    let id = invoice_id(&id)?;
    if body.amount <= 0 {
        return Err(Problem::field(
            "/amount",
            "invalid",
            "A payment is a positive amount, in minor units",
        ));
    }
    let identity = identity(&headers, "v2.invoice.payment", &(id, &body), true)?;
    state
        .services
        .record_manual_invoice_payment(
            auth.as_actor(),
            auth.tenant_id,
            id,
            ManualInvoicePayment {
                amount: body.amount,
                paid_at: body
                    .paid_at
                    .map(|at| at.to_utc())
                    .unwrap_or_else(common_clock::now),
                reference: body.reference,
            },
            ManualInvoicePaymentKind::Record,
            &identity,
        )
        .await?;
    Ok(written(&state, auth.tenant_id, StatusCode::OK, id).await)
}

/// Reverse a payment
///
/// Records that a payment recorded by hand was clawed back in whole (returned, charged back…).
/// An `Idempotency-Key` is required.
#[utoipa::path(
    post,
    tag = "Invoices",
    path = "/api/v2/invoices/{id}/payments/{payment_id}/reverse",
    params(
        ("id" = String, Path, description = "Invoice id"),
        ("payment_id" = String, Path, description = "Payment id"),
    ),
    request_body = ReversePayment,
    responses(
        (status = 200, description = "Reversed: the invoice with its payments", body = Invoice),
        (status = 404, description = "Not found, or not a payment of the invoice", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "The key was used for another reversal", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "The payment cannot be reversed, or no Idempotency-Key", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn reverse_invoice_payment(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, payment_id)): Path<(String, String)>,
    Body(body): Body<ReversePayment>,
) -> Result<Response, Problem> {
    let id = invoice_id(&id)?;
    let payment_id: PaymentTransactionId = ids::plain_id(&payment_id, "payment")?;
    let identity = identity(
        &headers,
        "v2.invoice.payment_reversal",
        &(id, payment_id, &body),
        true,
    )?;
    state
        .services
        .reverse_invoice_payment(
            auth.as_actor(),
            auth.tenant_id,
            id,
            ReverseManualPaymentCommand {
                transaction_id: payment_id,
                kind: body.kind.into(),
                reason: body.reason,
                reversed_at: body
                    .reversed_at
                    .map(|at| at.naive_utc())
                    .unwrap_or_else(|| common_clock::now().naive_utc()),
            },
            &identity,
        )
        .await?;
    Ok(written(&state, auth.tenant_id, StatusCode::OK, id).await)
}
