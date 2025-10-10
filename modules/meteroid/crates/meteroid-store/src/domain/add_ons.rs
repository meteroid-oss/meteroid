use crate::domain::FeeType;
use crate::errors::{StoreError, StoreErrorReport};
use chrono::NaiveDateTime;
use common_domain::ids::{AddOnId, BaseId, TenantId};
use diesel_models::add_ons::{AddOnRow, AddOnRowNew, AddOnRowPatch};
use error_stack::Report;

#[derive(Debug, Clone)]
pub struct AddOn {
    pub id: AddOnId,
    pub name: String,
    pub fee: FeeType,
    pub tenant_id: TenantId,
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
    pub tenant_id: TenantId,
}

impl TryInto<AddOnRowNew> for AddOnNew {
    type Error = StoreErrorReport;

    fn try_into(self) -> Result<AddOnRowNew, Self::Error> {
        let json_fee = (&self.fee).try_into()?;

        Ok(AddOnRowNew {
            id: AddOnId::new(),
            tenant_id: self.tenant_id,
            name: self.name,
            fee: json_fee,
        })
    }
}

#[derive(Debug, Clone)]
pub struct AddOnPatch {
    pub id: AddOnId,
    pub tenant_id: TenantId,
    pub name: Option<String>,
    pub fee: Option<FeeType>,
}

impl TryInto<AddOnRowPatch> for AddOnPatch {
    type Error = StoreErrorReport;

    fn try_into(self) -> Result<AddOnRowPatch, Self::Error> {
        let json_fee = self.fee.map(std::convert::TryInto::try_into).transpose()?;

        Ok(AddOnRowPatch {
            id: self.id,
            tenant_id: self.tenant_id,
            name: self.name,
            fee: json_fee,
            updated_at: chrono::Utc::now().naive_utc(),
        })
    }
}
