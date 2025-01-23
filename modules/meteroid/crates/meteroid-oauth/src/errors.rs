#[derive(Debug, thiserror::Error)]
pub enum OauthServiceError {
    #[error("Provider api error: {0}")]
    ProviderApi(String),
    #[error("User email not verified")]
    UserEmailNotVerified,
}
