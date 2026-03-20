use meteroid_store::domain;

use super::model;
use crate::api_rest::products::model::ProductFeeTypeEnum;

pub fn addon_to_rest(addon: domain::add_ons::AddOn) -> model::AddOn {
    model::AddOn {
        id: addon.id,
        name: addon.name,
        description: addon.description,
        product_id: addon.product_id,
        price_id: addon.price_id,
        fee_type: addon.fee_type.map(ProductFeeTypeEnum::from),
        self_serviceable: addon.self_serviceable,
        max_instances_per_subscription: addon.max_instances_per_subscription,
        created_at: addon.created_at,
        archived_at: addon.archived_at,
    }
}
