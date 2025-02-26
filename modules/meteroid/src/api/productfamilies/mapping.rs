pub mod product_family {
    use meteroid_grpc::meteroid::api::productfamilies::v1::ProductFamily;
    use meteroid_store::domain;

    pub struct ProductFamilyWrapper(pub ProductFamily);

    impl From<domain::ProductFamily> for ProductFamilyWrapper {
        fn from(domain_family: domain::ProductFamily) -> Self {
            ProductFamilyWrapper(ProductFamily {
                id: domain_family.id.as_proto(),
                name: domain_family.name,
                local_id: domain_family.id.as_proto(), //todo remove me
            })
        }
    }
}
