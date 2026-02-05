use super::model::{
    CheckoutSession as RestCheckoutSession, CheckoutSessionStatus as RestStatus,
    CheckoutType as RestCheckoutType,
};
use meteroid_store::domain::checkout_sessions::{
    CheckoutSession, CheckoutSessionStatus, CheckoutType,
};

pub fn domain_to_rest(
    session: CheckoutSession,
    checkout_url: Option<String>,
) -> RestCheckoutSession {
    RestCheckoutSession {
        id: session.id,
        customer_id: session.customer_id,
        plan_version_id: session.plan_version_id,
        coupon_code: session.coupon_code,
        billing_start_date: session.billing_start_date,
        billing_day_anchor: session.billing_day_anchor.map(|a| a as i32),
        net_terms: session.net_terms,
        trial_duration_days: session.trial_duration_days,
        status: status_domain_to_rest(session.status),
        checkout_type: checkout_type_domain_to_rest(session.checkout_type),
        created_at: session.created_at,
        expires_at: session.expires_at,
        completed_at: session.completed_at,
        subscription_id: session.subscription_id,
        checkout_url,
        payment_methods_config: session.payment_methods_config.map(Into::into),
    }
}

fn status_domain_to_rest(status: CheckoutSessionStatus) -> RestStatus {
    match status {
        CheckoutSessionStatus::Created => RestStatus::Created,
        CheckoutSessionStatus::AwaitingPayment => RestStatus::AwaitingPayment,
        CheckoutSessionStatus::Completed => RestStatus::Completed,
        CheckoutSessionStatus::Expired => RestStatus::Expired,
        CheckoutSessionStatus::Cancelled => RestStatus::Cancelled,
    }
}

fn checkout_type_domain_to_rest(checkout_type: CheckoutType) -> RestCheckoutType {
    match checkout_type {
        CheckoutType::SelfServe => RestCheckoutType::SelfServe,
        CheckoutType::SubscriptionActivation => RestCheckoutType::SubscriptionActivation,
    }
}

pub fn status_rest_to_domain(status: RestStatus) -> CheckoutSessionStatus {
    match status {
        RestStatus::Created => CheckoutSessionStatus::Created,
        RestStatus::AwaitingPayment => CheckoutSessionStatus::AwaitingPayment,
        RestStatus::Completed => CheckoutSessionStatus::Completed,
        RestStatus::Expired => CheckoutSessionStatus::Expired,
        RestStatus::Cancelled => CheckoutSessionStatus::Cancelled,
    }
}
