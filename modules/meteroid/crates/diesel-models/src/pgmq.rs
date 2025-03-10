use common_domain::pgmq::Headers;
use common_domain::pgmq::Message;
use common_domain::pgmq::MessageId;
use common_domain::pgmq::ReadCt;
use diesel::QueryableByName;
use diesel::sql_types;

#[derive(Debug, Clone)]
pub struct PgmqMessageRowNew {
    pub message: Message,
    pub headers: Headers,
}

#[derive(Debug, Clone, QueryableByName)]
pub struct PgmqMessageRow {
    #[diesel(sql_type = sql_types::BigInt)]
    pub msg_id: MessageId,
    #[diesel(sql_type = sql_types::Integer)]
    pub read_ct: ReadCt,
    #[diesel(sql_type = sql_types::Nullable<sql_types::Jsonb>)]
    pub message: Message,
    #[diesel(sql_type = sql_types::Nullable<sql_types::Jsonb>)]
    pub headers: Headers,
}
