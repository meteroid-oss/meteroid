use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Eq, PartialEq)]
#[cfg_attr(feature = "diesel", derive(diesel_derive_newtype::DieselNewType))]
pub struct MessageId(pub i64);

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
#[cfg_attr(feature = "diesel", derive(diesel_derive_newtype::DieselNewType))]
pub struct ReadCt(pub i32);

#[derive(Debug, Clone)]
#[cfg_attr(feature = "diesel", derive(diesel_derive_newtype::DieselNewType))]
pub struct Message(pub Option<serde_json::Value>);

impl Message {
    pub fn none() -> Self {
        Self(None)
    }

    pub fn some(value: serde_json::Value) -> Self {
        Self(Some(value))
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "diesel", derive(diesel_derive_newtype::DieselNewType))]
pub struct Headers(pub Option<serde_json::Value>);

impl Headers {
    pub fn none() -> Self {
        Self(None)
    }

    pub fn some(value: serde_json::Value) -> Self {
        Self(Some(value))
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
#[cfg_attr(feature = "diesel", derive(diesel_derive_newtype::DieselNewType))]
pub struct MessageReadQty(pub i16);

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
#[cfg_attr(feature = "diesel", derive(diesel_derive_newtype::DieselNewType))]
pub struct MessageReadVtSec(pub i16);
