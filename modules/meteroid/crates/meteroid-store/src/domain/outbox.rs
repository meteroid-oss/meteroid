use chrono::NaiveDateTime;
use core::fmt;
use diesel_models::enums::OutboxStatus;
use nanoid::nanoid;
use o2o::o2o;
use serde::{Deserialize, Serialize};
use std::fmt::Display;
use uuid::Uuid;

use crate::domain::{InvoicingEntityNew, Tenant};
use crate::errors::{StoreError, StoreErrorReport};
use diesel_models::outbox::{OutboxRow, OutboxRowNew, OutboxRowPatch};
use error_stack::{Report, ResultExt};

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
#[serde(rename_all = "kebab-case")]
pub enum OutboxEvent {
    #[serde(rename = "invoice.finalized")]
    InvoiceFinalized,
    // TODO meter created
}

impl TryFrom<String> for OutboxEvent {
    type Error = StoreErrorReport;
    fn try_from(event: String) -> Result<Self, Self::Error> {
        serde_json::from_str(&event).map_err(|e| {
            Report::from(StoreError::SerdeError(
                "Failed to deserialize event_type".to_string(),
                e,
            ))
        })
    }
}

impl TryInto<String> for OutboxEvent {
    type Error = StoreErrorReport;
    fn try_into(self) -> Result<String, Self::Error> {
        serde_json::to_string(&self).map_err(|e| {
            Report::from(StoreError::SerdeError(
                "Failed to serialize event_type".to_string(),
                e,
            ))
        })
    }
}

#[derive(Clone, Debug, o2o)]
#[try_from_owned(OutboxRow, StoreErrorReport)]
pub struct Outbox {
    pub id: Uuid,
    #[from(~.try_into()?)]
    pub event_type: OutboxEvent,
    pub resource_id: Uuid,
    #[from(~.into())]
    pub status: OutboxStatus,
    pub payload: Option<serde_json::Value>,
    pub created_at: NaiveDateTime,
    pub processing_started_at: Option<NaiveDateTime>,
    pub processing_completed_at: Option<NaiveDateTime>,
    pub processing_attempts: i32,
    pub error: Option<String>,
}

#[derive(Clone, Debug, o2o)]
#[owned_try_into(OutboxRowNew, StoreErrorReport)]
#[ghosts(id: {uuid::Uuid::now_v7()}, status: {OutboxStatus::Pending})]
pub struct OutboxNew {
    #[into(~.try_into()?)]
    pub event_type: OutboxEvent,
    pub resource_id: Uuid,
    pub payload: Option<serde_json::Value>,
}

#[derive(Clone, Debug, o2o)]
#[owned_into(OutboxRowPatch)]
pub struct OutboxPatch {
    pub id: Uuid,
    #[into(~.into())]
    pub status: OutboxStatus,
    pub processing_started_at: Option<NaiveDateTime>,
    pub processing_completed_at: Option<NaiveDateTime>,
    pub processing_attempts: i32,
    pub error: Option<String>,
}
