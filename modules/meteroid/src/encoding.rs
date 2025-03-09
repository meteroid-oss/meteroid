use base64::{Engine as _, engine::general_purpose};
use error_stack::{Result, ResultExt};

#[derive(Debug, thiserror::Error, PartialEq, Clone)]
pub enum EncodingError {
    #[error("base64 decode error")]
    Base64DecodeError,
}

pub fn base64_encode(data: &str) -> String {
    general_purpose::URL_SAFE_NO_PAD.encode(data)
}

pub fn base64_decode(data: &str) -> Result<String, EncodingError> {
    general_purpose::URL_SAFE_NO_PAD
        .decode(data)
        .map(|x| String::from_utf8_lossy(x.as_slice()).to_string())
        .change_context(EncodingError::Base64DecodeError)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base64_encode() {
        assert_eq!(base64_encode("hello"), "aGVsbG8");
    }

    #[test]
    fn test_base64_decode() {
        assert_eq!(base64_decode("aGVsbG8").unwrap(), "hello");
    }
}
