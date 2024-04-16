pub mod role {
    use meteroid_grpc::meteroid::api::users::v1 as server;

    pub fn db_to_server(role: meteroid_repository::OrganizationUserRole) -> server::UserRole {
        match role {
            meteroid_repository::OrganizationUserRole::ADMIN => server::UserRole::Admin,
            meteroid_repository::OrganizationUserRole::MEMBER => server::UserRole::Member,
        }
    }
}
