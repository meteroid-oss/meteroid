//! Volume agreements (ADR-0021): subscriptions whose pooled metrics' tiers count the volume of all
//! of them. Billing reads these and writes the window bases; the module writes the rest.
use chrono::{DateTime, NaiveDate, Utc};
use common_domain::ids::{
    BillableMetricId, CustomerId, SubscriptionId, TenantId, VolumeAgreementId,
    VolumeAgreementMemberId,
};
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::enums::BillingPeriodEnum;

#[derive(Debug, Clone, Queryable, Selectable, Identifiable, Insertable, AsChangeset)]
#[diesel(table_name = crate::schema::volume_agreement)]
#[diesel(treat_none_as_null = true)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct VolumeAgreementRow {
    pub id: VolumeAgreementId,
    pub tenant_id: TenantId,
    pub name: String,
    pub currency: String,
    pub cadence: BillingPeriodEnum,
    pub billing_day_anchor: i16,
    pub start_date: NaiveDate,
    pub end_date: Option<NaiveDate>,
    pub owner_customer_id: Option<CustomerId>,
    pub archived_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub created_by: Uuid,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::schema::volume_agreement_metric)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct VolumeAgreementMetricRow {
    pub agreement_id: VolumeAgreementId,
    pub tenant_id: TenantId,
    pub billable_metric_id: BillableMetricId,
}

#[derive(Debug, Clone, Queryable, Selectable, Identifiable, Insertable)]
#[diesel(table_name = crate::schema::volume_agreement_member)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct VolumeAgreementMemberRow {
    pub id: VolumeAgreementMemberId,
    pub tenant_id: TenantId,
    pub agreement_id: VolumeAgreementId,
    pub subscription_id: SubscriptionId,
    pub from_date: NaiveDate,
    pub until_date: Option<NaiveDate>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::schema::volume_agreement_basis)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct VolumeAgreementBasisRow {
    pub tenant_id: TenantId,
    pub agreement_id: VolumeAgreementId,
    pub billable_metric_id: BillableMetricId,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub quantity: Decimal,
    pub closed_at: DateTime<Utc>,
}
