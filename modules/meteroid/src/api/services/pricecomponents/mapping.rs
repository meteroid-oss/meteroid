pub mod components {
    use meteroid_grpc::meteroid::api::components::v1 as grpc;
    use meteroid_repository as db;
    use tonic::Status;

    pub fn db_to_server(
        db_comp: db::price_components::PriceComponent,
    ) -> Result<grpc::PriceComponent, Status> {
        let fee_type_decoded: grpc::fee::Type = serde_json::from_value(db_comp.fee)
            .map_err(|e| Status::internal(format!("Failed to decode fee type: {}", e)))?;

        Ok(grpc::PriceComponent {
            id: db_comp.id.to_string(),
            name: db_comp.name.to_string(),
            fee_type: Some(fee_type_decoded),
            product_item: db_comp.product_item_id.zip(db_comp.product_item_name).map(
                |(id, name)| grpc::price_component::ProductItem {
                    id: id.to_string(),
                    name: name.to_string(),
                },
            ),
        })
    }
}
