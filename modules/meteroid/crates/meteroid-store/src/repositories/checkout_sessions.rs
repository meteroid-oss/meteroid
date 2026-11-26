use crate::domain::checkout_sessions::{CheckoutSessionStatus, CheckoutType};
use crate::domain::coupons::Coupon;
use crate::domain::eligibility::{self, CustomerFacts, Eligibility};
use crate::domain::plan_options::{PlanOption, PlanOptionRef};
use crate::domain::price_cells::{resolve_option, sold_options};
use crate::domain::usage_levels::settle_levels;
use crate::domain::{
    CheckoutSession, CreateCheckoutSession, CreateSubscriptionComponents, Customer, PriceComponent,
};
use crate::errors::StoreError;
use crate::repositories::price_components::PriceComponentInterface;
use crate::services::subscriptions::currency::{
    SubscriptionCurrency, asked_cadences, plan_options_of, pricing,
};
use crate::store::PgConn;
use crate::{Store, StoreResult};
use chrono::Duration;
use common_domain::ids::{
    AddOnId, CheckoutSessionId, CustomerId, PlanId, PlanVersionId, SubscriptionId, TenantId,
};
use diesel_models::checkout_sessions::{CheckoutSessionRow, CheckoutSessionRowNew};
use diesel_models::coupons::CouponRow;
use diesel_models::customers::CustomerRow;
use diesel_models::enums::CheckoutSessionStatusEnum;
use diesel_models::plan_versions::PlanVersionRow;
use diesel_models::plans::PlanRow;
use diesel_models::query::coupons::CodeMatch;
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;

impl Store {
    /// The metrics the subscription a self-serve checkout creates prices: the plan's charges it
    /// keeps, its extra charges and its add-ons.
    async fn checkout_metric_ids(
        &self,
        conn: &mut PgConn,
        params: &CreateCheckoutSession,
        plan_components: &[PriceComponent],
    ) -> StoreResult<Vec<common_domain::ids::BillableMetricId>> {
        use crate::domain::price_components::PriceEntry;
        use crate::domain::prices::FeeStructure;
        use crate::repositories::add_ons::AddOnInterface;
        use common_domain::ids::BaseId;
        let asked = params.components.as_ref();
        let removed = asked.map_or(&[][..], |c| c.remove_components.as_slice());
        let mut metric_ids = plan_components
            .iter()
            .filter(|component| !removed.contains(&component.id))
            .filter_map(|component| component.structure().and_then(FeeStructure::metric_id))
            .collect::<Vec<_>>();
        for extra in asked.iter().flat_map(|c| &c.extra_components) {
            let structure = match extra
                .fee_structure
                .as_ref()
                .or_else(|| extra.product_ref.new_product_structure())
            {
                Some(structure) => Some(structure.clone()),
                None => match &extra.price_entry {
                    PriceEntry::Existing(price_id) => {
                        crate::repositories::price_components::linked_charge_structure(
                            conn,
                            params.tenant_id,
                            *price_id,
                        )
                        .await?
                    }
                    PriceEntry::New(_) => None,
                },
            };
            metric_ids.extend(structure.as_ref().and_then(FeeStructure::metric_id));
        }
        let add_on_ids = params
            .add_ons
            .iter()
            .flat_map(|bought| bought.add_ons.iter().map(|add_on| add_on.add_on_id))
            .collect::<Vec<_>>();
        if !add_on_ids.is_empty() {
            let add_ons = self
                .list_add_ons_by_ids_with_conn(conn, params.tenant_id, add_on_ids)
                .await?;
            metric_ids.extend(
                add_ons
                    .iter()
                    .filter_map(|add_on| add_on.fee_structure.as_ref())
                    .filter_map(FeeStructure::metric_id),
            );
        }
        metric_ids.sort_by_key(|id| id.as_uuid());
        metric_ids.dedup();
        Ok(metric_ids)
    }
}

/// How long a plan-change checkout waits on the customer to authenticate its payment (3DS, a
/// redirect) before it expires, as card issuers abandon the authentication well within it.
const PLAN_CHANGE_AUTHENTICATION_WINDOW: Duration = Duration::hours(24);

#[async_trait::async_trait]
pub trait CheckoutSessionsInterface {
    async fn create_checkout_session(
        &self,
        params: CreateCheckoutSession,
    ) -> StoreResult<CheckoutSession>;

    async fn get_checkout_session(
        &self,
        tenant_id: TenantId,
        id: CheckoutSessionId,
    ) -> StoreResult<CheckoutSession>;

    async fn get_checkout_session_by_subscription(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<CheckoutSession>;

    async fn list_checkout_sessions(
        &self,
        tenant_id: TenantId,
        customer_id: Option<CustomerId>,
        status: Option<CheckoutSessionStatus>,
    ) -> StoreResult<Vec<CheckoutSession>>;

    /// A page of the tenant's plan checkouts by keyset (API v2): in id order, those after
    /// `after`, at most `limit`. Top-up checkouts are left out.
    async fn list_checkout_sessions_after(
        &self,
        tenant_id: TenantId,
        customer_id: Option<CustomerId>,
        status: Option<Vec<CheckoutSessionStatus>>,
        after: Option<CheckoutSessionId>,
        limit: usize,
    ) -> StoreResult<Vec<CheckoutSession>>;

    async fn cancel_checkout_session(
        &self,
        tenant_id: TenantId,
        id: CheckoutSessionId,
    ) -> StoreResult<CheckoutSession>;

    async fn expire_sessions(&self) -> StoreResult<usize>;

    async fn cleanup_old_sessions(&self, older_than_days: u32) -> StoreResult<usize>;
}

#[async_trait::async_trait]
impl CheckoutSessionsInterface for Store {
    async fn create_checkout_session(
        &self,
        params: CreateCheckoutSession,
    ) -> StoreResult<CheckoutSession> {
        let mut conn = self.get_conn().await?;

        // Refused now rather than ignored when the customer pays.
        if params.coupon_ids.is_empty()
            && let Some(code) = params.coupon_code.as_deref()
        {
            let codes = [code.to_owned()];
            let rows = CouponRow::candidates_by_codes(&mut conn, params.tenant_id, &codes)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            crate::repositories::coupons::match_coupon_codes(&codes, &rows)?;
        }

        let mut params = params;
        let priced = matches!(
            params.checkout_type,
            CheckoutType::SelfServe | CheckoutType::PlanChange
        );
        // Only a self-serve checkout creates a subscription for its target.
        let creates = params.checkout_type == CheckoutType::SelfServe;
        let mut metric_ids = Vec::new();
        if priced || redeemed_code(&params).is_some() {
            let components = self
                .list_price_components_with_conn(
                    &mut conn,
                    params.plan_version_id,
                    params.tenant_id,
                )
                .await?;
            let version_currency = PlanVersionRow::find_by_id_and_tenant_id(
                &mut conn,
                params.plan_version_id,
                params.tenant_id,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .currency;
            let options = plan_options_of(&mut conn, params.tenant_id, &[params.plan_version_id])
                .await?
                .remove(&params.plan_version_id)
                .unwrap_or_default();
            let sold = checkout_offer(&mut conn, &params, &version_currency, &options, &components)
                .await?;
            let currency = sold.currency.clone();
            if priced {
                price_in_currency(
                    &mut conn,
                    &mut params,
                    &components,
                    sold.option.as_ref(),
                    &currency,
                    &version_currency,
                )
                .await?;
            }
            // And the checks its completion runs on who the customer is, in that currency.
            refuse_unpurchasable(&mut conn, &params, &currency).await?;
            if creates {
                metric_ids = self
                    .checkout_metric_ids(&mut conn, &params, &components)
                    .await?;
            }
        }

        use crate::repositories::subscriptions::usage_attribution as attribution;
        let mut resource_type_id = None;
        if let Some(resource_id) = params.resource_id {
            resource_type_id = Some(
                attribution::ensure_resource_for(
                    &mut conn,
                    params.tenant_id,
                    params.customer_id,
                    resource_id,
                )
                .await?
                .resource_type_id,
            );
            attribution::ensure_resource_unsold(
                &mut conn,
                params.tenant_id,
                resource_id,
                params
                    .billing_start_date
                    .unwrap_or_else(|| common_clock::now().date_naive()),
                params.end_date,
                None,
            )
            .await?;
        }
        let warnings = if creates {
            attribution::sale_warnings(
                &mut conn,
                params.tenant_id,
                params.plan_version_id,
                resource_type_id,
                &metric_ids,
            )
            .await?
        } else {
            crate::domain::SaleWarnings::default()
        };
        let row_new: CheckoutSessionRowNew = params.try_into_row()?;

        let row = row_new
            .insert(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(CheckoutSession {
            warnings: warnings.codes,
            metrics_not_attributed: warnings.metrics_not_attributed,
            ..row.into()
        })
    }

    async fn get_checkout_session(
        &self,
        tenant_id: TenantId,
        id: CheckoutSessionId,
    ) -> StoreResult<CheckoutSession> {
        let mut conn = self.get_conn().await?;

        let row = CheckoutSessionRow::get_by_id(&mut conn, tenant_id, id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(row.into())
    }

    async fn get_checkout_session_by_subscription(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<CheckoutSession> {
        let mut conn = self.get_conn().await?;

        let row = CheckoutSessionRow::get_by_subscription(&mut conn, tenant_id, subscription_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .ok_or_else(|| {
                Report::new(StoreError::ValueNotFound(
                    "No checkout session found for this subscription".to_string(),
                ))
            })?;

        Ok(row.into())
    }

    async fn list_checkout_sessions(
        &self,
        tenant_id: TenantId,
        customer_id: Option<CustomerId>,
        status: Option<CheckoutSessionStatus>,
    ) -> StoreResult<Vec<CheckoutSession>> {
        let mut conn = self.get_conn().await?;

        let db_status = status.map(status_domain_to_db);

        let rows = CheckoutSessionRow::list(&mut conn, tenant_id, customer_id, db_status)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn list_checkout_sessions_after(
        &self,
        tenant_id: TenantId,
        customer_id: Option<CustomerId>,
        status: Option<Vec<CheckoutSessionStatus>>,
        after: Option<CheckoutSessionId>,
        limit: usize,
    ) -> StoreResult<Vec<CheckoutSession>> {
        let mut conn = self.get_conn().await?;

        let rows = CheckoutSessionRow::list_page(
            &mut conn,
            tenant_id,
            customer_id,
            status.map(|statuses| statuses.into_iter().map(status_domain_to_db).collect()),
            after,
            i64::try_from(limit).unwrap_or(i64::MAX),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn cancel_checkout_session(
        &self,
        tenant_id: TenantId,
        id: CheckoutSessionId,
    ) -> StoreResult<CheckoutSession> {
        let mut conn = self.get_conn().await?;

        let row = CheckoutSessionRow::mark_cancelled(&mut conn, tenant_id, id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .ok_or_else(|| {
                Report::new(StoreError::InvalidArgument(
                    "Checkout session not found or cannot be cancelled".to_string(),
                ))
            })?;

        Ok(row.into())
    }

    async fn expire_sessions(&self) -> StoreResult<usize> {
        let mut conn = self.get_conn().await?;

        let now = common_clock::now();

        let expired = CheckoutSessionRow::mark_expired_batch(&mut conn, now)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let stale = CheckoutSessionRow::expire_stale_plan_change_checkouts(
            &mut conn,
            now - PLAN_CHANGE_AUTHENTICATION_WINDOW,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        Ok(expired + stale)
    }

    async fn cleanup_old_sessions(&self, older_than_days: u32) -> StoreResult<usize> {
        let mut conn = self.get_conn().await?;

        let older_than = common_clock::now() - Duration::days(older_than_days as i64);

        CheckoutSessionRow::delete_old(&mut conn, older_than)
            .await
            .map_err(Into::<Report<StoreError>>::into)
    }
}

fn invalid(message: String) -> Report<StoreError> {
    Report::new(StoreError::InvalidArgument(message))
}

/// Settles the option a purchase buys, kept by its alias; refused when the plan does not sell it
/// in `currency`.
async fn price_in_currency(
    conn: &mut PgConn,
    params: &mut CreateCheckoutSession,
    components: &[PriceComponent],
    option: Option<&PlanOption>,
    currency: &str,
    version_currency: &str,
) -> StoreResult<()> {
    params.option = option.map(|option| option.alias.clone());
    // A plan with no price of its own (free, legacy): subscription creation decides.
    if components.iter().all(|c| c.prices.is_empty()) {
        return Ok(());
    }
    if let Some(option) = option
        && !pricing(option, currency).sells(components)
    {
        return Err(invalid(format!(
            "The plan is not sold in {currency} for {}",
            option.name
        )));
    }

    // A checkout cannot override the plan's minimum, which is in the plan's currency.
    if !currency.eq_ignore_ascii_case(version_currency)
        && crate::repositories::plans::load_plan_minimum(conn, params.plan_version_id)
            .await?
            .is_some()
    {
        return Err(invalid(format!(
            "The plan's minimum commitment is in {version_currency}: a checkout in {currency} cannot take it. Create the subscription with a minimum override instead"
        )));
    }

    // A new subscription's usage-levels charges start on the level asked, else their lowest: kept
    // on the checkout, so its page, its preview and its completion bill the same level.
    if params.checkout_type == CheckoutType::SelfServe
        && let Some(option) = option
    {
        let mut cells = components.to_vec();
        pricing(option, currency).narrow(&mut cells);
        let had_components = params.components.is_some();
        let mut asked = params
            .components
            .take()
            .unwrap_or_else(|| CreateSubscriptionComponents {
                parameterized_components: vec![],
                overridden_components: vec![],
                extra_components: vec![],
                remove_components: vec![],
            });
        settle_levels(&cells, &mut asked).map_err(Report::new)?;
        if had_components || !asked.parameterized_components.is_empty() {
            params.components = Some(asked);
        }
    }
    Ok(())
}

/// The currency and option a checkout bills: its subscription's currency (an activation, an
/// add-on, a plan change, which keeps the option's alias when the plan has it), else a new
/// subscription's (`SubscriptionCurrency::for_new_subscription`).
async fn checkout_offer(
    conn: &mut PgConn,
    params: &CreateCheckoutSession,
    version_currency: &str,
    options: &[PlanOption],
    components: &[PriceComponent],
) -> StoreResult<SubscriptionCurrency> {
    let tenant_id = params.tenant_id;
    let asked = params.option.clone().map(PlanOptionRef::Alias);
    if let Some(sub) = params.subscription_id {
        let subscription = SubscriptionRow::get_subscription_by_id(conn, &tenant_id, sub)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .subscription;
        let currency = subscription.currency;
        if options.is_empty() {
            return Ok(SubscriptionCurrency::new(&currency));
        }
        if let Some(asked) = asked.as_ref() {
            let option = resolve_option(options, Some(asked))?.clone();
            return Ok(SubscriptionCurrency::with_option(&currency, Some(option)));
        }
        let kept = match subscription.plan_option_id {
            Some(id) if params.checkout_type == CheckoutType::PlanChange => {
                diesel_models::plan_version_options::PlanVersionOptionRow::list_by_plan_version_id(
                    conn,
                    tenant_id,
                    subscription.plan_version_id,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .find(|row| row.id == id)
                .and_then(|current| {
                    options
                        .iter()
                        .find(|option| option.alias.eq_ignore_ascii_case(&current.alias))
                        .cloned()
                })
            }
            _ => None,
        };
        let option = kept
            .or_else(|| {
                sold_options(options, components, &currency)
                    .first()
                    .map(|option| (*option).clone())
            })
            .or_else(|| options.first().cloned());
        return Ok(SubscriptionCurrency::with_option(&currency, option));
    }
    let customer_currency = CustomerRow::find_by_id(conn, &params.customer_id, &tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .currency;
    SubscriptionCurrency::for_new_subscription(
        &customer_currency,
        version_currency,
        options,
        components,
        asked.as_ref(),
        &asked_cadences(params.components.as_ref()),
    )
}

/// The coupon code a checkout redeems, unless it names its coupons.
fn redeemed_code(params: &CreateCheckoutSession) -> Option<&str> {
    params
        .coupon_code
        .as_deref()
        .map(str::trim)
        .filter(|code| params.coupon_ids.is_empty() && !code.is_empty())
}

/// Refuses a checkout its customer may not take: a plan whose rule leaves out who they are, or a
/// coupon code they cannot redeem on it.
async fn refuse_unpurchasable(
    conn: &mut PgConn,
    params: &CreateCheckoutSession,
    currency: &str,
) -> StoreResult<()> {
    let tenant_id = params.tenant_id;
    let ruled_plan = matches!(
        params.checkout_type,
        CheckoutType::SelfServe | CheckoutType::PlanChange
    );
    let code = redeemed_code(params);
    if !ruled_plan && code.is_none() {
        return Ok(());
    }
    let (plan_id, rule) = plan_rule(conn, tenant_id, params.plan_version_id).await?;
    let customer: Customer = CustomerRow::find_by_id(conn, &params.customer_id, &tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .try_into()?;
    let facts = CustomerFacts::of(&customer);
    if ruled_plan
        && let Some(rule) = rule.as_ref()
        && !moves_within(conn, params, plan_id).await?
    {
        eligibility::check(rule, &facts).map_err(|why| invalid(why.message()))?;
    }
    if let Some(code) = code {
        let add_on_ids: Vec<AddOnId> = params
            .add_ons
            .iter()
            .flat_map(|add_ons| add_ons.add_ons.iter().map(|a| a.add_on_id))
            .collect();
        coupon_for_code(
            conn,
            tenant_id,
            code,
            &CouponRedemption {
                plan_id,
                add_on_ids: &add_on_ids,
                currency,
                buyer: &facts,
            },
        )
        .await?
        .map_err(invalid)?;
    }
    Ok(())
}

/// A plan change to another option of the plan subscribed to: kept by who has it, whatever its
/// rule.
async fn moves_within(
    conn: &mut PgConn,
    params: &CreateCheckoutSession,
    plan_id: PlanId,
) -> StoreResult<bool> {
    let Some(subscription_id) = params.subscription_id else {
        return Ok(false);
    };
    if params.checkout_type != CheckoutType::PlanChange {
        return Ok(false);
    }
    let current = SubscriptionRow::get_subscription_by_id(conn, &params.tenant_id, subscription_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    Ok(current.plan_id == plan_id)
}

/// The plan of `version` and who may get it.
pub(crate) async fn plan_rule(
    conn: &mut PgConn,
    tenant_id: TenantId,
    version: PlanVersionId,
) -> StoreResult<(PlanId, Option<Eligibility>)> {
    let plan_id = PlanVersionRow::get_plan_ids_by_version_ids(conn, &[version])
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .remove(&version)
        .ok_or_else(|| Report::new(StoreError::ValueNotFound(format!("plan version {version}"))))?;
    let rule = PlanRow::list_by_ids(conn, &[plan_id], tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .next()
        .and_then(|plan| Eligibility::from_row(plan.eligibility));
    Ok((plan_id, rule))
}

/// A purchase a coupon code is redeemed on, as the checkout completing it reads it.
pub struct CouponRedemption<'a> {
    pub plan_id: PlanId,
    pub add_on_ids: &'a [AddOnId],
    pub currency: &'a str,
    pub buyer: &'a CustomerFacts,
}

/// The coupon `code` names, or why the buyer cannot redeem it on `redemption` now, in words for
/// them: no such code, not valid then or used up, no amount in the currency, out of its scope, or
/// its rule leaving them out. The checks of the completion, a returning customer's aside.
pub(crate) async fn coupon_for_code(
    conn: &mut PgConn,
    tenant_id: TenantId,
    code: &str,
    redemption: &CouponRedemption<'_>,
) -> StoreResult<Result<Coupon, String>> {
    let code = code.trim();
    let rows = CouponRow::candidates_by_codes(conn, tenant_id, &[code.to_owned()])
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let unknown = || Ok(Err(format!("Coupon code '{code}' is not valid")));
    let found = match CodeMatch::of(code, &rows) {
        CodeMatch::Found(row) => row.id,
        CodeMatch::Unknown => return unknown(),
        CodeMatch::Ambiguous => {
            return Ok(Err(format!(
                "Coupon code '{code}' matches several coupons, enter it exactly as you were given it"
            )));
        }
    };
    let rows: Vec<CouponRow> = rows.into_iter().filter(|row| row.id == found).collect();
    let Some(coupon) = crate::repositories::coupons::coupons_with_scope(conn, rows)
        .await?
        .pop()
    else {
        return unknown();
    };
    if let Err(why) =
        coupon.validate_for_use_at(common_clock::now().naive_utc(), redemption.currency)
    {
        return Ok(Err(why.to_error_message(&coupon.code)));
    }
    if !coupon.applies_to(Some(redemption.plan_id), redemption.add_on_ids) {
        return Ok(Err(format!(
            "Coupon code '{}' does not apply to this plan",
            coupon.code
        )));
    }
    if let Some(rule) = coupon.eligibility.as_ref()
        && let Err(why) = eligibility::check(rule, redemption.buyer)
    {
        return Ok(Err(why.message()));
    }
    Ok(Ok(coupon))
}

fn status_domain_to_db(status: CheckoutSessionStatus) -> CheckoutSessionStatusEnum {
    match status {
        CheckoutSessionStatus::Created => CheckoutSessionStatusEnum::Created,
        CheckoutSessionStatus::AwaitingPayment => CheckoutSessionStatusEnum::AwaitingPayment,
        CheckoutSessionStatus::Completed => CheckoutSessionStatusEnum::Completed,
        CheckoutSessionStatus::Expired => CheckoutSessionStatusEnum::Expired,
        CheckoutSessionStatus::Cancelled => CheckoutSessionStatusEnum::Cancelled,
    }
}
