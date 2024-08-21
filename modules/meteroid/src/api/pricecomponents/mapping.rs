pub mod components {
    use crate::api::domain_mapping::billing_period;

    use crate::api::shared::conversions::*;
    use crate::api::subscriptions::ext::{
        billing_type_from_grpc, billing_type_to_grpc, usage_pricing_model_from_grpc,
        usage_pricing_model_to_grpc,
    };
    use meteroid_grpc::meteroid::api::components::v1 as api;

    use meteroid_grpc::meteroid::api::shared::v1 as api_shared;

    use meteroid_store::domain::price_components as domain;
    use rust_decimal::Decimal;
    use tonic::Status;
    use uuid::Uuid;

    // TODO dedicated mapping error instead of status
    pub fn create_api_to_domain(
        comp: api::CreatePriceComponentRequest,
    ) -> Result<domain::PriceComponentNew, Status> {
        Ok(domain::PriceComponentNew {
            name: comp.name,
            fee: map_fee_to_domain(comp.fee)?,
            product_item_id: Uuid::from_proto_opt(comp.product_item_id)?,
            plan_version_id: Uuid::from_proto_ref(&comp.plan_version_id)?,
        })
    }

    pub fn edit_api_to_domain(
        comp: api::EditPriceComponentRequest,
    ) -> Result<domain::PriceComponent, Status> {
        let component = comp
            .component
            .ok_or(Status::invalid_argument("component is missing"))?;

        Ok(domain::PriceComponent {
            name: component.name,
            fee: map_fee_to_domain(component.fee)?,
            product_item_id: Uuid::from_proto_opt(component.product_item_id)?,
            id: Uuid::from_proto_ref(&component.id)?,
        })
    }

    pub fn map_fee_to_domain(fee: Option<api::Fee>) -> Result<domain::FeeType, Status> {
        match fee.as_ref().and_then(|fee| fee.fee_type.as_ref()) {
            Some(s) => match s {
                api::fee::FeeType::Rate(fee) => Ok::<_, Status>(domain::FeeType::Rate {
                    rates: fee
                        .rates
                        .iter()
                        .map(|rate| {
                            Ok::<_, Status>(domain::TermRate {
                                term: billing_period::from_proto(rate.term()),
                                price: Decimal::from_proto_ref(&rate.price)?,
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                }),
                api::fee::FeeType::Slot(fee) => Ok(domain::FeeType::Slot {
                    minimum_count: fee.minimum_count.clone(),
                    slot_unit_name: fee.slot_unit_name.clone(),
                    upgrade_policy: domain::UpgradePolicy::Prorated,
                    downgrade_policy: domain::DowngradePolicy::RemoveAtEndOfPeriod,
                    quota: fee.quota.clone(),
                    rates: fee
                        .rates
                        .iter()
                        .map(|rate| {
                            Ok::<_, Status>(domain::TermRate {
                                term: billing_period::from_proto(rate.term()),
                                price: Decimal::from_proto_ref(&rate.price)?,
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                }),
                api::fee::FeeType::Capacity(fee) => Ok(domain::FeeType::Capacity {
                    metric_id: Uuid::from_proto_ref(&fee.metric_id)?,
                    thresholds: fee
                        .thresholds
                        .iter()
                        .map(|threshold| {
                            Ok::<_, Status>(domain::CapacityThreshold {
                                price: Decimal::from_proto_ref(&threshold.price)?,
                                per_unit_overage: Decimal::from_proto_ref(
                                    &threshold.per_unit_overage,
                                )?,
                                included_amount: threshold.included_amount,
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                }),
                api::fee::FeeType::ExtraRecurring(fee) => {
                    let cadence = fee
                        .term
                        .ok_or(Status::invalid_argument("recurring fee term is missing"))?;
                    let cadence = api_shared::BillingPeriod::try_from(cadence as i32)
                        .map_err(|_| Status::invalid_argument("invalid billing period"))?;

                    Ok(domain::FeeType::ExtraRecurring {
                        unit_price: Decimal::from_proto_ref(&fee.unit_price)?,
                        quantity: fee.quantity,
                        billing_type: billing_type_from_grpc(fee.billing_type())?,
                        cadence: billing_period::from_proto(cadence),
                    })
                }
                api::fee::FeeType::OneTime(fee) => Ok(domain::FeeType::OneTime {
                    quantity: fee.quantity,
                    unit_price: Decimal::from_proto_ref(&fee.unit_price)?,
                }),
                api::fee::FeeType::Usage(fee) => {
                    let mapped = usage_pricing_model_from_grpc(fee)?;

                    Ok(domain::FeeType::Usage {
                        metric_id: Uuid::from_proto_ref(&fee.metric_id)?,
                        pricing: mapped,
                    })
                }
            },
            None => Err(Status::invalid_argument("fee is missing")),
        }
    }

    pub fn domain_to_api(comp: domain::PriceComponent) -> api::PriceComponent {
        api::PriceComponent {
            id: comp.id.to_string(),
            name: comp.name.to_string(),
            fee: Some(map_fee_domain_to_api(comp.fee)),
            product_item_id: comp.product_item_id.as_proto(),
        }
    }

    pub fn map_fee_domain_to_api(fee: domain::FeeType) -> api::Fee {
        let fee_type = match fee {
            domain::FeeType::Rate { rates } => {
                let rates = rates
                    .into_iter()
                    .map(|rate| api::fee::TermRate {
                        term: billing_period::to_proto(rate.term).into(),
                        price: rate.price.as_proto(),
                    })
                    .collect();

                api::fee::FeeType::Rate(api::fee::RateFee { rates })
            }
            domain::FeeType::Slot {
                minimum_count,
                slot_unit_name,
                upgrade_policy: _,
                downgrade_policy: _,
                quota,
                rates,
            } => {
                let rates = rates
                    .into_iter()
                    .map(|rate| api::fee::TermRate {
                        term: billing_period::to_proto(rate.term).into(),
                        price: rate.price.as_proto(),
                    })
                    .collect();

                api::fee::FeeType::Slot(api::fee::SlotFee {
                    minimum_count,
                    slot_unit_name,
                    upgrade_policy: api::fee::UpgradePolicy::Prorated.into(),
                    quota,
                    rates,
                    downgrade_policy: api::fee::DowngradePolicy::RemoveAtEndOfPeriod.into(),
                })
            }
            domain::FeeType::Capacity {
                metric_id,
                thresholds,
            } => {
                let thresholds = thresholds
                    .into_iter()
                    .map(|threshold| api::fee::capacity_fee::CapacityThreshold {
                        price: threshold.price.as_proto(),
                        per_unit_overage: threshold.per_unit_overage.as_proto(),
                        included_amount: threshold.included_amount,
                    })
                    .collect();

                api::fee::FeeType::Capacity(api::fee::CapacityFee {
                    metric_id: metric_id.as_proto(),
                    thresholds,
                })
            }
            domain::FeeType::ExtraRecurring {
                unit_price,
                quantity,
                billing_type,
                cadence,
            } => {
                api::fee::FeeType::ExtraRecurring(api::fee::ExtraRecurringFee {
                    unit_price: unit_price.as_proto(),
                    quantity,
                    billing_type: billing_type_to_grpc(billing_type).into(),
                    term: Some(billing_period::to_proto(cadence).into()), // TODO when is that optional ??
                })
            }
            domain::FeeType::OneTime {
                quantity,
                unit_price,
            } => api::fee::FeeType::OneTime(api::fee::OneTimeFee {
                quantity,
                unit_price: unit_price.as_proto(),
            }),
            domain::FeeType::Usage { metric_id, pricing } => {
                let model = usage_pricing_model_to_grpc(&metric_id, &pricing);

                api::fee::FeeType::Usage(model)
            }
        };

        api::Fee {
            fee_type: Some(fee_type),
        }
    }
}
