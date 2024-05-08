use uuid::Uuid;

use crate::enums::OrganizationUserRole;
use diesel::{Identifiable, Insertable, Queryable};

#[derive(Queryable, Debug, Identifiable, Insertable)]
#[diesel(primary_key(user_id, organization_id))]
#[diesel(table_name = crate::schema::organization_member)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct OrganizationMember {
    pub user_id: Uuid,
    pub organization_id: Uuid,
    pub role: OrganizationUserRole,
}
