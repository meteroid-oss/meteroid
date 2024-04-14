use uuid::Uuid;

use crate::enums::SubscriptionFeeBillingPeriod;
use diesel::{Identifiable, Insertable, Queryable, Selectable};

#[derive(Queryable, Debug, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::subscription_component)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionComponent {
    pub id: Uuid,
    pub name: String,
    pub subscription_id: Uuid,
    pub price_component_id: Option<Uuid>,
    pub product_item_id: Option<Uuid>,
    pub period: SubscriptionFeeBillingPeriod,
    // pub mrr_value: Option<Decimal>,
    pub fee: serde_json::Value,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::subscription_component)]
pub struct SubscriptionComponentNew {
    pub id: Uuid,
    pub name: String,
    pub subscription_id: Uuid,
    pub price_component_id: Option<Uuid>,
    pub product_item_id: Option<Uuid>,
    pub period: SubscriptionFeeBillingPeriod,
    // pub mrr_value: Option<Decimal>,
    pub fee: serde_json::Value,
}

// TODO golden tests
// #[derive(
//     serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq, FromSqlRow, AsExpression,
// )]
// #[diesel(sql_type = Jsonb)]
// #[serde(rename_all = "snake_case")]
// pub enum SubscriptionFee {
//     Rate(RatePricing),
//     OneTime(RatePricing),
//     Recurring(RatePricing),
//     Capacity(CapacityPricing),
//     Slot(SlotPricing),
//     Usage(UsagePricing),
// }
//
// impl<DB: Backend> FromSql<Jsonb, DB> for SubscriptionFee
// where
//     serde_json::Value: FromSql<Jsonb, DB>,
// {
//     fn from_sql(bytes: DB::RawValue<'_>) -> diesel::deserialize::Result<Self> {
//         let value = <serde_json::Value as FromSql<Jsonb, DB>>::from_sql(bytes)?;
//         Ok(serde_json::from_value(value)?)
//     }
// }
//
// impl ToSql<Jsonb, diesel::pg::Pg> for SubscriptionFee
// where
//     serde_json::Value: ToSql<Jsonb, diesel::pg::Pg>,
// {
//     fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, diesel::pg::Pg>) -> diesel::serialize::Result {
//         let value = serde_json::to_value(self)?;
//         <serde_json::Value as ToSql<Jsonb, diesel::pg::Pg>>::to_sql(&value, &mut out.reborrow())
//     }
// }
//
// #[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
// pub struct RatePricing {
//     pub rate: rust_decimal::Decimal,
//     pub quantity: u32,
//     pub total: rust_decimal::Decimal,
// }
//
// pub struct ExtraRecurringPricing {
//     pub rate: rust_decimal::Decimal,
//     pub quantity: u32,
//     pub total: rust_decimal::Decimal,
//     pub billing_type: crate::enums::BillingType,
// }
//
// #[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
// pub struct CapacityPricing {
//     pub rate: rust_decimal::Decimal,
//     pub included: u64,
//     pub overage_rate: rust_decimal::Decimal,
//     pub metric_id: Uuid,
// }
//
// #[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
// pub struct UsagePricing {
//     pub metric_id: Uuid,
//     pub model: UsagePricingModel,
// }
//
// #[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
// pub enum UsagePricingModel {
//     PerUnit(rust_decimal::Decimal),
//     Tiered(TierPricing),
//     Volume(TierPricing),
//     Package(PackagePricing),
// }
//
// #[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
// pub struct PackagePricing {
//     pub block_size: u64,
//     pub rate: rust_decimal::Decimal,
// }
//
// #[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
// pub struct TierPricing {
//     pub tiers: Vec<TierRow>,
//     // block size
// }
// #[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
// pub struct TierRow {
//     pub first_unit: u64,
//     // last unit is implicit.
//     pub rate: rust_decimal::Decimal,
//     pub flat_fee: Option<rust_decimal::Decimal>,
//     pub flat_cap: Option<rust_decimal::Decimal>,
// }
//
// #[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
// pub struct SlotPricing {
//     pub unit: String, // UUID?
//     pub unit_rate: rust_decimal::Decimal,
//     // upgrade downgrade policies TODO
//     pub min_slots: Option<u32>,
//     pub max_slots: Option<u32>,
//     pub initial_slots: u32,
// }
