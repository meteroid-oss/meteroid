//! `/api/v2/subscriptions/{id_or_external_id}/trial`: a running trial extended, or ended today
//! with its end action (R-153), and what ending it would do.
use super::changes::{left_out, require_key, today};
use super::model::{DocumentType, ExtendTrial, Subscription, SubscriptionStatus, TrialEndPreview};
use super::render::{self, AfterChange};
use crate::AppState;
use crate::v2::extract::{Body, Path};
use crate::v2::idempotency::committed_write;
use crate::v2::ids;
use crate::v2::problem::{Problem, ProblemCode};
use crate::v2::trials::{self, TrialOutcome, TrialStatus};
use axum::Extension;
use axum::extract::State;
use axum::response::Response;
use common_domain::ids::{SubscriptionId, TenantId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::{HeaderMap, StatusCode};
use meteroid_store::domain::subscription_changes::PlanChangeMode;
use meteroid_store::domain::trials::{TrialEndAction, TrialMove, TrialWithoutCard};
use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

/// The subscription once its trial moved, read whole (a write committed answers what it did).
async fn moved(
    state: &AppState,
    auth: &AuthorizedAsTenant,
    id: SubscriptionId,
    to: TrialMove,
    pointer: Option<&str>,
) -> Result<Response, Problem> {
    state
        .services
        .move_trial(auth.as_actor(), auth.tenant_id, id, to)
        .await
        .map_err(|error| match pointer {
            Some(pointer) => Problem::at(pointer, error),
            None => Problem::from(error),
        })?;
    Ok(committed_write(
        StatusCode::OK,
        render::subscription(state, auth.tenant_id, id).await,
        || serde_json::json!({"id": id.to_string()}),
    ))
}

/// Extend a subscription's trial
///
/// Its end moves later, to `until` or by `days`, and what follows it with it: a trial cancelling
/// at its end cancels on the new one, a trial switching plan switches then, billing starts on the
/// new end. A trial awaiting a decision is extended from today. Refused while other changes are
/// scheduled from its current end.
#[utoipa::path(
    post,
    tag = "Subscriptions",
    path = "/api/v2/subscriptions/{id_or_external_id}/trial/extend",
    operation_id = "extend_trial",
    params(("id_or_external_id" = String, Path, description = "Subscription id or external id")),
    request_body(content = ExtendTrial, example = json!(crate::v2::examples::request("extend_trial"))),
    responses(
        (status = 200, description = "The subscription, its trial extended", body = Subscription, example = json!(crate::v2::examples::response("extend_trial"))),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "Not in trial, an end before the current one, or changes scheduled from it", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn extend_trial(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_external_id): Path<String>,
    Body(body): Body<ExtendTrial>,
) -> Result<Response, Problem> {
    let (to, pointer) = match (body.until, body.days) {
        (Some(day), None) => (TrialMove::Until(day), "/until"),
        (None, Some(days)) if days > 0 => (TrialMove::Days(days), "/days"),
        (None, Some(_)) => {
            return Err(Problem::field("/days", "invalid", "At least one day"));
        }
        _ => {
            return Err(Problem::field(
                "/until",
                "required",
                "Set `until` or `days`, one of the two",
            ));
        }
    };
    let id = ids::subscription_id(&state, auth.tenant_id, &id_or_external_id).await?;
    moved(&state, &auth, id, to, Some(pointer)).await
}

/// End a subscription's trial now
///
/// The trial ends today and its end action applies now: a converting trial starts billing today,
/// its opening invoice issued; one cancelling at its end ends the subscription today; one
/// switching plan switches today; one awaiting a decision converts. An `Idempotency-Key` is
/// required.
#[utoipa::path(
    post,
    tag = "Subscriptions",
    path = "/api/v2/subscriptions/{id_or_external_id}/trial/end",
    operation_id = "end_trial",
    params(("id_or_external_id" = String, Path, description = "Subscription id or external id")),
    responses(
        (status = 200, description = "The subscription, its trial ended today", body = Subscription, example = json!(crate::v2::examples::response("end_trial"))),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "Not in trial", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn end_trial(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id_or_external_id): Path<String>,
) -> Result<Response, Problem> {
    require_key(&headers, "Ending a trial")?;
    let id = ids::subscription_id(&state, auth.tenant_id, &id_or_external_id).await?;
    moved(&state, &auth, id, TrialMove::Now, None).await
}

/// What ending the trial today does: its end action, a manual one converting, and for a
/// conversion of a subscription charged automatically without a payment method what its trial
/// says then.
fn ending_now(
    action: TrialEndAction,
    charged_without_method: bool,
    without_card: TrialWithoutCard,
) -> TrialEndAction {
    match action {
        TrialEndAction::Convert | TrialEndAction::Manual if charged_without_method => {
            match without_card {
                TrialWithoutCard::AwaitPayment => TrialEndAction::Convert,
                TrialWithoutCard::Cancel => TrialEndAction::Cancel,
                TrialWithoutCard::Switch { plan_version_id } => {
                    TrialEndAction::Fallback { plan_version_id }
                }
            }
        }
        TrialEndAction::Manual => TrialEndAction::Convert,
        other => other,
    }
}

async fn preview(
    state: &AppState,
    tenant_id: TenantId,
    id: SubscriptionId,
) -> Result<TrialEndPreview, Problem> {
    let details = state.store.get_subscription_details(tenant_id, id).await?;
    let Some(recorded) = trials::recorded_trial(state, tenant_id, &details).await? else {
        return Err(Problem::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            ProblemCode::ValidationFailed,
            "The subscription has no trial",
        ));
    };
    let status = recorded.status();
    if !matches!(status, TrialStatus::Active | TrialStatus::AwaitingDecision) {
        return Err(Problem::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            ProblemCode::ValidationFailed,
            format!("The trial ended on {}", recorded.trial.until),
        ));
    }
    let effective_date = today().max(recorded.trial.from);
    let charged_without_method = details.subscription.charge_automatically
        && !trials::has_payment_method(state, tenant_id, &details).await?;
    let action = ending_now(
        recorded.end_action(&details),
        charged_without_method,
        recorded.without_card(),
    );
    let currency = details.subscription.currency.clone();
    let (outcome, documents, missing, subscription_after) = match action {
        TrialEndAction::Convert | TrialEndAction::Manual => {
            let invoice = trials::opening_invoice(state, &details, effective_date).await?;
            let documents = render::change_documents(
                state,
                tenant_id,
                &[(DocumentType::Invoice, &invoice)],
                &currency,
            )
            .await?;
            let outcome = if charged_without_method {
                TrialOutcome::AwaitingPayment
            } else {
                TrialOutcome::Converted
            };
            let after = render::after_change(
                state,
                tenant_id,
                &details,
                &details,
                AfterChange {
                    status: Some(if charged_without_method {
                        SubscriptionStatus::TrialExpired
                    } else {
                        SubscriptionStatus::Active
                    }),
                    ..AfterChange::default()
                },
            )
            .await?;
            (outcome, documents, Vec::new(), after)
        }
        TrialEndAction::Cancel => {
            let after = render::after_change(
                state,
                tenant_id,
                &details,
                &details,
                AfterChange {
                    status: Some(SubscriptionStatus::Cancelled),
                    mrr: Some(0),
                    ..AfterChange::default()
                },
            )
            .await?;
            (TrialOutcome::Cancelled, Vec::new(), Vec::new(), after)
        }
        TrialEndAction::Fallback { plan_version_id } => {
            let full = state
                .services
                .preview_plan_change_in_full(
                    id,
                    tenant_id,
                    plan_version_id,
                    Vec::new(),
                    Some(PlanChangeMode::Immediate),
                )
                .await?;
            let documents =
                render::plan_change_documents(state, tenant_id, &full.outcome, &currency).await?;
            let change = AfterChange {
                plan: Some(trials::version_ref(state, tenant_id, plan_version_id).await?),
                mrr: full.outcome.mrr_after_cents,
                ..AfterChange::default()
            };
            let after = match &full.outcome.subscription_after {
                Some(after) => {
                    render::after_change(state, tenant_id, &details, after, change).await?
                }
                None => render::after_change(state, tenant_id, &details, &details, change).await?,
            };
            (
                TrialOutcome::Switched,
                documents,
                left_out(&full.outcome.missing),
                after,
            )
        }
    };
    Ok(TrialEndPreview {
        effective_date,
        outcome,
        documents,
        incomplete: !missing.is_empty(),
        missing,
        subscription_after,
    })
}

/// Preview ending a subscription's trial now
///
/// What `POST …/trial/end` would do today, nothing written: how the trial ends, the invoice it
/// issues (the opening invoice of a conversion, priced as the checkout prices it), and the
/// subscription it leaves.
#[utoipa::path(
    post,
    tag = "Subscriptions",
    path = "/api/v2/subscriptions/{id_or_external_id}/trial/end/preview",
    operation_id = "preview_trial_end",
    params(("id_or_external_id" = String, Path, description = "Subscription id or external id")),
    responses(
        (status = 200, description = "What ending the trial today would do", body = TrialEndPreview, example = json!(crate::v2::examples::response("preview_trial_end"))),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "Not in trial", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn preview_trial_end(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_external_id): Path<String>,
) -> Result<axum::Json<TrialEndPreview>, Problem> {
    let id = ids::subscription_id(&state, auth.tenant_id, &id_or_external_id).await?;
    Ok(axum::Json(preview(&state, auth.tenant_id, id).await?))
}

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(extend_trial))
        .routes(routes!(end_trial))
        .routes(routes!(preview_trial_end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use common_domain::ids::{BaseId, PlanVersionId};

    #[test]
    fn ending_now_applies_the_end_action_a_manual_one_converting() {
        let switch = PlanVersionId::new();
        let card = TrialWithoutCard::Cancel;
        assert_eq!(
            ending_now(TrialEndAction::Manual, false, card),
            TrialEndAction::Convert
        );
        assert_eq!(
            ending_now(TrialEndAction::Cancel, true, card),
            TrialEndAction::Cancel
        );
        assert_eq!(
            ending_now(TrialEndAction::Convert, true, card),
            TrialEndAction::Cancel
        );
        assert_eq!(
            ending_now(
                TrialEndAction::Convert,
                true,
                TrialWithoutCard::Switch {
                    plan_version_id: switch
                }
            ),
            TrialEndAction::Fallback {
                plan_version_id: switch
            }
        );
        assert_eq!(
            ending_now(
                TrialEndAction::Convert,
                true,
                TrialWithoutCard::AwaitPayment
            ),
            TrialEndAction::Convert
        );
    }
}
