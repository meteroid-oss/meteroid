pub mod product_family {
    use meteroid_grpc::meteroid::api::productfamilies::v1::ProductFamily;
    use meteroid_repository::products::ProductFamily as DbProductFamily;

    pub fn db_to_server(db_family: DbProductFamily) -> ProductFamily {
        ProductFamily {
            id: db_family.id.to_string(),
            name: db_family.name,
            external_id: db_family.external_id,
        }
    }
}
