use crate::currencies;
use crate::entitlements::mapping::entitlement_to_rest;
use crate::subscriptions::model::{
    AppliedCoupon, AppliedCouponDetailed, Coupon, CreateSubscriptionAddOn,
    CreateSubscriptionComponents, Subscription, SubscriptionAddOnCustomization,
    SubscriptionCreateRequest, SubscriptionDetails, SubscriptionUpdateRequest, UsageFilter,
};
use common_domain::ids::{CouponId, CustomerId, PlanVersionId, SubscriptionId};
use meteroid_app::errors::RestApiError;
use meteroid_store::domain;
use meteroid_store::domain::{CreateSubscription, SubscriptionNew, SubscriptionPatch};

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
        created_at: s.created_at.into(),
        activated_at: s.activated_at.map(Into::into),
        purchase_order: s.purchase_order,
        tax_inclusive: s.tax_inclusive,
        auto_advance_invoices: s.auto_advance_invoices,
        charge_automatically: s.charge_automatically,
        payment_methods_config: s.payment_methods_config.map(Into::into),
        custom_properties: s.custom_properties,
        usage_filter: s.usage_filter.map(usage_filter_to_rest),
        usage_filter_effective_from: (s.usage_filter_since.map(|since| since.and_utc()))
            .map(Into::into),
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

    let minimum_commitment = s.minimum.as_ref().map(|m| {
        crate::plans::mapping::subscription_minimum_to_rest(m, s.subscription.currency.as_str())
    });

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
        created_at: s.subscription.created_at.into(),
        activated_at: s.subscription.activated_at.map(Into::into),
        purchase_order: s.subscription.purchase_order,
        tax_inclusive: s.subscription.tax_inclusive,
        auto_advance_invoices: s.subscription.auto_advance_invoices,
        charge_automatically: s.subscription.charge_automatically,
        payment_methods_config: s.subscription.payment_methods_config.map(Into::into),
        components,
        add_ons,
        applied_coupons,
        checkout_url: s.checkout_url,
        entitlements: s
            .entitlements
            .into_iter()
            .map(entitlement_to_rest)
            .collect(),
        minimum_commitment,
        custom_properties: s.subscription.custom_properties,
        usage_filter: s.subscription.usage_filter.map(usage_filter_to_rest),
        usage_filter_effective_from: s.subscription.usage_filter_since.map(Into::into),
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
        last_applied_at: ac.applied_coupon.last_applied_at.map(Into::into),
        created_at: ac.applied_coupon.created_at.into(),
    };

    Ok(AppliedCouponDetailed {
        coupon,
        applied_coupon,
    })
}

fn domain_coupon_to_rest(c: domain::coupons::Coupon) -> Result<Coupon, RestApiError> {
    let discount = crate::coupons::mapping::coupon_discount_to_rest(&c.discount);

    Ok(Coupon {
        id: c.id,
        code: c.code,
        description: c.description,
        discount,
        expires_at: c.expires_at.map(Into::into),
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
            activation_condition: sub.activation_condition.into(),
            purchase_order: sub.purchase_order,
            auto_advance_invoices: sub.auto_advance_invoices.unwrap_or(true),
            charge_automatically: sub.charge_automatically.unwrap_or(true),
            backdate_invoices: sub.backdate_invoices.unwrap_or(false),
            skip_checkout_session: false,
            skip_past_invoices: sub.skip_past_invoices.unwrap_or(false),
            usage_filter: sub.usage_filter.map(usage_filter_from_rest).transpose()?,
            custom_properties: sub.custom_properties.unwrap_or(serde_json::Value::Null),

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
        entitlements: vec![],
        minimum_override: None,
        drop_minimum: false,
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
                Some(SubscriptionAddOnCustomization::PriceOverride(o)) => {
                    domain::SubscriptionAddOnCustomization::PriceOverride {
                        name: o.name,
                        price_entry: o.price_entry.into(),
                    }
                }
            },
            quantity: if add_on.quantity == 0 {
                1
            } else {
                add_on.quantity.min(i32::MAX as u32) as i32
            },
        }
    }
}

pub fn map_add_ons(add_ons: Vec<CreateSubscriptionAddOn>) -> domain::CreateSubscriptionAddOns {
    domain::CreateSubscriptionAddOns {
        add_ons: add_ons.into_iter().map(Into::into).collect(),
    }
}

pub fn rest_to_domain_update_request(
    subscription_id: SubscriptionId,
    req: SubscriptionUpdateRequest,
) -> Result<SubscriptionPatch, RestApiError> {
    Ok(SubscriptionPatch {
        id: subscription_id,
        charge_automatically: req.charge_automatically,
        auto_advance_invoices: req.auto_advance_invoices,
        net_terms: req.net_terms,
        invoice_memo: req
            .invoice_memo
            .map(|m| if m.is_empty() { None } else { Some(m) }),
        purchase_order: req
            .purchase_order
            .map(|p| if p.is_empty() { None } else { Some(p) }),
        payment_methods_config: req.payment_methods_config.map(|c| Some(c.into())),
        usage_filter: req
            .usage_filter
            .map(|filter| filter.map(usage_filter_from_rest).transpose())
            .transpose()?,
        custom_properties: req.custom_properties,
    })
}

fn usage_filter_to_rest(filter: domain::UsageFilter) -> UsageFilter {
    UsageFilter {
        property: filter.property,
        value: filter.value,
    }
}

fn usage_filter_from_rest(filter: UsageFilter) -> Result<domain::UsageFilter, RestApiError> {
    domain::UsageFilter::new(filter.property, filter.value).map_err(RestApiError::from)
}
