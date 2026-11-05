use chrono::NaiveDateTime;

use crate::enums::{EmailDeliveryModeEnum, TenantEnvironmentEnum, TenantStatusEnum};

use crate::organizations::OrganizationRow;
use common_domain::ids::{OrganizationId, TenantId};
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};

#[derive(Clone, Queryable, Debug, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::tenant)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct TenantRow {
    pub id: TenantId,
    pub name: String,
    pub slug: String,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
    pub organization_id: OrganizationId,
    // the reporting currency, used in dashboards
    pub reporting_currency: String,
    pub environment: TenantEnvironmentEnum,
    pub available_currencies: Vec<Option<String>>,
    pub disable_emails: bool,
    pub last_active_at: Option<NaiveDateTime>,
    pub status: TenantStatusEnum,
    pub alert_evaluation_interval_secs: Option<i32>,
    pub email_delivery_mode: EmailDeliveryModeEnum,
    #[diesel(deserialize_as = crate::extend::non_null_array::NonNullArray<String>)]
    pub email_test_recipients: Vec<String>,
    pub keep_last_events: bool,
    #[diesel(deserialize_as = crate::extend::non_null_array::NonNullArray<String>)]
    pub portal_embed_origins: Vec<String>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::tenant)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct TenantRowNew {
    pub id: TenantId,
    pub name: String,
    pub slug: String,
    pub organization_id: OrganizationId,
    pub reporting_currency: String,
    pub environment: TenantEnvironmentEnum,
    pub available_currencies: Vec<Option<String>>,
    /// Superseded by `email_delivery_mode`; written as `mode <> LIVE` for one release.
    pub disable_emails: bool,
    pub email_delivery_mode: EmailDeliveryModeEnum,
    pub email_test_recipients: Vec<String>,
}

#[derive(Debug, AsChangeset)]
#[diesel(table_name = crate::schema::tenant)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct TenantRowPatch {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub reporting_currency: Option<String>,
    pub environment: Option<TenantEnvironmentEnum>,
    pub disable_emails: Option<bool>,
    // Nullable-column changeset semantics: None skips, Some(None) sets NULL,
    // Some(Some(v)) sets v.
    pub alert_evaluation_interval_secs: Option<Option<i32>>,
    pub email_delivery_mode: Option<EmailDeliveryModeEnum>,
    pub email_test_recipients: Option<Vec<String>>,
    pub keep_last_events: Option<bool>,
    pub portal_embed_origins: Option<Vec<String>>,
}

#[derive(Debug, Queryable, Selectable)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct TenantWithOrganizationRow {
    #[diesel(embed)]
    pub tenant: TenantRow,
    #[diesel(embed)]
    pub organization: OrganizationRow,
}
