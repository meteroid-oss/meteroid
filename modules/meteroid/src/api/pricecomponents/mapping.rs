pub mod components {
    use crate::api::domain_mapping::billing_period;
    use crate::api::prices::mapping::prices::{
        PriceWrapper, cadence_from_proto, pricing_from_proto, pricing_to_proto,
    };
    use crate::api::productitems::mapping::products::{
        fee_structure_from_proto, fee_type_from_proto,
    };
    use common_domain::ids::{PriceId, ProductId};
    use meteroid_grpc::meteroid::api::components::v1 as api;
    use meteroid_grpc::meteroid::api::prices::v1 as proto;
    use meteroid_store::domain::price_components as domain;
    use meteroid_store::repositories::price_components::PriceInput;
    use tonic::Status;

    pub fn domain_to_api(comp: domain::PriceComponent) -> api::PriceComponent {
        let prices = if !comp.prices.is_empty() {
            // V2: real prices
            comp.prices
                .into_iter()
                .map(|p| PriceWrapper::from(p).0)
                .collect()
        } else if let Some(legacy) = &comp.legacy_pricing {
            // V1: map legacy entries to proto Prices with empty IDs
            legacy
                .pricing_entries
                .iter()
                .map(|(cadence, pricing)| proto::Price {
                    id: String::new(),
                    product_id: String::new(),
                    cadence: billing_period::to_proto(*cadence).into(),
                    currency: legacy.currency.clone(),
                    pricing: pricing_to_proto(pricing),
                    created_at: None,
                    archived_at: None,
                })
                .collect()
        } else {
            vec![]
        };

        api::PriceComponent {
            id: comp.id.as_proto(),
            local_id: comp.id.as_proto(),
            name: comp.name.to_string(),
            product_id: comp.product_id.map(|p| p.as_proto()),
            prices,
        }
    }

    pub fn product_ref_from_proto(
        product: Option<api::ProductRef>,
    ) -> Result<domain::ProductRef, Status> {
        let product = product.ok_or_else(|| Status::invalid_argument("product is required"))?;
        match product.r#ref {
            Some(api::product_ref::Ref::ExistingProductId(id)) => {
                let product_id = ProductId::from_proto(id)?;
                Ok(domain::ProductRef::Existing(product_id))
            }
            Some(api::product_ref::Ref::NewProduct(np)) => {
                let fee_type = fee_type_from_proto(np.fee_type)?;
                let fee_structure = np
                    .fee_structure
                    .ok_or_else(|| Status::invalid_argument("fee_structure is required"))?;
                let fee_structure = fee_structure_from_proto(fee_structure)?;
                Ok(domain::ProductRef::New {
                    name: np.name,
                    fee_type,
                    fee_structure,
                })
            }
            None => Err(Status::invalid_argument(
                "product ref must specify existing_product_id or new_product",
            )),
        }
    }

    pub fn price_entries_from_proto(
        entries: Vec<api::PriceEntry>,
    ) -> Result<Vec<domain::PriceEntry>, Status> {
        entries
            .into_iter()
            .map(|entry| match entry.entry {
                Some(api::price_entry::Entry::ExistingPriceId(id)) => {
                    let price_id = PriceId::from_proto(id)?;
                    Ok(domain::PriceEntry::Existing(price_id))
                }
                Some(api::price_entry::Entry::NewPrice(pi)) => {
                    let price_input = price_input_from_proto(pi)?;
                    Ok(domain::PriceEntry::New(price_input))
                }
                None => Err(Status::invalid_argument(
                    "price entry must specify existing_price_id or new_price",
                )),
            })
            .collect()
    }

    pub fn price_inputs_from_proto(
        inputs: Vec<api::PriceInput>,
    ) -> Result<Vec<PriceInput>, Status> {
        inputs.into_iter().map(price_input_from_proto).collect()
    }

    fn price_input_from_proto(pi: api::PriceInput) -> Result<PriceInput, Status> {
        let cadence = cadence_from_proto(pi.cadence)?;
        let pricing = pricing_from_proto(pi.pricing)?;
        Ok(PriceInput {
            cadence,
            currency: pi.currency,
            pricing,
        })
    }
}
