//! Promotional credit coupons: each redemption (an applied coupon) funds one promotional grant
//! on the customer's wallet in the coupon currency, and never discounts an invoice.
use crate::StoreResult;
use crate::domain::coupons::{
    Coupon, CouponCreditRedemption, CouponCreditScope, CouponDiscount, RedeemableSubscription,
    SubscriptionRequired,
};
use crate::domain::credit::{CreditCost, GrantClassification, GrantScope};
use crate::errors::StoreError;
use crate::hooks::CreditControlHooks;
use crate::repositories::credit::customer_credit::{
    self as credit, CreditFunding, DEFAULT_PRIORITY, source,
};
use crate::services::ServicesEdge;
use crate::services::credit::{invalid, precondition};
use crate::services::subscriptions::utils::apply_coupons_without_validation_at;
use crate::store::PgConn;
use chrono::{DateTime, Utc};
use common_domain::ids::{AppliedCouponId, BaseId, CouponId, CustomerId, SubscriptionId, TenantId};
use diesel_models::applied_coupons::{
    AppliedCouponDetailedRow, AppliedCouponRow, AppliedCouponRowNew,
};
use diesel_models::coupon_plans::CouponPlanRow;
use diesel_models::coupons::CouponRow;
use diesel_models::customer_credit::CustomerCreditGrantRow;
use diesel_models::enums::SubscriptionStatusEnum;
use diesel_models::plan_versions::PlanVersionRow;
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use itertools::Itertools;
use scoped_futures::ScopedFutureExt;
use std::collections::HashMap;

/// Whether a subscription has started: before that (pending checkout, start date or payment),
/// a promo code's credit waits, and an abandoned or cancelled subscription never gets it. A trial
/// still waiting for checkout has not started; an active subscription has, whatever it awaits.
pub(crate) fn has_started(subscription: &SubscriptionRow) -> bool {
    match subscription.status {
        SubscriptionStatusEnum::Active => true,
        SubscriptionStatusEnum::TrialActive => !subscription.pending_checkout,
        _ => false,
    }
}

/// Funds the promo credit of the coupons applied to a subscription once it has started, at `at`.
/// Every transition that starts a subscription calls it; replays fund nothing more.
pub(crate) async fn mint_started_subscription_coupon_credit_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    at: DateTime<Utc>,
) -> StoreResult<()> {
    let applied: Vec<AppliedCouponRowNew> =
        AppliedCouponDetailedRow::list_by_subscription_id(conn, &subscription_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .filter(|row| row.applied_coupon.is_active)
            .map(|row| AppliedCouponRowNew {
                id: row.applied_coupon.id,
                subscription_id: row.applied_coupon.subscription_id,
                coupon_id: row.applied_coupon.coupon_id,
                customer_id: row.applied_coupon.customer_id,
                is_active: row.applied_coupon.is_active,
                applied_amount: row.applied_coupon.applied_amount,
                applied_count: row.applied_coupon.applied_count,
                last_applied_at: row.applied_coupon.last_applied_at,
            })
            .collect();
    let refs: Vec<&AppliedCouponRowNew> = applied.iter().collect();
    mint_coupon_credit_tx(conn, controls, tenant_id, &refs, at)
        .await
        .map(|_| ())
}

/// Funds the credit of the promotional credit coupons among `applied`, one grant per applied
/// coupon (its `source_id`), so a replay funds nothing more. Other coupons, and coupons of a
/// subscription that has not started, fund nothing yet. `at` is when the credit is added, in
/// business time: its term runs from then.
pub(crate) async fn mint_coupon_credit_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    tenant_id: TenantId,
    applied: &[&AppliedCouponRowNew],
    at: DateTime<Utc>,
) -> StoreResult<Vec<CouponCreditRedemption>> {
    if applied.is_empty() {
        return Ok(Vec::new());
    }
    let coupon_ids: Vec<CouponId> = applied.iter().map(|a| a.coupon_id).unique().collect();
    let coupons: HashMap<CouponId, Coupon> = CouponRow::list_by_ids(conn, &coupon_ids, &tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .map(|row| TryInto::<Coupon>::try_into(row).map(|coupon| (coupon.id, coupon)))
        .collect::<Result<_, _>>()?;
    let credited: Vec<(&AppliedCouponRowNew, &Coupon)> = applied
        .iter()
        .filter_map(|a| {
            coupons
                .get(&a.coupon_id)
                .filter(|coupon| coupon.discount.is_credit())
                .map(|coupon| (*a, coupon))
        })
        .collect();
    if credited.is_empty() {
        return Ok(Vec::new());
    }
    // Serializes funding per coupon, so the once-per-customer and redemption-limit checks below see
    // the grants of concurrent settlements that applied the code without validating it again.
    let credit_coupon_ids: Vec<CouponId> = credited.iter().map(|(_, c)| c.id).unique().collect();
    CouponRow::lock_redemption_funding(conn, &credit_coupon_ids)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    let subscription_ids: Vec<_> = credited
        .iter()
        .map(|(a, _)| a.subscription_id)
        .unique()
        .collect();
    let subscriptions: HashMap<_, SubscriptionRow> =
        SubscriptionRow::list_by_ids(conn, &subscription_ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .filter(|row| row.subscription.tenant_id == tenant_id)
            .map(|row| (row.subscription.id, row.subscription))
            .collect();

    let mut redemptions = Vec::with_capacity(credited.len());
    for (applied_coupon, coupon) in credited {
        let CouponDiscount::Credit {
            currency,
            expires_in_days,
            scope,
            ..
        } = &coupon.discount
        else {
            continue;
        };
        let subscription = subscriptions
            .get(&applied_coupon.subscription_id)
            .ok_or_else(|| precondition("the subscription of a redeemed coupon was not found"))?;
        let subscription_currency = &subscription.currency;
        if subscription_currency != currency {
            return Err(invalid(format!(
                "coupon {} grants {currency} credit, but the subscription is billed in {subscription_currency}",
                coupon.code
            )));
        }
        if !has_started(subscription) {
            continue;
        }
        // Every transition that starts the subscription comes here: the first one's grant stands.
        if let Some(grant) = CustomerCreditGrantRow::find_by_source(
            conn,
            tenant_id,
            source::COUPON,
            applied_coupon.id.as_uuid(),
        )
        .await?
        {
            redemptions.push(CouponCreditRedemption {
                applied_coupon_id: applied_coupon.id,
                amount_minor: grant.amount,
                currency: currency.clone(),
                expires_at: grant.expires_at,
            });
            continue;
        }
        if AppliedCouponRow::customer_redemption_funded(
            conn,
            tenant_id,
            coupon.id,
            applied_coupon.customer_id,
            source::COUPON,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        {
            unfunded(conn, applied_coupon, "already funded for this customer").await?;
            continue;
        }
        if let Some(limit) = coupon.redemption_limit
            && AppliedCouponRow::count_funded_redemptions(
                conn,
                tenant_id,
                coupon.id,
                source::COUPON,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?
                >= i64::from(limit)
        {
            unfunded(conn, applied_coupon, "redemption limit reached").await?;
            continue;
        }
        let amount_minor = coupon.discount.credit_minor().ok_or_else(|| {
            invalid("promotional credit has more decimal places than its currency allows")
        })?;
        let expires_at = expires_in_days
            .map(|days| {
                chrono::Duration::try_days(i64::from(days))
                    .and_then(|term| at.checked_add_signed(term))
                    .ok_or_else(|| invalid("the promotional credit expiry is out of range"))
            })
            .transpose()?;
        let reason = format!("Promo code {}", coupon.code);
        let grant = credit::fund_tx(
            conn,
            controls,
            CreditFunding {
                tenant_id,
                customer_id: applied_coupon.customer_id,
                currency: currency.clone(),
                scope: match scope {
                    CouponCreditScope::Customer => GrantScope::customer_wide(),
                    CouponCreditScope::Subscription => GrantScope {
                        subscription_id: Some(applied_coupon.subscription_id),
                        ..GrantScope::customer_wide()
                    },
                },
                amount_minor,
                classification: GrantClassification::Promotional,
                priority: DEFAULT_PRIORITY,
                effective_at: at,
                expires_at,
                occurred_at: at,
                now: at,
                source_kind: source::COUPON,
                source_id: applied_coupon.id.as_uuid(),
                cost: Some(CreditCost::given(currency)),
                rollover: None,
                invoice_id: None,
                line_local_id: None,
                credit_note_id: None,
                reason,
                reference: Some(coupon.code.clone()),
                created_by: None,
                pays_usage_from: None,
            },
        )
        .await?;
        redemptions.push(CouponCreditRedemption {
            applied_coupon_id: applied_coupon.id,
            amount_minor: grant.amount,
            currency: currency.clone(),
            expires_at: grant.expires_at,
        });
    }
    Ok(redemptions)
}

/// A redemption applied without validation (an asynchronous checkout settling) that may not fund
/// its credit: it is deactivated, so the subscription shows it never applied.
async fn unfunded(
    conn: &mut PgConn,
    applied_coupon: &AppliedCouponRowNew,
    reason: &str,
) -> StoreResult<()> {
    log::warn!(
        "promo credit of coupon {} not funded for customer {} ({reason}); applied coupon {} deactivated",
        applied_coupon.coupon_id,
        applied_coupon.customer_id,
        applied_coupon.id
    );
    AppliedCouponRow::deactivate(conn, applied_coupon.id)
        .await
        .map_err(Into::<Report<StoreError>>::into)
}

/// Customer-facing: the portal shows these messages as they are.
fn refused(message: &str) -> Report<StoreError> {
    Report::new(StoreError::InvalidArgument(message.to_owned()))
}

/// One answer for every code that cannot be redeemed, so codes cannot be probed.
pub const CODE_NOT_REDEEMABLE: &str = "This code can't be redeemed on your account.";
pub const CODE_ALREADY_REDEEMED: &str = "You've already redeemed this code.";
pub const NO_ACTIVE_SUBSCRIPTION: &str = "You need an active subscription to redeem a code.";
/// A code for one subscription, with several it may apply to: the refusal carries them
/// (`SubscriptionRequired`), and naming one redeems it there.
pub const SUBSCRIPTION_REQUIRED: &str = "Choose the subscription this code applies to.";
/// Matches the longest code a coupon can be created with.
pub const MAX_CODE_LENGTH: usize = 64;

impl ServicesEdge {
    /// Redeems a promotional credit code on the customer's own behalf, with the same checks as a
    /// coupon given at signup, on `subscription_id` or else the oldest started subscription it
    /// applies to. A code for one subscription needs it named when several qualify.
    pub async fn redeem_coupon_credit_code(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        code: &str,
        subscription_id: Option<SubscriptionId>,
    ) -> StoreResult<CouponCreditRedemption> {
        self.redeem_coupon_credit_code_on(
            tenant_id,
            customer_id,
            code,
            subscription_id,
            common_domain::clock::now(),
            false,
        )
        .await
    }

    /// `redeem_coupon_credit_code` at `at`: the code must have existed and been valid then, and its
    /// credit is funded from then. Past moments are a sandbox replay's only.
    pub(crate) async fn redeem_coupon_credit_code_on(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        code: &str,
        subscription_id: Option<SubscriptionId>,
        at: DateTime<Utc>,
        replayed: bool,
    ) -> StoreResult<CouponCreditRedemption> {
        if at > common_domain::clock::now() {
            return Err(invalid("a code cannot be redeemed at a future moment"));
        }
        // Promotional-credit codes are stored upper-case and redeemed whatever the case.
        let code = code.trim().to_uppercase();
        if code.is_empty() || code.chars().count() > MAX_CODE_LENGTH {
            return Err(refused(CODE_NOT_REDEEMABLE));
        }
        self.store
            .transaction(|conn| {
                let code = &code;
                async move {
                    redeem_coupon_credit_code_tx(
                        conn,
                        &*self.store.hooks.credit,
                        tenant_id,
                        customer_id,
                        code,
                        subscription_id,
                        at,
                        replayed,
                    )
                    .await
                }
                .scope_boxed()
            })
            .await
    }
}

async fn redeem_coupon_credit_code_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    tenant_id: TenantId,
    customer_id: CustomerId,
    code: &str,
    subscription_id: Option<SubscriptionId>,
    at: DateTime<Utc>,
    replayed: bool,
) -> StoreResult<CouponCreditRedemption> {
    credit::lock_customer_tx(conn, tenant_id, customer_id).await?;
    let mut subscriptions = SubscriptionRow::find_active_by_customer(conn, customer_id, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    subscriptions.retain(has_started);
    if subscriptions.is_empty() {
        return Err(refused(NO_ACTIVE_SUBSCRIPTION));
    }
    subscriptions.sort_by_key(|subscription| (subscription.start_date, subscription.id.as_uuid()));

    let not_redeemable = || refused(CODE_NOT_REDEEMABLE);
    let coupon_id = CouponRow::list_by_codes(conn, tenant_id, &[code.to_owned()])
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .find(|row| row.code == code)
        .ok_or_else(not_redeemable)?
        .id;
    // Locked so concurrent redemptions cannot exceed the redemption limit nor redeem twice.
    let coupon: Coupon = CouponRow::list_by_ids_for_update(conn, &[coupon_id], &tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .next()
        .ok_or_else(not_redeemable)?
        .try_into()?;
    let CouponDiscount::Credit {
        currency, scope, ..
    } = &coupon.discount
    else {
        return Err(not_redeemable());
    };

    if !AppliedCouponRow::find_existing_customer_coupon_pairs(conn, &[(coupon.id, customer_id)])
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .is_empty()
    {
        return Err(refused(CODE_ALREADY_REDEEMED));
    }
    if (replayed && coupon.created_at > at.naive_utc())
        || coupon.is_expired(at.naive_utc())
        || coupon.archived_at.is_some()
        || coupon.disabled
        || coupon
            .redemption_limit
            .is_some_and(|limit| coupon.redemption_count >= limit)
    {
        return Err(not_redeemable());
    }

    subscriptions.retain(|subscription| subscription.currency == *currency);
    let allowed_plans = CouponPlanRow::list_by_coupon_ids(conn, &[coupon.id])
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .remove(&coupon.id)
        .unwrap_or_default();
    if !allowed_plans.is_empty() && !subscriptions.is_empty() {
        let version_ids: Vec<_> = subscriptions.iter().map(|s| s.plan_version_id).collect();
        let plans = PlanVersionRow::get_plan_ids_by_version_ids(conn, &version_ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        subscriptions.retain(|subscription| {
            plans
                .get(&subscription.plan_version_id)
                .is_some_and(|plan_id| allowed_plans.contains(plan_id))
        });
    }
    if let Some(named) = subscription_id {
        subscriptions.retain(|subscription| subscription.id == named);
    } else if *scope == CouponCreditScope::Subscription && subscriptions.len() > 1 {
        let ids: Vec<SubscriptionId> = subscriptions.iter().map(|s| s.id).collect();
        let plans: HashMap<SubscriptionId, String> = SubscriptionRow::list_by_ids(conn, &ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(|row| (row.subscription.id, row.plan_name))
            .collect();
        let choices = subscriptions
            .iter()
            .map(|subscription| RedeemableSubscription {
                id: subscription.id,
                plan_name: plans.get(&subscription.id).cloned().unwrap_or_default(),
                start_date: subscription.start_date,
            })
            .collect();
        return Err(refused(SUBSCRIPTION_REQUIRED).attach_opaque(SubscriptionRequired(choices)));
    }
    let subscription = subscriptions
        .into_iter()
        .next()
        .ok_or_else(not_redeemable)?;

    let applied_coupon = AppliedCouponRowNew {
        id: AppliedCouponId::new(),
        subscription_id: subscription.id,
        coupon_id: coupon.id,
        customer_id,
        is_active: true,
        applied_amount: None,
        applied_count: None,
        last_applied_at: None,
    };
    apply_coupons_without_validation_at(conn, controls, &[&applied_coupon], tenant_id, at)
        .await?
        .into_iter()
        .find(|redemption| redemption.applied_coupon_id == applied_coupon.id)
        .ok_or_else(|| precondition("the promotional credit was not funded"))
}
