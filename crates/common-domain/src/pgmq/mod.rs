use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Eq, PartialEq)]
#[cfg_attr(feature = "diesel", derive(diesel_derive_newtype::DieselNewType))]
pub struct MessageId(pub i64);

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
#[cfg_attr(feature = "diesel", derive(diesel_derive_newtype::DieselNewType))]
pub struct ReadCt(pub i32);

#[derive(Debug, Clone)]
#[cfg_attr(feature = "diesel", derive(diesel_derive_newtype::DieselNewType))]
pub struct Message(pub serde_json::Value);

#[derive(Debug, Clone)]
#[cfg_attr(feature = "diesel", derive(diesel_derive_newtype::DieselNewType))]
pub struct Headers(pub serde_json::Value);

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
#[cfg_attr(feature = "diesel", derive(diesel_derive_newtype::DieselNewType))]
pub struct MessageReadQty(pub i16);

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
#[cfg_attr(feature = "diesel", derive(diesel_derive_newtype::DieselNewType))]
pub struct MessageReadVtSec(pub i16);
