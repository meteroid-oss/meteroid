//! `GET /api/v2/invoices`, paged by keyset in SQL (invoices are an unbounded collection, ADR-0013
//! §3), `GET /api/v2/invoices/{id}` with its lines, payments and credit notes, and its files.
use super::model::{Invoice, InvoiceList, InvoiceListQuery, InvoicePayment};
use crate::AppState;
use crate::v2::documents::{self, ProductCodes, document_not_found, filter_not_found, moment};
use crate::v2::extract::{Path, Query};
use crate::v2::ids;
use crate::v2::pagination::{Cursor, PageQuery};
use crate::v2::problem::Problem;
use crate::v2::refs::{CreditNoteRef, ReferenceIds, Refs};
use axum::Extension;
use axum::Json;
use axum::extract::State;
use axum::response::Response;
use common_domain::ids::{BaseId, InvoiceId, TenantId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use meteroid_store::domain::{Invoice as DomainInvoice, PaymentTransaction};
use meteroid_store::repositories::document_pages::InvoicePageFilter;
use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;
use meteroid_store::repositories::{CreditNoteInterface, DocumentPagesInterface, InvoiceInterface};

// ── Rendering ───────────────────────────────────────────────────────

/// When the invoice last changed: its `updated_at`, which a trigger sets on every edit of the
/// row (a refresh of a draft's lines that moved none is not one), else its creation.
fn updated_at(invoice: &DomainInvoice) -> chrono::NaiveDateTime {
    invoice
        .updated_at
        .map_or(invoice.created_at, |at| at.max(invoice.created_at))
}

/// What invoices name, to read in one round: `corrected_by` the invoices reissued in their place.
fn reference_ids<'a>(
    invoices: impl IntoIterator<Item = &'a DomainInvoice>,
    corrected_by: impl IntoIterator<Item = InvoiceId>,
) -> ReferenceIds {
    let mut ids = ReferenceIds::default();
    for invoice in invoices {
        ids.customers.push(invoice.customer_id);
        ids.subscriptions.extend(invoice.subscription_id);
        ids.invoices.extend(invoice.parent_invoice_id);
        ids.invoices.extend(invoice.consolidated_into_invoice_id);
    }
    ids.invoices.extend(corrected_by);
    ids
}

/// What the invoice's own row reads as, what it names read in `refs`.
pub fn summary(invoice: &DomainInvoice, corrected_by: Option<InvoiceId>, refs: &Refs) -> Invoice {
    let details = &invoice.customer_details;
    Invoice {
        id: invoice.id,
        number: invoice.invoice_number.clone(),
        status: invoice.status.into(),
        kind: invoice.invoice_type.clone().into(),
        customer: refs.customer_or(invoice.customer_id, details.alias.as_deref(), &details.name),
        customer_details: documents::customer(details),
        subscription: invoice.subscription_id.map(|id| refs.subscription(id)),
        currency: invoice.currency.clone(),
        invoice_date: invoice.invoice_date,
        period_start: invoice.billing_period_start,
        due_date: invoice.due_at.map(|at| at.date()),
        net_terms: invoice.net_terms,
        tax_inclusive: invoice.tax_inclusive,
        subtotal: invoice.subtotal,
        discount: invoice.discount,
        promotional_credits: invoice.promotional_credits,
        tax: invoice.tax_amount,
        total: invoice.total,
        applied_credits: invoice.applied_credits,
        carried_balance: invoice.carried_balance,
        amount_due: invoice.amount_due,
        amount_paid: invoice.collected(),
        payment_status: invoice.payment_status.into(),
        paid_at: invoice.paid_at.map(Into::into),
        lines: None,
        taxes: documents::taxes(&invoice.tax_breakdown),
        coupons: documents::coupons(&invoice.coupons),
        payments: None,
        credit_notes: None,
        files: documents::files(invoice.pdf_document_id, invoice.xml_document_id),
        einvoicing_status: invoice.einvoicing_status.map(Into::into),
        corrects: invoice.parent_invoice_id.map(|id| refs.invoice(id)),
        corrected_by: corrected_by.map(|id| refs.invoice(id)),
        consolidated_into: invoice
            .consolidated_into_invoice_id
            .map(|id| refs.invoice(id)),
        memo: invoice.memo.clone(),
        reference: invoice.reference.clone(),
        purchase_order: invoice.purchase_order.clone(),
        custom_properties: invoice.custom_properties.clone(),
        created_at: invoice.created_at.into(),
        updated_at: updated_at(invoice).into(),
        finalized_at: invoice.finalized_at.map(Into::into),
        voided_at: invoice.voided_at.map(Into::into),
        marked_uncollectible_at: invoice.marked_as_uncollectible_at.map(Into::into),
    }
}

fn payment(tx: PaymentTransaction, refs: &Refs) -> InvoicePayment {
    InvoicePayment {
        id: tx.id,
        kind: tx.payment_type.into(),
        status: tx.status.into(),
        amount: tx.amount,
        currency: tx.currency,
        processed_at: tx.processed_at.map(Into::into),
        reference: tx.provider_transaction_id,
        amount_refunded: tx.amount_refunded,
        amount_reversed: tx.amount_reversed,
        amount_disputed: tx.amount_disputed,
        reversal_kind: tx.reversal_kind.map(Into::into),
        reversal_reason: tx.reversal_reason,
        refunds_payment: tx.parent_transaction_id,
        credit_note: tx.credit_note_id.map(|id| refs.credit_note(id)),
        error: tx.error_type,
    }
}

/// The invoice, read whole: a draft with the dates finalizing it today would stamp.
pub async fn invoice(
    state: &AppState,
    tenant_id: TenantId,
    id: InvoiceId,
) -> Result<Invoice, Problem> {
    let mut invoice = state
        .store
        .get_detailed_invoice_by_id(tenant_id, id)
        .await
        .map_err(|error| document_not_found("invoice", id)(Problem::from(error)))?
        .invoice;
    state
        .store
        .project_draft_emission_dates(&mut invoice)
        .await?;
    let mut payments: Vec<PaymentTransaction> = state
        .store
        .list_payment_tx_by_invoice_id(tenant_id, id)
        .await?
        .into_iter()
        .map(|tx| tx.transaction)
        .collect();
    payments.sort_by_key(|tx| (tx.processed_at, tx.id.as_uuid()));
    let corrected_by = state.store.find_child_invoice_id(tenant_id, id).await?;
    let mut credit_notes: Vec<CreditNoteRef> = state
        .store
        .list_credit_notes_by_invoice_id(tenant_id, id)
        .await?
        .into_iter()
        .map(|note| CreditNoteRef {
            id: note.id,
            number: note.credit_note_number,
        })
        .collect();
    credit_notes.sort_by_key(|note| note.id.as_uuid());
    let products = ProductCodes::load(state, tenant_id, &invoice.line_items).await?;
    let mut ids = reference_ids([&invoice], corrected_by);
    ids.credit_notes
        .extend(payments.iter().filter_map(|tx| tx.credit_note_id));
    let refs = Refs::load(&state.store, tenant_id, &ids).await?;

    let mut out = summary(&invoice, corrected_by, &refs);
    out.lines = Some(documents::lines(&invoice.line_items, &products));
    out.payments = Some(payments.into_iter().map(|tx| payment(tx, &refs)).collect());
    out.credit_notes = Some(credit_notes);
    Ok(out)
}

/// The invoice a path names, checked to be the tenant's.
pub async fn invoice_of(
    state: &AppState,
    tenant_id: TenantId,
    raw: &str,
) -> Result<DomainInvoice, Problem> {
    let id: InvoiceId = ids::plain_id(raw, "invoice")?;
    state
        .store
        .get_invoice_by_id(tenant_id, id)
        .await
        .map_err(|error| document_not_found("invoice", id)(Problem::from(error)))
}

// ── Handlers ────────────────────────────────────────────────────────

/// List invoices
///
/// The tenant's invoices in creation order, paged by cursor, without what is read on a single
/// one (lines, payments, credit notes). An invoice merged into a consolidated one is listed under
/// its subscription only.
#[utoipa::path(
    get,
    tag = "Invoices",
    path = "/api/v2/invoices",
    params(InvoiceListQuery),
    responses(
        (status = 200, description = "A page of invoices", body = InvoiceList),
        (status = 401, description = "Unauthorized", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A filter is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_invoices(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Query(query): Query<InvoiceListQuery>,
) -> Result<Json<InvoiceList>, Problem> {
    let tenant_id = auth.tenant_id;
    let page = PageQuery {
        limit: query.limit,
        after: query.after.clone(),
    };
    let limit = page.limit()?;
    let cursor = page.cursor()?;
    let customer_id = match query.customer.as_deref() {
        Some(raw) => Some(
            ids::customer(&state, tenant_id, raw)
                .await
                .map_err(filter_not_found("customer", raw))?
                .id,
        ),
        None => None,
    };
    let subscription_id = match query.subscription.as_deref() {
        Some(raw) => Some(
            ids::subscription_id(&state, tenant_id, raw)
                .await
                .map_err(filter_not_found("subscription", raw))?,
        ),
        None => None,
    };
    let issued_after = query
        .issued_after
        .as_deref()
        .map(|raw| moment(raw, "issued_after"))
        .transpose()?;
    let updated_after = query
        .updated_after
        .as_deref()
        .map(|raw| moment(raw, "updated_after"))
        .transpose()?;
    let mut items = state
        .store
        .list_invoices_after(
            tenant_id,
            InvoicePageFilter {
                customer_id,
                subscription_id,
                statuses: query.status.map(|status| vec![status.into()]),
                payment_statuses: query.payment_status.map(|status| vec![status.into()]),
                currency: query
                    .currency
                    .as_deref()
                    .map(|currency| currency.trim().to_uppercase()),
                issued_after: issued_after.map(|at| at.naive_utc()),
                updated_after: updated_after.map(|at| at.naive_utc()),
            },
            cursor.map(|Cursor(after)| InvoiceId::from(after)),
            limit + 1,
        )
        .await?;
    let has_more = items.len() > limit;
    items.truncate(limit);
    let next_cursor = if has_more {
        items
            .last()
            .map(|item| Cursor(item.invoice.id.as_uuid()).encode())
    } else {
        None
    };
    let ids: Vec<InvoiceId> = items.iter().map(|item| item.invoice.id).collect();
    let corrected = state.store.find_child_invoice_ids(tenant_id, &ids).await?;
    let refs = Refs::load(
        &state.store,
        tenant_id,
        &reference_ids(
            items.iter().map(|item| &item.invoice),
            corrected.values().copied(),
        ),
    )
    .await?;
    let data = items
        .iter()
        .map(|item| {
            summary(
                &item.invoice,
                corrected.get(&item.invoice.id).copied(),
                &refs,
            )
        })
        .collect();
    Ok(Json(InvoiceList {
        data,
        next_cursor,
        has_more,
    }))
}

/// Get an invoice
///
/// With its lines as the engine wrote them (tiers, packages, dimension rates and levels as
/// sublines), its taxes, coupons, payments and credit notes. A draft reads as it would be issued
/// today.
#[utoipa::path(
    get,
    tag = "Invoices",
    path = "/api/v2/invoices/{id}",
    params(("id" = String, Path, description = "Invoice id")),
    responses(
        (status = 200, description = "The invoice", body = Invoice),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_invoice(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Invoice>, Problem> {
    let id: InvoiceId = ids::plain_id(&id, "invoice")?;
    Ok(Json(invoice(&state, auth.tenant_id, id).await?))
}

/// Download an invoice's PDF
///
/// `404` until it is generated, shortly after the invoice is issued: see `files.pdf`.
#[utoipa::path(
    get,
    tag = "Invoices",
    path = "/api/v2/invoices/{id}/pdf",
    params(("id" = String, Path, description = "Invoice id")),
    responses(
        (status = 200, description = "The PDF", content_type = "application/pdf", body = inline(crate::invoices::model::BinaryFile)),
        (status = 404, description = "Not found, or no PDF yet", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn download_invoice_pdf(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, Problem> {
    let invoice = invoice_of(&state, auth.tenant_id, &id).await?;
    documents::pdf_response(&state, invoice.pdf_document_id).await
}

/// Download an invoice's e-invoice
///
/// The structured e-invoice (EN 16931 XML) issued with it; for Factur-X, the same XML is in the
/// PDF. `404` when e-invoicing is off or the invoice could not be issued as one: see
/// `einvoicing_status`.
#[utoipa::path(
    get,
    tag = "Invoices",
    path = "/api/v2/invoices/{id}/xml",
    params(("id" = String, Path, description = "Invoice id")),
    responses(
        (status = 200, description = "The e-invoice", content_type = "application/xml", body = inline(crate::invoices::model::BinaryFile)),
        (status = 404, description = "Not found, or not issued as an e-invoice", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn download_invoice_xml(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, Problem> {
    let invoice = invoice_of(&state, auth.tenant_id, &id).await?;
    documents::xml_response(&state, invoice.xml_document_id, &invoice.invoice_number).await
}
