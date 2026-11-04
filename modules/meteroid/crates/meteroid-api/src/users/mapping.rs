pub mod role {
    use meteroid_grpc::meteroid::api::users::v1 as server;
    use meteroid_store::domain::enums::OrganizationUserRole;

    pub fn domain_to_server(role: OrganizationUserRole) -> server::OrganizationUserRole {
        match role {
            OrganizationUserRole::Admin => server::OrganizationUserRole::Admin,
            OrganizationUserRole::Member => server::OrganizationUserRole::Member,
        }
    }

    pub fn server_to_domain(role: server::OrganizationUserRole) -> OrganizationUserRole {
        match role {
            server::OrganizationUserRole::Admin => OrganizationUserRole::Admin,
            server::OrganizationUserRole::Member => OrganizationUserRole::Member,
        }
    }
}

pub mod user {
    use crate::organizations::mapping::organization;
    use crate::users::mapping::role;
    use meteroid_grpc::meteroid::api::users::v1 as server;
    use meteroid_store::domain::users::{Me, User, UserWithRole};

    pub fn me_to_proto(domain: Me) -> server::MeResponse {
        server::MeResponse {
            user: Some(domain_to_proto(domain.user)),
            organizations: domain
                .organizations
                .into_iter()
                .map(organization::domain_to_proto)
                .collect(),
            mfa_enrollment_required: domain.mfa_enrollment_required,
        }
    }

    pub fn domain_to_proto(domain: User) -> server::User {
        server::User {
            id: domain.id.as_proto(),
            email: domain.email,
            department: domain.department,
            first_name: domain.first_name,
            last_name: domain.last_name,
            onboarded: domain.onboarded,
            mfa_enabled: domain.mfa_enabled,
            preferences: serde_json::to_string(&domain.preferences)
                .unwrap_or_else(|_| "{}".to_string()),
        }
    }

    pub fn domain_with_role_to_proto(domain: UserWithRole) -> server::UserWithRole {
        server::UserWithRole {
            id: domain.id.as_proto(),
            email: domain.email,
            department: domain.department,
            first_name: domain.first_name,
            last_name: domain.last_name,
            onboarded: domain.onboarded,
            role: role::domain_to_server(domain.role).into(),
            mfa_enabled: domain.mfa_enabled,
            custom_role_id: domain.custom_role_id.map(|id| id.as_proto()),
        }
    }
}
