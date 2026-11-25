use crate::client::model::{ClientCustomer, ClientPlanRef, ClientSubscription};
use crate::v2::subscriptions::model::Subscription;
use meteroid_store::domain;

pub fn customer_to_client(customer: domain::Customer) -> ClientCustomer {
    ClientCustomer {
        id: customer.id,
        alias: customer.alias,
        kind: customer.customer_type.into(),
        name: customer.name,
        first_name: customer.first_name,
        last_name: customer.last_name,
        email: customer.billing_email,
        currency: customer.currency,
    }
}

/// v2's summary of the subscription, with its plan's name.
pub fn subscription_to_client(summary: Subscription, plan_name: String) -> ClientSubscription {
    ClientSubscription {
        id: summary.id,
        plan: ClientPlanRef {
            code: summary.plan.code,
            name: plan_name,
            version: summary.plan.version.unwrap_or_default(),
            option: summary.plan.option,
        },
        status: summary.status,
        currency: summary.currency,
        period: summary.period,
        dates: summary.dates,
        current_period: summary.current_period,
        trial: summary.trial,
    }
}
