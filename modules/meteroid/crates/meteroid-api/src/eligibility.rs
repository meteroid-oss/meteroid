//! The eligibility rule of plans and coupons, as the API carries it.
use meteroid_grpc::meteroid::api::eligibility::v1 as server;
use meteroid_store::domain::eligibility::{Eligibility, PropertyMatch};

pub fn to_proto(rule: Eligibility) -> server::Eligibility {
    server::Eligibility {
        countries: rule.countries,
        properties: rule
            .properties
            .into_iter()
            .map(|p| server::PropertyMatch {
                key: p.key,
                values: p.values,
            })
            .collect(),
    }
}

/// The store normalizes and validates it; an empty rule lets anyone in.
pub fn from_proto(rule: server::Eligibility) -> Eligibility {
    Eligibility {
        countries: rule.countries,
        properties: rule
            .properties
            .into_iter()
            .map(|p| PropertyMatch {
                key: p.key,
                values: p.values,
            })
            .collect(),
    }
}
