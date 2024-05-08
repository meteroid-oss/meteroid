pub mod api_token {
    use meteroid_grpc::meteroid::api::apitokens::v1::ApiToken;
    use meteroid_store::domain;

    use crate::api::shared::mapping::datetime::chrono_to_timestamp;

    pub fn domain_to_api(api_token: domain::api_tokens::ApiToken) -> ApiToken {
        ApiToken {
            id: api_token.id.to_string(),
            tenant_id: api_token.tenant_id.to_string(),
            name: api_token.name,
            hint: api_token.hint,
            created_at: Some(chrono_to_timestamp(api_token.created_at)),
            created_by: api_token.created_by.to_string(),
        }
    }
}
