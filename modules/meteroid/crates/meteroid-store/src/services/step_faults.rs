//! Panics the tests inject into a subscription's steps, to check what a panicking step does
//! (R-121). Compiled only with the `test-faults` feature and debug assertions: never in a release.

use common_domain::ids::SubscriptionId;
use std::sync::Mutex;

static PANICKING: Mutex<Vec<SubscriptionId>> = Mutex::new(Vec::new());

/// Every step of `subscription_id` panics until [`clear`].
pub fn panic_in_steps_of(subscription_id: SubscriptionId) {
    PANICKING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(subscription_id);
}

pub fn clear() {
    PANICKING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
}

pub(crate) fn step_of(subscription_id: SubscriptionId) {
    let panics = PANICKING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains(&subscription_id);
    if panics {
        panic!("a fault injected into the step of {subscription_id}");
    }
}
