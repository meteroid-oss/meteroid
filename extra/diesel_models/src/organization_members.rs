


use uuid::Uuid;


use diesel::{Identifiable, Queryable};
use crate::enums::OrganizationUserRole;



#[derive(Queryable, Debug, Identifiable)]
#[diesel(primary_key(user_id, organization_id))]
#[diesel(table_name = crate::schema::organization_member)]
pub struct OrganizationMember {
    pub user_id: Uuid,
    pub organization_id: Uuid,
    pub role: OrganizationUserRole,
}
