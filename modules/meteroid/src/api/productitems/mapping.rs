pub mod products {
    use meteroid_grpc::meteroid::api::products::v1::{Product, ProductMeta};
    use meteroid_repository::products::{ListProduct as DbListProducts, Product as DbProduct};

    use crate::api::shared::mapping::datetime::datetime_to_timestamp;

    // TODO add the db_product.**count
    pub fn db_to_server(db_product: DbProduct) -> Product {
        Product {
            id: db_product.id.to_string(),
            name: db_product.name,
            description: db_product.description,
            created_at: Some(datetime_to_timestamp(db_product.created_at)),
        }
    }

    pub fn db_to_server_list(db_product: DbListProducts) -> ProductMeta {
        ProductMeta {
            id: db_product.id.to_string(),
            name: db_product.name,
        }
    }
}
