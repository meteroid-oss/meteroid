//! `/api/v2/coupons`: discounts and promotional credit redeemed by code. A coupon is addressed by
//! its id or its code: the live coupon of that code, else the newest archived one (a code is free
//! again once its coupon is archived).
use crate::AppState;
use crate::v2::Reader;
use crate::v2::extract::{Body, Path, Query};
use crate::v2::idempotency::committed_write;
use crate::v2::ids;
use crate::v2::pagination::{PageQuery, keyset};
use crate::v2::problem::{Problem, ProblemCode};
use axum::Extension;
use axum::Json;
use axum::extract::State;
use axum::response::Response;
use common_domain::ids::{AddOnId, BaseId, CouponId, PlanId, TenantId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_store::domain::coupons::{
    Coupon as DomainCoupon, CouponCreditScope, CouponDiscount, CouponDuration, CouponFilter,
    CouponNew, CouponPatch, CouponStatusPatch, CouponTerms, MAX_COUPON_DURATION_MONTHS,
};
use meteroid_store::domain::eligibility::{
    Eligibility as DomainEligibility, PropertyMatch as DomainPropertyMatch,
};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::CatalogCodesInterface;
use meteroid_store::repositories::add_ons::AddOnInterfaceAuto;
use meteroid_store::repositories::coupons::CouponInterface;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

// ── Model ───────────────────────────────────────────────────────────

/// Which subscriptions a promotional credit pays.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
#[schema(as = CouponCreditScope)]
pub enum CreditScope {
    /// Any of the customer's.
    #[default]
    Customer,
    /// Only the subscription it is redeemed on, ending with it.
    Subscription,
}

/// A percentage off each invoice it discounts.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PercentageDiscount {
    /// Above 0, at most 100.
    #[schema(value_type = String, format = "decimal", example = "20")]
    pub percentage: Decimal,
}

/// An amount off each invoice it discounts, in the subscription's currency.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FixedDiscount {
    pub currency: String,
    /// In the major unit of `currency`.
    #[schema(value_type = String, format = "decimal", example = "10")]
    pub amount: Decimal,
    /// The amount in other currencies, by ISO 4217 code: a subscription in a currency with no
    /// amount cannot redeem it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schema(schema_with = other_amounts)]
    pub amounts: BTreeMap<String, Decimal>,
}

fn other_amounts() -> utoipa::openapi::RefOr<utoipa::openapi::Schema> {
    crate::v2::charges::decimal_map(
        "The amount in other currencies, by ISO 4217 code: a subscription in a currency with no \
         amount cannot redeem it.",
        false,
    )
}

/// Credit added to the customer's balance when the subscription it is redeemed on starts; never
/// an invoice discount. Redeemed once per customer, its code matched whatever the case.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CreditDiscount {
    /// The currency of the subscriptions that may redeem it.
    pub currency: String,
    #[schema(value_type = String, format = "decimal", example = "25")]
    pub amount: Decimal,
    /// Days the credit stays spendable, 1 to 3660; none: it never expires.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_in_days: Option<u32>,
    #[serde(default)]
    pub scope: CreditScope,
}

/// What a coupon takes off: a `percentage` or a `fixed` amount off each invoice it discounts, or
/// promotional `credit` added to the customer's balance.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
#[schema(as = CouponDiscount)]
pub enum Discount {
    Percentage(PercentageDiscount),
    Fixed(FixedDiscount),
    Credit(CreditDiscount),
}

/// Every billing period of the subscription.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ForeverDuration {}

/// The billing periods starting within this many months of the first paid period.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MonthsDuration {
    /// 1 to 120.
    pub months: i32,
}

/// The first invoices of a subscription: how coupons made before durations in months last.
/// Read only.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct InvoicesDuration {
    /// How many invoices it discounts.
    pub invoices: i32,
}

/// How long a coupon discounts a subscription it is redeemed on; a redemption keeps the
/// duration it was made with.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
#[schema(as = CouponDuration)]
pub enum DiscountDuration {
    Forever(ForeverDuration),
    Months(MonthsDuration),
    Invoices(InvoicesDuration),
}

/// What a coupon may be redeemed with. Neither plans nor add-ons: anything. Scoped to add-ons
/// only, it discounts their lines alone.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq, Default)]
#[schema(as = CouponAppliesTo)]
#[serde(deny_unknown_fields)]
pub struct AppliesTo {
    /// By code (id or code on input).
    #[serde(default)]
    pub plans: Vec<String>,
    /// By code (id or code on input).
    #[serde(default)]
    pub add_ons: Vec<String>,
}

/// A merge patch of the scope: a list sent replaces it.
#[derive(Deserialize, ToSchema, Debug, Clone, Default)]
#[schema(as = CouponAppliesToPatch)]
#[serde(deny_unknown_fields)]
pub struct AppliesToPatch {
    /// The plans, by id or code; replaces the list.
    #[serde(default)]
    pub plans: Option<Vec<String>>,
    /// The add-ons, by id or code; replaces the list.
    #[serde(default)]
    pub add_ons: Option<Vec<String>>,
}

/// The customer's custom property `key` holds one of `values`.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[schema(as = CouponPropertyMatch)]
#[serde(deny_unknown_fields)]
pub struct PropertyMatch {
    /// The key of a customer custom property.
    pub key: String,
    /// The values it may hold, one at least.
    pub values: Vec<String>,
}

/// Who may redeem it: every condition set must hold.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq, Default)]
#[schema(as = CouponEligibility)]
#[serde(deny_unknown_fields)]
pub struct Eligibility {
    /// ISO 3166-1 alpha-2 of the customer's billing address; empty: any country.
    #[serde(default)]
    pub countries: Vec<String>,
    /// Custom property values the customer holds, each condition holding; empty: any.
    #[serde(default)]
    pub properties: Vec<PropertyMatch>,
}

/// A discount or promotional credit redeemed by code on a subscription.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct Coupon {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: CouponId,
    /// What customers redeem: exactly, else ignoring the case of ASCII letters when one live
    /// coupon matches so. A promotional credit code is stored upper-case.
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// What it takes off.
    pub discount: Discount,
    /// How long it discounts a subscription it is redeemed on.
    pub duration: DiscountDuration,
    /// The plans and add-ons it may be redeemed with.
    pub applies_to: AppliesTo,
    /// None: anyone with the code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eligibility: Option<Eligibility>,
    /// Not redeemable before; none: from its creation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<ApiDateTime>,
    /// Not redeemable from then on; its redemptions keep discounting. None: it never expires.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<ApiDateTime>,
    /// How many subscriptions may redeem it; none: no limit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redemption_limit: Option<i32>,
    /// One customer may redeem it more than once.
    pub reusable: bool,
    /// How many times it was redeemed.
    pub redemption_count: i32,
    /// None: never redeemed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_redemption_at: Option<ApiDateTime>,
    /// Not redeemable until enabled again; its redemptions keep discounting.
    pub disabled: bool,
    /// Archived: no longer redeemable, its code free for a new coupon.
    pub archived: bool,
    pub created_at: ApiDateTime,
    pub updated_at: ApiDateTime,
}

crate::v2_list!(CouponList, Coupon);

/// A coupon's state as a list filters it: `active`, redeemable (not disabled, expired, used up
/// or archived); `inactive`, disabled, expired or used up but not archived; `archived`.
#[derive(Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CouponStatus {
    /// Redeemable: not disabled, expired, used up or archived.
    Active,
    /// Disabled, expired or used up, not archived.
    Inactive,
    Archived,
}

#[derive(Deserialize, IntoParams, Debug, Clone, Default)]
#[into_params(parameter_in = Query)]
pub struct CouponListQuery {
    /// Only the coupons in one of these statuses (repeat the parameter for several); every
    /// coupon not archived when left out.
    #[param(style = Form, explode)]
    pub status: Option<Vec<CouponStatus>>,
    pub limit: Option<u32>,
    pub after: Option<String>,
}

/// A coupon to create. Its code, redemption limit and `reusable` never change after.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct CreateCoupon {
    /// 1 to 64 ASCII letters, digits, `-` or `_`, unique among live coupons ignoring case.
    pub code: String,
    #[serde(default)]
    pub description: Option<String>,
    pub discount: Discount,
    /// Forever when left out; `months` otherwise (`invoices` is refused).
    #[serde(default)]
    pub duration: Option<DiscountDuration>,
    /// The plans and add-ons it may be redeemed with; anything when left out.
    #[serde(default)]
    pub applies_to: AppliesTo,
    /// Who may redeem it; anyone with the code when left out.
    #[serde(default)]
    pub eligibility: Option<Eligibility>,
    /// Not redeemable before; before `expires_at` when both are sent.
    #[serde(default)]
    pub valid_from: Option<ApiDateTime>,
    /// Not redeemable from then on.
    #[serde(default)]
    pub expires_at: Option<ApiDateTime>,
    /// How many subscriptions may redeem it, 1 at least; no limit when left out.
    #[serde(default)]
    pub redemption_limit: Option<i32>,
    /// Never on promotional credit.
    #[serde(default)]
    pub reusable: bool,
}

/// A merge patch: a field left out is unchanged, `null` clears it. The code, the redemption
/// limit and `reusable` never change; `discount` replaces the discount whole (it cannot switch
/// between promotional credit and an invoice discount).
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct UpdateCoupon {
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub description: Option<Option<String>>,
    #[serde(default)]
    pub discount: Option<Discount>,
    /// `forever` or `months`; past redemptions keep theirs.
    #[serde(default)]
    pub duration: Option<DiscountDuration>,
    #[serde(default)]
    pub applies_to: Option<AppliesToPatch>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<Eligibility>)]
    pub eligibility: Option<Option<Eligibility>>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub valid_from: Option<Option<ApiDateTime>>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub expires_at: Option<Option<ApiDateTime>>,
}

// ── Reading ─────────────────────────────────────────────────────────

fn discount_to_api(coupon: &DomainCoupon) -> Discount {
    match &coupon.discount {
        CouponDiscount::Percentage(percentage) => Discount::Percentage(PercentageDiscount {
            percentage: percentage.normalize(),
        }),
        CouponDiscount::Fixed { currency, amount } => Discount::Fixed(FixedDiscount {
            currency: currency.clone(),
            amount: amount.normalize(),
            amounts: coupon
                .fixed_amounts
                .iter()
                .map(|(currency, amount)| (currency.clone(), amount.normalize()))
                .collect(),
        }),
        CouponDiscount::Credit {
            currency,
            amount,
            expires_in_days,
            scope,
        } => Discount::Credit(CreditDiscount {
            currency: currency.clone(),
            amount: amount.normalize(),
            expires_in_days: *expires_in_days,
            scope: match scope {
                CouponCreditScope::Customer => CreditScope::Customer,
                CouponCreditScope::Subscription => CreditScope::Subscription,
            },
        }),
    }
}

fn duration_to_api(coupon: &DomainCoupon) -> DiscountDuration {
    let count = |n: u32| i32::try_from(n).unwrap_or(i32::MAX);
    match coupon.duration() {
        CouponDuration::Forever => DiscountDuration::Forever(ForeverDuration {}),
        CouponDuration::Months(months) => DiscountDuration::Months(MonthsDuration {
            months: count(months),
        }),
        CouponDuration::Invoices(invoices) => DiscountDuration::Invoices(InvoicesDuration {
            invoices: count(invoices),
        }),
    }
}

/// The codes of the plans and add-ons `coupons` are scoped to.
async fn scope_codes(
    state: &impl Reader,
    tenant_id: TenantId,
    coupons: &[DomainCoupon],
) -> Result<(HashMap<PlanId, String>, HashMap<AddOnId, String>), Problem> {
    let mut plan_ids: Vec<PlanId> = coupons.iter().flat_map(|c| c.plan_ids.clone()).collect();
    plan_ids.sort_by_key(|id| id.as_uuid());
    plan_ids.dedup();
    let mut add_on_ids: Vec<AddOnId> = coupons.iter().flat_map(|c| c.add_on_ids.clone()).collect();
    add_on_ids.sort_by_key(|id| id.as_uuid());
    add_on_ids.dedup();
    let plans = if plan_ids.is_empty() {
        HashMap::new()
    } else {
        state.store().plan_codes(tenant_id, &plan_ids).await?
    };
    let add_ons = if add_on_ids.is_empty() {
        HashMap::new()
    } else {
        state
            .store()
            .list_add_ons_by_ids(tenant_id, add_on_ids)
            .await?
            .into_iter()
            .map(|add_on| (add_on.id, add_on.code))
            .collect()
    };
    Ok((plans, add_ons))
}

fn to_api(
    coupon: DomainCoupon,
    plans: &HashMap<PlanId, String>,
    add_ons: &HashMap<AddOnId, String>,
) -> Coupon {
    let discount = discount_to_api(&coupon);
    let duration = duration_to_api(&coupon);
    Coupon {
        id: coupon.id,
        applies_to: AppliesTo {
            plans: coupon
                .plan_ids
                .iter()
                .map(|id| plans.get(id).cloned().unwrap_or_else(|| id.to_string()))
                .collect(),
            add_ons: coupon
                .add_on_ids
                .iter()
                .map(|id| add_ons.get(id).cloned().unwrap_or_else(|| id.to_string()))
                .collect(),
        },
        eligibility: coupon.eligibility.map(|rule| Eligibility {
            countries: rule.countries,
            properties: rule
                .properties
                .into_iter()
                .map(|p| PropertyMatch {
                    key: p.key,
                    values: p.values,
                })
                .collect(),
        }),
        code: coupon.code,
        description: Some(coupon.description).filter(|d| !d.is_empty()),
        discount,
        duration,
        valid_from: coupon.valid_from.map(Into::into),
        expires_at: coupon.expires_at.map(Into::into),
        redemption_limit: coupon.redemption_limit,
        reusable: coupon.reusable,
        redemption_count: coupon.redemption_count,
        last_redemption_at: coupon.last_redemption_at.map(Into::into),
        disabled: coupon.disabled,
        archived: coupon.archived_at.is_some(),
        created_at: coupon.created_at.into(),
        updated_at: coupon.updated_at.into(),
    }
}

pub(crate) async fn one(
    state: &impl Reader,
    tenant_id: TenantId,
    coupon: DomainCoupon,
) -> Result<Coupon, Problem> {
    let (plans, add_ons) = scope_codes(state, tenant_id, std::slice::from_ref(&coupon)).await?;
    Ok(to_api(coupon, &plans, &add_ons))
}

/// A coupon written, as it reads; its scope by id when the codes are not read back (the write is
/// made all the same: a write committed never answers an error).
async fn written(
    state: &AppState,
    tenant_id: TenantId,
    status: StatusCode,
    coupon: DomainCoupon,
) -> Response {
    let known = coupon.clone();
    committed_write(status, one(state, tenant_id, coupon).await, || {
        to_api(known, &HashMap::new(), &HashMap::new())
    })
}

fn parse_id(raw: &str) -> Option<CouponId> {
    CouponId::parse_uuid(raw).ok().or_else(|| {
        raw.starts_with(CouponId::PREFIX)
            .then(|| CouponId::parse_base62(raw).ok())
            .flatten()
    })
}

/// The tenant's coupon of an id, else of a code: the live coupon redeemed by it, else the newest
/// archived coupon with exactly that code.
pub async fn coupon(
    state: &AppState,
    tenant_id: TenantId,
    raw: &str,
) -> Result<DomainCoupon, Problem> {
    let raw = raw.trim();
    if let Some(id) = parse_id(raw) {
        match state.store.get_listed_coupon(tenant_id, id).await {
            Ok(coupon) => return Ok(coupon),
            // A code may read as an id: it is tried as a code next.
            Err(error) if matches!(error.current_context(), StoreError::ValueNotFound(_)) => {}
            Err(error) => return Err(error.into()),
        }
    }
    if let Some(live) = state
        .store
        .list_coupons_by_codes(tenant_id, &[raw.to_owned()])
        .await?
        .into_iter()
        .next()
    {
        return Ok(live);
    }
    state
        .store
        .find_archived_coupon_by_code(tenant_id, raw)
        .await?
        .ok_or_else(|| Problem::not_found(format!("coupon {raw}")))
}

// ── Writing ─────────────────────────────────────────────────────────

fn check_code(code: &str) -> Result<(), Problem> {
    let valid = (1..=64).contains(&code.chars().count())
        && code
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if valid {
        Ok(())
    } else {
        Err(Problem::field(
            "/code",
            "invalid",
            "A coupon code is 1 to 64 ASCII letters, digits, `-` or `_`",
        ))
    }
}

fn currency(raw: &str, path: &str) -> Result<String, Problem> {
    let code = raw.trim().to_ascii_uppercase();
    if code.len() == 3 && code.chars().all(|c| c.is_ascii_uppercase()) {
        Ok(code)
    } else {
        Err(Problem::field(path, "invalid", "An ISO 4217 currency code"))
    }
}

/// The discount and, for a fixed one, its amounts in other currencies.
fn discount_to_domain(
    discount: &Discount,
    path: &str,
) -> Result<(CouponDiscount, BTreeMap<String, Decimal>), Problem> {
    match discount {
        Discount::Percentage(p) => {
            if p.percentage <= Decimal::ZERO || p.percentage > Decimal::ONE_HUNDRED {
                return Err(Problem::field(
                    format!("{path}/percentage"),
                    "invalid",
                    "A percentage is above 0 and at most 100",
                ));
            }
            Ok((CouponDiscount::Percentage(p.percentage), BTreeMap::new()))
        }
        Discount::Fixed(f) => {
            let own = currency(&f.currency, &format!("{path}/currency"))?;
            if f.amount <= Decimal::ZERO {
                return Err(Problem::field(
                    format!("{path}/amount"),
                    "invalid",
                    "The amount is positive",
                ));
            }
            let mut amounts = BTreeMap::new();
            for (code, amount) in &f.amounts {
                let at = format!("{path}/amounts/{code}");
                let code = currency(code, &at)?;
                if code == own {
                    return Err(Problem::field(
                        at,
                        "invalid",
                        format!("The discount is already in {own}: `amount`"),
                    ));
                }
                if *amount <= Decimal::ZERO {
                    return Err(Problem::field(at, "invalid", "The amount is positive"));
                }
                amounts.insert(code, *amount);
            }
            Ok((
                CouponDiscount::Fixed {
                    currency: own,
                    amount: f.amount,
                },
                amounts,
            ))
        }
        Discount::Credit(c) => {
            let discount = CouponDiscount::Credit {
                currency: currency(&c.currency, &format!("{path}/currency"))?,
                amount: c.amount,
                expires_in_days: c.expires_in_days,
                scope: match c.scope {
                    CreditScope::Customer => CouponCreditScope::Customer,
                    CreditScope::Subscription => CouponCreditScope::Subscription,
                },
            };
            discount.validate().map_err(|error| {
                Problem::field(path, "invalid", crate::v2::problem::store_message(&error))
            })?;
            Ok((discount, BTreeMap::new()))
        }
    }
}

/// `None`: forever; the months otherwise. Counting invoices is refused.
fn months_of(duration: &DiscountDuration, path: &str) -> Result<Option<i32>, Problem> {
    match duration {
        DiscountDuration::Forever(_) => Ok(None),
        DiscountDuration::Months(m) => {
            if !(1..=MAX_COUPON_DURATION_MONTHS).contains(&m.months) {
                return Err(Problem::field(
                    format!("{path}/months"),
                    "invalid",
                    format!("A coupon lasts 1 to {MAX_COUPON_DURATION_MONTHS} months"),
                ));
            }
            Ok(Some(m.months))
        }
        DiscountDuration::Invoices(_) => Err(Problem::field(
            format!("{path}/type"),
            "invalid",
            "A coupon lasts `forever` or a number of `months`; `invoices` is how coupons made \
             before durations in months read",
        )),
    }
}

fn eligibility_to_domain(rule: Eligibility, path: &str) -> Result<DomainEligibility, Problem> {
    let rule = DomainEligibility {
        countries: rule.countries,
        properties: rule
            .properties
            .into_iter()
            .map(|p| DomainPropertyMatch {
                key: p.key,
                values: p.values,
            })
            .collect(),
    };
    rule.clone()
        .normalized()
        .validate()
        .map_err(|message| Problem::field(path, "invalid", message))?;
    Ok(rule)
}

async fn plan_ids_of(
    state: &AppState,
    tenant_id: TenantId,
    plans: &[String],
    path: &str,
) -> Result<Vec<PlanId>, Problem> {
    let mut found = Vec::with_capacity(plans.len());
    for (index, plan) in plans.iter().enumerate() {
        let id = ids::plan_id(state, tenant_id, plan)
            .await
            .map_err(|problem| {
                if problem.status == 404 {
                    Problem::field(
                        format!("{path}/plans/{index}"),
                        "not_found",
                        format!("plan {plan} not found"),
                    )
                } else {
                    problem
                }
            })?;
        if !found.contains(&id) {
            found.push(id);
        }
    }
    Ok(found)
}

async fn add_on_ids_of(
    state: &AppState,
    tenant_id: TenantId,
    add_ons: &[String],
    path: &str,
) -> Result<Vec<AddOnId>, Problem> {
    let mut found = Vec::with_capacity(add_ons.len());
    for (index, add_on) in add_ons.iter().enumerate() {
        let id = ids::add_on_id(state, tenant_id, add_on)
            .await
            .map_err(|problem| {
                if problem.status == 404 {
                    Problem::field(
                        format!("{path}/add_ons/{index}"),
                        "not_found",
                        format!("add-on {add_on} not found"),
                    )
                } else {
                    problem
                }
            })?;
        if !found.contains(&id) {
            found.push(id);
        }
    }
    Ok(found)
}

fn refuse_valid_after_expiry(
    valid_from: Option<chrono::NaiveDateTime>,
    expires_at: Option<chrono::NaiveDateTime>,
) -> Result<(), Problem> {
    if let (Some(from), Some(until)) = (valid_from, expires_at)
        && from >= until
    {
        return Err(Problem::field(
            "/valid_from",
            "invalid",
            "A coupon becomes valid before it expires",
        ));
    }
    Ok(())
}

async fn set_status(
    state: &AppState,
    auth: &AuthorizedAsTenant,
    id_or_code: &str,
    archived_at: Option<Option<chrono::NaiveDateTime>>,
    disabled: Option<bool>,
) -> Result<Response, Problem> {
    let current = coupon(state, auth.tenant_id, id_or_code).await?;
    if archived_at == Some(None) && current.archived_at.is_some() {
        // Its code may have gone to a live coupon since.
        let holder = state
            .store
            .list_coupons_by_codes(auth.tenant_id, std::slice::from_ref(&current.code))
            .await?
            .into_iter()
            .find(|live| live.id != current.id);
        if let Some(holder) = holder {
            return Err(Problem::conflict(
                ProblemCode::CodeTaken,
                format!(
                    "The live coupon {} holds the code '{}': archive it first",
                    holder.id, current.code
                ),
            ));
        }
    }
    let updated = state
        .store
        .update_coupon_status(
            auth.as_actor(),
            CouponStatusPatch {
                id: current.id,
                tenant_id: auth.tenant_id,
                archived_at,
                disabled,
            },
        )
        .await?;
    Ok(written(state, auth.tenant_id, StatusCode::OK, updated).await)
}

// ── Handlers ────────────────────────────────────────────────────────

/// List coupons
///
/// The tenant's coupons in creation order, paged by cursor: every coupon not archived, or those
/// of `status`.
#[utoipa::path(
    get,
    tag = "Coupons",
    path = "/api/v2/coupons",
    params(CouponListQuery),
    responses(
        (status = 200, description = "A page of the tenant's coupons, archived ones left out unless asked", body = CouponList),
        (status = 401, description = "Unauthorized", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_coupons(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Query(query): Query<CouponListQuery>,
) -> Result<Json<CouponList>, Problem> {
    let page = PageQuery {
        limit: query.limit,
        after: query.after,
    };
    let limit = page.limit()?;
    let cursor = page.cursor()?;
    let filter = query.status.map(|statuses| {
        statuses
            .into_iter()
            .map(|status| match status {
                CouponStatus::Active => CouponFilter::ACTIVE,
                CouponStatus::Inactive => CouponFilter::INACTIVE,
                CouponStatus::Archived => CouponFilter::ARCHIVED,
            })
            .collect()
    });
    let rows = state
        .store
        .list_coupons_after(
            auth.tenant_id,
            filter,
            cursor.map(|cursor| CouponId::from(cursor.0)),
            limit + 1,
        )
        .await?;
    let paged = keyset(rows, limit, |coupon| coupon.id.as_uuid());
    let (plans, add_ons) = scope_codes(&state, auth.tenant_id, &paged.data).await?;
    Ok(Json(
        paged.map(|coupon| to_api(coupon, &plans, &add_ons)).into(),
    ))
}

/// Get a coupon
///
/// By id, or by code: the live coupon of that code, else the newest archived one.
#[utoipa::path(
    get,
    tag = "Coupons",
    path = "/api/v2/coupons/{id_or_code}",
    params(("id_or_code" = String, Path, description = "Coupon id or code")),
    responses(
        (status = 200, description = "The coupon", body = Coupon),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_coupon(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Json<Coupon>, Problem> {
    let found = coupon(&state, auth.tenant_id, &id_or_code).await?;
    Ok(Json(one(&state, auth.tenant_id, found).await?))
}

/// Create a coupon
///
/// Answers the coupon created, enabled. A code a live coupon holds, ignoring case, is refused
/// (`409 code_taken`). Promotional credit takes no duration and is never `reusable`; a
/// `valid_from` after `expires_at` is refused.
#[utoipa::path(
    post,
    tag = "Coupons",
    path = "/api/v2/coupons",
    request_body = CreateCoupon,
    responses(
        (status = 201, description = "Created", body = Coupon),
        (status = 409, description = "A live coupon holds the code", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn create_coupon(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Body(body): Body<CreateCoupon>,
) -> Result<Response, Problem> {
    let tenant_id = auth.tenant_id;
    let code = body.code.trim().to_owned();
    check_code(&code)?;
    crate::v2::bounds::description(body.description.as_deref(), "/description")?;
    let (discount, fixed_amounts) = discount_to_domain(&body.discount, "/discount")?;
    let duration_months = match &body.duration {
        None => None,
        Some(duration) => months_of(duration, "/duration")?,
    };
    if discount.is_credit() {
        if duration_months.is_some() {
            return Err(Problem::field(
                "/duration",
                "invalid",
                "Promotional credit has no duration",
            ));
        }
        if body.reusable {
            return Err(Problem::field(
                "/reusable",
                "invalid",
                "Promotional credit is redeemed once per customer: it is never reusable",
            ));
        }
    }
    if body.redemption_limit.is_some_and(|limit| limit < 1) {
        return Err(Problem::field(
            "/redemption_limit",
            "invalid",
            "A redemption limit is 1 at least",
        ));
    }
    let valid_from = body.valid_from.map(ApiDateTime::naive_utc);
    let expires_at = body.expires_at.map(ApiDateTime::naive_utc);
    refuse_valid_after_expiry(valid_from, expires_at)?;
    let eligibility = body
        .eligibility
        .map(|rule| eligibility_to_domain(rule, "/eligibility"))
        .transpose()?;
    let plan_ids = plan_ids_of(&state, tenant_id, &body.applies_to.plans, "/applies_to").await?;
    let add_on_ids =
        add_on_ids_of(&state, tenant_id, &body.applies_to.add_ons, "/applies_to").await?;
    let created = state
        .store
        .create_coupon_with_terms(
            auth.as_actor(),
            CouponNew {
                code: code.clone(),
                description: body.description.unwrap_or_default(),
                tenant_id,
                discount,
                expires_at,
                redemption_limit: body.redemption_limit,
                recurring_value: None,
                reusable: body.reusable,
                plan_ids,
            },
            CouponTerms {
                add_on_ids,
                eligibility,
                valid_from,
                fixed_amounts,
                duration_months,
            },
        )
        .await
        .map_err(|error| Problem::from(error).code_taken(&code))?;
    Ok(written(&state, tenant_id, StatusCode::CREATED, created).await)
}

/// Update a coupon
///
/// A JSON merge patch. The code, the redemption limit and `reusable` never change. A change of
/// discount or duration reaches later redemptions only, and is refused while an open quote
/// offers the coupon.
#[utoipa::path(
    patch,
    tag = "Coupons",
    path = "/api/v2/coupons/{id_or_code}",
    params(("id_or_code" = String, Path, description = "Coupon id or code")),
    request_body(content = UpdateCoupon, content_type = "application/merge-patch+json"),
    responses(
        (status = 200, description = "Updated", body = Coupon),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "An open quote offers the coupon", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn update_coupon(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
    Body(body): Body<UpdateCoupon>,
) -> Result<Response, Problem> {
    let tenant_id = auth.tenant_id;
    crate::v2::bounds::description(
        body.description.as_ref().and_then(|text| text.as_deref()),
        "/description",
    )?;
    let current = coupon(&state, tenant_id, &id_or_code).await?;
    let (discount, fixed_amounts) = match &body.discount {
        None => (None, None),
        Some(discount) => {
            let (discount, amounts) = discount_to_domain(discount, "/discount")?;
            // A fixed discount carries its amounts in other currencies; another clears them.
            let amounts = matches!(discount, CouponDiscount::Fixed { .. }).then_some(amounts);
            (Some(discount), amounts)
        }
    };
    let duration_months = body
        .duration
        .as_ref()
        .map(|duration| months_of(duration, "/duration"))
        .transpose()?;
    let valid_from = body
        .valid_from
        .map(|value| value.map(ApiDateTime::naive_utc));
    let expires_at = body
        .expires_at
        .map(|value| value.map(ApiDateTime::naive_utc));
    refuse_valid_after_expiry(
        valid_from.unwrap_or(current.valid_from),
        expires_at.unwrap_or(current.expires_at),
    )?;
    let eligibility = match body.eligibility {
        None => None,
        Some(None) => Some(None),
        Some(Some(rule)) => Some(Some(eligibility_to_domain(rule, "/eligibility")?)),
    };
    let scope = body.applies_to.unwrap_or_default();
    let plan_ids = match &scope.plans {
        None => None,
        Some(plans) => Some(plan_ids_of(&state, tenant_id, plans, "/applies_to").await?),
    };
    let add_on_ids = match &scope.add_ons {
        None => None,
        Some(add_ons) => Some(add_on_ids_of(&state, tenant_id, add_ons, "/applies_to").await?),
    };
    let updated = state
        .store
        .update_coupon(
            auth.as_actor(),
            CouponPatch {
                id: current.id,
                tenant_id,
                description: body
                    .description
                    .map(|description| description.unwrap_or_default()),
                discount,
                plan_ids,
                add_on_ids,
                eligibility,
                valid_from,
                expires_at,
                fixed_amounts,
                duration_months,
            },
        )
        .await?;
    Ok(written(&state, tenant_id, StatusCode::OK, updated).await)
}

/// Archive a coupon
///
/// It can no longer be redeemed, and its code is free for a new coupon; its redemptions keep
/// discounting.
#[utoipa::path(
    post,
    tag = "Coupons",
    path = "/api/v2/coupons/{id_or_code}/archive",
    params(("id_or_code" = String, Path, description = "Coupon id or code")),
    responses(
        (status = 200, description = "Archived", body = Coupon),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn archive_coupon(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Response, Problem> {
    let now = common_clock::now().naive_utc();
    set_status(&state, &auth, &id_or_code, Some(Some(now)), None).await
}

/// Unarchive a coupon
///
/// Refused while a live coupon holds its code.
#[utoipa::path(
    post,
    tag = "Coupons",
    path = "/api/v2/coupons/{id_or_code}/unarchive",
    params(("id_or_code" = String, Path, description = "Coupon id or code")),
    responses(
        (status = 200, description = "Unarchived", body = Coupon),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "A live coupon holds its code", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn unarchive_coupon(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Response, Problem> {
    set_status(&state, &auth, &id_or_code, Some(None), None).await
}

/// Disable a coupon
///
/// Not redeemable until enabled again; its redemptions keep discounting.
#[utoipa::path(
    post,
    tag = "Coupons",
    path = "/api/v2/coupons/{id_or_code}/disable",
    params(("id_or_code" = String, Path, description = "Coupon id or code")),
    responses(
        (status = 200, description = "Disabled", body = Coupon),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn disable_coupon(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Response, Problem> {
    set_status(&state, &auth, &id_or_code, None, Some(true)).await
}

/// Enable a coupon
///
/// Redeemable again, unless it is expired, used up or archived. Enabling an enabled coupon
/// changes nothing.
#[utoipa::path(
    post,
    tag = "Coupons",
    path = "/api/v2/coupons/{id_or_code}/enable",
    params(("id_or_code" = String, Path, description = "Coupon id or code")),
    responses(
        (status = 200, description = "Enabled", body = Coupon),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn enable_coupon(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Response, Problem> {
    set_status(&state, &auth, &id_or_code, None, Some(false)).await
}

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_coupons, create_coupon))
        .routes(routes!(get_coupon, update_coupon))
        .routes(routes!(archive_coupon))
        .routes(routes!(unarchive_coupon))
        .routes(routes!(disable_coupon))
        .routes(routes!(enable_coupon))
}
