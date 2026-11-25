//! `PATCH /api/v2/subscriptions/{id_or_external_id}`: a JSON merge patch of what a subscription
//! holds beside its billing — its invoicing terms and custom properties, on the store's patch v1
//! uses, its external id — and of its end date, on the store's end date service (R-140), written
//! in one transaction (`update_subscription`). A field left out is unchanged, `null` clears it.
use super::model::Subscription;
use super::render;
use crate::AppState;
use crate::v2::extract::{Body, Path};
use crate::v2::ids;
use crate::v2::problem::{FieldError, Problem, ProblemCode};
use axum::Extension;
use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use chrono::NaiveDate;
use common_domain::ids::{SubscriptionId, TenantId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use error_stack::Report;
use meteroid_store::domain::{SubscriptionPatch, SubscriptionUpdate, SubscriptionUpdatePart};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::CatalogCodesInterface;
use serde::Deserialize;
use utoipa::ToSchema;

/// How the subscription is invoiced from now on: a field left out is unchanged.
#[derive(Deserialize, ToSchema, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct InvoicingPatch {
    /// Days an invoice is due after it is issued.
    #[serde(default)]
    pub net_terms: Option<u32>,
    /// `null` clears it.
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub purchase_order: Option<Option<String>>,
    /// Printed on every invoice; `null` clears it.
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub memo: Option<Option<String>>,
    #[serde(default)]
    pub auto_finalize: Option<bool>,
    /// True needs a payment provider on the invoicing entity.
    #[serde(default)]
    pub charge_automatically: Option<bool>,
}

/// A merge patch: a field left out is unchanged, `null` clears it.
#[derive(Deserialize, ToSchema, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct UpdateSubscription {
    /// The integrator's own id, unique per tenant; `null` clears it.
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub external_id: Option<Option<String>>,
    #[serde(default)]
    pub invoicing: Option<InvoicingPatch>,
    /// The day it stops renewing and completes; `null`: it renews until cancelled. Only what is
    /// not billed yet moves: a date after today that cuts no period already billed (a date inside
    /// one is a cancellation on that date: `POST …/cancel`). Refused while a cancellation is
    /// scheduled, or a change on or after the date.
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>, format = Date, example = "2027-01-01")]
    pub end_date: Option<Option<NaiveDate>>,
    /// Merged into the subscription's, validated against the tenant's custom property
    /// definitions.
    #[serde(default)]
    #[schema(value_type = Option<HashMap<String, Value>>)]
    pub custom_properties: Option<serde_json::Value>,
}

fn taken(external_id: &str) -> Problem {
    let mut problem = Problem::conflict(
        ProblemCode::CodeTaken,
        format!("Another subscription has the external id '{external_id}'"),
    );
    problem.errors.push(FieldError {
        path: "/external_id".to_owned(),
        code: "duplicate".to_owned(),
        message: format!("Another subscription has the external id '{external_id}'"),
    });
    problem
}

/// The external id asked, checked unlike an id and free but for this subscription's own;
/// `None` clears it.
async fn external_id(
    state: &AppState,
    tenant_id: TenantId,
    id: SubscriptionId,
    asked: Option<&str>,
) -> Result<Option<String>, Problem> {
    let Some(asked) = asked else {
        return Ok(None);
    };
    let asked = asked.trim();
    if asked.is_empty() || asked.len() > 255 {
        return Err(Problem::field(
            "/external_id",
            "invalid",
            "An external id is 1 to 255 bytes",
        ));
    }
    if ids::reads_as_subscription_id(asked) {
        return Err(Problem::field(
            "/external_id",
            "invalid",
            "An external id never reads as a subscription id (a UUID or `sub_…`)",
        ));
    }
    match state
        .store
        .find_subscription_id_by_external_id(tenant_id, asked)
        .await
    {
        Ok(holder) if holder == id => Ok(Some(asked.to_owned())),
        Ok(_) => Err(taken(asked)),
        Err(error) if matches!(error.current_context(), StoreError::ValueNotFound(_)) => {
            Ok(Some(asked.to_owned()))
        }
        Err(error) => Err(error.into()),
    }
}

/// What refused the update, at the field it refuses: the end date's, or the external id's (taken
/// by a write racing this one), else the store's error.
fn update_refused(error: Report<StoreError>, external_id: Option<&str>) -> Problem {
    let part = error
        .frames()
        .find_map(|frame| frame.downcast_ref::<SubscriptionUpdatePart>())
        .copied();
    match part {
        Some(SubscriptionUpdatePart::ExternalId)
            if matches!(error.current_context(), StoreError::DuplicateValue { .. }) =>
        {
            taken(external_id.unwrap_or_default())
        }
        Some(SubscriptionUpdatePart::ExternalId) => Problem::at("/external_id", error),
        Some(SubscriptionUpdatePart::EndDate) => Problem::at("/end_date", error),
        None => error.into(),
    }
}

/// An empty text clears it, as v1 reads it.
fn cleared_if_empty(value: Option<Option<String>>) -> Option<Option<String>> {
    value.map(|text| {
        text.map(|text| text.trim().to_owned())
            .filter(|text| !text.is_empty())
    })
}

/// Update a subscription
///
/// A JSON merge patch of its invoicing terms, custom properties (merged into its own), external
/// id and end date: a field left out is unchanged, `null` clears it. The end date moves over what
/// is not billed yet only (a date cutting a period billed is a cancellation on that date). All of
/// it is written in one transaction: a refused field writes nothing. Its plan, charges and seats
/// change through the change endpoints, each with its preview.
#[utoipa::path(
    patch,
    tag = "Subscriptions",
    path = "/api/v2/subscriptions/{id_or_external_id}",
    params(("id_or_external_id" = String, Path, description = "Subscription id or external id")),
    request_body(content = UpdateSubscription, content_type = "application/merge-patch+json"),
    responses(
        (status = 200, description = "Updated", body = Subscription),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "The external id is another subscription's, or the subscription's state refuses its end date: ended, a cancellation scheduled, a change scheduled on or after the date", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused: a custom property not defined, automatic charging without a payment provider, a subscription ended, an end date not after today or cutting a period already billed…", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn update_subscription(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_external_id): Path<String>,
    Body(body): Body<UpdateSubscription>,
) -> Result<Response, Problem> {
    let tenant_id = auth.tenant_id;
    let id = ids::subscription_id(&state, tenant_id, &id_or_external_id).await?;
    // Checked before the store, at its field; the store checks it free again under its write.
    let external_id = match &body.external_id {
        Some(asked) => Some(external_id(&state, tenant_id, id, asked.as_deref()).await?),
        None => None,
    };

    let invoicing = body.invoicing.unwrap_or_default();
    let patch = SubscriptionPatch {
        id,
        charge_automatically: invoicing.charge_automatically,
        auto_advance_invoices: invoicing.auto_finalize,
        net_terms: invoicing.net_terms,
        invoice_memo: cleared_if_empty(invoicing.memo),
        purchase_order: cleared_if_empty(invoicing.purchase_order),
        payment_methods_config: None,
        custom_properties: body.custom_properties,
        usage_filter: None,
    };
    let patches_row = patch.charge_automatically.is_some()
        || patch.auto_advance_invoices.is_some()
        || patch.net_terms.is_some()
        || patch.invoice_memo.is_some()
        || patch.purchase_order.is_some()
        || patch.custom_properties.is_some();
    let asked_id = external_id.clone().flatten();
    state
        .services
        .update_subscription(
            auth.as_actor(),
            tenant_id,
            id,
            SubscriptionUpdate {
                end_date: body.end_date,
                patch: patches_row.then_some(patch),
                external_id,
            },
        )
        .await
        .map_err(|error| update_refused(error, asked_id.as_deref()))?;

    // Written: the patch answers what it did, whatever is read back.
    let body = match render::subscription(&state, tenant_id, id).await {
        Ok(subscription) => serde_json::to_value(subscription),
        Err(problem) => {
            log::warn!(
                "Subscription {id} was updated but is not read back whole in v2: {problem:?}"
            );
            match render::summary(&state, tenant_id, id).await {
                Ok(summary) => serde_json::to_value(summary),
                Err(_) => Ok(serde_json::json!({"id": id.to_string()})),
            }
        }
    }
    .unwrap_or_else(|_| serde_json::json!({"id": id.to_string()}));
    Ok(Json(body).into_response())
}
