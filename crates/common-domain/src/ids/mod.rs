use crate::id_type;
use std::error::Error;
use std::fmt::{Debug, Display};
use std::ops::Deref;
use std::str::FromStr;
use uuid::Uuid;

mod macros;

id_type!(CustomerId, "cus_");

#[derive(Debug)]
pub struct IdError(pub(crate) String);
impl Display for IdError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "IdError: {}", self.0)
    }
}

impl Error for IdError {}

pub trait BaseId: Deref<Target = Uuid> + FromStr + Display {
    const PREFIX: &'static str;
    type IdType;

    fn new() -> Self::IdType;
    fn parse_uuid(s: &str) -> Result<Self::IdType, IdError>;
}

pub mod string_serde {
    use crate::ids::{BaseId, IdError};
    use serde::{Deserialize, Deserializer, Serializer};
    use std::str::FromStr;

    pub fn serialize<S, T>(id: &T, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        T: BaseId + std::fmt::Display,
    {
        serializer.serialize_str(&id.to_string())
    }

    pub fn deserialize<'de, D, T>(deserializer: D) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
        T: BaseId + FromStr<Err = IdError> + std::fmt::Display,
    {
        let s = String::deserialize(deserializer)?;
        T::from_str(&s).map_err(serde::de::Error::custom)
    }
}
