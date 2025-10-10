// shared mappers between grpc and meteroid-store domain

use meteroid_grpc::meteroid::api::shared::v1 as api_shared;
use meteroid_store::domain;

pub(crate) mod billing_period {
    use super::{api_shared, domain};

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

pub(crate) mod discount {
    use super::domain;
    use common_grpc::meteroid::common::v1 as api_common;
    use error_stack::Report;
    use meteroid_grpc::meteroid::api::adjustments::v1 as api_adjustments;
    use meteroid_store::errors::StoreError;
    use std::str::FromStr;

    pub(crate) struct ServerAmountWrapper(pub api_adjustments::discount::Amount);
    impl From<domain::adjustments::discount::Amount> for ServerAmountWrapper {
        fn from(value: domain::adjustments::discount::Amount) -> Self {
            ServerAmountWrapper(api_adjustments::discount::Amount {
                value_in_cents: value.value_in_cents,
            })
        }
    }

    impl From<ServerAmountWrapper> for domain::adjustments::discount::Amount {
        fn from(value: ServerAmountWrapper) -> Self {
            domain::adjustments::discount::Amount {
                value_in_cents: value.0.value_in_cents,
            }
        }
    }

    pub(crate) struct ServerPercentWrapper(pub api_adjustments::discount::Percent);
    impl From<domain::adjustments::discount::Percent> for ServerPercentWrapper {
        fn from(value: domain::adjustments::discount::Percent) -> Self {
            ServerPercentWrapper(api_adjustments::discount::Percent {
                percentage: Some(api_common::Decimal {
                    value: value.percentage.to_string(),
                }),
            })
        }
    }

    impl TryFrom<ServerPercentWrapper> for domain::adjustments::discount::Percent {
        type Error = Report<StoreError>;

        fn try_from(value: ServerPercentWrapper) -> Result<Self, Self::Error> {
            let percentage = value
                .0
                .percentage
                .ok_or(StoreError::InvalidArgument("missing percentage".into()))?;

            Ok(domain::adjustments::discount::Percent {
                percentage: rust_decimal::Decimal::from_str(&percentage.value)
                    .map_err(|_| StoreError::InvalidArgument("invalid percentage".into()))?,
            })
        }
    }

    pub(crate) struct ServerStandardDiscountWrapper(pub api_adjustments::StandardDiscount);
    impl From<domain::adjustments::discount::StandardDiscount> for ServerStandardDiscountWrapper {
        fn from(value: domain::adjustments::discount::StandardDiscount) -> Self {
            match value {
                domain::adjustments::discount::StandardDiscount::Amount(amount) => {
                    ServerStandardDiscountWrapper(api_adjustments::StandardDiscount {
                        discount_type: Some(
                            api_adjustments::standard_discount::DiscountType::Amount(
                                ServerAmountWrapper::from(amount).0,
                            ),
                        ),
                    })
                }
                domain::adjustments::discount::StandardDiscount::Percent(percent) => {
                    ServerStandardDiscountWrapper(api_adjustments::StandardDiscount {
                        discount_type: Some(
                            api_adjustments::standard_discount::DiscountType::Percent(
                                ServerPercentWrapper::from(percent).0,
                            ),
                        ),
                    })
                }
            }
        }
    }

    impl TryFrom<ServerStandardDiscountWrapper> for domain::adjustments::discount::StandardDiscount {
        type Error = Report<StoreError>;

        fn try_from(value: ServerStandardDiscountWrapper) -> Result<Self, Self::Error> {
            let discount_type = value
                .0
                .discount_type
                .ok_or(StoreError::InvalidArgument("missing discount_type".into()))?;

            match discount_type {
                api_adjustments::standard_discount::DiscountType::Amount(amount) => {
                    Ok(domain::adjustments::discount::StandardDiscount::Amount(
                        ServerAmountWrapper(amount).into(),
                    ))
                }
                api_adjustments::standard_discount::DiscountType::Percent(percent) => {
                    Ok(domain::adjustments::discount::StandardDiscount::Percent(
                        ServerPercentWrapper(percent).try_into()?,
                    ))
                }
            }
        }
    }
}
