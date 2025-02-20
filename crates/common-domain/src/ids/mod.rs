use crate::id_type;
use std::error::Error;
use std::fmt::Display;
use std::ops::Deref;
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

pub trait BaseId: Deref<Target = Uuid> {
    const PREFIX: &'static str;
    type IdType;

    fn new() -> Self::IdType;
    fn parse_uuid(s: &str) -> Result<Self::IdType, IdError>;
}
