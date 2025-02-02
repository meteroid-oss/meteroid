use serde_with::{serde_as, DisplayFromStr};
use std::ops::Deref;
use utoipa::ToSchema;
use validator::{Validate, ValidationError};

#[serde_as]
#[derive(ToSchema, serde::Serialize, serde::Deserialize, Validate)]
pub struct PaginatedRequest {
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[validate(range(min = 0))]
    pub offset: Option<u32>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[validate(range(min = 1, max = 100))]
    pub limit: Option<u32>,
}

#[derive(ToSchema, serde::Serialize, serde::Deserialize)]
pub struct PaginatedResponse<T> {
    pub data: Vec<T>,
    pub total: u64,
    pub offset: u32,
}

#[derive(serde::Deserialize)]
pub struct IdOrAlias(pub String);

impl Deref for IdOrAlias {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<IdOrAlias> for String {
    fn from(value: IdOrAlias) -> Self {
        value.0
    }
}

impl Validate for IdOrAlias {
    fn validate(&self) -> Result<(), validator::ValidationErrors> {
        if self.0.contains(' ') || self.0.is_empty() {
            let mut errors = validator::ValidationErrors::new();
            errors.add("id_or_alias", ValidationError::new("invalid_id_or_alias"));
            return Err(errors);
        }
        Ok(())
    }
}
