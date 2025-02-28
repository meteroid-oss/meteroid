use crate::api_rest::productfamilies::model::{ProductFamily, ProductFamilyCreateRequest};
use common_domain::ids::TenantId;
use meteroid_store::domain;
use meteroid_store::domain::ProductFamilyNew;

pub(crate) fn domain_to_rest(d: domain::ProductFamily) -> ProductFamily {
    ProductFamily {
        id: d.id,
        name: d.name,
    }
}

pub(crate) fn create_req_to_domain(
    req: ProductFamilyCreateRequest,
    tenant_id: TenantId,
) -> ProductFamilyNew {
    ProductFamilyNew {
        name: req.name,
        tenant_id,
    }
}
