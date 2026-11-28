use std::collections::BTreeMap;

use common_domain::ids::{AddOnId, FeatureId};
use meteroid_app::conversions::ProtoConv;
use meteroid_grpc::meteroid::api::offers::v1 as proto;
use meteroid_grpc::meteroid::api::plans::v1 as plans_proto;
use meteroid_store::domain::coupons::{CouponCreditScope, CouponDiscount};
use meteroid_store::domain::offers::{
    AddOnEnd, GrantAddOn, GrantAnchor, GrantCredit, GrantDefinition, GrantDiscount, GrantDuration,
    GrantEntitlement, GrantKind, GrantStatus, GrantTarget, GrantWindow, Offer, OfferGrant,
    OfferGrantNew, OfferSource, OfferTrial, OfferUse, SubscriptionGrant,
};
use meteroid_store::domain::trials::TrialEligibility;
use rust_decimal::Decimal;
use tonic::Status;

use crate::entitlements::mapping::{
    effect_from_proto, effect_to_proto, grant_value_from_proto, grant_value_to_proto,
};
use crate::plans::mapping::plans::{
    trial_eligibility_to_proto, trial_terms_from_proto, trial_terms_to_proto,
};

pub(crate) fn eligibility_from_proto(value: i32) -> Result<TrialEligibility, Status> {
    use plans_proto::TrialEligibility as Proto;
    Ok(
        match Proto::try_from(value).map_err(|_| Status::invalid_argument("unknown eligibility"))? {
            Proto::None => TrialEligibility::None,
            Proto::Offer => TrialEligibility::Offer,
            Proto::Plan => TrialEligibility::Plan,
            Proto::ProductFamily => TrialEligibility::ProductFamily,
        },
    )
}

pub(crate) fn trial_from_proto(
    trial: Option<plans_proto::TrialTerms>,
) -> Result<Option<OfferTrial>, Status> {
    Ok(trial
        .map(trial_terms_from_proto)
        .transpose()?
        .map(OfferTrial::from_terms))
}

pub(crate) fn offer_to_proto(offer: Offer, used_by: OfferUse) -> proto::Offer {
    proto::Offer {
        id: offer.id.as_proto(),
        name: offer.name,
        description: offer.description,
        trial: offer.trial.map(|trial| trial_terms_to_proto(trial.terms())),
        grants: offer.grants.iter().map(grant_to_proto).collect(),
        eligibility: trial_eligibility_to_proto(offer.eligibility) as i32,
        standalone: offer.standalone,
        archived: offer.archived_at.is_some(),
        used_by: Some(proto::OfferUse {
            plan_ids: used_by.plans.iter().map(|id| id.as_proto()).collect(),
            coupons: used_by
                .coupons
                .into_iter()
                .map(|coupon| proto::OfferCoupon {
                    id: coupon.id.as_proto(),
                    code: coupon.code,
                })
                .collect(),
            applications: used_by.applications,
            retention_offer_ids: used_by.retention.iter().map(|id| id.as_proto()).collect(),
        }),
    }
}

fn kind_to_proto(kind: GrantKind) -> proto::grant::Kind {
    match kind {
        GrantKind::Discount => proto::grant::Kind::Discount,
        GrantKind::Credit => proto::grant::Kind::Credit,
        GrantKind::Entitlement => proto::grant::Kind::Entitlement,
        GrantKind::AddOn => proto::grant::Kind::AddOn,
    }
}

pub(crate) fn grant_to_proto(grant: &OfferGrant) -> proto::Grant {
    use proto::grant::Definition;
    proto::Grant {
        id: grant.id.as_proto(),
        kind: kind_to_proto(grant.kind()) as i32,
        window: Some(window_to_proto(grant.window)),
        definition: Some(match &grant.definition {
            GrantDefinition::Discount(discount) => {
                Definition::Discount(discount_to_proto(discount))
            }
            GrantDefinition::Credit(credit) => Definition::Credit(credit_to_proto(credit)),
            GrantDefinition::Entitlement(entitlement) => {
                Definition::Entitlement(entitlement_to_proto(entitlement))
            }
            GrantDefinition::AddOn(add_on) => Definition::AddOn(add_on_to_proto(add_on)),
        }),
    }
}

pub(crate) fn grants_from_proto(
    grants: Vec<proto::GrantNew>,
) -> Result<Vec<OfferGrantNew>, Status> {
    grants.into_iter().map(grant_new_from_proto).collect()
}

fn grant_new_from_proto(grant: proto::GrantNew) -> Result<OfferGrantNew, Status> {
    use proto::grant_new::Definition;
    Ok(OfferGrantNew {
        window: window_from_proto(grant.window)?,
        definition: match grant.definition {
            Some(Definition::Discount(discount)) => {
                GrantDefinition::Discount(discount_from_proto(discount)?)
            }
            Some(Definition::Credit(credit)) => GrantDefinition::Credit(credit_from_proto(credit)?),
            Some(Definition::Entitlement(entitlement)) => {
                GrantDefinition::Entitlement(entitlement_from_proto(entitlement)?)
            }
            Some(Definition::AddOn(add_on)) => GrantDefinition::AddOn(add_on_from_proto(add_on)?),
            None => return Err(Status::invalid_argument("a grant has its definition")),
        },
    })
}

fn window_to_proto(window: GrantWindow) -> proto::GrantWindow {
    use proto::grant_window::{Anchor, Duration};
    proto::GrantWindow {
        anchor: match window.anchor {
            GrantAnchor::Start => Anchor::Start,
            GrantAnchor::BillingStart => Anchor::BillingStart,
        } as i32,
        duration: Some(match window.duration {
            GrantDuration::Forever => Duration::Forever(true),
            GrantDuration::Days(days) => Duration::Days(days),
            GrantDuration::Months(months) => Duration::Months(months),
            GrantDuration::UntilBillingStarts => Duration::UntilBillingStarts(true),
            GrantDuration::Invoices(invoices) => Duration::Invoices(invoices),
        }),
    }
}

fn window_from_proto(window: Option<proto::GrantWindow>) -> Result<GrantWindow, Status> {
    use proto::grant_window::{Anchor, Duration};
    let window = window.ok_or_else(|| Status::invalid_argument("a grant has its window"))?;
    Ok(GrantWindow {
        anchor: match Anchor::try_from(window.anchor)
            .map_err(|_| Status::invalid_argument("unknown grant anchor"))?
        {
            Anchor::Start => GrantAnchor::Start,
            Anchor::BillingStart => GrantAnchor::BillingStart,
        },
        duration: match window.duration {
            Some(Duration::Forever(_)) => GrantDuration::Forever,
            Some(Duration::Days(days)) => GrantDuration::Days(days),
            Some(Duration::Months(months)) => GrantDuration::Months(months),
            Some(Duration::UntilBillingStarts(_)) => GrantDuration::UntilBillingStarts,
            Some(Duration::Invoices(invoices)) => GrantDuration::Invoices(invoices),
            None => return Err(Status::invalid_argument("a grant has its duration")),
        },
    })
}

fn currency_amount(currency: &str, amount: &Decimal) -> proto::CurrencyAmount {
    proto::CurrencyAmount {
        currency: currency.to_string(),
        amount: amount.as_proto(),
    }
}

fn discount_to_proto(discount: &GrantDiscount) -> proto::GrantDiscount {
    use proto::grant_discount::Discount;
    proto::GrantDiscount {
        discount: match &discount.discount {
            CouponDiscount::Percentage(rate) => Some(Discount::Percentage(rate.as_proto())),
            CouponDiscount::Fixed { currency, amount } => {
                Some(Discount::Fixed(currency_amount(currency, amount)))
            }
            CouponDiscount::Credit { .. } => None,
        },
        fixed_amounts: discount
            .fixed_amounts
            .iter()
            .map(|(currency, amount)| currency_amount(currency, amount))
            .collect(),
        add_on_ids: discount.add_on_ids.iter().map(|id| id.as_proto()).collect(),
    }
}

/// An amount repeating the discount's own currency and amount is dropped.
fn discount_from_proto(discount: proto::GrantDiscount) -> Result<GrantDiscount, Status> {
    use proto::grant_discount::Discount;
    let value = match discount.discount {
        Some(Discount::Percentage(rate)) => CouponDiscount::Percentage(Decimal::from_proto(rate)?),
        Some(Discount::Fixed(fixed)) => CouponDiscount::Fixed {
            amount: Decimal::from_proto_ref(&fixed.amount)?,
            currency: fixed.currency,
        },
        None => {
            return Err(Status::invalid_argument(
                "a discount is a percentage or a fixed amount",
            ));
        }
    };
    let mut fixed_amounts = BTreeMap::new();
    for entry in discount.fixed_amounts {
        let amount = Decimal::from_proto_ref(&entry.amount)?;
        if matches!(&value, CouponDiscount::Fixed { currency, amount: own }
            if *currency == entry.currency && *own == amount)
        {
            continue;
        }
        if fixed_amounts
            .insert(entry.currency.clone(), amount)
            .is_some()
        {
            return Err(Status::invalid_argument(format!(
                "{} is given more than one amount",
                entry.currency
            )));
        }
    }
    Ok(GrantDiscount {
        discount: value,
        fixed_amounts,
        add_on_ids: discount
            .add_on_ids
            .iter()
            .map(AddOnId::from_proto)
            .collect::<Result<_, _>>()?,
    })
}

fn credit_to_proto(credit: &GrantCredit) -> proto::GrantCredit {
    use proto::grant_credit::Scope;
    proto::GrantCredit {
        currency: credit.currency.clone(),
        amount: credit.amount.as_proto(),
        scope: match credit.scope {
            CouponCreditScope::Customer => Scope::Customer,
            CouponCreditScope::Subscription => Scope::Subscription,
        } as i32,
    }
}

fn credit_from_proto(credit: proto::GrantCredit) -> Result<GrantCredit, Status> {
    use proto::grant_credit::Scope;
    Ok(GrantCredit {
        amount: Decimal::from_proto_ref(&credit.amount)?,
        scope: match Scope::try_from(credit.scope)
            .map_err(|_| Status::invalid_argument("unknown credit scope"))?
        {
            Scope::Customer => CouponCreditScope::Customer,
            Scope::Subscription => CouponCreditScope::Subscription,
        },
        currency: credit.currency,
    })
}

fn entitlement_to_proto(entitlement: &GrantEntitlement) -> proto::GrantEntitlement {
    proto::GrantEntitlement {
        feature_id: entitlement.feature_id.as_proto(),
        value: Some(grant_value_to_proto(entitlement.value.clone())),
        effect: entitlement.effect.map_or(
            meteroid_grpc::meteroid::api::entitlements::v1::EntitlementEffect::Unspecified as i32,
            effect_to_proto,
        ),
    }
}

fn entitlement_from_proto(
    entitlement: proto::GrantEntitlement,
) -> Result<GrantEntitlement, Status> {
    Ok(GrantEntitlement {
        feature_id: FeatureId::from_proto(&entitlement.feature_id)?,
        value: grant_value_from_proto(entitlement.value)?,
        effect: effect_from_proto(entitlement.effect)?,
    })
}

fn add_on_to_proto(add_on: &GrantAddOn) -> proto::GrantAddOn {
    use proto::grant_add_on::AtEnd;
    proto::GrantAddOn {
        add_on_id: add_on.add_on_id.as_proto(),
        quantity: add_on.quantity,
        at_end: match add_on.at_end {
            AddOnEnd::Revert => AtEnd::Revert,
            AddOnEnd::Convert => AtEnd::Convert,
        } as i32,
    }
}

fn add_on_from_proto(add_on: proto::GrantAddOn) -> Result<GrantAddOn, Status> {
    use proto::grant_add_on::AtEnd;
    Ok(GrantAddOn {
        add_on_id: AddOnId::from_proto(&add_on.add_on_id)?,
        quantity: add_on.quantity,
        at_end: match AtEnd::try_from(add_on.at_end)
            .map_err(|_| Status::invalid_argument("unknown add-on end"))?
        {
            AtEnd::Revert => AddOnEnd::Revert,
            AtEnd::Convert => AddOnEnd::Convert,
        },
    })
}

fn source_to_proto(source: OfferSource) -> proto::subscription_grant::Source {
    use proto::subscription_grant::Source;
    match source {
        OfferSource::Plan => Source::Plan,
        OfferSource::Operator => Source::Operator,
        OfferSource::Coupon => Source::Coupon,
        OfferSource::Retention => Source::Retention,
        OfferSource::Quote => Source::Quote,
        OfferSource::Api => Source::Api,
    }
}

fn status_to_proto(status: GrantStatus) -> proto::subscription_grant::Status {
    use proto::subscription_grant::Status as Proto;
    match status {
        GrantStatus::Scheduled => Proto::Scheduled,
        GrantStatus::Active => Proto::Active,
        GrantStatus::Ended => Proto::Ended,
        GrantStatus::Withdrawn => Proto::Withdrawn,
    }
}

/// A dated add-on's timeline item before it gives its add-on has no row to name.
fn target_to_proto(target: &GrantTarget) -> Option<proto::GrantRef> {
    use proto::grant_ref::Target;
    let target = match target {
        GrantTarget::AppliedCoupon(id) => Target::AppliedCouponId(id.as_proto()),
        GrantTarget::CreditGrant(id) => Target::CreditGrantId(id.as_proto()),
        GrantTarget::Entitlement(id) => Target::EntitlementId(id.as_proto()),
        GrantTarget::AddOn { add_on, .. } => Target::SubscriptionAddOnId((*add_on)?.as_proto()),
    };
    Some(proto::GrantRef {
        target: Some(target),
    })
}

pub(crate) fn subscription_grant_to_proto(grant: &SubscriptionGrant) -> proto::SubscriptionGrant {
    proto::SubscriptionGrant {
        grant: Some(grant_to_proto(&grant.grant)),
        status: status_to_proto(grant.status) as i32,
        from: grant.from.as_proto(),
        until: grant.until.map(|until| until.as_proto()),
        source: source_to_proto(grant.application.source) as i32,
        offer_id: grant.application.offer_id.as_proto(),
        coupon_id: grant.application.coupon_id.map(|id| id.as_proto()),
        application_id: grant.application.id.as_proto(),
        refs: grant.targets.iter().filter_map(target_to_proto).collect(),
    }
}
