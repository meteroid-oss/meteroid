pub mod prices {
    use crate::api::domain_mapping::billing_period;
    use crate::api::shared::conversions::ProtoConv;

    use meteroid_grpc::meteroid::api::prices::v1 as proto;
    use meteroid_grpc::meteroid::api::shared::v1 as api_shared;
    use meteroid_store::domain;
    use meteroid_store::domain::price_components::{
        MatrixDimension, MatrixRow, TierRow, UsagePricingModel,
    };
    use meteroid_store::domain::prices::Pricing;
    use rust_decimal::Decimal;
    use tonic::Status;

    pub struct PriceWrapper(pub proto::Price);

    impl From<domain::Price> for PriceWrapper {
        fn from(price: domain::Price) -> Self {
            PriceWrapper(proto::Price {
                id: price.id.as_proto(),
                product_id: price.product_id.as_proto(),
                cadence: billing_period::to_proto(price.cadence).into(),
                currency: price.currency,
                pricing: pricing_to_proto(&price.pricing),
                created_at: Some(price.created_at.as_proto()),
                archived_at: price.archived_at.map(|d| d.as_proto()),
            })
        }
    }

    pub fn pricing_to_proto(pricing: &Pricing) -> Option<proto::price::Pricing> {
        match pricing {
            Pricing::Rate { rate } => {
                Some(proto::price::Pricing::RatePricing(proto::RatePricing {
                    rate: rate.as_proto(),
                }))
            }
            Pricing::Slot {
                unit_rate,
                min_slots,
                max_slots,
            } => Some(proto::price::Pricing::SlotPricing(proto::SlotPricing {
                unit_rate: unit_rate.as_proto(),
                min_slots: *min_slots,
                max_slots: *max_slots,
            })),
            Pricing::Capacity {
                rate,
                included,
                overage_rate,
            } => Some(proto::price::Pricing::CapacityPricing(
                proto::CapacityPricing {
                    rate: rate.as_proto(),
                    included: *included,
                    overage_rate: overage_rate.as_proto(),
                },
            )),
            Pricing::Usage(model) => Some(proto::price::Pricing::UsagePricing(
                usage_model_to_proto(model),
            )),
            Pricing::ExtraRecurring {
                unit_price,
                quantity,
            } => Some(proto::price::Pricing::ExtraRecurringPricing(
                proto::ExtraRecurringPricing {
                    unit_price: unit_price.as_proto(),
                    quantity: *quantity,
                },
            )),
            Pricing::OneTime {
                unit_price,
                quantity,
            } => Some(proto::price::Pricing::OneTimePricing(
                proto::OneTimePricing {
                    unit_price: unit_price.as_proto(),
                    quantity: *quantity,
                },
            )),
        }
    }

    pub fn usage_model_to_proto(model: &UsagePricingModel) -> proto::UsagePricing {
        let model_oneof = match model {
            UsagePricingModel::PerUnit { rate } => {
                proto::usage_pricing::Model::PerUnit(rate.as_proto())
            }
            UsagePricingModel::Tiered { tiers, block_size } => {
                proto::usage_pricing::Model::Tiered(proto::usage_pricing::TieredAndVolumePricing {
                    rows: tiers.iter().map(tier_row_to_proto).collect(),
                    block_size: *block_size,
                })
            }
            UsagePricingModel::Volume { tiers, block_size } => {
                proto::usage_pricing::Model::Volume(proto::usage_pricing::TieredAndVolumePricing {
                    rows: tiers.iter().map(tier_row_to_proto).collect(),
                    block_size: *block_size,
                })
            }
            UsagePricingModel::Package { block_size, rate } => {
                proto::usage_pricing::Model::Package(proto::usage_pricing::PackagePricing {
                    package_price: rate.as_proto(),
                    block_size: *block_size,
                })
            }
            UsagePricingModel::Matrix { rates } => {
                proto::usage_pricing::Model::Matrix(proto::usage_pricing::MatrixPricing {
                    rows: rates.iter().map(matrix_row_to_proto).collect(),
                })
            }
        };
        proto::UsagePricing {
            model: Some(model_oneof),
        }
    }

    pub fn tier_row_to_proto(
        tier: &TierRow,
    ) -> proto::usage_pricing::tiered_and_volume_pricing::TierRow {
        proto::usage_pricing::tiered_and_volume_pricing::TierRow {
            first_unit: tier.first_unit,
            unit_price: tier.rate.as_proto(),
            flat_fee: tier.flat_fee.map(|f| f.as_proto()),
            flat_cap: tier.flat_cap.map(|c| c.as_proto()),
        }
    }

    pub fn matrix_row_to_proto(row: &MatrixRow) -> proto::usage_pricing::matrix_pricing::MatrixRow {
        proto::usage_pricing::matrix_pricing::MatrixRow {
            per_unit_price: row.per_unit_price.as_proto(),
            dimension1: Some(proto::usage_pricing::matrix_pricing::MatrixDimension {
                key: row.dimension1.key.clone(),
                value: row.dimension1.value.clone(),
            }),
            dimension2: row.dimension2.as_ref().map(|d| {
                proto::usage_pricing::matrix_pricing::MatrixDimension {
                    key: d.key.clone(),
                    value: d.value.clone(),
                }
            }),
        }
    }

    pub fn pricing_from_proto(
        pricing_oneof: Option<meteroid_grpc::meteroid::api::components::v1::price_input::Pricing>,
    ) -> Result<Pricing, Status> {
        use meteroid_grpc::meteroid::api::components::v1::price_input::Pricing as P;
        match pricing_oneof {
            Some(P::RatePricing(p)) => Ok(Pricing::Rate {
                rate: Decimal::from_proto_ref(&p.rate)?,
            }),
            Some(P::SlotPricing(p)) => Ok(Pricing::Slot {
                unit_rate: Decimal::from_proto_ref(&p.unit_rate)?,
                min_slots: p.min_slots,
                max_slots: p.max_slots,
            }),
            Some(P::CapacityPricing(p)) => Ok(Pricing::Capacity {
                rate: Decimal::from_proto_ref(&p.rate)?,
                included: p.included,
                overage_rate: Decimal::from_proto_ref(&p.overage_rate)?,
            }),
            Some(P::UsagePricing(p)) => {
                let model = usage_model_from_proto(&p)?;
                Ok(Pricing::Usage(model))
            }
            Some(P::ExtraRecurringPricing(p)) => Ok(Pricing::ExtraRecurring {
                unit_price: Decimal::from_proto_ref(&p.unit_price)?,
                quantity: p.quantity,
            }),
            Some(P::OneTimePricing(p)) => Ok(Pricing::OneTime {
                unit_price: Decimal::from_proto_ref(&p.unit_price)?,
                quantity: p.quantity,
            }),
            None => Err(Status::invalid_argument("pricing is required")),
        }
    }

    pub fn usage_model_from_proto(
        usage: &proto::UsagePricing,
    ) -> Result<UsagePricingModel, Status> {
        match usage.model.as_ref() {
            Some(proto::usage_pricing::Model::PerUnit(rate_str)) => {
                let rate = Decimal::from_proto_ref(rate_str)?;
                Ok(UsagePricingModel::PerUnit { rate })
            }
            Some(proto::usage_pricing::Model::Tiered(tiered)) => {
                let tiers = tiered
                    .rows
                    .iter()
                    .map(tier_row_from_proto)
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(UsagePricingModel::Tiered {
                    tiers,
                    block_size: tiered.block_size,
                })
            }
            Some(proto::usage_pricing::Model::Volume(volume)) => {
                let tiers = volume
                    .rows
                    .iter()
                    .map(tier_row_from_proto)
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(UsagePricingModel::Volume {
                    tiers,
                    block_size: volume.block_size,
                })
            }
            Some(proto::usage_pricing::Model::Package(package)) => {
                let rate = Decimal::from_proto_ref(&package.package_price)?;
                Ok(UsagePricingModel::Package {
                    block_size: package.block_size,
                    rate,
                })
            }
            Some(proto::usage_pricing::Model::Matrix(matrix)) => {
                let rates = matrix
                    .rows
                    .iter()
                    .map(matrix_row_from_proto)
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(UsagePricingModel::Matrix { rates })
            }
            None => Err(Status::invalid_argument("usage pricing model is required")),
        }
    }

    pub fn tier_row_from_proto(
        tier: &proto::usage_pricing::tiered_and_volume_pricing::TierRow,
    ) -> Result<TierRow, Status> {
        let rate = Decimal::from_proto_ref(&tier.unit_price)?;
        let flat_fee = tier
            .flat_fee
            .as_ref()
            .map(Decimal::from_proto_ref)
            .transpose()?;
        let flat_cap = tier
            .flat_cap
            .as_ref()
            .map(Decimal::from_proto_ref)
            .transpose()?;
        Ok(TierRow {
            first_unit: tier.first_unit,
            rate,
            flat_fee,
            flat_cap,
        })
    }

    pub fn matrix_row_from_proto(
        row: &proto::usage_pricing::matrix_pricing::MatrixRow,
    ) -> Result<MatrixRow, Status> {
        let dimension1 = row
            .dimension1
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("Missing dimension1"))?;

        Ok(MatrixRow {
            dimension1: MatrixDimension {
                key: dimension1.key.clone(),
                value: dimension1.value.clone(),
            },
            dimension2: row.dimension2.as_ref().map(|d| MatrixDimension {
                key: d.key.clone(),
                value: d.value.clone(),
            }),
            per_unit_price: Decimal::from_proto_ref(&row.per_unit_price)?,
        })
    }

    pub fn cadence_from_proto(cadence: i32) -> Result<domain::enums::BillingPeriodEnum, Status> {
        let period = api_shared::BillingPeriod::try_from(cadence)
            .map_err(|_| Status::invalid_argument(format!("Invalid cadence value: {cadence}")))?;
        Ok(billing_period::from_proto(period))
    }

    pub fn dimension_key_from_proto(
        key: &proto::MatrixDimensionKey,
    ) -> Result<domain::prices::MatrixDimensionKey, Status> {
        let dimension1 = key
            .dimension1
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("Missing dimension1"))?;

        Ok(domain::prices::MatrixDimensionKey {
            dimension1: MatrixDimension {
                key: dimension1.key.clone(),
                value: dimension1.value.clone(),
            },
            dimension2: key.dimension2.as_ref().map(|d| MatrixDimension {
                key: d.key.clone(),
                value: d.value.clone(),
            }),
        })
    }

    pub fn matrix_row_add_from_proto(
        row: &proto::MatrixRowAdd,
    ) -> Result<domain::prices::MatrixRowAdd, Status> {
        let dimension1 = row
            .dimension1
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("Missing dimension1"))?;

        let per_unit_prices = row
            .per_unit_prices
            .iter()
            .map(|(currency, price_str)| {
                let price = Decimal::from_proto_ref(price_str)?;
                Ok((currency.clone(), price))
            })
            .collect::<Result<std::collections::HashMap<_, _>, Status>>()?;

        Ok(domain::prices::MatrixRowAdd {
            dimension1: MatrixDimension {
                key: dimension1.key.clone(),
                value: dimension1.value.clone(),
            },
            dimension2: row.dimension2.as_ref().map(|d| MatrixDimension {
                key: d.key.clone(),
                value: d.value.clone(),
            }),
            per_unit_prices,
        })
    }

    pub fn matrix_price_update_from_proto(
        req: &proto::UpdateMatrixPricesRequest,
    ) -> Result<domain::prices::MatrixPriceUpdate, Status> {
        let add_rows = req
            .add_rows
            .iter()
            .map(matrix_row_add_from_proto)
            .collect::<Result<Vec<_>, Status>>()?;

        let remove_rows = req
            .remove_rows
            .iter()
            .map(dimension_key_from_proto)
            .collect::<Result<Vec<_>, Status>>()?;

        Ok(domain::prices::MatrixPriceUpdate {
            add_rows,
            remove_rows,
        })
    }

    pub fn matrix_preview_from_proto(
        req: &proto::PreviewMatrixUpdateRequest,
    ) -> Result<domain::prices::MatrixPriceUpdate, Status> {
        let add_rows = req
            .add_rows
            .iter()
            .map(|key| {
                let dk = dimension_key_from_proto(key)?;
                Ok(domain::prices::MatrixRowAdd {
                    dimension1: dk.dimension1,
                    dimension2: dk.dimension2,
                    per_unit_prices: std::collections::HashMap::new(),
                })
            })
            .collect::<Result<Vec<_>, Status>>()?;

        let remove_rows = req
            .remove_rows
            .iter()
            .map(dimension_key_from_proto)
            .collect::<Result<Vec<_>, Status>>()?;

        Ok(domain::prices::MatrixPriceUpdate {
            add_rows,
            remove_rows,
        })
    }

    pub fn matrix_update_preview_to_proto(
        preview: domain::prices::MatrixUpdatePreview,
    ) -> proto::PreviewMatrixUpdateResponse {
        proto::PreviewMatrixUpdateResponse {
            affected_prices: preview.affected_prices_count as u32,
            affected_subscriptions: preview.affected_subscriptions_count as u32,
            rows_to_add: preview.rows_to_add as u32,
            rows_to_remove: preview.rows_to_remove as u32,
            affected_plans: preview
                .affected_plans
                .into_iter()
                .map(|p| proto::AffectedPlan {
                    plan_name: p.plan_name,
                    versions: p.versions,
                })
                .collect(),
        }
    }
}
