use common_domain::ids::{FeatureId, PlanId, ProductId};
use meteroid_grpc::meteroid::api::pricingpage::v1 as proto;
use meteroid_store::domain::pricing_pages as domain;
use rust_decimal::Decimal;
use tonic::Status;

use crate::domain_mapping::billing_period;

pub fn plan_page_to_proto(page: domain::PlanPricingPage) -> proto::PlanPricingPage {
    proto::PlanPricingPage {
        plan_id: page.plan_id.as_proto(),
        tagline: page.tagline,
        popular: page.popular,
        cta_label: page.cta_label,
        sales_led: page.sales_led,
        hide_price: page.hide_price,
        contact_url: page.contact_url,
        product_order: page.product_order.iter().map(|id| id.as_proto()).collect(),
        hidden_product_ids: page
            .hidden_product_ids
            .iter()
            .map(|id| id.as_proto())
            .collect(),
        highlights: page
            .highlights
            .into_iter()
            .map(|highlight| proto::Highlight {
                item: Some(match highlight {
                    domain::Highlight::Feature(id) => {
                        proto::highlight::Item::FeatureId(id.as_proto())
                    }
                    domain::Highlight::Text(text) => proto::highlight::Item::Text(text),
                }),
            })
            .collect(),
    }
}

pub fn plan_page_from_proto(
    page: proto::PlanPricingPage,
) -> Result<domain::PlanPricingPage, Status> {
    Ok(domain::PlanPricingPage {
        plan_id: PlanId::from_proto(&page.plan_id)?,
        tagline: page.tagline,
        popular: page.popular,
        cta_label: page.cta_label,
        sales_led: page.sales_led,
        hide_price: page.hide_price,
        contact_url: page.contact_url,
        product_order: page
            .product_order
            .iter()
            .map(ProductId::from_proto)
            .collect::<Result<_, Status>>()?,
        hidden_product_ids: page
            .hidden_product_ids
            .iter()
            .map(ProductId::from_proto)
            .collect::<Result<_, Status>>()?,
        highlights: page
            .highlights
            .into_iter()
            .map(|highlight| match highlight.item {
                Some(proto::highlight::Item::FeatureId(id)) => {
                    Ok(domain::Highlight::Feature(FeatureId::from_proto(&id)?))
                }
                Some(proto::highlight::Item::Text(text)) => Ok(domain::Highlight::Text(text)),
                None => Err(Status::invalid_argument(
                    "A highlight is a feature or a text",
                )),
            })
            .collect::<Result<_, Status>>()?,
    })
}

/// The fixed amount: "0" when there is none.
fn amount_to_proto(amount: Decimal) -> String {
    if amount.is_zero() {
        "0".to_owned()
    } else {
        amount.to_string()
    }
}

fn extra_to_proto(extra: domain::CardExtra) -> proto::CardExtra {
    let kind = match extra.kind {
        domain::CardExtraKind::Base => proto::card_extra::Kind::Base,
        domain::CardExtraKind::PerSeat => proto::card_extra::Kind::PerSeat,
        domain::CardExtraKind::Usage => proto::card_extra::Kind::Usage,
        domain::CardExtraKind::OneTime => proto::card_extra::Kind::OneTime,
        domain::CardExtraKind::CreditPack => proto::card_extra::Kind::CreditPack,
        domain::CardExtraKind::Recurring => proto::card_extra::Kind::Recurring,
    };
    proto::CardExtra {
        kind: kind.into(),
        name: extra.name,
        amount: extra.amount.map(|amount| amount.to_string()),
        unit: extra.unit,
        period: extra
            .period
            .map(|period| billing_period::to_proto(period).into()),
        lead: extra.lead,
        levels: false,
        from_lowest: extra.from_lowest,
        included: extra.included,
        block_size: extra.block_size,
        product_ids: extra.product_ids.iter().map(|id| id.as_proto()).collect(),
    }
}

fn levels_to_proto(levels: domain::CardLevels) -> proto::CardLevels {
    let (more, more_contact_url, more_plan_name) = match levels.more {
        Some(more) => (true, more.contact_url, more.plan_name),
        None => (false, None, None),
    };
    proto::CardLevels {
        name: levels.name,
        levels: levels
            .levels
            .into_iter()
            .map(|level| proto::CardLevel {
                included: level.included,
                amount: amount_to_proto(level.amount),
                monthly_equivalent: level.monthly_equivalent.map(|amount| amount.to_string()),
                overage_rate: level.overage_rate.map(|rate| rate.to_string()),
            })
            .collect(),
        metric_name: levels.metric_name,
        unit_singular: levels.unit_singular,
        unit_plural: levels.unit_plural,
        unit_conversion_factor: levels.unit_conversion_factor,
        more,
        more_contact_url,
        more_plan_name,
    }
}

pub fn charge_to_proto(charge: domain::PricingCharge) -> proto::PricingCharge {
    let kind = match charge.kind {
        domain::PricingChargeKind::Fixed => proto::pricing_charge::Kind::Fixed,
        domain::PricingChargeKind::Seats => proto::pricing_charge::Kind::Seats,
        domain::PricingChargeKind::Usage => proto::pricing_charge::Kind::Usage,
        domain::PricingChargeKind::OneTime => proto::pricing_charge::Kind::OneTime,
        domain::PricingChargeKind::CreditPack => proto::pricing_charge::Kind::CreditPack,
    };
    proto::PricingCharge {
        product_id: charge.product_id.as_proto(),
        name: charge.name,
        kind: kind.into(),
        can_lead: charge.can_lead,
        can_hide: charge.can_hide,
    }
}

fn price_to_proto(price: domain::CardPrice) -> proto::CardPrice {
    // The levels' own extra, told by its product.
    let levels_product = price.levels.as_ref().and_then(|levels| levels.product_id);
    proto::CardPrice {
        amount: amount_to_proto(price.amount),
        period: price
            .period
            .map(|period| billing_period::to_proto(period).into()),
        monthly_equivalent: price.monthly_equivalent.map(|amount| amount.to_string()),
        saving_percent: price.saving_percent,
        extras: price
            .extras
            .into_iter()
            .map(|extra| {
                // Not the "+ usage" line folding it when left off: that one has no name.
                let levels = extra.kind == domain::CardExtraKind::Usage
                    && !extra.name.is_empty()
                    && levels_product.is_some_and(|id| extra.product_ids == [id]);
                proto::CardExtra {
                    levels,
                    ..extra_to_proto(extra)
                }
            })
            .collect(),
        from_lowest: price.from_lowest,
        partial: price.partial,
        levels: price.levels.map(levels_to_proto),
    }
}

fn card_to_proto(card: domain::PricingCard) -> proto::PricingCard {
    proto::PricingCard {
        plan_id: card.plan_id.as_proto(),
        plan_name: card.plan_name,
        tagline: card.tagline,
        popular: card.popular,
        cta_label: card.cta_label,
        free: card.free,
        price: card.price.map(price_to_proto),
        includes_plan_name: card.includes_plan_name,
        highlights: card
            .highlights
            .into_iter()
            .map(|highlight| proto::RenderedHighlight {
                text: highlight.text,
                feature_id: highlight.feature_id.map(|id| id.as_proto()),
            })
            .collect(),
        trial_days: card.trial_days,
        options: card.options,
        sales_led: card.sales_led,
        hide_price: card.hide_price,
        contact_url: card.contact_url,
        option_note: card.option_note,
    }
}

pub fn page_to_proto(page: domain::PricingPage) -> proto::PricingPage {
    proto::PricingPage {
        product_family_id: page.product_family_id.as_proto(),
        product_family_name: page.product_family_name,
        currency: page.currency,
        currencies: page.currencies,
        cards: page.cards.into_iter().map(card_to_proto).collect(),
        option: page.option,
        options: page
            .options
            .into_iter()
            .map(|option| proto::PageOption {
                alias: option.alias,
                name: option.name,
            })
            .collect(),
        not_sold_plan_names: page.not_sold,
    }
}
