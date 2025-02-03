use crate::api_rest::productfamilies::model::{ProductFamily, ProductFamilyCreateRequest};
use meteroid_store::domain;
use meteroid_store::domain::ProductFamilyNew;
use uuid::Uuid;

pub(crate) fn domain_to_rest(d: domain::ProductFamily) -> ProductFamily {
    ProductFamily {
        id: d.local_id,
        name: d.name,
    }
}

pub(crate) fn create_req_to_domain(
    req: ProductFamilyCreateRequest,
    tenant_id: Uuid,
) -> ProductFamilyNew {
    ProductFamilyNew {
        name: req.name,
        tenant_id,
    }
}
