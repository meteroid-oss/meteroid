use crate::api_rest::productfamilies::model::ProductFamily;
use meteroid_store::domain;

pub fn domain_to_rest(d: domain::ProductFamily) -> ProductFamily {
    ProductFamily {
        local_id: d.local_id,
        name: d.name,
    }
}
