use crate::api_rest::currencies;
use crate::api_rest::subscriptions::model::{
    AppliedCoupon, AppliedCouponDetailed, Coupon, CouponDiscount, CreateSubscriptionAddOn,
    CreateSubscriptionComponents, FixedDiscount, PercentageDiscount, Subscription,
    SubscriptionAddOnCustomization, SubscriptionCreateRequest, SubscriptionDetails,
};
use crate::errors::RestApiError;
use common_domain::ids::{CouponId, CustomerId, PlanVersionId};
use meteroid_store::domain;
use meteroid_store::domain::{CreateSubscription, SubscriptionNew};
use uuid::Uuid;

pub fn domain_to_rest(s: domain::Subscription) -> Result<Subscription, RestApiError> {
    Ok(Subscription {
        id: s.id,
        customer_id: s.customer_id,
        customer_name: s.customer_name,
        customer_alias: s.customer_alias,
        billing_day_anchor: s.billing_day_anchor as i16,
        currency: currencies::mapping::from_str(s.currency.as_str())?,
        plan_id: s.plan_id,
        plan_name: s.plan_name,
        plan_description: s.plan_description,
        plan_version_id: s.plan_version_id,
        plan_version: s.version,
        status: s.status.into(),
        start_date: s.start_date,
        end_date: s.end_date,
        billing_start_date: s.billing_start_date,
        current_period_start: s.current_period_start,
        current_period_end: s.current_period_end,
        trial_duration: s.trial_duration,
        net_terms: s.net_terms,
        invoice_memo: s.invoice_memo,
        mrr_cents: s.mrr_cents,
        period: s.period.into(),
        created_at: s.created_at,
        activated_at: s.activated_at,
        purchase_order: s.purchase_order,
        auto_advance_invoices: s.auto_advance_invoices,
        charge_automatically: s.charge_automatically,
        payment_methods_config: s.payment_methods_config.map(Into::into),
    })
}

pub fn domain_to_rest_details(
    s: domain::SubscriptionDetails,
) -> Result<SubscriptionDetails, RestApiError> {
    let components = s.price_components.into_iter().map(|c| c.into()).collect();

    let add_ons = s.add_ons.into_iter().map(|a| a.into()).collect();

    let applied_coupons = s
        .applied_coupons
        .into_iter()
        .map(domain_applied_coupon_to_rest)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(SubscriptionDetails {
        id: s.subscription.id,
        customer_id: s.subscription.customer_id,
        customer_name: s.subscription.customer_name,
        customer_alias: s.subscription.customer_alias,
        billing_day_anchor: s.subscription.billing_day_anchor as i16,
        currency: currencies::mapping::from_str(s.subscription.currency.as_str())?,
        plan_id: s.subscription.plan_id,
        plan_name: s.subscription.plan_name,
        plan_version_id: s.subscription.plan_version_id,
        plan_version: s.subscription.version,
        status: s.subscription.status.into(),
        start_date: s.subscription.start_date,
        end_date: s.subscription.end_date,
        billing_start_date: s.subscription.billing_start_date,
        current_period_start: s.subscription.current_period_start,
        current_period_end: s.subscription.current_period_end,
        trial_duration: s.subscription.trial_duration,
        net_terms: s.subscription.net_terms,
        invoice_memo: s.subscription.invoice_memo,
        mrr_cents: s.subscription.mrr_cents,
        period: s.subscription.period.into(),
        created_at: s.subscription.created_at,
        activated_at: s.subscription.activated_at,
        purchase_order: s.subscription.purchase_order,
        auto_advance_invoices: s.subscription.auto_advance_invoices,
        charge_automatically: s.subscription.charge_automatically,
        payment_methods_config: s.subscription.payment_methods_config.map(Into::into),
        components,
        add_ons,
        applied_coupons,
        checkout_url: s.checkout_url,
    })
}

fn domain_applied_coupon_to_rest(
    ac: domain::AppliedCouponDetailed,
) -> Result<AppliedCouponDetailed, RestApiError> {
    let coupon = domain_coupon_to_rest(ac.coupon)?;
    let applied_coupon = AppliedCoupon {
        id: ac.applied_coupon.id,
        coupon_id: ac.applied_coupon.coupon_id,
        is_active: ac.applied_coupon.is_active,
        applied_amount: ac.applied_coupon.applied_amount,
        applied_count: ac.applied_coupon.applied_count,
        last_applied_at: ac.applied_coupon.last_applied_at,
        created_at: ac.applied_coupon.created_at,
    };

    Ok(AppliedCouponDetailed {
        coupon,
        applied_coupon,
    })
}

fn domain_coupon_to_rest(c: domain::coupons::Coupon) -> Result<Coupon, RestApiError> {
    let discount = match c.discount {
        domain::coupons::CouponDiscount::Percentage(percentage) => {
            CouponDiscount::Percentage(PercentageDiscount { percentage })
        }
        domain::coupons::CouponDiscount::Fixed { currency, amount } => {
            CouponDiscount::Fixed(FixedDiscount { currency, amount })
        }
    };

    Ok(Coupon {
        id: c.id,
        code: c.code,
        description: c.description,
        discount,
        expires_at: c.expires_at,
        redemption_limit: c.redemption_limit,
        recurring_value: c.recurring_value,
        reusable: c.reusable,
        disabled: c.disabled,
    })
}

pub fn rest_to_domain_create_request(
    resolved_plan_version_id: PlanVersionId,
    resolved_customer_id: CustomerId,
    resolved_coupon_ids: Option<Vec<CouponId>>,
    created_by: Uuid,
    sub: SubscriptionCreateRequest,
) -> Result<CreateSubscription, RestApiError> {
    let converted = CreateSubscription {
        subscription: SubscriptionNew {
            plan_version_id: resolved_plan_version_id,
            customer_id: resolved_customer_id,
            trial_duration: sub.trial_days,
            start_date: sub.start_date,
            end_date: sub.end_date,
            billing_day_anchor: sub.billing_day_anchor,
            net_terms: sub.net_terms,
            invoice_memo: sub.invoice_memo,
            created_by,
            activation_condition: sub.activation_condition.into(),
            purchase_order: sub.purchase_order,
            auto_advance_invoices: sub.auto_advance_invoices.unwrap_or(true),
            charge_automatically: sub.charge_automatically.unwrap_or(true),
            backdate_invoices: false,
            skip_checkout_session: false,

            payment_methods_config: sub.payment_methods_config.map(Into::into),
            invoice_threshold: None,
            billing_start_date: None,
        },
        price_components: sub.price_components.map(Into::into),
        add_ons: sub.add_ons.map(map_add_ons),
        coupons: resolved_coupon_ids.map(|ids| domain::CreateSubscriptionCoupons {
            coupons: ids
                .into_iter()
                .map(|coupon_id| domain::CreateSubscriptionCoupon { coupon_id })
                .collect(),
        }),
    };

    Ok(converted)
}

impl From<CreateSubscriptionComponents> for domain::CreateSubscriptionComponents {
    fn from(pc: CreateSubscriptionComponents) -> Self {
        Self {
            parameterized_components: pc
                .parameterized_components
                .unwrap_or_default()
                .into_iter()
                .map(Into::into)
                .collect(),
            overridden_components: pc
                .overridden_components
                .unwrap_or_default()
                .into_iter()
                .map(Into::into)
                .collect(),
            extra_components: pc
                .extra_components
                .unwrap_or_default()
                .into_iter()
                .map(Into::into)
                .collect(),
            remove_components: pc.remove_components.unwrap_or_default(),
        }
    }
}

impl From<CreateSubscriptionAddOn> for domain::CreateSubscriptionAddOn {
    fn from(add_on: CreateSubscriptionAddOn) -> Self {
        Self {
            add_on_id: add_on.add_on_id,
            customization: match add_on.customization {
                None => domain::SubscriptionAddOnCustomization::None,
                Some(SubscriptionAddOnCustomization::Parameterization(p)) => {
                    domain::SubscriptionAddOnCustomization::Parameterization(p.into())
                }
                Some(SubscriptionAddOnCustomization::Override(o)) => {
                    domain::SubscriptionAddOnCustomization::Override(o.into())
                }
            },
        }
    }
}

pub fn map_add_ons(add_ons: Vec<CreateSubscriptionAddOn>) -> domain::CreateSubscriptionAddOns {
    domain::CreateSubscriptionAddOns {
        add_ons: add_ons.into_iter().map(Into::into).collect(),
    }
}
