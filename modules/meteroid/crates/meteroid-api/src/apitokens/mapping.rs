pub mod api_token {
    use meteroid_app::conversions::ProtoConv;
    use meteroid_grpc::meteroid::api::apitokens::v1 as proto;
    use meteroid_grpc::meteroid::api::apitokens::v1::{ApiToken, ApiTokenKind};
    use meteroid_store::domain;
    use meteroid_store::domain::api_key_permissions::{
        AccessLevel, ApiKeyPermissions, ApiResource,
    };

    fn resource_to_api(resource: ApiResource) -> proto::ApiResource {
        match resource {
            ApiResource::Customers => proto::ApiResource::Customers,
            ApiResource::Subscriptions => proto::ApiResource::Subscriptions,
            ApiResource::Invoices => proto::ApiResource::Invoices,
            ApiResource::Payments => proto::ApiResource::Payments,
            ApiResource::Catalog => proto::ApiResource::Catalog,
            ApiResource::Usage => proto::ApiResource::Usage,
            ApiResource::Entitlements => proto::ApiResource::Entitlements,
            ApiResource::Webhooks => proto::ApiResource::Webhooks,
            ApiResource::BatchJobs => proto::ApiResource::BatchJobs,
        }
    }

    fn resource_from_api(resource: proto::ApiResource) -> Option<ApiResource> {
        Some(match resource {
            proto::ApiResource::Unspecified => return None,
            proto::ApiResource::Customers => ApiResource::Customers,
            proto::ApiResource::Subscriptions => ApiResource::Subscriptions,
            proto::ApiResource::Invoices => ApiResource::Invoices,
            proto::ApiResource::Payments => ApiResource::Payments,
            proto::ApiResource::Catalog => ApiResource::Catalog,
            proto::ApiResource::Usage => ApiResource::Usage,
            proto::ApiResource::Entitlements => ApiResource::Entitlements,
            proto::ApiResource::Webhooks => ApiResource::Webhooks,
            proto::ApiResource::BatchJobs => ApiResource::BatchJobs,
        })
    }

    fn level_to_api(level: AccessLevel) -> proto::ApiAccessLevel {
        match level {
            AccessLevel::None => proto::ApiAccessLevel::None,
            AccessLevel::Read => proto::ApiAccessLevel::Read,
            AccessLevel::Write => proto::ApiAccessLevel::Write,
        }
    }

    fn level_from_api(level: proto::ApiAccessLevel) -> AccessLevel {
        match level {
            proto::ApiAccessLevel::None => AccessLevel::None,
            proto::ApiAccessLevel::Read => AccessLevel::Read,
            proto::ApiAccessLevel::Write => AccessLevel::Write,
        }
    }

    pub fn permissions_to_api(permissions: &ApiKeyPermissions) -> proto::ApiKeyPermissions {
        proto::ApiKeyPermissions {
            grants: permissions
                .grants()
                .map(|(resource, level)| proto::ApiKeyGrant {
                    resource: resource_to_api(resource) as i32,
                    level: level_to_api(level) as i32,
                })
                .collect(),
        }
    }

    pub fn permissions_from_api(
        permissions: proto::ApiKeyPermissions,
    ) -> Result<ApiKeyPermissions, String> {
        permissions
            .grants
            .into_iter()
            .map(|grant| {
                let resource = proto::ApiResource::try_from(grant.resource)
                    .ok()
                    .and_then(resource_from_api)
                    .ok_or_else(|| format!("unknown api resource {}", grant.resource))?;
                let level = proto::ApiAccessLevel::try_from(grant.level)
                    .map_err(|_| format!("unknown access level {}", grant.level))?;
                Ok((resource, level_from_api(level)))
            })
            .collect::<Result<Vec<_>, String>>()
            .and_then(|grants| {
                let mut seen = std::collections::HashSet::new();
                match grants.iter().find(|(resource, _)| !seen.insert(*resource)) {
                    Some((resource, _)) => Err(format!("{resource} is granted more than once")),
                    None => Ok(ApiKeyPermissions::new(grants)),
                }
            })
    }

    pub fn domain_to_api(api_token: domain::api_tokens::ApiToken) -> ApiToken {
        let kind = match api_token.kind {
            domain::ApiTokenKind::Secret => ApiTokenKind::Secret,
            domain::ApiTokenKind::Publishable => ApiTokenKind::Publishable,
        };
        ApiToken {
            id: api_token.id.as_proto(),
            tenant_id: api_token.tenant_id.as_proto(),
            name: api_token.name,
            hint: api_token.hint,
            created_at: api_token.created_at.as_proto(),
            last_used_at: api_token.last_used_at.map(|v| v.as_proto()),
            kind: kind as i32,
            // Only a publishable key is ever stored whole.
            publishable_key: api_token
                .publishable_key
                .filter(|_| kind == ApiTokenKind::Publishable),
            allowed_origins: api_token.allowed_origins,
            permissions: api_token.permissions.as_ref().map(permissions_to_api),
            expires_at: api_token.expires_at.map(|v| v.as_proto()),
            lifetime_days: api_token.lifetime_days,
            replaced_by_id: api_token.replaced_by_id.map(|v| v.as_proto()),
        }
    }
}
