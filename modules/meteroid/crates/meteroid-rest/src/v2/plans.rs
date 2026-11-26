//! `/api/v2/plans` (ADR-0013 §5, §6): a plan's live fields and its versions, each a price sheet
//! of charges in options. A draft is written whole (`PUT …/draft`) and published.
use crate::AppState;
use crate::v2::Reader;
use crate::v2::bounds;
use crate::v2::charges::{
    Cadence, Charge, EVERY_OPTION, StoredCharge, StoredPrice, option_set, render, to_domain,
};
use crate::v2::entitlements::{Entitlement, EntitlementInput};
use crate::v2::extract::{Body, Path, Query};
use crate::v2::idempotency::committed_write;
use crate::v2::ids;
use crate::v2::pagination::{Cursor, PageQuery, keyset, page};
use crate::v2::problem::{Problem, ProblemCode};
use crate::v2::product_families::{WHOLE, family_codes, family_ref};
use crate::v2::references::{CatalogNames, resolve_refs};
use crate::v2::refs::{ProductFamilyRef, ResourceTypeRef};
use axum::Extension;
use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use common_domain::ids::{
    AddOnId, BaseId, PlanId, ProductFamilyId, ProductId, ResourceTypeId, TenantId,
};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use meteroid_app::features::minimum_commitments_open;
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_store::domain::entitlement_fold::Tier;
use meteroid_store::domain::entitlements::EntitlementSpec;
use meteroid_store::domain::enums::{PlanStatusEnum, PlanTypeEnum};
use meteroid_store::domain::plan_drafts::{
    DraftAddOn, DraftCharge, DraftMinimum, DraftPrice, DraftPriceOptions, PlanDraft,
    PlanDraftOption,
};
use meteroid_store::domain::plan_minimums::MinimumScope;
use meteroid_store::domain::plan_options::{
    DEFAULT_OPTION_ALIAS, DEFAULT_OPTION_NAME, alias_from_name,
};
use meteroid_store::domain::pricing_pages::PlanPricingPage;
use meteroid_store::domain::{
    FullPlan, PlanNew, PlanPatch, PlanTrial, PlanVersionFilter, PlanVersionNewInternal,
};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::add_ons::AddOnInterfaceAuto;
use meteroid_store::repositories::plan_version_add_ons::PlanVersionAddOnInterface;
use meteroid_store::repositories::{
    CatalogCodesInterface, PlanDraftsInterface, PlansInterface, PricingPagesInterface,
    ResourcesInterface,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

// ── Model ───────────────────────────────────────────────────────────

/// Where a plan is in its life: `draft`, never published; `active`, published and sold;
/// `inactive`, published but off pricing pages and checkout links (no v2 write sets it);
/// `archived`, no longer sold, its subscriptions keeping it.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    /// Never published.
    Draft,
    Active,
    Inactive,
    Archived,
}

impl From<PlanStatusEnum> for PlanStatus {
    fn from(status: PlanStatusEnum) -> Self {
        match status {
            PlanStatusEnum::Draft => PlanStatus::Draft,
            PlanStatusEnum::Active => PlanStatus::Active,
            PlanStatusEnum::Inactive => PlanStatus::Inactive,
            PlanStatusEnum::Archived => PlanStatus::Archived,
        }
    }
}

/// What kind of plan it is: `standard`, sold to anyone; `free`, subscribed without a payment
/// method; `custom`, negotiated for one customer.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PlanType {
    #[default]
    Standard,
    Free,
    /// Negotiated for one customer.
    Custom,
}

impl From<PlanTypeEnum> for PlanType {
    fn from(plan_type: PlanTypeEnum) -> Self {
        match plan_type {
            PlanTypeEnum::Standard => PlanType::Standard,
            PlanTypeEnum::Free => PlanType::Free,
            PlanTypeEnum::Custom => PlanType::Custom,
        }
    }
}

impl From<PlanType> for PlanTypeEnum {
    fn from(plan_type: PlanType) -> Self {
        match plan_type {
            PlanType::Standard => PlanTypeEnum::Standard,
            PlanType::Free => PlanTypeEnum::Free,
            PlanType::Custom => PlanTypeEnum::Custom,
        }
    }
}

/// Where a version is in its life: `draft`, being written and not sold yet; `active`, the one
/// new subscriptions get; `superseded`, published then replaced by a later version, its
/// subscriptions keeping it.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VersionStatus {
    Draft,
    /// The version new subscriptions get.
    Active,
    /// Published, then replaced by a later version; its subscriptions keep it.
    Superseded,
}

/// What a buyer picks when buying the version ("Monthly", "Annual", "Startup").
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PlanOption {
    /// The handle prices and subscriptions name it by, kept across versions.
    pub alias: String,
    /// What a buyer reads, on the pricing page, the portal and invoices.
    pub name: String,
    /// Sold when no option is named; always the first, never hidden.
    pub default: bool,
    /// Sold when named, never advertised (off the pricing page and the portal).
    pub hidden: bool,
}

/// An option of a draft: one way of buying the version ("Monthly", "Annual"), its prices named
/// by its `alias`. 20 a version at most.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct OptionInput {
    /// Lowercase letters, digits, `-` and `_`; the name's slug when left out. Keep an option's
    /// alias from one version to the next: subscriptions name their option by it.
    #[serde(default)]
    pub alias: Option<String>,
    /// What a buyer reads, 1 to 255 characters.
    pub name: String,
    /// One option at most; the first is the default when none says so.
    #[serde(default)]
    pub default: bool,
    /// Sold when named, never advertised (off the pricing page and the portal); the default
    /// option is never hidden.
    #[serde(default)]
    pub hidden: bool,
}

/// A trial a subscription to the version starts with.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Trial {
    /// Its length in days, 0 to 3650.
    pub days: u32,
    /// Nothing is billed during the trial; `true` when left out.
    #[serde(default = "yes")]
    pub free: bool,
    /// The plan the trial gives access to, by code (id or code on input); none: this plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
}

fn yes() -> bool {
    true
}

/// A floor on what a subscription is billed each period.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Minimum {
    /// In minor units of `currency`.
    pub amount: i64,
    /// The version's currency; refused when another is sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    /// The products whose charges count, by code; none: every charge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub products: Option<Vec<String>>,
}

/// An add-on the version offers.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VersionAddOn {
    /// The add-on, by code (id or code on input).
    pub add_on: String,
    /// Customers may add it themselves (portal, checkout); overrides the add-on's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub self_serviceable: Option<bool>,
    /// How many of it a subscription may hold; overrides the add-on's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_instances: Option<i32>,
}

/// A version of a plan: its options, charges, terms and rows. A published version never
/// changes; a change is written in the draft and published as the next version.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct PlanVersion {
    /// 1 for the first version, counting up.
    pub number: i32,
    /// Draft, active or superseded.
    pub status: VersionStatus,
    /// The version's default currency: its minimum's, and what it is shown in first.
    pub currency: String,
    /// By position; the first is the default.
    pub options: Vec<PlanOption>,
    /// What the version bills, in the order written, each charge in its canonical composition.
    pub charges: Vec<Charge>,
    /// The trial a subscription starts with; none: no trial.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trial: Option<Trial>,
    /// Days an invoice is due after it is issued.
    pub net_terms: i32,
    /// A floor on what a subscription is billed each period; none: no floor.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum: Option<Minimum>,
    /// The add-ons a subscription to the version may hold.
    pub add_ons: Vec<VersionAddOn>,
    /// Its prices include tax.
    pub tax_inclusive: bool,
    /// How many periods a subscription runs; none: until cancelled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing_cycles: Option<i32>,
    /// The version's rows, canonical.
    pub entitlements: Vec<Entitlement>,
}

/// The full desired state of a draft.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[schema(as = PlanDraft)]
#[serde(deny_unknown_fields)]
pub struct VersionInput {
    /// The version's own, read-only: taken and ignored, so a version read is written back as is.
    #[serde(default)]
    #[schema(read_only)]
    pub number: Option<i32>,
    /// The version's own, read-only: taken and ignored, so a version read is written back as is.
    #[serde(default)]
    #[schema(read_only)]
    pub status: Option<VersionStatus>,
    /// The default currency; the first price's when left out.
    #[serde(default)]
    pub currency: Option<String>,
    /// One "Standard" option when left out; 20 at most.
    #[serde(default)]
    pub options: Vec<OptionInput>,
    /// What the version bills, 100 charges at most. Each price names an option by alias, or
    /// `*` for every one.
    #[serde(default)]
    pub charges: Vec<Charge>,
    /// None: no trial.
    #[serde(default)]
    pub trial: Option<Trial>,
    /// Days an invoice is due after it is issued, 0 to 365; 0 when left out.
    #[serde(default)]
    pub net_terms: i32,
    /// A floor on what a subscription is billed each period, in the version's currency.
    #[serde(default)]
    pub minimum: Option<Minimum>,
    /// The add-ons a subscription to the version may hold, 100 at most.
    #[serde(default)]
    pub add_ons: Vec<VersionAddOn>,
    /// Its prices include tax; `false` when left out.
    #[serde(default)]
    pub tax_inclusive: bool,
    /// 1 or more; none: until cancelled.
    #[serde(default)]
    pub billing_cycles: Option<i32>,
    /// The version's rows (`set`); left out, those of the version replaced are carried over,
    /// sent (even `[]`) they replace them.
    #[serde(default)]
    #[schema(examples(json!([
        {"feature": "sso", "value": {"type": "on"}},
        {"feature": "projects", "value": {"type": "limit", "limit": "5"}},
        {"feature": "projects", "value": {"type": "limit", "limit": "50"}, "level": "customer"},
        {
            "feature": "api_calls",
            "value": {"type": "limit", "limit": "100000"},
            "policy": {
                "resets": {"type": "billing_cycle"},
                "at_limit": "allow_and_bill",
                "grace_pct": 0,
                "thresholds": [80]
            }
        }
    ])))]
    pub entitlements: Option<Vec<EntitlementInput>>,
}

/// How the plan reads on its product family's pricing page and self-serve ladder. Live, never
/// versioned.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct Listing {
    /// Its rung on the self-serve ladder (1 first); none: not offered self-serve.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub self_serve_rank: Option<i32>,
    /// A line under the plan's name on the pricing page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tagline: Option<String>,
    /// Highlighted on the pricing page.
    pub popular: bool,
    /// The label of the plan's button on the pricing page; none: the page's default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cta_label: Option<String>,
    /// Sold by contacting sales: on the page, never self-serve.
    pub sales_led: bool,
    /// A sales-led plan's price stays off the page.
    pub hide_price: bool,
    /// Where a sales-led plan's button leads.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact_url: Option<String>,
}

/// A merge patch of the listing: a field left out is unchanged, `null` clears it.
#[derive(Deserialize, ToSchema, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct ListingPatch {
    /// Its rung on the self-serve ladder, 1 or more; `null` takes it off the ladder.
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<i32>)]
    pub self_serve_rank: Option<Option<i32>>,
    /// A line under the plan's name on the pricing page.
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub tagline: Option<Option<String>>,
    /// Highlighted on the pricing page.
    #[serde(default)]
    pub popular: Option<bool>,
    /// The label of the plan's button on the pricing page.
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub cta_label: Option<Option<String>>,
    /// Sold by contacting sales: on the page, never self-serve.
    #[serde(default)]
    pub sales_led: Option<bool>,
    /// A sales-led plan's price stays off the page.
    #[serde(default)]
    pub hide_price: Option<bool>,
    /// Where a sales-led plan's button leads.
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub contact_url: Option<Option<String>>,
}

/// A plan: a price sheet sold in versions. Its live fields (name, description, listing) change
/// in place; what it bills is in its versions, written through a draft and published.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct Plan {
    /// The plan's id, `plan_…`.
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: PlanId,
    /// Unique per tenant, immutable; paths take it in place of the id.
    pub code: String,
    /// What a buyer reads, on the pricing page, the portal and invoices.
    pub name: String,
    /// What the plan is, in the tenant's words.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Standard, free or custom.
    #[serde(rename = "type")]
    pub plan_type: PlanType,
    /// Draft, active, inactive or archived.
    pub status: PlanStatus,
    /// The product family it is sold in.
    pub family: ProductFamilyRef,
    /// How it reads on its family's pricing page and self-serve ladder.
    pub listing: Listing,
    /// The resource type its subscriptions are for; none: the customer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sold_for: Option<ResourceTypeRef>,
    /// The number of the version new subscriptions get.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_version: Option<i32>,
    /// A draft is being written: read it with `?version=draft`.
    pub has_draft: bool,
    /// When the plan was created.
    pub created_at: ApiDateTime,
    /// The version asked (`?version=`); left out of lists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<PlanVersion>,
}

crate::v2_list!(PlanList, Plan);

/// A version of a plan as listed, without its charges: read it whole with
/// `GET /plans/{id_or_code}?version=<number>`.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct PlanVersionSummary {
    /// 1 for the first version, counting up.
    pub number: i32,
    /// Draft, active or superseded.
    pub status: VersionStatus,
    /// The version's default currency (ISO 4217).
    pub currency: String,
    /// When the version was first written, as a draft.
    pub created_at: ApiDateTime,
}

crate::v2_list!(PlanVersionList, PlanVersionSummary);

/// A plan to create, with its first version as a draft.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct CreatePlan {
    /// Unique per tenant, immutable; the name's slug when left out.
    #[serde(default)]
    pub code: Option<String>,
    /// 1 to 255 characters.
    pub name: String,
    /// 4,096 characters at most.
    #[serde(default)]
    pub description: Option<String>,
    /// The product family, by code or id; the default family when left out.
    #[serde(default)]
    pub family: Option<String>,
    /// `standard` when left out.
    #[serde(default, rename = "type")]
    pub plan_type: PlanType,
    /// The resource type its subscriptions are for, by code or id; none: the customer.
    #[serde(default)]
    pub sold_for: Option<String>,
    /// The first version, a draft.
    pub draft: VersionInput,
}

/// A merge patch of the live fields; versions are written through the draft, the status by
/// `…/archive` and `…/unarchive`.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct UpdatePlan {
    /// 1 to 255 characters; it cannot be cleared.
    #[serde(default)]
    pub name: Option<String>,
    /// 4,096 characters at most; `null` clears it.
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub description: Option<Option<String>>,
    /// Merged field by field into the plan's listing.
    #[serde(default)]
    pub listing: Option<ListingPatch>,
    /// The resource type its subscriptions are for, by code or id; `null`: the customer. A plan
    /// sold for a type carries no rows across all resources.
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub sold_for: Option<Option<String>>,
}

#[derive(Deserialize, IntoParams, Debug, Clone, Default)]
#[into_params(parameter_in = Query)]
pub struct PlanListQuery {
    /// Only the plans of this family, by code or id.
    pub family: Option<String>,
    /// Only the plans in one of these statuses (repeat the parameter for several); left out,
    /// every plan but the archived ones.
    #[param(style = Form, explode)]
    pub status: Option<Vec<PlanStatus>>,
    /// How many items, 1 to 100; 20 by default.
    pub limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    pub after: Option<String>,
}

#[derive(Deserialize, IntoParams, Debug, Clone, Default)]
#[into_params(parameter_in = Query)]
pub struct PlanGetQuery {
    /// `active` (the default; the draft of a plan never published), `draft`, or a version
    /// number.
    #[param(schema_with = version_selector)]
    pub version: Option<String>,
}

/// The schema of `?version=`: one of two words, or a version number.
fn version_selector() -> utoipa::openapi::RefOr<utoipa::openapi::Schema> {
    use utoipa::openapi::schema::{AnyOfBuilder, ObjectBuilder, Type};
    AnyOfBuilder::new()
        .item(
            ObjectBuilder::new()
                .schema_type(Type::String)
                .enum_values(Some(["active", "draft"])),
        )
        .item(
            ObjectBuilder::new()
                .schema_type(Type::String)
                .pattern(Some("^[1-9][0-9]*$"))
                .description(Some("A version number")),
        )
        .description(Some(
            "`active` (the default; the draft of a plan never published), `draft`, or a \
             version number.",
        ))
        .into()
}

// ── Reading ─────────────────────────────────────────────────────────

fn version_status(fp: &FullPlan) -> VersionStatus {
    if fp.version.is_draft_version {
        VersionStatus::Draft
    } else if fp.plan.active_version_id == Some(fp.version.id) {
        VersionStatus::Active
    } else {
        VersionStatus::Superseded
    }
}

/// The version's charges as stored, in the order they were written (their ids are UUIDv7), each
/// price in the options it is linked in.
fn stored_charges(fp: &FullPlan) -> Result<Vec<StoredCharge>, Problem> {
    let positions: HashMap<_, usize> = fp
        .version
        .options
        .iter()
        .enumerate()
        .map(|(position, option)| (option.id, position))
        .collect();
    let mut components: Vec<_> = fp.price_components.iter().collect();
    components.sort_by_key(|component| component.id.as_uuid());
    components
        .into_iter()
        .enumerate()
        .map(|(index, component)| {
            let path = format!("/version/charges/{index}");
            let product_id = component.product_id.ok_or_else(|| {
                Problem::not_expressible(&path, "a charge without a product: read it with API v1")
            })?;
            if component.legacy_pricing.is_some() {
                return Err(Problem::not_expressible(
                    &path,
                    "a charge priced before plan versions held prices: read it with API v1",
                ));
            }
            let prices = component
                .prices
                .iter()
                .map(|price| {
                    let linked: Vec<usize> = component
                        .option_prices
                        .iter()
                        .filter(|link| link.price_id == price.id)
                        .filter_map(|link| positions.get(&link.option_id).copied())
                        .collect();
                    StoredPrice {
                        cadence: price.cadence,
                        currency: price.currency.clone(),
                        pricing: price.pricing.clone(),
                        credit_unit: price.credit_unit.clone(),
                        options: option_set(&linked, positions.len()),
                    }
                })
                .collect();
            Ok(StoredCharge {
                name: component.name.clone(),
                product_id,
                structure: component.fee_structure.clone(),
                prices,
            })
        })
        .collect()
}

async fn render_version(
    state: &impl Reader,
    tenant_id: TenantId,
    fp: &FullPlan,
) -> Result<PlanVersion, Problem> {
    let stored = stored_charges(fp)?;
    let names = CatalogNames::load(state, tenant_id, &stored).await?;
    let aliases: Vec<String> = fp.version.options.iter().map(|o| o.alias.clone()).collect();
    let charges = stored
        .iter()
        .enumerate()
        .map(|(index, charge)| {
            render(
                charge,
                &aliases,
                &names,
                &format!("/version/charges/{index}"),
            )
        })
        .collect::<Result<Vec<_>, Problem>>()?;

    let attached = state
        .store()
        .list_plan_version_add_ons(fp.version.id, tenant_id)
        .await?;
    let add_on_codes: HashMap<AddOnId, String> = if attached.is_empty() {
        HashMap::new()
    } else {
        state
            .store()
            .list_add_ons_by_ids(tenant_id, attached.iter().map(|a| a.add_on_id).collect())
            .await?
            .into_iter()
            .map(|add_on| (add_on.id, add_on.code))
            .collect()
    };
    let mut add_ons: Vec<VersionAddOn> = attached
        .iter()
        .map(|attached| VersionAddOn {
            add_on: add_on_codes
                .get(&attached.add_on_id)
                .cloned()
                .unwrap_or_else(|| attached.add_on_id.to_string()),
            self_serviceable: attached.self_serviceable,
            max_instances: attached.max_instances_per_subscription,
        })
        .collect();
    add_ons.sort_by(|a, b| a.add_on.cmp(&b.add_on));

    let trial = match fp.version.trial_duration_days {
        None => None,
        Some(days) => {
            let plan = match fp.version.trialing_plan_id {
                None => None,
                Some(id) => Some(
                    state
                        .store()
                        .plan_codes(tenant_id, &[id])
                        .await?
                        .remove(&id)
                        .unwrap_or_else(|| id.to_string()),
                ),
            };
            Some(Trial {
                days: u32::try_from(days).unwrap_or(0),
                free: fp.version.trial_is_free,
                plan,
            })
        }
    };

    let minimum = fp.version.minimum.as_ref().map(|minimum| Minimum {
        amount: minimum.amount_subunit,
        currency: Some(fp.version.currency.clone()),
        products: match &minimum.scope {
            MinimumScope::AllComponents => None,
            MinimumScope::Products(products) => Some(
                products
                    .iter()
                    .map(|id| {
                        fp.products
                            .get(id)
                            .map(|product| product.code.clone())
                            .unwrap_or_else(|| id.to_string())
                    })
                    .collect(),
            ),
        },
    });

    let entitlements =
        crate::v2::entitlements::render(state, tenant_id, fp.version.entitlements.clone()).await?;

    Ok(PlanVersion {
        number: fp.version.version,
        status: version_status(fp),
        currency: fp.version.currency.clone(),
        options: fp
            .version
            .options
            .iter()
            .enumerate()
            .map(|(position, option)| PlanOption {
                alias: option.alias.clone(),
                name: option.name.clone(),
                default: position == 0,
                hidden: option.hidden,
            })
            .collect(),
        charges,
        trial,
        net_terms: fp.version.net_terms,
        minimum,
        add_ons,
        tax_inclusive: fp.version.tax_inclusive,
        billing_cycles: fp.version.billing_cycles,
        entitlements,
    })
}

async fn listing(
    state: &impl Reader,
    tenant_id: TenantId,
    plan_id: PlanId,
    rank: Option<i32>,
) -> Result<Listing, Problem> {
    let page = state
        .store()
        .get_plan_pricing_page(tenant_id, plan_id)
        .await?;
    Ok(listing_of(page, rank))
}

fn listing_of(page: PlanPricingPage, rank: Option<i32>) -> Listing {
    Listing {
        self_serve_rank: rank,
        tagline: page.tagline,
        popular: page.popular,
        cta_label: page.cta_label,
        sales_led: page.sales_led,
        hide_price: page.hide_price,
        contact_url: page.contact_url,
    }
}

pub(crate) async fn render_plan(
    state: &impl Reader,
    tenant_id: TenantId,
    fp: &FullPlan,
    with_version: bool,
) -> Result<Plan, Problem> {
    let version = if with_version {
        Some(render_version(state, tenant_id, fp).await?)
    } else {
        None
    };
    let overview = state
        .store()
        .get_plan_overview(fp.plan.id, tenant_id)
        .await?;
    let families = family_codes(state, tenant_id).await?;
    let sold_for = sold_for_codes(state, tenant_id, [fp.plan.sold_for_resource_type_id]).await?;
    Ok(Plan {
        id: fp.plan.id,
        code: fp.plan.code.clone(),
        name: fp.plan.name.clone(),
        description: fp.plan.description.clone(),
        plan_type: fp.plan.plan_type.clone().into(),
        status: fp.plan.status.clone().into(),
        family: family_ref(&families, fp.plan.product_family_id),
        listing: listing(state, tenant_id, fp.plan.id, fp.plan.self_service_rank).await?,
        sold_for: fp
            .plan
            .sold_for_resource_type_id
            .map(|id| sold_for_ref(&sold_for, id)),
        active_version: overview.active_version.map(|version| version.version),
        has_draft: overview.has_draft_version,
        created_at: fp.plan.created_at.into(),
        version,
    })
}

/// The codes of the resource types plans are sold for, read when one is.
async fn sold_for_codes(
    state: &impl Reader,
    tenant_id: TenantId,
    sold_for: impl IntoIterator<Item = Option<ResourceTypeId>>,
) -> Result<HashMap<ResourceTypeId, String>, Problem> {
    if sold_for.into_iter().all(|id| id.is_none()) {
        return Ok(HashMap::new());
    }
    Ok(state
        .store()
        .list_resource_types(tenant_id, true)
        .await?
        .into_iter()
        .map(|resource_type| (resource_type.id, resource_type.code))
        .collect())
}

fn sold_for_ref(codes: &HashMap<ResourceTypeId, String>, id: ResourceTypeId) -> ResourceTypeRef {
    ResourceTypeRef {
        id,
        code: codes.get(&id).cloned().unwrap_or_else(|| id.to_string()),
    }
}

/// The resource type a plan is sold for, by code or id, refused at `path` when there is none.
async fn sold_for_of(
    state: &AppState,
    tenant_id: TenantId,
    raw: &str,
    path: &str,
) -> Result<ResourceTypeId, Problem> {
    ids::resource_type(state, tenant_id, raw)
        .await
        .map(|resource_type| resource_type.id)
        .map_err(|problem| {
            if problem.status == 404 {
                Problem::field(
                    path,
                    "not_found",
                    format!("No resource type '{}'", raw.trim()),
                )
            } else {
                problem
            }
        })
}

/// What is known of a plan written when v2 cannot read it back (the write is made all the same: a
/// write committed never answers an error).
fn known(fp: &FullPlan) -> serde_json::Value {
    serde_json::json!({
        "id": fp.plan.id.to_string(),
        "code": fp.plan.code,
        "name": fp.plan.name,
    })
}

fn known_id(plan_id: PlanId) -> serde_json::Value {
    serde_json::json!({"id": plan_id.to_string()})
}

/// The plan's version of `filter`; none: the active one, else the draft of a plan never
/// published.
pub(crate) async fn full_plan(
    state: &impl Reader,
    tenant_id: TenantId,
    plan_id: PlanId,
    asked: Option<&str>,
) -> Result<FullPlan, Problem> {
    let filter = match asked.map(str::trim) {
        None | Some("") => None,
        Some("active") => Some(PlanVersionFilter::Active),
        Some("draft") => Some(PlanVersionFilter::Draft),
        Some(number) => Some(PlanVersionFilter::Version(number.parse::<i32>().map_err(
            |_| {
                Problem::field(
                    "version",
                    "invalid",
                    "`active`, `draft` or a version number",
                )
            },
        )?)),
    };
    match filter {
        Some(filter) => Ok(state
            .store()
            .get_full_plan(plan_id, tenant_id, filter)
            .await?),
        None => match state
            .store()
            .get_full_plan(plan_id, tenant_id, PlanVersionFilter::Active)
            .await
        {
            Ok(fp) => Ok(fp),
            Err(error) if matches!(error.current_context(), StoreError::ValueNotFound(_)) => {
                Ok(state
                    .store()
                    .get_full_plan(plan_id, tenant_id, PlanVersionFilter::Draft)
                    .await?)
            }
            Err(error) => Err(error.into()),
        },
    }
}

/// The version as it bills and grants, whatever its number and status: a draft reading the same
/// as the active version publishes nothing. A row's id and history are its version's own, so
/// rows compare by what they grant.
fn same_sheet(a: &PlanVersion, b: &PlanVersion) -> bool {
    const ROW_IDENTITY: [&str; 7] = [
        "id",
        "owner",
        "created_by",
        "created_at",
        "updated_at",
        "revoked_at",
        "revoked_by",
    ];
    let sheet = |version: &PlanVersion| {
        let mut version = version.clone();
        version.number = 0;
        version.status = VersionStatus::Draft;
        version.charges.sort_by(|x, y| x.name.cmp(&y.name));
        let mut value = serde_json::to_value(&version).unwrap_or_default();
        if let Some(rows) = value
            .get_mut("entitlements")
            .and_then(serde_json::Value::as_array_mut)
        {
            for row in rows.iter_mut().filter_map(serde_json::Value::as_object_mut) {
                for key in ROW_IDENTITY {
                    row.remove(key);
                }
            }
        }
        value
    };
    sheet(a) == sheet(b)
}

// ── Writing ─────────────────────────────────────────────────────────

/// What a draft keeps from the version it replaces, v2 not writing it (yet).
#[derive(Default)]
struct Carried {
    period_start_day: Option<i16>,
    credit_exhausted_rule: Option<meteroid_store::domain::credit_exhausted::CreditExhaustedRule>,
    entitlements: Vec<EntitlementSpec>,
    minimum: Option<(i64, MinimumScope)>,
}

impl Carried {
    fn of(fp: &FullPlan) -> Self {
        Carried {
            period_start_day: fp.version.period_start_day,
            credit_exhausted_rule: fp.version.credit_exhausted_rule.clone(),
            entitlements: fp
                .version
                .entitlements
                .iter()
                .map(EntitlementSpec::from)
                .collect(),
            minimum: fp
                .version
                .minimum
                .as_ref()
                .map(|minimum| (minimum.amount_subunit, minimum.scope.clone())),
        }
    }
}

fn options_of(input: &[OptionInput]) -> Result<Vec<PlanDraftOption>, Problem> {
    if input.is_empty() {
        return Ok(vec![PlanDraftOption {
            name: DEFAULT_OPTION_NAME.to_owned(),
            alias: DEFAULT_OPTION_ALIAS.to_owned(),
            hidden: false,
        }]);
    }
    if input.iter().filter(|option| option.default).count() > 1 {
        return Err(Problem::field(
            "/options",
            "invalid",
            "One option is the default at most",
        ));
    }
    let mut taken: Vec<String> = input
        .iter()
        .filter_map(|option| option.alias.as_ref().map(|alias| alias.trim().to_owned()))
        .collect();
    let mut options: Vec<(bool, PlanDraftOption)> = Vec::with_capacity(input.len());
    for (index, option) in input.iter().enumerate() {
        let name = option.name.trim().to_owned();
        if name.is_empty() {
            return Err(Problem::field(
                format!("/options/{index}/name"),
                "required",
                "An option has a name",
            ));
        }
        bounds::name(&name, &format!("/options/{index}/name"))?;
        let alias = match option.alias.as_deref().map(str::trim) {
            Some(alias) => alias.to_owned(),
            None => {
                let alias = alias_from_name(&name, &taken);
                taken.push(alias.clone());
                alias
            }
        };
        if option.default && option.hidden {
            return Err(Problem::field(
                format!("/options/{index}/hidden"),
                "invalid",
                "The default option is always shown",
            ));
        }
        options.push((
            option.default,
            PlanDraftOption {
                name,
                alias,
                hidden: option.hidden,
            },
        ));
    }
    // The default goes first.
    options.sort_by_key(|(default, _)| !*default);
    Ok(options.into_iter().map(|(_, option)| option).collect())
}

/// Refuses two prices of `charge` in one option and currency (`*` is in every option): an option
/// is one way of buying the plan, billed on one cadence per currency — monthly and annual are
/// two options. Two cadences are refused at the second price's `cadence`, two prices of one
/// cadence at the second price (a cell's levels are one price's). `path` is the charge's.
fn check_one_cadence(charge: &Charge, aliases: &[&str], path: &str) -> Result<(), Problem> {
    let mut cells: HashMap<(&str, String), (usize, Cadence)> = HashMap::new();
    for (index, price) in charge.prices.iter().enumerate() {
        let option = price.option.trim();
        let options: Vec<&str> = if option == EVERY_OPTION {
            aliases.to_vec()
        } else {
            vec![option]
        };
        let currency = price.currency.trim().to_ascii_uppercase();
        for option in options {
            let Some((first, cadence)) =
                cells.insert((option, currency.clone()), (index, price.cadence))
            else {
                continue;
            };
            if cadence != price.cadence {
                return Err(Problem::field(
                    format!("{path}/prices/{index}/cadence"),
                    "invalid",
                    format!(
                        "Option '{option}' already sells this charge {} in {currency} \
                         (prices/{first}): an option bills one cadence per currency, so sell \
                         another cadence in another option",
                        cadence.as_str()
                    ),
                ));
            }
            return Err(Problem::field(
                format!("{path}/prices/{index}"),
                "duplicate",
                format!(
                    "Option '{option}' already has a price of this charge in {currency} \
                     (prices/{first}): one price per option and currency, a cell's levels in one \
                     price",
                ),
            ));
        }
    }
    Ok(())
}

/// The draft `input` asks, its references resolved; `prefix` is the input's JSON pointer.
async fn draft_of(
    state: &AppState,
    auth: &AuthorizedAsTenant,
    input: &VersionInput,
    carried: Carried,
    prefix: &str,
) -> Result<PlanDraft, Problem> {
    let tenant_id = auth.tenant_id;
    if input.net_terms < 0 || input.net_terms > bounds::MAX_NET_TERMS_DAYS as i32 {
        return Err(Problem::field(
            format!("{prefix}/net_terms"),
            "invalid",
            format!("Net terms are 0 to {} days", bounds::MAX_NET_TERMS_DAYS),
        ));
    }
    if let Some(trial) = &input.trial {
        bounds::trial_days(trial.days, &format!("{prefix}/trial/days"))?;
    }
    bounds::count(
        input.options.len(),
        bounds::MAX_OPTIONS,
        &format!("{prefix}/options"),
        "options",
    )?;
    bounds::count(
        input.charges.len(),
        bounds::MAX_CHARGES,
        &format!("{prefix}/charges"),
        "charges",
    )?;
    bounds::count(
        input.add_ons.len(),
        bounds::MAX_ADD_ONS,
        &format!("{prefix}/add_ons"),
        "add-ons",
    )?;
    if let Some(currency) = input.currency.as_deref()
        && meteroid_store::constants::Currencies::resolve_currency(
            &currency.trim().to_ascii_uppercase(),
        )
        .is_none()
    {
        return Err(Problem::field(
            format!("{prefix}/currency"),
            "invalid",
            format!(
                "'{}' is not a currency Meteroid supports: an ISO 4217 code (EUR, USD…)",
                currency.trim()
            ),
        ));
    }
    if input.billing_cycles.is_some_and(|cycles| cycles < 1) {
        return Err(Problem::field(
            format!("{prefix}/billing_cycles"),
            "invalid",
            "A subscription runs one period at least; leave it out to run until cancelled",
        ));
    }
    let options = options_of(&input.options).map_err(|mut problem| {
        for error in &mut problem.errors {
            error.path = format!("{prefix}{}", error.path);
        }
        problem
    })?;
    let aliases: Vec<&str> = options.iter().map(|option| option.alias.as_str()).collect();

    let mut charges = Vec::with_capacity(input.charges.len());
    for (index, charge) in input.charges.iter().enumerate() {
        let path = format!("{prefix}/charges/{index}");
        for (p, price) in charge.prices.iter().enumerate() {
            let option = price.option.trim();
            if option != EVERY_OPTION && !aliases.contains(&option) {
                return Err(Problem::field(
                    format!("{path}/prices/{p}/option"),
                    "not_found",
                    format!("'{option}' is not an option of the version: name one, or `*`"),
                ));
            }
        }
        check_one_cadence(charge, &aliases, &path)?;
        let refs = resolve_refs(state, tenant_id, charge, &path).await?;
        let domain = to_domain(charge, &refs, &path)?;
        charges.push(DraftCharge {
            name: domain.name,
            product_id: domain.product_id,
            fee_structure: domain.structure,
            prices: domain
                .prices
                .into_iter()
                .map(|(price, option)| DraftPrice {
                    price,
                    // A version of one option links every price into it, which reads `*`: the
                    // option named is read so at once.
                    options: if option == EVERY_OPTION || aliases.len() == 1 {
                        DraftPriceOptions::Every
                    } else {
                        DraftPriceOptions::Only(vec![option])
                    },
                })
                .collect(),
        });
    }

    // The version's currency is the one asked, else its prices' (the first); a minimum is in it,
    // never deciding it.
    let currency = input
        .currency
        .as_deref()
        .map(|currency| currency.trim().to_ascii_uppercase())
        .or_else(|| {
            charges
                .iter()
                .flat_map(|charge| charge.prices.iter())
                .map(|price| price.price.currency.clone())
                .next()
        })
        .or_else(|| {
            input
                .minimum
                .as_ref()
                .and_then(|minimum| minimum.currency.as_deref())
                .map(|currency| currency.trim().to_ascii_uppercase())
        });
    if let Some(currency) = currency.as_deref()
        && !charges
            .iter()
            .flat_map(|charge| charge.prices.iter())
            .any(|price| price.price.currency == currency)
        && charges.iter().any(|charge| !charge.prices.is_empty())
    {
        return Err(Problem::field(
            format!("{prefix}/currency"),
            "invalid",
            format!("No price of the version is in {currency}, its currency"),
        ));
    }

    let minimum = match &input.minimum {
        None => None,
        Some(minimum) => {
            if minimum.amount < 0 {
                return Err(Problem::field(
                    format!("{prefix}/minimum/amount"),
                    "invalid",
                    "A minimum is 0 or more",
                ));
            }
            if let (Some(asked), Some(currency)) =
                (minimum.currency.as_deref(), currency.as_deref())
                && !asked.trim().eq_ignore_ascii_case(currency)
            {
                return Err(Problem::field(
                    format!("{prefix}/minimum/currency"),
                    "invalid",
                    format!("A minimum is in the version's currency, {currency}"),
                ));
            }
            let scope = match &minimum.products {
                None => MinimumScope::AllComponents,
                Some(products) => {
                    let mut ids_of: Vec<ProductId> = Vec::with_capacity(products.len());
                    for (index, product) in products.iter().enumerate() {
                        ids_of.push(ids::product_id(state, tenant_id, product).await.map_err(
                            |problem| {
                                if problem.status == 404 {
                                    Problem::field(
                                        format!("{prefix}/minimum/products/{index}"),
                                        "not_found",
                                        format!("product {product} not found"),
                                    )
                                } else {
                                    problem
                                }
                            },
                        )?);
                    }
                    MinimumScope::Products(ids_of)
                }
            };
            let kept = carried.minimum.as_ref().is_some_and(|(amount, stored)| {
                *amount == minimum.amount && stored.same_as(&scope)
            });
            if !kept
                && !minimum_commitments_open(state.feature_resolver.as_ref(), auth.organization_id)
                    .await
            {
                return Err(Problem::feature_not_available(
                    "Minimum commitments are not in your plan: a version may only keep the \
                     minimum it has",
                ));
            }
            Some(DraftMinimum {
                amount_subunit: minimum.amount,
                scope,
            })
        }
    };

    let trial = match &input.trial {
        None => None,
        Some(trial) => Some(PlanTrial {
            duration_days: trial.days,
            trialing_plan_id: match trial.plan.as_deref() {
                None => None,
                Some(plan) => Some(ids::plan_id(state, tenant_id, plan).await?),
            },
            trial_is_free: trial.free,
        }),
    };

    let mut add_ons = Vec::with_capacity(input.add_ons.len());
    for (index, add_on) in input.add_ons.iter().enumerate() {
        let add_on_id = ids::add_on_id(state, tenant_id, &add_on.add_on)
            .await
            .map_err(|problem| {
                if problem.status == 404 {
                    Problem::field(
                        format!("{prefix}/add_ons/{index}/add_on"),
                        "not_found",
                        format!("add-on {} not found", add_on.add_on),
                    )
                } else {
                    problem
                }
            })?;
        add_ons.push(DraftAddOn {
            add_on_id,
            self_serviceable: add_on.self_serviceable,
            max_instances_per_subscription: add_on.max_instances,
        });
    }

    let entitlements = match &input.entitlements {
        None => carried.entitlements,
        Some(inputs) => {
            crate::v2::entitlements::specs(
                state,
                tenant_id,
                inputs,
                Tier::PlanVersion,
                &format!("{prefix}/entitlements"),
            )
            .await?
        }
    };

    Ok(PlanDraft {
        version: PlanVersionNewInternal {
            is_draft_version: true,
            period_start_day: carried.period_start_day,
            net_terms: input.net_terms,
            currency,
            billing_cycles: input.billing_cycles,
            trial,
            entitlements,
            tax_inclusive: input.tax_inclusive,
            credit_exhausted_rule: carried.credit_exhausted_rule,
        },
        options,
        charges,
        add_ons,
        minimum,
    })
}

// ── Handlers ────────────────────────────────────────────────────────

/// List plans
///
/// The tenant's plans in creation order, paged by cursor, each without its versions (read one
/// with `GET /plans/{id_or_code}`). Archived plans are left out unless `status=archived`.
#[utoipa::path(
    get,
    tag = "Plans",
    path = "/api/v2/plans",
    params(PlanListQuery),
    responses(
        (status = 200, description = "A page of the tenant's plans, without their versions", body = PlanList, example = json!(crate::v2::examples::response("list_plans"))),
        (status = 401, description = "Unauthorized", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_plans(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Query(query): Query<PlanListQuery>,
) -> Result<Json<PlanList>, Problem> {
    let family = match query.family.as_deref() {
        Some(family) => Some(ids::product_family_id(&state, auth.tenant_id, family).await?),
        None => None,
    };
    let status = query.status.map(|statuses| {
        statuses
            .into_iter()
            .map(|status| match status {
                PlanStatus::Draft => PlanStatusEnum::Draft,
                PlanStatus::Active => PlanStatusEnum::Active,
                PlanStatus::Inactive => PlanStatusEnum::Inactive,
                PlanStatus::Archived => PlanStatusEnum::Archived,
            })
            .collect()
    });
    let page_query = PageQuery {
        limit: query.limit,
        after: query.after,
    };
    let limit = page_query.limit()?;
    let rows = state
        .store
        .list_plans_after(
            auth.tenant_id,
            family,
            status,
            page_query
                .cursor()?
                .map(|Cursor(after)| PlanId::from(after)),
            limit + 1,
        )
        .await?;
    let paged = keyset(rows, limit, |plan| plan.id.as_uuid());
    let ids_of: Vec<PlanId> = paged.data.iter().map(|plan| plan.id).collect();
    let codes = state.store.plan_codes(auth.tenant_id, &ids_of).await?;
    let mut pages = state
        .store
        .get_plan_pricing_pages(auth.tenant_id, &ids_of)
        .await?;
    let families: HashMap<ProductFamilyId, String> = family_codes(&state, auth.tenant_id).await?;
    let sold_for = sold_for_codes(
        &state,
        auth.tenant_id,
        paged.data.iter().map(|plan| plan.sold_for_resource_type_id),
    )
    .await?;
    let mut data = Vec::with_capacity(paged.data.len());
    for plan in paged.data {
        data.push(Plan {
            id: plan.id,
            code: codes
                .get(&plan.id)
                .cloned()
                .unwrap_or_else(|| plan.id.to_string()),
            name: plan.name,
            description: plan.description,
            plan_type: plan.plan_type.into(),
            status: plan.status.into(),
            family: family_ref(&families, plan.product_family_id),
            listing: match pages.remove(&plan.id) {
                Some(page) => listing_of(page, plan.self_service_rank),
                None => listing(&state, auth.tenant_id, plan.id, plan.self_service_rank).await?,
            },
            sold_for: plan
                .sold_for_resource_type_id
                .map(|id| sold_for_ref(&sold_for, id)),
            active_version: plan.active_version.map(|version| version.version),
            has_draft: plan.has_draft_version,
            created_at: plan.created_at.into(),
            version: None,
        });
    }
    Ok(Json(PlanList {
        data,
        next_cursor: paged.next_cursor,
        has_more: paged.has_more,
    }))
}

/// Get a plan
///
/// The plan's live fields with one of its versions: the active one by default (the draft of a
/// plan never published), `?version=draft`, or `?version=<number>`.
#[utoipa::path(
    get,
    tag = "Plans",
    path = "/api/v2/plans/{id_or_code}",
    params(
        ("id_or_code" = String, Path, description = "Plan id or code"),
        PlanGetQuery,
    ),
    responses(
        (status = 200, description = "The plan and the version asked", body = Plan, example = json!(crate::v2::examples::response("get_plan"))),
        (status = 404, description = "No such plan, or no such version", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "The version uses what v2 cannot express: read it with v1", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_plan(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
    Query(query): Query<PlanGetQuery>,
) -> Result<Json<Plan>, Problem> {
    let plan_id = ids::plan_id(&state, auth.tenant_id, &id_or_code).await?;
    let fp = full_plan(&state, auth.tenant_id, plan_id, query.version.as_deref()).await?;
    Ok(Json(render_plan(&state, auth.tenant_id, &fp, true).await?))
}

/// Create a plan
///
/// The plan with its first version, a draft, in one transaction. Publish it with
/// `POST /api/v2/plans/{id_or_code}/draft/publish`.
#[utoipa::path(
    post,
    tag = "Plans",
    path = "/api/v2/plans",
    request_body(content = CreatePlan, example = json!(crate::v2::examples::request("create_plan"))),
    responses(
        (status = 201, description = "Created, with its draft", body = Plan, example = json!(crate::v2::examples::response("create_plan"))),
        (status = 409, description = "The code is taken", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused, or the model cannot express a charge", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn create_plan(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Body(body): Body<CreatePlan>,
) -> Result<Response, Problem> {
    let name = body.name.trim().to_owned();
    if name.is_empty() {
        return Err(Problem::field("/name", "required", "A plan has a name"));
    }
    crate::v2::bounds::name(&name, "/name")?;
    crate::v2::bounds::description(body.description.as_deref(), "/description")?;
    let family_id = crate::v2::product_families::family_for_new_use(
        &state,
        auth.tenant_id,
        body.family.as_deref(),
        "/family",
    )
    .await?;
    ids::check_code(body.code.as_deref(), PlanId::PREFIX)?;
    let sold_for = match body.sold_for.as_deref() {
        Some(raw) => Some(sold_for_of(&state, auth.tenant_id, raw, "/sold_for").await?),
        None => None,
    };
    let draft = draft_of(&state, &auth, &body.draft, Carried::default(), "/draft").await?;
    let code = body.code.clone().unwrap_or_default();
    let fp = state
        .store
        .create_plan_with_draft(
            auth.as_actor(),
            PlanNew {
                name,
                description: body.description,
                tenant_id: auth.tenant_id,
                product_family_id: family_id,
                plan_type: body.plan_type.into(),
                status: PlanStatusEnum::Draft,
                sold_for_resource_type_id: sold_for,
            },
            body.code,
            draft,
        )
        .await
        .map_err(|error| Problem::at("/draft", error).code_taken(&code))?;
    Ok(committed_write(
        StatusCode::CREATED,
        render_plan(&state, auth.tenant_id, &fp, true).await,
        || known(&fp),
    ))
}

/// Update a plan
///
/// A JSON merge patch of its live fields: name, description, what it is sold for and listing,
/// written together or not at all. Versions are written through the draft, the status by `…/archive` and `…/unarchive`.
#[utoipa::path(
    patch,
    tag = "Plans",
    path = "/api/v2/plans/{id_or_code}",
    params(("id_or_code" = String, Path, description = "Plan id or code")),
    request_body(content = UpdatePlan, content_type = "application/merge-patch+json", example = json!(crate::v2::examples::request("update_plan"))),
    responses(
        (status = 200, description = "Updated, with its active version (else its draft)", body = Plan, example = json!(crate::v2::examples::response("update_plan"))),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "The plan's state refuses the change", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn update_plan(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
    Body(body): Body<UpdatePlan>,
) -> Result<Response, Problem> {
    let plan_id = ids::plan_id(&state, auth.tenant_id, &id_or_code).await?;
    if body
        .name
        .as_deref()
        .is_some_and(|name| name.trim().is_empty())
    {
        return Err(Problem::field("/name", "invalid", "A plan has a name"));
    }
    if let Some(name) = body.name.as_deref() {
        crate::v2::bounds::name(name.trim(), "/name")?;
    }
    crate::v2::bounds::description(
        body.description.as_ref().and_then(|text| text.as_deref()),
        "/description",
    )?;
    let listing_patch = body.listing.clone().unwrap_or_default();
    if listing_patch
        .self_serve_rank
        .flatten()
        .is_some_and(|rank| rank < 1)
    {
        return Err(Problem::field(
            "/listing/self_serve_rank",
            "invalid",
            "A rung is 1 or more",
        ));
    }
    let page_changes = listing_patch.tagline.is_some()
        || listing_patch.popular.is_some()
        || listing_patch.cta_label.is_some()
        || listing_patch.sales_led.is_some()
        || listing_patch.hide_price.is_some()
        || listing_patch.contact_url.is_some();
    let page = if page_changes {
        let mut page = state
            .store
            .get_plan_pricing_page(auth.tenant_id, plan_id)
            .await?;
        if let Some(tagline) = listing_patch.tagline {
            page.tagline = tagline;
        }
        if let Some(popular) = listing_patch.popular {
            page.popular = popular;
        }
        if let Some(cta_label) = listing_patch.cta_label {
            page.cta_label = cta_label;
        }
        if let Some(sales_led) = listing_patch.sales_led {
            page.sales_led = sales_led;
        }
        if let Some(hide_price) = listing_patch.hide_price {
            page.hide_price = hide_price;
        }
        if let Some(contact_url) = listing_patch.contact_url {
            page.contact_url = contact_url;
        }
        Some(page)
    } else {
        None
    };
    let sold_for = match &body.sold_for {
        None => None,
        Some(None) => Some(None),
        Some(Some(raw)) => Some(Some(
            sold_for_of(&state, auth.tenant_id, raw, "/sold_for").await?,
        )),
    };
    // A store refusal points at the one part that could refuse it.
    let pointer = match (sold_for.is_some(), body.listing.is_some()) {
        (true, false) => Some("/sold_for"),
        (false, _) => Some("/listing"),
        (true, true) => None,
    };
    // The live fields, what it is sold for and the copy together, or nothing.
    state
        .store
        .update_plan_listing(
            auth.as_actor(),
            PlanPatch {
                id: plan_id,
                tenant_id: auth.tenant_id,
                name: body.name.map(|name| name.trim().to_owned()),
                description: body.description,
                active_version_id: None,
                self_service_rank: listing_patch.self_serve_rank,
                issues_license: None,
            },
            sold_for,
            page,
        )
        .await
        .map_err(|error| match pointer {
            Some(pointer) => Problem::at(pointer, error),
            None => Problem::from(error),
        })?;
    // The live fields changed whatever the version reads as: one v2 cannot read is left out.
    Ok(committed_write(
        StatusCode::OK,
        plan_now(&state, auth.tenant_id, plan_id).await,
        || known_id(plan_id),
    ))
}

/// Write the draft
///
/// The full desired state of the plan's draft: options, charges, terms and rows. With no draft,
/// one is made with the next version number; a draft there replaces it, keeping its number. What
/// v2 does not write yet (the billing day, what happens once credit runs out), and the rows when
/// left out, are carried over from the version replaced (the draft, else the active version). The
/// same body sent again reads the same.
#[utoipa::path(
    put,
    tag = "Plans",
    path = "/api/v2/plans/{id_or_code}/draft",
    operation_id = "put_plan_draft",
    params(("id_or_code" = String, Path, description = "Plan id or code")),
    request_body(content = VersionInput, example = json!(crate::v2::examples::request("put_plan_draft"))),
    responses(
        (status = 200, description = "The plan with its draft", body = Plan, example = json!(crate::v2::examples::response("put_plan_draft"))),
        (status = 403, description = "Minimum commitments are not in your plan", body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "The plan is archived", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused, or the model cannot express a charge", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn put_draft(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
    Body(body): Body<VersionInput>,
) -> Result<Response, Problem> {
    let plan_id = ids::plan_id(&state, auth.tenant_id, &id_or_code).await?;
    let carried = match full_plan(&state, auth.tenant_id, plan_id, Some("draft")).await {
        Ok(fp) => Carried::of(&fp),
        Err(problem) if problem.status == 404 => {
            match full_plan(&state, auth.tenant_id, plan_id, Some("active")).await {
                Ok(fp) => Carried::of(&fp),
                Err(problem) if problem.status == 404 => {
                    // The plan itself must be there.
                    state
                        .store
                        .get_plan_overview(plan_id, auth.tenant_id)
                        .await?;
                    Carried::default()
                }
                Err(problem) => return Err(problem),
            }
        }
        Err(problem) => return Err(problem),
    };
    let draft = draft_of(&state, &auth, &body, carried, "").await?;
    let fp = state
        .store
        .write_plan_draft(auth.as_actor(), plan_id, auth.tenant_id, draft)
        .await?;
    Ok(committed_write(
        StatusCode::OK,
        render_plan(&state, auth.tenant_id, &fp, true).await,
        || known(&fp),
    ))
}

/// Publish the draft
///
/// Makes the draft the active version. A draft reading the same as the active version is
/// discarded instead, and the active version returned: `200` either way.
#[utoipa::path(
    post,
    tag = "Plans",
    path = "/api/v2/plans/{id_or_code}/draft/publish",
    operation_id = "publish_plan_draft",
    params(("id_or_code" = String, Path, description = "Plan id or code")),
    responses(
        (status = 200, description = "The plan with its active version", body = Plan, example = json!(crate::v2::examples::response("publish_plan_draft"))),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "The plan has no draft", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn publish_draft(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Response, Problem> {
    let plan_id = ids::plan_id(&state, auth.tenant_id, &id_or_code).await?;
    let draft = match full_plan(&state, auth.tenant_id, plan_id, Some("draft")).await {
        Ok(draft) => draft,
        Err(problem) if problem.status == 404 => {
            state
                .store
                .get_plan_overview(plan_id, auth.tenant_id)
                .await?;
            return Err(Problem::conflict(
                ProblemCode::NoDraft,
                "The plan has no draft: write one with PUT /api/v2/plans/{id_or_code}/draft",
            ));
        }
        Err(problem) => return Err(problem),
    };
    let unchanged = match full_plan(&state, auth.tenant_id, plan_id, Some("active")).await {
        Ok(active) => {
            // An active version v2 cannot read (written with v1) is never the same.
            let draft_sheet = render_version(&state, auth.tenant_id, &draft).await;
            let active_sheet = render_version(&state, auth.tenant_id, &active).await;
            match (draft_sheet, active_sheet) {
                (Ok(draft_sheet), Ok(active_sheet)) => same_sheet(&draft_sheet, &active_sheet),
                (Err(problem), _) | (_, Err(problem))
                    if problem.code != ProblemCode::NotExpressible =>
                {
                    return Err(problem);
                }
                _ => false,
            }
        }
        Err(problem) if problem.status == 404 => false,
        Err(problem) => return Err(problem),
    };
    if unchanged {
        state
            .store
            .discard_draft_plan_version(auth.as_actor(), draft.version.id, auth.tenant_id)
            .await?;
    } else {
        state
            .store
            .publish_plan_version(auth.as_actor(), draft.version.id, auth.tenant_id)
            .await?;
    }
    let read = match full_plan(&state, auth.tenant_id, plan_id, Some("active")).await {
        Ok(fp) => render_plan(&state, auth.tenant_id, &fp, true).await,
        Err(problem) => Err(problem),
    };
    Ok(committed_write(StatusCode::OK, read, || known_id(plan_id)))
}

/// Discard the draft
///
/// The plan keeps its published versions; a plan never published goes with its draft.
#[utoipa::path(
    delete,
    tag = "Plans",
    path = "/api/v2/plans/{id_or_code}/draft",
    operation_id = "discard_plan_draft",
    params(("id_or_code" = String, Path, description = "Plan id or code")),
    responses(
        (status = 204, description = "Discarded"),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "The plan has no draft", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn delete_draft(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<StatusCode, Problem> {
    let plan_id = ids::plan_id(&state, auth.tenant_id, &id_or_code).await?;
    let draft = state
        .store
        .get_plan(plan_id, auth.tenant_id, PlanVersionFilter::Draft)
        .await?
        .version
        .filter(|version| version.is_draft_version);
    let Some(draft_id) = draft.map(|version| version.id) else {
        return Err(Problem::conflict(
            ProblemCode::NoDraft,
            "The plan has no draft",
        ));
    };
    state
        .store
        .discard_draft_plan_version(auth.as_actor(), draft_id, auth.tenant_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// List a plan's versions
///
/// Every version of the plan, oldest first, paged by cursor: the published ones and the draft,
/// each without its charges.
#[utoipa::path(
    get,
    tag = "Plans",
    path = "/api/v2/plans/{id_or_code}/versions",
    operation_id = "list_plan_versions",
    params(
        ("id_or_code" = String, Path, description = "Plan id or code"),
        PageQuery,
    ),
    responses(
        (status = 200, description = "A page of the plan's versions, oldest first", body = PlanVersionList, example = json!(crate::v2::examples::response("list_plan_versions"))),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_versions(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
    Query(query): Query<PageQuery>,
) -> Result<Json<PlanVersionList>, Problem> {
    let plan_id = ids::plan_id(&state, auth.tenant_id, &id_or_code).await?;
    let active = state
        .store
        .get_plan_overview(plan_id, auth.tenant_id)
        .await?
        .active_version
        .map(|version| version.id);
    let versions = state
        .store
        .list_plan_versions(plan_id, auth.tenant_id, WHOLE)
        .await?
        .items;
    let paged = page(versions, |version| version.id.as_uuid(), &query)?;
    Ok(Json(
        paged
            .map(|version| PlanVersionSummary {
                number: version.version,
                status: if version.is_draft_version {
                    VersionStatus::Draft
                } else if active == Some(version.id) {
                    VersionStatus::Active
                } else {
                    VersionStatus::Superseded
                },
                currency: version.currency,
                created_at: version.created_at.into(),
            })
            .into(),
    ))
}

/// The plan as it reads after a change of status: with its active version (else its draft), or
/// without when v2 cannot read that version.
async fn plan_now(state: &AppState, tenant_id: TenantId, plan_id: PlanId) -> Result<Plan, Problem> {
    let fp = full_plan(state, tenant_id, plan_id, None).await?;
    match render_plan(state, tenant_id, &fp, true).await {
        Err(problem) if problem.code == ProblemCode::NotExpressible => {
            render_plan(state, tenant_id, &fp, false).await
        }
        rendered => rendered,
    }
}

/// Archive a plan
///
/// No longer sold, and off the self-serve ladder; its subscriptions keep it.
#[utoipa::path(
    post,
    tag = "Plans",
    path = "/api/v2/plans/{id_or_code}/archive",
    params(("id_or_code" = String, Path, description = "Plan id or code")),
    responses(
        (status = 200, description = "Archived", body = Plan, example = json!(crate::v2::examples::response("archive_plan"))),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "The plan's state refuses it", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn archive_plan(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Response, Problem> {
    let plan_id = ids::plan_id(&state, auth.tenant_id, &id_or_code).await?;
    let overview = state
        .store
        .get_plan_overview(plan_id, auth.tenant_id)
        .await?;
    if matches!(overview.status, PlanStatusEnum::Archived) {
        return Ok(Json(plan_now(&state, auth.tenant_id, plan_id).await?).into_response());
    }
    state
        .store
        .archive_plan(auth.as_actor(), plan_id, auth.tenant_id)
        .await?;
    Ok(committed_write(
        StatusCode::OK,
        plan_now(&state, auth.tenant_id, plan_id).await,
        || known_id(plan_id),
    ))
}

/// Unarchive a plan
///
/// Sold again; it does not return to the self-serve ladder by itself.
#[utoipa::path(
    post,
    tag = "Plans",
    path = "/api/v2/plans/{id_or_code}/unarchive",
    params(("id_or_code" = String, Path, description = "Plan id or code")),
    responses(
        (status = 200, description = "Unarchived", body = Plan, example = json!(crate::v2::examples::response("unarchive_plan"))),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn unarchive_plan(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Response, Problem> {
    let plan_id = ids::plan_id(&state, auth.tenant_id, &id_or_code).await?;
    let overview = state
        .store
        .get_plan_overview(plan_id, auth.tenant_id)
        .await?;
    if !matches!(overview.status, PlanStatusEnum::Archived) {
        return Ok(Json(plan_now(&state, auth.tenant_id, plan_id).await?).into_response());
    }
    state.store.unarchive_plan(plan_id, auth.tenant_id).await?;
    Ok(committed_write(
        StatusCode::OK,
        plan_now(&state, auth.tenant_id, plan_id).await,
        || known_id(plan_id),
    ))
}

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_plans, create_plan))
        .routes(routes!(get_plan, update_plan))
        .routes(routes!(archive_plan))
        .routes(routes!(unarchive_plan))
        .routes(routes!(put_draft, delete_draft))
        .routes(routes!(publish_draft))
        .routes(routes!(list_versions))
}
