use crate::client::model::{ClientCustomer, ClientSubscription};
use meteroid_store::domain;

pub fn customer_to_client(customer: domain::Customer) -> ClientCustomer {
    ClientCustomer {
        id: customer.id,
        alias: customer.alias,
        name: customer.name,
        currency: customer.currency,
        billing_email: customer.billing_email,
    }
}

pub fn subscription_to_client(subscription: domain::Subscription) -> ClientSubscription {
    ClientSubscription {
        id: subscription.id,
        plan_id: subscription.plan_id,
        plan_name: subscription.plan_name,
        plan_version: subscription.version,
        status: subscription.status.into(),
        currency: subscription.currency,
        billing_period: Some(subscription.period.into()),
        start_date: subscription.start_date,
        end_date: subscription.end_date,
        current_period_start: subscription.current_period_start,
        current_period_end: subscription.current_period_end,
    }
}
