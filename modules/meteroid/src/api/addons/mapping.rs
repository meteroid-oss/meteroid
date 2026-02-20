pub mod addons {
    use crate::api::prices::mapping::prices::PriceWrapper;
    use crate::api::productitems::mapping::products::fee_type_to_proto;
    use meteroid_grpc::meteroid::api::addons::v1 as server;
    use meteroid_store::domain;

    pub struct AddOnWrapper(pub server::AddOn);
    impl From<domain::add_ons::AddOn> for AddOnWrapper {
        fn from(value: domain::add_ons::AddOn) -> Self {
            Self(server::AddOn {
                id: value.id.to_string(),
                local_id: value.id.to_string(),
                name: value.name,
                description: value.description,
                product_id: value.product_id.to_string(),
                fee_type: value.fee_type.map(fee_type_to_proto).unwrap_or(0),
                price: value.price.map(|p| PriceWrapper::from(p).0),
                self_serviceable: value.self_serviceable,
                max_instances_per_subscription: value.max_instances_per_subscription,
            })
        }
    }

    pub struct PlanVersionAddOnWrapper(pub server::PlanVersionAddOn);
    impl From<domain::plan_version_add_ons::PlanVersionAddOn> for PlanVersionAddOnWrapper {
        fn from(value: domain::plan_version_add_ons::PlanVersionAddOn) -> Self {
            Self(server::PlanVersionAddOn {
                id: value.id.to_string(),
                plan_version_id: value.plan_version_id.to_string(),
                add_on_id: value.add_on_id.to_string(),
                price_id: value.price_id.map(|p| p.to_string()),
                self_serviceable: value.self_serviceable,
                max_instances_per_subscription: value.max_instances_per_subscription,
            })
        }
    }
}
