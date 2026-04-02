use chrono::NaiveDateTime;
use common_domain::ids::{OrganizationId, TenantId};
use diesel::{Identifiable, Insertable, NullableExpressionMethods, Queryable, Selectable};
use uuid::Uuid;

use crate::enums::DeadLetterStatusEnum;
use crate::schema::{organization, tenant};

#[derive(Debug, Clone, Queryable, Selectable, Identifiable)]
#[diesel(table_name = crate::schema::dead_letter_message)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct DeadLetterMessageRow {
    pub id: Uuid,
    pub tenant_id: Option<TenantId>,
    pub queue: String,
    pub pgmq_msg_id: i64,
    pub message: Option<serde_json::Value>,
    pub headers: Option<serde_json::Value>,
    pub read_ct: i32,
    pub enqueued_at: NaiveDateTime,
    pub dead_lettered_at: NaiveDateTime,
    pub last_error: Option<String>,
    pub status: DeadLetterStatusEnum,
    pub resolved_at: Option<NaiveDateTime>,
    pub resolved_by: Option<Uuid>,
    pub requeued_pgmq_msg_id: Option<i64>,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct DeadLetterWithTenantRow {
    #[diesel(embed)]
    pub dead_letter: DeadLetterMessageRow,
    #[diesel(select_expression = tenant::name.nullable())]
    #[diesel(select_expression_type = diesel::dsl::Nullable<tenant::name>)]
    pub tenant_name: Option<String>,
    #[diesel(select_expression = tenant::slug.nullable())]
    #[diesel(select_expression_type = diesel::dsl::Nullable<tenant::slug>)]
    pub tenant_slug: Option<String>,
    #[diesel(select_expression = tenant::organization_id.nullable())]
    #[diesel(select_expression_type = diesel::dsl::Nullable<tenant::organization_id>)]
    pub organization_id: Option<OrganizationId>,
    #[diesel(select_expression = organization::trade_name.nullable())]
    #[diesel(select_expression_type = diesel::dsl::Nullable<organization::trade_name>)]
    pub organization_name: Option<String>,
    #[diesel(select_expression = organization::slug.nullable())]
    #[diesel(select_expression_type = diesel::dsl::Nullable<organization::slug>)]
    pub organization_slug: Option<String>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::dead_letter_message)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct DeadLetterMessageRowNew {
    pub tenant_id: Option<TenantId>,
    pub queue: String,
    pub pgmq_msg_id: i64,
    pub message: Option<serde_json::Value>,
    pub headers: Option<serde_json::Value>,
    pub read_ct: i32,
    pub enqueued_at: NaiveDateTime,
    pub last_error: Option<String>,
}
