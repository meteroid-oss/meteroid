pub mod subscriptions {
    use chrono::NaiveDate;

    use meteroid_store::domain;

    use crate::services::subscription::ext::DbSubscriptionExt;
    use tonic::Status;
    use uuid::Uuid;

    use crate::api::shared::conversions::*;

    use meteroid_grpc::meteroid::api::subscriptions::v1 as proto2;

    pub(crate) fn domain_to_proto(s: domain::Subscription) -> Result<proto2::Subscription, Status> {
        let status = s.status_proto()? as i32;

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
            billing_day: s.billing_day as u32,
            trial_start_date: s.trial_start_date.as_proto(),
            created_by: s.created_by.as_proto(),
            activated_at: s.activated_at.as_proto(),
            mrr_cents: s.mrr_cents,
            status,
        })
    }

    pub(crate) fn create_proto_to_domain(
        param: proto2::CreateSubscription,
        actor: &Uuid,
    ) -> Result<domain::CreateSubscription, Status> {
        let subscription_new = meteroid_store::domain::SubscriptionNew {
            customer_id: Uuid::from_proto(param.customer_id)?,
            billing_day: param.billing_day as i16,
            currency: param.currency,
            trial_start_date: NaiveDate::from_proto_opt(param.trial_start_date)?,
            billing_start_date: NaiveDate::from_proto(param.billing_start_date)?,
            billing_end_date: NaiveDate::from_proto_opt(param.billing_end_date)?,
            plan_version_id: Uuid::from_proto(param.plan_version_id)?,
            created_by: *actor,
            net_terms: param.net_terms as i32,
            invoice_memo: param.invoice_memo,
            invoice_threshold: rust_decimal::Decimal::from_proto_opt(param.invoice_threshold)?,
            activated_at: None, //NaiveDateTime::from_proto_opt(param.activated_at)?,
        };

        let res = domain::CreateSubscription {
            subscription: subscription_new,
            price_components: param
                .components
                .map(super::price_components::create_subscription_components_from_grpc)
                .transpose()?,
            add_ons: param
                .add_ons
                .map(super::add_ons::create_subscription_add_ons_from_grpc)
                .transpose()?,
            coupons: param
                .coupons
                .as_ref()
                .map(super::coupons::create_subscription_coupons_from_grpc)
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
            billing_day: sub.billing_day as u32,
            tenant_id: sub.tenant_id.as_proto(),
            currency: sub.currency,
            trial_start_date: sub.trial_start_date.as_proto(),
            billing_start_date: sub.billing_start_date.as_proto(),
            billing_end_date: sub.billing_end_date.as_proto(),
            plan_version_id: sub.plan_version_id.as_proto(),
            created_at: sub.created_at.as_proto(),
            created_by: sub.created_by.as_proto(),
            net_terms: sub.net_terms as u32,
            invoice_memo: sub.invoice_memo,
            invoice_threshold: sub.invoice_threshold.as_proto(),
            activated_at: sub.activated_at.as_proto(),
            mrr_cents: sub.mrr_cents as u64,
        })
    }

    pub(crate) fn details_domain_to_proto(
        sub: domain::SubscriptionDetails,
    ) -> Result<proto2::SubscriptionDetails, Status> {
        let status = sub.status_proto()? as i32;
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
                billing_day: sub.billing_day as u32,
                trial_start_date: sub.trial_start_date.as_proto(),
                created_by: sub.created_by.as_proto(),
                activated_at: sub.activated_at.as_proto(),
                mrr_cents: sub.mrr_cents,
                status,
            }),
            schedules: vec![], // TODO
            price_components: sub
                .price_components
                .iter()
                .map(super::price_components::subscription_component_to_grpc)
                .collect(),
            add_ons: sub
                .add_ons
                .iter()
                .map(super::add_ons::subscription_add_on_to_grpc)
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
            coupons: sub
                .coupons
                .iter()
                .map(super::coupons::subscription_coupon_to_grpc)
                .collect(),
        })
    }
}

mod price_components {
    // In meteroid/src/subscription/mod.rs

    use crate::api::shared::conversions::*;
    use meteroid_grpc::meteroid::api::components::v1 as api_components;
    use meteroid_grpc::meteroid::api::shared::v1 as api_shared;
    use meteroid_grpc::meteroid::api::subscriptions::v1 as api;
    use meteroid_store::domain;

    use meteroid_grpc::meteroid::api::components::v1::usage_fee::matrix::MatrixDimension;
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

                let billing_period = c
                    .billing_period
                    .map(api_shared::BillingPeriod::try_from)
                    .transpose()
                    .map_err(|_| Status::invalid_argument("Invalid billing period".to_string()))?
                    .map(map_billing_period_from_grpc);

                Ok::<_, Status>(domain::ComponentParameterization {
                    component_id,
                    parameters: domain::ComponentParameters {
                        initial_slot_count: c.initial_slot_count,
                        billing_period,
                        committed_capacity: c.committed_capacity,
                    },
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
                crate::api::utils::parse_uuid(remove_component_id, "remove_component_id")
            })
            .collect::<Result<Vec<Uuid>>>()?;

        Ok(domain::CreateSubscriptionComponents {
            parameterized_components,
            overridden_components,
            extra_components,
            remove_components,
        })
    }

    pub(crate) fn subscription_fee_billing_period_to_grpc(
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
                fee: Some(api::subscription_fee::Fee::Rate(
                    api::subscription_fee::RateSubscriptionFee {
                        rate: rate.to_string(),
                    },
                )),
            },
            domain::SubscriptionFee::OneTime { rate, quantity } => api::SubscriptionFee {
                fee: Some(api::subscription_fee::Fee::OneTime(
                    api::subscription_fee::OneTimeSubscriptionFee {
                        rate: rate.to_string(),
                        quantity: *quantity,
                        total: (rate * rust_decimal::Decimal::from(*quantity)).to_string(),
                    },
                )),
            },
            domain::SubscriptionFee::Recurring {
                rate,
                quantity,
                billing_type,
            } => api::SubscriptionFee {
                fee: Some(api::subscription_fee::Fee::Recurring(
                    api::subscription_fee::ExtraRecurringSubscriptionFee {
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
                fee: Some(api::subscription_fee::Fee::Capacity(
                    api::subscription_fee::CapacitySubscriptionFee {
                        rate: rate.to_string(),
                        included: *included,
                        overage_rate: overage_rate.to_string(),
                        metric_id: metric_id.to_string(),
                    },
                )),
            },
            domain::SubscriptionFee::Slot {
                unit,
                unit_rate,
                min_slots,
                max_slots,
                initial_slots,
            } => api::SubscriptionFee {
                fee: Some(api::subscription_fee::Fee::Slot(
                    api::subscription_fee::SlotSubscriptionFee {
                        unit: unit.clone(),
                        unit_rate: unit_rate.to_string(),
                        min_slots: *min_slots,
                        max_slots: *max_slots,
                        initial_slots: *initial_slots,
                        upgrade_policy: 0, // TODO add to domain
                        downgrade_policy: 0,
                    },
                )),
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
    ) -> api_components::UsageFee {
        match model {
            domain::UsagePricingModel::PerUnit { rate } => api_components::UsageFee {
                metric_id: metric_id.as_proto(),
                model: Some(api_components::usage_fee::Model::PerUnit(rate.as_proto())),
            },
            domain::UsagePricingModel::Tiered { tiers, block_size } => api_components::UsageFee {
                metric_id: metric_id.as_proto(),
                model: Some(api_components::usage_fee::Model::Tiered(
                    api_components::usage_fee::TieredAndVolume {
                        rows: tiers.iter().map(tier_row_to_grpc).collect(),
                        block_size: *block_size,
                    },
                )),
            },
            domain::UsagePricingModel::Volume { tiers, block_size } => api_components::UsageFee {
                metric_id: metric_id.as_proto(),
                model: Some(api_components::usage_fee::Model::Volume(
                    api_components::usage_fee::TieredAndVolume {
                        rows: tiers.iter().map(tier_row_to_grpc).collect(),
                        block_size: *block_size,
                    },
                )),
            },
            domain::UsagePricingModel::Package { block_size, rate } => api_components::UsageFee {
                metric_id: metric_id.as_proto(),
                model: Some(api_components::usage_fee::Model::Package(
                    api_components::usage_fee::Package {
                        block_size: *block_size,
                        package_price: rate.as_proto(),
                    },
                )),
            },
            domain::UsagePricingModel::Matrix { rates } => api_components::UsageFee {
                metric_id: metric_id.as_proto(),
                model: Some(api_components::usage_fee::Model::Matrix(
                    api_components::usage_fee::Matrix {
                        rows: rates
                            .iter()
                            .map(|r| api_components::usage_fee::matrix::MatrixRow {
                                dimension1: Some(
                                    api_components::usage_fee::matrix::MatrixDimension {
                                        key: r.dimension1.key.clone(),
                                        value: r.dimension1.value.clone(),
                                    },
                                ),
                                dimension2: r.dimension2.as_ref().map(|d| MatrixDimension {
                                    key: d.key.clone(),
                                    value: d.value.clone(),
                                }),
                                per_unit_price: r.per_unit_price.as_proto(),
                            })
                            .collect(),
                    },
                )),
            },
        }
    }

    pub fn tier_row_to_grpc(
        tier: &domain::TierRow,
    ) -> api_components::usage_fee::tiered_and_volume::TierRow {
        api_components::usage_fee::tiered_and_volume::TierRow {
            first_unit: tier.first_unit,
            unit_price: tier.rate.as_proto(),
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
                let model = usage_pricing_model_from_grpc(usage)?;
                Ok(domain::SubscriptionFee::Usage { metric_id, model })
            }
            None => Err(Status::new(
                Code::InvalidArgument,
                "Missing subscription fee",
            )),
        }
    }

    pub fn usage_pricing_model_from_grpc(
        usage: &api_components::UsageFee,
    ) -> Result<domain::UsagePricingModel, Status> {
        match usage.model.as_ref() {
            Some(api_components::usage_fee::Model::PerUnit(per_unit)) => {
                let per_unit = rust_decimal::Decimal::from_proto_ref(per_unit)?;
                Ok(domain::UsagePricingModel::PerUnit { rate: per_unit })
            }
            Some(api_components::usage_fee::Model::Tiered(tiered)) => {
                let tiers = tiered
                    .rows
                    .iter()
                    .map(tier_row_from_grpc)
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(domain::UsagePricingModel::Tiered {
                    tiers,
                    block_size: None, // TODO
                })
            }
            Some(api_components::usage_fee::Model::Volume(volume)) => {
                let tiers = volume
                    .rows
                    .iter()
                    .map(tier_row_from_grpc)
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(domain::UsagePricingModel::Volume {
                    tiers,
                    block_size: None, // TODO
                })
            }
            Some(api_components::usage_fee::Model::Package(package)) => {
                let block_size = package.block_size;
                let rate = rust_decimal::Decimal::from_proto_ref(&package.package_price)?;
                Ok(domain::UsagePricingModel::Package { block_size, rate })
            }
            Some(api_components::usage_fee::Model::Matrix(matrix)) => {
                let rates = matrix
                    .rows
                    .iter()
                    .map(|r| {
                        let dimension1 = r
                            .dimension1
                            .as_ref()
                            .ok_or(Status::invalid_argument("Missing dimension1"))?;

                        Ok::<_, Status>(domain::MatrixRow {
                            dimension1: domain::MatrixDimension {
                                key: dimension1.key.clone(),
                                value: dimension1.value.clone(),
                            },
                            dimension2: r.dimension2.as_ref().map(|d| domain::MatrixDimension {
                                key: d.key.clone(),
                                value: d.value.clone(),
                            }),
                            per_unit_price: rust_decimal::Decimal::from_proto_ref(
                                &r.per_unit_price,
                            )?,
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(domain::UsagePricingModel::Matrix { rates })
            }
            None => Err(Status::new(
                Code::InvalidArgument,
                "Missing usage pricing model",
            )),
        }
    }

    pub fn tier_row_from_grpc(
        tier: &api_components::usage_fee::tiered_and_volume::TierRow,
    ) -> Result<domain::TierRow, Status> {
        let rate = rust_decimal::Decimal::from_proto_ref(&tier.unit_price)?;
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
        billing_type: api_components::fee::BillingType,
    ) -> Result<domain::enums::BillingType, Status> {
        match billing_type {
            api_components::fee::BillingType::Arrear => Ok(domain::enums::BillingType::Arrears),
            api_components::fee::BillingType::Advance => Ok(domain::enums::BillingType::Advance),
        }
    }

    pub fn billing_type_to_grpc(
        billing_type: domain::enums::BillingType,
    ) -> api_components::fee::BillingType {
        match billing_type {
            domain::enums::BillingType::Arrears => api_components::fee::BillingType::Arrear,
            domain::enums::BillingType::Advance => api_components::fee::BillingType::Advance,
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

mod add_ons {
    use crate::api::shared::conversions::ProtoConv;
    use crate::api::subscriptions::mapping::price_components::{
        map_billing_period_from_grpc, subscription_fee_billing_period_from_grpc,
        subscription_fee_billing_period_to_grpc, subscription_fee_from_grpc,
        subscription_fee_to_grpc,
    };
    use meteroid_grpc::meteroid::api::shared::v1 as api_shared;
    use meteroid_grpc::meteroid::api::subscriptions::v1 as api;
    use meteroid_store::domain;
    use tonic::Status;
    use uuid::Uuid;

    pub fn subscription_add_on_to_grpc(
        add_on: &domain::SubscriptionAddOn,
    ) -> api::SubscriptionAddOn {
        api::SubscriptionAddOn {
            id: add_on.id.to_string(),
            add_on_id: add_on.add_on_id.to_string(),
            subscription_id: add_on.subscription_id.to_string(),
            name: add_on.name.clone(),
            period: subscription_fee_billing_period_to_grpc(add_on.period.clone()).into(),
            fee: Some(subscription_fee_to_grpc(&add_on.fee)),
        }
    }

    pub fn create_subscription_add_ons_from_grpc(
        data: api::CreateSubscriptionAddOns,
    ) -> tonic::Result<domain::CreateSubscriptionAddOns> {
        let add_ons = data
            .add_ons
            .into_iter()
            .map(create_subscription_add_on_from_grpc)
            .collect::<tonic::Result<Vec<_>, _>>()?;

        Ok(domain::CreateSubscriptionAddOns { add_ons })
    }

    fn create_subscription_add_on_from_grpc(
        data: api::CreateSubscriptionAddOn,
    ) -> tonic::Result<domain::CreateSubscriptionAddOn> {
        let id = Uuid::from_proto_ref(&data.add_on_id)?;

        let customization: domain::SubscriptionAddOnCustomization = match data.customization {
            Some(api::create_subscription_add_on::Customization::Override(override_)) => {
                let fee = subscription_fee_from_grpc(&override_.fee)?;
                Ok::<domain::SubscriptionAddOnCustomization, Status>(
                    domain::SubscriptionAddOnCustomization::Override(
                        domain::SubscriptionAddOnOverride {
                            name: override_.name.clone(),
                            period: subscription_fee_billing_period_from_grpc(override_.period())?,
                            fee,
                        },
                    ),
                )
            }
            Some(api::create_subscription_add_on::Customization::Parameterization(param)) => {
                let billing_period = param
                    .billing_period
                    .map(api_shared::BillingPeriod::try_from)
                    .transpose()
                    .map_err(|_| Status::invalid_argument("Invalid billing period".to_string()))?
                    .map(map_billing_period_from_grpc);

                Ok(domain::SubscriptionAddOnCustomization::Parameterization(
                    domain::SubscriptionAddOnParameterization {
                        initial_slot_count: param.initial_slot_count,
                        billing_period,
                        committed_capacity: param.committed_capacity,
                    },
                ))
            }
            None => Ok(domain::SubscriptionAddOnCustomization::None),
        }?;

        Ok(domain::CreateSubscriptionAddOn {
            add_on_id: id,
            customization,
        })
    }
}

pub mod ext {
    pub use super::price_components::{
        billing_type_from_grpc, billing_type_to_grpc, usage_pricing_model_from_grpc,
        usage_pricing_model_to_grpc,
    };
}

pub mod coupons {
    use crate::api::coupons::mapping::coupons::discount;
    use crate::api::shared::conversions::ProtoConv;
    use crate::api::shared::mapping::datetime::chrono_to_timestamp;
    use meteroid_grpc::meteroid::api::subscriptions::v1 as api;
    use meteroid_store::domain;
    use uuid::Uuid;

    pub fn create_subscription_coupons_from_grpc(
        data: &api::CreateSubscriptionCoupons,
    ) -> tonic::Result<domain::CreateSubscriptionCoupons> {
        let coupons = data
            .coupons
            .as_slice()
            .iter()
            .map(create_subscription_coupon_from_grpc)
            .collect::<tonic::Result<Vec<_>, _>>()?;

        Ok(domain::CreateSubscriptionCoupons { coupons })
    }

    pub fn create_subscription_coupon_from_grpc(
        data: &api::CreateSubscriptionCoupon,
    ) -> tonic::Result<domain::CreateSubscriptionCoupon> {
        Ok(domain::CreateSubscriptionCoupon {
            coupon_id: Uuid::from_proto_ref(&data.coupon_id)?,
        })
    }

    pub fn subscription_coupon_to_grpc(
        coupon: &domain::SubscriptionCoupon,
    ) -> api::SubscriptionCoupon {
        api::SubscriptionCoupon {
            id: coupon.id.to_string(),
            coupon_id: coupon.coupon_id.to_string(),
            subscription_id: coupon.subscription_id.to_string(),
            coupon_code: coupon.coupon_code.clone(),
            coupon_description: coupon.coupon_description.clone(),
            coupon_discount: Some(discount::to_server(&coupon.coupon_discount)),
            coupon_expires_at: coupon.coupon_expires_at.map(chrono_to_timestamp),
            coupon_redemption_limit: coupon.coupon_redemption_limit,
        }
    }
}
