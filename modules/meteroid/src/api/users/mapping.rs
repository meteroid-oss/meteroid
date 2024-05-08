pub mod role {
    use meteroid_grpc::meteroid::api::users::v1 as server;
    use meteroid_store::domain::enums::OrganizationUserRole;

    pub fn db_to_server(role: meteroid_repository::OrganizationUserRole) -> server::UserRole {
        match role {
            meteroid_repository::OrganizationUserRole::ADMIN => server::UserRole::Admin,
            meteroid_repository::OrganizationUserRole::MEMBER => server::UserRole::Member,
        }
    }

    pub fn domain_to_server(role: OrganizationUserRole) -> server::UserRole {
        match role {
            OrganizationUserRole::Admin => server::UserRole::Admin,
            OrganizationUserRole::Member => server::UserRole::Member,
        }
    }
}

pub mod user {
    use crate::api::users::mapping::role;
    use meteroid_grpc::meteroid::api::users::v1 as server;
    use meteroid_store::domain::users::User;

    pub fn domain_to_proto(domain: User) -> server::User {
        server::User {
            id: domain.id.to_string(),
            email: domain.email,
            role: role::domain_to_server(domain.role).into(),
        }
    }
}
