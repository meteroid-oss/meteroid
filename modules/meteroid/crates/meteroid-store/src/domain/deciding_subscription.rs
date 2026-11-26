//! Which subscription decides a level (ADR 0011 §3.7). Pure: the caller folds each subscription first.
use crate::domain::entitlement_fold::{GrantValue, ValueOrder};
use crate::domain::resources::ResolvedResource;
use chrono::NaiveDate;
use common_domain::ids::{ResourceId, ResourceTypeId, SubscriptionId};
use std::cmp::Ordering;

/// A subscription that includes the feature at the level, with the value its fold gives there.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub subscription_id: SubscriptionId,
    /// `None` for a subscription for the whole customer.
    pub resource: Option<(ResourceTypeId, ResourceId)>,
    pub start_date: NaiveDate,
    pub value: Option<GrantValue>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Deciding {
    Subscription {
        id: SubscriptionId,
        /// Customer subscriptions answered a check without a resource while resource ones also include the
        /// feature: `hint: "resource_omitted"`.
        resource_omitted: bool,
    },
    /// No subscription includes the feature: the default, customer and resource-override tiers decide, and a
    /// billing-cycle reset is the calendar month.
    NoSubscription,
    /// Only resource subscriptions include it, and the check names no resource of their types.
    ResourceRequired,
    /// The check's resource is of a subscribed type, but none of its subscriptions includes the feature.
    NotSubscribed,
}

/// The check's resource's subscriptions, else the customer's, the best value among them, ties to the oldest.
pub fn deciding(
    order: &ValueOrder,
    candidates: &[Candidate],
    resource: Option<&ResolvedResource>,
) -> Deciding {
    let stored = resource.and_then(|r| r.reference.stored_id());
    let at_resource: Vec<&Candidate> = candidates
        .iter()
        .filter(|c| stored.is_some() && c.resource.map(|(_, id)| id) == stored)
        .collect();
    let for_customer: Vec<&Candidate> =
        candidates.iter().filter(|c| c.resource.is_none()).collect();
    let (pool, resource_omitted) = match (at_resource.is_empty(), for_customer.is_empty()) {
        (false, _) => (at_resource, false),
        (true, false) => (
            for_customer,
            resource.is_none() && candidates.iter().any(|c| c.resource.is_some()),
        ),
        (true, true) if candidates.is_empty() => return Deciding::NoSubscription,
        (true, true) => {
            let subscribed_type = resource.is_some_and(|r| {
                candidates
                    .iter()
                    .any(|c| c.resource.is_some_and(|(t, _)| t == r.type_id))
            });
            return if subscribed_type {
                Deciding::NotSubscribed
            } else {
                Deciding::ResourceRequired
            };
        }
    };
    let best = pool
        .into_iter()
        .min_by(|a, b| {
            better(order, &b.value, &a.value).then_with(|| {
                (a.start_date, *a.subscription_id).cmp(&(b.start_date, *b.subscription_id))
            })
        })
        .expect("the pool is not empty");
    Deciding::Subscription {
        id: best.subscription_id,
        resource_omitted,
    }
}

/// `Greater` when `a` is the better value; no value is the worst.
fn better(order: &ValueOrder, a: &Option<GrantValue>, b: &Option<GrantValue>) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => order.compare(a, b).unwrap_or(Ordering::Equal),
        (a, b) => a.is_some().cmp(&b.is_some()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entitlement_fold::LimitValue;
    use crate::domain::resources::ResourceRef;
    use common_domain::ids::BaseId;
    use rust_decimal::Decimal;
    use std::collections::BTreeMap;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 1, d).unwrap()
    }

    fn n(v: i64) -> Option<GrantValue> {
        Some(GrantValue::Limit(LimitValue::Amount(Decimal::from(v))))
    }

    fn candidate(
        resource: Option<(ResourceTypeId, ResourceId)>,
        started: u32,
        value: Option<GrantValue>,
    ) -> Candidate {
        Candidate {
            subscription_id: SubscriptionId::new(),
            resource,
            start_date: day(started),
            value,
        }
    }

    fn checked(type_id: ResourceTypeId, reference: ResourceRef) -> ResolvedResource {
        ResolvedResource {
            type_id,
            type_code: "site".into(),
            key_value: "s".into(),
            path: "site=s".into(),
            reference,
            attributes: BTreeMap::new(),
            attributes_ignored: vec![],
        }
    }

    fn decided(c: &Candidate, resource_omitted: bool) -> Deciding {
        Deciding::Subscription {
            id: c.subscription_id,
            resource_omitted,
        }
    }

    #[test]
    fn pagecraft() {
        let (site, tenant) = (ResourceTypeId::new(), ResourceTypeId::new());
        let (s1, s3) = (ResourceId::new(), ResourceId::new());
        let pro_s1 = candidate(Some((site, s1)), 5, n(5));
        let business = candidate(None, 1, n(3));
        let both = [pro_s1.clone(), business.clone()];
        let on = |id| checked(site, ResourceRef::Stored(id));
        let order = ValueOrder::Limit;

        assert_eq!(
            deciding(&order, &both, Some(&on(s1))),
            decided(&pro_s1, false)
        );
        assert_eq!(
            deciding(&order, &both, Some(&on(s3))),
            decided(&business, false)
        );
        assert_eq!(deciding(&order, &both, None), decided(&business, true));
        let ad_hoc = checked(
            site,
            ResourceRef::AdHoc {
                type_id: site,
                key_value: "s1".into(),
            },
        );
        assert_eq!(
            deciding(&order, &both, Some(&ad_hoc)),
            decided(&business, false)
        );

        let only_site = [pro_s1];
        assert_eq!(
            deciding(&order, &only_site, None),
            Deciding::ResourceRequired
        );
        let unrelated = checked(tenant, ResourceRef::Stored(ResourceId::new()));
        assert_eq!(
            deciding(&order, &only_site, Some(&unrelated)),
            Deciding::ResourceRequired
        );
        assert_eq!(
            deciding(&order, &only_site, Some(&on(s3))),
            Deciding::NotSubscribed
        );
        assert_eq!(
            deciding(&order, &[], Some(&on(s1))),
            Deciding::NoSubscription
        );
        assert_eq!(deciding(&order, &[], None), Deciding::NoSubscription);
    }

    #[test]
    fn the_best_value_then_the_oldest() {
        let order = ValueOrder::Limit;
        let older_starter = candidate(None, 1, n(10));
        let newer_pro = candidate(None, 9, n(100));
        assert_eq!(
            deciding(&order, &[older_starter.clone(), newer_pro.clone()], None),
            decided(&newer_pro, false)
        );
        let tie = candidate(None, 9, n(10));
        assert_eq!(
            deciding(&order, &[tie, older_starter.clone()], None),
            decided(&older_starter, false)
        );
        let nothing = candidate(None, 1, None);
        assert_eq!(
            deciding(&order, &[nothing, newer_pro.clone()], None),
            decided(&newer_pro, false)
        );
        let text = |s: &str| Some(GrantValue::Text(s.into()));
        let (a, b) = (candidate(None, 3, text("b")), candidate(None, 2, text("a")));
        assert_eq!(
            deciding(&ValueOrder::Unordered, &[a, b.clone()], None),
            decided(&b, false)
        );
    }
}
