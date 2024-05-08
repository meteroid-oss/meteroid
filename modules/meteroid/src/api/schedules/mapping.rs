pub mod schedules {
    use crate::api::domain_mapping::billing_period;
    use crate::api::domain_mapping::discount::{
        ServerAmountWrapper, ServerStandardDiscountWrapper,
    };
    use error_stack::Report;
    use meteroid_grpc::meteroid::api::schedules::v1 as server;
    use meteroid_store::domain;
    use meteroid_store::errors::StoreError;

    pub struct PlanRampsWrapper(pub server::PlanRamps);
    impl From<domain::PlanRamps> for PlanRampsWrapper {
        fn from(value: domain::PlanRamps) -> Self {
            PlanRampsWrapper(server::PlanRamps {
                ramps: value
                    .ramps
                    .into_iter()
                    .map(|ramp| server::plan_ramps::PlanRamp {
                        index: ramp.index,
                        duration_in_months: ramp.duration_in_months,
                        ramp_adjustment: Some(server::plan_ramps::plan_ramp::PlanRampAdjustment {
                            minimum: Some(
                                ServerAmountWrapper::from(ramp.ramp_adjustment.minimum).0,
                            ),
                            discount: Some(
                                ServerStandardDiscountWrapper::from(ramp.ramp_adjustment.discount)
                                    .0,
                            ),
                        }),
                    })
                    .collect(),
            })
        }
    }

    impl TryFrom<PlanRampsWrapper> for domain::PlanRamps {
        type Error = Report<StoreError>;

        fn try_from(value: PlanRampsWrapper) -> Result<Self, Self::Error> {
            Ok(domain::PlanRamps {
                ramps: value
                    .0
                    .ramps
                    .into_iter()
                    .map(|ramp| {
                        let ramp_adj = ramp.ramp_adjustment.ok_or_else(|| {
                            StoreError::InvalidArgument("missing ramp_adjustment".into())
                        })?;

                        let minimum: domain::adjustments::discount::Amount =
                            ServerAmountWrapper(ramp_adj.minimum.ok_or_else(|| {
                                StoreError::InvalidArgument("missing minimum".into())
                            })?)
                            .into();

                        let discount: domain::adjustments::discount::StandardDiscount =
                            ServerStandardDiscountWrapper(ramp_adj.discount.ok_or_else(|| {
                                StoreError::InvalidArgument("missing discount".into())
                            })?)
                            .try_into()?;

                        Ok::<domain::PlanRamp, Report<StoreError>>(domain::PlanRamp {
                            index: ramp.index,
                            duration_in_months: ramp.duration_in_months,
                            ramp_adjustment: domain::PlanRampAdjustment { minimum, discount },
                        })
                    })
                    .collect::<Result<_, _>>()?,
            })
        }
    }

    pub struct ScheduleWrapper(pub server::Schedule);
    impl From<domain::Schedule> for ScheduleWrapper {
        fn from(value: domain::Schedule) -> Self {
            ScheduleWrapper(server::Schedule {
                id: value.id.to_string(),
                term: billing_period::to_proto(value.billing_period) as i32,
                name: "".to_string(), // TODO drop from db ?
                ramps: Some(PlanRampsWrapper::from(value.ramps).0),
            })
        }
    }
}
