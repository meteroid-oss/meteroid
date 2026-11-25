pub mod prices {
    use crate::domain_mapping::billing_period;
    use meteroid_app::conversions::ProtoConv;

    use meteroid_grpc::meteroid::api::prices::v1 as proto;
    use meteroid_grpc::meteroid::api::ratecards::v1::RateCardUnmatched as ProtoUnmatched;
    use meteroid_grpc::meteroid::api::shared::v1 as api_shared;
    use meteroid_store::domain;
    use meteroid_store::domain::price_components::{
        CreditExpiry, CreditRollover, DimensionRate, DimensionRates, MatrixDimension, TierRow,
        UsagePricingModel,
    };
    use meteroid_store::domain::prices::Pricing;
    use meteroid_store::domain::rate_cards::RateCardUnmatched;
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
                credit_unit: price.credit_unit.map(|unit| proto::PriceCreditUnit {
                    credit_unit_id: unit.unit_id.as_proto(),
                    rate: unit.rate.as_proto(),
                }),
            })
        }
    }

    pub fn price_credit_unit_from_proto(
        unit: Option<proto::PriceCreditUnit>,
    ) -> Result<Option<domain::credit_units::PriceCreditUnit>, Status> {
        unit.map(|unit| {
            Ok(domain::credit_units::PriceCreditUnit {
                unit_id: common_domain::ids::CreditUnitId::from_proto(&unit.credit_unit_id)?,
                rate: Decimal::from_proto_ref(&unit.rate)?,
            })
        })
        .transpose()
    }

    pub fn pricing_to_proto(pricing: &Pricing) -> Option<proto::price::Pricing> {
        match pricing {
            Pricing::Rate { rate } => {
                Some(proto::price::Pricing::RatePricing(proto::RatePricing {
                    rate: rate.as_proto(),
                }))
            }
            Pricing::Slot {
                model,
                min_slots,
                max_slots,
            } => Some(proto::price::Pricing::SlotPricing(proto::SlotPricing {
                unit_rate: model
                    .per_unit_rate()
                    .unwrap_or_else(|| model.headline_rate())
                    .as_proto(),
                min_slots: *min_slots,
                max_slots: *max_slots,
                model: rating_model_to_proto(model),
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
                model,
            } => Some(proto::price::Pricing::ExtraRecurringPricing(
                proto::ExtraRecurringPricing {
                    unit_price: unit_price.as_proto(),
                    quantity: *quantity,
                    model: fixed_quantity_model_to_proto(model.as_ref()),
                },
            )),
            Pricing::OneTime {
                unit_price,
                quantity,
                model,
            } => Some(proto::price::Pricing::OneTimePricing(
                proto::OneTimePricing {
                    unit_price: unit_price.as_proto(),
                    quantity: *quantity,
                    model: fixed_quantity_model_to_proto(model.as_ref()),
                },
            )),
            Pricing::Credit {
                unit_price,
                quantity,
                credit_amount,
                expiry,
                rollover,
                credit_unit_id,
                once,
            } => Some(proto::price::Pricing::CreditPricing(proto::CreditPricing {
                unit_price: unit_price.as_proto(),
                quantity: *quantity,
                credit_amount: credit_amount.as_proto(),
                expiry: expiry.map(credit_expiry_to_proto),
                rollover: rollover.as_ref().map(credit_rollover_to_proto),
                credit_unit_id: credit_unit_id.map(|id| id.as_proto()),
                once: *once,
            })),
        }
    }

    pub fn credit_expiry_to_proto(expiry: CreditExpiry) -> proto::CreditPackExpiry {
        use proto::credit_pack_expiry::Unit;
        let (unit, value) = match expiry {
            CreditExpiry::Day(days) => (Unit::Day, days),
            CreditExpiry::Month(months) => (Unit::Month, months),
            CreditExpiry::BillingPeriod => (Unit::BillingPeriod, 1),
        };
        proto::CreditPackExpiry {
            unit: unit.into(),
            value,
        }
    }

    pub fn credit_rollover_to_proto(rollover: &CreditRollover) -> proto::CreditPackRollover {
        proto::CreditPackRollover {
            max_amount: rollover.max_amount.map(|max| max.as_proto()),
            expiry: Some(credit_expiry_to_proto(rollover.expiry)),
        }
    }

    pub fn credit_rollover_from_proto(
        rollover: &proto::CreditPackRollover,
    ) -> Result<CreditRollover, Status> {
        Ok(CreditRollover {
            max_amount: rollover
                .max_amount
                .as_ref()
                .map(Decimal::from_proto_ref)
                .transpose()?,
            expiry: credit_expiry_from_proto(rollover.expiry.as_ref().ok_or_else(|| {
                Status::invalid_argument("a rollover needs the term of the rolled credit")
            })?)?,
        })
    }

    pub fn credit_expiry_from_proto(
        expiry: &proto::CreditPackExpiry,
    ) -> Result<CreditExpiry, Status> {
        use proto::credit_pack_expiry::Unit;
        match Unit::try_from(expiry.unit) {
            Ok(Unit::Day) if expiry.value > 0 => Ok(CreditExpiry::Day(expiry.value)),
            Ok(Unit::Month) if expiry.value > 0 => Ok(CreditExpiry::Month(expiry.value)),
            Ok(Unit::BillingPeriod) => Ok(CreditExpiry::BillingPeriod),
            _ => Err(Status::invalid_argument(
                "a credit pack expiry is a positive number of days or months, or the billing period",
            )),
        }
    }

    /// A per-unit model travels as its rate alone; any other in `model`.
    pub fn rating_model_to_proto(
        model: &meteroid_store::domain::price_components::RatingModel,
    ) -> Option<proto::UsagePricing> {
        model
            .per_unit_rate()
            .is_none()
            .then(|| usage_model_to_proto(&model.as_usage_model()))
    }

    pub fn rating_model_from_proto(
        model: Option<&proto::UsagePricing>,
        unit_rate: &String,
    ) -> Result<meteroid_store::domain::price_components::RatingModel, Status> {
        use meteroid_store::domain::price_components::RatingModel;
        match model {
            Some(model) => RatingModel::from_usage_model(usage_model_from_proto(model)?)
                .map_err(|e| Status::invalid_argument(e.to_string())),
            None => Ok(RatingModel::PerUnit {
                rate: Decimal::from_proto_ref(unit_rate)?,
            }),
        }
    }

    /// The model of an extra recurring or one-time price, when it has one.
    pub fn fixed_quantity_model_to_proto(
        model: Option<&meteroid_store::domain::price_components::RatingModel>,
    ) -> Option<proto::UsagePricing> {
        model.map(|model| usage_model_to_proto(&model.as_usage_model()))
    }

    /// The unit price and model of an extra recurring or one-time price. A per-unit model is
    /// kept as the unit price; any other leaves its first rate there, for readers without models.
    pub fn fixed_quantity_model_from_proto(
        model: Option<&proto::UsagePricing>,
        unit_price: &String,
    ) -> Result<
        (
            Decimal,
            Option<meteroid_store::domain::price_components::RatingModel>,
        ),
        Status,
    > {
        use meteroid_store::domain::price_components::RatingModel;
        let Some(model) = model else {
            return Ok((Decimal::from_proto_ref(unit_price)?, None));
        };
        match RatingModel::from_usage_model(usage_model_from_proto(model)?)
            .map_err(|e| Status::invalid_argument(e.to_string()))?
        {
            RatingModel::PerUnit { rate } => Ok((rate, None)),
            model => Ok((model.headline_rate(), Some(model))),
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
            UsagePricingModel::DimensionRates(rates) => {
                proto::usage_pricing::Model::DimensionRates(dimension_rates_to_proto(rates))
            }
            UsagePricingModel::RateCard {
                rate_card_id,
                multiplier,
            } => proto::usage_pricing::Model::RateCard(proto::usage_pricing::RateCardPricing {
                rate_card_id: rate_card_id.as_proto(),
                multiplier: multiplier.map(|m| m.as_proto()),
            }),
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

    pub fn dimension_rates_to_proto(
        rates: &DimensionRates,
    ) -> proto::usage_pricing::DimensionRatesPricing {
        proto::usage_pricing::DimensionRatesPricing {
            dimensions: rates.dimensions.clone(),
            rates: rates
                .rates
                .iter()
                .map(
                    |rate| proto::usage_pricing::dimension_rates_pricing::DimensionRate {
                        key: rate.key.clone(),
                        rate: rate.rate.as_proto(),
                        label: rate.label.clone(),
                    },
                )
                .collect(),
            unit_size: rates.unit_size.as_proto(),
            on_unmatched: match rates.on_unmatched {
                RateCardUnmatched::Hold => ProtoUnmatched::Hold,
                RateCardUnmatched::Ignore => ProtoUnmatched::Ignore,
            }
            .into(),
        }
    }

    pub fn dimension_rates_from_proto(
        rates: &proto::usage_pricing::DimensionRatesPricing,
    ) -> Result<DimensionRates, Status> {
        let unit_size = match rates.unit_size.trim() {
            "" => Decimal::ONE,
            size => Decimal::from_proto_ref(&size.to_owned())?,
        };
        let on_unmatched = match ProtoUnmatched::try_from(rates.on_unmatched) {
            Ok(ProtoUnmatched::Hold) => RateCardUnmatched::Hold,
            Ok(ProtoUnmatched::Ignore) => RateCardUnmatched::Ignore,
            Err(_) => return Err(Status::invalid_argument("unknown unmatched policy")),
        };
        Ok(DimensionRates {
            dimensions: rates.dimensions.clone(),
            rates: rates
                .rates
                .iter()
                .map(|rate| {
                    Ok(DimensionRate {
                        key: rate.key.clone(),
                        rate: Decimal::from_proto_ref(&rate.rate)?,
                        label: rate.label.clone().filter(|label| !label.trim().is_empty()),
                    })
                })
                .collect::<Result<Vec<_>, Status>>()?,
            unit_size,
            on_unmatched,
        })
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
                model: rating_model_from_proto(p.model.as_ref(), &p.unit_rate)?,
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
            Some(P::ExtraRecurringPricing(p)) => {
                let (unit_price, model) =
                    fixed_quantity_model_from_proto(p.model.as_ref(), &p.unit_price)?;
                Ok(Pricing::ExtraRecurring {
                    unit_price,
                    quantity: p.quantity,
                    model,
                })
            }
            Some(P::OneTimePricing(p)) => {
                let (unit_price, model) =
                    fixed_quantity_model_from_proto(p.model.as_ref(), &p.unit_price)?;
                Ok(Pricing::OneTime {
                    unit_price,
                    quantity: p.quantity,
                    model,
                })
            }
            Some(P::CreditPricing(p)) => Ok(Pricing::Credit {
                unit_price: Decimal::from_proto_ref(&p.unit_price)?,
                quantity: p.quantity,
                credit_amount: Decimal::from_proto_ref(&p.credit_amount)?,
                expiry: p
                    .expiry
                    .as_ref()
                    .map(credit_expiry_from_proto)
                    .transpose()?,
                credit_unit_id: p
                    .credit_unit_id
                    .as_deref()
                    .filter(|id| !id.is_empty())
                    .map(common_domain::ids::CreditUnitId::from_proto)
                    .transpose()?,
                rollover: p
                    .rollover
                    .as_ref()
                    .map(credit_rollover_from_proto)
                    .transpose()?,
                once: p.once,
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
            Some(proto::usage_pricing::Model::RateCard(card)) => Ok(UsagePricingModel::RateCard {
                rate_card_id: common_domain::ids::RateCardId::from_proto(&card.rate_card_id)?,
                multiplier: card
                    .multiplier
                    .as_ref()
                    .map(Decimal::from_proto_ref)
                    .transpose()?,
            }),
            Some(proto::usage_pricing::Model::DimensionRates(rates)) => Ok(
                UsagePricingModel::DimensionRates(dimension_rates_from_proto(rates)?),
            ),
            // Deprecated: a matrix is read as its rates by dimension.
            Some(proto::usage_pricing::Model::Matrix(matrix)) => {
                let cells = matrix
                    .rows
                    .iter()
                    .map(matrix_row_from_proto)
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(UsagePricingModel::DimensionRates(
                    DimensionRates::from_matrix_cells(cells),
                ))
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

    /// A deprecated matrix row: its cell's dimensions and its rate.
    pub fn matrix_row_from_proto(
        row: &proto::usage_pricing::matrix_pricing::MatrixRow,
    ) -> Result<(MatrixDimension, Option<MatrixDimension>, Decimal), Status> {
        let dimension1 = row
            .dimension1
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("Missing dimension1"))?;

        Ok((
            MatrixDimension {
                key: dimension1.key.clone(),
                value: dimension1.value.clone(),
            },
            row.dimension2.as_ref().map(|d| MatrixDimension {
                key: d.key.clone(),
                value: d.value.clone(),
            }),
            Decimal::from_proto_ref(&row.per_unit_price)?,
        ))
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
            effective_on: req
                .effective_on
                .map(|date| {
                    meteroid_grpc_mapping::shared::mapping::date::chrono_from_proto(date)
                        .ok_or_else(|| Status::invalid_argument("effective_on is not a valid date"))
                })
                .transpose()?,
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
            effective_on: None,
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

#[cfg(test)]
mod tests {
    use super::prices::{usage_model_from_proto, usage_model_to_proto};
    use meteroid_grpc::meteroid::api::prices::v1 as proto;
    use meteroid_store::domain::price_components::UsagePricingModel;
    use rust_decimal_macros::dec;

    /// A matrix sent by an older client is read as rates by dimension, returned as such.
    #[test]
    fn a_matrix_input_reads_as_rates_by_dimension() {
        let dimension =
            |key: &str, value: &str| proto::usage_pricing::matrix_pricing::MatrixDimension {
                key: key.to_owned(),
                value: value.to_owned(),
            };
        let input = proto::UsagePricing {
            model: Some(proto::usage_pricing::Model::Matrix(
                proto::usage_pricing::MatrixPricing {
                    rows: vec![
                        proto::usage_pricing::matrix_pricing::MatrixRow {
                            per_unit_price: "0.2".to_owned(),
                            dimension1: Some(dimension("region", "eu")),
                            dimension2: Some(dimension("tier", "gold")),
                        },
                        proto::usage_pricing::matrix_pricing::MatrixRow {
                            per_unit_price: "0.1".to_owned(),
                            dimension1: Some(dimension("region", "us")),
                            dimension2: None,
                        },
                    ],
                },
            )),
        };
        let UsagePricingModel::DimensionRates(rates) = usage_model_from_proto(&input).unwrap()
        else {
            panic!("rates by dimension");
        };
        assert_eq!(rates.dimensions, vec!["region", "tier"]);
        assert_eq!(rates.rates[1].key, vec!["us", "*"]);
        assert_eq!(rates.rates[0].rate, dec!(0.2));

        let Some(proto::usage_pricing::Model::DimensionRates(output)) =
            usage_model_to_proto(&UsagePricingModel::DimensionRates(rates.clone())).model
        else {
            panic!("returned as rates by dimension");
        };
        assert_eq!(output.unit_size, "1");
        assert_eq!(
            output.on_unmatched,
            meteroid_grpc::meteroid::api::ratecards::v1::RateCardUnmatched::Ignore as i32
        );
        let read = usage_model_from_proto(&proto::UsagePricing {
            model: Some(proto::usage_pricing::Model::DimensionRates(output)),
        })
        .unwrap();
        assert!(matches!(read, UsagePricingModel::DimensionRates(back) if back == rates));
    }
}
