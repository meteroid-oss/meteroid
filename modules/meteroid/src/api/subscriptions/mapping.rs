pub mod subscriptions {
    use chrono::NaiveDate;
    use meteroid_store::domain;

    use crate::api::connectors::mapping::connectors::connection_metadata_to_server;
    use crate::api::shared::conversions::{AsProtoOpt, FromProtoOpt, ProtoConv};
    use common_domain::ids::{CustomerId, PlanVersionId, SubscriptionId};
    use common_utils::integers::ToNonNegativeU64;
    use meteroid_grpc::meteroid::api::subscriptions::v1 as proto2;
    use meteroid_store::domain::SubscriptionStatusEnum;
    use meteroid_store::domain::enums::SubscriptionActivationCondition;
    use meteroid_store::domain::subscriptions::{
        OnlineMethodConfig, OnlineMethodsConfig, PaymentMethodsConfig,
    };
    use tonic::Status;
    use uuid::Uuid;

    fn map_activation_condition_proto(
        e: proto2::ActivationCondition,
    ) -> SubscriptionActivationCondition {
        match e {
            proto2::ActivationCondition::OnStart => SubscriptionActivationCondition::OnStart,
            proto2::ActivationCondition::Manual => SubscriptionActivationCondition::Manual,
            proto2::ActivationCondition::OnCheckout => SubscriptionActivationCondition::OnCheckout,
        }
    }

    fn domain_payment_methods_config_to_proto(
        config: Option<PaymentMethodsConfig>,
    ) -> Option<proto2::PaymentMethodsConfig> {
        config.map(|c| match c {
            PaymentMethodsConfig::Online { config } => proto2::PaymentMethodsConfig {
                config: Some(proto2::payment_methods_config::Config::Online(
                    proto2::OnlinePayment {
                        config: config.map(|cfg| proto2::OnlineMethodsConfig {
                            card: cfg
                                .card
                                .map(|m| proto2::OnlineMethodConfig { enabled: m.enabled }),
                            direct_debit: cfg
                                .direct_debit
                                .map(|m| proto2::OnlineMethodConfig { enabled: m.enabled }),
                        }),
                    },
                )),
            },
            PaymentMethodsConfig::BankTransfer { account_id: _ } => proto2::PaymentMethodsConfig {
                config: Some(proto2::payment_methods_config::Config::BankTransfer(
                    proto2::BankTransfer {},
                )),
            },
            PaymentMethodsConfig::External => proto2::PaymentMethodsConfig {
                config: Some(proto2::payment_methods_config::Config::External(
                    proto2::External {},
                )),
            },
        })
    }

    fn proto_payment_methods_config_to_domain(
        config: Option<proto2::PaymentMethodsConfig>,
    ) -> Result<Option<PaymentMethodsConfig>, Status> {
        match config {
            None => Ok(None),
            Some(proto_config) => match proto_config.config {
                None => Ok(Some(PaymentMethodsConfig::Online { config: None })),
                Some(proto2::payment_methods_config::Config::Online(online)) => {
                    Ok(Some(PaymentMethodsConfig::Online {
                        config: online.config.map(|cfg| OnlineMethodsConfig {
                            card: cfg.card.map(|m| OnlineMethodConfig { enabled: m.enabled }),
                            direct_debit: cfg
                                .direct_debit
                                .map(|m| OnlineMethodConfig { enabled: m.enabled }),
                        }),
                    }))
                }
                Some(proto2::payment_methods_config::Config::BankTransfer(_)) => {
                    Ok(Some(PaymentMethodsConfig::BankTransfer {
                        account_id: None,
                    }))
                }
                Some(proto2::payment_methods_config::Config::External(_)) => {
                    Ok(Some(PaymentMethodsConfig::External))
                }
            },
        }
    }

    pub fn map_subscription_status(e: SubscriptionStatusEnum) -> proto2::SubscriptionStatus {
        match e {
            SubscriptionStatusEnum::PendingActivation => proto2::SubscriptionStatus::Pending,
            SubscriptionStatusEnum::PendingCharge => proto2::SubscriptionStatus::Pending,
            SubscriptionStatusEnum::TrialActive => proto2::SubscriptionStatus::Trialing,
            SubscriptionStatusEnum::Active => proto2::SubscriptionStatus::Active,
            SubscriptionStatusEnum::TrialExpired => proto2::SubscriptionStatus::TrialExpired,
            SubscriptionStatusEnum::Paused => proto2::SubscriptionStatus::Ended,
            SubscriptionStatusEnum::Suspended => proto2::SubscriptionStatus::Ended,
            SubscriptionStatusEnum::Cancelled => proto2::SubscriptionStatus::Canceled,
            SubscriptionStatusEnum::Completed => proto2::SubscriptionStatus::Ended,
            SubscriptionStatusEnum::Superseded => proto2::SubscriptionStatus::Ended,
            SubscriptionStatusEnum::Errored => proto2::SubscriptionStatus::Errored,
        }
    }

    pub(crate) fn map_proto_status_to_domain(
        status: proto2::SubscriptionStatus,
    ) -> Vec<SubscriptionStatusEnum> {
        match status {
            proto2::SubscriptionStatus::Pending => {
                vec![
                    SubscriptionStatusEnum::PendingActivation,
                    SubscriptionStatusEnum::PendingCharge,
                ]
            }
            proto2::SubscriptionStatus::Trialing => vec![SubscriptionStatusEnum::TrialActive],
            proto2::SubscriptionStatus::Active => vec![SubscriptionStatusEnum::Active],
            proto2::SubscriptionStatus::TrialExpired => vec![SubscriptionStatusEnum::TrialExpired],
            proto2::SubscriptionStatus::Canceled => vec![SubscriptionStatusEnum::Cancelled],
            proto2::SubscriptionStatus::Ended => vec![
                SubscriptionStatusEnum::Paused,
                SubscriptionStatusEnum::Suspended,
                SubscriptionStatusEnum::Completed,
                SubscriptionStatusEnum::Superseded,
            ],
            proto2::SubscriptionStatus::Errored => vec![SubscriptionStatusEnum::Errored],
        }
    }

    pub(crate) fn domain_to_proto(s: domain::Subscription) -> Result<proto2::Subscription, Status> {
        let status = map_subscription_status(s.status) as i32;

        Ok(proto2::Subscription {
            id: s.id.as_proto(),
            local_id: s.id.as_proto(), // todo remove me
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
            start_date: s.start_date.as_proto(),
            end_date: s.end_date.as_proto(),
            billing_start_date: s.billing_start_date.as_proto(),
            customer_name: s.customer_name,
            customer_alias: s.customer_alias,
            billing_day_anchor: u32::from(s.billing_day_anchor),
            trial_duration: s.trial_duration,
            created_by: s.created_by.as_proto(),
            activated_at: s.activated_at.as_proto(),
            mrr_cents: s.mrr_cents,
            status,
            checkout_url: None,
            connection_metadata: s.conn_meta.as_ref().map(connection_metadata_to_server),
            purchase_order: s.purchase_order,
            auto_advance_invoices: s.auto_advance_invoices,
            charge_automatically: s.charge_automatically,
            pending_checkout: s.pending_checkout,
            current_period_start: Some(s.current_period_start.as_proto()),
            current_period_end: s.current_period_end.as_proto(),
            error_count: s.error_count,
            last_error: s.last_error,
            next_retry: s.next_retry.as_proto(),
            quote_id: s.quote_id.map(|id| id.as_proto()),
            payment_methods_config: domain_payment_methods_config_to_proto(
                s.payment_methods_config,
            ),
        })
    }

    pub(crate) fn create_proto_to_domain(
        param: proto2::CreateSubscription,
        actor: &Uuid,
    ) -> Result<domain::CreateSubscription, Status> {
        let subscription_new = domain::SubscriptionNew {
            customer_id: CustomerId::from_proto(&param.customer_id)?,
            billing_day_anchor: param.billing_day_anchor.map(|day| day as u16),
            billing_start_date: None, //TODO
            activation_condition: map_activation_condition_proto(
                proto2::ActivationCondition::try_from(param.activation_condition).map_err(
                    |_| Status::invalid_argument("Invalid activation condition".to_string()),
                )?,
            ),
            plan_version_id: PlanVersionId::from_proto(param.plan_version_id)?,
            created_by: *actor,
            net_terms: param.net_terms,
            invoice_memo: param.invoice_memo,
            invoice_threshold: rust_decimal::Decimal::from_proto_opt(param.invoice_threshold)?,
            start_date: NaiveDate::from_proto(param.start_date)?,
            end_date: NaiveDate::from_proto_opt(param.end_date)?,
            trial_duration: param.trial_duration,
            payment_methods_config: proto_payment_methods_config_to_domain(
                param.payment_methods_config,
            )?,
            auto_advance_invoices: param.auto_advance_invoices.unwrap_or(true),
            charge_automatically: param.charge_automatically.unwrap_or(true),
            purchase_order: param.purchase_order,
            backdate_invoices: false,
            skip_checkout_session: false,
            skip_past_invoices: param.skip_past_invoices.unwrap_or(false),
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
            local_id: sub.id.as_proto(), //todo remove me
            customer_id: sub.customer_id.as_proto(),
            billing_day_anchor: sub.billing_day_anchor as u32,
            tenant_id: sub.tenant_id.as_proto(),
            currency: sub.currency,
            trial_duration: sub.trial_duration.map(|d| d as u32),
            billing_start_date: sub.billing_start_date.as_proto(),
            start_date: sub.start_date.as_proto(),
            end_date: sub.end_date.as_proto(),
            plan_version_id: sub.plan_version_id.as_proto(),
            created_at: sub.created_at.as_proto(),
            created_by: sub.created_by.as_proto(),
            net_terms: sub.net_terms as u32,
            invoice_memo: sub.invoice_memo,
            invoice_threshold: sub.invoice_threshold.as_proto(),
            activated_at: sub.activated_at.as_proto(),
            mrr_cents: sub.mrr_cents.to_non_negative_u64(),
            checkout_url: sub.checkout_url,
            purchase_order: sub.purchase_order,
        })
    }

    pub(crate) fn details_domain_to_proto(
        details: domain::SubscriptionDetails,
    ) -> Result<proto2::SubscriptionDetails, Status> {
        let sub = details.subscription;
        let status = map_subscription_status(sub.status) as i32;
        Ok(proto2::SubscriptionDetails {
            subscription: Some(proto2::Subscription {
                id: sub.id.as_proto(),
                local_id: sub.id.as_proto(), //todo remove me
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
                end_date: sub.end_date.as_proto(),
                start_date: sub.start_date.as_proto(),
                billing_start_date: sub.billing_start_date.as_proto(),
                customer_name: sub.customer_name,
                customer_alias: sub.customer_alias,
                billing_day_anchor: u32::from(sub.billing_day_anchor),
                trial_duration: sub.trial_duration,
                created_by: sub.created_by.as_proto(),
                activated_at: sub.activated_at.as_proto(),
                mrr_cents: sub.mrr_cents,
                status,
                checkout_url: details.checkout_url,
                connection_metadata: sub.conn_meta.as_ref().map(connection_metadata_to_server),
                purchase_order: sub.purchase_order,
                auto_advance_invoices: sub.auto_advance_invoices,
                charge_automatically: sub.charge_automatically,
                pending_checkout: sub.pending_checkout,
                current_period_start: Some(sub.current_period_start.as_proto()),
                current_period_end: sub.current_period_end.as_proto(),
                error_count: sub.error_count,
                last_error: sub.last_error,
                next_retry: sub.next_retry.as_proto(),
                quote_id: sub.quote_id.map(|id| id.as_proto()),
                payment_methods_config: domain_payment_methods_config_to_proto(
                    sub.payment_methods_config,
                ),
            }),
            schedules: vec![], // TODO
            price_components: details
                .price_components
                .iter()
                .map(super::price_components::subscription_component_to_grpc)
                .collect(),
            add_ons: details
                .add_ons
                .iter()
                .map(super::add_ons::subscription_add_on_to_grpc)
                .collect(),
            metrics: details
                .metrics
                .into_iter()
                .map(|m| proto2::BillableMetric {
                    id: m.id.as_proto(),
                    name: m.name,
                    alias: m.code,
                })
                .collect(),
            applied_coupons: details
                .applied_coupons
                .into_iter()
                .map(super::coupons::applied_coupon_detailed_to_grpc)
                .collect(),
            trial_config: details.trial_config.map(|tc| proto2::TrialConfig {
                duration_days: tc.duration_days,
                is_free: tc.is_free,
                trialing_plan_id: tc.trialing_plan_id.map(|id| id.as_proto()),
                trialing_plan_name: tc.trialing_plan_name,
            }),
        })
    }

    pub(crate) fn update_request_to_patch(
        subscription_id: SubscriptionId,
        req: &proto2::UpdateSubscriptionRequest,
    ) -> Result<domain::SubscriptionPatch, Status> {
        Ok(domain::SubscriptionPatch {
            id: subscription_id,
            charge_automatically: req.charge_automatically,
            auto_advance_invoices: req.auto_advance_invoices,
            net_terms: req.net_terms,
            invoice_memo: req
                .invoice_memo
                .as_ref()
                .map(|m| if m.is_empty() { None } else { Some(m.clone()) }),
            purchase_order: req
                .purchase_order
                .as_ref()
                .map(|p| if p.is_empty() { None } else { Some(p.clone()) }),
            payment_methods_config: req
                .payment_methods_config
                .as_ref()
                .map(|proto_config| proto_payment_methods_config_to_domain(Some(*proto_config)))
                .transpose()?,
        })
    }
}

pub mod price_components {
    // In meteroid/src/subscription/mod.rs

    use crate::api::pricecomponents::mapping::components::{
        price_entries_from_proto, product_ref_from_proto,
    };
    use crate::api::shared::conversions::ProtoConv;
    use meteroid_grpc::meteroid::api::components::v1 as api_components;
    use meteroid_grpc::meteroid::api::prices::v1 as api_prices;
    use meteroid_grpc::meteroid::api::shared::v1 as api_shared;
    use meteroid_grpc::meteroid::api::subscriptions::v1 as api;
    use meteroid_store::domain;

    use common_domain::ids::{BillableMetricId, PriceComponentId};
    use meteroid_store::domain::BillingPeriodEnum;
    use tonic::{Code, Result, Status};

    pub fn create_subscription_components_from_grpc(
        data: api::CreateSubscriptionComponents,
    ) -> Result<domain::CreateSubscriptionComponents> {
        let parameterized_components = data
            .parameterized_components
            .into_iter()
            .map(|c| {
                let component_id = PriceComponentId::from_proto(&c.component_id)?;

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
                let component_id = PriceComponentId::from_proto(&c.component_id)?;
                let price_entry = price_entries_from_proto(vec![c.price.ok_or_else(|| {
                    Status::invalid_argument("Missing override price entry".to_string())
                })?])?
                .into_iter()
                .next()
                .ok_or_else(|| {
                    Status::invalid_argument("Missing override price entry".to_string())
                })?;

                Ok::<_, Status>(domain::ComponentOverride {
                    component_id,
                    name: c.name,
                    price_entry,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        let extra_components = data
            .extra_components
            .into_iter()
            .map(|c| {
                let product_ref = product_ref_from_proto(c.product)?;
                let price_entry = price_entries_from_proto(vec![c.price.ok_or_else(|| {
                    Status::invalid_argument("Missing extra component price entry".to_string())
                })?])?
                .into_iter()
                .next()
                .ok_or_else(|| {
                    Status::invalid_argument("Missing extra component price entry".to_string())
                })?;

                Ok::<_, Status>(domain::ExtraComponent {
                    name: c.name,
                    product_ref,
                    price_entry,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        let remove_components = data
            .remove_components
            .iter()
            .map(PriceComponentId::from_proto)
            .collect::<Result<Vec<PriceComponentId>>>()?;

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
            domain::enums::SubscriptionFeeBillingPeriod::Semiannual => {
                api::SubscriptionFeeBillingPeriod::Semiannual
            }
            domain::enums::SubscriptionFeeBillingPeriod::Annual => {
                api::SubscriptionFeeBillingPeriod::Yearly
            }
        }
    }

    pub fn subscription_component_to_grpc(
        component: &domain::SubscriptionComponent,
    ) -> api::SubscriptionComponent {
        api::SubscriptionComponent {
            id: component.id.to_string(),
            price_component_id: component.price_component_id.map(|id| id.to_string()),
            product_id: component.product_id.map(|id| id.to_string()),
            subscription_id: component.subscription_id.to_string(),
            name: component.name.clone(),
            period: subscription_fee_billing_period_to_grpc(component.period).into(),
            fee: Some(subscription_fee_to_grpc(
                &component.fee,
                component.period.as_billing_period_opt().unwrap_or_default(),
            )),
            is_override: false, // TODO: Update this based on your logic
        }
    }

    pub fn subscription_fee_to_grpc(
        fee: &domain::SubscriptionFee,
        period: BillingPeriodEnum,
    ) -> api::SubscriptionFee {
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
                    usage_pricing_model_to_grpc(metric_id, model, period),
                )),
            },
        }
    }

    pub fn usage_pricing_model_to_grpc(
        metric_id: &BillableMetricId,
        model: &domain::UsagePricingModel,
        cadence: BillingPeriodEnum,
    ) -> api_components::UsageFee {
        use crate::api::prices::mapping::prices::usage_model_to_proto;

        let pricing = usage_model_to_proto(model);
        let fee_model = pricing.model.map(usage_pricing_model_to_fee);

        api_components::UsageFee {
            metric_id: metric_id.as_proto(),
            model: fee_model,
            term: billing_period_to_grpc(cadence).into(),
        }
    }

    fn usage_pricing_model_to_fee(
        model: api_prices::usage_pricing::Model,
    ) -> api_components::usage_fee::Model {
        match model {
            api_prices::usage_pricing::Model::PerUnit(v) => {
                api_components::usage_fee::Model::PerUnit(v)
            }
            api_prices::usage_pricing::Model::Tiered(v) => {
                api_components::usage_fee::Model::Tiered(v)
            }
            api_prices::usage_pricing::Model::Volume(v) => {
                api_components::usage_fee::Model::Volume(v)
            }
            api_prices::usage_pricing::Model::Package(v) => {
                api_components::usage_fee::Model::Package(v)
            }
            api_prices::usage_pricing::Model::Matrix(v) => {
                api_components::usage_fee::Model::Matrix(v)
            }
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
                let metric_id = BillableMetricId::from_proto(&capacity.metric_id)?;
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
                let metric_id = BillableMetricId::from_proto(&usage.metric_id)?;
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
        use crate::api::prices::mapping::prices::usage_model_from_proto;

        let pricing_model = usage.model.as_ref().map(fee_model_to_usage_pricing);
        let as_pricing = api_prices::UsagePricing {
            model: pricing_model,
        };
        usage_model_from_proto(&as_pricing)
    }

    fn fee_model_to_usage_pricing(
        model: &api_components::usage_fee::Model,
    ) -> api_prices::usage_pricing::Model {
        match model {
            api_components::usage_fee::Model::PerUnit(v) => {
                api_prices::usage_pricing::Model::PerUnit(v.clone())
            }
            api_components::usage_fee::Model::Tiered(v) => {
                api_prices::usage_pricing::Model::Tiered(v.clone())
            }
            api_components::usage_fee::Model::Volume(v) => {
                api_prices::usage_pricing::Model::Volume(v.clone())
            }
            api_components::usage_fee::Model::Package(v) => {
                api_prices::usage_pricing::Model::Package(v.clone())
            }
            api_components::usage_fee::Model::Matrix(v) => {
                api_prices::usage_pricing::Model::Matrix(v.clone())
            }
        }
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
            api::SubscriptionFeeBillingPeriod::Semiannual => {
                Ok(domain::enums::SubscriptionFeeBillingPeriod::Semiannual)
            }
            api::SubscriptionFeeBillingPeriod::Yearly => {
                Ok(domain::enums::SubscriptionFeeBillingPeriod::Annual)
            } // _ => Err(Status::new(Code::InvalidArgument, "Invalid billing period")),
        }
    }

    pub fn billing_type_from_grpc(
        billing_type: api_prices::fee_structure::BillingType,
    ) -> Result<domain::enums::BillingType, Status> {
        match billing_type {
            api_prices::fee_structure::BillingType::Arrear => {
                Ok(domain::enums::BillingType::Arrears)
            }
            api_prices::fee_structure::BillingType::Advance => {
                Ok(domain::enums::BillingType::Advance)
            }
        }
    }

    pub fn billing_type_to_grpc(
        billing_type: domain::enums::BillingType,
    ) -> api_prices::fee_structure::BillingType {
        match billing_type {
            domain::enums::BillingType::Arrears => api_prices::fee_structure::BillingType::Arrear,
            domain::enums::BillingType::Advance => api_prices::fee_structure::BillingType::Advance,
        }
    }

    pub fn map_billing_period_from_grpc(
        period: api_shared::BillingPeriod,
    ) -> domain::enums::BillingPeriodEnum {
        match period {
            api_shared::BillingPeriod::Monthly => domain::enums::BillingPeriodEnum::Monthly,
            api_shared::BillingPeriod::Quarterly => domain::enums::BillingPeriodEnum::Quarterly,
            api_shared::BillingPeriod::Semiannual => domain::enums::BillingPeriodEnum::Semiannual,
            api_shared::BillingPeriod::Annual => domain::enums::BillingPeriodEnum::Annual,
        }
    }

    fn billing_period_to_grpc(period: BillingPeriodEnum) -> api_shared::BillingPeriod {
        match period {
            BillingPeriodEnum::Monthly => api_shared::BillingPeriod::Monthly,
            BillingPeriodEnum::Quarterly => api_shared::BillingPeriod::Quarterly,
            BillingPeriodEnum::Semiannual => api_shared::BillingPeriod::Semiannual,
            BillingPeriodEnum::Annual => api_shared::BillingPeriod::Annual,
        }
    }
}

pub mod add_ons {
    use crate::api::pricecomponents::mapping::components::price_entries_from_proto;
    use crate::api::subscriptions::mapping::price_components::{
        map_billing_period_from_grpc, subscription_fee_billing_period_to_grpc,
        subscription_fee_to_grpc,
    };
    use common_domain::ids::AddOnId;
    use meteroid_grpc::meteroid::api::shared::v1 as api_shared;
    use meteroid_grpc::meteroid::api::subscriptions::v1 as api;
    use meteroid_store::domain;
    use tonic::Status;

    pub fn subscription_add_on_to_grpc(
        add_on: &domain::SubscriptionAddOn,
    ) -> api::SubscriptionAddOn {
        api::SubscriptionAddOn {
            id: add_on.id.to_string(),
            add_on_id: add_on.add_on_id.to_string(),
            subscription_id: add_on.subscription_id.to_string(),
            name: add_on.name.clone(),
            period: subscription_fee_billing_period_to_grpc(add_on.period).into(),
            fee: Some(subscription_fee_to_grpc(
                &add_on.fee,
                add_on.period.as_billing_period_opt().unwrap_or_default(),
            )),
            quantity: add_on.quantity as u32,
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
        let id = AddOnId::from_proto(&data.add_on_id)?;
        let quantity = if data.quantity == 0 { 1 } else { data.quantity };

        let customization: domain::SubscriptionAddOnCustomization = match data.customization {
            Some(api::create_subscription_add_on::Customization::PriceOverride(ov)) => {
                let price_entry = ov
                    .price_entry
                    .ok_or_else(|| {
                        Status::invalid_argument("price_entry is required for price override")
                    })?;
                let entries = price_entries_from_proto(vec![price_entry])?;
                let entry = entries.into_iter().next().ok_or_else(|| {
                    Status::invalid_argument("price_entry is required for price override")
                })?;

                Ok::<domain::SubscriptionAddOnCustomization, Status>(
                    domain::SubscriptionAddOnCustomization::PriceOverride {
                        name: ov.name,
                        price_entry: entry,
                    },
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
            quantity,
        })
    }
}

pub mod ext {
    pub use super::price_components::{
        billing_type_from_grpc, billing_type_to_grpc, usage_pricing_model_from_grpc,
        usage_pricing_model_to_grpc,
    };
}

pub mod plan_change {
    use super::price_components::map_billing_period_from_grpc;
    use common_domain::ids::PriceComponentId;
    use meteroid_grpc::meteroid::api::shared::v1 as api_shared;
    use meteroid_grpc::meteroid::api::subscriptions::v1 as api;
    use meteroid_store::domain::subscription_components::{
        ComponentParameterization, ComponentParameters,
    };
    use tonic::Status;

    pub fn map_component_parameterizations(
        protos: Vec<api::PlanChangeComponentParameterization>,
    ) -> Result<Vec<ComponentParameterization>, Status> {
        protos
            .into_iter()
            .map(|p| {
                let component_id = PriceComponentId::from_proto(&p.component_id)?;

                let billing_period = p
                    .billing_period
                    .map(api_shared::BillingPeriod::try_from)
                    .transpose()
                    .map_err(|_| Status::invalid_argument("Invalid billing period"))?
                    .map(map_billing_period_from_grpc);

                Ok(ComponentParameterization {
                    component_id,
                    parameters: ComponentParameters {
                        initial_slot_count: p.initial_slot_count,
                        billing_period,
                        committed_capacity: p.committed_capacity,
                    },
                })
            })
            .collect()
    }
}

pub mod coupons {
    use crate::api::coupons::mapping::coupons as coupon_mapping;
    use crate::api::shared::mapping::datetime::chrono_to_timestamp;
    use common_domain::ids::CouponId;
    use meteroid_grpc::meteroid::api::coupons::v1 as coupon_api;
    use meteroid_grpc::meteroid::api::subscriptions::v1 as api;
    use meteroid_store::domain;

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
            coupon_id: CouponId::from_proto(&data.coupon_id)?,
        })
    }

    pub fn applied_coupon_detailed_to_grpc(
        applied_coupon: domain::AppliedCouponDetailed,
    ) -> coupon_api::AppliedCouponDetailed {
        coupon_api::AppliedCouponDetailed {
            coupon: Some(coupon_mapping::to_server(applied_coupon.coupon)),
            applied_coupon: Some(applied_coupon_to_grpc(&applied_coupon.applied_coupon)),
        }
    }

    pub fn applied_coupon_to_grpc(
        applied_coupon: &domain::AppliedCoupon,
    ) -> coupon_api::AppliedCoupon {
        coupon_api::AppliedCoupon {
            id: applied_coupon.id.to_string(),
            coupon_id: applied_coupon.coupon_id.to_string(),
            customer_id: applied_coupon.customer_id.to_string(),
            subscription_id: applied_coupon.subscription_id.to_string(),
            is_active: applied_coupon.is_active,
            applied_amount: applied_coupon
                .applied_amount
                .as_ref()
                .map(std::string::ToString::to_string),
            applied_count: applied_coupon.applied_count,
            last_applied_at: applied_coupon.last_applied_at.map(chrono_to_timestamp),
            created_at: Some(chrono_to_timestamp(applied_coupon.created_at)),
        }
    }
}

pub mod slot_transactions {
    use crate::api::shared::conversions::ProtoConv;
    use meteroid_grpc::meteroid::api::subscriptions::v1 as proto;
    use meteroid_store::domain::SlotTransactionStatusEnum;
    use meteroid_store::domain::slot_transactions::{SlotTransaction, SlotUpdatePreview};
    use tonic::Status;

    pub fn domain_to_proto(transaction: SlotTransaction) -> Result<proto::SlotTransaction, Status> {
        let proto_status = match transaction.status {
            SlotTransactionStatusEnum::Pending => proto::SlotTransactionStatus::SlotPending,
            SlotTransactionStatusEnum::Active => proto::SlotTransactionStatus::SlotActive,
        };

        Ok(proto::SlotTransaction {
            id: transaction.id.to_string(),
            subscription_id: transaction.subscription_id.to_string(),
            unit: transaction.unit,
            delta: transaction.delta,
            prev_active_slots: transaction.prev_active_slots,
            new_active_slots: transaction.prev_active_slots + transaction.delta,
            effective_at: transaction.effective_at.as_proto(),
            transaction_at: transaction.transaction_at.as_proto(),
            status: proto_status.into(),
            invoice_id: transaction.invoice_id.map(|id| id.to_string()),
        })
    }

    pub fn preview_domain_to_proto(
        preview: SlotUpdatePreview,
    ) -> Result<proto::PreviewSlotUpdateResponse, Status> {
        Ok(proto::PreviewSlotUpdateResponse {
            current_slots: preview.current_slots,
            new_slots: preview.new_slots,
            delta: preview.delta,
            unit: preview.unit,
            unit_rate: preview.unit_rate.to_string(),
            prorated_amount: preview.prorated_amount.to_string(),
            full_period_amount: preview.full_period_amount.to_string(),
            days_remaining: preview.days_remaining,
            days_total: preview.days_total,
            effective_at: preview.effective_at.as_proto(),
            current_period_end: preview.current_period_end.as_proto(),
            next_invoice_delta: preview.next_invoice_delta.to_string(),
        })
    }
}
