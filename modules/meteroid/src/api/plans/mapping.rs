pub mod plans {
    use crate::api::domain_mapping::billing_period::to_proto;
    use meteroid_grpc::meteroid::api::plans::v1::plan_billing_configuration as billing_config_grpc;
    use meteroid_grpc::meteroid::api::plans::v1::{
        ListPlan, ListSubscribablePlanVersion, Plan, PlanBillingConfiguration, PlanDetails,
        PlanStatus, PlanType, PlanVersion, TrialConfig,
    };
    use meteroid_store::domain;
    use meteroid_store::domain::enums::{PlanStatusEnum, PlanTypeEnum};

    pub struct PlanDetailsWrapper(pub PlanDetails);
    pub struct PlanTypeWrapper(pub PlanType);
    pub struct PlanStatusWrapper(pub PlanStatus);
    pub struct ListPlanWrapper(pub ListPlan);
    pub struct ListSubscribablePlanVersionWrapper(pub ListSubscribablePlanVersion);

    impl From<domain::FullPlan> for PlanDetailsWrapper {
        fn from(value: domain::FullPlan) -> Self {
            fn trial_config(version: &domain::PlanVersion) -> Option<TrialConfig> {
                Some(TrialConfig {
                    duration_in_days: version.trial_duration_days? as u32,
                    fallback_plan_id: version.trial_fallback_plan_id?.to_string(),
                })
            }

            fn billing_config(version: &domain::PlanVersion) -> Option<PlanBillingConfiguration> {
                Some(PlanBillingConfiguration {
                    billing_periods: version
                        .billing_periods
                        .clone()
                        .into_iter()
                        .map(|freq| to_proto(freq) as i32)
                        .collect(),
                    billing_cycles: Some(match version.billing_cycles {
                        Some(count) => {
                            billing_config_grpc::BillingCycles::Fixed(billing_config_grpc::Fixed {
                                count: count as u32,
                            })
                        }
                        None => billing_config_grpc::BillingCycles::Forever(
                            billing_config_grpc::Forever {},
                        ),
                    }),
                    net_terms: version.net_terms as u32,
                    service_period_start: Some(match version.period_start_day {
                        Some(day) => billing_config_grpc::ServicePeriodStart::DayOfMonth(
                            billing_config_grpc::DayOfMonth {
                                day_of_month: day as u32,
                            },
                        ),
                        None => billing_config_grpc::ServicePeriodStart::SubscriptionAnniversary(
                            billing_config_grpc::SubscriptionAnniversary {},
                        ),
                    }),
                })
            }

            Self(PlanDetails {
                plan: Some(Plan {
                    id: value.plan.id.to_string(),
                    external_id: value.plan.external_id,
                    name: value.plan.name,
                    description: value.plan.description,
                    plan_type: PlanTypeWrapper::from(value.plan.plan_type).0 as i32,
                    plan_status: PlanStatusWrapper::from(value.plan.status).0 as i32,
                }),
                current_version: Some(PlanVersion {
                    id: value.version.id.to_string(),
                    version: value.version.version as u32,
                    is_draft: value.version.is_draft_version,
                    trial_config: trial_config(&value.version),
                    billing_config: billing_config(&value.version),
                    currency: value.version.currency,
                }),
                metadata: vec![],
            })
        }
    }

    impl Into<PlanTypeEnum> for PlanTypeWrapper {
        fn into(self) -> PlanTypeEnum {
            match self.0 {
                PlanType::Standard => PlanTypeEnum::Standard,
                PlanType::Free => PlanTypeEnum::Free,
                PlanType::Custom => PlanTypeEnum::Custom,
            }
        }
    }

    impl From<PlanTypeEnum> for PlanTypeWrapper {
        fn from(e: PlanTypeEnum) -> Self {
            Self(match e {
                PlanTypeEnum::Standard => PlanType::Standard,
                PlanTypeEnum::Free => PlanType::Free,
                PlanTypeEnum::Custom => PlanType::Custom,
            })
        }
    }

    impl Into<PlanStatusEnum> for PlanStatusWrapper {
        fn into(self) -> PlanStatusEnum {
            match self.0 {
                PlanStatus::Draft => PlanStatusEnum::Draft,
                PlanStatus::Active => PlanStatusEnum::Active,
                PlanStatus::Archived => PlanStatusEnum::Archived,
                PlanStatus::Inactive => PlanStatusEnum::Inactive,
            }
        }
    }

    impl From<PlanStatusEnum> for PlanStatusWrapper {
        fn from(e: PlanStatusEnum) -> Self {
            Self(match e {
                PlanStatusEnum::Draft => PlanStatus::Draft,
                PlanStatusEnum::Active => PlanStatus::Active,
                PlanStatusEnum::Archived => PlanStatus::Archived,
                PlanStatusEnum::Inactive => PlanStatus::Inactive,
            })
        }
    }

    impl From<domain::PlanForList> for ListPlanWrapper {
        fn from(value: domain::PlanForList) -> Self {
            Self(ListPlan {
                id: value.id.to_string(),
                name: value.name,
                external_id: value.external_id,
                description: value.description,
                plan_type: PlanTypeWrapper::from(value.plan_type).0 as i32,
                plan_status: PlanStatusWrapper::from(value.status).0 as i32,
                product_family_id: value.product_family_id.to_string(),
                product_family_name: value.product_family_name,
            })
        }
    }

    impl From<domain::PlanVersionLatest> for ListSubscribablePlanVersionWrapper {
        fn from(value: domain::PlanVersionLatest) -> Self {
            Self(ListSubscribablePlanVersion {
                id: value.id.to_string(),
                plan_id: value.plan_id.to_string(),
                plan_name: value.plan_name,
                version: value.version,
                created_by: value.created_by.to_string(),
                trial_duration_days: value.trial_duration_days,
                trial_fallback_plan_id: value.trial_fallback_plan_id.map(|x| x.to_string()),
                period_start_day: value.period_start_day.map(|x| x as i32),
                net_terms: value.net_terms,
                currency: value.currency,
                product_family_id: value.product_family_id.to_string(),
                product_family_name: value.product_family_name,
            })
        }
    }

    pub mod overview {
        use meteroid_grpc::meteroid::api::plans::v1::PlanOverview;
        use meteroid_repository::plans::PlanOverview as DbPlanOverview;

        use crate::api::shared::mapping::period::billing_period_to_server;

        pub fn db_to_server(plan: DbPlanOverview) -> PlanOverview {
            PlanOverview {
                plan_id: plan.id.to_string(),
                plan_version_id: plan.plan_version_id.to_string(),
                version: plan.version as u32,
                name: plan.name,
                description: plan.description,
                net_terms: plan.net_terms as u32,
                currency: plan.currency,
                billing_periods: plan
                    .billing_periods
                    .iter()
                    .map(|freq| billing_period_to_server(freq) as i32)
                    .collect(),
                is_draft: plan.is_draft_version,
            }
        }
    }

    pub mod version {
        use meteroid_grpc::meteroid::api::plans::v1::plan_billing_configuration as billing_config_grpc;
        use meteroid_grpc::meteroid::api::plans::v1::{ListPlanVersion, PlanVersion};
        use meteroid_repository::plans::{
            ListPlanVersion as DbListPlanVersion, PlanVersion as DbPlanVersion,
        };

        use crate::api::shared::mapping::period::billing_period_to_server;

        fn map_trial_config(
            version: &DbPlanVersion,
        ) -> Option<meteroid_grpc::meteroid::api::plans::v1::TrialConfig> {
            match (version.trial_duration_days, version.trial_fallback_plan_id) {
                (Some(duration), Some(fallback)) => {
                    Some(meteroid_grpc::meteroid::api::plans::v1::TrialConfig {
                        duration_in_days: duration as u32,
                        fallback_plan_id: fallback.to_string(),
                    })
                }
                _ => None,
            }
        }

        fn map_billing_config(
            version: &DbPlanVersion,
        ) -> Option<meteroid_grpc::meteroid::api::plans::v1::PlanBillingConfiguration> {
            Some(
                meteroid_grpc::meteroid::api::plans::v1::PlanBillingConfiguration {
                    billing_periods: version
                        .billing_periods
                        .iter()
                        .map(|freq| billing_period_to_server(freq) as i32)
                        .collect(),
                    billing_cycles: Some(match version.billing_cycles {
                        Some(count) => {
                            billing_config_grpc::BillingCycles::Fixed(billing_config_grpc::Fixed {
                                count: count as u32,
                            })
                        }
                        None => billing_config_grpc::BillingCycles::Forever(
                            billing_config_grpc::Forever {},
                        ),
                    }),
                    net_terms: version.net_terms as u32,
                    service_period_start: Some(match version.period_start_day {
                        Some(day) => billing_config_grpc::ServicePeriodStart::DayOfMonth(
                            billing_config_grpc::DayOfMonth {
                                day_of_month: day as u32,
                            },
                        ),
                        None => billing_config_grpc::ServicePeriodStart::SubscriptionAnniversary(
                            billing_config_grpc::SubscriptionAnniversary {},
                        ),
                    }),
                },
            )
        }

        pub fn db_to_server(version: DbPlanVersion) -> PlanVersion {
            PlanVersion {
                id: version.id.to_string(),
                is_draft: version.is_draft_version,
                version: version.version as u32,
                trial_config: map_trial_config(&version),
                billing_config: map_billing_config(&version),
                currency: version.currency,
            }
        }

        pub fn list_db_to_server(version: DbListPlanVersion) -> ListPlanVersion {
            ListPlanVersion {
                id: version.id.to_string(),
                is_draft: version.is_draft_version,
                version: version.version as u32,
                currency: version.currency,
            }
        }
    }

    // pub mod parameters {
    //     use meteroid_grpc::meteroid::api::plans::v1 as grpc;
    //
    //     use crate::api::pricecomponents::ext::PlanParameter;
    //
    //     pub fn to_grpc(param: PlanParameter) -> grpc::PlanParameter {
    //         let param = match param {
    //             PlanParameter::BillingPeriodTerm => grpc::plan_parameter::Param::BillingPeriodTerm(
    //                 grpc::plan_parameter::BillingPeriodTerm {},
    //             ),
    //             PlanParameter::CapacityThresholdValue {
    //                 capacity_values,
    //                 component_id,
    //             } => grpc::plan_parameter::Param::CapacityThresholdValue(
    //                 grpc::plan_parameter::CapacityThresholdValue {
    //                     component_id,
    //                     capacity_values,
    //                 },
    //             ),
    //             PlanParameter::CommittedSlot { component_id } => {
    //                 grpc::plan_parameter::Param::CommittedSlot(
    //                     grpc::plan_parameter::CommittedSlot { component_id },
    //                 )
    //             }
    //         };
    //
    //         grpc::PlanParameter { param: Some(param) }
    //     }
    // }
}
