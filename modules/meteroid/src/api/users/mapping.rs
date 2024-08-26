pub mod role {
    use meteroid_grpc::meteroid::api::users::v1 as server;
    use meteroid_store::domain::enums::OrganizationUserRole;

    pub fn domain_to_server(role: OrganizationUserRole) -> server::OrganizationUserRole {
        match role {
            OrganizationUserRole::Admin => server::OrganizationUserRole::Admin,
            OrganizationUserRole::Member => server::OrganizationUserRole::Member,
        }
    }
}

pub mod user {
    use crate::api::users::mapping::role;
    use crate::api::organizations::mapping::organization;
    use meteroid_grpc::meteroid::api::users::v1 as server;
    use meteroid_store::domain::users::{Me, User, UserWithRole};

    pub fn me_to_proto(domain: Me) -> server::MeResponse {
        server::MeResponse {
            user: Some(domain_to_proto(domain.user)),
            organizations: domain.organizations.into_iter().map(|x| organization::domain_to_proto(x)).collect(),
            current_organization_role: domain.current_organization_role.map(|x| super::role::domain_to_server(x).into()),
        }
    }

    pub fn domain_to_proto(domain: User) -> server::User {
        server::User {
            id: domain.id.to_string(),
            email: domain.email,
            department: domain.department,
            first_name: domain.first_name,
            last_name: domain.last_name,
            onboarded: domain.onboarded,
        }
    }

    pub fn domain_with_role_to_proto(domain: UserWithRole) -> server::UserWithRole {
        server::UserWithRole {
            id: domain.id.to_string(),
            email: domain.email,
            department: domain.department,
            first_name: domain.first_name,
            last_name: domain.last_name,
            onboarded: domain.onboarded,
            role: role::domain_to_server(domain.role).into(),
        }
    }
}
