pub mod product_family {
    use meteroid_grpc::meteroid::api::productfamilies::v1::ProductFamily;
    use meteroid_store::domain;

    pub struct ProductFamilyWrapper(pub ProductFamily);

    impl From<domain::ProductFamily> for ProductFamilyWrapper {
        fn from(domain_family: domain::ProductFamily) -> Self {
            ProductFamilyWrapper(ProductFamily {
                id: domain_family.id.to_string(),
                name: domain_family.name,
                local_id: domain_family.local_id,
            })
        }
    }
}
