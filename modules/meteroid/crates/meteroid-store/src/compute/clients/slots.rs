use chrono::NaiveDate;
use std::collections::HashMap;
use uuid::Uuid;

use crate::compute::errors::ComputeError;

use crate::repositories::subscriptions::SubscriptionSlotsInterface;
use common_domain::ids::TenantId;
use error_stack::{Result, ResultExt};

#[async_trait::async_trait]
pub trait SlotClient {
    async fn fetch_slots(
        &self,
        tenant_id: TenantId,
        subscription_id: &Uuid,
        component_id: &Uuid,
        // slot_unit: &String,
        invoice_date: &NaiveDate,
    ) -> Result<u32, ComputeError>;
}

#[async_trait::async_trait]
impl SlotClient for crate::Store {
    async fn fetch_slots(
        &self,
        tenant_id: TenantId,
        subscription_id: &Uuid,
        component_id: &Uuid,
        invoice_date: &NaiveDate,
    ) -> Result<u32, ComputeError> {
        let res = self
            .get_current_slots_value(
                tenant_id,
                *subscription_id,
                *component_id,
                invoice_date.clone().and_hms_opt(0, 0, 0),
            )
            .await
            .change_context(ComputeError::InternalError)?;

        Ok(res)
    }
}

pub struct MockSlotClient {
    pub data: HashMap<(Uuid, NaiveDate), u32>,
}

#[async_trait::async_trait]
impl SlotClient for MockSlotClient {
    async fn fetch_slots(
        &self,
        _tenant_id: TenantId,
        _subscription_id: &Uuid,
        component_id: &Uuid,
        invoice_date: &NaiveDate,
    ) -> Result<u32, ComputeError> {
        match self.data.get(&(*component_id, *invoice_date)) {
            Some(v) => Ok(*v),
            // None => Err(ComputeError::InternalError),
            None => Ok(0),
        }
    }
}
