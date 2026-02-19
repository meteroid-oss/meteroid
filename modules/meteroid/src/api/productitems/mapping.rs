pub mod products {
    use crate::api::prices::mapping::prices::PriceWrapper;
    use crate::api::shared::mapping::datetime::chrono_to_timestamp;
    use common_domain::ids::BillableMetricId;
    use meteroid_grpc::meteroid::api::prices::v1 as prices_proto;
    use meteroid_grpc::meteroid::api::products::v1::{Product, ProductMeta, ProductWithPrice};
    use meteroid_store::domain;
    use meteroid_store::domain::ProductWithLatestPrice;
    use meteroid_store::domain::enums::FeeTypeEnum;
    use meteroid_store::domain::price_components::{DowngradePolicy, UpgradePolicy};
    use meteroid_store::domain::prices::{FeeStructure, UsageModel};
    use tonic::Status;

    pub struct ProductWrapper(pub Product);

    impl From<domain::Product> for ProductWrapper {
        fn from(product: domain::Product) -> Self {
            ProductWrapper(Product {
                id: product.id.as_proto(),
                local_id: product.id.as_proto(),
                name: product.name,
                description: product.description,
                created_at: Some(chrono_to_timestamp(product.created_at)),
                fee_type: Some(fee_type_to_proto(product.fee_type)),
                fee_structure: Some(fee_structure_to_proto(product.fee_structure)),
            })
        }
    }

    pub struct ProductMetaWrapper(pub ProductMeta);
    impl From<domain::Product> for ProductMetaWrapper {
        fn from(product: domain::Product) -> Self {
            ProductMetaWrapper(ProductMeta {
                id: product.id.as_proto(),
                local_id: product.id.as_proto(),
                name: product.name,
                fee_type: Some(fee_type_to_proto(product.fee_type)),
            })
        }
    }

    pub struct ProductWithPriceWrapper(pub ProductWithPrice);
    impl From<ProductWithLatestPrice> for ProductWithPriceWrapper {
        fn from(pwp: ProductWithLatestPrice) -> Self {
            ProductWithPriceWrapper(ProductWithPrice {
                product: Some(ProductWrapper::from(pwp.product).0),
                latest_price: pwp.latest_price.map(|p| PriceWrapper::from(p).0),
            })
        }
    }

    pub fn fee_type_to_proto(ft: FeeTypeEnum) -> i32 {
        match ft {
            FeeTypeEnum::Rate => prices_proto::FeeType::Rate.into(),
            FeeTypeEnum::Slot => prices_proto::FeeType::Slot.into(),
            FeeTypeEnum::Capacity => prices_proto::FeeType::Capacity.into(),
            FeeTypeEnum::Usage => prices_proto::FeeType::Usage.into(),
            FeeTypeEnum::ExtraRecurring => prices_proto::FeeType::ExtraRecurring.into(),
            FeeTypeEnum::OneTime => prices_proto::FeeType::OneTime.into(),
        }
    }

    pub fn fee_type_from_proto(value: i32) -> Result<FeeTypeEnum, Status> {
        match prices_proto::FeeType::try_from(value) {
            Ok(prices_proto::FeeType::Rate) => Ok(FeeTypeEnum::Rate),
            Ok(prices_proto::FeeType::Slot) => Ok(FeeTypeEnum::Slot),
            Ok(prices_proto::FeeType::Capacity) => Ok(FeeTypeEnum::Capacity),
            Ok(prices_proto::FeeType::Usage) => Ok(FeeTypeEnum::Usage),
            Ok(prices_proto::FeeType::ExtraRecurring) => Ok(FeeTypeEnum::ExtraRecurring),
            Ok(prices_proto::FeeType::OneTime) => Ok(FeeTypeEnum::OneTime),
            Err(_) => Err(Status::invalid_argument(format!(
                "Invalid fee_type value: {value}"
            ))),
        }
    }

    pub fn fee_structure_to_proto(fs: FeeStructure) -> prices_proto::FeeStructure {
        use prices_proto::fee_structure::*;
        let structure = match fs {
            FeeStructure::Rate {} => Structure::Rate(RateStructure {}),
            FeeStructure::Slot {
                unit_name,
                upgrade_policy,
                downgrade_policy,
            } => Structure::Slot(SlotStructure {
                unit_name,
                upgrade_policy: upgrade_policy_to_proto(&upgrade_policy).into(),
                downgrade_policy: downgrade_policy_to_proto(&downgrade_policy).into(),
            }),
            FeeStructure::Capacity { metric_id } => Structure::Capacity(CapacityStructure {
                metric_id: metric_id.as_proto(),
            }),
            FeeStructure::Usage { metric_id, model } => Structure::Usage(UsageStructure {
                metric_id: metric_id.as_proto(),
                model: usage_model_to_proto(&model).into(),
            }),
            FeeStructure::ExtraRecurring { billing_type } => {
                Structure::ExtraRecurring(ExtraRecurringStructure {
                    billing_type: billing_type_to_proto(&billing_type).into(),
                })
            }
            FeeStructure::OneTime {} => Structure::OneTime(OneTimeStructure {}),
        };
        prices_proto::FeeStructure {
            structure: Some(structure),
        }
    }

    pub fn fee_structure_from_proto(
        fs: prices_proto::FeeStructure,
    ) -> Result<FeeStructure, Status> {
        use prices_proto::fee_structure::*;
        match fs.structure {
            Some(Structure::Rate(_)) => Ok(FeeStructure::Rate {}),
            Some(Structure::Slot(s)) => Ok(FeeStructure::Slot {
                unit_name: s.unit_name,
                upgrade_policy: upgrade_policy_from_proto(s.upgrade_policy),
                downgrade_policy: downgrade_policy_from_proto(s.downgrade_policy),
            }),
            Some(Structure::Capacity(c)) => Ok(FeeStructure::Capacity {
                metric_id: BillableMetricId::from_proto(c.metric_id)?,
            }),
            Some(Structure::Usage(u)) => {
                let model = usage_model_from_proto(u.model)?;
                Ok(FeeStructure::Usage {
                    metric_id: BillableMetricId::from_proto(u.metric_id)?,
                    model,
                })
            }
            Some(Structure::ExtraRecurring(e)) => {
                let billing_type = billing_type_from_proto(e.billing_type)?;
                Ok(FeeStructure::ExtraRecurring { billing_type })
            }
            Some(Structure::OneTime(_)) => Ok(FeeStructure::OneTime {}),
            None => Err(Status::invalid_argument("fee_structure is required")),
        }
    }

    fn usage_model_to_proto(model: &UsageModel) -> prices_proto::fee_structure::UsageModel {
        match model {
            UsageModel::PerUnit => prices_proto::fee_structure::UsageModel::PerUnit,
            UsageModel::Tiered => prices_proto::fee_structure::UsageModel::Tiered,
            UsageModel::Volume => prices_proto::fee_structure::UsageModel::Volume,
            UsageModel::Package => prices_proto::fee_structure::UsageModel::Package,
            UsageModel::Matrix { .. } => prices_proto::fee_structure::UsageModel::Matrix,
        }
    }

    fn usage_model_from_proto(value: i32) -> Result<UsageModel, Status> {
        match prices_proto::fee_structure::UsageModel::try_from(value) {
            Ok(prices_proto::fee_structure::UsageModel::PerUnit) => Ok(UsageModel::PerUnit),
            Ok(prices_proto::fee_structure::UsageModel::Tiered) => Ok(UsageModel::Tiered),
            Ok(prices_proto::fee_structure::UsageModel::Volume) => Ok(UsageModel::Volume),
            Ok(prices_proto::fee_structure::UsageModel::Package) => Ok(UsageModel::Package),
            Ok(prices_proto::fee_structure::UsageModel::Matrix) => Ok(UsageModel::Matrix),
            Err(_) => Err(Status::invalid_argument(format!(
                "Invalid usage model value: {value}"
            ))),
        }
    }

    fn billing_type_to_proto(
        bt: &domain::enums::BillingType,
    ) -> prices_proto::fee_structure::BillingType {
        match bt {
            domain::enums::BillingType::Arrears => {
                prices_proto::fee_structure::BillingType::Arrear
            }
            domain::enums::BillingType::Advance => {
                prices_proto::fee_structure::BillingType::Advance
            }
        }
    }

    fn billing_type_from_proto(value: i32) -> Result<domain::enums::BillingType, Status> {
        match prices_proto::fee_structure::BillingType::try_from(value) {
            Ok(prices_proto::fee_structure::BillingType::Arrear) => {
                Ok(domain::enums::BillingType::Arrears)
            }
            Ok(prices_proto::fee_structure::BillingType::Advance) => {
                Ok(domain::enums::BillingType::Advance)
            }
            Err(_) => Err(Status::invalid_argument(format!(
                "Invalid billing_type value: {value}"
            ))),
        }
    }

    fn upgrade_policy_to_proto(
        p: &UpgradePolicy,
    ) -> prices_proto::fee_structure::UpgradePolicy {
        match p {
            UpgradePolicy::Prorated => prices_proto::fee_structure::UpgradePolicy::Prorated,
        }
    }

    fn upgrade_policy_from_proto(value: i32) -> UpgradePolicy {
        // Only one variant for now
        let _ = value;
        UpgradePolicy::Prorated
    }

    fn downgrade_policy_to_proto(
        p: &DowngradePolicy,
    ) -> prices_proto::fee_structure::DowngradePolicy {
        match p {
            DowngradePolicy::RemoveAtEndOfPeriod => {
                prices_proto::fee_structure::DowngradePolicy::RemoveAtEndOfPeriod
            }
        }
    }

    fn downgrade_policy_from_proto(value: i32) -> DowngradePolicy {
        // Only one variant for now
        let _ = value;
        DowngradePolicy::RemoveAtEndOfPeriod
    }
}
