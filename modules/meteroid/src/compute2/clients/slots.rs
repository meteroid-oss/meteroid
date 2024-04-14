use chrono::NaiveDate;
use std::collections::HashMap;
use uuid::Uuid;

use crate::compute2::errors::ComputeError;

use meteroid_store::repositories::subscriptions::SubscriptionSlotsInterface;


#[async_trait::async_trait]
pub trait SlotClient {
    async fn fetch_slots(
        &self,
        tenant_id: &Uuid,
        subscription_id: &Uuid,
        component_id: &Uuid,
        // slot_unit: &String,
        invoice_date: &NaiveDate,
    ) -> Result<u32, ComputeError>;
}

#[async_trait::async_trait]
impl SlotClient for meteroid_store::Store {
    async fn fetch_slots(
        &self,
        tenant_id: &Uuid,
        subscription_id: &Uuid,
        component_id: &Uuid,
        invoice_date: &NaiveDate,
    ) -> Result<u32, ComputeError> {
        let res = self
            .get_current_slots_value(
                tenant_id.clone(),
                subscription_id.clone(),
                component_id.clone(),
                invoice_date.clone().and_hms_opt(0, 0, 0),
            )
            .await
            .map_err(|_e| ComputeError::InternalError)?;

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
        _tenant_id: &Uuid,
        _subscription_id: &Uuid,
        component_id: &Uuid,
        invoice_date: &NaiveDate,
    ) -> Result<u32, ComputeError> {
        match self.data.get(&(component_id.clone(), invoice_date.clone())) {
            Some(v) => Ok(*v),
            // None => Err(ComputeError::InternalError),
            None => Ok(0),
        }
    }
}
