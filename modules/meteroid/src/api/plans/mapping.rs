pub mod plans {
    use meteroid_grpc::meteroid::api::plans::v1::{ListPlan, Plan};
    use meteroid_repository::plans::{ListPlan as DbListPlans, Plan as DbPlan};

    fn status_db_to_server(
        e: meteroid_repository::PlanStatusEnum,
    ) -> meteroid_grpc::meteroid::api::plans::v1::PlanStatus {
        match e {
            meteroid_repository::PlanStatusEnum::ACTIVE => {
                meteroid_grpc::meteroid::api::plans::v1::PlanStatus::Active
            }
            meteroid_repository::PlanStatusEnum::ARCHIVED => {
                meteroid_grpc::meteroid::api::plans::v1::PlanStatus::Archived
            }
            meteroid_repository::PlanStatusEnum::DRAFT => {
                meteroid_grpc::meteroid::api::plans::v1::PlanStatus::Draft
            }
            meteroid_repository::PlanStatusEnum::INACTIVE => {
                meteroid_grpc::meteroid::api::plans::v1::PlanStatus::Inactive
            }
        }
    }

    fn type_db_to_server(
        e: meteroid_repository::PlanTypeEnum,
    ) -> meteroid_grpc::meteroid::api::plans::v1::PlanType {
        match e {
            meteroid_repository::PlanTypeEnum::CUSTOM => {
                meteroid_grpc::meteroid::api::plans::v1::PlanType::Custom
            }
            meteroid_repository::PlanTypeEnum::FREE => {
                meteroid_grpc::meteroid::api::plans::v1::PlanType::Free
            }
            meteroid_repository::PlanTypeEnum::STANDARD => {
                meteroid_grpc::meteroid::api::plans::v1::PlanType::Standard
            }
        }
    }

    pub fn db_to_server(plan: DbPlan) -> Plan {
        Plan {
            id: plan.id.to_string(),
            name: plan.name,
            external_id: plan.external_id,
            description: plan.description,
            plan_type: type_db_to_server(plan.plan_type).into(),
            plan_status: status_db_to_server(plan.status).into(),
        }
    }

    pub fn list_db_to_server(plan: DbListPlans) -> ListPlan {
        ListPlan {
            id: plan.id.to_string(),
            name: plan.name,
            external_id: plan.external_id,
            description: plan.description,
            plan_type: type_db_to_server(plan.plan_type).into(),
            plan_status: status_db_to_server(plan.status).into(),
            product_family_id: plan.product_family_id.to_string(),
            product_family_name: plan.product_family_name,
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

    pub mod parameters {
        use meteroid_grpc::meteroid::api::plans::v1 as grpc;

        use crate::api::pricecomponents::ext::PlanParameter;

        pub fn to_grpc(param: PlanParameter) -> grpc::PlanParameter {
            let param = match param {
                PlanParameter::BillingPeriodTerm => grpc::plan_parameter::Param::BillingPeriodTerm(
                    grpc::plan_parameter::BillingPeriodTerm {},
                ),
                PlanParameter::CapacityThresholdValue {
                    capacity_values,
                    component_id,
                } => grpc::plan_parameter::Param::CapacityThresholdValue(
                    grpc::plan_parameter::CapacityThresholdValue {
                        component_id,
                        capacity_values,
                    },
                ),
                PlanParameter::CommittedSlot { component_id } => {
                    grpc::plan_parameter::Param::CommittedSlot(
                        grpc::plan_parameter::CommittedSlot { component_id },
                    )
                }
            };

            grpc::PlanParameter { param: Some(param) }
        }
    }
}
