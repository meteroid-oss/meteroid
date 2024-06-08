use chrono::NaiveDate;
use chrono::NaiveDateTime;
use uuid::Uuid;

use diesel::{Identifiable, Insertable, Queryable, Selectable};
use rust_decimal::Decimal;

#[derive(Queryable, Debug, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::subscription)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionRow {
    pub id: Uuid,
    pub customer_id: Uuid,
    pub billing_day: i16,
    pub tenant_id: Uuid,
    pub trial_start_date: Option<NaiveDate>,
    pub billing_start_date: NaiveDate,
    pub billing_end_date: Option<NaiveDate>,
    pub plan_version_id: Uuid,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub net_terms: i32,
    pub invoice_memo: Option<String>,
    pub invoice_threshold: Option<Decimal>,
    pub activated_at: Option<NaiveDateTime>,
    pub canceled_at: Option<NaiveDateTime>,
    pub cancellation_reason: Option<String>,
    pub currency: String,
    pub mrr_cents: i64,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::subscription)]
pub struct SubscriptionRowNew {
    pub id: Uuid,
    pub customer_id: Uuid,
    pub billing_day: i16,
    pub tenant_id: Uuid,
    pub trial_start_date: Option<NaiveDate>,
    pub billing_start_date: NaiveDate,
    pub billing_end_date: Option<NaiveDate>,
    pub plan_version_id: Uuid,
    pub created_by: Uuid,
    pub net_terms: i32,
    pub invoice_memo: Option<String>,
    pub invoice_threshold: Option<Decimal>,
    pub activated_at: Option<NaiveDateTime>,
    pub currency: String,
    pub mrr_cents: i64,
}

pub struct CancelSubscriptionParams {
    pub subscription_id: uuid::Uuid,
    pub tenant_id: uuid::Uuid,
    pub canceled_at: chrono::NaiveDateTime,
    pub billing_end_date: chrono::NaiveDate,
    pub reason: Option<String>,
}

use crate::schema::customer;
use crate::schema::plan;
use crate::schema::plan_version;

#[derive(Debug, Queryable, Selectable)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionForDisplayRow {
    #[diesel(embed)]
    pub subscription: SubscriptionRow,
    #[diesel(select_expression = customer::alias)]
    #[diesel(select_expression_type = customer::alias)]
    pub customer_external_id: Option<String>,
    #[diesel(select_expression = customer::name)]
    #[diesel(select_expression_type = customer::name)]
    pub customer_name: String,
    #[diesel(select_expression = plan_version::version)]
    #[diesel(select_expression_type = plan_version::version)]
    pub version: i32,
    #[diesel(select_expression = plan::name)]
    #[diesel(select_expression_type = plan::name)]
    pub plan_name: String,
    #[diesel(select_expression = plan::id)]
    #[diesel(select_expression_type = plan::id)]
    pub plan_id: Uuid,
}

#[derive(Debug, Queryable, Selectable)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionInvoiceCandidateRow {
    #[diesel(embed)]
    pub subscription: subscription_invoice_candidate::SubscriptionEmbedRow,
    #[diesel(embed)]
    pub plan_version: subscription_invoice_candidate::PlanVersionEmbedRow,
    #[diesel(embed)]
    pub subscription_components: subscription_invoice_candidate::SubscriptionComponentsEmbedRow,
}

#[derive(Debug, Queryable, Selectable)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionInvoiceCandidateGroupByRow {
    #[diesel(embed)]
    pub subscription: subscription_invoice_candidate::SubscriptionEmbedRow,
    #[diesel(embed)]
    pub plan_version: subscription_invoice_candidate::PlanVersionEmbedRow,
}

mod subscription_invoice_candidate {
    use crate::enums::SubscriptionFeeBillingPeriod;
    use crate::schema::sql_types::SubscriptionFeeBillingPeriod as SubscriptionFeeBillingPeriodSql;
    use chrono::{NaiveDate, NaiveDateTime};
    use diesel::dsl::sql;
    use diesel::expression::SqlLiteral;
    use diesel::sql_types;
    use diesel::{Queryable, Selectable};
    use uuid::Uuid;

    #[derive(Debug, Queryable, Selectable)]
    #[diesel(table_name = crate::schema::subscription)]
    #[diesel(check_for_backend(diesel::pg::Pg))]
    pub struct SubscriptionEmbedRow {
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub customer_id: Uuid,
        pub plan_version_id: Uuid,
        pub billing_start_date: NaiveDate,
        pub billing_end_date: Option<NaiveDate>,
        pub billing_day: i16,
        pub activated_at: Option<NaiveDateTime>,
        pub canceled_at: Option<NaiveDateTime>,
    }

    #[derive(Debug, Queryable, Selectable)]
    #[diesel(table_name = crate::schema::plan_version)]
    #[diesel(check_for_backend(diesel::pg::Pg))]
    pub struct PlanVersionEmbedRow {
        pub plan_id: Uuid,
        pub currency: String,
        pub net_terms: i32,
        pub version: i32,
    }

    #[derive(Debug, Queryable, Selectable)]
    #[diesel(table_name = crate::schema::subscription_component)]
    #[diesel(check_for_backend(diesel::pg::Pg))]
    pub struct SubscriptionComponentsEmbedRow {
        #[diesel(select_expression = sql("array_agg(distinct(subscription_component.period)) periods"))]
        #[diesel(
        select_expression_type = SqlLiteral::< sql_types::Array < sql_types::Nullable < SubscriptionFeeBillingPeriodSql >> >
        )]
        pub periods: Vec<Option<SubscriptionFeeBillingPeriod>>,
    }
}
