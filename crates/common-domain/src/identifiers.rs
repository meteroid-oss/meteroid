use std::fmt;

/// Maximum length for identifier strings (codes, property keys, etc.)
const MAX_IDENTIFIER_LEN: usize = 128;

#[derive(Debug, Clone)]
pub enum IdentifierError {
    Empty,
    TooLong { max: usize, actual: usize },
    InvalidChar { ch: char, kind: &'static str },
    InvalidTimezone(String),
}

impl fmt::Display for IdentifierError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "identifier must not be empty"),
            Self::TooLong { max, actual } => {
                write!(f, "identifier too long ({actual} > {max} chars)")
            }
            Self::InvalidChar { ch, kind } => {
                write!(f, "invalid character '{ch}' in {kind}")
            }
            Self::InvalidTimezone(tz) => {
                write!(f, "invalid IANA timezone: '{tz}'")
            }
        }
    }
}

impl std::error::Error for IdentifierError {}

/// Allowed: `[a-zA-Z0-9._-]`. Used for event codes, meter codes, property keys,
/// aggregation keys, dimension keys, usage group keys.
fn is_valid_identifier_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-'
}

/// Validates a code (event code, meter code).
pub fn validate_code(code: &str) -> Result<(), IdentifierError> {
    validate_identifier_inner(code, "code")
}

/// Validates a property key (group_by, value_property, dimension key, usage_group_key).
pub fn validate_property_key(key: &str) -> Result<(), IdentifierError> {
    validate_identifier_inner(key, "property key")
}

fn validate_identifier_inner(s: &str, kind: &'static str) -> Result<(), IdentifierError> {
    if s.is_empty() {
        return Err(IdentifierError::Empty);
    }
    if s.len() > MAX_IDENTIFIER_LEN {
        return Err(IdentifierError::TooLong {
            max: MAX_IDENTIFIER_LEN,
            actual: s.len(),
        });
    }
    for ch in s.chars() {
        if !is_valid_identifier_char(ch) {
            return Err(IdentifierError::InvalidChar { ch, kind });
        }
    }
    Ok(())
}

pub fn parse_timezone(tz: &str) -> Result<chrono_tz::Tz, IdentifierError> {
    tz.parse::<chrono_tz::Tz>()
        .map_err(|_| IdentifierError::InvalidTimezone(tz.to_string()))
}

/// Validator adapter for the `validator` crate's `#[validate(custom(...))]`.
pub fn validator_code(code: &str) -> Result<(), validator::ValidationError> {
    validate_code(code).map_err(|e| {
        let mut err = validator::ValidationError::new("invalid_code");
        err.message = Some(e.to_string().into());
        err
    })
}

/// Validator adapter for property keys.
pub fn validator_property_key(key: &str) -> Result<(), validator::ValidationError> {
    validate_property_key(key).map_err(|e| {
        let mut err = validator::ValidationError::new("invalid_property_key");
        err.message = Some(e.to_string().into());
        err
    })
}

/// Validator adapter for timezone.
pub fn validator_timezone(tz: &str) -> Result<(), validator::ValidationError> {
    parse_timezone(tz).map(|_| ()).map_err(|e| {
        let mut err = validator::ValidationError::new("invalid_timezone");
        err.message = Some(e.to_string().into());
        err
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_codes() {
        assert!(validate_code("api_call").is_ok());
        assert!(validate_code("openstack.instance.uptime").is_ok());
        assert!(validate_code("my-event-code").is_ok());
        assert!(validate_code("CREDIT10").is_ok());
        assert!(validate_code("a").is_ok());
    }

    #[test]
    fn invalid_codes() {
        assert!(validate_code("").is_err());
        assert!(validate_code("code with space").is_err());
        assert!(validate_code("code'injection").is_err());
        assert!(validate_code("') OR 1=1 --").is_err());
        assert!(validate_code("code;DROP TABLE").is_err());
        assert!(validate_code("code\nline").is_err());
    }

    #[test]
    fn valid_property_keys() {
        assert!(validate_property_key("instance_id").is_ok());
        assert!(validate_property_key("flavor").is_ok());
        assert!(validate_property_key("req_cpus").is_ok());
        assert!(validate_property_key("resource.type").is_ok());
    }

    #[test]
    fn invalid_property_keys() {
        assert!(validate_property_key("").is_err());
        assert!(validate_property_key("key'] OR 1=1 --").is_err());
        assert!(validate_property_key("key with space").is_err());
    }

    #[test]
    fn valid_timezones() {
        assert!(parse_timezone("UTC").is_ok());
        assert!(parse_timezone("America/New_York").is_ok());
        assert!(parse_timezone("Etc/GMT+1").is_ok());
        assert!(parse_timezone("Europe/London").is_ok());
    }

    #[test]
    fn invalid_timezones() {
        assert!(parse_timezone("").is_err());
        assert!(parse_timezone("UTC') UNION SELECT 1 --").is_err());
        assert!(parse_timezone("Not/A/Real/Zone").is_err());
    }

    #[test]
    fn too_long_identifier() {
        let long = "a".repeat(MAX_IDENTIFIER_LEN + 1);
        assert!(validate_code(&long).is_err());
    }
}
