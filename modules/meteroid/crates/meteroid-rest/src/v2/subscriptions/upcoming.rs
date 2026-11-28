//! `GET /subscriptions/{id_or_external_id}/upcoming-invoice`: the subscription's next invoice as
//! the dashboard and the portal forecast it (`upcoming_invoice_forecast`), its lines rendered as
//! an invoice's, with the credit it is expected to draw.
use crate::AppState;
use crate::v2::customers::credits::GrantKind;
use crate::v2::documents::{self, DocumentCoupon, DocumentLine, DocumentTax, ProductCodes};
use crate::v2::extract::Path;
use crate::v2::ids;
use crate::v2::problem::{Problem, ProblemCode};
use crate::v2::refs::{InvoiceRef, PlanRef, Refs, SubscriptionRef};
use axum::Extension;
use axum::Json;
use axum::extract::State;
use chrono::NaiveDate;
use common_domain::ids::BaseId;
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use meteroid_store::domain::SubscriptionDetails;
use meteroid_store::domain::credit_outlook::{
    CreditDeduction, CreditGroupKind, UpcomingChange, UpcomingInvoiceForecast,
};
use meteroid_store::domain::portal_credit::recurring_issue_and_due;
use meteroid_store::repositories::catalog_codes::ReferenceIds;
use meteroid_store::repositories::plans::PlansInterfaceAuto;
use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;
use serde::Serialize;
use utoipa::ToSchema;

/// What the next invoice settles besides renewing the period: `ends`, the subscription's final
/// invoice (cancelled, or reaching its end date); `plan_change`, a scheduled plan change takes
/// effect on its date; `amendment`, a scheduled amendment does.
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UpcomingInvoiceChange {
    Ends,
    PlanChange,
    Amendment,
}

/// One kind of credit the invoice is expected to draw.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct UpcomingCredit {
    /// The kind of credit.
    pub kind: GrantKind,
    /// "Included with Scale", "Purchased credit", "Free credit" or "Account balance".
    pub label: String,
    /// The subscription the credit belongs to; absent when it pays any of the customer's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription: Option<SubscriptionRef>,
    /// In minor units of `currency`.
    pub amount: i64,
    /// ISO 4217.
    pub currency: String,
    /// Taken off the charges before tax (promotional credit); otherwise paid on the taxed total.
    pub before_tax: bool,
}

/// A pending invoice billed before this one that draws on the same customer-wide credit.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct BilledFirst {
    /// The invoice, once drafted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invoice: Option<InvoiceRef>,
    /// The subscription it bills.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription: Option<SubscriptionRef>,
    pub invoice_date: NaiveDate,
}

/// A subscription's next invoice as it would be issued today: its fixed charges for the next
/// period, the usage of the current one so far, the changes scheduled by its date, and the credit
/// it would draw once every invoice billed before it drew its own. Amounts in minor units.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct UpcomingInvoice {
    pub subscription: SubscriptionRef,
    /// The plan the subscription is on now.
    pub plan: PlanRef,
    /// ISO 4217.
    pub currency: String,
    /// The date it is issued for: the period's end, or the end date of a final invoice
    /// (`change: ends`), which can fall before `period_end`.
    pub invoice_date: NaiveDate,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    /// When it is finalized (and charged, when charged automatically): after the grace period.
    pub finalized_on: NaiveDate,
    /// `finalized_on` plus `net_terms`.
    pub due_date: NaiveDate,
    pub net_terms: i32,
    /// What it settles besides a renewal; absent for a plain renewal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change: Option<UpcomingInvoiceChange>,
    /// The plan a scheduled plan change moves to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_plan: Option<PlanRef>,
    /// False when nothing will be invoiced: a subscription that never activated ends without one.
    pub invoiced: bool,
    /// The lines' total before discounts, credit and tax.
    pub subtotal: i64,
    /// What discounts and coupons take off.
    pub discount: i64,
    /// Promotional credit drawn before tax.
    pub promotional_credits: i64,
    pub tax: i64,
    /// The total, tax included.
    pub total: i64,
    /// Credit drawn on the taxed total.
    pub applied_credits: i64,
    /// What is left to pay once credit applies.
    pub amount_due: i64,
    pub lines: Vec<DocumentLine>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub taxes: Vec<DocumentTax>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub coupons: Vec<DocumentCoupon>,
    /// The credit drawn, by kind, in draw order.
    pub credits: Vec<UpcomingCredit>,
    /// The credit is shared with other pending invoices, so the amounts after it are estimates.
    pub credit_estimate_shared: bool,
    /// The pending invoices billed first that draw on the same credit.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub billed_first: Vec<BilledFirst>,
}

/// Get a subscription's upcoming invoice
///
/// Its next invoice as it would be issued today, written nowhere: the next period's fixed
/// charges, the current period's usage so far, discounts, tax, and the changes scheduled by its
/// date (a cancellation, a plan change, an amendment). The credit it draws is estimated in billing
/// order across the customer's pending invoices. For where usage is heading, read the
/// subscription's usage (`projected_costs`).
#[utoipa::path(
    get,
    tag = "Subscriptions",
    path = "/api/v2/subscriptions/{id_or_external_id}/upcoming-invoice",
    params(("id_or_external_id" = String, Path, description = "Subscription id or external id")),
    responses(
        (status = 200, description = "The subscription's next invoice", body = UpcomingInvoice),
        (status = 404, description = "The subscription is not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "The subscription has ended (cancelled, aborted, completed): it has no next invoice", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_upcoming_invoice(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_external_id): Path<String>,
) -> Result<Json<UpcomingInvoice>, Problem> {
    let tenant_id = auth.tenant_id;
    let subscription_id = ids::subscription_id(&state, tenant_id, &id_or_external_id).await?;
    let details = state
        .store
        .get_subscription_details(tenant_id, subscription_id)
        .await?;
    if details.subscription.status.is_terminal() {
        return Err(Problem::conflict(
            ProblemCode::Conflict,
            "The subscription has ended: it has no next invoice",
        ));
    }
    let forecast = state.services.upcoming_invoice_forecast(&details).await?;

    let new_plan = match &forecast.change {
        Some(UpcomingChange::PlanChange { plan_version_id }) => {
            let plan = state
                .store
                .get_plan_by_version_id(*plan_version_id, tenant_id)
                .await?;
            Some(PlanRef {
                id: plan.plan.id,
                code: plan.plan.code,
                version: plan.version.map(|version| version.version),
                option: None,
            })
        }
        _ => None,
    };
    let products = ProductCodes::load(&state, tenant_id, &forecast.content.invoice_lines).await?;
    let refs = Refs::load(&state.store, tenant_id, &reference_ids(&details, &forecast)).await?;
    Ok(Json(upcoming_invoice(
        &details, forecast, new_plan, &products, &refs,
    )?))
}

fn reference_ids(
    details: &SubscriptionDetails,
    forecast: &UpcomingInvoiceForecast,
) -> ReferenceIds {
    let mut subscriptions = vec![details.subscription.id];
    subscriptions.extend(
        forecast
            .credit
            .iter()
            .filter_map(|c| c.group.subscription_id),
    );
    subscriptions.extend(
        forecast
            .billed_first
            .iter()
            .filter_map(|b| b.subscription_id),
    );
    subscriptions.sort_by_key(|id| id.as_uuid());
    subscriptions.dedup();
    ReferenceIds {
        subscriptions,
        plans: vec![details.subscription.plan_id],
        invoices: forecast
            .billed_first
            .iter()
            .filter_map(|b| b.invoice_id)
            .collect(),
        ..ReferenceIds::default()
    }
}

fn upcoming_invoice(
    details: &SubscriptionDetails,
    forecast: UpcomingInvoiceForecast,
    new_plan: Option<PlanRef>,
    products: &ProductCodes,
    refs: &Refs,
) -> Result<UpcomingInvoice, Problem> {
    let subscription = &details.subscription;
    let entity = &details.invoicing_entity;
    let (finalized_on, due_date) = recurring_issue_and_due(
        forecast.invoice_date,
        subscription.net_terms,
        entity.grace_period_hours,
        entity.consolidate_recurring_invoices,
    );
    let content = &forecast.content;
    let credits = credits(&forecast.credit, refs)?;
    Ok(UpcomingInvoice {
        subscription: refs.subscription(subscription.id),
        plan: refs.plan(
            subscription.plan_id,
            Some(subscription.version as i32),
            None,
        ),
        currency: subscription.currency.clone(),
        invoice_date: forecast.invoice_date,
        period_start: subscription.current_period_start,
        period_end: subscription
            .current_period_end
            .unwrap_or_else(|| common_clock::now().date_naive()),
        finalized_on,
        due_date,
        net_terms: subscription.net_terms as i32,
        change: forecast.change.as_ref().map(|change| match change {
            UpcomingChange::Ends { .. } => UpcomingInvoiceChange::Ends,
            UpcomingChange::PlanChange { .. } => UpcomingInvoiceChange::PlanChange,
            UpcomingChange::Amendment => UpcomingInvoiceChange::Amendment,
        }),
        new_plan,
        invoiced: !matches!(
            forecast.change,
            Some(UpcomingChange::Ends { billed: false, .. })
        ),
        subtotal: content.subtotal,
        discount: content.discount,
        promotional_credits: content.promotional_credits,
        tax: content.tax_amount,
        total: content.total,
        applied_credits: content.applied_credits,
        amount_due: content.amount_due,
        lines: documents::lines(&content.invoice_lines, products),
        taxes: documents::taxes(&content.tax_breakdown),
        coupons: documents::coupons(&content.applied_coupons),
        credits,
        credit_estimate_shared: forecast.credit_estimate_shared,
        billed_first: forecast
            .billed_first
            .iter()
            .map(|first| BilledFirst {
                invoice: first.invoice_id.map(|id| refs.invoice(id)),
                subscription: first.subscription_id.map(|id| refs.subscription(id)),
                invoice_date: first.invoice_date,
            })
            .collect(),
    })
}

fn grant_kind(kind: CreditGroupKind) -> GrantKind {
    match kind {
        CreditGroupKind::Included => GrantKind::Included,
        CreditGroupKind::Prepaid => GrantKind::Prepaid,
        CreditGroupKind::Bonus => GrantKind::Bonus,
        CreditGroupKind::AccountBalance => GrantKind::AccountBalance,
    }
}

/// One row per kind of credit and phase (before or after tax), in draw order.
fn credits(deductions: &[CreditDeduction], refs: &Refs) -> Result<Vec<UpcomingCredit>, Problem> {
    let mut rows: Vec<(CreditDeduction, i64)> = Vec::new();
    for deduction in deductions {
        match rows.iter_mut().find(|(row, _)| {
            row.group.kind == deduction.group.kind
                && row.group.label == deduction.group.label
                && row.group.subscription_id == deduction.group.subscription_id
                && row.denomination == deduction.denomination
                && row.before_tax == deduction.before_tax
        }) {
            Some((_, amount)) => {
                *amount = amount
                    .checked_add(deduction.amount_minor)
                    .ok_or_else(Problem::internal)?;
            }
            None => rows.push((deduction.clone(), deduction.amount_minor)),
        }
    }
    Ok(rows
        .into_iter()
        .map(|(row, amount)| UpcomingCredit {
            kind: grant_kind(row.group.kind),
            label: row.group.label,
            subscription: row.group.subscription_id.map(|id| refs.subscription(id)),
            amount,
            currency: row.denomination,
            before_tax: row.before_tax,
        })
        .collect())
}
