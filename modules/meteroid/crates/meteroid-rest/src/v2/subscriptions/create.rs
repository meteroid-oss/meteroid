//! `POST /api/v2/subscriptions`: a subscription created on the store's create path, every code
//! of the request resolved first — the customer, the plan version, its option and currency, the
//! seat counts, levels, price overrides, own charges, add-ons and coupons. A checkout session
//! resolves its purchase the same way (`resolve_purchase`).
use super::changes::require_key;
use super::model::{AddOnPurchase, CreateCharges, CreateSubscription, Subscription, TrialInput};
use super::render;
use super::resolve::{
    cells, count, counts, not_found_at, option_named, override_price, own_charge, plan_charge,
    plan_stored, published_version, sale, seats,
};
use crate::AppState;
use crate::v2::bounds;
use crate::v2::customers::resources::stored_resource_id;
use crate::v2::entitlements::model::{EntitlementInput, granted_feature, spec_of};
use crate::v2::extract::Body;
use crate::v2::idempotency::committed_write;
use crate::v2::ids;
use crate::v2::problem::{FieldError, Problem, ProblemCode, store_message};
use axum::Extension;
use axum::extract::State;
use axum::response::Response;
use common_domain::ids::{CouponId, ManagedQuantityId, TenantId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::{HeaderMap, StatusCode};
use meteroid_app::features::ensure_credit_pricing_allowed;
use meteroid_store::domain::entitlement_fold::Tier;
use meteroid_store::domain::entitlements::EntitlementSpec;
use meteroid_store::domain::enums::BillingPeriodEnum;
use meteroid_store::domain::plan_options::{PlanOption, PlanOptionRef};
use meteroid_store::domain::price_cells::OptionPricing;
use meteroid_store::domain::price_components::{PriceEntry, ProductRef};
use meteroid_store::domain::prices::FeeStructure;
use meteroid_store::domain::subscription_add_ons::{
    CreateSubscriptionAddOn, CreateSubscriptionAddOns, SubscriptionAddOnCustomization,
    SubscriptionAddOnParameterization,
};
use meteroid_store::domain::usage_levels::settle_levels;
use meteroid_store::domain::{
    ComponentOverride, ComponentParameterization, ComponentParameters,
    CreateSubscription as DomainCreate, CreateSubscriptionComponents, CreateSubscriptionCoupon,
    CreateSubscriptionCoupons, Customer, ExtraComponent, FullPlan, SubscriptionNew,
};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::CatalogCodesInterface;
use meteroid_store::repositories::ResourcesInterface;
use meteroid_store::repositories::add_ons::AddOnInterface;
use meteroid_store::repositories::coupons::CouponInterface;
use std::collections::{BTreeMap, HashMap, HashSet};

/// The integrator's id, checked free and unlike an id.
async fn external_id(
    state: &AppState,
    auth: &AuthorizedAsTenant,
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
        .find_subscription_id_by_external_id(auth.tenant_id, asked)
        .await
    {
        Ok(_) => Err(taken(asked)),
        Err(error) if matches!(error.current_context(), StoreError::ValueNotFound(_)) => {
            Ok(Some(asked.to_owned()))
        }
        Err(error) => Err(error.into()),
    }
}

/// A subscription's own rows (the subscription tier), each checked at `pointer/<i>`; two rows of
/// one feature, level and condition are refused.
pub(crate) async fn override_specs(
    state: &AppState,
    tenant_id: TenantId,
    inputs: &[EntitlementInput],
    pointer: &str,
) -> Result<Vec<EntitlementSpec>, Problem> {
    bounds::count(
        inputs.len(),
        bounds::MAX_ENTITLEMENTS,
        pointer,
        "entitlements",
    )?;
    if inputs.is_empty() {
        return Ok(Vec::new());
    }
    let types = state.store.list_resource_types(tenant_id, true).await?;
    let mut keys = HashSet::new();
    let mut specs = Vec::with_capacity(inputs.len());
    for (index, input) in inputs.iter().enumerate() {
        let at = format!("{pointer}/{index}");
        let feature =
            granted_feature(state, tenant_id, &input.feature, &format!("{at}/feature")).await?;
        let spec = spec_of(input, &feature, &types, Tier::Subscription, false, &at)?;
        let key = (
            spec.feature_id,
            format!("{:?}", spec.level),
            spec.condition
                .as_ref()
                .map(|condition| serde_json::to_string(&condition.0).unwrap_or_default()),
        );
        if !keys.insert(key) {
            return Err(Problem::field(
                format!("{at}/feature"),
                "duplicate",
                "Another row has the same feature, level and condition",
            ));
        }
        specs.push(spec);
    }
    Ok(specs)
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

/// Create a subscription
///
/// The customer (its id, or `ext:<alias>`) on a plan (by code or id): its option by alias, seat
/// counts by managed quantity code, usage levels by product code, price overrides, charges of its
/// own, add-ons and coupons by code. It is sold in its customer's currency when the plan sells it,
/// else in the plan's; a `currency` sent is checked against it. An `Idempotency-Key` is required:
/// a retry never creates a second subscription.
#[utoipa::path(
    post,
    tag = "Subscriptions",
    path = "/api/v2/subscriptions",
    request_body(content = CreateSubscription, example = json!(crate::v2::examples::request("create_subscription"))),
    responses(
        (status = 201, description = "Created", body = Subscription, example = json!(crate::v2::examples::response("create_subscription"))),
        (status = 403, description = "A catalog offer is asked and the plan does not include the offer catalog (`feature_not_available`)", body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "The customer is not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "The external id is taken (`code_taken`)", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused: an unknown option, a quantity no seat charge counts, a level the charge does not sell, a resource not the customer's or sold to another subscription over the term…; an offer that is not the catalog's (`offer_unavailable`) or the customer already had (`offer_already_given`); or no Idempotency-Key was sent", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn create_subscription(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Body(body): Body<CreateSubscription>,
) -> Result<Response, Problem> {
    require_key(&headers, "A subscription")?;
    let tenant_id = auth.tenant_id;
    check_terms(Terms {
        billing_day_anchor: body.billing_day_anchor,
        net_terms: body.invoicing.net_terms,
    })?;
    let start_date = body
        .start_date
        .unwrap_or_else(|| common_clock::now().date_naive());
    let offer =
        crate::v2::offers::offer_request(&state, &auth, body.offer.as_ref(), "/offer").await?;
    let trial = trial_request(&state, tenant_id, body.trial.as_ref(), start_date, "/trial").await?;
    let external_id = external_id(&state, &auth, body.external_id.as_deref()).await?;
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
    let entitlements = override_specs(
        &state,
        tenant_id,
        &body.entitlement_overrides,
        "/entitlement_overrides",
    )
    .await?;
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
    let create = DomainCreate {
        subscription: SubscriptionNew {
            customer_id: customer.id,
            plan_version_id: purchase.fp.version.id,
            net_terms: invoicing.net_terms,
            invoice_memo: invoicing.memo,
            invoice_threshold: None,
            start_date,
            end_date: body.end_date,
            billing_start_date: None,
            activation_condition: body.activation.into(),
            offer,
            trial: trial.clone(),
            billing_day_anchor: body.billing_day_anchor,
            payment_methods_config: None,
            auto_advance_invoices: invoicing.auto_finalize.unwrap_or(true),
            charge_automatically: invoicing.charge_automatically.unwrap_or(true),
            purchase_order: invoicing.purchase_order,
            backdate_invoices: false,
            skip_checkout_session: false,
            skip_past_invoices: false,
            custom_properties: body.custom_properties.unwrap_or(serde_json::Value::Null),
            // Checked under the customer's lock: the customer's own, live, unsold over the term.
            resource_id,
            // Written with the row: a create racing another on it fails whole.
            external_id: external_id.clone(),
        },
        // Named explicitly, so the store sells the option and currency checked here.
        option: purchase
            .option
            .as_ref()
            .map(|option| PlanOptionRef::Id(option.id)),
        price_components: purchase.components,
        add_ons: (!purchase.add_ons.is_empty()).then_some(CreateSubscriptionAddOns {
            add_ons: purchase.add_ons,
        }),
        coupons: (!purchase.coupons.is_empty()).then(|| CreateSubscriptionCoupons {
            coupons: purchase
                .coupons
                .into_iter()
                .map(|coupon_id| CreateSubscriptionCoupon { coupon_id })
                .collect(),
        }),
        // Written in the subscription's own transaction.
        entitlements,
        minimum_override: None,
        drop_minimum: false,
    };

    let created = match state
        .services
        .insert_subscription(auth.as_actor(), create, tenant_id)
        .await
    {
        Ok(created) => created,
        Err(error) => {
            if let Some(problem) = crate::v2::trials::trial_refusal(&error, &trial, "/trial") {
                return Err(problem);
            }
            return Err(insert_refused(&state, &auth, external_id.as_deref(), error).await);
        }
    };
    if let Err(error) = state
        .services
        .mark_sold_via(tenant_id, &[created.id], "API")
        .await
    {
        log::warn!(
            "subscription {} sold via the API not recorded: {error:?}",
            created.id
        );
    }
    // Written: a retry replays this answer, never a second subscription, whatever is read back.
    // Whole, else as lists read it, else by its id alone.
    let id = created.id;
    let warnings = created.warnings;
    let metrics_not_attributed: Vec<String> = created
        .metrics_not_attributed
        .into_iter()
        .map(|m| m.catalog_code)
        .collect();
    let read = match render::subscription(&state, tenant_id, id).await {
        Ok(subscription) => Ok(subscription),
        Err(problem) => {
            log::warn!(
                "Subscription {id} was created but is not read back whole in v2: {problem:?}"
            );
            render::summary(&state, tenant_id, id).await
        }
    }
    .map(|subscription| Subscription {
        warnings,
        metrics_not_attributed,
        ..subscription
    });
    Ok(committed_write(
        StatusCode::CREATED,
        read,
        || serde_json::json!({"id": id.to_string()}),
    ))
}

/// The trial asked at `pointer` for a subscription billed from `billing_start`, as the store
/// resolves it (R-153): the plan's when left out, unless the customer already had it.
pub(crate) async fn trial_request(
    state: &AppState,
    tenant_id: common_domain::ids::TenantId,
    input: Option<&TrialInput>,
    billing_start: chrono::NaiveDate,
    pointer: &str,
) -> Result<meteroid_store::domain::trials::TrialRequest, Problem> {
    use meteroid_store::domain::trials::{TrialLength, TrialRequest};
    Ok(match input {
        None => TrialRequest::Default,
        Some(TrialInput::Plan(_)) => TrialRequest::Plan,
        Some(TrialInput::None(_)) => TrialRequest::None,
        Some(TrialInput::Offer(trial)) => {
            let raw = trial.offer.trim();
            let named = match ids::parse_id::<common_domain::ids::TrialOfferId>(raw) {
                Some(id) => common_domain::ids::AliasOr::Id(id),
                None => common_domain::ids::AliasOr::Alias(raw.to_owned()),
            };
            let offer = state
                .services
                .find_trial_offer(tenant_id, named)
                .await
                .map_err(|_| {
                    Problem::field(
                        format!("{pointer}/offer"),
                        "not_found",
                        format!("trial offer {raw}"),
                    )
                })?;
            if offer.archived_at.is_some() {
                return Err(Problem::field(
                    format!("{pointer}/offer"),
                    "archived",
                    format!("The trial offer {raw} is archived"),
                ));
            }
            TrialRequest::Offer(offer.id)
        }
        Some(TrialInput::Custom(trial)) => {
            let length = match (trial.days, trial.until) {
                (Some(days), None) => {
                    bounds::trial_days(days, &format!("{pointer}/days"))?;
                    TrialLength::Days(days)
                }
                (None, Some(until)) => {
                    crate::v2::trials::check_until(
                        until,
                        billing_start,
                        &format!("{pointer}/until"),
                    )?;
                    TrialLength::Until(until)
                }
                _ => {
                    return Err(Problem::field(
                        pointer.to_owned(),
                        "invalid",
                        "A trial of its own lasts `days` or ends `until`, one of them",
                    ));
                }
            };
            let trial_plan = match trial.features_of.as_deref() {
                None => None,
                Some(plan) => Some(Some(
                    crate::v2::trials::plan_in(
                        state,
                        tenant_id,
                        plan,
                        &format!("{pointer}/features_of"),
                    )
                    .await?,
                )),
            };
            TrialRequest::Custom {
                length,
                free: None,
                trial_plan,
                manual: false,
            }
        }
    })
}

/// The terms a new subscription is sold on beside its purchase, as `POST /subscriptions` and
/// `POST /checkout-sessions` send them.
pub(crate) struct Terms {
    pub billing_day_anchor: Option<u16>,
    pub net_terms: Option<u32>,
}

/// Refuses terms out of their bounds, each at its pointer: a billing day of 1 to 31, net terms of
/// 0 to 365 days.
pub(crate) fn check_terms(terms: Terms) -> Result<(), Problem> {
    if let Some(day) = terms.billing_day_anchor {
        bounds::billing_day_anchor(day, "/billing_day_anchor")?;
    }
    if let Some(days) = terms.net_terms {
        bounds::net_terms(days, "/invoicing/net_terms")?;
    }
    Ok(())
}

/// What a purchase of a plan names, as `POST /subscriptions` and `POST /checkout-sessions` send
/// it.
pub(crate) struct PurchaseAsked<'a> {
    pub plan: &'a str,
    pub version: Option<i32>,
    pub option: Option<&'a str>,
    pub currency: Option<&'a str>,
    pub quantities: &'a BTreeMap<String, String>,
    pub levels: &'a BTreeMap<String, String>,
    pub charges: &'a CreateCharges,
    pub add_ons: &'a [AddOnPurchase],
    pub coupons: &'a [String],
}

/// A purchase resolved before anything is written: the published version bought, the option
/// it is sold in (its currency checked), how its charges differ from the plan's (none when they do not),
/// its add-ons and coupons.
pub(crate) struct Purchase {
    pub fp: FullPlan,
    pub option: Option<PlanOption>,
    pub components: Option<CreateSubscriptionComponents>,
    pub add_ons: Vec<CreateSubscriptionAddOn>,
    pub coupons: Vec<CouponId>,
}

/// Every code a purchase names, resolved for `customer` and checked at its JSON pointer: the
/// plan version, its option and currency, the seat counts, levels, price overrides, own charges,
/// add-ons and coupons.
pub(crate) async fn resolve_purchase(
    state: &AppState,
    auth: &AuthorizedAsTenant,
    customer: &Customer,
    asked: PurchaseAsked<'_>,
) -> Result<Purchase, Problem> {
    let tenant_id = auth.tenant_id;
    bounds::count(
        asked.add_ons.len(),
        bounds::MAX_ADD_ONS,
        "/add_ons",
        "add-ons",
    )?;
    bounds::count(
        asked.coupons.len(),
        bounds::MAX_COUPONS,
        "/coupons",
        "coupons",
    )?;
    bounds::count(
        asked.charges.overrides.len(),
        bounds::MAX_CHARGES,
        "/charges/override",
        "overrides",
    )?;
    bounds::count(
        asked.charges.add.len(),
        bounds::MAX_CHARGES,
        "/charges/add",
        "charges",
    )?;
    let plan_id = ids::plan_id(state, tenant_id, asked.plan)
        .await
        .map_err(not_found_at("/plan".to_owned(), "plan", asked.plan))?;
    let fp = published_version(state, tenant_id, plan_id, asked.version, "").await?;

    let named = match asked.option {
        Some(alias) => Some(option_named(&fp, alias, "/option")?),
        None => None,
    };
    let (currency, option) = sale(&customer.currency, &fp, named);
    if let Some(sent) = asked.currency
        && !sent.trim().eq_ignore_ascii_case(&currency)
    {
        return Err(Problem::field(
            "/currency",
            "invalid",
            format!(
                "This subscription is sold in {currency}: a subscription is sold in its \
                 customer's currency ({}) when the plan sells it, else in the plan's",
                customer.currency
            ),
        ));
    }
    let cells = cells(&fp, option.as_ref(), &currency);

    let mut components = CreateSubscriptionComponents {
        parameterized_components: Vec::new(),
        overridden_components: Vec::new(),
        extra_components: Vec::new(),
        remove_components: Vec::new(),
    };

    for (index, raw) in asked.charges.remove.iter().enumerate() {
        let component = plan_charge(&fp, &cells, raw, &format!("/charges/remove/{index}"))?;
        components.remove_components.push(component.id);
    }

    // Add-ons first: a seat count may be theirs.
    let mut add_ons = Vec::with_capacity(asked.add_ons.len());
    for (index, purchase) in asked.add_ons.iter().enumerate() {
        let path = format!("/add_ons/{index}");
        if purchase.quantity == 0 {
            return Err(Problem::field(
                format!("{path}/quantity"),
                "invalid",
                "An add-on is held once at least",
            ));
        }
        let quantity = i32::try_from(purchase.quantity).map_err(|_| {
            Problem::field(
                format!("{path}/quantity"),
                "invalid",
                format!("An add-on is held {} times at most", i32::MAX),
            )
        })?;
        let add_on_id = ids::add_on_id(state, tenant_id, &purchase.add_on)
            .await
            .map_err(not_found_at(
                format!("{path}/add_on"),
                "add-on",
                &purchase.add_on,
            ))?;
        let add_on = state
            .store
            .get_add_on_by_id(tenant_id, add_on_id)
            .await
            .map_err(|error| Problem::at(format!("{path}/add_on"), error))?;
        add_ons.push((add_on, quantity));
    }

    let mut seat_counts: HashMap<ManagedQuantityId, u32> = HashMap::new();
    for (code, raw) in asked.quantities {
        let path = format!("/quantities/{code}");
        let count = seats(raw, &path)?;
        let quantity = ids::quantity(state, tenant_id, code)
            .await
            .map_err(not_found_at(path.clone(), "managed quantity", code))?;
        let plan_seats: Vec<_> = cells
            .iter()
            .filter(|component| counts(component.fee_structure.as_ref(), &quantity))
            .filter(|component| !components.remove_components.contains(&component.id))
            .collect();
        let add_on_seats = add_ons
            .iter()
            .any(|(add_on, _)| counts(add_on.fee_structure.as_ref(), &quantity));
        if plan_seats.is_empty() && !add_on_seats {
            return Err(Problem::field(
                path,
                "not_found",
                format!("No seat charge of the plan or of its add-ons counts {code}"),
            ));
        }
        for component in plan_seats {
            components
                .parameterized_components
                .push(ComponentParameterization {
                    component_id: component.id,
                    parameters: ComponentParameters {
                        initial_slot_count: Some(count),
                        billing_period: None,
                        committed_capacity: None,
                    },
                });
        }
        seat_counts.insert(quantity.id, count);
    }

    for (code, raw) in asked.levels {
        let path = format!("/levels/{code}");
        let included = count(raw, &path)?;
        let component = plan_charge(&fp, &cells, code, &path)?;
        if meteroid_store::domain::usage_levels::levels(component).is_empty() {
            return Err(Problem::field(
                path,
                "not_found",
                format!("The charge of {code} is not sold in levels"),
            ));
        }
        components
            .parameterized_components
            .push(ComponentParameterization {
                component_id: component.id,
                parameters: ComponentParameters {
                    initial_slot_count: None,
                    billing_period: None,
                    committed_capacity: Some(included),
                },
            });
    }

    for (index, wanted) in asked.charges.overrides.iter().enumerate() {
        let path = format!("/charges/override/{index}");
        let component = plan_charge(&fp, &cells, &wanted.charge, &format!("{path}/charge"))?;
        let stored = plan_stored(component, &path)?;
        // The cadence the cell bills on, unless the override names one.
        let cadence = wanted.cadence.unwrap_or_else(|| {
            option
                .as_ref()
                .and_then(|option| {
                    OptionPricing::new(option.id, &currency)
                        .prices(component)
                        .first()
                        .map(|price| price.cadence)
                })
                .or_else(|| component.prices.first().map(|price| price.cadence))
                .unwrap_or(BillingPeriodEnum::Monthly)
                .into()
        });
        let price =
            override_price(state, tenant_id, &stored, wanted, cadence, &currency, &path).await?;
        components.overridden_components.push(ComponentOverride {
            component_id: component.id,
            name: wanted
                .name
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .unwrap_or(&component.name)
                .to_owned(),
            price_entry: PriceEntry::New(price),
        });
    }

    for (index, charge) in asked.charges.add.iter().enumerate() {
        let path = format!("/charges/add/{index}");
        let (name, product_id, structure, price) =
            own_charge(state, tenant_id, charge, &currency, &path).await?;
        components.extra_components.push(ExtraComponent {
            name,
            product_ref: ProductRef::Existing(product_id),
            price_entry: PriceEntry::New(price),
            fee_structure: Some(structure),
        });
    }

    // Each usage-levels charge on the level asked, checked against its cell, else its lowest.
    settle_levels(&cells, &mut components)
        .map_err(|error| Problem::field("/levels", "invalid", store_message(&error)))?;

    let introduces_credit = components.extra_components.iter().any(|extra| {
        extra
            .fee_structure
            .as_ref()
            .is_some_and(FeeStructure::is_credit)
    });
    ensure_credit_pricing_allowed(
        state.feature_resolver.as_ref(),
        auth.organization_id,
        introduces_credit,
    )
    .await
    .map_err(Problem::gated)?;

    let coupons = if asked.coupons.is_empty() {
        Vec::new()
    } else {
        state
            .store
            .resolve_coupon_codes(tenant_id, asked.coupons)
            .await
            .map_err(|error| Problem::at("/coupons", error))?
    };

    let add_ons: Vec<CreateSubscriptionAddOn> = add_ons
        .into_iter()
        .map(|(add_on, quantity)| {
            let seats = match &add_on.fee_structure {
                Some(FeeStructure::Slot {
                    quantity_id: Some(quantity_id),
                    ..
                }) => seat_counts.get(quantity_id).copied(),
                _ => None,
            };
            CreateSubscriptionAddOn {
                add_on_id: add_on.id,
                customization: match seats {
                    Some(count) => SubscriptionAddOnCustomization::Parameterization(
                        SubscriptionAddOnParameterization {
                            initial_slot_count: Some(count),
                            billing_period: None,
                            committed_capacity: None,
                        },
                    ),
                    None => SubscriptionAddOnCustomization::None,
                },
                quantity,
            }
        })
        .collect();

    let changes_components = !(components.parameterized_components.is_empty()
        && components.overridden_components.is_empty()
        && components.extra_components.is_empty()
        && components.remove_components.is_empty());

    Ok(Purchase {
        fp,
        option,
        components: changes_components.then_some(components),
        add_ons,
        coupons,
    })
}

/// Why the insert failed: a duplicate is the external id taken by a create racing this one when
/// another subscription now holds it, else the store's error.
async fn insert_refused(
    state: &AppState,
    auth: &AuthorizedAsTenant,
    external_id: Option<&str>,
    error: error_stack::Report<StoreError>,
) -> Problem {
    if let Some(external_id) = external_id
        && matches!(error.current_context(), StoreError::DuplicateValue { .. })
        && state
            .store
            .find_subscription_id_by_external_id(auth.tenant_id, external_id)
            .await
            .is_ok()
    {
        return taken(external_id);
    }
    Problem::resource_or(None, error)
}
