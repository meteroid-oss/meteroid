use thiserror::Error;

#[derive(Debug, Error)]
pub enum LicenseError {
    #[error("Invalid license format")]
    InvalidFormat,

    #[error("Failed to decode license: {0}")]
    DecodeError(String),

    #[error("Unsupported license algorithm: {0}")]
    UnsupportedAlgorithm(String),

    #[error("License signed by an unknown key: {0}")]
    UnknownKey(String),

    #[error("Invalid signature")]
    InvalidSignature,

    #[error("Invalid issuer: {0}")]
    InvalidIssuer(String),

    #[error("License has expired")]
    Expired,

    #[error("License is not valid yet")]
    NotYetValid,

    #[error("Legacy license format is no longer accepted")]
    LegacyFormatRetired,

    #[error("Invalid public key: {0}")]
    InvalidPublicKey(String),

    #[error("An offline license is configured")]
    OfflineLicenseConfigured,

    #[error("Online licenses must be self-hosted")]
    UnexpectedMode,

    #[error("Invalid instance signature")]
    InvalidInstanceSignature,

    #[cfg(feature = "issuer")]
    #[error("Invalid signing key: {0}")]
    InvalidSigningKey(String),
}
