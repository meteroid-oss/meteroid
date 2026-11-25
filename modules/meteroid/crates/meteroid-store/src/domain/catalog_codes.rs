//! The code of a plan, a product, an add-on or a product family: the handle an integrator names
//! it by beside its id, unique per tenant and immutable (ADR-0013). One left out on create is
//! the slug of the name, suffixed `-2`, `-3`… when taken (the `fill_catalog_code` trigger).
use crate::errors::StoreError;

pub const MAX_CODE_LEN: usize = 64;

/// Lowercase letters, digits, `-` and `_`, starting with a letter or a digit, at most 64; never
/// starting with `id_prefix` (`plan_`, `prd_`…), which a path would read as an id.
pub fn validate_code(code: &str, id_prefix: &str) -> Result<(), StoreError> {
    let mut chars = code.chars();
    let starts_well = chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    let valid = starts_well
        && code.len() <= MAX_CODE_LEN
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if !valid {
        return Err(StoreError::InvalidArgument(format!(
            "'{code}' is not a code: lowercase letters, digits, '-' and '_', starting with a \
             letter or a digit, at most {MAX_CODE_LEN}"
        )));
    }
    if code.starts_with(id_prefix) || uuid::Uuid::parse_str(code).is_ok() {
        return Err(StoreError::InvalidArgument(format!(
            "'{code}' starts like an id ('{id_prefix}'): pick another code"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_is_a_lowercase_handle_unlike_an_id() {
        assert!(validate_code("pro", "plan_").is_ok());
        assert!(validate_code("pro-annual_2", "plan_").is_ok());
        assert!(validate_code("9lives", "plan_").is_ok());
        assert!(validate_code("", "plan_").is_err());
        assert!(validate_code("Pro", "plan_").is_err());
        assert!(validate_code("-pro", "plan_").is_err());
        assert!(validate_code("pro plan", "plan_").is_err());
        assert!(validate_code(&"a".repeat(65), "plan_").is_err());
        assert!(validate_code(&"a".repeat(64), "plan_").is_ok());
        assert!(validate_code("plan_pro", "plan_").is_err());
        assert!(validate_code("plan-pro", "plan_").is_ok());
    }
}
