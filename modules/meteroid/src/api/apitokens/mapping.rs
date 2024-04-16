pub mod api_token {
    use meteroid_grpc::meteroid::api::apitokens::v1::ApiToken;
    use meteroid_repository::api_tokens::ApiToken as DbApiToken;

    use crate::api::shared::mapping::datetime::datetime_to_timestamp;

    pub fn db_to_server(api_token: DbApiToken) -> ApiToken {
        ApiToken {
            id: api_token.id.to_string(),
            tenant_id: api_token.tenant_id.to_string(),
            name: api_token.name,
            hint: api_token.hint,
            created_at: Some(datetime_to_timestamp(api_token.created_at)),
            created_by: api_token.created_by.to_string(),
        }
    }
}
