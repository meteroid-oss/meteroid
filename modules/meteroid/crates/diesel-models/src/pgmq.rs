use diesel::sql_types;
use diesel::QueryableByName;

#[derive(Debug, Clone, diesel_derive_newtype::DieselNewType)]
pub struct MessageId(pub i64);
#[derive(Debug, Clone, diesel_derive_newtype::DieselNewType)]
pub struct ReadCt(pub i32);
#[derive(Debug, Clone, diesel_derive_newtype::DieselNewType)]
pub struct Message(pub Option<serde_json::Value>);
#[derive(Debug, Clone, diesel_derive_newtype::DieselNewType)]
pub struct Headers(pub Option<serde_json::Value>);

#[derive(Debug, Clone, diesel_derive_newtype::DieselNewType)]
pub struct MessageReadQty(pub i16);
#[derive(Debug, Clone, diesel_derive_newtype::DieselNewType)]
pub struct MessageReadVtSec(pub i16);

#[derive(Debug, Clone)]
pub struct PgmqRowNew {
    pub message: Message,
    pub headers: Headers,
}

#[derive(Debug, Clone, QueryableByName)]
pub struct PgmqRow {
    #[diesel(sql_type = sql_types::BigInt)]
    pub msg_id: MessageId,
    #[diesel(sql_type = sql_types::Integer)]
    pub read_ct: ReadCt,
    #[diesel(sql_type = sql_types::Nullable<sql_types::Jsonb>)]
    pub message: Message,
    #[diesel(sql_type = sql_types::Nullable<sql_types::Jsonb>)]
    pub headers: Headers,
}
