use crate::domain::FeeType;
use crate::errors::{StoreError, StoreErrorReport};
use crate::utils::local_id::{IdType, LocalId};
use chrono::NaiveDateTime;
use diesel_models::add_ons::{AddOnRow, AddOnRowNew, AddOnRowPatch};
use error_stack::Report;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct AddOn {
    pub id: Uuid,
    pub local_id: String,
    pub name: String,
    pub fee: FeeType,
    pub tenant_id: Uuid,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

impl TryInto<AddOn> for AddOnRow {
    type Error = Report<StoreError>;

    fn try_into(self) -> Result<AddOn, Self::Error> {
        let fee: FeeType = self.fee.try_into()?;

        Ok(AddOn {
            id: self.id,
            name: self.name,
            local_id: self.local_id,
            fee,
            tenant_id: self.tenant_id,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[derive(Debug, Clone)]
pub struct AddOnNew {
    pub name: String,
    pub fee: FeeType,
    pub tenant_id: Uuid,
}

impl TryInto<AddOnRowNew> for AddOnNew {
    type Error = StoreErrorReport;

    fn try_into(self) -> Result<AddOnRowNew, Self::Error> {
        let json_fee = (&self.fee).try_into()?;

        Ok(AddOnRowNew {
            id: Uuid::now_v7(),
            local_id: LocalId::generate_for(IdType::AddOn),
            tenant_id: self.tenant_id,
            name: self.name,
            fee: json_fee,
        })
    }
}

#[derive(Debug, Clone)]
pub struct AddOnPatch {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: Option<String>,
    pub fee: Option<FeeType>,
}

impl TryInto<AddOnRowPatch> for AddOnPatch {
    type Error = StoreErrorReport;

    fn try_into(self) -> Result<AddOnRowPatch, Self::Error> {
        let json_fee = self.fee.map(|x| x.try_into()).transpose()?;

        Ok(AddOnRowPatch {
            id: self.id,
            tenant_id: self.tenant_id,
            name: self.name,
            fee: json_fee,
            updated_at: chrono::Utc::now().naive_utc(),
        })
    }
}
