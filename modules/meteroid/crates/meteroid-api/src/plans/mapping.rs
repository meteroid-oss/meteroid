pub mod plans {
    use meteroid_app::conversions::ProtoConv;
    use meteroid_grpc::meteroid::api::plans::v1::minimum_commitment::Scope as MinScope;
    use meteroid_grpc::meteroid::api::plans::v1::minimum_commitment::scope::{
        Kind as MinScopeKind, Products as MinScopeProducts,
    };
    use meteroid_grpc::meteroid::api::plans::v1::plan_overview::ActiveVersionInfo;
    use meteroid_grpc::meteroid::api::plans::v1::{
        CreditCap, CreditExhaustedRule, MinimumCommitment, Plan, PlanBillingConfiguration,
        PlanStatus, PlanType, PlanVersion, PlanWithVersion, TrialConfig, credit_exhausted_rule,
    };
    use meteroid_grpc::meteroid::api::plans::v1::{
        ListPlanVersion, PlanOverview, plan_billing_configuration as billing_config_grpc,
    };
    use meteroid_store::domain;
    use meteroid_store::domain::enums::{PlanStatusEnum, PlanTypeEnum};
    use meteroid_store::domain::plan_minimums::{MinimumScope, PlanMinimum};

    pub fn minimum_commitment_to_proto(min: &PlanMinimum) -> MinimumCommitment {
        build_minimum_commitment_proto(min.amount_subunit, &min.scope)
    }

    pub fn subscription_minimum_to_proto(
        min: &meteroid_store::domain::SubscriptionMinimum,
    ) -> MinimumCommitment {
        build_minimum_commitment_proto(min.amount_subunit, &min.scope)
    }

    pub fn quote_minimum_to_proto(min: &meteroid_store::domain::QuoteMinimum) -> MinimumCommitment {
        let mut proto = build_minimum_commitment_proto(min.amount_subunit, &min.scope);
        // Surface unresolved new-product-extra names so a draft quote round-trips them (display
        // and re-editing). They resolve to product ids only at conversion.
        if !min.pending_extra_product_names.is_empty()
            && let Some(scope) = proto.scope.as_mut()
            && let Some(MinScopeKind::Products(p)) = scope.kind.as_mut()
        {
            p.pending_extra_product_names = min.pending_extra_product_names.clone();
        }
        proto
    }

    fn build_minimum_commitment_proto(
        amount_subunit: i64,
        scope: &MinimumScope,
    ) -> MinimumCommitment {
        let kind = match scope {
            MinimumScope::AllComponents => MinScopeKind::AllComponents(
                meteroid_grpc::meteroid::api::plans::v1::minimum_commitment::scope::AllComponents {},
            ),
            MinimumScope::Products(ids) => MinScopeKind::Products(MinScopeProducts {
                product_ids: ids.iter().map(|id| id.as_proto()).collect(),
                // Read/response path: resolved ids only, no unresolved create-time names.
                pending_extra_product_names: vec![],
            }),
        };
        MinimumCommitment {
            amount_subunit,
            scope: Some(MinScope { kind: Some(kind) }),
        }
    }

    pub fn minimum_commitment_from_proto(
        proto: MinimumCommitment,
    ) -> Result<(i64, MinimumScope), String> {
        if proto.amount_subunit <= 0 {
            return Err(format!(
                "amount_subunit must be > 0 (got {})",
                proto.amount_subunit
            ));
        }
        let scope_kind = proto
            .scope
            .and_then(|s| s.kind)
            .ok_or_else(|| "scope is required".to_string())?;
        let scope = match scope_kind {
            MinScopeKind::AllComponents(_) => MinimumScope::AllComponents,
            MinScopeKind::Products(p) => {
                let ids = p
                    .product_ids
                    .iter()
                    .map(common_domain::ids::ProductId::from_proto)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| format!("Invalid product id: {}", e))?;
                MinimumScope::Products(ids)
            }
        };
        Ok((proto.amount_subunit, scope))
    }

    /// Like [`minimum_commitment_from_proto`], but for the subscription create path: it preserves
    /// names of inline new-product extras (`pending_extra_product_names`) so they can be resolved
    /// to freshly-minted product ids inside the create transaction.
    pub fn create_minimum_from_proto(
        proto: MinimumCommitment,
    ) -> Result<(i64, domain::plan_minimums::CreateMinimumScope), String> {
        use domain::plan_minimums::CreateMinimumScope;
        if proto.amount_subunit <= 0 {
            return Err(format!(
                "amount_subunit must be > 0 (got {})",
                proto.amount_subunit
            ));
        }
        let scope_kind = proto
            .scope
            .and_then(|s| s.kind)
            .ok_or_else(|| "scope is required".to_string())?;
        let scope = match scope_kind {
            MinScopeKind::AllComponents(_) => CreateMinimumScope::AllComponents,
            MinScopeKind::Products(p) => {
                let product_ids = p
                    .product_ids
                    .iter()
                    .map(common_domain::ids::ProductId::from_proto)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| format!("Invalid product id: {}", e))?;
                CreateMinimumScope::Products {
                    product_ids,
                    extra_product_names: p.pending_extra_product_names,
                }
            }
        };
        Ok((proto.amount_subunit, scope))
    }

    pub struct PlanWithVersionWrapper(pub PlanWithVersion);

    pub struct PlanVersionWrapper(pub PlanVersion);

    pub struct PlanTypeWrapper(pub PlanType);

    pub struct PlanStatusWrapper(pub PlanStatus);

    pub struct ListPlanVersionWrapper(pub ListPlanVersion);

    pub struct PlanOverviewWrapper(pub PlanOverview);

    impl From<domain::PlanVersion> for ListPlanVersionWrapper {
        fn from(value: domain::PlanVersion) -> Self {
            Self(ListPlanVersion {
                id: value.id.as_proto(),
                is_draft: value.is_draft_version,
                version: value.version as u32,
                currency: value.currency,
                created_at: value.created_at.as_proto(),
            })
        }
    }

    impl From<domain::PlanVersion> for PlanVersionWrapper {
        fn from(value: domain::PlanVersion) -> Self {
            fn trial_config(version: &domain::PlanVersion) -> Option<TrialConfig> {
                match version.trial_duration_days {
                    Some(days) if days > 0 => Some(TrialConfig {
                        trialing_plan_id: version.trialing_plan_id.map(|x| x.as_proto()),
                        duration_days: days as u32,
                        trial_is_free: version.trial_is_free,
                    }),
                    _ => None,
                }
            }

            fn billing_config(version: &domain::PlanVersion) -> Option<PlanBillingConfiguration> {
                Some(PlanBillingConfiguration {
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
            let minimum_commitment = value.minimum.as_ref().map(minimum_commitment_to_proto);
            Self(PlanVersion {
                id: value.id.as_proto(),
                version: value.version as u32,
                is_draft: value.is_draft_version,
                trial_config: trial_config(&value),
                billing_config: billing_config(&value),
                currency: value.currency,
                net_terms: value.net_terms,
                period_start_day: value.period_start_day.map(i32::from),
                uses_product_pricing: value.uses_product_pricing,
                minimum_commitment,
                tax_inclusive: value.tax_inclusive,
                created_at: value.created_at.as_proto(),
                credit_exhausted_rule: value
                    .credit_exhausted_rule
                    .as_ref()
                    .map(credit_exhausted_rule_to_proto),
            })
        }
    }

    pub fn credit_exhausted_rule_to_proto(
        rule: &domain::credit_exhausted::CreditExhaustedRule,
    ) -> CreditExhaustedRule {
        use domain::credit_exhausted::CreditExhaustedRule as Rule;
        let (kind, caps) = match rule {
            Rule::Invoice => (credit_exhausted_rule::Kind::Invoice, Vec::new()),
            Rule::Pause => (credit_exhausted_rule::Kind::Pause, Vec::new()),
            Rule::InvoiceUpTo { caps } => (
                credit_exhausted_rule::Kind::InvoiceUpTo,
                caps.iter()
                    .map(|(denomination, amount)| CreditCap {
                        denomination: denomination.clone(),
                        amount: *amount,
                    })
                    .collect(),
            ),
        };
        CreditExhaustedRule {
            kind: kind.into(),
            caps,
        }
    }

    pub fn credit_exhausted_rule_from_proto(
        rule: CreditExhaustedRule,
    ) -> Result<domain::credit_exhausted::CreditExhaustedRule, tonic::Status> {
        use domain::credit_exhausted::CreditExhaustedRule as Rule;
        let rule = match rule.kind() {
            credit_exhausted_rule::Kind::Invoice => Rule::Invoice,
            credit_exhausted_rule::Kind::Pause => Rule::Pause,
            credit_exhausted_rule::Kind::InvoiceUpTo => Rule::InvoiceUpTo {
                caps: rule
                    .caps
                    .into_iter()
                    .map(|cap| (cap.denomination, cap.amount))
                    .collect(),
            },
        };
        rule.validated()
            .map_err(|error| tonic::Status::invalid_argument(error.to_string()))
    }

    impl From<domain::FullPlan> for PlanWithVersionWrapper {
        fn from(value: domain::FullPlan) -> Self {
            Self(PlanWithVersion {
                plan: Some(Plan {
                    id: value.plan.id.as_proto(),
                    local_id: value.plan.id.as_proto(),
                    name: value.plan.name,
                    description: value.plan.description,
                    plan_type: PlanTypeWrapper::from(value.plan.plan_type).0 as i32,
                    plan_status: PlanStatusWrapper::from(value.plan.status).0 as i32,
                    active_version_id: value.plan.active_version_id.map(|x| x.as_proto()),
                    draft_version_id: value.plan.draft_version_id.map(|x| x.as_proto()),
                    self_service_rank: value.plan.self_service_rank,
                    issues_license: value.plan.issues_license,
                }),
                version: Some(PlanVersionWrapper::from(value.version).0),
            })
        }
    }

    impl From<domain::PlanWithVersion> for PlanWithVersionWrapper {
        fn from(value: domain::PlanWithVersion) -> Self {
            Self(PlanWithVersion {
                plan: Some(Plan {
                    id: value.plan.id.as_proto(),
                    local_id: value.plan.id.as_proto(), //todo remove me
                    name: value.plan.name,
                    description: value.plan.description,
                    plan_type: PlanTypeWrapper::from(value.plan.plan_type).0 as i32,
                    plan_status: PlanStatusWrapper::from(value.plan.status).0 as i32,
                    active_version_id: value.plan.active_version_id.map(|x| x.as_proto()),
                    draft_version_id: value.plan.draft_version_id.map(|x| x.as_proto()),
                    self_service_rank: value.plan.self_service_rank,
                    issues_license: value.plan.issues_license,
                }),
                version: value.version.map(|v| PlanVersionWrapper::from(v).0),
            })
        }
    }

    impl From<PlanTypeWrapper> for PlanTypeEnum {
        fn from(val: PlanTypeWrapper) -> Self {
            match val.0 {
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

    impl From<PlanStatusWrapper> for PlanStatusEnum {
        fn from(val: PlanStatusWrapper) -> Self {
            match val.0 {
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

    impl From<domain::PlanOverview> for PlanOverviewWrapper {
        fn from(value: domain::PlanOverview) -> Self {
            Self(PlanOverview {
                id: value.id.as_proto(),
                name: value.name,
                local_id: value.id.as_proto(), //todo remove me
                description: value.description,
                plan_type: PlanTypeWrapper::from(value.plan_type).0 as i32,
                plan_status: PlanStatusWrapper::from(value.status).0 as i32,
                product_family_name: value.product_family_name,
                product_family_local_id: value.product_family_id.as_proto(), // todo rename product_family_local_id
                created_at: value.created_at.as_proto(),
                has_draft_version: value.has_draft_version,
                active_version: value.active_version.map(|v| ActiveVersionInfo {
                    id: v.id.as_proto(),
                    version: v.version as u32,
                    trial_duration_days: v.trial_duration_days.map(|x| x as u32),
                }),
                subscription_count: value.subscription_count.map_or(0, |x| x as u32),
                self_service_rank: value.self_service_rank,
                issues_license: value.issues_license,
            })
        }
    }

    // pub mod parameters {
    //     use meteroid_grpc::meteroid::api::plans::v1 as grpc;
    //
    //     use crate::pricecomponents::ext::PlanParameter;
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
