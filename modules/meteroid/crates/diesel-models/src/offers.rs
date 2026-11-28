//! Offers' grants, and each application of an offer to a subscription with the engine rows it
//! wrote (ADR 0019). The offer itself is a `trial_offer` row (`TrialOfferRow`).
use chrono::{NaiveDate, NaiveDateTime};
use common_domain::ids::{
    AddOnId, CouponId, CustomerId, FeatureId, OfferApplicationId, OfferGrantId, OfferId,
    SubscriptionId, TenantId,
};
use diesel::{Identifiable, Insertable, Queryable, QueryableByName, Selectable};
use uuid::Uuid;

use crate::enums::ActorTypeEnum;

#[derive(Debug, Clone, Queryable, Selectable, Identifiable, Insertable)]
#[diesel(table_name = crate::schema::offer_grant)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct OfferGrantRow {
    pub id: OfferGrantId,
    pub tenant_id: TenantId,
    pub offer_id: OfferId,
    pub seq: i32,
    pub kind: String,
    pub anchor: String,
    pub duration_kind: String,
    pub duration_value: Option<i32>,
    pub coupon_id: Option<CouponId>,
    pub feature_id: Option<FeatureId>,
    pub entitlement: Option<serde_json::Value>,
    pub add_on_id: Option<AddOnId>,
    pub quantity: Option<i32>,
    pub add_on_end: Option<String>,
    /// Replaced by an edit: applications made before keep reading it.
    pub removed_at: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Clone, Queryable, Selectable, Identifiable, Insertable)]
#[diesel(table_name = crate::schema::offer_application)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct OfferApplicationRow {
    pub id: OfferApplicationId,
    pub tenant_id: TenantId,
    pub offer_id: OfferId,
    pub customer_id: CustomerId,
    pub subscription_id: SubscriptionId,
    pub coupon_id: Option<CouponId>,
    pub source: String,
    pub start_on: NaiveDate,
    pub billing_start_on: NaiveDate,
    pub applied_at: NaiveDateTime,
    pub actor_type: ActorTypeEnum,
    pub actor_uuid: Option<Uuid>,
    pub actor_alias: Option<String>,
    pub started_at: Option<NaiveDateTime>,
    pub withdrawn_at: Option<NaiveDateTime>,
    pub grant_ids: Vec<Option<OfferGrantId>>,
}

#[derive(Debug, Clone, Queryable, QueryableByName, Selectable, Insertable)]
#[diesel(table_name = crate::schema::offer_application_item)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct OfferApplicationItemRow {
    pub application_id: OfferApplicationId,
    pub grant_id: OfferGrantId,
    pub tenant_id: TenantId,
    pub kind: String,
    pub target_id: Uuid,
    pub created_at: NaiveDateTime,
    pub ended_at: Option<NaiveDateTime>,
    /// When its grant's window ends on its own; `None` never.
    pub ends_at: Option<NaiveDateTime>,
}
