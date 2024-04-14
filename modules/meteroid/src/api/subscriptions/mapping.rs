pub mod subscriptions {
    use chrono::{NaiveDate, NaiveDateTime};


    use meteroid_store::domain;

    use crate::services::subscription::ext::DbSubscriptionExt;
    use tonic::Status;
    use uuid::Uuid;

    use crate::api::shared::conversions::*;

    use meteroid_grpc::meteroid::api::subscriptions::v1_2 as proto2;


    pub(crate) fn domain_to_proto(
        s: meteroid_store::domain::Subscription,
    ) -> Result<proto2::Subscription, Status> {
        let status = *(&s.status_proto()?) as i32;

        Ok(proto2::Subscription {
            id: s.id.as_proto(),
            tenant_id: s.tenant_id.as_proto(),
            customer_id: s.customer_id.as_proto(),
            plan_id: s.plan_id.as_proto(),
            plan_name: s.plan_name,
            plan_version_id: s.plan_version_id.as_proto(),
            net_terms: s.net_terms,
            invoice_memo: s.invoice_memo,
            invoice_threshold: s.invoice_threshold.as_proto(),
            currency: s.currency,
            version: s.version,
            created_at: s.created_at.as_proto(),
            billing_end_date: s.billing_end_date.as_proto(),
            billing_start_date: s.billing_start_date.as_proto(),
            customer_name: s.customer_name,
            customer_alias: s.customer_alias,
            canceled_at: s.canceled_at.as_proto(),
            cancellation_reason: s.cancellation_reason,
            billing_day: s.billing_day as i32,
            trial_start_date: s.trial_start_date.as_proto(),
            created_by: s.created_by.as_proto(),
            activated_at: s.activated_at.as_proto(),
            mrr_cents: s.mrr_cents,
            status,
        })
    }

    pub(crate) fn create_proto_to_domain(
        param: proto2::CreateSubscription,
        tenant_id: &Uuid,
        actor: &Uuid,
    ) -> Result<domain::CreateSubscription, Status> {
        let subscription_new = meteroid_store::domain::SubscriptionNew {
            customer_id: Uuid::from_proto(param.customer_id)?,
            billing_day: param.billing_day as i16,
            tenant_id: tenant_id.clone(),
            currency: param.currency,
            trial_start_date: NaiveDate::from_proto_opt(param.trial_start_date)?,
            billing_start_date: NaiveDate::from_proto(param.billing_start_date)?,
            billing_end_date: NaiveDate::from_proto_opt(param.billing_end_date)?,
            plan_version_id: Uuid::from_proto(param.plan_version_id)?,
            created_by: actor.clone(),
            net_terms: param.net_terms,
            invoice_memo: param.invoice_memo,
            invoice_threshold: rust_decimal::Decimal::from_proto_opt(param.invoice_threshold)?,
            activated_at: NaiveDateTime::from_proto_opt(param.activated_at)?,
        };

        let res = meteroid_store::domain::CreateSubscription {
            subscription: subscription_new,
            price_components: param
                .components
                .map(|a| super::price_components::create_subscription_components_from_grpc(a))
                .transpose()?,
        };

        Ok(res)
    }

    pub(crate) fn created_domain_to_proto(
        sub: domain::CreatedSubscription,
    ) -> Result<proto2::CreatedSubscription, Status> {
        Ok(proto2::CreatedSubscription {
            id: sub.id.as_proto(),
            customer_id: sub.customer_id.as_proto(),
            billing_day: sub.billing_day as i32,
            tenant_id: sub.tenant_id.as_proto(),
            currency: sub.currency,
            trial_start_date: sub.trial_start_date.as_proto(),
            billing_start_date: sub.billing_start_date.as_proto(),
            billing_end_date: sub.billing_end_date.as_proto(),
            plan_version_id: sub.plan_version_id.as_proto(),
            created_at: sub.created_at.as_proto(),
            created_by: sub.created_by.as_proto(),
            net_terms: sub.net_terms,
            invoice_memo: sub.invoice_memo,
            invoice_threshold: sub.invoice_threshold.as_proto(),
            activated_at: sub.activated_at.as_proto(),
            mrr_cents: sub.mrr_cents,
        })
    }

    pub(crate) fn details_domain_to_proto(
        sub: domain::SubscriptionDetails,
    ) -> Result<proto2::SubscriptionDetails, Status> {
        let status = *(&sub.status_proto()?) as i32;
        Ok(proto2::SubscriptionDetails {
            subscription: Some(proto2::Subscription {
                id: sub.id.as_proto(),
                tenant_id: sub.tenant_id.as_proto(),
                customer_id: sub.customer_id.as_proto(),
                plan_id: sub.plan_id.as_proto(),
                plan_name: sub.plan_name,
                plan_version_id: sub.plan_version_id.as_proto(),
                net_terms: sub.net_terms,
                invoice_memo: sub.invoice_memo,
                invoice_threshold: sub.invoice_threshold.as_proto(),
                currency: sub.currency,
                version: sub.version,
                created_at: sub.created_at.as_proto(),
                billing_end_date: sub.billing_end_date.as_proto(),
                billing_start_date: sub.billing_start_date.as_proto(),
                customer_name: sub.customer_name,
                customer_alias: sub.customer_external_id,
                canceled_at: sub.canceled_at.as_proto(),
                cancellation_reason: sub.cancellation_reason,
                billing_day: sub.billing_day as i32,
                trial_start_date: sub.trial_start_date.as_proto(),
                created_by: sub.created_by.as_proto(),
                activated_at: sub.activated_at.as_proto(),
                mrr_cents: sub.mrr_cents,
                status: status,
            }),
            schedules: vec![], // TODO
            price_components: sub
                .price_components
                .iter()
                .map(|c| super::price_components::subscription_component_to_grpc(c))
                .collect(),
            metrics: sub
                .metrics
                .into_iter()
                .map(|m| proto2::BillableMetric {
                    id: m.id.as_proto(),
                    name: m.name,
                    alias: m.code,
                })
                .collect(),
        })
    }
}

mod price_components {
    // In meteroid/src/subscription/mod.rs

    use crate::api::shared::conversions::*;
    use meteroid_grpc::meteroid::api::shared::v1 as api_shared;
    use meteroid_grpc::meteroid::api::subscriptions::v1_2 as api;
    use meteroid_store::domain;

    use tonic::{Code, Result, Status};
    use uuid::Uuid;

    pub fn create_subscription_components_from_grpc(
        data: api::CreateSubscriptionComponents,
    ) -> Result<domain::CreateSubscriptionComponents> {
        let parameterized_components = data
            .parameterized_components
            .into_iter()
            .map(|c| {
                let component_id = Uuid::from_proto_ref(&c.component_id)?;
                let parameters = c
                    .parameters
                    .as_ref()
                    .map(|p| {
                        let billing_period = p
                            .billing_period
                            .map(|p| api_shared::BillingPeriod::try_from(p))
                            .transpose()
                            .map_err(|_| {
                                Status::invalid_argument("Invalid billing period".to_string())
                            })?
                            .map(map_billing_period_from_grpc);

                        Ok::<domain::ComponentParameters, Status>(domain::ComponentParameters {
                            initial_slot_count: p.initial_slot_count,
                            billing_period,
                            committed_capacity: p.committed_capacity,
                        })
                    })
                    .ok_or_else(|| Status::invalid_argument("Missing parameters".to_string()))??;

                Ok::<_, Status>(domain::ComponentParameterization {
                    component_id,
                    parameters,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        let overridden_components = data
            .overridden_components
            .into_iter()
            .map(|c| {
                let component_id = Uuid::from_proto_ref(&c.component_id)?;
                let component = c
                    .component
                    .ok_or_else(|| {
                        Status::invalid_argument("Missing overridden component data".to_string())
                    })
                    .and_then(subscription_component_new_internal_from_grpc)?;

                Ok::<_, Status>(domain::ComponentOverride {
                    component_id,
                    component,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        let extra_components = data
            .extra_components
            .into_iter()
            .map(|c| {
                let component = c
                    .component
                    .ok_or_else(|| {
                        Status::invalid_argument("Missing extra component data".to_string())
                    })
                    .and_then(subscription_component_new_internal_from_grpc)?;

                Ok::<_, Status>(domain::ExtraComponent { component })
            })
            .collect::<Result<Vec<_>, _>>()?;

        let remove_components = data
            .remove_components
            .iter()
            .map(|remove_component_id| {
                crate::api::utils::parse_uuid(&remove_component_id, "remove_component_id")
            })
            .collect::<Result<Vec<Uuid>>>()?;

        Ok(domain::CreateSubscriptionComponents {
            parameterized_components,
            overridden_components,
            extra_components,
            remove_components,
        })
    }

    fn subscription_fee_billing_period_to_grpc(
        period: domain::enums::SubscriptionFeeBillingPeriod,
    ) -> api::SubscriptionFeeBillingPeriod {
        match period {
            domain::enums::SubscriptionFeeBillingPeriod::OneTime => {
                api::SubscriptionFeeBillingPeriod::OneTime
            }
            domain::enums::SubscriptionFeeBillingPeriod::Monthly => {
                api::SubscriptionFeeBillingPeriod::Monthly
            }
            domain::enums::SubscriptionFeeBillingPeriod::Quarterly => {
                api::SubscriptionFeeBillingPeriod::Quarterly
            }
            domain::enums::SubscriptionFeeBillingPeriod::Annual => {
                api::SubscriptionFeeBillingPeriod::Yearly
            }
        }
    }


    fn subscription_component_new_internal_from_grpc(
        component: api::SubscriptionComponentNewInternal,
    ) -> Result<domain::SubscriptionComponentNewInternal> {
        Ok(domain::SubscriptionComponentNewInternal {
            period: subscription_fee_billing_period_from_grpc(component.period())?,
            price_component_id: component
                .price_component_id
                .map(|id| Uuid::from_proto_ref(&id))
                .transpose()?,
            product_item_id: component
                .product_item_id
                .map(|id| Uuid::from_proto_ref(&id))
                .transpose()?,
            name: component.name.clone(),
            fee: subscription_fee_from_grpc(&component.fee)?,
            is_override: false,
        })
    }


    pub fn subscription_component_to_grpc(
        component: &domain::SubscriptionComponent,
    ) -> api::SubscriptionComponent {
        api::SubscriptionComponent {
            id: component.id.to_string(),
            price_component_id: component.price_component_id.map(|id| id.to_string()),
            product_item_id: component.product_item_id.map(|id| id.to_string()),
            subscription_id: component.subscription_id.to_string(),
            name: component.name.clone(),
            period: subscription_fee_billing_period_to_grpc(component.period.clone()).into(),
            fee: Some(subscription_fee_to_grpc(&component.fee)),
            is_override: false, // TODO: Update this based on your logic
        }
    }

    pub fn subscription_fee_to_grpc(fee: &domain::SubscriptionFee) -> api::SubscriptionFee {
        match fee {
            domain::SubscriptionFee::Rate { rate } => api::SubscriptionFee {
                fee: Some(api::subscription_fee::Fee::Rate(api::RatePricing {
                    rate: rate.to_string(),
                })),
            },
            domain::SubscriptionFee::OneTime { rate, quantity } => api::SubscriptionFee {
                fee: Some(api::subscription_fee::Fee::OneTime(api::OneTimePricing {
                    rate: rate.to_string(),
                    quantity: *quantity,
                    total: (rate * rust_decimal::Decimal::from(*quantity)).to_string(),
                })),
            },
            domain::SubscriptionFee::Recurring {
                rate,
                quantity,
                billing_type,
            } => api::SubscriptionFee {
                fee: Some(api::subscription_fee::Fee::Recurring(
                    api::ExtraRecurringPricing {
                        rate: rate.to_string(),
                        quantity: *quantity,
                        total: (rate * rust_decimal::Decimal::from(*quantity)).to_string(),
                        billing_type: billing_type_to_grpc(billing_type.clone()).into(),
                    },
                )),
            },
            domain::SubscriptionFee::Capacity {
                rate,
                included,
                overage_rate,
                metric_id,
            } => api::SubscriptionFee {
                fee: Some(api::subscription_fee::Fee::Capacity(api::CapacityPricing {
                    rate: rate.to_string(),
                    included: *included,
                    overage_rate: overage_rate.to_string(),
                    metric_id: metric_id.to_string(),
                })),
            },
            domain::SubscriptionFee::Slot {
                unit,
                unit_rate,
                min_slots,
                max_slots,
                initial_slots,
            } => api::SubscriptionFee {
                fee: Some(api::subscription_fee::Fee::Slot(api::SlotPricing {
                    unit: unit.clone(),
                    unit_rate: unit_rate.to_string(),
                    min_slots: *min_slots,
                    max_slots: *max_slots,
                    initial_slots: *initial_slots,
                })),
            },
            domain::SubscriptionFee::Usage { metric_id, model } => api::SubscriptionFee {
                fee: Some(api::subscription_fee::Fee::Usage(
                    usage_pricing_model_to_grpc(metric_id, model),
                )),
            },
        }
    }

    pub fn usage_pricing_model_to_grpc(
        metric_id: &Uuid,
        model: &domain::UsagePricingModel,
    ) -> api::UsagePricing {
        match model {
            domain::UsagePricingModel::PerUnit { rate } => api::UsagePricing {
                metric_id: metric_id.as_proto(),
                model: Some(api::usage_pricing::Model::PerUnit(rate.as_proto())),
            },
            domain::UsagePricingModel::Tiered { tiers, .. } => api::UsagePricing {
                metric_id: metric_id.as_proto(),
                model: Some(api::usage_pricing::Model::Tiered(api::TierPricing {
                    tiers: tiers.iter().map(tier_row_to_grpc).collect(),
                })),
            },
            domain::UsagePricingModel::Volume { tiers, .. } => api::UsagePricing {
                metric_id: metric_id.as_proto(),
                model: Some(api::usage_pricing::Model::Volume(api::TierPricing {
                    tiers: tiers.iter().map(tier_row_to_grpc).collect(),
                })),
            },
            domain::UsagePricingModel::Package { block_size, rate } => api::UsagePricing {
                metric_id: metric_id.as_proto(),
                model: Some(api::usage_pricing::Model::Package(api::PackagePricing {
                    block_size: *block_size,
                    rate: rate.as_proto(),
                })),
            },
        }
    }

    pub fn tier_row_to_grpc(tier: &domain::TierRow) -> api::TierRow {
        api::TierRow {
            first_unit: tier.first_unit,
            rate: tier.rate.as_proto(),
            flat_fee: tier.flat_fee.map(|fee| fee.as_proto()),
            flat_cap: tier.flat_cap.map(|cap| cap.as_proto()),
        }
    }


    pub fn subscription_fee_from_grpc(
        grpc_fee: &Option<api::SubscriptionFee>,
    ) -> Result<domain::SubscriptionFee, Status> {
        match grpc_fee.as_ref().and_then(|fee| fee.fee.as_ref()) {
            Some(api::subscription_fee::Fee::Rate(rate)) => {
                let rate = rust_decimal::Decimal::from_proto_ref(&rate.rate)?;
                Ok(domain::SubscriptionFee::Rate { rate })
            }
            Some(api::subscription_fee::Fee::OneTime(one_time)) => {
                let rate = rust_decimal::Decimal::from_proto_ref(&one_time.rate)?;
                Ok(domain::SubscriptionFee::OneTime {
                    rate,
                    quantity: one_time.quantity,
                })
            }
            Some(api::subscription_fee::Fee::Recurring(recurring)) => {
                let rate = rust_decimal::Decimal::from_proto_ref(&recurring.rate)?;
                let billing_type = billing_type_from_grpc(recurring.billing_type())?;
                Ok(domain::SubscriptionFee::Recurring {
                    rate,
                    quantity: recurring.quantity,
                    billing_type,
                })
            }
            Some(api::subscription_fee::Fee::Capacity(capacity)) => {
                let rate = rust_decimal::Decimal::from_proto_ref(&capacity.rate)?;
                let overage_rate = rust_decimal::Decimal::from_proto_ref(&capacity.overage_rate)?;
                let metric_id = Uuid::from_proto_ref(&capacity.metric_id)?;
                Ok(domain::SubscriptionFee::Capacity {
                    rate,
                    included: capacity.included,
                    overage_rate,
                    metric_id,
                })
            }
            Some(api::subscription_fee::Fee::Slot(slot)) => {
                let unit_rate = rust_decimal::Decimal::from_proto_ref(&slot.unit_rate)?;
                Ok(domain::SubscriptionFee::Slot {
                    unit: slot.unit.clone(),
                    unit_rate,
                    min_slots: slot.min_slots,
                    max_slots: slot.max_slots,
                    initial_slots: slot.initial_slots,
                })
            }
            Some(api::subscription_fee::Fee::Usage(usage)) => {
                let metric_id = Uuid::from_proto_ref(&usage.metric_id)?;
                let model = usage_pricing_model_from_grpc(&usage)?;
                Ok(domain::SubscriptionFee::Usage { metric_id, model })
            }
            None => Err(Status::new(
                Code::InvalidArgument,
                "Missing subscription fee",
            )),
        }
    }

    pub fn usage_pricing_model_from_grpc(
        usage: &api::UsagePricing,
    ) -> Result<domain::UsagePricingModel, Status> {
        match usage.model.as_ref() {
            Some(api::usage_pricing::Model::PerUnit(per_unit)) => {
                let per_unit = rust_decimal::Decimal::from_proto_ref(per_unit)?;
                Ok(domain::UsagePricingModel::PerUnit { rate: per_unit })
            }
            Some(api::usage_pricing::Model::Tiered(tiered)) => {
                let tiers = tiered
                    .tiers
                    .iter()
                    .map(tier_row_from_grpc)
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(domain::UsagePricingModel::Tiered {
                    tiers,
                    block_size: None, // TODO
                })
            }
            Some(api::usage_pricing::Model::Volume(volume)) => {
                let tiers = volume
                    .tiers
                    .iter()
                    .map(tier_row_from_grpc)
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(domain::UsagePricingModel::Volume {
                    tiers,
                    block_size: None, // TODO
                })
            }
            Some(api::usage_pricing::Model::Package(package)) => {
                let block_size = package.block_size;
                let rate = rust_decimal::Decimal::from_proto_ref(&package.rate)?;
                Ok(domain::UsagePricingModel::Package { block_size, rate })
            }
            None => Err(Status::new(
                Code::InvalidArgument,
                "Missing usage pricing model",
            )),
        }
    }

    pub fn tier_row_from_grpc(tier: &api::TierRow) -> Result<domain::TierRow, Status> {
        let rate = rust_decimal::Decimal::from_proto_ref(&tier.rate)?;
        let flat_fee = tier
            .flat_fee
            .as_ref()
            .map(rust_decimal::Decimal::from_proto_ref)
            .transpose()?;
        let flat_cap = tier
            .flat_cap
            .as_ref()
            .map(rust_decimal::Decimal::from_proto_ref)
            .transpose()?;
        Ok(domain::TierRow {
            first_unit: tier.first_unit,
            rate,
            flat_fee,
            flat_cap,
        })
    }

    pub fn subscription_fee_billing_period_from_grpc(
        period: api::SubscriptionFeeBillingPeriod,
    ) -> Result<domain::enums::SubscriptionFeeBillingPeriod, Status> {
        match period {
            api::SubscriptionFeeBillingPeriod::OneTime => {
                Ok(domain::enums::SubscriptionFeeBillingPeriod::OneTime)
            }
            api::SubscriptionFeeBillingPeriod::Monthly => {
                Ok(domain::enums::SubscriptionFeeBillingPeriod::Monthly)
            }
            api::SubscriptionFeeBillingPeriod::Quarterly => {
                Ok(domain::enums::SubscriptionFeeBillingPeriod::Quarterly)
            }
            api::SubscriptionFeeBillingPeriod::Yearly => {
                Ok(domain::enums::SubscriptionFeeBillingPeriod::Annual)
            } // _ => Err(Status::new(Code::InvalidArgument, "Invalid billing period")),
        }
    }

    pub fn billing_type_from_grpc(
        billing_type: api::BillingType,
    ) -> Result<domain::enums::BillingType, Status> {
        match billing_type {
            api::BillingType::Arrear => Ok(domain::enums::BillingType::Arrears),
            api::BillingType::Advance => Ok(domain::enums::BillingType::Advance),
            // _ => Err(Status::new(Code::InvalidArgument, "Invalid billing type")),
        }
    }

    pub fn billing_type_to_grpc(billing_type: domain::enums::BillingType) -> api::BillingType {
        match billing_type {
            domain::enums::BillingType::Arrears => api::BillingType::Arrear,
            domain::enums::BillingType::Advance => api::BillingType::Advance,
        }
    }

    pub fn map_billing_period_from_grpc(
        period: api_shared::BillingPeriod,
    ) -> domain::enums::BillingPeriodEnum {
        match period {
            api_shared::BillingPeriod::Monthly => domain::enums::BillingPeriodEnum::Monthly,
            api_shared::BillingPeriod::Quarterly => domain::enums::BillingPeriodEnum::Quarterly,
            api_shared::BillingPeriod::Annual => domain::enums::BillingPeriodEnum::Annual,
        }
    }
}
