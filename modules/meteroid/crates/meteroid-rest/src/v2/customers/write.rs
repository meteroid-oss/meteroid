//! Writing customers through API v2 (ADR-0013 §11): create, a merge patch applied to the customer
//! as written and stored by the full update the dashboard uses, archive and restore, and the
//! portal token. Each on the store call v1 uses.
use super::model::{
    Customer, CustomerInput, PortalToken, PortalTokenInput, ShippingAddress, UpdateCustomer,
};
use super::read::render;
use super::resources::resource_input;
use crate::AppState;
use crate::auth::DelegatedCredentials;
use crate::v2::extract::{Body, Path};
use crate::v2::idempotency::committed_write;
use crate::v2::ids;
use crate::v2::problem::{FieldError, Problem, ProblemCode};
use axum::Extension;
use axum::Json;
use axum::extract::State;
use axum::response::Response;
use common_domain::ids::{AliasOr, InvoicingEntityId, TenantId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use error_stack::Report;
use http::StatusCode;
use meteroid_store::domain::{self, CustomerNew, CustomerUpdate};
use meteroid_store::errors::StoreError;
use meteroid_store::jwt_claims::default_customer_portal_token_ttl;
use meteroid_store::repositories::portal_tokens::PortalTokensInterface;
use meteroid_store::repositories::{CustomersInterface, ResourcesInterface};
use std::str::FromStr;

/// The customer written, as v2 reads it; as the store returned it when it cannot be read back
/// (the write is made all the same: a write committed never answers an error).
async fn written(
    state: &AppState,
    tenant_id: TenantId,
    status: StatusCode,
    customer: domain::Customer,
) -> Response {
    let read = render(state, tenant_id, customer.id).await;
    committed_write(status, read, || {
        let updated_at = customer.updated_at.unwrap_or(customer.created_at).and_utc();
        Customer::new(customer, updated_at)
    })
}

/// A trimmed text, none when blank.
fn text(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/// What the store is handed, read off a customer as written, every field checked first.
struct Written {
    alias: Option<String>,
    name: String,
    currency: String,
    invoicing_entity_id: Option<InvoicingEntityId>,
    preferred_locales: Vec<String>,
    custom_properties: Option<serde_json::Value>,
}

fn checked(input: &CustomerInput) -> Result<Written, Problem> {
    if input
        .alias
        .as_deref()
        .is_some_and(|alias| alias.trim().is_empty())
    {
        return Err(Problem::field("/alias", "invalid", "An alias is not blank"));
    }
    let name = text(&input.name).unwrap_or_default();
    if input.kind == super::model::CustomerKind::Company && name.is_empty() {
        return Err(Problem::field("/name", "required", "A company has a name"));
    }
    crate::v2::bounds::name(&name, "/name")?;
    let currency = input.currency.trim().to_ascii_uppercase();
    if currency.len() != 3 {
        return Err(Problem::field(
            "/currency",
            "invalid",
            "An ISO 4217 currency code, like EUR",
        ));
    }
    let invoicing_entity_id = input
        .invoicing
        .entity
        .as_deref()
        .map(|raw| {
            InvoicingEntityId::from_str(raw.trim()).map_err(|_| {
                Problem::field(
                    "/invoicing/entity",
                    "invalid",
                    format!("Not an invoicing entity id: {raw}"),
                )
            })
        })
        .transpose()?;
    let preferred_locales: Vec<String> = input
        .invoicing
        .preferred_locales
        .iter()
        .map(|tag| tag.trim().to_owned())
        .filter(|tag| !tag.is_empty())
        .collect();
    meteroid_invoicing::lang::reject_unsupported_preferences(&preferred_locales).map_err(
        |tag| {
            Problem::field(
                "/invoicing/preferred_locales",
                "invalid",
                format!(
                    "No document is rendered in '{tag}'; one of {:?} at least",
                    meteroid_invoicing::lang::SUPPORTED
                ),
            )
        },
    )?;
    if input
        .custom_properties
        .as_ref()
        .is_some_and(|properties| !properties.is_object())
    {
        return Err(Problem::field(
            "/custom_properties",
            "invalid",
            "Custom properties are an object of values by key",
        ));
    }
    Ok(Written {
        alias: input.alias.as_deref().map(|alias| alias.trim().to_owned()),
        name,
        currency,
        invoicing_entity_id,
        preferred_locales,
        custom_properties: input.custom_properties.clone(),
    })
}

fn shipping(input: &Option<ShippingAddress>) -> Option<domain::ShippingAddress> {
    input.clone().map(Into::into)
}

fn tax_rates(input: &CustomerInput) -> Vec<domain::CustomerTaxRate> {
    input
        .tax
        .custom_rates
        .iter()
        .cloned()
        .map(Into::into)
        .collect()
}

fn emails(input: &CustomerInput) -> Vec<String> {
    input
        .invoicing
        .emails
        .iter()
        .map(|email| email.trim().to_owned())
        .filter(|email| !email.is_empty())
        .collect()
}

/// A unique violation on write read as the alias being taken.
fn alias_taken(alias: Option<String>) -> impl FnOnce(Report<StoreError>) -> Problem {
    move |error| {
        let problem = Problem::from(error);
        match (problem.code, alias) {
            (ProblemCode::AlreadyExists, Some(alias)) => {
                let message = format!("The alias '{alias}' names another customer");
                let mut taken = Problem::conflict(ProblemCode::AlreadyExists, message.clone());
                taken.errors.push(FieldError {
                    path: "/alias".to_owned(),
                    code: "duplicate".to_owned(),
                    message,
                });
                taken
            }
            (_, _) => problem,
        }
    }
}

/// Create a customer
#[utoipa::path(
    post,
    tag = "Customers",
    path = "/api/v2/customers",
    request_body = CustomerInput,
    responses(
        (status = 201, description = "Created", body = Customer),
        (status = 409, description = "The alias names another customer", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn create_customer(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Body(body): Body<CustomerInput>,
) -> Result<Response, Problem> {
    let written = checked(&body)?;
    let created = state
        .store
        .insert_customer(
            auth.as_actor(),
            CustomerNew {
                name: written.name,
                alias: written.alias.clone(),
                billing_email: text(&body.email),
                invoicing_emails: emails(&body),
                phone: text(&body.phone),
                currency: written.currency,
                billing_address: body.billing_address.clone().map(Into::into),
                shipping_address: shipping(&body.shipping_address),
                invoicing_entity_id: written.invoicing_entity_id,
                force_created_date: None,
                vat_number: text(&body.tax.vat_number),
                legal_number: text(&body.tax.legal_number),
                buyer_reference: text(&body.invoicing.buyer_reference),
                custom_taxes: tax_rates(&body),
                tax_status: body.tax.status.into(),
                exemption_reason: text(&body.tax.exemption_reason),
                connected_account_id: None,
                custom_properties: written.custom_properties.unwrap_or(serde_json::Value::Null),
                preferred_locales: written.preferred_locales,
                customer_type: body.kind.into(),
                first_name: text(&body.first_name),
                last_name: text(&body.last_name),
            },
            auth.tenant_id,
        )
        .await
        .map_err(alias_taken(written.alias))?;
    Ok(self::written(&state, auth.tenant_id, StatusCode::CREATED, created).await)
}

/// Applies a JSON merge patch (RFC 7396) to `target`.
fn merge_patch(target: &mut serde_json::Value, patch: &serde_json::Value) {
    let serde_json::Value::Object(patch) = patch else {
        *target = patch.clone();
        return;
    };
    if !target.is_object() {
        *target = serde_json::Value::Object(serde_json::Map::new());
    }
    if let serde_json::Value::Object(target) = target {
        for (key, value) in patch {
            if value.is_null() {
                target.remove(key);
            } else {
                merge_patch(
                    target.entry(key.clone()).or_insert(serde_json::Value::Null),
                    value,
                );
            }
        }
    }
}

/// Update a customer
///
/// A JSON merge patch of the customer as written (`POST /customers`): a field left out is
/// unchanged, `null` clears it, an object (an address, `tax`, `invoicing`,
/// `custom_properties`) is merged field by field and a list is replaced.
#[utoipa::path(
    patch,
    tag = "Customers",
    path = "/api/v2/customers/{id_or_alias}",
    params(("id_or_alias" = String, Path, description = "Customer id, or its alias as `ext:<alias>`")),
    request_body(content = UpdateCustomer, content_type = "application/merge-patch+json"),
    responses(
        (status = 200, description = "Updated", body = Customer),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "The alias names another customer", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn update_customer(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_alias): Path<String>,
    Body(patch): Body<serde_json::Value>,
) -> Result<Response, Problem> {
    if !patch.is_object() {
        return Err(Problem::invalid_request(
            "A merge patch of a customer is a JSON object",
        ));
    }
    // Refuses a field of the wrong type before anything is read.
    crate::v2::extract::from_value::<UpdateCustomer>(patch.clone())?;
    let existing = ids::customer(&state, auth.tenant_id, &id_or_alias).await?;
    let mut target =
        serde_json::to_value(CustomerInput::of(&existing)).map_err(|_| Problem::internal())?;
    merge_patch(&mut target, &patch);
    let body: CustomerInput = serde_json::from_value(target).map_err(|error| {
        Problem::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            ProblemCode::ValidationFailed,
            format!("The customer patched is refused: {error}"),
        )
    })?;
    let written = checked(&body)?;
    let updated = state
        .store
        .update_customer(
            auth.as_actor(),
            auth.tenant_id,
            CustomerUpdate {
                id_or_alias: AliasOr::Id(existing.id),
                name: written.name,
                alias: written.alias.clone(),
                billing_email: text(&body.email),
                invoicing_emails: emails(&body),
                phone: text(&body.phone),
                currency: written.currency,
                billing_address: body.billing_address.clone().map(Into::into),
                shipping_address: shipping(&body.shipping_address),
                invoicing_entity_id: written
                    .invoicing_entity_id
                    .unwrap_or(existing.invoicing_entity_id),
                vat_number: text(&body.tax.vat_number),
                legal_number: text(&body.tax.legal_number),
                buyer_reference: text(&body.invoicing.buyer_reference),
                custom_taxes: tax_rates(&body),
                tax_status: body.tax.status.into(),
                exemption_reason: text(&body.tax.exemption_reason),
                // The whole bag as patched: a property cleared is gone.
                custom_properties: written
                    .custom_properties
                    .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new())),
                preferred_locales: written.preferred_locales,
                customer_type: body.kind.into(),
                first_name: text(&body.first_name),
                last_name: text(&body.last_name),
            },
        )
        .await
        .map_err(alias_taken(written.alias))?;
    Ok(self::written(&state, auth.tenant_id, StatusCode::OK, updated).await)
}

/// A refusal of the customer's state (live subscriptions) reads as a conflict.
fn state_refusal(error: Report<StoreError>) -> Problem {
    let problem = Problem::from(error);
    if problem.code == ProblemCode::ValidationFailed {
        Problem::conflict(ProblemCode::Conflict, problem.detail.unwrap_or_default())
    } else {
        problem
    }
}

/// Archive a customer
///
/// Nothing linked to it is deleted. A customer with a subscription that is active, in trial or
/// waiting for its activation is refused: cancel them first.
#[utoipa::path(
    post,
    tag = "Customers",
    path = "/api/v2/customers/{id_or_alias}/archive",
    params(("id_or_alias" = String, Path, description = "Customer id, or its alias as `ext:<alias>`")),
    responses(
        (status = 200, description = "Archived", body = Customer),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "The customer has live subscriptions", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn archive_customer(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_alias): Path<String>,
) -> Result<Response, Problem> {
    let mut customer = ids::customer(&state, auth.tenant_id, &id_or_alias).await?;
    if customer.archived_at.is_none() {
        state
            .store
            .archive_customer(auth.as_actor(), auth.tenant_id, AliasOr::Id(customer.id))
            .await
            .map_err(state_refusal)?;
        customer.archived_at = Some(common_clock::now().naive_utc());
    }
    Ok(written(&state, auth.tenant_id, StatusCode::OK, customer).await)
}

/// Restore an archived customer
#[utoipa::path(
    post,
    tag = "Customers",
    path = "/api/v2/customers/{id_or_alias}/unarchive",
    params(("id_or_alias" = String, Path, description = "Customer id, or its alias as `ext:<alias>`")),
    responses(
        (status = 200, description = "Restored", body = Customer),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn unarchive_customer(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_alias): Path<String>,
) -> Result<Response, Problem> {
    let mut customer = ids::customer(&state, auth.tenant_id, &id_or_alias).await?;
    if customer.archived_at.is_some() {
        state
            .store
            .unarchive_customer(auth.as_actor(), auth.tenant_id, AliasOr::Id(customer.id))
            .await
            .map_err(state_refusal)?;
        customer.archived_at = None;
    }
    Ok(written(&state, auth.tenant_id, StatusCode::OK, customer).await)
}

/// Issue a portal token
///
/// A token opening the customer portal, or reading the customer's billing state from a browser
/// with the read scope. The body may be left out: a day, every scope.
#[utoipa::path(
    post,
    tag = "Customers",
    path = "/api/v2/customers/{id_or_alias}/portal-token",
    params(("id_or_alias" = String, Path, description = "Customer id, or its alias as `ext:<alias>`")),
    request_body(content = PortalTokenInput, content_type = "application/json"),
    responses(
        (status = 201, description = "Issued", body = PortalToken),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn create_portal_token(
    Extension(auth): Extension<AuthorizedAsTenant>,
    delegated: Option<Extension<DelegatedCredentials>>,
    State(state): State<AppState>,
    Path(id_or_alias): Path<String>,
    body: axum::body::Bytes,
) -> Result<(StatusCode, Json<PortalToken>), Problem> {
    let input: PortalTokenInput = if body.iter().all(u8::is_ascii_whitespace) {
        PortalTokenInput::default()
    } else {
        crate::v2::extract::read_body(&body)?
    };
    let ttl = match input.expires_in {
        Some(seconds) if !(60..=2_592_000).contains(&seconds) => {
            return Err(Problem::field(
                "/expires_in",
                "invalid",
                "A token lives 60 seconds to 30 days (2592000)",
            ));
        }
        Some(seconds) => chrono::Duration::seconds(i64::from(seconds)),
        None => default_customer_portal_token_ttl(),
    };
    if input.scopes.as_ref().is_some_and(Vec::is_empty) {
        return Err(Problem::field(
            "/scopes",
            "invalid",
            "A token has a scope at least; leave scopes out for every one",
        ));
    }
    let customer = ids::customer(&state, auth.tenant_id, &id_or_alias).await?;
    let claims = resource_input(
        &state,
        auth.tenant_id,
        customer.id,
        input.resource.as_ref(),
        input.attributes,
        "/resource",
    )
    .await?;
    state
        .store
        .resolve_resource_input(auth.tenant_id, customer.id, &claims)
        .await
        .map_err(|error| Problem::at("/attributes", error))?;
    let issued = state
        .store
        .customer_portal_token(
            auth.tenant_id,
            customer.id,
            ttl,
            input
                .scopes
                .map(|scopes| scopes.into_iter().map(Into::into).collect()),
            delegated.is_some(),
            claims.into(),
        )
        .await?;
    let expires_at = issued.expires_at.ok_or_else(Problem::internal)?;
    let token = issued.token;
    Ok((
        StatusCode::CREATED,
        Json(PortalToken {
            url: format!(
                "{}/portal/customer?token={token}",
                state.portal_url.trim_end_matches('/')
            ),
            token,
            expires_at: expires_at.into(),
            api_url: state.api_url.clone(),
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::merge_patch;
    use serde_json::json;

    #[test]
    fn a_merge_patch_merges_objects_clears_nulls_and_replaces_lists() {
        let mut target = json!({
            "name": "Acme",
            "email": "billing@acme.test",
            "billing_address": {"line1": "1 rue", "city": "Paris", "country": "FR"},
            "invoicing": {"emails": ["a@acme.test"], "preferred_locales": ["fr-FR"]},
            "custom_properties": {"tier": "gold", "region": "eu"}
        });
        merge_patch(
            &mut target,
            &json!({
                "email": null,
                "billing_address": {"city": "Lyon"},
                "invoicing": {"emails": ["b@acme.test"]},
                "custom_properties": {"tier": null, "segment": "smb"}
            }),
        );
        assert_eq!(
            target,
            json!({
                "name": "Acme",
                "billing_address": {"line1": "1 rue", "city": "Lyon", "country": "FR"},
                "invoicing": {"emails": ["b@acme.test"], "preferred_locales": ["fr-FR"]},
                "custom_properties": {"region": "eu", "segment": "smb"}
            })
        );
    }
}
