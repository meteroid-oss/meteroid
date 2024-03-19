
#[derive(diesel_derive_enum::DbEnum, Debug)]
#[ExistingTypePath = "crate::schema::sql_types::BillingMetricAggregateEnum"]
pub enum BillingMetricAggregateEnum {
    Test,
}

#[derive(diesel_derive_enum::DbEnum, Debug)]
#[ExistingTypePath = "crate::schema::sql_types::BillingPeriodEnum"]
pub enum BillingPeriodEnum {
    Test,

}

#[derive(diesel_derive_enum::DbEnum, Debug)]
#[ExistingTypePath = "crate::schema::sql_types::CreditNoteStatus"]
pub enum CreditNoteStatus {
    Test,

}

#[derive(diesel_derive_enum::DbEnum, Debug)]
#[ExistingTypePath = "crate::schema::sql_types::FangTaskState"]
pub enum FangTaskState {
    Test,

}

#[derive(diesel_derive_enum::DbEnum, Debug)]
#[ExistingTypePath = "crate::schema::sql_types::InvoiceExternalStatusEnum"]
pub enum InvoiceExternalStatusEnum {
    Test,

}

#[derive(diesel_derive_enum::DbEnum, Debug)]
#[ExistingTypePath = "crate::schema::sql_types::InvoiceStatusEnum"]
pub enum InvoiceStatusEnum {
    Test,

}

#[derive(diesel_derive_enum::DbEnum, Debug)]
#[ExistingTypePath = "crate::schema::sql_types::InvoiceType"]
pub enum InvoiceType {
    Test,

}

#[derive(diesel_derive_enum::DbEnum, Debug)]
#[ExistingTypePath = "crate::schema::sql_types::InvoicingProviderEnum"]
pub enum InvoicingProviderEnum {
    Test,

}

#[derive(diesel_derive_enum::DbEnum, Debug)]
#[ExistingTypePath = "crate::schema::sql_types::MrrMovementType"]
pub enum MrrMovementType {
    Test,

}

#[derive(diesel_derive_enum::DbEnum, Debug)]
#[ExistingTypePath = "crate::schema::sql_types::OrganizationUserRole"]
pub enum OrganizationUserRole {
    Test,

}

#[derive(diesel_derive_enum::DbEnum, Debug)]
#[ExistingTypePath = "crate::schema::sql_types::PlanStatusEnum"]
pub enum PlanStatusEnum {
    Test,

}

#[derive(diesel_derive_enum::DbEnum, Debug)]
#[ExistingTypePath = "crate::schema::sql_types::PlanTypeEnum"]
pub enum PlanTypeEnum {
    Test,

}

#[derive(diesel_derive_enum::DbEnum, Debug)]
#[ExistingTypePath = "crate::schema::sql_types::UnitConversionRoundingEnum"]
pub enum UnitConversionRoundingEnum {
    Test,

}

#[derive(diesel_derive_enum::DbEnum, Debug)]
#[ExistingTypePath = "crate::schema::sql_types::WebhookOutEventTypeEnum"]
pub enum WebhookOutEventTypeEnum {
    Test,

}



