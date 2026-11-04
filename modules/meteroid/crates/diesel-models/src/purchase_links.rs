use chrono::NaiveDateTime;
use common_domain::ids::{
    CheckoutSessionId, CustomerId, PlanId, PurchaseLinkId, PurchaseLinkSessionId, SubscriptionId,
    TenantId,
};
use diesel::{Identifiable, Insertable, Queryable, QueryableByName, Selectable};

use crate::enums::{CustomerTypeEnum, TaxIdCollectionEnum};

#[derive(Debug, Clone, Queryable, Selectable, Identifiable)]
#[diesel(table_name = crate::schema::purchase_link)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PurchaseLinkRow {
    pub id: PurchaseLinkId,
    pub tenant_id: TenantId,
    pub plan_id: PlanId,
    pub name: String,
    pub token: String,
    pub allow_coupon_codes: bool,
    pub listed: bool,
    pub success_url: Option<String>,
    pub cancel_url: Option<String>,
    pub require_billing_address: bool,
    pub tax_id_collection: TaxIdCollectionEnum,
    pub only_buyer_type: Option<CustomerTypeEnum>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub archived_at: Option<NaiveDateTime>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::purchase_link)]
pub struct PurchaseLinkRowNew {
    pub id: PurchaseLinkId,
    pub tenant_id: TenantId,
    pub plan_id: PlanId,
    pub name: String,
    pub token: String,
    pub allow_coupon_codes: bool,
    pub listed: bool,
    pub success_url: Option<String>,
    pub cancel_url: Option<String>,
    pub require_billing_address: bool,
    pub tax_id_collection: TaxIdCollectionEnum,
    pub only_buyer_type: Option<CustomerTypeEnum>,
}

/// Full replacement of the editable fields; `None` URLs clear them.
#[derive(Debug)]
pub struct PurchaseLinkRowUpdate {
    pub name: String,
    pub allow_coupon_codes: bool,
    pub listed: bool,
    pub success_url: Option<String>,
    pub cancel_url: Option<String>,
    pub require_billing_address: bool,
    pub tax_id_collection: TaxIdCollectionEnum,
    pub only_buyer_type: Option<CustomerTypeEnum>,
}

#[derive(Debug, Clone, Queryable, Selectable, Identifiable)]
#[diesel(table_name = crate::schema::purchase_link_session)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PurchaseLinkSessionRow {
    pub id: PurchaseLinkSessionId,
    pub tenant_id: TenantId,
    pub purchase_link_id: PurchaseLinkId,
    pub customer_id: CustomerId,
    pub checkout_session_id: Option<CheckoutSessionId>,
    pub attempt_id: String,
    pub license_instance_id: Option<String>,
    pub license_instance_public_key: Option<String>,
    pub created_at: NaiveDateTime,
    pub cleaned_up_at: Option<NaiveDateTime>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::purchase_link_session)]
pub struct PurchaseLinkSessionRowNew {
    pub id: PurchaseLinkSessionId,
    pub tenant_id: TenantId,
    pub purchase_link_id: PurchaseLinkId,
    pub customer_id: CustomerId,
    pub checkout_session_id: Option<CheckoutSessionId>,
    pub attempt_id: String,
    pub license_instance_id: Option<String>,
    pub license_instance_public_key: Option<String>,
}

/// Points an attempt whose checkout closed unpaid at a new checkout.
#[derive(Debug)]
pub struct PurchaseLinkSessionRestart {
    pub customer_id: CustomerId,
    pub checkout_session_id: CheckoutSessionId,
    pub license_instance_id: Option<String>,
    pub license_instance_public_key: Option<String>,
}

/// A proof-of-work challenge already used to start a purchase.
#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::purchase_pow_spent)]
pub struct PurchasePowSpentRowNew {
    pub challenge: String,
    pub expires_at: NaiveDateTime,
}

#[derive(Debug, Clone, Queryable)]
pub struct ListedPurchaseLinkRow {
    pub id: PurchaseLinkId,
    pub name: String,
    pub plan_name: String,
    pub plan_description: Option<String>,
    pub token: String,
}

#[derive(Debug, Clone, QueryableByName)]
pub struct PurchaseLinkStatsRow {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub purchase_link_id: PurchaseLinkId,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub started: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub completed: i64,
}

#[derive(Debug, Clone, QueryableByName)]
pub struct PurchaseBindingRow {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub subscription_id: SubscriptionId,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub instance_id: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub instance_public_key: String,
}

/// A purchase whose checkout went nowhere and whose customer holds nothing billable.
#[derive(Debug, Clone, QueryableByName)]
pub struct AbandonedPurchaseRow {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub id: PurchaseLinkSessionId,
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub tenant_id: TenantId,
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub customer_id: CustomerId,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Uuid>)]
    pub checkout_session_id: Option<CheckoutSessionId>,
}
