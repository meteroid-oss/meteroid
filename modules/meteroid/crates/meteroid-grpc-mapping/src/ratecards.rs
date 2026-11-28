//! Rate cards between the store and the gRPC API, shared with the Enterprise cost cards.
use meteroid_grpc::meteroid::api::activity::v1::ActivityEntry;
use meteroid_grpc::meteroid::api::ratecards::v1 as proto;
use meteroid_store::domain::entity_activity::ActivityType;
use meteroid_store::domain::rate_cards::{
    RateCard, RateCardDenomination, RateCardEntry, RateCardRatesChanged, RateCardUnmatched,
};

pub fn unmatched_from_proto(value: proto::RateCardUnmatched) -> RateCardUnmatched {
    match value {
        proto::RateCardUnmatched::Hold => RateCardUnmatched::Hold,
        proto::RateCardUnmatched::Ignore => RateCardUnmatched::Ignore,
    }
}

fn unmatched_to_proto(value: RateCardUnmatched) -> proto::RateCardUnmatched {
    match value {
        RateCardUnmatched::Hold => proto::RateCardUnmatched::Hold,
        RateCardUnmatched::Ignore => proto::RateCardUnmatched::Ignore,
    }
}

pub fn card_to_proto(card: RateCard) -> proto::RateCard {
    proto::RateCard {
        id: card.id.as_proto(),
        code: card.code,
        name: card.name,
        metric_id: card.metric_id.as_proto(),
        dimensions: card.dimensions,
        on_unmatched: unmatched_to_proto(card.on_unmatched).into(),
        credit_unit_id: match card.denomination {
            RateCardDenomination::Money => None,
            RateCardDenomination::CreditUnit(unit) => Some(unit.as_proto()),
        },
        unit_size: card.unit_size.normalize().to_string(),
    }
}

pub fn entry_to_proto(entry: &RateCardEntry) -> proto::RateCardEntry {
    proto::RateCardEntry {
        id: entry.id.as_proto(),
        key: entry.key_text(),
        effective_from: entry.effective_from.to_string(),
        rate: entry.rate.map(|rate| rate.normalize().to_string()),
        label: entry.label.clone(),
        currency: entry.currency.clone(),
        any_other: entry.is_any_other(),
    }
}

fn any_other(key: &[String]) -> bool {
    key.iter().all(|pattern| pattern == "*")
}

/// A card's activity as its change log shows it; None for an activity of another kind.
pub fn change_to_proto(entry: ActivityEntry) -> Option<proto::RateCardChange> {
    let kind = match entry.activity_type.parse::<ActivityType>().ok()? {
        ActivityType::RateCardCreated => proto::RateCardChangeKind::Created,
        ActivityType::RateCardRatesChanged => proto::RateCardChangeKind::RatesSet,
        _ => return None,
    };
    let changed: RateCardRatesChanged = match kind {
        proto::RateCardChangeKind::Created => RateCardRatesChanged::default(),
        proto::RateCardChangeKind::RatesSet => entry
            .metadata_json
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok())
            .unwrap_or_default(),
    };
    let decimal = |rate: rust_decimal::Decimal| rate.normalize().to_string();
    Some(proto::RateCardChange {
        id: entry.id,
        kind: kind.into(),
        occurred_at: entry.occurred_at,
        actor_type: entry.actor_type,
        actor_id: entry.actor_id,
        actor_name: entry.actor_name,
        effective_from: changed.effective_from.map(|day| day.to_string()),
        rates: changed
            .changes
            .into_iter()
            .map(|change| proto::RateCardRateChange {
                any_other: any_other(&change.key),
                key: change.key,
                currency: change.currency,
                from_rate: change.from.map(decimal),
                to_rate: change.to.map(decimal),
            })
            .collect(),
        withdrawn: changed
            .withdrawn
            .into_iter()
            .map(|entry| proto::RateCardEntry {
                id: entry.id.as_proto(),
                any_other: any_other(&entry.key),
                key: entry.key,
                effective_from: entry.effective_from.to_string(),
                rate: entry.rate.map(decimal),
                label: entry.label,
                currency: entry.currency,
            })
            .collect(),
    })
}
