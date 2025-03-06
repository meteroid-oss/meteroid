use crate::domain::{InvoicingEntityNew, Tenant};
use chrono::NaiveDateTime;
use common_domain::ids::OrganizationId;
use common_utils::rng::UPPER_ALPHANUMERIC;
use diesel_models::organizations::OrganizationRow;
use nanoid::nanoid;
use o2o::o2o;

#[derive(Clone, Debug, o2o)]
#[from_owned(OrganizationRow)]
pub struct Organization {
    pub id: OrganizationId,
    pub slug: String,
    // when a trade name gets changed, or an accounting entity gets set as default and has a different country, we update the defaults
    // This is just to simplify creating more tenants
    pub trade_name: String,
    pub default_country: String,
    pub created_at: NaiveDateTime,
    pub archived_at: Option<NaiveDateTime>,
    // pub invite_link_hash: Option<String>,
}

pub struct OrganizationWithTenants {
    pub organization: Organization,
    pub tenants: Vec<Tenant>,
}

impl Organization {
    pub fn new_slug() -> String {
        nanoid!(9, &UPPER_ALPHANUMERIC)
    }
}

#[derive(Clone, Debug)]
pub struct OrganizationNew {
    pub trade_name: String,
    pub country: String,
    pub invoicing_entity: Option<InvoicingEntityNew>,
}

pub struct InstanceFlags {
    pub multi_organization_enabled: bool,
    pub instance_initiated: bool,
    pub skip_email_validation: bool,
    pub google_oauth_client_id: Option<String>,
}
