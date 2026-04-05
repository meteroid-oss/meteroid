use std::fmt;

const MAX_CODE_LEN: usize = 512;

#[derive(Debug, Clone)]
pub enum IdentifierError {
    Empty,
    TooLong { max: usize, actual: usize },
    InvalidTimezone(String),
}

impl fmt::Display for IdentifierError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "must not be empty"),
            Self::TooLong { max, actual } => {
                write!(f, "too long ({actual} > {max} chars)")
            }
            Self::InvalidTimezone(tz) => {
                write!(f, "invalid IANA timezone: '{tz}'")
            }
        }
    }
}

impl std::error::Error for IdentifierError {}

pub fn validate_code(code: &str) -> Result<(), IdentifierError> {
    if code.is_empty() {
        return Err(IdentifierError::Empty);
    }
    if code.len() > MAX_CODE_LEN {
        return Err(IdentifierError::TooLong {
            max: MAX_CODE_LEN,
            actual: code.len(),
        });
    }
    Ok(())
}

pub fn parse_timezone(tz: &str) -> Result<chrono_tz::Tz, IdentifierError> {
    tz.parse::<chrono_tz::Tz>()
        .map_err(|_| IdentifierError::InvalidTimezone(tz.to_string()))
}

pub fn validator_code(code: &str) -> Result<(), validator::ValidationError> {
    validate_code(code).map_err(|e| {
        let mut err = validator::ValidationError::new("invalid_code");
        err.message = Some(e.to_string().into());
        err
    })
}

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
        assert!(validate_code("code with space").is_ok());
        assert!(validate_code("code'quotes").is_ok());
        assert!(validate_code("a").is_ok());
    }

    #[test]
    fn invalid_codes() {
        assert!(validate_code("").is_err());
        let long = "a".repeat(MAX_CODE_LEN + 1);
        assert!(validate_code(&long).is_err());
    }

    #[test]
    fn valid_timezones() {
        assert!(parse_timezone("UTC").is_ok());
        assert!(parse_timezone("America/New_York").is_ok());
        assert!(parse_timezone("Europe/London").is_ok());
    }

    #[test]
    fn invalid_timezones() {
        assert!(parse_timezone("").is_err());
        assert!(parse_timezone("Not/A/Real/Zone").is_err());
    }
}
