pub mod addons {
    use meteroid_grpc::meteroid::api::addons::v1 as server;
    use meteroid_store::domain;

    pub struct AddOnWrapper(pub server::AddOn);
    impl From<domain::add_ons::AddOn> for AddOnWrapper {
        fn from(value: domain::add_ons::AddOn) -> Self {
            Self(server::AddOn {
                id: value.id.to_string(),
                name: value.name.into(),
                fee: Some(
                    crate::api::pricecomponents::mapping::components::map_fee_domain_to_api(
                        value.fee,
                    ),
                ),
            })
        }
    }
}
