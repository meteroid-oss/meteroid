use crate::ids::BaseId;
use serde::{Serialize, Serializer};
use std::convert::Infallible;
use std::fmt::Display;
use std::str::FromStr;
use validator::{Validate, ValidationError};

#[derive(Debug, PartialEq, Clone)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub enum AliasOr<ID: BaseId> {
    Id(ID),
    Alias(String),
}

impl<ID> From<ID> for AliasOr<ID>
where
    ID: BaseId,
{
    fn from(id: ID) -> Self {
        AliasOr::Id(id)
    }
}

impl<ID> FromStr for AliasOr<ID>
where
    ID: BaseId + FromStr,
{
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.starts_with(ID::PREFIX) {
            Ok(ID::from_str(s)
                .map(AliasOr::Id)
                .unwrap_or(AliasOr::Alias(s.to_owned())))
        } else {
            Ok(AliasOr::Alias(s.to_owned()))
        }
    }
}

impl<ID> Display for AliasOr<ID>
where
    ID: BaseId + Display,
{
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            AliasOr::Id(id) => write!(f, "{}", id),
            AliasOr::Alias(alias) => write!(f, "{}", alias),
        }
    }
}

impl<'de, ID> serde::Deserialize<'de> for AliasOr<ID>
where
    ID: BaseId + FromStr,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;

        AliasOr::from_str(&s).map_err(serde::de::Error::custom)
    }
}

impl<ID> Serialize for AliasOr<ID>
where
    ID: BaseId + Display,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<ID> Validate for AliasOr<ID>
where
    ID: BaseId,
{
    fn validate(&self) -> Result<(), validator::ValidationErrors> {
        match self {
            AliasOr::Id(_) => Ok(()),
            AliasOr::Alias(alias) => {
                if alias.contains(' ') || alias.is_empty() {
                    let mut errors = validator::ValidationErrors::new();
                    errors.add("id_or_alias", ValidationError::new("invalid_id_or_alias"));
                    return Err(errors);
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::CustomerId;
    use std::str::FromStr;

    #[test]
    fn test_to_string() {
        let id = CustomerId::default();
        let or_alias: AliasOr<CustomerId> = id.into();

        assert_eq!(or_alias.to_string(), "cus_7n42DGM5Tflk9n8mt7Fhc7");
    }

    #[test]
    fn test_from_str() {
        let or_alias: AliasOr<CustomerId> = AliasOr::from_str("cus_2zAL4dEXftiMkNjwxwiwI").unwrap();

        assert_eq!(
            or_alias,
            AliasOr::Id(CustomerId::from_str("cus_2zAL4dEXftiMkNjwxwiwI").unwrap())
        );

        let or_alias: AliasOr<CustomerId> =
            AliasOr::from_str("hello2zAL4dEXftiMkNjwxwiwI").unwrap();

        assert_eq!(
            or_alias,
            AliasOr::Alias("hello2zAL4dEXftiMkNjwxwiwI".to_owned())
        );

        let or_alias: AliasOr<CustomerId> =
            AliasOr::from_str("3e016721-bb0b-4c05-b1ba-f40f74d4f680").unwrap();
        assert_eq!(
            or_alias,
            AliasOr::Alias("3e016721-bb0b-4c05-b1ba-f40f74d4f680".to_owned())
        );
    }

    #[test]
    fn test_deserialize() {
        let or_alias: AliasOr<CustomerId> =
            serde_json::from_str("\"cus_2zAL4dEXftiMkNjwxwiwI\"").expect("Failed to deserialize");

        assert_eq!(
            or_alias,
            AliasOr::Id(CustomerId::from_str("cus_2zAL4dEXftiMkNjwxwiwI").unwrap())
        );

        let or_alias: AliasOr<CustomerId> =
            serde_json::from_str("\"hello2zAL4dEXftiMkNjwxwiwI\"").expect("Failed to deserialize");

        assert_eq!(
            or_alias,
            AliasOr::Alias("hello2zAL4dEXftiMkNjwxwiwI".to_owned())
        );
    }
}
