use crate::domain::{InvoicingEntityNew, Tenant};
use chrono::{DateTime, NaiveDateTime, Utc};
use common_domain::country::CountryCode;
use common_domain::ids::{OrganizationId, OrganizationInviteId};
use common_utils::rng::UPPER_ALPHANUMERIC;
use diesel_models::enums::OrganizationUserRole;
use diesel_models::organization_invites::OrganizationInviteRow;
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
    pub default_country: CountryCode,
    pub created_at: NaiveDateTime,
    pub archived_at: Option<NaiveDateTime>,
    pub is_express: bool,
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
    pub country: CountryCode,
    pub invoicing_entity: Option<InvoicingEntityNew>,
}

pub struct InstanceFlags {
    pub multi_organization_enabled: bool,
    pub instance_initiated: bool,
    pub mailer_enabled: bool,
    pub google_oauth_client_id: Option<String>,
    pub hubspot_oauth_client_id: Option<String>,
    pub pennylane_oauth_client_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct OrganizationInvite {
    pub id: OrganizationInviteId,
    pub organization_id: OrganizationId,
    pub invited_email: String,
    pub invited_by_email: String,
    pub role: OrganizationUserRole,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub is_expired: bool,
}

impl OrganizationInvite {
    pub fn from_row_and_inviter(row: OrganizationInviteRow, inviter_email: String) -> Self {
        let is_expired = row.expires_at < Utc::now();
        OrganizationInvite {
            id: row.id,
            organization_id: row.organization_id,
            invited_email: row.invited_email,
            invited_by_email: inviter_email,
            role: row.role,
            created_at: row.created_at,
            expires_at: row.expires_at,
            is_expired,
        }
    }
}

#[derive(Clone, Debug)]
pub struct InviteDetails {
    pub organization_name: String,
    pub role: OrganizationUserRole,
    pub invited_email: String,
}
