pub mod organization {
    use super::super::super::tenants::mapping::tenants as tenants_mapping;
    use crate::api::shared::conversions::ProtoConv;
    use meteroid_grpc::meteroid::api::organizations::v1 as server;
    use meteroid_grpc::meteroid::api::tenants::v1::Tenant as GrpcTenant;
    use meteroid_store::domain;
    use meteroid_store::domain::Tenant;

    pub fn domain_to_proto(domain: domain::Organization) -> server::Organization {
        server::Organization {
            id: domain.id.as_proto(),
            slug: domain.slug,
            created_at: domain.created_at.as_proto(),
            trade_name: domain.trade_name,
        }
    }

    pub fn domain_with_tenants_to_proto(
        domain: domain::OrganizationWithTenants,
        prepend_tenant: Option<Tenant>,
    ) -> server::OrganizationWithTenant {
        let mut tenants: Vec<GrpcTenant> = domain
            .tenants
            .into_iter()
            .map(tenants_mapping::domain_to_server)
            .collect();

        if let Some(prepend_tenant) = prepend_tenant {
            tenants.insert(0, tenants_mapping::domain_to_server(prepend_tenant));
        }

        server::OrganizationWithTenant {
            id: domain.organization.id.as_proto(),
            slug: domain.organization.slug,
            created_at: domain.organization.created_at.as_proto(),
            trade_name: domain.organization.trade_name,
            tenants,
        }
    }
}
