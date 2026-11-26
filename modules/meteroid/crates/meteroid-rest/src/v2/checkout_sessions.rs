//! `/api/v2/checkout-sessions` (ADR-0013 §15): a hosted checkout of a plan, its purchase named
//! as `POST /subscriptions` names it (the plan by code, its option by alias, the customer by id or
//! `ext:<alias>`, seats, levels, charges, add-ons and coupons by code) and resolved by the same
//! code (`subscriptions::create::resolve_purchase`), created, read, listed by keyset in SQL and
//! cancelled on the store calls v1 uses. Completing it, on the hosted page, creates the
//! subscription.
use crate::AppState;
use crate::checkoutsessions::router::{generate_checkout_url, validate_redirect_url};
use crate::v2::bounds;
use crate::v2::customers::resources::{ResourceName, resource_refs, stored_resource_id};
use crate::v2::documents::{document_not_found, filter_not_found};
use crate::v2::extract::{Body, Path, Query};
use crate::v2::idempotency::committed_write;
use crate::v2::ids;
use crate::v2::pagination::{Cursor, PageQuery};
use crate::v2::problem::{Problem, ProblemCode};
use crate::v2::refs::{
    AddOnRef, CouponRef, CustomerRef, PlanRef, ReferenceIds, Refs, ResourceRef, SubscriptionRef,
};
use crate::v2::subscriptions::create::{PurchaseAsked, Terms, check_terms, resolve_purchase};
use crate::v2::subscriptions::model::{AddOnPurchase, CreateCharges, InvoicingInput};
use crate::v2::subscriptions::resolve::not_found_at;
use axum::Extension;
use axum::Json;
use axum::extract::State;
use axum::response::Response;
use chrono::NaiveDate;
use common_domain::ids::{BaseId, CheckoutSessionId, PlanId, PlanVersionId, ResourceId, TenantId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use meteroid_app::errors::RestApiError;
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_store::domain::checkout_sessions::{
    CheckoutSession as DomainSession, CheckoutSessionStatus, CheckoutType, CreateCheckoutSession,
};
use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;
use meteroid_store::repositories::coupons::CouponInterface;
use meteroid_store::repositories::plans::PlansInterfaceAuto;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

// ── Model ───────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckoutStatus {
    /// Waiting for the customer.
    Created,
    /// The customer paid and the payment is being confirmed, or failed and may be retried.
    AwaitingPayment,
    /// Paid: its subscription is created.
    Completed,
    Expired,
    Cancelled,
}

impl From<CheckoutSessionStatus> for CheckoutStatus {
    fn from(status: CheckoutSessionStatus) -> Self {
        match status {
            CheckoutSessionStatus::Created => CheckoutStatus::Created,
            CheckoutSessionStatus::AwaitingPayment => CheckoutStatus::AwaitingPayment,
            CheckoutSessionStatus::Completed => CheckoutStatus::Completed,
            CheckoutSessionStatus::Expired => CheckoutStatus::Expired,
            CheckoutSessionStatus::Cancelled => CheckoutStatus::Cancelled,
        }
    }
}

impl From<CheckoutStatus> for CheckoutSessionStatus {
    fn from(status: CheckoutStatus) -> Self {
        match status {
            CheckoutStatus::Created => CheckoutSessionStatus::Created,
            CheckoutStatus::AwaitingPayment => CheckoutSessionStatus::AwaitingPayment,
            CheckoutStatus::Completed => CheckoutSessionStatus::Completed,
            CheckoutStatus::Expired => CheckoutSessionStatus::Expired,
            CheckoutStatus::Cancelled => CheckoutSessionStatus::Cancelled,
        }
    }
}

/// What the checkout buys.
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckoutKind {
    /// A new subscription, created when it is paid (`POST /checkout-sessions`).
    SelfServe,
    /// The activation of a subscription waiting for its checkout.
    SubscriptionActivation,
    /// A plan change of a subscription, from the customer portal.
    PlanChange,
    /// An add-on bought for a subscription, from the customer portal.
    AddOnPurchase,
}

/// The kind a plan checkout reads as; a top-up checkout buys credit, not a plan, and is not
/// one.
fn kind(checkout_type: CheckoutType) -> Option<CheckoutKind> {
    match checkout_type {
        CheckoutType::SelfServe => Some(CheckoutKind::SelfServe),
        CheckoutType::SubscriptionActivation => Some(CheckoutKind::SubscriptionActivation),
        CheckoutType::PlanChange => Some(CheckoutKind::PlanChange),
        CheckoutType::AddonPurchase => Some(CheckoutKind::AddOnPurchase),
        CheckoutType::TopUp => None,
    }
}

/// An add-on the checkout buys.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq, Eq)]
pub struct CheckoutAddOn {
    pub add_on: AddOnRef,
    pub quantity: i32,
}

/// The invoicing terms of the subscription the checkout creates.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq, Eq)]
pub struct CheckoutInvoicing {
    /// The plan's when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net_terms: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purchase_order: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,
    pub auto_finalize: bool,
    pub charge_automatically: bool,
}

/// A hosted checkout: the customer pays on `url`, which creates (or changes) the subscription.
/// The quantities, levels and charges asked are the page's and the subscription's, not read back
/// here; a list item carries no `url` (read it on the session).
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct CheckoutSession {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: CheckoutSessionId,
    #[serde(rename = "type")]
    pub kind: CheckoutKind,
    pub status: CheckoutStatus,
    pub customer: CustomerRef,
    pub plan: PlanRef,
    /// The resource the subscription is for; none: the whole customer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<ResourceRef>,
    /// On creation, what its subscription would be accepted with: `plan_sold_for_mismatch` (the
    /// plan is sold for another target), `metric_not_attributed` (its resource would never bill
    /// some metrics it prices).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// With `metric_not_attributed`: the codes of the metrics it prices that are not attributed
    /// to its resource's type, so its subscription would bill none of their usage.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub metrics_not_attributed: Vec<String>,
    pub add_ons: Vec<CheckoutAddOn>,
    /// A coupon deleted since the session was created is left out.
    pub coupons: Vec<CouponRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_date: Option<NaiveDate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_date: Option<NaiveDate>,
    /// The plan's trial when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trial_days: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing_day_anchor: Option<i16>,
    pub invoicing: CheckoutInvoicing,
    /// The hosted page, while the customer can pay.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancel_url: Option<String>,
    /// The subscription created (or changed) by the checkout.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription: Option<SubscriptionRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<HashMap<String, Value>>)]
    pub metadata: Option<serde_json::Value>,
    /// None: it never expires.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<ApiDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<ApiDateTime>,
    pub created_at: ApiDateTime,
}

crate::v2_list!(CheckoutSessionList, CheckoutSession);

#[derive(Deserialize, ToSchema, Debug, Clone)]
#[schema(as = CreateCheckoutSession)]
#[serde(deny_unknown_fields)]
pub struct CreateCheckoutSessionBody {
    /// The customer's id, or its alias as `ext:<alias>`.
    pub customer: String,
    /// By code or id.
    pub plan: String,
    /// The option's alias; the first the currency sells when left out.
    #[serde(default)]
    pub option: Option<String>,
    /// A published version's number; the active one when left out.
    #[serde(default)]
    pub version: Option<i32>,
    /// Checked when sent: a checkout is sold in its customer's currency when the plan sells it,
    /// else in the plan's.
    #[serde(default)]
    pub currency: Option<String>,
    /// A stored resource of the customer the subscription is for, `{"id": …}` or
    /// `{"type": …, "key": …}`; the whole customer when left out.
    #[serde(default)]
    pub resource: Option<ResourceName>,
    /// The starting count of each managed quantity its seat charges count, by code.
    #[serde(default)]
    pub quantities: BTreeMap<String, String>,
    /// The level of each usage-levels charge, by product code, as the usage it includes.
    #[serde(default)]
    pub levels: BTreeMap<String, String>,
    #[serde(default)]
    pub charges: CreateCharges,
    #[serde(default)]
    pub add_ons: Vec<AddOnPurchase>,
    /// Coupon codes.
    #[serde(default)]
    pub coupons: Vec<String>,
    /// When billing starts; the day it is paid when left out.
    #[serde(default)]
    pub start_date: Option<NaiveDate>,
    #[serde(default)]
    pub end_date: Option<NaiveDate>,
    /// The plan's trial when left out.
    #[serde(default)]
    pub trial_days: Option<u32>,
    #[serde(default)]
    pub billing_day_anchor: Option<u16>,
    #[serde(default)]
    pub invoicing: InvoicingInput,
    /// Hours before it expires, 1 to 8760: 1 by default, 0 for never.
    #[serde(default)]
    pub expires_in_hours: Option<u32>,
    /// Absolute http(s) URL the customer is sent to once paid, `checkout_session_id` appended.
    #[serde(default)]
    pub success_url: Option<String>,
    /// Absolute http(s) URL offered to the customer to leave without paying.
    #[serde(default)]
    pub cancel_url: Option<String>,
    #[serde(default)]
    #[schema(value_type = Option<HashMap<String, Value>>)]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Deserialize, IntoParams, Debug, Clone)]
#[into_params(parameter_in = Query)]
pub struct CheckoutSessionListQuery {
    /// How many items, 1 to 100; 20 by default.
    pub limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    pub after: Option<String>,
    /// The customer's id, or its alias as `ext:<alias>`.
    pub customer: Option<String>,
    pub status: Option<CheckoutStatus>,
}

// ── Rendering ───────────────────────────────────────────────────────

/// What the sessions of a response name, read once for all of them: a plan version each
/// distinct one, the rest in one round.
struct Names<'a> {
    state: &'a AppState,
    tenant_id: TenantId,
    plans: HashMap<PlanVersionId, (PlanId, i32)>,
    refs: Refs,
    /// The coupons of codes redeemed when paid, by code as the session holds it.
    redeemed: HashMap<String, CouponRef>,
    resources: HashMap<ResourceId, ResourceRef>,
}

/// The code a session redeems when paid, as v1 names a coupon.
fn redeemed_code(session: &DomainSession) -> Option<&str> {
    session
        .coupon_code
        .as_deref()
        .map(str::trim)
        .filter(|code| !code.is_empty())
}

impl<'a> Names<'a> {
    async fn load(
        state: &'a AppState,
        tenant_id: TenantId,
        sessions: &[DomainSession],
    ) -> Result<Self, Problem> {
        let mut ids = ReferenceIds::default();
        let mut codes = Vec::new();
        for session in sessions {
            ids.customers.push(session.customer_id);
            ids.subscriptions.extend(session.subscription_id);
            ids.add_ons.extend(
                session
                    .add_ons
                    .iter()
                    .flat_map(|bought| &bought.add_ons)
                    .map(|add_on| add_on.add_on_id),
            );
            ids.coupons.extend(&session.coupon_ids);
            if session.coupon_ids.is_empty()
                && let Some(code) = redeemed_code(session)
            {
                codes.push(code.to_owned());
            }
        }
        let refs = Refs::load(&state.store, tenant_id, &ids).await?;
        let resources = resource_refs(
            state,
            tenant_id,
            sessions.iter().filter_map(|s| s.resource_id),
        )
        .await?;
        let redeemed = if codes.is_empty() {
            HashMap::new()
        } else {
            let coupons = state.store.list_coupons_by_codes(tenant_id, &codes).await?;
            codes
                .into_iter()
                .filter_map(|code| {
                    coupons
                        .iter()
                        .find(|coupon| coupon.code.eq_ignore_ascii_case(&code))
                        .map(|coupon| {
                            (
                                code,
                                CouponRef {
                                    id: coupon.id,
                                    code: coupon.code.clone(),
                                },
                            )
                        })
                })
                .collect()
        };
        Ok(Names {
            state,
            tenant_id,
            plans: HashMap::new(),
            refs,
            redeemed,
            resources,
        })
    }

    /// The resource as read; by its id when it was deleted since.
    fn resource(&self, id: ResourceId) -> ResourceRef {
        self.resources
            .get(&id)
            .cloned()
            .unwrap_or_else(|| ResourceRef::of_id(id))
    }

    async fn plan(
        &mut self,
        version_id: PlanVersionId,
        option: Option<String>,
    ) -> Result<PlanRef, Problem> {
        let (plan_id, number) = match self.plans.get(&version_id) {
            Some(found) => *found,
            None => {
                let plan = self
                    .state
                    .store
                    .get_plan_by_version_id(version_id, self.tenant_id)
                    .await?;
                let number = plan.version.map(|version| version.version).unwrap_or(0);
                let found = (plan.plan.id, number);
                self.plans.insert(version_id, found);
                self.refs.know_plan(plan.plan.id, plan.plan.code);
                found
            }
        };
        Ok(self.refs.plan(plan_id, Some(number), option))
    }

    /// The session as v2 reads it; none for a checkout that buys no plan (a top-up).
    async fn session(
        &mut self,
        session: DomainSession,
        url: Option<String>,
    ) -> Result<Option<CheckoutSession>, Problem> {
        let (Some(kind), Some(version_id)) = (kind(session.checkout_type), session.plan_version_id)
        else {
            return Ok(None);
        };
        let plan = self.plan(version_id, session.option.clone()).await?;
        let add_ons = session
            .add_ons
            .iter()
            .flat_map(|bought| &bought.add_ons)
            .map(|add_on| CheckoutAddOn {
                add_on: self.refs.add_on(add_on.add_on_id),
                quantity: add_on.quantity,
            })
            .collect();
        // A coupon deleted since the session named it is left out.
        let mut coupons: Vec<CouponRef> = session
            .coupon_ids
            .iter()
            .filter_map(|id| self.refs.found_coupon(*id))
            .collect();
        if session.coupon_ids.is_empty()
            && let Some(coupon) = redeemed_code(&session).and_then(|code| self.redeemed.get(code))
        {
            coupons.push(coupon.clone());
        }
        Ok(Some(CheckoutSession {
            id: session.id,
            kind,
            status: session.status.into(),
            customer: self.refs.customer(session.customer_id),
            plan,
            resource: session.resource_id.map(|id| self.resource(id)),
            warnings: session.warnings,
            metrics_not_attributed: session
                .metrics_not_attributed
                .into_iter()
                .map(|m| m.catalog_code)
                .collect(),
            add_ons,
            coupons,
            start_date: session.billing_start_date,
            end_date: session.end_date,
            trial_days: session.trial_duration_days,
            billing_day_anchor: session.billing_day_anchor,
            invoicing: CheckoutInvoicing {
                net_terms: session.net_terms,
                purchase_order: session.purchase_order,
                memo: session.invoice_memo,
                auto_finalize: session.auto_advance_invoices,
                charge_automatically: session.charge_automatically,
            },
            url,
            success_url: session.success_url,
            cancel_url: session.cancel_url,
            subscription: session.subscription_id.map(|id| self.refs.subscription(id)),
            metadata: session.metadata,
            expires_at: session.expires_at.map(Into::into),
            completed_at: session.completed_at.map(Into::into),
            created_at: session.created_at.into(),
        }))
    }
}

/// The plan checkout a path names, checked to be the tenant's.
async fn find_session(
    state: &AppState,
    tenant_id: TenantId,
    raw: &str,
) -> Result<DomainSession, Problem> {
    let id: CheckoutSessionId = ids::plain_id(raw, "checkout session")?;
    let session = state
        .store
        .get_checkout_session(tenant_id, id)
        .await
        .map_err(|error| document_not_found("checkout session", id)(Problem::from(error)))?;
    if session.plan_version_id.is_none() || kind(session.checkout_type).is_none() {
        return Err(Problem::not_found(format!("checkout session {id}")));
    }
    Ok(session)
}

/// The session read whole, with its page while the customer can pay.
async fn render(
    state: &AppState,
    tenant_id: TenantId,
    session: DomainSession,
) -> Result<CheckoutSession, Problem> {
    let id = session.id;
    let url = if session.can_complete() {
        Some(generate_checkout_url(state, tenant_id, id).await?)
    } else {
        None
    };
    Names::load(state, tenant_id, std::slice::from_ref(&session))
        .await?
        .session(session, url)
        .await?
        .ok_or_else(|| Problem::not_found(format!("checkout session {id}")))
}

fn redirect(field: &'static str, url: Option<&str>) -> Result<(), Problem> {
    validate_redirect_url(field, url).map_err(|error| match error {
        RestApiError::InvalidInput(message) => {
            Problem::field(format!("/{field}"), "invalid", message)
        }
        other => Problem::from(other),
    })
}

// ── Handlers ────────────────────────────────────────────────────────

/// Create a checkout session
///
/// A hosted checkout of a plan for a customer (its id, or `ext:<alias>`): the plan by code, its
/// option by alias, seat counts by managed quantity code, usage levels by product code, price
/// overrides, charges of its own, add-ons and coupons by code, resolved and checked as
/// `POST /subscriptions` checks them. The customer pays on `url`; the subscription is created
/// then. Expires in an hour unless `expires_in_hours` says otherwise.
#[utoipa::path(
    post,
    tag = "Checkout sessions",
    path = "/api/v2/checkout-sessions",
    request_body = CreateCheckoutSessionBody,
    responses(
        (status = 201, description = "Created, with its hosted page", body = CheckoutSession),
        (status = 422, description = "A field is refused: an unknown customer, plan or option, a currency the plan is not sold in, a resource not the customer's or sold to another subscription, a redirect URL that is not http(s)…", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn create_checkout_session(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Body(body): Body<CreateCheckoutSessionBody>,
) -> Result<Response, Problem> {
    let tenant_id = auth.tenant_id;
    redirect("success_url", body.success_url.as_deref())?;
    redirect("cancel_url", body.cancel_url.as_deref())?;
    check_terms(Terms {
        trial_days: body.trial_days,
        billing_day_anchor: body.billing_day_anchor,
        net_terms: body.invoicing.net_terms,
    })?;
    let expires_in_hours = match body.expires_in_hours.unwrap_or(1) {
        0 => None,
        hours if hours <= bounds::MAX_CHECKOUT_EXPIRY_HOURS => Some(hours),
        _ => {
            return Err(Problem::field(
                "/expires_in_hours",
                "invalid",
                format!(
                    "A session expires 1 to {} hours after it is created, or never (0)",
                    bounds::MAX_CHECKOUT_EXPIRY_HOURS
                ),
            ));
        }
    };
    let customer = ids::customer(&state, tenant_id, &body.customer)
        .await
        .map_err(not_found_at(
            "/customer".to_owned(),
            "customer",
            &body.customer,
        ))?;
    let resource_id = match &body.resource {
        Some(name) => {
            Some(stored_resource_id(&state, tenant_id, customer.id, name, "/resource").await?)
        }
        None => None,
    };
    let purchase = resolve_purchase(
        &state,
        &auth,
        &customer,
        PurchaseAsked {
            plan: &body.plan,
            version: body.version,
            option: body.option.as_deref(),
            currency: body.currency.as_deref(),
            quantities: &body.quantities,
            levels: &body.levels,
            charges: &body.charges,
            add_ons: &body.add_ons,
            coupons: &body.coupons,
        },
    )
    .await?;
    let invoicing = body.invoicing;
    let to_i32 = |value: u32, path: &str| {
        i32::try_from(value).map_err(|_| Problem::field(path, "invalid", "The number is too large"))
    };
    let created = state
        .store
        .create_checkout_session(CreateCheckoutSession {
            tenant_id,
            customer_id: customer.id,
            plan_version_id: purchase.fp.version.id,
            // Kept by alias, as the hosted page and its completion read it.
            option: purchase.option.map(|option| option.alias),
            billing_start_date: body.start_date,
            billing_day_anchor: body
                .billing_day_anchor
                .map(|day| {
                    i16::try_from(day).map_err(|_| {
                        Problem::field("/billing_day_anchor", "invalid", "A day of the month")
                    })
                })
                .transpose()?,
            net_terms: invoicing
                .net_terms
                .map(|days| to_i32(days, "/invoicing/net_terms"))
                .transpose()?,
            trial_duration_days: body
                .trial_days
                .map(|days| to_i32(days, "/trial_days"))
                .transpose()?,
            end_date: body.end_date,
            auto_advance_invoices: invoicing.auto_finalize.unwrap_or(true),
            charge_automatically: invoicing.charge_automatically.unwrap_or(true),
            invoice_memo: invoicing.memo,
            invoice_threshold: None,
            purchase_order: invoicing.purchase_order,
            payment_methods_config: None,
            components: purchase.components,
            add_ons: (!purchase.add_ons.is_empty()).then_some(
                meteroid_store::domain::subscription_add_ons::CreateSubscriptionAddOns {
                    add_ons: purchase.add_ons,
                },
            ),
            coupon_code: None,
            coupon_ids: purchase.coupons,
            expires_in_hours,
            metadata: body.metadata,
            checkout_type: CheckoutType::SelfServe,
            subscription_id: None,
            change_date: None,
            success_url: body.success_url,
            cancel_url: body.cancel_url,
            resource_id,
        })
        .await
        .map_err(|error| Problem::resource_or(Some("/plan"), error))?;
    // Written: a retry replays this answer, never a second session, whatever is read back.
    let id = created.id;
    let read = render(&state, tenant_id, created).await;
    Ok(committed_write(
        StatusCode::CREATED,
        read,
        || serde_json::json!({"id": id.to_string()}),
    ))
}

/// Get a checkout session
///
/// With its hosted page while the customer can pay.
#[utoipa::path(
    get,
    tag = "Checkout sessions",
    path = "/api/v2/checkout-sessions/{id}",
    params(("id" = String, Path, description = "Checkout session id")),
    responses(
        (status = 200, description = "The checkout session", body = CheckoutSession),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_checkout_session(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<CheckoutSession>, Problem> {
    let session = find_session(&state, auth.tenant_id, &id).await?;
    Ok(Json(render(&state, auth.tenant_id, session).await?))
}

/// List checkout sessions
///
/// The tenant's plan checkouts in creation order, paged by cursor, without their page (`url`,
/// read on a single one). Top-up checkouts are not listed.
#[utoipa::path(
    get,
    tag = "Checkout sessions",
    path = "/api/v2/checkout-sessions",
    params(CheckoutSessionListQuery),
    responses(
        (status = 200, description = "A page of checkout sessions", body = CheckoutSessionList),
        (status = 422, description = "A filter is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_checkout_sessions(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Query(query): Query<CheckoutSessionListQuery>,
) -> Result<Json<CheckoutSessionList>, Problem> {
    let tenant_id = auth.tenant_id;
    let paging = PageQuery {
        limit: query.limit,
        after: query.after.clone(),
    };
    let limit = paging.limit()?;
    let cursor = paging.cursor()?;
    let customer_id = match query.customer.as_deref() {
        Some(raw) => Some(
            ids::customer(&state, tenant_id, raw)
                .await
                .map_err(filter_not_found("customer", raw))?
                .id,
        ),
        None => None,
    };
    let mut rows = state
        .store
        .list_checkout_sessions_after(
            tenant_id,
            customer_id,
            query.status.map(Into::into),
            cursor.map(|Cursor(after)| CheckoutSessionId::from(after)),
            limit + 1,
        )
        .await?;
    let has_more = rows.len() > limit;
    rows.truncate(limit);
    let next_cursor = if has_more {
        rows.last().map(|row| Cursor(row.id.as_uuid()).encode())
    } else {
        None
    };
    let mut names = Names::load(&state, tenant_id, &rows).await?;
    let mut data = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(session) = names.session(row, None).await? {
            data.push(session);
        }
    }
    Ok(Json(CheckoutSessionList {
        data,
        next_cursor,
        has_more,
    }))
}

/// Cancel a checkout session
///
/// Only while the customer has not paid (`created` or `awaiting_payment`); its page then
/// refuses the payment.
#[utoipa::path(
    post,
    tag = "Checkout sessions",
    path = "/api/v2/checkout-sessions/{id}/cancel",
    params(("id" = String, Path, description = "Checkout session id")),
    responses(
        (status = 200, description = "Cancelled", body = CheckoutSession),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "Completed, expired or cancelled already", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn cancel_checkout_session(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, Problem> {
    let tenant_id = auth.tenant_id;
    let session = find_session(&state, tenant_id, &id).await?;
    let refused = || {
        Problem::conflict(
            ProblemCode::Conflict,
            format!(
                "The checkout session is {}: only one the customer has not paid is cancelled",
                serde_json::to_value(CheckoutStatus::from(session.status.clone()))
                    .ok()
                    .and_then(|status| status.as_str().map(str::to_owned))
                    .unwrap_or_default()
            ),
        )
    };
    if !matches!(
        session.status,
        CheckoutSessionStatus::Created | CheckoutSessionStatus::AwaitingPayment
    ) {
        return Err(refused());
    }
    let cancelled = match state
        .store
        .cancel_checkout_session(tenant_id, session.id)
        .await
    {
        Ok(cancelled) => cancelled,
        // Paid or expired since it was read.
        Err(error)
            if matches!(
                error.current_context(),
                meteroid_store::errors::StoreError::InvalidArgument(_)
            ) =>
        {
            return Err(refused());
        }
        Err(error) => return Err(error.into()),
    };
    // Cancelled: what it answers from here is what it did, whatever is read back.
    let known = serde_json::json!({
        "id": cancelled.id.to_string(),
        "status": CheckoutStatus::from(cancelled.status.clone()),
    });
    let read = render(&state, tenant_id, cancelled).await;
    Ok(committed_write(StatusCode::OK, read, || known))
}

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_checkout_sessions, create_checkout_session))
        .routes(routes!(get_checkout_session))
        .routes(routes!(cancel_checkout_session))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_map_both_ways_in_snake_case() {
        for status in [
            CheckoutStatus::Created,
            CheckoutStatus::AwaitingPayment,
            CheckoutStatus::Completed,
            CheckoutStatus::Expired,
            CheckoutStatus::Cancelled,
        ] {
            assert_eq!(
                CheckoutStatus::from(CheckoutSessionStatus::from(status)),
                status
            );
        }
        assert_eq!(
            serde_json::to_value(CheckoutStatus::AwaitingPayment).unwrap(),
            "awaiting_payment"
        );
        assert_eq!(kind(CheckoutType::TopUp), None);
        assert_eq!(
            serde_json::to_value(kind(CheckoutType::AddonPurchase)).unwrap(),
            "add_on_purchase"
        );
    }
}
