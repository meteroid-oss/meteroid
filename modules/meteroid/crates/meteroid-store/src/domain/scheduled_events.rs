use crate::domain::enums::{ScheduledEventStatus, ScheduledEventTypeEnum};
use crate::errors::StoreErrorReport;
use crate::json_value_serde;
use chrono::NaiveDateTime;
use common_domain::ids::{InvoiceId, PlanVersionId, SubscriptionId, TenantId};
use diesel_models::scheduled_events::ScheduledEventRow;
use diesel_models::scheduled_events::ScheduledEventRowNew;
use o2o::o2o;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, o2o)]
#[try_from_owned(ScheduledEventRow, StoreErrorReport)]
pub struct ScheduledEvent {
    pub id: Uuid,
    pub subscription_id: SubscriptionId,
    pub tenant_id: TenantId,
    #[from(~.into())]
    pub event_type: ScheduledEventTypeEnum,
    pub scheduled_time: NaiveDateTime,
    pub priority: i32,
    #[from(~.try_into()?)]
    pub event_data: ScheduledEventData,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    #[from(~.into())]
    pub status: ScheduledEventStatus,
    pub retries: i32,
    pub last_retry_at: Option<NaiveDateTime>,
    pub error: Option<String>,
    pub processed_at: Option<NaiveDateTime>,
    pub source: String, // API, System, etc.
}

#[derive(Clone, Debug, o2o)]
#[owned_try_into(ScheduledEventRowNew, StoreErrorReport)]
#[ghosts(
    id: Uuid::now_v7(),
    event_type: @.event_data.to_event_type_enum().into(),
    status: diesel_models::enums::ScheduledEventStatus::Pending,
    priority: 0,
    retries: 0
)] // TODO drop priority if unused
pub struct ScheduledEventNew {
    pub subscription_id: SubscriptionId,
    pub tenant_id: TenantId,
    pub scheduled_time: NaiveDateTime,
    #[into(~.clone().try_into()?)]
    pub event_data: ScheduledEventData,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ScheduledEventData {
    CancelSubscription { reason: Option<String> },
    PauseSubscription,
    FinalizeInvoice { invoice_id: InvoiceId },
    RetryPayment { invoice_id: InvoiceId },
    ApplyPlanChange { new_plan_version_id: PlanVersionId },
    // Promotions events
    // ApplyCoupon {
    //     coupon_id: String,
    //     discount_amount_cents: Option<i64>,
    //     discount_percentage: Option<f64>,
    // },
    //
    // RemoveCoupon {
    //     coupon_id: String,
    // },
}

json_value_serde!(ScheduledEventData);

impl ScheduledEventData {
    pub fn to_event_type_enum(&self) -> ScheduledEventTypeEnum {
        match self {
            Self::CancelSubscription { .. } => ScheduledEventTypeEnum::CancelSubscription,
            Self::PauseSubscription { .. } => ScheduledEventTypeEnum::PauseSubscription,
            Self::FinalizeInvoice { .. } => ScheduledEventTypeEnum::FinalizeInvoice,
            Self::RetryPayment { .. } => ScheduledEventTypeEnum::RetryPayment,
            Self::ApplyPlanChange { .. } => ScheduledEventTypeEnum::ApplyPlanChange,
        }
    }
}
