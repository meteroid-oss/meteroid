// shared mappers between grpc and meteroid-store domain

use meteroid_grpc::meteroid::api::shared::v1 as api_shared;
use meteroid_store::domain;

pub(crate) mod billing_period {
    use super::*;

    pub fn from_proto(period: api_shared::BillingPeriod) -> domain::enums::BillingPeriodEnum {
        match period {
            api_shared::BillingPeriod::Monthly => domain::enums::BillingPeriodEnum::Monthly,
            api_shared::BillingPeriod::Quarterly => domain::enums::BillingPeriodEnum::Quarterly,
            api_shared::BillingPeriod::Annual => domain::enums::BillingPeriodEnum::Annual,
        }
    }

    pub fn to_proto(period: domain::enums::BillingPeriodEnum) -> api_shared::BillingPeriod {
        match period {
            domain::enums::BillingPeriodEnum::Monthly => api_shared::BillingPeriod::Monthly,
            domain::enums::BillingPeriodEnum::Quarterly => api_shared::BillingPeriod::Quarterly,
            domain::enums::BillingPeriodEnum::Annual => api_shared::BillingPeriod::Annual,
        }
    }
}
