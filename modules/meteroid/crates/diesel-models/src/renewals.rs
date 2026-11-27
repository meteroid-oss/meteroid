//! Renewal notices (R-157, R-161): the tenant's notice policy, the subscriptions that can renew
//! or end, and the claim of each notice sent.
use chrono::{DateTime, NaiveDate, Utc};
use common_domain::ids::{
    CustomerId, OrganizationId, SubscriptionId, SubscriptionPhaseId, TenantId,
};
use diesel::sql_types::{Bool, Date, Int4, Int8, Nullable, Numeric, Text, Uuid as SqlUuid};
use diesel::{Insertable, Queryable, QueryableByName, Selectable};

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::schema::renewal_settings)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct RenewalSettingsRow {
    pub tenant_id: TenantId,
    pub checkpoints: Vec<Option<i32>>,
    pub customer_reminder_days: Option<i32>,
    pub updated_at: DateTime<Utc>,
}

/// A live subscription with every date that can renew or end it, and its term when it has one.
#[derive(Debug, Clone, QueryableByName)]
pub struct RenewalCandidateRow {
    #[diesel(sql_type = SqlUuid)]
    pub tenant_id: TenantId,
    #[diesel(sql_type = SqlUuid)]
    pub organization_id: OrganizationId,
    #[diesel(sql_type = SqlUuid)]
    pub subscription_id: SubscriptionId,
    #[diesel(sql_type = SqlUuid)]
    pub customer_id: CustomerId,
    #[diesel(sql_type = Text)]
    pub customer_name: String,
    #[diesel(sql_type = Text)]
    pub plan_name: String,
    #[diesel(sql_type = Text)]
    pub currency: String,
    #[diesel(sql_type = Int8)]
    pub mrr_cents: i64,
    #[diesel(sql_type = Text)]
    pub period: String,
    #[diesel(sql_type = Nullable<Date>)]
    pub current_period_end: Option<NaiveDate>,
    #[diesel(sql_type = Nullable<Date>)]
    pub end_date: Option<NaiveDate>,
    #[diesel(sql_type = Nullable<Date>)]
    pub cancel_date: Option<NaiveDate>,
    #[diesel(sql_type = Nullable<Date>)]
    pub term_end: Option<NaiveDate>,
    #[diesel(sql_type = Nullable<Bool>)]
    pub term_auto_renew: Option<bool>,
    #[diesel(sql_type = Nullable<Int4>)]
    pub term_renewal_months: Option<i32>,
    #[diesel(sql_type = Nullable<Numeric>)]
    pub term_uplift_percent: Option<rust_decimal::Decimal>,
    #[diesel(sql_type = Nullable<Int4>)]
    pub term_notice_days: Option<i32>,
    #[diesel(sql_type = Nullable<SqlUuid>)]
    pub term_renewal_phase_id: Option<SubscriptionPhaseId>,
    /// The term is the one a trial ending in a cancellation wrote at its end (R-158 notices it).
    #[diesel(sql_type = Bool)]
    pub term_written_by_trial: bool,
}

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::schema::subscription_renewal_notice)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct RenewalNoticeRow {
    pub subscription_id: SubscriptionId,
    pub tenant_id: TenantId,
    pub renewal_date: NaiveDate,
    pub days_before: i32,
    pub mode: String,
    pub audience: String,
    pub notified_at: DateTime<Utc>,
}
