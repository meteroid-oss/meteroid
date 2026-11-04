pub mod subscriptions {
    use chrono::NaiveDate;
    use meteroid_store::domain;

    use crate::connectors::mapping::connectors::connection_metadata_to_server;
    use crate::entitlements::mapping::entitlement_spec_from_proto;
    use common_domain::ids::{CustomerId, PlanVersionId, SubscriptionId};
    use common_utils::integers::ToNonNegativeU64;
    use meteroid_app::conversions::{AsProtoOpt, FromProtoOpt, ProtoConv};
    use meteroid_grpc::meteroid::api::subscriptions::v1 as proto2;
    use meteroid_store::domain::SubscriptionStatusEnum;
    use meteroid_store::domain::enums::{ScheduledEventTypeEnum, SubscriptionActivationCondition};
    use meteroid_store::domain::subscriptions::{
        OnlineMethodConfig, OnlineMethodsConfig, PaymentMethodsConfig, PendingScheduledEvent,
    };
    use tonic::Status;

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
            SubscriptionStatusEnum::Aborted => proto2::SubscriptionStatus::Aborted,
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
            proto2::SubscriptionStatus::Aborted => vec![SubscriptionStatusEnum::Aborted],
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
            custom_properties: (!s.custom_properties.is_null())
                .then(|| s.custom_properties.to_string()),
            tax_inclusive: s.tax_inclusive,
            usage_filter: usage_filter_to_proto(s.usage_filter),
            usage_filter_effective_from: s
                .usage_filter_since
                .map(|since| since.and_utc().to_rfc3339()),
        })
    }

    pub(crate) fn unattributed_usage_to_proto(
        usage: domain::UnattributedUsage,
    ) -> proto2::UnattributedUsage {
        proto2::UnattributedUsage {
            metric_id: usage.metric.id.as_proto(),
            metric_name: usage.metric.name,
            unit_label: crate::billablemetrics::mapping::metric::unit_label_to_server(
                usage.metric.unit_singular,
                usage.metric.unit_plural,
            ),
            from: usage.from.and_utc().to_rfc3339(),
            until: usage.until.and_utc().to_rfc3339(),
            quantity: usage.quantity.normalize().to_string(),
            subscription_ids: usage
                .subscription_ids
                .iter()
                .map(|id| id.as_proto())
                .collect(),
        }
    }

    pub(crate) fn usage_filter_to_proto(
        filter: Option<domain::UsageFilter>,
    ) -> Option<proto2::UsageFilter> {
        filter.map(|filter| proto2::UsageFilter {
            property: filter.property,
            value: filter.value,
        })
    }

    pub(crate) fn usage_filter_from_proto(
        filter: proto2::UsageFilter,
    ) -> Result<domain::UsageFilter, Status> {
        domain::UsageFilter::new(filter.property, filter.value)
            .map_err(|e| Status::invalid_argument(e.current_context().to_string()))
    }

    pub(crate) fn create_proto_to_domain(
        param: proto2::CreateSubscription,
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
            usage_filter: param
                .usage_filter
                .map(usage_filter_from_proto)
                .transpose()?,
            custom_properties: param
                .custom_properties
                .map(|s| serde_json::from_str::<serde_json::Value>(&s))
                .transpose()
                .map_err(|e| {
                    Status::invalid_argument(format!("Invalid custom_properties JSON: {e}"))
                })?
                .unwrap_or(serde_json::Value::Null),
        };

        let entitlements = param
            .entitlements
            .into_iter()
            .map(entitlement_spec_from_proto)
            .collect::<Result<Vec<_>, _>>()?;

        let minimum_override = param
            .minimum_commitment_override
            .map(crate::plans::mapping::plans::create_minimum_from_proto)
            .transpose()
            .map_err(Status::invalid_argument)?;

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
            entitlements,
            minimum_override,
            drop_minimum: param.drop_minimum_commitment,
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
        let minimum_period = crate::domain_mapping::billing_period::to_proto(sub.period) as i32;
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
                custom_properties: (!sub.custom_properties.is_null())
                    .then(|| sub.custom_properties.to_string()),
                tax_inclusive: sub.tax_inclusive,
                usage_filter: usage_filter_to_proto(sub.usage_filter),
                usage_filter_effective_from: sub
                    .usage_filter_since
                    .map(|since| since.and_utc().to_rfc3339()),
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
                    unit_conversion_factor: m.unit_conversion_factor,
                    unit_label: crate::billablemetrics::mapping::metric::unit_label_to_server(
                        m.unit_singular,
                        m.unit_plural,
                    ),
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
            pending_events: details
                .pending_events
                .into_iter()
                .filter_map(pending_event_to_proto)
                .collect(),
            minimum_commitment: details
                .minimum
                .as_ref()
                .map(|m| proto2::SubscriptionMinimum {
                    commitment: Some(crate::plans::mapping::plans::subscription_minimum_to_proto(
                        m,
                    )),
                    is_override: m.is_override,
                    period: minimum_period,
                }),
        })
    }

    fn pending_event_to_proto(
        event: PendingScheduledEvent,
    ) -> Option<proto2::PendingScheduledEvent> {
        let event_type = match event.event_type {
            ScheduledEventTypeEnum::ApplyPlanChange => proto2::ScheduledEventType::PlanChange,
            ScheduledEventTypeEnum::CancelSubscription => proto2::ScheduledEventType::Cancel,
            ScheduledEventTypeEnum::PauseSubscription => proto2::ScheduledEventType::Pause,
            ScheduledEventTypeEnum::EndTrial => proto2::ScheduledEventType::EndTrial,
            ScheduledEventTypeEnum::ApplyAmendment => proto2::ScheduledEventType::Amendment,
            // Truly internal billing events are not surfaced to API clients.
            ScheduledEventTypeEnum::FinalizeInvoice | ScheduledEventTypeEnum::RetryPayment => {
                return None;
            }
        };

        Some(proto2::PendingScheduledEvent {
            id: event.id.as_proto(),
            event_type: event_type.into(),
            scheduled_date: event.scheduled_date.as_proto(),
            new_plan_name: event.new_plan_name,
            new_plan_version_id: event.new_plan_version_id.map(|id| id.as_proto()),
            cancel_reason: event.cancel_reason,
            customer_initiated: event.customer_initiated,
            amendment_summary: event.amendment_summary.map(|s| proto2::AmendmentSummary {
                added_component_names: s.added_component_names,
                removed_component_names: s.removed_component_names,
                added_add_on_names: s.added_add_on_names,
                removed_add_on_names: s.removed_add_on_names,
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
            usage_filter: req
                .usage_filter
                .clone()
                .map(|change| change.filter.map(usage_filter_from_proto).transpose())
                .transpose()?,
            custom_properties: req
                .custom_properties
                .as_ref()
                .map(|raw| {
                    serde_json::from_str::<serde_json::Value>(raw).map_err(|e| {
                        Status::invalid_argument(format!("Invalid custom_properties JSON: {e}"))
                    })
                })
                .transpose()?,
        })
    }
}

pub mod price_components {
    // In meteroid/src/subscription/mod.rs

    use crate::pricecomponents::mapping::components::{
        price_entries_from_proto, product_ref_from_proto,
    };
    use meteroid_grpc::meteroid::api::components::v1 as api_components;
    use meteroid_grpc::meteroid::api::prices::v1 as api_prices;
    use meteroid_grpc::meteroid::api::shared::v1 as api_shared;
    use meteroid_grpc::meteroid::api::subscriptions::v1 as api;
    use meteroid_store::domain;

    use common_domain::ids::{BillableMetricId, PriceComponentId};
    use meteroid_store::domain::BillingPeriodEnum;
    use tonic::{Result, Status};

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
        api::SubscriptionFee {
            credit_unit: fee_credit_unit_to_grpc(fee),
            ..subscription_fee_kind_to_grpc(fee, period)
        }
    }

    fn fee_credit_unit_to_grpc(
        fee: &domain::SubscriptionFee,
    ) -> Option<api::subscription_fee::FeeCreditUnit> {
        match fee {
            domain::SubscriptionFee::Usage {
                credit_unit: Some(unit),
                ..
            }
            | domain::SubscriptionFee::Capacity {
                credit_unit: Some(unit),
                ..
            } => Some(api::subscription_fee::FeeCreditUnit {
                credit_unit_id: unit.unit_id.as_proto(),
                code: unit.code.clone(),
                precision: u32::from(unit.precision),
                rate: Some(unit.rate.to_string()),
                name: None,
                plural_name: None,
            }),
            domain::SubscriptionFee::Credit {
                credit_unit: Some(unit),
                ..
            } => Some(api::subscription_fee::FeeCreditUnit {
                credit_unit_id: unit.unit_id.as_proto(),
                code: unit.code.clone(),
                precision: u32::from(unit.precision),
                rate: None,
                name: None,
                plural_name: None,
            }),
            _ => None,
        }
    }

    fn subscription_fee_kind_to_grpc(
        fee: &domain::SubscriptionFee,
        period: BillingPeriodEnum,
    ) -> api::SubscriptionFee {
        match fee {
            domain::SubscriptionFee::Rate { rate } => api::SubscriptionFee {
                credit_unit: None,
                fee: Some(api::subscription_fee::Fee::Rate(
                    api::subscription_fee::RateSubscriptionFee {
                        rate: rate.to_string(),
                    },
                )),
            },
            domain::SubscriptionFee::OneTime { rate, quantity } => api::SubscriptionFee {
                credit_unit: None,
                fee: Some(api::subscription_fee::Fee::OneTime(
                    api::subscription_fee::OneTimeSubscriptionFee {
                        rate: rate.to_string(),
                        quantity: *quantity,
                        total: (rate * rust_decimal::Decimal::from(*quantity)).to_string(),
                    },
                )),
            },
            domain::SubscriptionFee::Credit {
                rate,
                quantity,
                credit_amount,
                rollover,
                once,
                ..
            } => api::SubscriptionFee {
                credit_unit: None,
                fee: Some(api::subscription_fee::Fee::Credit(
                    api::subscription_fee::CreditSubscriptionFee {
                        rate: rate.to_string(),
                        quantity: *quantity,
                        total: (rate * rust_decimal::Decimal::from(*quantity)).to_string(),
                        credit_amount: credit_amount.to_string(),
                        credit_total: (credit_amount * rust_decimal::Decimal::from(*quantity))
                            .to_string(),
                        rollover: rollover
                            .as_ref()
                            .map(crate::prices::mapping::prices::credit_rollover_to_proto),
                        once: *once,
                    },
                )),
            },
            domain::SubscriptionFee::Recurring {
                rate,
                quantity,
                billing_type,
            } => api::SubscriptionFee {
                credit_unit: None,
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
                ..
            } => api::SubscriptionFee {
                credit_unit: None,
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
                credit_unit: None,
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
            domain::SubscriptionFee::Usage {
                metric_id, model, ..
            } => api::SubscriptionFee {
                credit_unit: None,
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
        use crate::prices::mapping::prices::usage_model_to_proto;

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
    use crate::pricecomponents::mapping::components::price_entries_from_proto;
    use crate::subscriptions::mapping::price_components::{
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
            quantity: add_on.quantity.max(0) as u32,
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
        let quantity: i32 = if data.quantity == 0 {
            1
        } else {
            i32::try_from(data.quantity)
                .map_err(|_| Status::invalid_argument("quantity exceeds maximum allowed value"))?
        };

        let customization: domain::SubscriptionAddOnCustomization = match data.customization {
            Some(api::create_subscription_add_on::Customization::PriceOverride(ov)) => {
                let price_entry = ov.price_entry.ok_or_else(|| {
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
        billing_type_from_grpc, billing_type_to_grpc, usage_pricing_model_to_grpc,
    };
}

pub mod plan_change {
    use super::price_components::map_billing_period_from_grpc;
    use common_domain::ids::PriceComponentId;
    use meteroid_grpc::meteroid::api::shared::v1 as api_shared;
    use meteroid_grpc::meteroid::api::subscriptions::v1 as api;
    use meteroid_store::domain::subscription_changes::{
        ChangeDirection, PlanChangeMode, ProrationSummary,
    };
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

    pub fn map_apply_mode(mode: i32) -> PlanChangeMode {
        match api::PlanChangeApplyMode::try_from(mode) {
            Ok(api::PlanChangeApplyMode::Immediate) => PlanChangeMode::Immediate,
            _ => PlanChangeMode::EndOfPeriod,
        }
    }

    pub fn change_direction_to_string(dir: ChangeDirection) -> String {
        match dir {
            ChangeDirection::Upgrade => "upgrade".to_string(),
            ChangeDirection::Downgrade => "downgrade".to_string(),
            ChangeDirection::Lateral => "lateral".to_string(),
        }
    }

    pub fn proration_summary_to_grpc(summary: &ProrationSummary) -> api::ProrationSummary {
        api::ProrationSummary {
            credits_total_cents: summary.credits_total_cents,
            charges_total_cents: summary.charges_total_cents,
            net_amount_cents: summary.net_amount_cents,
            net_credit_cents: summary.net_credit_cents,
            arrears_charge_cents: summary.arrears_charge_cents,
            proration_factor: summary.proration_factor,
            days_remaining: summary.days_remaining,
            days_in_period: summary.days_in_period,
            pack_changes: summary
                .pack_changes
                .iter()
                .map(pack_change_to_grpc)
                .collect(),
        }
    }

    pub fn pack_change_to_grpc(
        change: &meteroid_store::domain::subscription_changes::PackChange,
    ) -> api::PlanChangePackLine {
        use meteroid_store::domain::subscription_changes::PackChangeKind;
        api::PlanChangePackLine {
            name: change.name.clone(),
            kind: match change.kind {
                PackChangeKind::Charge => api::plan_change_pack_line::Kind::Charge,
                PackChangeKind::CreditRemoved => api::plan_change_pack_line::Kind::CreditRemoved,
                PackChangeKind::FeeCredit => api::plan_change_pack_line::Kind::FeeCredit,
            }
            .into(),
            amount_cents: change.amount_cents,
            credit_amount: rust_decimal::Decimal::new(
                change.credit_minor,
                u32::from(change.credit_precision),
            )
            .normalize()
            .to_string(),
            credit_unit: change.credit_unit.as_ref().map(|unit| {
                api::subscription_fee::FeeCreditUnit {
                    credit_unit_id: unit.unit_id.as_proto(),
                    code: unit.code.clone(),
                    precision: u32::from(unit.precision),
                    rate: None,
                    name: None,
                    plural_name: None,
                }
            }),
            promotional: change.promotional,
        }
    }
}

pub mod amendment {
    use super::add_ons::create_subscription_add_ons_from_grpc;
    use super::price_components::map_billing_period_from_grpc;
    use crate::pricecomponents::mapping::components::{
        price_entries_from_proto, product_ref_from_proto,
    };
    use common_domain::ids::{SubscriptionAddOnId, SubscriptionPriceComponentId};
    use meteroid_grpc::meteroid::api::shared::v1 as api_shared;
    use meteroid_grpc::meteroid::api::subscriptions::v1 as api;
    use meteroid_store::domain;
    use meteroid_store::domain::subscription_amendment::{
        AddExtraComponent, AddOnChanges, ComponentChanges, EditComponent, EditSubscriptionAddOn,
        SubscriptionAmendment,
    };
    use tonic::{Result, Status};

    fn one_price_entry(
        price: Option<meteroid_grpc::meteroid::api::components::v1::PriceEntry>,
    ) -> Result<domain::PriceEntry> {
        let price =
            price.ok_or_else(|| Status::invalid_argument("price entry is required".to_string()))?;
        price_entries_from_proto(vec![price])?
            .into_iter()
            .next()
            .ok_or_else(|| Status::invalid_argument("price entry is required".to_string()))
    }

    fn map_edit_addon_customization(
        e: &api::AmendmentEditAddOn,
    ) -> Result<Option<domain::SubscriptionAddOnCustomization>> {
        match &e.customization {
            Some(api::amendment_edit_add_on::Customization::PriceOverride(ov)) => {
                let entry = one_price_entry(ov.price_entry.clone())?;
                Ok(Some(
                    domain::SubscriptionAddOnCustomization::PriceOverride {
                        name: ov.name.clone(),
                        price_entry: entry,
                    },
                ))
            }
            Some(api::amendment_edit_add_on::Customization::Parameterization(param)) => {
                let billing_period = param
                    .billing_period
                    .map(api_shared::BillingPeriod::try_from)
                    .transpose()
                    .map_err(|_| Status::invalid_argument("Invalid billing period".to_string()))?
                    .map(map_billing_period_from_grpc);
                Ok(Some(
                    domain::SubscriptionAddOnCustomization::Parameterization(
                        domain::SubscriptionAddOnParameterization {
                            initial_slot_count: param.initial_slot_count,
                            billing_period,
                            committed_capacity: param.committed_capacity,
                        },
                    ),
                ))
            }
            None => Ok(None),
        }
    }

    pub fn map_amendment(
        apply_mode: i32,
        component_changes: Option<api::AmendmentComponentChanges>,
        add_on_changes: Option<api::AmendmentAddOnChanges>,
    ) -> Result<SubscriptionAmendment> {
        let apply_mode = super::plan_change::map_apply_mode(apply_mode);

        let component_changes = match component_changes {
            Some(cc) => {
                let edited = cc
                    .edited
                    .into_iter()
                    .map(|e| {
                        Ok::<_, Status>(EditComponent {
                            subscription_component_id: SubscriptionPriceComponentId::from_proto(
                                &e.subscription_component_id,
                            )?,
                            name: e.name,
                            price_entry: one_price_entry(e.price)?,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                let added = cc
                    .added
                    .into_iter()
                    .map(|a| {
                        Ok::<_, Status>(AddExtraComponent {
                            name: a.name,
                            product: product_ref_from_proto(a.product)?,
                            price_entry: one_price_entry(a.price)?,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                let removed = cc
                    .removed_component_ids
                    .iter()
                    .map(SubscriptionPriceComponentId::from_proto)
                    .collect::<Result<Vec<_>>>()?;
                ComponentChanges {
                    edited,
                    added,
                    removed,
                }
            }
            None => ComponentChanges::default(),
        };

        let add_on_changes = match add_on_changes {
            Some(ac) => {
                let added = create_subscription_add_ons_from_grpc(api::CreateSubscriptionAddOns {
                    add_ons: ac.added,
                })?
                .add_ons;
                let edited = ac
                    .edited
                    .iter()
                    .map(|e| {
                        Ok::<_, Status>(EditSubscriptionAddOn {
                            subscription_add_on_id: SubscriptionAddOnId::from_proto(
                                &e.subscription_add_on_id,
                            )?,
                            quantity: e.quantity,
                            customization: map_edit_addon_customization(e)?,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                let removed = ac
                    .removed_add_on_ids
                    .iter()
                    .map(SubscriptionAddOnId::from_proto)
                    .collect::<Result<Vec<_>>>()?;
                AddOnChanges {
                    added,
                    edited,
                    removed,
                }
            }
            None => AddOnChanges::default(),
        };

        Ok(SubscriptionAmendment {
            apply_mode,
            component_changes,
            add_on_changes,
        })
    }
}

pub mod coupons {
    use crate::coupons::mapping::coupons as coupon_mapping;
    use common_domain::ids::CouponId;
    use meteroid_grpc::meteroid::api::coupons::v1 as coupon_api;
    use meteroid_grpc::meteroid::api::subscriptions::v1 as api;
    use meteroid_grpc_mapping::shared::mapping::datetime::chrono_to_timestamp;
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
    use meteroid_app::conversions::ProtoConv;
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

pub mod upcoming {
    use crate::invoices::mapping::invoices::{
        domain_coupon_line_item_to_server, domain_invoice_lines_to_server,
        domain_tax_breakdown_to_server,
    };
    use meteroid_app::conversions::ProtoConv;
    use meteroid_grpc::meteroid::api::subscriptions::v1::{
        GetSubscriptionComponentUsageResponse, UpcomingBilledFirst, UpcomingCreditDeduction,
        UpcomingInvoice, UpcomingInvoiceChange, UsageDataPoint,
    };
    use meteroid_store::domain::SubscriptionDetails;
    use meteroid_store::domain::credit_outlook::{
        CreditDeduction, UpcomingChange, UpcomingInvoiceForecast,
    };
    use meteroid_store::domain::portal_credit::recurring_issue_and_due;
    use meteroid_store::services::clients::usage::WindowedUsageData;
    use meteroid_store::services::invoice_lines::invoice_lines::ComputedInvoiceContent;

    /// The issue (charge) and due dates of a recurring invoice billed on `invoice_date`.
    fn issue_and_due(
        invoice_date: chrono::NaiveDate,
        details: &SubscriptionDetails,
    ) -> (String, String) {
        let entity = &details.invoicing_entity;
        let (issued_on, due_on) = recurring_issue_and_due(
            invoice_date,
            details.subscription.net_terms,
            entity.grace_period_hours,
            entity.consolidate_recurring_invoices,
        );
        (issued_on.as_proto(), due_on.as_proto())
    }

    pub fn computed_content_to_upcoming_proto(
        content: ComputedInvoiceContent,
        details: &SubscriptionDetails,
    ) -> UpcomingInvoice {
        let invoice_date = details
            .subscription
            .current_period_end
            .unwrap_or_else(|| chrono::Utc::now().date_naive());
        let period = (details.subscription.current_period_start, invoice_date);
        computed_content_to_dated_proto(content, details, invoice_date, period)
    }

    /// An invoice issued on `invoice_date` for `period`, priced on `details`' terms.
    pub fn computed_content_to_dated_proto(
        content: ComputedInvoiceContent,
        details: &SubscriptionDetails,
        invoice_date: chrono::NaiveDate,
        (period_start, period_end): (chrono::NaiveDate, chrono::NaiveDate),
    ) -> UpcomingInvoice {
        let net_terms = details.subscription.net_terms;
        let (charge_date, due_date) = issue_and_due(invoice_date, details);

        UpcomingInvoice {
            currency: details.subscription.currency.clone(),
            invoice_date: invoice_date.as_proto(),
            line_items: domain_invoice_lines_to_server(content.invoice_lines),
            coupon_line_items: domain_coupon_line_item_to_server(content.applied_coupons),
            tax_breakdown: content
                .tax_breakdown
                .iter()
                .map(domain_tax_breakdown_to_server)
                .collect(),
            subtotal: content.subtotal,
            subtotal_recurring: content.subtotal_recurring,
            tax_amount: content.tax_amount,
            total: content.total,
            amount_due: content.amount_due,
            applied_credits: content.applied_credits,
            discount: content.discount,
            promotional_credits: content.promotional_credits,
            period_start: period_start.as_proto(),
            period_end: period_end.as_proto(),
            plan_name: Some(details.subscription.plan_name.clone()),
            due_date: Some(due_date),
            net_terms: net_terms as i32,
            credit_estimate_shared: false,
            billed_first: Vec::new(),
            credit_deductions: Vec::new(),
            change: UpcomingInvoiceChange::None.into(),
            new_plan_name: None,
            charge_date,
            not_invoiced: false,
        }
    }

    /// The next invoice with the credit it draws once every invoice billed before it drew its own,
    /// dated as the forecast bills it (a cancellation's date for a final invoice).
    pub fn forecast_to_upcoming_proto(
        forecast: UpcomingInvoiceForecast,
        details: &SubscriptionDetails,
        new_plan_name: Option<String>,
    ) -> Result<UpcomingInvoice, tonic::Status> {
        let credit_deductions = credit_deductions(&forecast.credit)?;
        let mut invoice = computed_content_to_upcoming_proto(forecast.content, details);
        let (charge_date, due_date) = issue_and_due(forecast.invoice_date, details);
        invoice.invoice_date = forecast.invoice_date.as_proto();
        invoice.due_date = Some(due_date);
        invoice.charge_date = charge_date;
        invoice.not_invoiced = matches!(
            forecast.change,
            Some(
                UpcomingChange::Ends { billed: false, .. }
                    | UpcomingChange::Pauses { billed: false, .. }
            )
        );
        invoice.change = match forecast.change {
            None => UpcomingInvoiceChange::None,
            Some(UpcomingChange::Ends { .. }) => UpcomingInvoiceChange::Ends,
            Some(UpcomingChange::Pauses { .. }) => UpcomingInvoiceChange::Pause,
            Some(UpcomingChange::PlanChange { .. }) => UpcomingInvoiceChange::PlanChange,
            Some(UpcomingChange::Amendment) => UpcomingInvoiceChange::Amendment,
        }
        .into();
        invoice.new_plan_name = new_plan_name;
        invoice.credit_estimate_shared = forecast.credit_estimate_shared;
        invoice.billed_first = forecast
            .billed_first
            .iter()
            .map(|first| UpcomingBilledFirst {
                invoice_id: first.invoice_id.map(|id| id.as_proto()),
                subscription_id: first.subscription_id.map(|id| id.as_proto()),
                plan_name: first.plan_name.clone(),
                invoice_date: first.invoice_date.as_proto(),
            })
            .collect();
        invoice.credit_deductions = credit_deductions;
        Ok(invoice)
    }

    /// One row per kind of credit and phase (before or after tax), in draw order.
    fn credit_deductions(
        credit: &[CreditDeduction],
    ) -> Result<Vec<UpcomingCreditDeduction>, tonic::Status> {
        let mut rows: Vec<UpcomingCreditDeduction> = Vec::new();
        for deduction in credit {
            let kind: i32 = crate::wallets::mapping::group_kind(deduction.group.kind).into();
            let subscription_id = deduction.group.subscription_id.map(|id| id.as_proto());
            match rows.iter_mut().find(|row| {
                row.kind == kind
                    && row.label == deduction.group.label
                    && row.subscription_id == subscription_id
                    && row.currency == deduction.denomination
                    && row.promotional == deduction.before_tax
            }) {
                Some(row) => {
                    row.amount = row
                        .amount
                        .checked_add(deduction.amount_minor)
                        .ok_or_else(|| tonic::Status::internal("credit total overflowed"))?;
                }
                None => rows.push(UpcomingCreditDeduction {
                    kind,
                    label: deduction.group.label.clone(),
                    subscription_id,
                    currency: deduction.denomination.clone(),
                    amount: deduction.amount_minor,
                    promotional: deduction.before_tax,
                }),
            }
        }
        Ok(rows)
    }

    /// Maps the computed first-invoice content of a create-subscription preview to UpcomingInvoice.
    /// A new subscription is billed in advance, so the invoice date is the period start (today),
    /// unlike `computed_content_to_upcoming_proto` which dates a renewal at the period end.
    pub fn create_preview_to_upcoming_proto(
        content: ComputedInvoiceContent,
        details: &SubscriptionDetails,
    ) -> UpcomingInvoice {
        let invoice_date = details.subscription.current_period_start;
        let net_terms = details.subscription.net_terms;
        let due_date = invoice_date + chrono::Duration::days(net_terms as i64);

        UpcomingInvoice {
            currency: details.subscription.currency.clone(),
            invoice_date: invoice_date.as_proto(),
            line_items: domain_invoice_lines_to_server(content.invoice_lines),
            coupon_line_items: domain_coupon_line_item_to_server(content.applied_coupons),
            tax_breakdown: content
                .tax_breakdown
                .iter()
                .map(domain_tax_breakdown_to_server)
                .collect(),
            subtotal: content.subtotal,
            subtotal_recurring: content.subtotal_recurring,
            tax_amount: content.tax_amount,
            total: content.total,
            amount_due: content.amount_due,
            applied_credits: content.applied_credits,
            discount: content.discount,
            promotional_credits: content.promotional_credits,
            period_start: details.subscription.current_period_start.as_proto(),
            period_end: details
                .subscription
                .current_period_end
                .map(|d| d.as_proto())
                .unwrap_or_default(),
            plan_name: Some(details.subscription.plan_name.clone()),
            due_date: Some(due_date.as_proto()),
            net_terms: net_terms as i32,
            credit_estimate_shared: false,
            billed_first: Vec::new(),
            credit_deductions: Vec::new(),
            change: UpcomingInvoiceChange::None.into(),
            new_plan_name: None,
            charge_date: invoice_date.as_proto(),
            not_invoiced: false,
        }
    }

    pub fn windowed_usage_to_proto(
        usage: WindowedUsageData,
    ) -> GetSubscriptionComponentUsageResponse {
        GetSubscriptionComponentUsageResponse {
            data_points: usage
                .data
                .into_iter()
                .map(|point| UsageDataPoint {
                    window_start: point.window_start.to_string(),
                    window_end: point.window_end.to_string(),
                    value: point.value.to_string(),
                    dimensions: point.dimensions,
                })
                .collect(),
            period_start: usage.period.start.as_proto(),
            period_end: usage.period.end.as_proto(),
        }
    }
}

pub mod usage {
    use meteroid_app::conversions::{AsProtoOpt, ProtoConv};
    use meteroid_grpc::meteroid::api::subscriptions::v1 as server;
    use meteroid_store::domain::customer_summary as domain;

    pub fn usage_to_server(
        value: &domain::SubscriptionUsage,
    ) -> server::GetSubscriptionUsageResponse {
        server::GetSubscriptionUsageResponse {
            cycle: value.cycle.as_ref().map(|c| server::SubscriptionCycle {
                period_start: c.period_start.as_proto(),
                period_end: c.period_end.as_proto(),
                elapsed_pct: c.elapsed_pct,
                days_left: c.days_left,
            }),
            usage: value
                .usage
                .iter()
                .map(|u| server::SubscriptionFeatureUsage {
                    feature_id: u.feature_id.as_proto(),
                    feature_name: u.feature_name.clone(),
                    consumed: u.consumed.as_proto(),
                    limit: u.limit.map(|l| l.as_proto()),
                    pct: u.pct,
                    warning_threshold_pct: u.warning_threshold_pct,
                    reset: u.reset.clone(),
                    period_start: u.period_start.as_proto(),
                    resets_at: u.resets_at.as_proto(),
                    overage_behavior: u.overage_behavior.clone(),
                    projected: u.projected.map(|p| p.as_proto()),
                    refused: u.refused,
                    grace_pct: u.grace_pct,
                    group_by: u.group_by.clone(),
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod credit_unit_fee_tests {
    use super::price_components::subscription_fee_to_grpc;
    use common_domain::ids::{BaseId, BillableMetricId, CreditUnitId};
    use meteroid_store::domain::credit_units::{CreditUnitRef, FeeCreditUnit};
    use meteroid_store::domain::{BillingPeriodEnum, SubscriptionFee, UsagePricingModel};
    use rust_decimal_macros::dec;

    /// A fee priced in credits shows its amounts in the unit, with the rate they bill at.
    #[test]
    fn a_credit_priced_fee_carries_its_unit_and_rate() {
        let unit_id = CreditUnitId::new();
        let usage = SubscriptionFee::Usage {
            metric_id: BillableMetricId::new(),
            model: UsagePricingModel::PerUnit { rate: dec!(3) },
            credit_unit: Some(FeeCreditUnit {
                unit_id,
                code: "CREDITS".to_owned(),
                precision: 2,
                rate: dec!(0.01),
            }),
        };
        let unit = subscription_fee_to_grpc(&usage, BillingPeriodEnum::Monthly)
            .credit_unit
            .expect("the unit");
        assert_eq!(
            (unit.credit_unit_id, unit.code.as_str(), unit.precision),
            (unit_id.to_string(), "CREDITS", 2)
        );
        assert_eq!(unit.rate.as_deref(), Some("0.01"));

        let pack = SubscriptionFee::Credit {
            rate: dec!(40),
            quantity: 1,
            credit_amount: dec!(5000),
            expiry: None,
            credit_unit: Some(CreditUnitRef {
                unit_id,
                code: "CREDITS".to_owned(),
                precision: 2,
                product_id: None,
            }),
            rollover: None,
            once: false,
        };
        let granted = subscription_fee_to_grpc(&pack, BillingPeriodEnum::Monthly)
            .credit_unit
            .expect("the granted unit");
        assert_eq!((granted.code.as_str(), granted.rate), ("CREDITS", None));

        let money = SubscriptionFee::Rate { rate: dec!(10) };
        assert!(
            subscription_fee_to_grpc(&money, BillingPeriodEnum::Monthly)
                .credit_unit
                .is_none()
        );
    }
}
