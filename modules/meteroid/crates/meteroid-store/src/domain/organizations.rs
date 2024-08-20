use chrono::NaiveDateTime;
use nanoid::nanoid;
use o2o::o2o;
use uuid::Uuid;
use common_utils::rng::UPPER_ALPHANUMERIC;

use diesel_models::organizations::OrganizationRow;
use diesel_models::organizations::OrganizationRowNew;


#[derive(Clone, Debug, o2o)]
#[from_owned(OrganizationRow)]
#[owned_into(OrganizationRow)]
pub struct Organization {
    pub id: Uuid,
    pub slug: String,
    // when a trade name gets changed, or an accounting entity gets set as default and has a different country, we update the defaults
    // This is just to simplify creating more tenants
    pub default_trade_name: String,
    pub default_country: String,
    pub created_at: NaiveDateTime,
    pub archived_at: Option<NaiveDateTime>,
    pub invite_link_hash: Option<String>,
}


impl Organization {
    pub fn new_slug() -> String {
        nanoid!(9, &UPPER_ALPHANUMERIC)
    }
}

#[derive(Clone, Debug, o2o)]
#[from_owned(OrganizationRowNew)]
pub struct OrganizationNew {
    pub id: Uuid,
    pub slug: String,
    pub default_trade_name: String,
    pub default_country: String,
}


pub struct InstanceFlags {
    pub multi_organization_enabled: bool,
    pub instance_initiated: bool,
}