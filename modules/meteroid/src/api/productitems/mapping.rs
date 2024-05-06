pub mod products {
    use meteroid_grpc::meteroid::api::products::v1::{Product, ProductMeta};
    use meteroid_store::domain;

    use crate::api::shared::mapping::datetime::{chrono_to_timestamp, datetime_to_timestamp};
    pub struct ProductWrapper(pub Product);

    impl From<domain::Product> for ProductWrapper {
        fn from(product: domain::Product) -> Self {
            ProductWrapper(Product {
                id: product.id.to_string(),
                name: product.name,
                description: product.description,
                created_at: Some(chrono_to_timestamp(product.created_at)),
            })
        }
    }

    pub struct ProductMetaWrapper(pub ProductMeta);
    impl From<domain::Product> for ProductMetaWrapper {
        fn from(product: domain::Product) -> Self {
            ProductMetaWrapper(ProductMeta {
                id: product.id.to_string(),
                name: product.name,
            })
        }
    }
}
