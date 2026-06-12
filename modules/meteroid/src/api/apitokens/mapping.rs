pub mod api_token {
    use crate::api::shared::conversions::ProtoConv;
    use meteroid_grpc::meteroid::api::apitokens::v1::ApiToken;
    use meteroid_store::domain;

    pub fn domain_to_api(api_token: domain::api_tokens::ApiToken) -> ApiToken {
        ApiToken {
            id: api_token.id.as_proto(),
            tenant_id: api_token.tenant_id.as_proto(),
            name: api_token.name,
            hint: api_token.hint,
            created_at: api_token.created_at.as_proto(),
        }
    }
}
