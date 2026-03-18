use meteroid_store::domain;
use meteroid_store::domain::price_components::{DowngradePolicy, UpgradePolicy};
use meteroid_store::domain::prices::{FeeStructure, UsageModel};

use super::model::*;
use crate::errors::RestApiError;

pub fn product_to_rest(product: domain::Product) -> Product {
    Product {
        id: product.id,
        name: product.name,
        description: product.description,
        fee_type: product.fee_type.into(),
        fee_structure: fee_structure_to_rest(&product.fee_structure),
        product_family_id: product.product_family_id,
        catalog: product.catalog,
        created_at: product.created_at,
        archived_at: product.archived_at,
    }
}

fn fee_structure_to_rest(fs: &FeeStructure) -> ProductFeeStructure {
    match fs {
        FeeStructure::Rate {} => ProductFeeStructure::Rate {},
        FeeStructure::Slot {
            unit_name,
            upgrade_policy,
            downgrade_policy,
        } => ProductFeeStructure::Slot {
            slot_unit_name: unit_name.clone(),
            upgrade_policy: match upgrade_policy {
                UpgradePolicy::Prorated => SlotUpgradePolicyEnum::Prorated,
            },
            downgrade_policy: match downgrade_policy {
                DowngradePolicy::RemoveAtEndOfPeriod => {
                    SlotDowngradePolicyEnum::RemoveAtEndOfPeriod
                }
            },
        },
        FeeStructure::Capacity { metric_id } => ProductFeeStructure::Capacity {
            metric_id: *metric_id,
        },
        FeeStructure::Usage { metric_id, model } => ProductFeeStructure::Usage {
            metric_id: *metric_id,
            model: match model {
                UsageModel::PerUnit => UsageModelEnum::PerUnit,
                UsageModel::Tiered => UsageModelEnum::Tiered,
                UsageModel::Volume => UsageModelEnum::Volume,
                UsageModel::Package => UsageModelEnum::Package,
                UsageModel::Matrix => UsageModelEnum::Matrix,
            },
        },
        FeeStructure::ExtraRecurring { billing_type } => ProductFeeStructure::ExtraRecurring {
            billing_type: match billing_type {
                domain::enums::BillingType::Advance => ExtraRecurringBillingTypeEnum::Advance,
                domain::enums::BillingType::Arrears => ExtraRecurringBillingTypeEnum::Arrears,
            },
        },
        FeeStructure::OneTime {} => ProductFeeStructure::OneTime {},
    }
}

pub fn rest_fee_structure_to_domain(
    fs: &ProductFeeStructure,
) -> Result<(domain::enums::FeeTypeEnum, FeeStructure), RestApiError> {
    match fs {
        ProductFeeStructure::Rate {} => Ok((
            domain::enums::FeeTypeEnum::Rate,
            FeeStructure::Rate {},
        )),
        ProductFeeStructure::Slot {
            slot_unit_name,
            upgrade_policy,
            downgrade_policy,
        } => Ok((
            domain::enums::FeeTypeEnum::Slot,
            FeeStructure::Slot {
                unit_name: slot_unit_name.clone(),
                upgrade_policy: match upgrade_policy {
                    SlotUpgradePolicyEnum::Prorated => UpgradePolicy::Prorated,
                },
                downgrade_policy: match downgrade_policy {
                    SlotDowngradePolicyEnum::RemoveAtEndOfPeriod => {
                        DowngradePolicy::RemoveAtEndOfPeriod
                    }
                },
            },
        )),
        ProductFeeStructure::Capacity { metric_id } => Ok((
            domain::enums::FeeTypeEnum::Capacity,
            FeeStructure::Capacity {
                metric_id: *metric_id,
            },
        )),
        ProductFeeStructure::Usage { metric_id, model } => Ok((
            domain::enums::FeeTypeEnum::Usage,
            FeeStructure::Usage {
                metric_id: *metric_id,
                model: match model {
                    UsageModelEnum::PerUnit => UsageModel::PerUnit,
                    UsageModelEnum::Tiered => UsageModel::Tiered,
                    UsageModelEnum::Volume => UsageModel::Volume,
                    UsageModelEnum::Package => UsageModel::Package,
                    UsageModelEnum::Matrix => UsageModel::Matrix,
                },
            },
        )),
        ProductFeeStructure::ExtraRecurring { billing_type } => Ok((
            domain::enums::FeeTypeEnum::ExtraRecurring,
            FeeStructure::ExtraRecurring {
                billing_type: match billing_type {
                    ExtraRecurringBillingTypeEnum::Advance => domain::enums::BillingType::Advance,
                    ExtraRecurringBillingTypeEnum::Arrears => domain::enums::BillingType::Arrears,
                },
            },
        )),
        ProductFeeStructure::OneTime {} => Ok((
            domain::enums::FeeTypeEnum::OneTime,
            FeeStructure::OneTime {},
        )),
    }
}
