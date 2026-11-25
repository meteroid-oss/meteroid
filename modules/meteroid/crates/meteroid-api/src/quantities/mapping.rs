use meteroid_grpc::meteroid::api::quantities::v1 as proto;
use meteroid_store::domain::managed_quantities::{
    ManagedQuantity, QuantityDecrease, QuantityIncrease,
};

pub fn increase_to_proto(increase: QuantityIncrease) -> proto::QuantityIncrease {
    match increase {
        QuantityIncrease::Immediately => proto::QuantityIncrease::Immediately,
        QuantityIncrease::NextPeriod => proto::QuantityIncrease::NextPeriod,
    }
}

pub fn increase_from_proto(increase: proto::QuantityIncrease) -> QuantityIncrease {
    match increase {
        proto::QuantityIncrease::Immediately => QuantityIncrease::Immediately,
        proto::QuantityIncrease::NextPeriod => QuantityIncrease::NextPeriod,
    }
}

pub fn decrease_to_proto(decrease: QuantityDecrease) -> proto::QuantityDecrease {
    match decrease {
        QuantityDecrease::AtPeriodEnd => proto::QuantityDecrease::AtPeriodEnd,
        QuantityDecrease::Immediately => proto::QuantityDecrease::Immediately,
    }
}

pub fn decrease_from_proto(decrease: proto::QuantityDecrease) -> QuantityDecrease {
    match decrease {
        proto::QuantityDecrease::AtPeriodEnd => QuantityDecrease::AtPeriodEnd,
        proto::QuantityDecrease::Immediately => QuantityDecrease::Immediately,
    }
}

pub fn to_proto(quantity: ManagedQuantity) -> proto::ManagedQuantity {
    proto::ManagedQuantity {
        id: quantity.id.as_proto(),
        code: quantity.code,
        name: quantity.name,
        on_increase: increase_to_proto(quantity.on_increase).into(),
        on_decrease: decrease_to_proto(quantity.on_decrease).into(),
        archived: quantity.archived_at.is_some(),
        per_unit_feature_ids: quantity
            .per_unit_features
            .iter()
            .map(|id| id.as_proto())
            .collect(),
    }
}
