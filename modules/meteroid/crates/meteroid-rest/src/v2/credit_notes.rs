//! `/api/v2/credit-notes` (ADR-0013 §12): credit notes listed by keyset in SQL, read with their
//! lines as invoices render theirs (`v2::documents`), their files and custom properties, and issued
//! against an invoice by an operator on the store write the dashboard uses
//! (`create_operator_credit_note`). A subscription change issues its own (`ChangeResult`).
use crate::AppState;
use crate::v2::documents::{
    self, DocumentCustomer, DocumentFiles, DocumentLine, DocumentTax, EInvoicingStatus,
    ProductCodes, document_not_found, filter_not_found,
};
use crate::v2::extract::{Body, Path, Query};
use crate::v2::idempotency::{IDEMPOTENCY_KEY_HEADER, committed_write};
use crate::v2::ids;
use crate::v2::pagination::{Cursor, PageQuery};
use crate::v2::problem::{Problem, ProblemCode};
use crate::v2::refs::{CustomerRef, InvoiceRef, ReferenceIds, Refs, SubscriptionRef};
use axum::Extension;
use axum::Json;
use axum::extract::State;
use axum::response::Response;
use common_domain::ids::{BaseId, CreditNoteId, InvoiceId, TenantId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::{HeaderMap, StatusCode};
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_store::domain::CreditNote as DomainCreditNote;
use meteroid_store::domain::InvoiceStatusEnum;
use meteroid_store::domain::enums::{
    CreditNoteOrigin as DomainOrigin, CreditNoteStatus as DomainStatus, CreditType,
    OperatorNoteKind,
};
use meteroid_store::repositories::credit_notes::{
    CreateCreditNoteParams, CreditLineItem, CreditSubLineItem,
};
use meteroid_store::repositories::document_pages::CreditNotePageFilter;
use meteroid_store::repositories::{CreditNoteInterface, DocumentPagesInterface, InvoiceInterface};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_credit_notes, create_credit_note))
        .routes(routes!(get_credit_note, update_credit_note))
        .routes(routes!(download_credit_note_pdf))
        .routes(routes!(download_credit_note_xml))
}

// ── Model ───────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CreditNoteStatus {
    Draft,
    Finalized,
    Voided,
}

impl From<DomainStatus> for CreditNoteStatus {
    fn from(status: DomainStatus) -> Self {
        match status {
            DomainStatus::Draft => CreditNoteStatus::Draft,
            DomainStatus::Finalized => CreditNoteStatus::Finalized,
            DomainStatus::Voided => CreditNoteStatus::Voided,
        }
    }
}

impl From<CreditNoteStatus> for DomainStatus {
    fn from(status: CreditNoteStatus) -> Self {
        match status {
            CreditNoteStatus::Draft => DomainStatus::Draft,
            CreditNoteStatus::Finalized => DomainStatus::Finalized,
            CreditNoteStatus::Voided => DomainStatus::Voided,
        }
    }
}

/// How the credited amount goes back to the customer.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CreditNoteType {
    /// To the customer's credit balance, drawn on later invoices.
    #[default]
    CreditToBalance,
    /// Refunded in money.
    Refund,
    /// What the invoice still owed, cancelled.
    DebtCancellation,
}

impl From<CreditType> for CreditNoteType {
    fn from(kind: CreditType) -> Self {
        match kind {
            CreditType::CreditToBalance => CreditNoteType::CreditToBalance,
            CreditType::Refund => CreditNoteType::Refund,
            CreditType::DebtCancellation => CreditNoteType::DebtCancellation,
        }
    }
}

impl From<CreditNoteType> for CreditType {
    fn from(kind: CreditNoteType) -> Self {
        match kind {
            CreditNoteType::CreditToBalance => CreditType::CreditToBalance,
            CreditNoteType::Refund => CreditType::Refund,
            CreditNoteType::DebtCancellation => CreditType::DebtCancellation,
        }
    }
}

/// Who issued it: an operator, or the engine for a change it settled.
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CreditNoteOrigin {
    Manual,
    Amendment,
    PlanChange,
    Cancellation,
    /// A credit pack billed late, given back.
    LatePack,
    /// A scheduled change that did not apply, given back.
    UnappliedChange,
    /// Its invoice voided.
    Void,
    /// An operator correcting what was billed.
    Correction,
}

impl From<DomainOrigin> for CreditNoteOrigin {
    fn from(origin: DomainOrigin) -> Self {
        match origin {
            DomainOrigin::Manual => CreditNoteOrigin::Manual,
            DomainOrigin::Amendment => CreditNoteOrigin::Amendment,
            DomainOrigin::PlanChange => CreditNoteOrigin::PlanChange,
            DomainOrigin::Cancellation => CreditNoteOrigin::Cancellation,
            DomainOrigin::LatePack => CreditNoteOrigin::LatePack,
            DomainOrigin::UnappliedChange => CreditNoteOrigin::UnappliedChange,
            DomainOrigin::Void => CreditNoteOrigin::Void,
            DomainOrigin::Correction => CreditNoteOrigin::Correction,
        }
    }
}

/// A credit note, its amounts in minor units of its currency and negative, as it gives back.
/// Lists carry what the note itself holds; `lines` are read on a single one.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct CreditNote {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: CreditNoteId,
    pub number: String,
    pub status: CreditNoteStatus,
    #[serde(rename = "type")]
    pub kind: CreditNoteType,
    pub origin: CreditNoteOrigin,
    /// The invoice it credits: `GET /api/v2/invoices/{id}`.
    pub invoice: InvoiceRef,
    pub customer: CustomerRef,
    /// The customer as the note prints it.
    pub customer_details: DocumentCustomer,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription: Option<SubscriptionRef>,
    pub currency: String,
    pub subtotal: i64,
    pub tax: i64,
    pub total: i64,
    /// Of the total, what was refunded in money (positive).
    pub refunded: i64,
    /// Of the total, what went to the customer's credit balance (positive).
    pub credited: i64,
    /// Of the total, the invoice's debt it cancelled (positive).
    pub debt_cancelled: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines: Option<Vec<DocumentLine>>,
    pub taxes: Vec<DocumentTax>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,
    pub files: DocumentFiles,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub einvoicing_status: Option<EInvoicingStatus>,
    #[schema(value_type = HashMap<String, Value>)]
    pub custom_properties: serde_json::Value,
    pub created_at: ApiDateTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<ApiDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finalized_at: Option<ApiDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voided_at: Option<ApiDateTime>,
}

crate::v2_list!(CreditNoteList, CreditNote);

#[derive(Deserialize, IntoParams, Debug, Clone, Default)]
#[into_params(parameter_in = Query)]
pub struct CreditNoteListQuery {
    /// Only this customer's: its id, or `ext:<alias>`.
    pub customer: Option<String>,
    /// Only those crediting this invoice, by id.
    pub invoice: Option<String>,
    /// Only this subscription's, by id or external id.
    pub subscription: Option<String>,
    pub status: Option<CreditNoteStatus>,
    /// How many items, 1 to 100; 20 by default.
    pub limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    pub after: Option<String>,
}

/// A JSON merge patch of a credit note: its custom properties.
#[derive(Deserialize, ToSchema, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct UpdateCreditNote {
    /// Merged into the note's: a key sent `null` is removed. Validated against the tenant's credit
    /// note custom property definitions.
    #[serde(default)]
    #[schema(value_type = Option<HashMap<String, Value>>)]
    pub custom_properties: Option<serde_json::Value>,
}

/// A sub-line of an invoice line credited: how many of its units.
#[derive(Deserialize, Serialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct CreditedSubLine {
    /// The sub-line, by its `id` (`lines[].sub_lines[].id` of the invoice).
    pub sub_line: String,
    #[schema(value_type = String, format = "decimal", example = "2")]
    pub quantity: Decimal,
}

/// An invoice line credited, by exactly one of `quantity`, `amount` and `sub_lines`.
#[derive(Deserialize, Serialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct CreditedLine {
    /// The invoice line, by its `id` (`lines[].id` of the invoice).
    pub line: String,
    /// Units credited at the line's unit price; a line priced by tiers, a volume or a package
    /// credits what its model priced less what it prices the units kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal", example = "1")]
    pub quantity: Option<Decimal>,
    /// An amount credited before tax, in minor units, at most what is left of the line: for a
    /// line no unit price expresses (a minimum's shortfall, usage priced in credits).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<i64>,
    /// Sub-lines credited; the line's other sub-lines are not.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sub_lines: Vec<CreditedSubLine>,
}

/// The status a credit note is issued in.
#[derive(Deserialize, Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum IssuedStatus {
    /// Finalized at once: numbered, its credit applied as `type` says.
    #[default]
    Finalized,
    /// Left a draft, finalized later (from the dashboard).
    Draft,
}

/// A credit note an operator issues against a finalized invoice.
#[derive(Deserialize, Serialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct IssueCreditNote {
    /// The invoice credited, by id.
    pub invoice: String,
    /// The lines credited; none: every line of the invoice in full.
    #[serde(default)]
    pub lines: Vec<CreditedLine>,
    /// How the credit goes back: to the customer's balance (by default), refunded, or the
    /// invoice's debt cancelled.
    #[serde(rename = "type", default)]
    pub kind: CreditNoteType,
    /// True when it corrects what was billed (days refunded by hand): counted like the engine's
    /// own notes, so no later change gives it back again; it reads `origin: correction`. False
    /// (by default), a goodwill credit, `origin: manual`, which caps what a later change takes
    /// back (R-082).
    #[serde(default)]
    pub correction: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,
    #[serde(default)]
    pub status: IssuedStatus,
    /// Validated against the tenant's credit note custom property definitions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<HashMap<String, Value>>)]
    pub custom_properties: Option<serde_json::Value>,
}

/// The lines asked, as the store credits them, each checked at its pointer: a line of the invoice,
/// by exactly one of a positive quantity, a positive amount or its sub-lines.
fn credited_lines(
    lines: &[CreditedLine],
    invoice_lines: &[meteroid_store::domain::LineItem],
) -> Result<Vec<CreditLineItem>, Problem> {
    lines
        .iter()
        .enumerate()
        .map(|(index, asked)| {
            let at = |field: &str| format!("/lines/{index}/{field}");
            let Some(line) = invoice_lines
                .iter()
                .find(|line| line.local_id == asked.line)
            else {
                return Err(Problem::field(
                    at("line"),
                    "not_found",
                    format!("The invoice has no line '{}'", asked.line),
                ));
            };
            match (asked.quantity, asked.amount, asked.sub_lines.is_empty()) {
                (Some(quantity), None, true) => {
                    if quantity <= Decimal::ZERO {
                        return Err(Problem::field(
                            at("quantity"),
                            "invalid",
                            "A quantity credited is positive",
                        ));
                    }
                    Ok(CreditLineItem::Line {
                        local_id: line.local_id.clone(),
                        quantity,
                    })
                }
                (None, Some(amount), true) => {
                    if amount <= 0 {
                        return Err(Problem::field(
                            at("amount"),
                            "invalid",
                            "An amount credited is positive, in minor units",
                        ));
                    }
                    Ok(CreditLineItem::Amount {
                        local_id: line.local_id.clone(),
                        amount_cents: amount,
                    })
                }
                (None, None, false) => {
                    let sub_lines = asked
                        .sub_lines
                        .iter()
                        .enumerate()
                        .map(|(sub_index, sub)| {
                            let sub_at = |field: &str| {
                                format!("/lines/{index}/sub_lines/{sub_index}/{field}")
                            };
                            if !line
                                .sub_lines
                                .iter()
                                .any(|candidate| candidate.local_id == sub.sub_line)
                            {
                                return Err(Problem::field(
                                    sub_at("sub_line"),
                                    "not_found",
                                    format!("The line has no sub-line '{}'", sub.sub_line),
                                ));
                            }
                            if sub.quantity <= Decimal::ZERO {
                                return Err(Problem::field(
                                    sub_at("quantity"),
                                    "invalid",
                                    "A quantity credited is positive",
                                ));
                            }
                            Ok(CreditSubLineItem {
                                local_id: sub.sub_line.clone(),
                                quantity: sub.quantity,
                            })
                        })
                        .collect::<Result<Vec<_>, Problem>>()?;
                    Ok(CreditLineItem::SubLines {
                        local_id: line.local_id.clone(),
                        sub_lines,
                    })
                }
                (None, None, true) => Err(Problem::field(
                    format!("/lines/{index}"),
                    "required",
                    "Send the line's `quantity`, `amount` or `sub_lines` credited",
                )),
                _ => Err(Problem::field(
                    format!("/lines/{index}"),
                    "invalid",
                    "Send one of `quantity`, `amount` and `sub_lines`",
                )),
            }
        })
        .collect()
}

// ── Rendering ───────────────────────────────────────────────────────

/// What credit notes name, to read in one round.
fn reference_ids<'a>(notes: impl IntoIterator<Item = &'a DomainCreditNote>) -> ReferenceIds {
    let mut ids = ReferenceIds::default();
    for note in notes {
        ids.customers.push(note.customer_id);
        ids.subscriptions.extend(note.subscription_id);
    }
    ids
}

fn summary(note: &DomainCreditNote, refs: &Refs) -> CreditNote {
    let details = &note.customer_details;
    CreditNote {
        id: note.id,
        number: note.credit_note_number.clone(),
        status: note.status.clone().into(),
        kind: note.credit_type.into(),
        origin: note.origin.into(),
        invoice: InvoiceRef {
            id: note.invoice_id,
            number: note.invoice_number.clone(),
        },
        customer: refs.customer_or(note.customer_id, details.alias.as_deref(), &details.name),
        customer_details: documents::customer(details),
        subscription: note.subscription_id.map(|id| refs.subscription(id)),
        currency: note.currency.clone(),
        subtotal: note.subtotal,
        tax: note.tax_amount,
        total: note.total,
        refunded: note.refunded_amount_cents,
        credited: note.credited_amount_cents,
        debt_cancelled: note.debt_cancelled_amount_cents,
        lines: None,
        taxes: documents::taxes(&note.tax_breakdown),
        reason: note.reason.clone(),
        memo: note.memo.clone(),
        files: documents::files(note.pdf_document_id, note.xml_document_id),
        einvoicing_status: note.einvoicing_status.map(Into::into),
        custom_properties: note.custom_properties.clone(),
        created_at: note.created_at.into(),
        updated_at: note.updated_at.map(Into::into),
        finalized_at: note.finalized_at.map(Into::into),
        voided_at: note.voided_at.map(Into::into),
    }
}

async fn detailed(
    state: &AppState,
    tenant_id: TenantId,
    note: &DomainCreditNote,
) -> Result<CreditNote, Problem> {
    let products = ProductCodes::load(state, tenant_id, &note.line_items).await?;
    let refs = Refs::load(&state.store, tenant_id, &reference_ids([note])).await?;
    let mut out = summary(note, &refs);
    out.lines = Some(documents::lines(&note.line_items, &products));
    Ok(out)
}

async fn credit_note_of(
    state: &AppState,
    tenant_id: TenantId,
    raw: &str,
) -> Result<DomainCreditNote, Problem> {
    let id: CreditNoteId = ids::plain_id(raw, "credit note")?;
    state
        .store
        .get_credit_note_by_id(tenant_id, id)
        .await
        .map_err(|error| document_not_found("credit note", id)(Problem::from(error)))
}

// ── Handlers ────────────────────────────────────────────────────────

/// List credit notes
///
/// The tenant's credit notes in creation order, paged by cursor, without their lines.
#[utoipa::path(
    get,
    tag = "Credit notes",
    path = "/api/v2/credit-notes",
    params(CreditNoteListQuery),
    responses(
        (status = 200, description = "A page of credit notes", body = CreditNoteList),
        (status = 401, description = "Unauthorized", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A filter is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_credit_notes(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Query(query): Query<CreditNoteListQuery>,
) -> Result<Json<CreditNoteList>, Problem> {
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
    let invoice_id = match query.invoice.as_deref() {
        Some(raw) => Some(
            ids::plain_id::<InvoiceId>(raw, "invoice").map_err(filter_not_found("invoice", raw))?,
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
    let mut items = state
        .store
        .list_credit_notes_after(
            tenant_id,
            CreditNotePageFilter {
                customer_id,
                invoice_id,
                subscription_id,
                statuses: query.status.map(|status| vec![status.into()]),
            },
            cursor.map(|Cursor(after)| CreditNoteId::from(after)),
            limit + 1,
        )
        .await?;
    let has_more = items.len() > limit;
    items.truncate(limit);
    let next_cursor = if has_more {
        items.last().map(|note| Cursor(note.id.as_uuid()).encode())
    } else {
        None
    };
    let refs = Refs::load(&state.store, tenant_id, &reference_ids(&items)).await?;
    Ok(Json(CreditNoteList {
        data: items.iter().map(|note| summary(note, &refs)).collect(),
        next_cursor,
        has_more,
    }))
}

/// Issue a credit note
///
/// Credits a finalized invoice, by line (`lines`, each by its id on the invoice) or in full, on
/// the store write the dashboard uses: the credit goes back as `type` says (to the customer's
/// balance, refunded, or the invoice's debt cancelled), its tax reversed as it was charged.
/// Finalized at once unless `status: draft`. An `Idempotency-Key` is required: a retry never
/// credits twice.
#[utoipa::path(
    post,
    tag = "Credit notes",
    path = "/api/v2/credit-notes",
    request_body = IssueCreditNote,
    responses(
        (status = 201, description = "Issued", body = CreditNote),
        (status = 404, description = "No such invoice", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "The invoice's state refuses it: not finalized, or a payment of it in progress", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A line or an amount refused (more than what is left of a line…), or no Idempotency-Key", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn create_credit_note(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Body(body): Body<IssueCreditNote>,
) -> Result<Response, Problem> {
    if !headers.contains_key(IDEMPOTENCY_KEY_HEADER) {
        return Err(Problem::key_required(
            "A credit note is issued with an Idempotency-Key: a retry must never credit twice",
        ));
    }
    let tenant_id = auth.tenant_id;
    let invoice_id: InvoiceId = ids::plain_id(&body.invoice, "invoice")?;
    let invoice = state
        .store
        .get_invoice_by_id(tenant_id, invoice_id)
        .await
        .map_err(|error| document_not_found("invoice", invoice_id)(Problem::from(error)))?;
    if invoice.status != InvoiceStatusEnum::Finalized {
        return Err(Problem::conflict(
            ProblemCode::Conflict,
            "A credit note credits a finalized invoice: a draft's lines are edited instead \
             (PATCH /api/v2/invoices/{id})",
        ));
    }
    let line_items = credited_lines(&body.lines, &invoice.line_items)?;
    let blank = |text: &Option<String>| {
        text.as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    let params = CreateCreditNoteParams {
        invoice_id,
        line_items,
        reason: blank(&body.reason),
        memo: blank(&body.memo),
        credit_type: body.kind.into(),
        custom_properties: body.custom_properties.clone(),
    };
    let kind = if body.correction {
        OperatorNoteKind::Correction
    } else {
        OperatorNoteKind::Goodwill
    };
    let note = state
        .store
        .create_operator_credit_note(
            auth.as_actor(),
            tenant_id,
            params,
            kind,
            body.status == IssuedStatus::Finalized,
        )
        .await?;
    // Written: from here the request answers what it issued, whatever is read back.
    let read = detailed(&state, tenant_id, &note).await;
    Ok(committed_write(StatusCode::CREATED, read, || {
        summary(&note, &Refs::default())
    }))
}

/// Get a credit note
///
/// With its lines, rendered as an invoice's.
#[utoipa::path(
    get,
    tag = "Credit notes",
    path = "/api/v2/credit-notes/{id}",
    params(("id" = String, Path, description = "Credit note id")),
    responses(
        (status = 200, description = "The credit note", body = CreditNote),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_credit_note(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<CreditNote>, Problem> {
    let note = credit_note_of(&state, auth.tenant_id, &id).await?;
    Ok(Json(detailed(&state, auth.tenant_id, &note).await?))
}

/// Update a credit note
///
/// A JSON merge patch of its custom properties.
#[utoipa::path(
    patch,
    tag = "Credit notes",
    path = "/api/v2/credit-notes/{id}",
    params(("id" = String, Path, description = "Credit note id")),
    request_body(content = UpdateCreditNote, content_type = "application/merge-patch+json"),
    responses(
        (status = 200, description = "Updated", body = CreditNote),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A custom property is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn update_credit_note(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<UpdateCreditNote>,
) -> Result<Response, Problem> {
    let mut note = credit_note_of(&state, auth.tenant_id, &id).await?;
    if let Some(properties) = body.custom_properties {
        note = state
            .store
            .patch_credit_note_custom_properties(auth.tenant_id, note.id, properties)
            .await
            .map_err(|error| Problem::at("/custom_properties", error))?;
    }
    let read = detailed(&state, auth.tenant_id, &note).await;
    Ok(committed_write(StatusCode::OK, read, || {
        summary(&note, &Refs::default())
    }))
}

/// Download a credit note's PDF
///
/// `404` until it is generated, shortly after the note is finalized: see `files.pdf`.
#[utoipa::path(
    get,
    tag = "Credit notes",
    path = "/api/v2/credit-notes/{id}/pdf",
    params(("id" = String, Path, description = "Credit note id")),
    responses(
        (status = 200, description = "The PDF", content_type = "application/pdf", body = inline(crate::invoices::model::BinaryFile)),
        (status = 404, description = "Not found, or no PDF yet", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn download_credit_note_pdf(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, Problem> {
    let note = credit_note_of(&state, auth.tenant_id, &id).await?;
    documents::pdf_response(&state, note.pdf_document_id).await
}

/// Download a credit note's e-invoice
///
/// The structured e-invoice (EN 16931 XML) issued with it. `404` when it was not issued as one.
#[utoipa::path(
    get,
    tag = "Credit notes",
    path = "/api/v2/credit-notes/{id}/xml",
    params(("id" = String, Path, description = "Credit note id")),
    responses(
        (status = 200, description = "The e-invoice", content_type = "application/xml", body = inline(crate::invoices::model::BinaryFile)),
        (status = 404, description = "Not found, or not issued as an e-invoice", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn download_credit_note_xml(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, Problem> {
    let note = credit_note_of(&state, auth.tenant_id, &id).await?;
    documents::xml_response(&state, note.xml_document_id, &note.credit_note_number).await
}
