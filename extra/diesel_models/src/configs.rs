use chrono::NaiveDateTime;
use uuid::Uuid;

use crate::enums::InvoicingProviderEnum;
use diesel::{Identifiable, Insertable, Queryable};

#[derive(Queryable, Debug, Identifiable, Insertable)]
#[diesel(table_name = crate::schema::invoicing_config)]
pub struct InvoicingConfig {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub grace_period_hours: i32,
}

#[derive(Queryable, Debug, Identifiable)]
#[diesel(table_name = crate::schema::provider_config)]
pub struct ProviderConfig {
    pub id: Uuid,
    pub created_at: NaiveDateTime,
    pub tenant_id: Uuid,
    pub invoicing_provider: InvoicingProviderEnum,
    pub enabled: bool,
    pub webhook_security: serde_json::Value,
    pub api_security: serde_json::Value,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::provider_config)]
pub struct ProviderConfigNew {
    pub tenant_id: Uuid,
    pub invoicing_provider: InvoicingProviderEnum,
    pub enabled: bool,
    pub webhook_security: serde_json::Value,
    pub api_security: serde_json::Value,
}
